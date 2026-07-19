use std::{
    str::FromStr,
    time::{Duration, Instant},
};

use anyhow::Result;
use clap::{Parser, Subcommand};
use logilightshow::{
    FrameSource, Rgb8, ZoneColors,
    capture::{GStreamerFrameSource, PortalCapture},
    usb::{G560, LibUsbTransport},
};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    SetZones {
        #[arg(long)]
        left_rear: HexColor,
        #[arg(long)]
        left_front: HexColor,
        #[arg(long)]
        right_front: HexColor,
        #[arg(long)]
        right_rear: HexColor,
    },
    /// Hardware verification pulse: all black, one raw zone white, all black.
    VerifyZone {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=3))]
        protocol_index: u8,
    },
    /// Five-minute hardware/audio verification with rotating per-zone colors.
    VerifySoak,
    /// Capture frames from one portal-authorized monitor without saving images.
    CaptureTest {
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..))]
        frames: u64,
    },
}

#[derive(Clone)]
struct HexColor(Rgb8);

impl FromStr for HexColor {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("invalid color: expected exactly six hexadecimal digits (RRGGBB)".into());
        }
        let channel = |range| u8::from_str_radix(&value[range], 16).expect("validated hex");
        Ok(Self(Rgb8 {
            r: channel(0..2),
            g: channel(2..4),
            b: channel(4..6),
        }))
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let Cli { command } = Cli::parse();
    match command {
        Command::SetZones {
            left_rear,
            left_front,
            right_front,
            right_rear,
        } => {
            let mut device = G560::new(LibUsbTransport::open()?);
            device.write(ZoneColors([
                left_rear.0,
                left_front.0,
                right_front.0,
                right_rear.0,
            ]))?;
        }
        Command::VerifyZone { protocol_index } => {
            let mut device = G560::new(LibUsbTransport::open()?);
            device.pulse_protocol_zone(protocol_index, Duration::from_secs(12))?;
        }
        Command::VerifySoak => {
            let mut device = G560::new(LibUsbTransport::open()?);
            let mut colors = [
                Rgb8 { r: 255, g: 0, b: 0 },
                Rgb8 { r: 0, g: 255, b: 0 },
                Rgb8 { r: 0, g: 0, b: 255 },
                Rgb8 {
                    r: 255,
                    g: 255,
                    b: 255,
                },
            ];
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(300) {
                if let Err(error) = device.write(ZoneColors(colors)) {
                    let _ = device.blackout();
                    return Err(error.into());
                }
                colors.rotate_left(1);
                std::thread::sleep(Duration::from_millis(190));
            }
            device.blackout()?;
        }
        Command::CaptureTest { frames } => {
            let grant = PortalCapture::open(None).await?;
            let permission_is_persistent = grant.restore_token.is_some();
            let mut source = GStreamerFrameSource::open(grant).await?;
            let started = Instant::now();
            let capture_result: Result<(usize, usize)> = async {
                let mut dimensions = None;
                for _ in 0..frames {
                    let frame = source
                        .next_frame()
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("capture ended before {frames} frames"))?;
                    dimensions = Some((frame.width, frame.height));
                }
                Ok(dimensions.expect("positive frame count has dimensions"))
            }
            .await;
            let shutdown_result = source.shutdown().await.map_err(anyhow::Error::new);
            let (width, height) = match (capture_result, shutdown_result) {
                (Ok(dimensions), Ok(())) => dimensions,
                (Err(error), Ok(())) => return Err(error),
                (Ok(_), Err(error)) => return Err(error),
                (Err(primary), Err(shutdown)) => {
                    return Err(anyhow::anyhow!(
                        "{primary:#}; capture shutdown also failed: {shutdown:#}"
                    ));
                }
            };
            let elapsed = started.elapsed();
            let effective_fps = frames as f64 / elapsed.as_secs_f64();
            println!(
                "captured {frames} frames at {width}x{height} in {:.2}s ({effective_fps:.1} fps); persistent permission: {}; saved images: 0",
                elapsed.as_secs_f64(),
                if permission_is_persistent {
                    "yes"
                } else {
                    "not returned"
                }
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_six_digit_hex_colors() {
        assert_eq!(
            HexColor::from_str("12aBf0").unwrap().0,
            Rgb8 {
                r: 0x12,
                g: 0xab,
                b: 0xf0
            }
        );
    }

    #[test]
    fn rejects_malformed_colors() {
        for value in ["12345", "1234567", "not-a-color", "12345g"] {
            assert!(HexColor::from_str(value).is_err(), "accepted {value}");
        }
    }
}
