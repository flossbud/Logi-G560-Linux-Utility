use std::{
    fs,
    path::{Path, PathBuf},
    pin::Pin,
    time::Instant,
};

use anyhow::{Context, Result};
use futures_util::future::OptionFuture;
use tokio::{
    sync::{mpsc, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::{
    AppConfig, CaptureBackend, ConfigLoad, ConfigStore, ControllerModel, EngineMetrics,
    FileConfigStore, FrameSourceFactory, LightSink, LightUpdateStatus, LightingMode, ModelEffect,
    RecoveringFrameSource, RecoveringLightSink, RecoveryMetrics, SamplerConfig, ServiceSnapshot,
    ZoneColors, ZoneLayout,
    capture::{CaptureError, GStreamerFrameSource, GamescopeFrameSource, PortalCapture},
    config_path, run_engine_with_metrics,
    usb::{AsyncG560, G560, LibUsbTransport},
};

use super::{
    ipc::{IpcRequest, bind_listener, default_socket_path, run_accept_loop},
    protocol::{ClientCommand, RequestResult},
    proxy_sink::{ProxyCommand, ProxySink},
};

/// Runtime configuration for the persistent lighting service.
pub struct ServiceOptions {
    pub backend: CaptureBackend,
    pub socket_path: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    pub shutdown: CancellationToken,
}

impl ServiceOptions {
    pub fn new(backend: CaptureBackend) -> Self {
        Self {
            backend,
            socket_path: None,
            config_path: None,
            shutdown: CancellationToken::new(),
        }
    }
}

pub async fn run_service(options: ServiceOptions) -> Result<()> {
    let ServiceOptions {
        backend,
        socket_path,
        config_path: config_path_override,
        shutdown,
    } = options;

    let config_file = match config_path_override {
        Some(path) => path,
        None => config_path()?,
    };
    let config_store = FileConfigStore::new(config_file.clone());
    let load = config_store.load()?;
    if let ConfigLoad::Recovered { invalid_path, .. } = &load {
        warn!(
            preserved = %invalid_path.display(),
            "existing configuration was invalid; loaded safe defaults"
        );
    }
    let config = load.into_config();
    let mut model = ControllerModel::new(config)
        .context("initial configuration failed controller validation")?;

    let mut engine: Option<EngineHandle> = None;
    let socket_path = match socket_path {
        Some(path) => path,
        None => default_socket_path()?,
    };
    let listener = bind_listener(&socket_path)?;
    info!(socket = %socket_path.display(), "lighting service listening");

    let mut sink = RecoveringLightSink::new(
        || -> Result<AsyncG560<LibUsbTransport>> {
            let mut device = G560::new(LibUsbTransport::open()?);
            device.blackout()?;
            Ok(AsyncG560::new(device))
        },
        shutdown.clone(),
    );
    let sink_metrics = sink.metrics();

    let (snapshot_tx, snapshot_rx) =
        watch::channel(augment_snapshot(&model, engine.as_ref(), &sink_metrics));
    let (request_tx, mut request_rx) = mpsc::channel::<IpcRequest>(64);
    let (proxy_tx, mut proxy_rx) = mpsc::channel::<ProxyCommand>(8);

    let accept_handle = tokio::spawn(run_accept_loop(
        listener,
        request_tx,
        snapshot_rx.clone(),
        shutdown.clone(),
    ));

    if let Err(err) = sink.write(ZoneColors::BLACK).await {
        cleanup_socket(&socket_path);
        accept_handle.abort();
        let _ = accept_handle.await;
        if shutdown.is_cancelled() {
            return Ok(());
        }
        return Err(err.context("initial G560 blackout failed"));
    }
    info!("G560 opened; initial safety blackout complete");
    model.confirm_write(ZoneColors::BLACK);
    publish_snapshot(&model, engine.as_ref(), &sink_metrics, &snapshot_tx);

    match (model.config().lights_enabled, model.config().mode) {
        (true, LightingMode::Manual) => {
            let target = model.manual_target();
            apply_manual_write(
                &mut model,
                &mut sink,
                target,
                &snapshot_tx,
                &sink_metrics,
                &engine,
            )
            .await;
        }
        (true, LightingMode::ContentAware) => {
            engine = Some(start_engine(backend, &config_file, &proxy_tx, &shutdown));
            publish_snapshot(&model, engine.as_ref(), &sink_metrics, &snapshot_tx);
        }
        (false, _) => {}
    }

    let loop_result = service_loop(
        &mut model,
        &mut sink,
        &sink_metrics,
        &snapshot_tx,
        &mut request_rx,
        &mut proxy_rx,
        &mut engine,
        &config_store,
        backend,
        &config_file,
        &proxy_tx,
        &shutdown,
    )
    .await;

    debug!("service loop exited; performing shutdown");
    if let Some(handle) = engine.take() {
        handle.cancel.cancel();
        if let Err(err) = handle.task.await {
            debug!(?err, "engine task join failed during shutdown");
        }
    }
    if let Err(err) = sink.blackout().await {
        warn!(?err, "final blackout failed during shutdown");
    }
    accept_handle.abort();
    let _ = accept_handle.await;
    cleanup_socket(&socket_path);
    loop_result
}

struct EngineHandle {
    task: JoinHandle<Result<()>>,
    cancel: CancellationToken,
    metrics: EngineMetrics,
    backend: CaptureBackend,
}

fn augment_snapshot(
    model: &ControllerModel,
    engine: Option<&EngineHandle>,
    sink_metrics: &RecoveryMetrics,
) -> ServiceSnapshot {
    let mut snapshot = model.snapshot();
    snapshot.capture_backend = engine.map(|h| h.backend);
    if let Some(handle) = engine {
        let engine_snap = handle.metrics.snapshot();
        snapshot.diagnostics.captured_frames = engine_snap.captured_frames;
        snapshot.diagnostics.newest_value_replacements = engine_snap.dropped_frames;
        snapshot.diagnostics.capture_stalls = engine_snap.capture_stalls;
        let elapsed_ms = engine_snap.elapsed.as_millis() as u64;
        snapshot.diagnostics.capture_rate_millihertz = if elapsed_ms > 0 {
            engine_snap
                .captured_frames
                .saturating_mul(1_000_000)
                .checked_div(elapsed_ms)
                .unwrap_or(0)
        } else {
            0
        };
    }
    let sink_snap = sink_metrics.snapshot();
    snapshot.diagnostics.usb_recoveries = sink_snap.reopen_count;
    snapshot.diagnostics.usb_report_failures = sink_snap.usb_write_failures;
    snapshot
}

fn publish_snapshot(
    model: &ControllerModel,
    engine: Option<&EngineHandle>,
    sink_metrics: &RecoveryMetrics,
    tx: &watch::Sender<ServiceSnapshot>,
) {
    let _ = tx.send(augment_snapshot(model, engine, sink_metrics));
}

#[allow(clippy::too_many_arguments)]
async fn service_loop(
    model: &mut ControllerModel,
    sink: &mut RecoveringLightSink<
        impl FnMut() -> Result<AsyncG560<LibUsbTransport>> + Send,
        AsyncG560<LibUsbTransport>,
    >,
    sink_metrics: &RecoveryMetrics,
    snapshot_tx: &watch::Sender<ServiceSnapshot>,
    request_rx: &mut mpsc::Receiver<IpcRequest>,
    proxy_rx: &mut mpsc::Receiver<ProxyCommand>,
    engine: &mut Option<EngineHandle>,
    config_store: &FileConfigStore,
    backend: CaptureBackend,
    config_path: &Path,
    proxy_tx: &mpsc::Sender<ProxyCommand>,
    shutdown: &CancellationToken,
) -> Result<()> {
    loop {
        let engine_join: OptionFuture<_> = engine
            .as_mut()
            .map(|handle| Pin::new(&mut handle.task))
            .into();

        tokio::select! {
            biased;
            _ = shutdown.cancelled() => return Ok(()),
            proxy = proxy_rx.recv() => {
                let Some(command) = proxy else { continue };
                dispatch_proxy_command(command, model, sink, sink_metrics, snapshot_tx, engine.as_ref()).await;
            }
            request = request_rx.recv() => {
                let Some(request) = request else { continue };
                let response = handle_client_command(
                    request.request.command,
                    model,
                    sink,
                    sink_metrics,
                    snapshot_tx,
                    engine,
                    config_store,
                    backend,
                    config_path,
                    proxy_tx,
                    shutdown,
                )
                .await;
                let _ = request.response.send(response);
            }
            Some(join_result) = engine_join => {
                *engine = None;
                match join_result {
                    Ok(Ok(())) => debug!("content-aware engine finished normally"),
                    Ok(Err(err)) => {
                        if !capture_was_cancelled(&err) {
                            warn!(?err, "content-aware engine exited with error");
                            if matches!(model.config().mode, LightingMode::ContentAware)
                                && model.config().lights_enabled
                                && !shutdown.is_cancelled()
                            {
                                let _ = sink.blackout().await;
                                model.record_failure();
                                publish_snapshot(model, engine.as_ref(), sink_metrics, snapshot_tx);
                            }
                        }
                    }
                    Err(join_err) => {
                        warn!(?join_err, "content-aware engine task panicked");
                        if matches!(model.config().mode, LightingMode::ContentAware)
                            && model.config().lights_enabled
                            && !shutdown.is_cancelled()
                        {
                            let _ = sink.blackout().await;
                            model.record_failure();
                            publish_snapshot(model, engine.as_ref(), sink_metrics, snapshot_tx);
                        }
                    }
                }
                publish_snapshot(model, engine.as_ref(), sink_metrics, snapshot_tx);
            }
        }
    }
}

async fn dispatch_proxy_command<F>(
    command: ProxyCommand,
    model: &mut ControllerModel,
    sink: &mut RecoveringLightSink<F, AsyncG560<LibUsbTransport>>,
    sink_metrics: &RecoveryMetrics,
    snapshot_tx: &watch::Sender<ServiceSnapshot>,
    engine: Option<&EngineHandle>,
) where
    F: FnMut() -> Result<AsyncG560<LibUsbTransport>> + Send,
{
    match command {
        ProxyCommand::Write(write) => {
            let captured_at = write.captured_at.unwrap_or_else(Instant::now);
            let result = sink.write_update(write.colors, captured_at).await;
            match &result {
                Ok(LightUpdateStatus::Rendered) | Ok(LightUpdateStatus::Unchanged) => {
                    model.confirm_write(write.colors);
                    publish_snapshot(model, engine, sink_metrics, snapshot_tx);
                }
                Ok(LightUpdateStatus::Expired) => {}
                Err(_) => {
                    model.record_failure();
                    publish_snapshot(model, engine, sink_metrics, snapshot_tx);
                }
            }
            let _ = write.response.send(result);
        }
        ProxyCommand::Blackout(blackout) => {
            let result = sink.blackout().await;
            if result.is_ok() {
                model.confirm_write(ZoneColors::BLACK);
                publish_snapshot(model, engine, sink_metrics, snapshot_tx);
            }
            let _ = blackout.response.send(result);
        }
    }
}

async fn apply_manual_write<F>(
    model: &mut ControllerModel,
    sink: &mut RecoveringLightSink<F, AsyncG560<LibUsbTransport>>,
    colors: ZoneColors,
    snapshot_tx: &watch::Sender<ServiceSnapshot>,
    sink_metrics: &RecoveryMetrics,
    engine: &Option<EngineHandle>,
) where
    F: FnMut() -> Result<AsyncG560<LibUsbTransport>> + Send,
{
    model.mark_pending();
    publish_snapshot(model, engine.as_ref(), sink_metrics, snapshot_tx);
    match sink.write(colors).await {
        Ok(()) => {
            model.confirm_write(colors);
        }
        Err(err) => {
            warn!(?err, "manual write failed");
            model.record_failure();
        }
    }
    publish_snapshot(model, engine.as_ref(), sink_metrics, snapshot_tx);
}

#[allow(clippy::too_many_arguments)]
async fn handle_client_command<F>(
    command: ClientCommand,
    model: &mut ControllerModel,
    sink: &mut RecoveringLightSink<F, AsyncG560<LibUsbTransport>>,
    sink_metrics: &RecoveryMetrics,
    snapshot_tx: &watch::Sender<ServiceSnapshot>,
    engine: &mut Option<EngineHandle>,
    config_store: &FileConfigStore,
    backend: CaptureBackend,
    config_path: &Path,
    proxy_tx: &mpsc::Sender<ProxyCommand>,
    shutdown: &CancellationToken,
) -> RequestResult
where
    F: FnMut() -> Result<AsyncG560<LibUsbTransport>> + Send,
{
    let effect = match command {
        ClientCommand::GetSnapshot => {
            return RequestResult::Ok {
                snapshot: augment_snapshot(model, engine.as_ref(), sink_metrics),
            };
        }
        ClientCommand::SetLightsEnabled { enabled } => model.set_lights_enabled(enabled),
        ClientCommand::SetMode { mode } => model.set_mode(mode),
        ClientCommand::MarkSetupComplete => model.mark_setup_complete(),
        ClientCommand::SetManualZones { updates } => match model.apply_manual_updates(&updates) {
            Ok(effect) => effect,
            Err(err) => return RequestResult::Err { error: err },
        },
        ClientCommand::ChooseDesktopDisplay => {
            if backend != CaptureBackend::DesktopPortal {
                return RequestResult::Ok {
                    snapshot: augment_snapshot(model, engine.as_ref(), sink_metrics),
                };
            }
            if let Err(err) = clear_restore_token(config_store) {
                warn!(?err, "failed to clear portal restore token");
            }
            restart_engine(engine, backend, config_path, proxy_tx, shutdown).await;
            publish_snapshot(model, engine.as_ref(), sink_metrics, snapshot_tx);
            return RequestResult::Ok {
                snapshot: augment_snapshot(model, engine.as_ref(), sink_metrics),
            };
        }
        ClientCommand::RestartCapture => {
            restart_engine(engine, backend, config_path, proxy_tx, shutdown).await;
            publish_snapshot(model, engine.as_ref(), sink_metrics, snapshot_tx);
            return RequestResult::Ok {
                snapshot: augment_snapshot(model, engine.as_ref(), sink_metrics),
            };
        }
    };

    if let Err(err) = config_store.save(model.config()) {
        warn!(?err, "failed to persist configuration change");
    }
    publish_snapshot(model, engine.as_ref(), sink_metrics, snapshot_tx);

    match effect {
        ModelEffect::NoWrite => {}
        ModelEffect::Write(colors) => {
            apply_manual_write(model, sink, colors, snapshot_tx, sink_metrics, engine).await;
        }
        ModelEffect::StartContentAware => {
            if engine.is_none() {
                *engine = Some(start_engine(backend, config_path, proxy_tx, shutdown));
                publish_snapshot(model, engine.as_ref(), sink_metrics, snapshot_tx);
            }
        }
        ModelEffect::StopContentAwareAndWrite(colors) => {
            if let Some(handle) = engine.take() {
                handle.cancel.cancel();
                if let Err(err) = handle.task.await {
                    debug!(?err, "engine task join failed during mode change");
                }
            }
            apply_manual_write(model, sink, colors, snapshot_tx, sink_metrics, engine).await;
        }
        ModelEffect::PriorityBlackout => {
            if let Some(handle) = engine.take() {
                handle.cancel.cancel();
                if let Err(err) = handle.task.await {
                    debug!(?err, "engine task join failed during blackout");
                }
            }
            model.mark_pending();
            publish_snapshot(model, engine.as_ref(), sink_metrics, snapshot_tx);
            match sink.blackout().await {
                Ok(()) => {
                    model.confirm_write(ZoneColors::BLACK);
                }
                Err(err) => {
                    warn!(?err, "priority blackout write failed");
                    model.record_failure();
                }
            }
            publish_snapshot(model, engine.as_ref(), sink_metrics, snapshot_tx);
        }
    }

    RequestResult::Ok {
        snapshot: augment_snapshot(model, engine.as_ref(), sink_metrics),
    }
}

async fn restart_engine(
    engine: &mut Option<EngineHandle>,
    backend: CaptureBackend,
    config_path: &Path,
    proxy_tx: &mpsc::Sender<ProxyCommand>,
    shutdown: &CancellationToken,
) {
    if let Some(handle) = engine.take() {
        handle.cancel.cancel();
        if let Err(err) = handle.task.await {
            debug!(?err, "engine task join failed during restart");
        }
    }
    if shutdown.is_cancelled() {
        return;
    }
    *engine = Some(start_engine(backend, config_path, proxy_tx, shutdown));
}

fn clear_restore_token(store: &FileConfigStore) -> Result<()> {
    let load = store.load()?;
    let mut config = load.into_config();
    if config.restore_token.is_some() {
        config.restore_token = None;
        store.save(&config)?;
    }
    Ok(())
}

fn start_engine(
    backend: CaptureBackend,
    config_path: &Path,
    proxy_tx: &mpsc::Sender<ProxyCommand>,
    shutdown: &CancellationToken,
) -> EngineHandle {
    let metrics = EngineMetrics::new();
    let cancel = shutdown.child_token();
    let engine_cancel = cancel.clone();
    let proxy = ProxySink::new(proxy_tx.clone());
    let config_path = config_path.to_owned();
    let engine_metrics = metrics.clone();
    let task = tokio::spawn(async move {
        run_engine_backend(backend, config_path, proxy, engine_cancel, engine_metrics).await
    });
    EngineHandle {
        task,
        cancel,
        metrics,
        backend,
    }
}

async fn run_engine_backend(
    backend: CaptureBackend,
    config_path: PathBuf,
    proxy: ProxySink,
    cancel: CancellationToken,
    metrics: EngineMetrics,
) -> Result<()> {
    match backend {
        CaptureBackend::DesktopPortal => {
            let source = RecoveringFrameSource::new(PortalFrameSourceFactory {
                config_store: FileConfigStore::new(config_path),
                has_opened: false,
            });
            drive_engine(source, proxy, cancel, metrics).await
        }
        CaptureBackend::Gamescope => {
            let source = RecoveringFrameSource::new(GamescopeFrameSourceFactory);
            drive_engine(source, proxy, cancel, metrics).await
        }
    }
}

async fn drive_engine<F>(
    source: RecoveringFrameSource<F>,
    proxy: ProxySink,
    cancel: CancellationToken,
    metrics: EngineMetrics,
) -> Result<()>
where
    F: FrameSourceFactory + 'static,
    F::Source: 'static,
{
    run_engine_with_metrics(
        source,
        proxy,
        ZoneLayout::g560_default(),
        SamplerConfig::default(),
        cancel.cancelled_owned(),
        metrics,
    )
    .await
    .map(|_| ())
}

struct PortalFrameSourceFactory {
    config_store: FileConfigStore,
    has_opened: bool,
}

#[async_trait::async_trait]
impl FrameSourceFactory for PortalFrameSourceFactory {
    type Source = GStreamerFrameSource;

    async fn open(&mut self) -> Result<Self::Source> {
        let restore_token = self
            .config_store
            .load()
            .ok()
            .and_then(|load| load.into_config().restore_token);
        let grant = PortalCapture::open(restore_token).await?;
        if let Some(token) = grant.restore_token.as_deref()
            && let Ok(load) = self.config_store.load()
        {
            let mut config: AppConfig = load.into_config();
            if config.restore_token.as_deref() != Some(token) {
                config.restore_token = Some(token.to_owned());
                if let Err(err) = self.config_store.save(&config) {
                    warn!(?err, "failed to persist portal restore token");
                }
            }
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

struct GamescopeFrameSourceFactory;

#[async_trait::async_trait]
impl FrameSourceFactory for GamescopeFrameSourceFactory {
    type Source = GamescopeFrameSource;

    async fn open(&mut self) -> Result<Self::Source> {
        let source = GamescopeFrameSource::open()
            .await
            .map_err(anyhow::Error::from)?;
        Ok(source)
    }

    fn should_retry(&self, error: &anyhow::Error) -> bool {
        !capture_was_cancelled(error)
    }
}

fn capture_error_is_retryable(error: &CaptureError, has_opened: bool) -> bool {
    match error {
        CaptureError::Portal { .. } => true,
        CaptureError::Pipeline { .. } | CaptureError::Shutdown { .. } => has_opened,
        CaptureError::GStreamer { .. } | CaptureError::PipeWire { .. } => has_opened,
        CaptureError::Cleanup { primary, .. } => capture_error_is_retryable(primary, has_opened),
        CaptureError::CaptureCancelled
        | CaptureError::UnexpectedStreamCount { .. }
        | CaptureError::MissingSampleData { .. }
        | CaptureError::UnsupportedCaps { .. }
        | CaptureError::InvalidFrameLayout { .. }
        | CaptureError::EndOfStream => false,
    }
}

pub fn capture_was_cancelled(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<CaptureError>(),
            Some(CaptureError::CaptureCancelled)
        )
    })
}

fn cleanup_socket(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => debug!(path = %path.display(), "removed service socket on shutdown"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => warn!(?err, path = %path.display(), "failed to remove service socket"),
    }
}
