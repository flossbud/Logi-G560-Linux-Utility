use std::{
    str::FromStr,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use logig560::{
    CaptureBackend, ConfigStore, FileConfigStore, FrameSource, Rgb8, SamplerConfig, ZoneColors,
    ZoneLayout, ZoneMasks,
    capture::{GStreamerFrameSource, GamescopeFrameSource, PortalCapture},
    config_path, sample_zones,
    service::{ServiceOptions, run_service},
    usb::{G560, LibUsbTransport},
};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

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
    /// Diagnostic-only USB pacing calibration; never changes saved settings.
    CalibratePacing {
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        delay_ms: u64,
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
        seconds: u64,
    },
    /// Capture frames from one portal-authorized monitor without saving images.
    CaptureTest {
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..))]
        frames: u64,
        /// Reuse the monitor permission saved by `run` instead of opening a chooser.
        #[arg(long)]
        saved_permission: bool,
        /// Capture Gamescope's Gaming Mode output instead of using a portal.
        #[arg(long, conflicts_with = "saved_permission")]
        gamescope: bool,
    },
    /// Match the selected monitor's four edge regions on the four G560 zones.
    Run,
    /// Match Bazzite Gaming Mode through Gamescope's native PipeWire source.
    RunGaming,
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
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("logig560=info,warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init()
        .ok();

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
        Command::CalibratePacing { delay_ms, seconds } => {
            let delay = Duration::from_millis(delay_ms);
            let mut device =
                G560::<LibUsbTransport>::open_diagnostic(delay, LibUsbTransport::open)?;
            println!(
                "starting diagnostic USB pacing calibration: delay_ms={delay_ms} duration_s={seconds}; configuration writes: 0"
            );
            let report = device.calibrate_pacing(Duration::from_secs(seconds));
            println!(
                "calibration reports: attempted={} successful={}; cleanup reports: attempted={} successful={}",
                report.attempted_reports,
                report.successful_reports,
                report.cleanup_attempted_reports,
                report.cleanup_successful_reports,
            );
            if let Some(error) = report.first_error.as_deref() {
                eprintln!("first calibration error: {error}");
            }
            if let Some(error) = report.cleanup_error.as_deref() {
                eprintln!("first cleanup error: {error}");
            }
            if !report.succeeded() {
                anyhow::bail!("USB pacing calibration failed");
            }
            println!("calibration completed without USB errors; final all-zone black succeeded");
        }
        Command::CaptureTest {
            frames,
            saved_permission,
            gamescope,
        } => {
            let (mut source, permission_status): (Box<dyn FrameSource>, _) = if gamescope {
                (
                    Box::new(GamescopeFrameSource::open().await?),
                    "not applicable",
                )
            } else {
                let restore_token = if saved_permission {
                    FileConfigStore::new(config_path()?)
                        .load()?
                        .into_config()
                        .restore_token
                } else {
                    None
                };
                let grant = PortalCapture::open(restore_token).await?;
                let permission_status = if grant.restore_token.is_some() {
                    "yes"
                } else {
                    "not returned"
                };
                (
                    Box::new(GStreamerFrameSource::open(grant).await?),
                    permission_status,
                )
            };
            let started = Instant::now();
            let capture_result: Result<(usize, usize, usize, usize, u8, ZoneColors)> = async {
                let mut dimensions = None;
                let mut non_black_frames = 0;
                let mut peak_visible_pixels = 0;
                let mut peak_component = 0;
                let mut last_sampled = ZoneColors::BLACK;
                for _ in 0..frames {
                    let frame = source
                        .next_frame()
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("capture ended before {frames} frames"))?;
                    let packed_stride = frame.width * 3;
                    let mut visible_pixels = 0;
                    let mut frame_peak = 0;
                    for pixel in frame
                        .pixels
                        .chunks_exact(frame.stride)
                        .flat_map(|row| row[..packed_stride].chunks_exact(3))
                    {
                        let component = *pixel.iter().max().expect("RGB pixel has components");
                        frame_peak = frame_peak.max(component);
                        visible_pixels += usize::from(component >= 32);
                    }
                    if frame_peak != 0 {
                        non_black_frames += 1;
                    }
                    peak_visible_pixels = peak_visible_pixels.max(visible_pixels);
                    peak_component = peak_component.max(frame_peak);
                    let masks =
                        ZoneMasks::compile(&ZoneLayout::g560_default(), frame.width, frame.height)?;
                    last_sampled = sample_zones(&frame, &masks, SamplerConfig::default());
                    dimensions = Some((frame.width, frame.height));
                }
                let (width, height) = dimensions.expect("positive frame count has dimensions");
                Ok((
                    width,
                    height,
                    non_black_frames,
                    peak_visible_pixels,
                    peak_component,
                    last_sampled,
                ))
            }
            .await;
            let shutdown_result = source.shutdown().await;
            let (
                width,
                height,
                non_black_frames,
                peak_visible_pixels,
                peak_component,
                last_sampled,
            ) = match (capture_result, shutdown_result) {
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
                "captured {frames} frames at {width}x{height} in {:.2}s ({effective_fps:.1} fps); non-black frames: {non_black_frames}; peak visible pixels: {peak_visible_pixels}/{}; peak component: {peak_component}; sampled zones: {last_sampled:?}; persistent permission: {}; saved images: 0",
                elapsed.as_secs_f64(),
                width * height,
                permission_status,
            );
        }
        Command::Run => serve(CaptureBackend::DesktopPortal).await?,
        Command::RunGaming => serve(CaptureBackend::Gamescope).await?,
    }
    Ok(())
}

async fn serve(backend: CaptureBackend) -> Result<()> {
    let shutdown = CancellationToken::new();
    let signal_cancel = shutdown.clone();
    let signal_task = tokio::spawn(async move {
        if shutdown_signal().await.is_ok() {
            signal_cancel.cancel();
        }
    });
    let mut options = ServiceOptions::new(backend);
    options.shutdown = shutdown;
    let result = run_service(options).await;
    signal_task.abort();
    result
}

async fn shutdown_signal() -> Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("install SIGTERM handler")?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.context("install Ctrl-C handler"),
        _ = terminate.recv() => Ok(()),
    }
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

    #[test]
    fn parses_run_command() {
        let cli = Cli::try_parse_from(["logig560", "run"]).unwrap();
        assert!(matches!(cli.command, Command::Run));
    }

    #[test]
    fn parses_run_gaming_command() {
        let cli = Cli::try_parse_from(["logig560", "run-gaming"]).unwrap();
        assert!(matches!(cli.command, Command::RunGaming));
    }

    #[test]
    fn calibration_cli_requires_positive_delay_and_duration() {
        let cli = Cli::try_parse_from([
            "logig560",
            "calibrate-pacing",
            "--delay-ms",
            "6",
            "--seconds",
            "120",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Command::CalibratePacing {
                delay_ms: 6,
                seconds: 120
            }
        ));

        assert!(
            Cli::try_parse_from([
                "logig560",
                "calibrate-pacing",
                "--delay-ms",
                "0",
                "--seconds",
                "15",
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "logig560",
                "calibrate-pacing",
                "--delay-ms",
                "6",
                "--seconds",
                "0",
            ])
            .is_err()
        );
    }
}
