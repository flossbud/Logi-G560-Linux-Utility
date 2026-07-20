use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    str::FromStr,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use directories::BaseDirs;
use logilightshow::{
    CaptureRecoverySnapshot, EngineMetrics, EngineSnapshot, FrameSource, FrameSourceFactory,
    LightSink, RecoveringFrameSource, RecoveringLightSink, Rgb8, SamplerConfig, ZoneColors,
    ZoneLayout,
    capture::{CaptureError, GStreamerFrameSource, PortalCapture},
    run_engine_with_metrics,
    usb::{AsyncG560, G560, LibUsbTransport},
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

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
    },
    /// Match the selected monitor's four edge regions on the four G560 zones.
    Run,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct CaptureConfig {
    version: u32,
    restore_token: Option<String>,
}

const CAPTURE_CONFIG_VERSION: u32 = 1;

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
        Command::Run => run_live().await?,
    }
    Ok(())
}

async fn run_live() -> Result<()> {
    let config_path = capture_config_path()?;
    let saved_restore_token = load_capture_config(&config_path)?.restore_token;
    let source = RecoveringFrameSource::new(PortalFrameSourceFactory {
        config_path,
        restore_token: saved_restore_token,
        has_opened: false,
    });
    let capture_recovery_metrics = source.metrics();

    let cancellation = CancellationToken::new();
    let signal_cancellation = cancellation.clone();
    let signal_task = tokio::spawn(async move {
        if shutdown_signal().await.is_ok() {
            signal_cancellation.cancel();
        }
    });
    let factory = || -> Result<AsyncG560<LibUsbTransport>> {
        let mut device = G560::new(LibUsbTransport::open()?);
        device.blackout()?;
        Ok(AsyncG560::new(device))
    };
    let mut sink = RecoveringLightSink::new(factory, cancellation.clone());
    let recovery_metrics = sink.metrics();
    if let Err(error) = sink.write(ZoneColors::BLACK).await {
        signal_task.abort();
        if cancellation.is_cancelled() {
            return Ok(());
        }
        return Err(error.context("initial G560 blackout failed"));
    }

    let metrics = EngineMetrics::new();
    let mut previous = metrics.snapshot();
    let engine = run_engine_with_metrics(
        source,
        sink,
        ZoneLayout::g560_default(),
        SamplerConfig::default(),
        cancellation.clone().cancelled_owned(),
        metrics.clone(),
    );
    tokio::pin!(engine);
    let mut reporting = tokio::time::interval(Duration::from_secs(5));
    reporting.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    reporting.tick().await;

    let result = loop {
        tokio::select! {
            result = &mut engine => break result,
            _ = reporting.tick() => {
                let current = metrics.snapshot();
                print_interval(
                    previous,
                    current,
                    recovery_metrics.snapshot(),
                    capture_recovery_metrics.snapshot(),
                );
                previous = current;
            }
        }
    };
    signal_task.abort();
    let totals = metrics.snapshot();
    let recovery = recovery_metrics.snapshot();
    let capture_recovery = capture_recovery_metrics.snapshot();
    println!(
        "totals: elapsed={:.2}s captured={} rendered={} dropped={} stalls={} latency_ms[p50={:.2} p95={:.2} p99={:.2}] USB_errors={} open_failures={} reopens={} blackout_failures={} capture_stream_failures={} capture_open_failures={} capture_reopens={} capture_shutdown_failures={}",
        totals.elapsed.as_secs_f64(),
        totals.captured_frames,
        totals.rendered_updates,
        totals.dropped_frames,
        totals.capture_stalls,
        micros_to_millis(totals.capture_to_write_p50_us),
        micros_to_millis(totals.capture_to_write_p95_us),
        micros_to_millis(totals.capture_to_write_p99_us),
        recovery.usb_write_failures,
        recovery.open_failures,
        recovery.reopen_count,
        recovery.blackout_failures,
        capture_recovery.stream_failures,
        capture_recovery.open_failures,
        capture_recovery.successful_reopens,
        capture_recovery.shutdown_failures,
    );
    match result {
        Ok(_) => Ok(()),
        Err(error) if capture_was_cancelled(&error) => Ok(()),
        Err(error) => Err(error),
    }
}

async fn shutdown_signal() -> Result<()> {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("install SIGTERM handler")?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result.context("install Ctrl-C handler"),
        _ = terminate.recv() => Ok(()),
    }
}

fn print_interval(
    previous: EngineSnapshot,
    current: EngineSnapshot,
    recovery: logilightshow::RecoverySnapshot,
    capture_recovery: CaptureRecoverySnapshot,
) {
    let elapsed = current
        .elapsed
        .saturating_sub(previous.elapsed)
        .as_secs_f64()
        .max(f64::EPSILON);
    let captured_fps = current
        .captured_frames
        .saturating_sub(previous.captured_frames) as f64
        / elapsed;
    let rendered_fps = current
        .rendered_updates
        .saturating_sub(previous.rendered_updates) as f64
        / elapsed;
    println!(
        "stats: captured_fps={captured_fps:.1} rendered_updates_per_second={rendered_fps:.1} dropped={} stalls={} latency_ms[p50={:.2} p95={:.2} p99={:.2}] USB_errors={} capture_errors[stream={} open={} reopens={} shutdown={}]",
        current.dropped_frames,
        current.capture_stalls,
        micros_to_millis(current.capture_to_write_p50_us),
        micros_to_millis(current.capture_to_write_p95_us),
        micros_to_millis(current.capture_to_write_p99_us),
        recovery.usb_write_failures,
        capture_recovery.stream_failures,
        capture_recovery.open_failures,
        capture_recovery.successful_reopens,
        capture_recovery.shutdown_failures,
    );
}

struct PortalFrameSourceFactory {
    config_path: PathBuf,
    restore_token: Option<String>,
    has_opened: bool,
}

#[async_trait::async_trait]
impl FrameSourceFactory for PortalFrameSourceFactory {
    type Source = GStreamerFrameSource;

    async fn open(&mut self) -> Result<Self::Source> {
        let grant = PortalCapture::open(self.restore_token.clone()).await?;
        if let Some(restore_token) = grant.restore_token.as_deref() {
            save_restore_token(&self.config_path, restore_token)?;
            self.restore_token = Some(restore_token.to_owned());
        }
        let source = GStreamerFrameSource::open(grant).await?;
        self.has_opened = true;
        Ok(source)
    }

    fn should_retry(&self, error: &anyhow::Error) -> bool {
        error
            .chain()
            .find_map(|cause| cause.downcast_ref::<CaptureError>())
            .is_some_and(|error| capture_error_is_retryable(error, self.has_opened))
    }
}

fn capture_error_is_retryable(error: &CaptureError, has_opened: bool) -> bool {
    match error {
        CaptureError::Portal { .. } => true,
        CaptureError::Pipeline { .. } | CaptureError::Shutdown { .. } => has_opened,
        CaptureError::GStreamer { .. } => has_opened,
        CaptureError::Cleanup { primary, .. } => capture_error_is_retryable(primary, has_opened),
        CaptureError::CaptureCancelled
        | CaptureError::UnexpectedStreamCount { .. }
        | CaptureError::MissingSampleData { .. }
        | CaptureError::UnsupportedCaps { .. }
        | CaptureError::InvalidFrameLayout { .. }
        | CaptureError::EndOfStream => false,
    }
}

fn capture_was_cancelled(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<CaptureError>(),
            Some(CaptureError::CaptureCancelled)
        )
    })
}

fn micros_to_millis(micros: u64) -> f64 {
    micros as f64 / 1_000.0
}

fn capture_config_path() -> Result<PathBuf> {
    let base = BaseDirs::new().context("could not determine the user configuration directory")?;
    Ok(base.config_dir().join("logilightshow/capture.toml"))
}

fn load_capture_config(path: &Path) -> Result<CaptureConfig> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(CaptureConfig::default()),
        Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
    };
    toml::from_str(&contents).with_context(|| format!("parse {}", path.display()))
}

fn save_restore_token(path: &Path, restore_token: &str) -> Result<()> {
    let parent = path
        .parent()
        .context("capture configuration path has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let contents = toml::to_string(&CaptureConfig {
        version: CAPTURE_CONFIG_VERSION,
        restore_token: Some(restore_token.to_owned()),
    })?;
    let (temporary, mut file) = (0..100_u8)
        .find_map(|attempt| {
            let suffix = if attempt == 0 {
                String::new()
            } else {
                format!("-{attempt}")
            };
            let temporary =
                parent.join(format!(".capture.toml.tmp-{}{suffix}", std::process::id()));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
            {
                Ok(file) => Some(Ok((temporary, file))),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => None,
                Err(error) => {
                    Some(Err(error).with_context(|| format!("open {}", temporary.display())))
                }
            }
        })
        .transpose()?
        .context("could not allocate a private temporary capture configuration file")?;
    let write_result = (|| -> Result<()> {
        file.write_all(contents.as_bytes())
            .with_context(|| format!("write {}", temporary.display()))?;
        file.sync_all()
            .with_context(|| format!("sync {}", temporary.display()))?;
        fs::rename(&temporary, path).with_context(|| format!("replace {}", path.display()))?;
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .with_context(|| format!("sync {}", parent.display()))?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
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
        let cli = Cli::try_parse_from(["logilightshow", "run"]).unwrap();
        assert!(matches!(cli.command, Command::Run));
    }

    #[test]
    fn calibration_cli_requires_positive_delay_and_duration() {
        let cli = Cli::try_parse_from([
            "logilightshow",
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
                "logilightshow",
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
                "logilightshow",
                "calibrate-pacing",
                "--delay-ms",
                "6",
                "--seconds",
                "0",
            ])
            .is_err()
        );
    }

    #[test]
    fn restore_token_is_atomically_saved_with_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/capture.toml");
        save_restore_token(&path, "private-token").unwrap();

        let config = load_capture_config(&path).unwrap();
        assert_eq!(config.version, CAPTURE_CONFIG_VERSION);
        assert_eq!(config.restore_token.as_deref(), Some("private-token"));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    }

    #[test]
    fn stale_temporary_file_cannot_weaken_restore_token_permissions() {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("nested");
        fs::create_dir_all(&parent).unwrap();
        let stale = parent.join(format!(".capture.toml.tmp-{}", std::process::id()));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o666)
            .open(&stale)
            .unwrap();
        fs::set_permissions(&stale, fs::Permissions::from_mode(0o644)).unwrap();
        let path = parent.join("capture.toml");

        save_restore_token(&path, "still-private").unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(stale).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }

    #[test]
    fn runtime_pipeline_failure_retries_only_after_a_capture_opened() {
        let error = CaptureError::Pipeline {
            element: "pipewiresrc".to_owned(),
            message: "remote node was destroyed".to_owned(),
            debug: String::new(),
        };

        assert!(!capture_error_is_retryable(&error, false));
        assert!(capture_error_is_retryable(&error, true));
    }

    #[test]
    fn explicit_capture_cancel_is_not_retried() {
        let error =
            anyhow::Error::new(CaptureError::CaptureCancelled).context("frame capture failed");

        assert!(!capture_error_is_retryable(
            &CaptureError::CaptureCancelled,
            true
        ));
        assert!(capture_was_cancelled(&error));
    }
}
