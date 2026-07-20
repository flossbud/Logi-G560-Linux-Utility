use std::{
    array,
    future::Future,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use crate::{
    DEFAULT_TRANSITION_DURATION, LatestReceiver, Rgb8, RgbFrame, SamplerConfig,
    TransitionController, ZoneColors, ZoneLayout, ZoneMasks, latest_channel, sampler::sample_zones,
};

#[async_trait::async_trait]
pub trait FrameSource: Send {
    async fn next_frame(&mut self) -> Result<Option<RgbFrame>>;

    async fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait FrameSourceFactory: Send {
    type Source: FrameSource;

    async fn open(&mut self) -> Result<Self::Source>;

    fn should_retry(&self, error: &anyhow::Error) -> bool;
}

#[async_trait::async_trait]
pub trait LightSink: Send {
    async fn write(&mut self, colors: ZoneColors) -> Result<()>;

    async fn write_update(
        &mut self,
        colors: ZoneColors,
        _captured_at: Instant,
    ) -> Result<LightUpdateStatus> {
        self.write(colors).await?;
        Ok(LightUpdateStatus::Rendered)
    }

    async fn blackout(&mut self) -> Result<()> {
        self.write(ZoneColors::BLACK).await
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LightUpdateStatus {
    Rendered,
    Unchanged,
    Expired,
}

pub const CAPTURE_STALL_TIMEOUT: Duration = Duration::from_millis(500);
const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(250);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(5);
const MAX_CONSECUTIVE_WRITE_FAILURES: usize = 3;
const LIGHT_UPDATE_INTERVAL: Duration = Duration::from_millis(20);
const NORMAL_FADE_TO_BLACK_DURATION: Duration = Duration::from_millis(200);
const SAFETY_BLACKOUT_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, thiserror::Error)]
#[error("recovery cancelled")]
struct RecoveryCancelled;

#[derive(Debug, Default)]
struct RetryBackoff {
    failures: u32,
}

impl RetryBackoff {
    fn reset(&mut self) {
        self.failures = 0;
    }

    fn next_delay(&mut self) -> Duration {
        let multiplier = 1_u32.checked_shl(self.failures.min(31)).unwrap_or(u32::MAX);
        self.failures = self.failures.saturating_add(1);
        INITIAL_RETRY_DELAY
            .checked_mul(multiplier)
            .unwrap_or(MAX_RETRY_DELAY)
            .min(MAX_RETRY_DELAY)
    }

    async fn wait(&mut self, cancellation: &CancellationToken) -> Result<()> {
        let delay = self.next_delay();
        tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(RecoveryCancelled.into()),
            () = tokio::time::sleep(delay) => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CaptureRecoverySnapshot {
    pub open_failures: u64,
    pub stream_failures: u64,
    pub successful_reopens: u64,
    pub shutdown_failures: u64,
}

#[derive(Default)]
struct CaptureRecoveryCounters {
    open_failures: AtomicU64,
    stream_failures: AtomicU64,
    successful_reopens: AtomicU64,
    shutdown_failures: AtomicU64,
}

#[derive(Clone, Default)]
pub struct CaptureRecoveryMetrics(Arc<CaptureRecoveryCounters>);

impl CaptureRecoveryMetrics {
    pub fn snapshot(&self) -> CaptureRecoverySnapshot {
        CaptureRecoverySnapshot {
            open_failures: self.0.open_failures.load(Ordering::Relaxed),
            stream_failures: self.0.stream_failures.load(Ordering::Relaxed),
            successful_reopens: self.0.successful_reopens.load(Ordering::Relaxed),
            shutdown_failures: self.0.shutdown_failures.load(Ordering::Relaxed),
        }
    }
}

pub struct RecoveringFrameSource<F>
where
    F: FrameSourceFactory,
{
    factory: F,
    current: Option<F::Source>,
    backoff: RetryBackoff,
    retry_cancellation: CancellationToken,
    pending_reopen: bool,
    metrics: CaptureRecoveryMetrics,
}

impl<F> RecoveringFrameSource<F>
where
    F: FrameSourceFactory,
{
    pub fn new(factory: F) -> Self {
        Self {
            factory,
            current: None,
            backoff: RetryBackoff::default(),
            retry_cancellation: CancellationToken::new(),
            pending_reopen: false,
            metrics: CaptureRecoveryMetrics::default(),
        }
    }

    pub fn metrics(&self) -> CaptureRecoveryMetrics {
        self.metrics.clone()
    }

    async fn connect(&mut self) -> Result<()> {
        while self.current.is_none() {
            match self.factory.open().await {
                Ok(source) => {
                    if self.pending_reopen {
                        self.metrics
                            .0
                            .successful_reopens
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    self.current = Some(source);
                }
                Err(error) if self.factory.should_retry(&error) => {
                    self.pending_reopen = true;
                    self.metrics.0.open_failures.fetch_add(1, Ordering::Relaxed);
                    self.backoff.wait(&self.retry_cancellation).await?;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl<F> FrameSource for RecoveringFrameSource<F>
where
    F: FrameSourceFactory,
{
    async fn next_frame(&mut self) -> Result<Option<RgbFrame>> {
        loop {
            self.connect().await?;
            let result = self
                .current
                .as_mut()
                .expect("connect installs a frame source")
                .next_frame()
                .await;
            match result {
                Ok(Some(frame)) => {
                    self.backoff.reset();
                    return Ok(Some(frame));
                }
                Ok(None) => return Ok(None),
                Err(error) if self.factory.should_retry(&error) => {
                    self.metrics
                        .0
                        .stream_failures
                        .fetch_add(1, Ordering::Relaxed);
                    let mut failed = self.current.take().expect("connected source exists");
                    if failed.shutdown().await.is_err() {
                        self.metrics
                            .0
                            .shutdown_failures
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    self.pending_reopen = true;
                    self.backoff.wait(&self.retry_cancellation).await?;
                }
                Err(error) => {
                    let mut failed = self.current.take().expect("connected source exists");
                    return match failed.shutdown().await {
                        Ok(()) => Err(error),
                        Err(shutdown) => Err(error
                            .context(format!("frame source shutdown also failed: {shutdown:#}"))),
                    };
                }
            }
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        self.retry_cancellation.cancel();
        let Some(mut current) = self.current.take() else {
            return Ok(());
        };
        current.shutdown().await
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RecoverySnapshot {
    pub open_failures: u64,
    pub usb_write_failures: u64,
    pub reopen_count: u64,
    pub blackout_failures: u64,
}

#[derive(Default)]
struct RecoveryCounters {
    open_failures: AtomicU64,
    usb_write_failures: AtomicU64,
    reopen_count: AtomicU64,
    blackout_failures: AtomicU64,
}

#[derive(Clone, Default)]
pub struct RecoveryMetrics(Arc<RecoveryCounters>);

impl RecoveryMetrics {
    pub fn snapshot(&self) -> RecoverySnapshot {
        RecoverySnapshot {
            open_failures: self.0.open_failures.load(Ordering::Relaxed),
            usb_write_failures: self.0.usb_write_failures.load(Ordering::Relaxed),
            reopen_count: self.0.reopen_count.load(Ordering::Relaxed),
            blackout_failures: self.0.blackout_failures.load(Ordering::Relaxed),
        }
    }
}

pub struct RecoveringLightSink<F, L> {
    factory: F,
    current: Option<L>,
    cancellation: CancellationToken,
    backoff: RetryBackoff,
    consecutive_write_failures: usize,
    metrics: RecoveryMetrics,
}

impl<F, L> RecoveringLightSink<F, L>
where
    F: FnMut() -> Result<L> + Send,
    L: LightSink,
{
    pub fn new(factory: F, cancellation: CancellationToken) -> Self {
        Self {
            factory,
            current: None,
            cancellation,
            backoff: RetryBackoff::default(),
            consecutive_write_failures: 0,
            metrics: RecoveryMetrics::default(),
        }
    }

    pub fn metrics(&self) -> RecoveryMetrics {
        self.metrics.clone()
    }

    async fn connect(&mut self) -> Result<()> {
        while self.current.is_none() {
            if self.cancellation.is_cancelled() {
                return Err(RecoveryCancelled.into());
            }
            match (self.factory)() {
                Ok(sink) => self.current = Some(sink),
                Err(_) => {
                    self.metrics.0.open_failures.fetch_add(1, Ordering::Relaxed);
                    self.backoff.wait(&self.cancellation).await?;
                }
            }
        }
        Ok(())
    }

    async fn write_recovering(
        &mut self,
        colors: ZoneColors,
        captured_at: Option<Instant>,
    ) -> Result<LightUpdateStatus> {
        loop {
            self.connect().await?;
            let fresh = captured_at
                .map(|captured_at| captured_at.elapsed() < CAPTURE_STALL_TIMEOUT)
                .unwrap_or(true);
            let current = self.current.as_mut().expect("connect installs a sink");
            let result = match (fresh, captured_at) {
                (true, Some(captured_at)) => current.write_update(colors, captured_at).await,
                (true, None) => current
                    .write(colors)
                    .await
                    .map(|()| LightUpdateStatus::Rendered),
                (false, _) => current
                    .write(ZoneColors::BLACK)
                    .await
                    .map(|()| LightUpdateStatus::Expired),
            };
            match result {
                Ok(status) => {
                    self.consecutive_write_failures = 0;
                    self.backoff.reset();
                    return Ok(status);
                }
                Err(_) => {
                    self.metrics
                        .0
                        .usb_write_failures
                        .fetch_add(1, Ordering::Relaxed);
                    self.consecutive_write_failures += 1;
                    if self.consecutive_write_failures >= MAX_CONSECUTIVE_WRITE_FAILURES {
                        let mut failed = self.current.take().expect("connected sink exists");
                        if failed.blackout().await.is_err() {
                            self.metrics
                                .0
                                .blackout_failures
                                .fetch_add(1, Ordering::Relaxed);
                        }
                        self.metrics.0.reopen_count.fetch_add(1, Ordering::Relaxed);
                        self.consecutive_write_failures = 0;
                    }
                    self.backoff.wait(&self.cancellation).await?;
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl<F, L> LightSink for RecoveringLightSink<F, L>
where
    F: FnMut() -> Result<L> + Send,
    L: LightSink,
{
    async fn write(&mut self, colors: ZoneColors) -> Result<()> {
        self.write_recovering(colors, None).await.map(|_| ())
    }

    async fn write_update(
        &mut self,
        colors: ZoneColors,
        captured_at: Instant,
    ) -> Result<LightUpdateStatus> {
        self.write_recovering(colors, Some(captured_at)).await
    }

    async fn blackout(&mut self) -> Result<()> {
        let Some(sink) = &mut self.current else {
            return Ok(());
        };
        let result = sink.blackout().await;
        if result.is_err() {
            self.metrics
                .0
                .blackout_failures
                .fetch_add(1, Ordering::Relaxed);
        }
        result
    }
}

#[derive(Debug)]
pub struct EngineStats {
    pub captured_frames: u64,
    pub rendered_updates: u64,
    pub dropped_frames: u64,
    pub capture_stalls: u64,
    pub capture_to_write: hdrhistogram::Histogram<u64>,
}

struct EngineMetricCounters {
    started_at: Instant,
    captured_frames: AtomicU64,
    rendered_updates: AtomicU64,
    dropped_frames: AtomicU64,
    capture_stalls: AtomicU64,
    capture_to_write: Mutex<hdrhistogram::Histogram<u64>>,
}

#[derive(Clone)]
pub struct EngineMetrics(Arc<EngineMetricCounters>);

#[derive(Clone, Copy, Debug)]
pub struct EngineSnapshot {
    pub elapsed: Duration,
    pub captured_frames: u64,
    pub rendered_updates: u64,
    pub dropped_frames: u64,
    pub capture_stalls: u64,
    pub capture_to_write_p50_us: u64,
    pub capture_to_write_p95_us: u64,
    pub capture_to_write_p99_us: u64,
}

impl Default for EngineMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl EngineMetrics {
    pub fn new() -> Self {
        Self(Arc::new(EngineMetricCounters {
            started_at: Instant::now(),
            captured_frames: AtomicU64::new(0),
            rendered_updates: AtomicU64::new(0),
            dropped_frames: AtomicU64::new(0),
            capture_stalls: AtomicU64::new(0),
            capture_to_write: Mutex::new(
                hdrhistogram::Histogram::<u64>::new(3).expect("fixed histogram precision is valid"),
            ),
        }))
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        let histogram = self
            .0
            .capture_to_write
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        EngineSnapshot {
            elapsed: self.0.started_at.elapsed(),
            captured_frames: self.0.captured_frames.load(Ordering::Relaxed),
            rendered_updates: self.0.rendered_updates.load(Ordering::Relaxed),
            dropped_frames: self.0.dropped_frames.load(Ordering::Relaxed),
            capture_stalls: self.0.capture_stalls.load(Ordering::Relaxed),
            capture_to_write_p50_us: histogram.value_at_quantile(0.50),
            capture_to_write_p95_us: histogram.value_at_quantile(0.95),
            capture_to_write_p99_us: histogram.value_at_quantile(0.99),
        }
    }

    fn captured(&self, replaced: bool) {
        self.0.captured_frames.fetch_add(1, Ordering::Relaxed);
        if replaced {
            self.dropped();
        }
    }

    fn rendered(&self, latency_us: u64) -> Result<()> {
        self.0.rendered_updates.fetch_add(1, Ordering::Relaxed);
        self.0
            .capture_to_write
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .record(latency_us)
            .context("live latency record failed")
    }

    fn dropped(&self) {
        self.0.dropped_frames.fetch_add(1, Ordering::Relaxed);
    }

    fn stalled(&self) {
        self.0.capture_stalls.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Clone)]
struct CapturedFrame {
    captured_at: Instant,
    frame: RgbFrame,
    capture_generation: u64,
}

#[derive(Clone)]
struct SampledUpdate {
    captured_at: Instant,
    colors: ZoneColors,
    capture_generation: u64,
}

struct SafetyBlackout {
    resume_generation: u64,
    acknowledged: oneshot::Sender<std::result::Result<(), String>>,
}

struct LightingWriterState {
    displayed: ZoneColors,
    transition: TransitionController,
    transition_active: bool,
    current_update: Option<SampledUpdate>,
    current_update_rendered: bool,
    accepted_generation: u64,
    next_write: Option<tokio::time::Instant>,
}

impl LightingWriterState {
    fn new() -> Self {
        Self {
            displayed: ZoneColors::BLACK,
            transition: TransitionController::new(ZoneColors::BLACK, DEFAULT_TRANSITION_DURATION),
            transition_active: false,
            current_update: None,
            current_update_rendered: false,
            accepted_generation: 0,
            next_write: None,
        }
    }

    /// Accepts a current-generation update and reports whether it displaced
    /// an update that had never reached the hardware.
    fn accept_update(
        &mut self,
        update: SampledUpdate,
        now: tokio::time::Instant,
        preserve_write_deadline: bool,
    ) -> Option<bool> {
        if update.capture_generation != self.accepted_generation {
            return None;
        }
        let dropped_unrendered = self.current_update.is_some() && !self.current_update_rendered;
        self.transition = TransitionController::new(self.displayed, DEFAULT_TRANSITION_DURATION);
        self.transition.retarget_with_zone_durations(
            update.colors,
            now.into_std(),
            normal_transition_durations(self.displayed, update.colors),
        );
        self.current_update = Some(update);
        self.current_update_rendered = false;
        self.transition_active = true;
        if !preserve_write_deadline || self.next_write.is_none() {
            self.next_write = Some(now + LIGHT_UPDATE_INTERVAL);
        }
        Some(dropped_unrendered)
    }

    fn prepare_safety(&mut self, resume_generation: u64) -> bool {
        let dropped_unrendered = self.current_update.is_some() && !self.current_update_rendered;
        self.accepted_generation = resume_generation;
        self.current_update = None;
        self.current_update_rendered = false;
        self.transition = TransitionController::new(ZoneColors::BLACK, DEFAULT_TRANSITION_DURATION);
        self.transition_active = false;
        self.next_write = None;
        dropped_unrendered
    }

    fn mark_displayed(&mut self, colors: ZoneColors) {
        self.displayed = colors;
        self.current_update_rendered = true;
    }

    fn clear_as_dropped(&mut self) -> bool {
        let dropped_unrendered = self.current_update.is_some() && !self.current_update_rendered;
        self.current_update = None;
        self.current_update_rendered = false;
        self.transition_active = false;
        self.next_write = None;
        dropped_unrendered
    }
}

fn normal_transition_durations(displayed: ZoneColors, target: ZoneColors) -> [Duration; 4] {
    array::from_fn(|index| {
        if displayed.0[index] != Rgb8::BLACK && target.0[index] == Rgb8::BLACK {
            NORMAL_FADE_TO_BLACK_DURATION
        } else {
            DEFAULT_TRANSITION_DURATION
        }
    })
}

async fn request_safety_blackout(
    sender: &mpsc::UnboundedSender<SafetyBlackout>,
    capture_generation: &AtomicU64,
) -> Result<()> {
    let previous_generation = capture_generation
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |generation| {
            Some(if generation % 2 == 0 {
                generation.wrapping_add(1)
            } else {
                generation
            })
        })
        .expect("capture generation update is infallible");
    let blackout_generation = if previous_generation % 2 == 0 {
        previous_generation.wrapping_add(1)
    } else {
        previous_generation
    };
    debug_assert_eq!(blackout_generation % 2, 1);
    let resume_generation = blackout_generation.wrapping_add(1);
    let (acknowledged, completion) = oneshot::channel();
    sender
        .send(SafetyBlackout {
            resume_generation,
            acknowledged,
        })
        .map_err(|_| anyhow!("lighting writer stopped before safety blackout"))?;
    let result = completion
        .await
        .map_err(|_| anyhow!("lighting writer stopped before acknowledging safety blackout"))?
        .map_err(|error| anyhow!(error));
    if result.is_ok() {
        capture_generation.store(resume_generation, Ordering::SeqCst);
    }
    result
}

enum WriterWake {
    Safety(Option<SafetyBlackout>),
    WriteDeadline,
    Update(Option<SampledUpdate>),
}

async fn next_writer_wake(
    safety_receiver: &mut mpsc::UnboundedReceiver<SafetyBlackout>,
    safety_open: bool,
    update_receiver: &mut LatestReceiver<SampledUpdate>,
    updates_open: bool,
    next_write: Option<tokio::time::Instant>,
) -> WriterWake {
    let deadline = next_write.unwrap_or_else(tokio::time::Instant::now);
    tokio::select! {
        biased;
        command = safety_receiver.recv(), if safety_open => WriterWake::Safety(command),
        () = tokio::time::sleep_until(deadline), if next_write.is_some() => {
            WriterWake::WriteDeadline
        }
        update = update_receiver.recv(), if updates_open => WriterWake::Update(update),
    }
}

#[derive(Default)]
struct ZoneMaskCache {
    current: Option<Arc<ZoneMasks>>,
}

impl ZoneMaskCache {
    fn get_or_compile(
        &mut self,
        layout: &ZoneLayout,
        width: usize,
        height: usize,
    ) -> Result<Arc<ZoneMasks>> {
        let dimensions_changed = self
            .current
            .as_ref()
            .is_none_or(|masks| masks.width() != width || masks.height() != height);
        if dimensions_changed {
            self.current = Some(Arc::new(ZoneMasks::compile(layout, width, height)?));
        }

        Ok(self
            .current
            .as_ref()
            .expect("the requested dimensions have compiled masks")
            .clone())
    }
}

pub async fn run_engine<S, L, C>(
    source: S,
    sink: L,
    layout: ZoneLayout,
    config: SamplerConfig,
    cancellation: C,
) -> Result<EngineStats>
where
    S: FrameSource + 'static,
    L: LightSink + 'static,
    C: Future<Output = ()> + Send,
{
    run_engine_with_sampler_and_metrics(
        source,
        sink,
        layout,
        config,
        cancellation,
        sample_zones,
        EngineMetrics::new(),
    )
    .await
}

pub async fn run_engine_with_metrics<S, L, C>(
    source: S,
    sink: L,
    layout: ZoneLayout,
    config: SamplerConfig,
    cancellation: C,
    metrics: EngineMetrics,
) -> Result<EngineStats>
where
    S: FrameSource + 'static,
    L: LightSink + 'static,
    C: Future<Output = ()> + Send,
{
    run_engine_with_sampler_and_metrics(
        source,
        sink,
        layout,
        config,
        cancellation,
        sample_zones,
        metrics,
    )
    .await
}

#[cfg(test)]
async fn run_engine_with_sampler<S, L, C, F>(
    source: S,
    sink: L,
    layout: ZoneLayout,
    config: SamplerConfig,
    cancellation: C,
    sampler: F,
) -> Result<EngineStats>
where
    S: FrameSource + 'static,
    L: LightSink + 'static,
    C: Future<Output = ()> + Send,
    F: Fn(&RgbFrame, &ZoneMasks, SamplerConfig) -> ZoneColors + Send + Sync + 'static,
{
    run_engine_with_sampler_and_metrics(
        source,
        sink,
        layout,
        config,
        cancellation,
        sampler,
        EngineMetrics::new(),
    )
    .await
}

async fn run_engine_with_sampler_and_metrics<S, L, C, F>(
    mut source: S,
    mut sink: L,
    layout: ZoneLayout,
    config: SamplerConfig,
    cancellation: C,
    sampler: F,
    metrics: EngineMetrics,
) -> Result<EngineStats>
where
    S: FrameSource + 'static,
    L: LightSink + 'static,
    C: Future<Output = ()> + Send,
    F: Fn(&RgbFrame, &ZoneMasks, SamplerConfig) -> ZoneColors + Send + Sync + 'static,
{
    let (cancel_sender, cancel_receiver) = watch::channel(false);
    let (frame_sender, mut frame_receiver) = latest_channel::<CapturedFrame>();
    let capture_generation = Arc::new(AtomicU64::new(0));
    let capture_task_generation = capture_generation.clone();
    let capture_cancel = cancel_receiver.clone();
    let capture_cancel_sender = cancel_sender.clone();
    let capture_metrics = metrics.clone();
    let capture = tokio::spawn(async move {
        let mut captured = 0_u64;
        let mut replacements = 0_u64;
        let mut cancel = capture_cancel;
        let capture_result: Result<()> = loop {
            let next = tokio::select! {
                biased;
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() { break Ok(()); }
                    continue;
                }
                next = source.next_frame() => next,
            };
            match next {
                Ok(Some(frame)) => {
                    captured += 1;
                    match frame_sender.send(CapturedFrame {
                        captured_at: Instant::now(),
                        frame,
                        capture_generation: capture_task_generation.load(Ordering::SeqCst),
                    }) {
                        Ok(replaced) => {
                            replacements += u64::from(replaced);
                            capture_metrics.captured(replaced);
                        }
                        Err(_) => break Ok(()),
                    }
                }
                Ok(None) => break Ok(()),
                Err(error) => {
                    let _ = capture_cancel_sender.send(true);
                    break Err(error);
                }
            }
        };
        let shutdown_result = source
            .shutdown()
            .await
            .context("frame source shutdown failed");
        if shutdown_result.is_err() {
            let _ = capture_cancel_sender.send(true);
        }
        match (capture_result, shutdown_result) {
            (Ok(()), Ok(())) => Ok((captured, replacements)),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Err(primary), Err(shutdown)) => Err(anyhow!(
                "{primary:#}; frame source shutdown also failed: {shutdown:#}"
            )),
        }
    });

    let (update_sender, mut update_receiver) = latest_channel::<SampledUpdate>();
    let (safety_sender, mut safety_receiver) = mpsc::unbounded_channel::<SafetyBlackout>();
    let writer_metrics = metrics.clone();
    let writer_cancel_sender = cancel_sender.clone();
    let writer = tokio::spawn(async move {
        let mut rendered = 0_u64;
        let mut stale_updates = 0_u64;
        let mut histogram = hdrhistogram::Histogram::<u64>::new(3)?;
        let mut write_error = None;
        let mut state = LightingWriterState::new();
        let mut updates_open = true;
        let mut safety_open = true;
        let mut pending_safety = None::<SafetyBlackout>;

        'writer: while updates_open || safety_open || state.transition_active {
            if let Some(command) = pending_safety.take() {
                if state.prepare_safety(command.resume_generation) {
                    stale_updates += 1;
                    writer_metrics.dropped();
                }
                let blackout = tokio::time::timeout(SAFETY_BLACKOUT_TIMEOUT, sink.blackout()).await;
                match blackout {
                    Ok(Ok(())) => {
                        state.displayed = ZoneColors::BLACK;
                        let _ = command.acknowledged.send(Ok(()));
                    }
                    Ok(Err(error)) => {
                        let summary = format!("{error:#}");
                        let _ = command.acknowledged.send(Err(summary));
                        write_error = Some(error.context("blackout failed"));
                        let _ = writer_cancel_sender.send(true);
                        break;
                    }
                    Err(_) => {
                        let error = anyhow!(
                            "blackout timed out after {} ms",
                            SAFETY_BLACKOUT_TIMEOUT.as_millis()
                        );
                        let _ = command.acknowledged.send(Err(error.to_string()));
                        write_error = Some(error);
                        let _ = writer_cancel_sender.send(true);
                        break;
                    }
                }
                continue;
            }

            match next_writer_wake(
                &mut safety_receiver,
                safety_open,
                &mut update_receiver,
                updates_open,
                state.next_write,
            )
            .await
            {
                WriterWake::Safety(command) => match command {
                    Some(command) => pending_safety = Some(command),
                    None => safety_open = false,
                },
                WriterWake::Update(update) => {
                    let Some(update) = update else {
                        updates_open = false;
                        continue;
                    };
                    match state.accept_update(update, tokio::time::Instant::now(), true) {
                        Some(true) => {
                            stale_updates += 1;
                            writer_metrics.dropped();
                        }
                        Some(false) => {}
                        None => {
                            stale_updates += 1;
                            writer_metrics.dropped();
                        }
                    }
                }
                WriterWake::WriteDeadline => {
                    let now = tokio::time::Instant::now();
                    let Some(update) = state.current_update.as_ref() else {
                        state.transition_active = false;
                        state.next_write = None;
                        continue;
                    };
                    let captured_at = update.captured_at;
                    let colors = state.transition.colors_at(now.into_std());
                    let wrote_final_target = state.transition.is_complete(now.into_std());
                    let write_result = {
                        let write = sink.write_update(colors, captured_at);
                        tokio::pin!(write);
                        loop {
                            tokio::select! {
                                biased;
                                command = safety_receiver.recv(), if safety_open => {
                                    match command {
                                        Some(command) => break Err(command),
                                        None => {
                                            safety_open = false;
                                            continue;
                                        }
                                    }
                                }
                                result = &mut write => break Ok(result),
                            }
                        }
                    };
                    let write_result = match write_result {
                        Ok(result) => result,
                        Err(command) => {
                            pending_safety = Some(command);
                            continue 'writer;
                        }
                    };
                    match write_result {
                        Ok(LightUpdateStatus::Rendered) => {
                            state.mark_displayed(colors);
                            rendered += 1;
                            let micros = u64::try_from(captured_at.elapsed().as_micros())
                                .unwrap_or(u64::MAX)
                                .max(1);
                            if let Err(error) = histogram.record(micros) {
                                write_error = Some(
                                    anyhow::Error::new(error).context("latency record failed"),
                                );
                                let _ = writer_cancel_sender.send(true);
                                break;
                            }
                            if let Err(error) = writer_metrics.rendered(micros) {
                                write_error = Some(error);
                                let _ = writer_cancel_sender.send(true);
                                break;
                            }
                        }
                        Ok(LightUpdateStatus::Unchanged) => state.mark_displayed(colors),
                        Ok(LightUpdateStatus::Expired) => {
                            stale_updates += 1;
                            writer_metrics.dropped();
                            state.displayed = ZoneColors::BLACK;
                            state.current_update = None;
                            state.current_update_rendered = false;
                            state.transition_active = false;
                            state.next_write = None;
                        }
                        Err(error) if error.downcast_ref::<RecoveryCancelled>().is_some() => {
                            if state.clear_as_dropped() {
                                stale_updates += 1;
                                writer_metrics.dropped();
                            }
                            continue;
                        }
                        Err(error) => {
                            write_error = Some(error);
                            let _ = writer_cancel_sender.send(true);
                            break;
                        }
                    }

                    let write_finished_at = tokio::time::Instant::now();
                    if let Some(update) = update_receiver.try_recv() {
                        match state.accept_update(update, write_finished_at, false) {
                            Some(true) => {
                                stale_updates += 1;
                                writer_metrics.dropped();
                            }
                            Some(false) => {}
                            None => {
                                stale_updates += 1;
                                writer_metrics.dropped();
                            }
                        }
                    } else if state.current_update.is_some() {
                        state.transition_active = !wrote_final_target;
                        state.next_write = state.transition_active.then(|| {
                            if state.transition.is_complete(write_finished_at.into_std()) {
                                write_finished_at
                            } else {
                                write_finished_at + LIGHT_UPDATE_INTERVAL
                            }
                        });
                    }
                }
            }
        }

        match write_error {
            None => Ok((rendered, stale_updates, histogram)),
            Some(error) => Err(error.context("light update failed")),
        }
    });

    tokio::pin!(cancellation);
    let sampler = Arc::new(sampler);
    let mut sampled_replacements = 0_u64;
    let mut capture_stalls = 0_u64;
    let mut sampling_error = None;
    let mut mask_cache = ZoneMaskCache::default();
    let mut cancel = cancel_receiver;
    let stall = tokio::time::sleep(CAPTURE_STALL_TIMEOUT);
    tokio::pin!(stall);
    let mut capture_stalled = false;
    'sampling: loop {
        let captured = tokio::select! {
            biased;
            () = &mut cancellation => {
                let _ = cancel_sender.send(true);
                break;
            }
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() { break; }
                continue;
            }
            () = &mut stall, if !capture_stalled => {
                capture_stalled = true;
                capture_stalls += 1;
                metrics.stalled();
                if let Err(error) =
                    request_safety_blackout(&safety_sender, &capture_generation).await
                {
                    sampling_error = Some(error.context("capture-stall blackout failed"));
                    let _ = cancel_sender.send(true);
                    break;
                }
                continue;
            }
            captured = frame_receiver.recv() => captured,
        };
        let Some(captured) = captured else { break };
        if captured.capture_generation != capture_generation.load(Ordering::SeqCst)
            || captured.capture_generation % 2 == 1
        {
            sampled_replacements += 1;
            metrics.dropped();
            continue;
        }
        capture_stalled = false;
        stall
            .as_mut()
            .reset(tokio::time::Instant::now() + CAPTURE_STALL_TIMEOUT);
        let CapturedFrame {
            captured_at,
            frame,
            capture_generation: frame_generation,
        } = captured;
        let masks = match mask_cache.get_or_compile(&layout, frame.width, frame.height) {
            Ok(masks) => masks,
            Err(error) => {
                sampling_error = Some(error.context("zone mask compilation failed"));
                let _ = cancel_sender.send(true);
                break;
            }
        };
        let sampler = sampler.clone();
        let sample = tokio::task::spawn_blocking(move || sampler(&frame, &masks, config));
        tokio::pin!(sample);
        let colors = tokio::select! {
            biased;
            () = &mut cancellation => {
                let _ = cancel_sender.send(true);
                break 'sampling;
            }
            changed = cancel.changed() => {
                if changed.is_err() || *cancel.borrow() { break 'sampling; }
                continue 'sampling;
            }
            result = &mut sample => match result {
                Ok(colors) => colors,
                Err(error) => {
                    sampling_error = Some(anyhow::Error::new(error).context("sampling task failed"));
                    let _ = cancel_sender.send(true);
                    break 'sampling;
                }
            },
        };
        if *cancel.borrow() {
            break;
        }
        if frame_generation != capture_generation.load(Ordering::SeqCst)
            || frame_generation % 2 == 1
        {
            sampled_replacements += 1;
            metrics.dropped();
            continue;
        }
        match update_sender.send(SampledUpdate {
            captured_at,
            colors,
            capture_generation: frame_generation,
        }) {
            Ok(replaced) => {
                sampled_replacements += u64::from(replaced);
                if replaced {
                    metrics.dropped();
                }
            }
            Err(_) => break,
        }
    }

    let _ = cancel_sender.send(true);
    drop(frame_receiver);
    let teardown_blackout_error = request_safety_blackout(&safety_sender, &capture_generation)
        .await
        .context("engine teardown blackout failed")
        .err();
    let capture_join = capture.await;
    drop(update_sender);
    drop(safety_sender);
    let writer_result = writer.await.context("writer task failed to join")?;

    let mut capture_stats = None;
    let capture_error = match capture_join {
        Ok(Ok(stats)) => {
            capture_stats = Some(stats);
            None
        }
        Ok(Err(error)) => Some(error.context("frame capture failed")),
        Err(error) => Some(anyhow::Error::new(error).context("capture task failed to join")),
    };
    let primary_error = sampling_error.or(capture_error).or(teardown_blackout_error);
    match (primary_error, writer_result) {
        (Some(primary), Err(shutdown)) => Err(anyhow!(
            "{primary:#}; engine shutdown also failed: {shutdown:#}"
        )),
        (Some(primary), Ok(_)) => Err(primary),
        (None, Err(error)) => Err(error),
        (None, Ok((rendered_updates, stale_updates, capture_to_write))) => {
            let (captured_frames, frame_replacements) =
                capture_stats.expect("successful capture has statistics");
            Ok(EngineStats {
                captured_frames,
                rendered_updates,
                dropped_frames: frame_replacements + sampled_replacements + stale_updates,
                capture_stalls,
                capture_to_write,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::{Duration, Instant};

    use tokio::sync::{Notify, Semaphore, mpsc};
    use tokio_util::sync::CancellationToken;

    use super::{
        CAPTURE_STALL_TIMEOUT, EngineMetrics, FrameSource, FrameSourceFactory, LightSink,
        LightUpdateStatus, LightingWriterState, RecoveringFrameSource, RecoveringLightSink,
        RetryBackoff, SAFETY_BLACKOUT_TIMEOUT, SampledUpdate, WriterWake, ZoneMaskCache,
        next_writer_wake, run_engine_with_metrics, run_engine_with_sampler,
    };
    use crate::{
        DEFAULT_TRANSITION_DURATION, Rgb8, RgbFrame, SamplerConfig, TransitionController,
        ZoneColors, ZoneLayout, ZoneMasks,
    };

    fn frame(value: u8) -> RgbFrame {
        RgbFrame::new(1, 1, 3, vec![value, 0, 0]).unwrap()
    }

    fn color_frame(color: Rgb8) -> RgbFrame {
        RgbFrame::new(1, 1, 3, vec![color.r, color.g, color.b]).unwrap()
    }

    fn layout() -> ZoneLayout {
        ZoneLayout::g560_default()
    }

    #[test]
    fn mask_cache_reuses_dimensions_and_rebuilds_on_either_dimension_change() {
        let layout = layout();
        let mut cache = ZoneMaskCache::default();

        let initial = cache.get_or_compile(&layout, 160, 90).unwrap();
        let same = cache.get_or_compile(&layout, 160, 90).unwrap();
        assert!(Arc::ptr_eq(&initial, &same));

        let width_changed = cache.get_or_compile(&layout, 161, 90).unwrap();
        assert!(!Arc::ptr_eq(&same, &width_changed));
        let same_new_width = cache.get_or_compile(&layout, 161, 90).unwrap();
        assert!(Arc::ptr_eq(&width_changed, &same_new_width));

        let height_changed = cache.get_or_compile(&layout, 161, 91).unwrap();
        assert!(!Arc::ptr_eq(&same_new_width, &height_changed));
    }

    #[test]
    fn writer_state_reports_each_unrendered_target_replacement_once() {
        let now = tokio::time::Instant::now();
        let update = |red| SampledUpdate {
            captured_at: Instant::now(),
            colors: ZoneColors([Rgb8 { r: red, g: 0, b: 0 }; 4]),
            capture_generation: 0,
        };
        let mut state = LightingWriterState::new();

        assert_eq!(state.accept_update(update(10), now, true), Some(false));
        assert_eq!(state.accept_update(update(20), now, true), Some(true));
        state.mark_displayed(ZoneColors([Rgb8 { r: 20, g: 0, b: 0 }; 4]));
        assert_eq!(state.accept_update(update(30), now, true), Some(false));
    }

    #[test]
    fn writer_state_uses_90ms_transition_with_a_45ms_midpoint() {
        let now = tokio::time::Instant::now();
        let target = ZoneColors([Rgb8 { r: 255, g: 0, b: 0 }; 4]);
        let update = SampledUpdate {
            captured_at: Instant::now(),
            colors: target,
            capture_generation: 0,
        };
        let mut state = LightingWriterState::new();

        assert_eq!(state.accept_update(update, now, true), Some(false));
        let midpoint = state
            .transition
            .colors_at((now + Duration::from_millis(45)).into_std());

        assert_ne!(midpoint, ZoneColors::BLACK);
        assert_ne!(midpoint, target);
        assert_eq!(
            state
                .transition
                .colors_at((now + Duration::from_millis(90)).into_std()),
            target
        );
    }

    #[test]
    fn normal_black_targets_fade_longer_per_zone() {
        let now = tokio::time::Instant::now();
        let visible = Rgb8 { r: 80, g: 20, b: 5 };
        let blue = Rgb8 { r: 0, g: 0, b: 120 };
        let mut state = LightingWriterState::new();
        state.displayed = ZoneColors([visible; 4]);
        let target = ZoneColors([Rgb8::BLACK, blue, Rgb8::BLACK, blue]);
        let update = SampledUpdate {
            captured_at: Instant::now(),
            colors: target,
            capture_generation: 0,
        };

        assert_eq!(state.accept_update(update, now, true), Some(false));
        let at_ninety = state
            .transition
            .colors_at((now + Duration::from_millis(90)).into_std());
        assert_ne!(at_ninety.0[0], Rgb8::BLACK);
        assert_eq!(at_ninety.0[1], blue);
        assert_ne!(at_ninety.0[2], Rgb8::BLACK);
        assert_eq!(at_ninety.0[3], blue);
        assert_eq!(
            state
                .transition
                .colors_at((now + Duration::from_millis(200)).into_std()),
            target
        );
    }

    #[tokio::test(start_paused = true)]
    async fn due_deadlines_win_repeatedly_while_target_input_stays_ready() {
        let (_safety_sender, mut safety_receiver) = mpsc::unbounded_channel();
        let (update_sender, mut update_receiver) = crate::latest_channel();
        assert!(
            update_sender
                .send(SampledUpdate {
                    captured_at: Instant::now(),
                    colors: ZoneColors([Rgb8 { r: 1, g: 0, b: 0 }; 4]),
                    capture_generation: 0,
                })
                .is_ok()
        );

        for _ in 0..10 {
            let wake = next_writer_wake(
                &mut safety_receiver,
                true,
                &mut update_receiver,
                true,
                Some(tokio::time::Instant::now()),
            )
            .await;
            assert!(matches!(wake, WriterWake::WriteDeadline));
        }
    }

    struct TestSource {
        frames: std::vec::IntoIter<RgbFrame>,
        error: bool,
        pending_at_end: bool,
        release_after_first: Option<Arc<Notify>>,
        ended: Option<Arc<Notify>>,
        shutdowns: Option<Arc<AtomicUsize>>,
    }

    struct ReopeningTestSourceFactory {
        sources: std::collections::VecDeque<TestSource>,
        opens: Arc<AtomicUsize>,
        allow_reopen: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl FrameSourceFactory for ReopeningTestSourceFactory {
        type Source = TestSource;

        async fn open(&mut self) -> anyhow::Result<Self::Source> {
            let attempt = self.opens.fetch_add(1, Ordering::SeqCst);
            if attempt > 0 {
                self.allow_reopen.notified().await;
            }
            self.sources
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("no fake capture source remains"))
        }

        fn should_retry(&self, _: &anyhow::Error) -> bool {
            true
        }
    }

    #[async_trait::async_trait]
    impl FrameSource for TestSource {
        async fn next_frame(&mut self) -> anyhow::Result<Option<RgbFrame>> {
            if self.frames.len() == 2
                && let Some(release) = self.release_after_first.take()
            {
                release.notified().await;
            }
            if let Some(frame) = self.frames.next() {
                return Ok(Some(frame));
            }
            if self.error {
                self.error = false;
                anyhow::bail!("fake capture failure");
            }
            if self.pending_at_end {
                return std::future::pending().await;
            }
            if let Some(ended) = self.ended.take() {
                ended.notify_one();
            }
            Ok(None)
        }

        async fn shutdown(&mut self) -> anyhow::Result<()> {
            if let Some(shutdowns) = &self.shutdowns {
                shutdowns.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    struct RecordingSink {
        writes: Arc<Mutex<Vec<ZoneColors>>>,
        fail_blackout: bool,
    }

    enum SourceEvent {
        Frame(RgbFrame),
        Error,
        End,
    }

    struct EventSource {
        events: mpsc::UnboundedReceiver<SourceEvent>,
        consumed: Arc<Semaphore>,
        shutdowns: Arc<AtomicUsize>,
    }

    struct HotSource {
        next_red: u8,
        shutdowns: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl FrameSource for HotSource {
        async fn next_frame(&mut self) -> anyhow::Result<Option<RgbFrame>> {
            tokio::task::yield_now().await;
            self.next_red = self.next_red.wrapping_add(1).max(1);
            Ok(Some(frame(self.next_red)))
        }

        async fn shutdown(&mut self) -> anyhow::Result<()> {
            self.shutdowns.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl FrameSource for EventSource {
        async fn next_frame(&mut self) -> anyhow::Result<Option<RgbFrame>> {
            match self.events.recv().await {
                Some(SourceEvent::Frame(frame)) => {
                    self.consumed.add_permits(1);
                    Ok(Some(frame))
                }
                Some(SourceEvent::Error) => anyhow::bail!("fake source error"),
                Some(SourceEvent::End) | None => Ok(None),
            }
        }

        async fn shutdown(&mut self) -> anyhow::Result<()> {
            self.shutdowns.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct BlockingRecordingSink {
        operations: Arc<Mutex<Vec<ZoneColors>>>,
        operation_started: Arc<Semaphore>,
        block_first: Option<(Arc<Notify>, Arc<Notify>)>,
    }

    struct HangingBlackoutSink;

    #[async_trait::async_trait]
    impl LightSink for HangingBlackoutSink {
        async fn write(&mut self, _: ZoneColors) -> anyhow::Result<()> {
            Ok(())
        }

        async fn blackout(&mut self) -> anyhow::Result<()> {
            std::future::pending().await
        }
    }

    #[async_trait::async_trait]
    impl LightSink for BlockingRecordingSink {
        async fn write(&mut self, colors: ZoneColors) -> anyhow::Result<()> {
            self.operations.lock().unwrap().push(colors);
            self.operation_started.add_permits(1);
            if let Some((started, release)) = self.block_first.take() {
                started.notify_one();
                release.notified().await;
            }
            Ok(())
        }
    }

    async fn wait_for_count(counter: &AtomicUsize, expected: usize) {
        while counter.load(Ordering::SeqCst) < expected {
            tokio::task::yield_now().await;
        }
    }

    fn event_source() -> (
        mpsc::UnboundedSender<SourceEvent>,
        EventSource,
        Arc<Semaphore>,
        Arc<AtomicUsize>,
    ) {
        let (sender, events) = mpsc::unbounded_channel();
        let consumed = Arc::new(Semaphore::new(0));
        let shutdowns = Arc::new(AtomicUsize::new(0));
        (
            sender,
            EventSource {
                events,
                consumed: consumed.clone(),
                shutdowns: shutdowns.clone(),
            },
            consumed,
            shutdowns,
        )
    }

    struct BlockingSinkFixture {
        sink: BlockingRecordingSink,
        operations: Arc<Mutex<Vec<ZoneColors>>>,
        operation_started: Arc<Semaphore>,
        first_started: Arc<Notify>,
        release_first: Arc<Notify>,
    }

    fn blocking_sink() -> BlockingSinkFixture {
        let operations = Arc::new(Mutex::new(Vec::new()));
        let operation_started = Arc::new(Semaphore::new(0));
        let first_started = Arc::new(Notify::new());
        let release_first = Arc::new(Notify::new());
        BlockingSinkFixture {
            sink: BlockingRecordingSink {
                operations: operations.clone(),
                operation_started: operation_started.clone(),
                block_first: Some((first_started.clone(), release_first.clone())),
            },
            operations,
            operation_started,
            first_started,
            release_first,
        }
    }

    fn counting_sink() -> (
        BlockingRecordingSink,
        Arc<Mutex<Vec<ZoneColors>>>,
        Arc<Semaphore>,
    ) {
        let operations = Arc::new(Mutex::new(Vec::new()));
        let operation_started = Arc::new(Semaphore::new(0));
        (
            BlockingRecordingSink {
                operations: operations.clone(),
                operation_started: operation_started.clone(),
                block_first: None,
            },
            operations,
            operation_started,
        )
    }

    #[async_trait::async_trait]
    impl LightSink for RecordingSink {
        async fn write(&mut self, colors: ZoneColors) -> anyhow::Result<()> {
            self.writes.lock().unwrap().push(colors);
            if self.fail_blackout && colors == ZoneColors::BLACK {
                anyhow::bail!("fake blackout failure");
            }
            Ok(())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn normal_target_emits_an_intermediate_before_exact_target_at_90ms() {
        let (events, source, consumed, _) = event_source();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let wrote = Arc::new(Notify::new());
        let cancellation = CancellationToken::new();
        let engine = tokio::spawn(run_engine_with_sampler(
            source,
            NotifyingSink {
                writes: writes.clone(),
                wrote: wrote.clone(),
            },
            layout(),
            SamplerConfig::default(),
            cancellation.clone().cancelled_owned(),
            |frame, _, _| {
                ZoneColors(
                    [Rgb8 {
                        r: frame.pixels[0],
                        g: 0,
                        b: 0,
                    }; 4],
                )
            },
        ));

        events.send(SourceEvent::Frame(frame(255))).unwrap();
        consumed.acquire().await.unwrap().forget();
        tokio::time::advance(Duration::from_millis(20)).await;
        wrote.notified().await;
        let first = writes.lock().unwrap()[0];
        assert_ne!(first, ZoneColors::BLACK);
        assert_ne!(first, ZoneColors([Rgb8 { r: 255, g: 0, b: 0 }; 4]));

        tokio::time::advance(Duration::from_millis(70)).await;
        wrote.notified().await;
        assert_eq!(
            *writes.lock().unwrap().last().unwrap(),
            ZoneColors([Rgb8 { r: 255, g: 0, b: 0 }; 4])
        );

        cancellation.cancel();
        engine.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn due_write_deadline_is_not_starved_by_a_hot_frame_producer() {
        let sampled = Arc::new(AtomicUsize::new(0));
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let (sink, operations, operation_started) = counting_sink();
        let cancellation = CancellationToken::new();
        let sampler_calls = sampled.clone();
        let engine = tokio::spawn(run_engine_with_sampler(
            HotSource {
                next_red: 0,
                shutdowns: shutdowns.clone(),
            },
            sink,
            layout(),
            SamplerConfig::default(),
            cancellation.clone().cancelled_owned(),
            move |frame, _, _| {
                sampler_calls.fetch_add(1, Ordering::SeqCst);
                ZoneColors(
                    [Rgb8 {
                        r: frame.pixels[0],
                        g: 0,
                        b: 0,
                    }; 4],
                )
            },
        ));

        wait_for_count(&sampled, 10).await;
        tokio::time::advance(Duration::from_millis(100)).await;
        for _ in 0..100 {
            tokio::task::yield_now().await;
            if !operations.lock().unwrap().is_empty() {
                break;
            }
        }
        assert!(
            !operations.lock().unwrap().is_empty(),
            "a due hardware deadline must beat continuously ready sampled targets"
        );
        operation_started.acquire().await.unwrap().forget();

        cancellation.cancel();
        engine.await.unwrap().unwrap();
        assert_eq!(shutdowns.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_prioritizes_black_over_a_pending_target() {
        let (events, source, consumed, shutdowns) = event_source();
        let BlockingSinkFixture {
            sink,
            operations,
            operation_started,
            first_started,
            release_first,
        } = blocking_sink();
        let cancellation = CancellationToken::new();
        let sampled = Arc::new(Semaphore::new(0));
        let sampler_signal = sampled.clone();
        let engine = tokio::spawn(run_engine_with_sampler(
            source,
            sink,
            layout(),
            SamplerConfig::default(),
            cancellation.clone().cancelled_owned(),
            move |frame, _, _| {
                sampler_signal.add_permits(1);
                ZoneColors(
                    [Rgb8 {
                        r: frame.pixels[0],
                        g: 0,
                        b: 0,
                    }; 4],
                )
            },
        ));

        events.send(SourceEvent::Frame(frame(100))).unwrap();
        consumed.acquire().await.unwrap().forget();
        sampled.acquire().await.unwrap().forget();
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(Duration::from_millis(20)).await;
        first_started.notified().await;
        events.send(SourceEvent::Frame(frame(200))).unwrap();
        consumed.acquire().await.unwrap().forget();
        cancellation.cancel();
        wait_for_count(&shutdowns, 1).await;
        tokio::task::yield_now().await;

        release_first.notify_one();
        operation_started.acquire().await.unwrap().forget();
        operation_started.acquire().await.unwrap().forget();
        assert_eq!(operations.lock().unwrap()[1], ZoneColors::BLACK);
        engine.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn retarget_abandons_red_and_fades_from_the_visible_intermediate() {
        let (events, source, consumed, _) = event_source();
        let (sink, operations, operation_started) = counting_sink();
        let sampled = Arc::new(Semaphore::new(0));
        let cancellation = CancellationToken::new();
        let sampler_signal = sampled.clone();
        let engine = tokio::spawn(run_engine_with_sampler(
            source,
            sink,
            layout(),
            SamplerConfig::default(),
            cancellation.clone().cancelled_owned(),
            move |frame, _, _| {
                sampler_signal.add_permits(1);
                ZoneColors(
                    [Rgb8 {
                        r: frame.pixels[0],
                        g: frame.pixels[1],
                        b: frame.pixels[2],
                    }; 4],
                )
            },
        ));

        let red = ZoneColors([Rgb8 { r: 255, g: 0, b: 0 }; 4]);
        let blue = ZoneColors([Rgb8 { r: 0, g: 0, b: 255 }; 4]);
        events
            .send(SourceEvent::Frame(color_frame(red.0[0])))
            .unwrap();
        consumed.acquire().await.unwrap().forget();
        sampled.acquire().await.unwrap().forget();
        tokio::time::advance(Duration::from_millis(60)).await;
        operation_started.acquire().await.unwrap().forget();
        let red_intermediate = *operations.lock().unwrap().last().unwrap();
        assert_ne!(red_intermediate, red);

        events
            .send(SourceEvent::Frame(color_frame(blue.0[0])))
            .unwrap();
        consumed.acquire().await.unwrap().forget();
        sampled.acquire().await.unwrap().forget();
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        let after_retarget = operations.lock().unwrap().len();
        tokio::time::advance(Duration::from_millis(20)).await;
        operation_started.acquire().await.unwrap().forget();
        let first_blue_step = *operations.lock().unwrap().last().unwrap();
        assert_ne!(first_blue_step, red);
        assert_ne!(first_blue_step, blue);
        assert_ne!(first_blue_step, ZoneColors::BLACK);

        tokio::time::advance(Duration::from_millis(100)).await;
        operation_started.acquire().await.unwrap().forget();
        let writes = operations.lock().unwrap().clone();
        assert_eq!(*writes.last().unwrap(), blue);
        assert!(!writes[after_retarget..].contains(&red));

        cancellation.cancel();
        engine.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn blocked_write_discards_ten_old_targets_and_uses_only_the_newest() {
        let (events, source, consumed, _) = event_source();
        let BlockingSinkFixture {
            sink,
            operations,
            operation_started,
            first_started,
            release_first,
        } = blocking_sink();
        let cancellation = CancellationToken::new();
        let newest = Rgb8 { r: 1, g: 2, b: 253 };
        let newest_sampled = Arc::new(Notify::new());
        let sampler_signal = newest_sampled.clone();
        let engine = tokio::spawn(run_engine_with_sampler(
            source,
            sink,
            layout(),
            SamplerConfig::default(),
            cancellation.clone().cancelled_owned(),
            move |frame, _, _| {
                let color = Rgb8 {
                    r: frame.pixels[0],
                    g: frame.pixels[1],
                    b: frame.pixels[2],
                };
                if color == newest {
                    sampler_signal.notify_one();
                }
                ZoneColors([color; 4])
            },
        ));

        events
            .send(SourceEvent::Frame(color_frame(Rgb8 { r: 255, g: 0, b: 0 })))
            .unwrap();
        consumed.acquire().await.unwrap().forget();
        tokio::time::advance(Duration::from_millis(20)).await;
        first_started.notified().await;
        operation_started.acquire().await.unwrap().forget();

        let abandoned = [
            Rgb8 { r: 0, g: 255, b: 0 },
            Rgb8 {
                r: 255,
                g: 255,
                b: 0,
            },
            Rgb8 {
                r: 0,
                g: 255,
                b: 255,
            },
            Rgb8 {
                r: 255,
                g: 0,
                b: 255,
            },
            Rgb8 {
                r: 255,
                g: 128,
                b: 0,
            },
            Rgb8 {
                r: 128,
                g: 255,
                b: 0,
            },
            Rgb8 {
                r: 0,
                g: 255,
                b: 128,
            },
            Rgb8 {
                r: 0,
                g: 128,
                b: 255,
            },
            Rgb8 {
                r: 128,
                g: 0,
                b: 255,
            },
        ];
        for color in abandoned {
            events.send(SourceEvent::Frame(color_frame(color))).unwrap();
        }
        events
            .send(SourceEvent::Frame(color_frame(newest)))
            .unwrap();
        consumed.acquire_many(10).await.unwrap().forget();
        newest_sampled.notified().await;
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }

        tokio::time::advance(Duration::from_millis(60)).await;
        let displayed_before_release = operations.lock().unwrap()[0];
        let before_release = operations.lock().unwrap().len();
        release_first.notify_one();
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(Duration::from_millis(20)).await;
        operation_started.acquire().await.unwrap().forget();
        let transition_start = Instant::now();
        let mut expected_transition =
            TransitionController::new(displayed_before_release, DEFAULT_TRANSITION_DURATION);
        expected_transition.retarget(ZoneColors([newest; 4]), transition_start);
        let expected_first_step =
            expected_transition.colors_at(transition_start + Duration::from_millis(20));
        let writes = operations.lock().unwrap().clone();
        assert_eq!(writes[before_release], expected_first_step);
        for abandoned in abandoned.map(|color| ZoneColors([color; 4])) {
            assert!(!writes[before_release..].contains(&abandoned));
        }

        tokio::time::advance(Duration::from_millis(100)).await;
        operation_started.acquire().await.unwrap().forget();
        let writes = operations.lock().unwrap().clone();
        assert_eq!(*writes.last().unwrap(), ZoneColors([newest; 4]));
        for abandoned in abandoned.map(|color| ZoneColors([color; 4])) {
            assert!(!writes[before_release..].contains(&abandoned));
        }

        cancellation.cancel();
        let stats = engine.await.unwrap().unwrap();
        assert!(stats.dropped_frames >= 9);
    }

    #[tokio::test(start_paused = true)]
    async fn frame_captured_during_blackout_cannot_relight_after_ack() {
        let (events, source, consumed, _) = event_source();
        let BlockingSinkFixture {
            sink,
            operations,
            operation_started,
            first_started,
            release_first,
        } = blocking_sink();
        let sampled = Arc::new(AtomicUsize::new(0));
        let cancellation = CancellationToken::new();
        let sampler_calls = sampled.clone();
        let engine = tokio::spawn(run_engine_with_sampler(
            source,
            sink,
            layout(),
            SamplerConfig::default(),
            cancellation.clone().cancelled_owned(),
            move |frame, _, _| {
                sampler_calls.fetch_add(1, Ordering::SeqCst);
                ZoneColors(
                    [Rgb8 {
                        r: frame.pixels[0],
                        g: frame.pixels[1],
                        b: frame.pixels[2],
                    }; 4],
                )
            },
        ));

        tokio::time::advance(CAPTURE_STALL_TIMEOUT).await;
        first_started.notified().await;
        operation_started.acquire().await.unwrap().forget();
        assert_eq!(operations.lock().unwrap()[0], ZoneColors::BLACK);

        events
            .send(SourceEvent::Frame(color_frame(Rgb8 { r: 255, g: 0, b: 0 })))
            .unwrap();
        consumed.acquire().await.unwrap().forget();
        release_first.notify_one();
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(Duration::from_millis(20)).await;
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert_eq!(sampled.load(Ordering::SeqCst), 0);
        assert_eq!(operations.lock().unwrap().len(), 1);

        events
            .send(SourceEvent::Frame(color_frame(Rgb8 { r: 0, g: 0, b: 255 })))
            .unwrap();
        consumed.acquire().await.unwrap().forget();
        wait_for_count(&sampled, 1).await;
        tokio::time::advance(Duration::from_millis(20)).await;
        operation_started.acquire().await.unwrap().forget();
        let first_recovered = *operations.lock().unwrap().last().unwrap();
        assert_ne!(first_recovered, ZoneColors::BLACK);
        assert_ne!(
            first_recovered,
            ZoneColors([Rgb8 { r: 0, g: 0, b: 255 }; 4])
        );

        cancellation.cancel();
        engine.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn safety_preempts_an_unbounded_missing_usb_reconnect() {
        let (events, source, consumed, shutdowns) = event_source();
        let opens = Arc::new(AtomicUsize::new(0));
        let factory_opens = opens.clone();
        let sink = RecoveringLightSink::<_, RecordingSink>::new(
            move || {
                factory_opens.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!("fake USB remains absent")
            },
            CancellationToken::new(),
        );
        let sampled = Arc::new(Semaphore::new(0));
        let sampler_signal = sampled.clone();
        let engine = tokio::spawn(run_engine_with_sampler(
            source,
            sink,
            layout(),
            SamplerConfig::default(),
            std::future::pending(),
            move |frame, _, _| {
                sampler_signal.add_permits(1);
                ZoneColors(
                    [Rgb8 {
                        r: frame.pixels[0],
                        g: 0,
                        b: 0,
                    }; 4],
                )
            },
        ));

        events.send(SourceEvent::Frame(frame(255))).unwrap();
        consumed.acquire().await.unwrap().forget();
        sampled.acquire().await.unwrap().forget();
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(Duration::from_millis(20)).await;
        wait_for_count(&opens, 1).await;
        events.send(SourceEvent::End).unwrap();
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
        assert!(
            engine.is_finished(),
            "safety blackout must preempt missing-device reconnect"
        );
        engine.await.unwrap().unwrap();
        assert_eq!(shutdowns.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn safety_blackout_acknowledges_timeout_in_bounded_time() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let started = tokio::time::Instant::now();

        let result = run_engine_with_sampler(
            TestSource {
                frames: Vec::new().into_iter(),
                error: false,
                pending_at_end: true,
                release_after_first: None,
                ended: None,
                shutdowns: None,
            },
            HangingBlackoutSink,
            layout(),
            SamplerConfig::default(),
            cancellation.cancelled_owned(),
            |_, _, _| ZoneColors::BLACK,
        )
        .await;

        assert_eq!(started.elapsed(), SAFETY_BLACKOUT_TIMEOUT);
        assert!(result.unwrap_err().to_string().contains("timed out"));
    }

    #[tokio::test(start_paused = true)]
    async fn slow_write_does_not_stretch_transition_past_wall_clock_duration() {
        let (events, source, consumed, _) = event_source();
        let BlockingSinkFixture {
            sink,
            operations,
            operation_started,
            first_started,
            release_first,
        } = blocking_sink();
        let cancellation = CancellationToken::new();
        let target = ZoneColors([Rgb8 { r: 255, g: 0, b: 0 }; 4]);
        let engine = tokio::spawn(run_engine_with_sampler(
            source,
            sink,
            layout(),
            SamplerConfig::default(),
            cancellation.clone().cancelled_owned(),
            |frame, _, _| {
                ZoneColors(
                    [Rgb8 {
                        r: frame.pixels[0],
                        g: 0,
                        b: 0,
                    }; 4],
                )
            },
        ));

        events.send(SourceEvent::Frame(frame(255))).unwrap();
        consumed.acquire().await.unwrap().forget();
        tokio::time::advance(Duration::from_millis(20)).await;
        first_started.notified().await;
        operation_started.acquire().await.unwrap().forget();
        assert_ne!(operations.lock().unwrap()[0], target);

        tokio::time::advance(Duration::from_millis(200)).await;
        release_first.notify_one();
        operation_started.acquire().await.unwrap().forget();
        assert_eq!(operations.lock().unwrap()[1], target);

        cancellation.cancel();
        engine.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn source_error_prioritizes_black_over_a_pending_target() {
        let (events, source, consumed, shutdowns) = event_source();
        let BlockingSinkFixture {
            sink,
            operations,
            operation_started,
            first_started,
            release_first,
        } = blocking_sink();
        let engine = tokio::spawn(run_engine_with_sampler(
            source,
            sink,
            layout(),
            SamplerConfig::default(),
            std::future::pending(),
            |frame, _, _| {
                ZoneColors(
                    [Rgb8 {
                        r: frame.pixels[0],
                        g: 0,
                        b: 0,
                    }; 4],
                )
            },
        ));

        events.send(SourceEvent::Frame(frame(100))).unwrap();
        consumed.acquire().await.unwrap().forget();
        tokio::time::advance(Duration::from_millis(20)).await;
        first_started.notified().await;
        operation_started.acquire().await.unwrap().forget();
        events.send(SourceEvent::Frame(frame(200))).unwrap();
        consumed.acquire().await.unwrap().forget();
        events.send(SourceEvent::Error).unwrap();
        wait_for_count(&shutdowns, 1).await;

        release_first.notify_one();
        operation_started.acquire().await.unwrap().forget();
        assert_eq!(operations.lock().unwrap()[1], ZoneColors::BLACK);
        assert!(
            engine
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("capture")
        );
    }

    #[tokio::test(start_paused = true)]
    async fn source_end_prioritizes_black_over_a_pending_target() {
        let (events, source, consumed, shutdowns) = event_source();
        let BlockingSinkFixture {
            sink,
            operations,
            operation_started,
            first_started,
            release_first,
        } = blocking_sink();
        let engine = tokio::spawn(run_engine_with_sampler(
            source,
            sink,
            layout(),
            SamplerConfig::default(),
            std::future::pending(),
            |frame, _, _| {
                ZoneColors(
                    [Rgb8 {
                        r: frame.pixels[0],
                        g: 0,
                        b: 0,
                    }; 4],
                )
            },
        ));

        events.send(SourceEvent::Frame(frame(100))).unwrap();
        consumed.acquire().await.unwrap().forget();
        tokio::time::advance(Duration::from_millis(20)).await;
        first_started.notified().await;
        operation_started.acquire().await.unwrap().forget();
        events.send(SourceEvent::Frame(frame(200))).unwrap();
        consumed.acquire().await.unwrap().forget();
        events.send(SourceEvent::End).unwrap();
        wait_for_count(&shutdowns, 1).await;

        release_first.notify_one();
        operation_started.acquire().await.unwrap().forget();
        assert_eq!(operations.lock().unwrap()[1], ZoneColors::BLACK);
        engine.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn capture_stall_prioritizes_black_over_a_pending_target() {
        let (events, source, consumed, _) = event_source();
        let BlockingSinkFixture {
            sink,
            operations,
            operation_started,
            first_started,
            release_first,
        } = blocking_sink();
        let cancellation = CancellationToken::new();
        let sampled = Arc::new(Semaphore::new(0));
        let sampler_signal = sampled.clone();
        let engine = tokio::spawn(run_engine_with_sampler(
            source,
            sink,
            layout(),
            SamplerConfig::default(),
            cancellation.clone().cancelled_owned(),
            move |frame, _, _| {
                sampler_signal.add_permits(1);
                ZoneColors(
                    [Rgb8 {
                        r: frame.pixels[0],
                        g: 0,
                        b: 0,
                    }; 4],
                )
            },
        ));

        events.send(SourceEvent::Frame(frame(100))).unwrap();
        consumed.acquire().await.unwrap().forget();
        sampled.acquire().await.unwrap().forget();
        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(Duration::from_millis(20)).await;
        first_started.notified().await;
        operation_started.acquire().await.unwrap().forget();
        events.send(SourceEvent::Frame(frame(200))).unwrap();
        consumed.acquire().await.unwrap().forget();
        sampled.acquire().await.unwrap().forget();
        tokio::time::advance(CAPTURE_STALL_TIMEOUT).await;
        tokio::task::yield_now().await;

        release_first.notify_one();
        operation_started.acquire().await.unwrap().forget();
        assert_eq!(operations.lock().unwrap()[1], ZoneColors::BLACK);
        cancellation.cancel();
        engine.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn retry_backoff_waits_250ms_then_doubles_and_caps_at_five_seconds() {
        let cancellation = CancellationToken::new();
        let mut retry = RetryBackoff::default();
        for expected in [250, 500, 1_000, 2_000, 4_000, 5_000, 5_000] {
            let started = tokio::time::Instant::now();
            retry.wait(&cancellation).await.unwrap();
            assert_eq!(
                started.elapsed(),
                std::time::Duration::from_millis(expected)
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn missing_device_retries_with_bounded_backoff_until_connected() {
        let opens = Arc::new(AtomicUsize::new(0));
        let writes = Arc::new(Mutex::new(Vec::new()));
        let factory = {
            let opens = opens.clone();
            let writes = writes.clone();
            move || {
                let attempt = opens.fetch_add(1, Ordering::SeqCst);
                if attempt < 6 {
                    anyhow::bail!("fake device missing");
                }
                Ok(RecordingSink {
                    writes: writes.clone(),
                    fail_blackout: false,
                })
            }
        };
        let cancellation = CancellationToken::new();
        let mut sink = RecoveringLightSink::new(factory, cancellation);

        sink.write(ZoneColors([Rgb8 { r: 1, g: 2, b: 3 }; 4]))
            .await
            .unwrap();

        assert_eq!(opens.load(Ordering::SeqCst), 7);
        assert_eq!(
            *writes.lock().unwrap(),
            vec![ZoneColors([Rgb8 { r: 1, g: 2, b: 3 }; 4])]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn reconnect_never_writes_an_expired_scene_color() {
        let opens = Arc::new(AtomicUsize::new(0));
        let writes = Arc::new(Mutex::new(Vec::new()));
        let factory = {
            let opens = opens.clone();
            let writes = writes.clone();
            move || {
                let attempt = opens.fetch_add(1, Ordering::SeqCst);
                if attempt < 2 {
                    anyhow::bail!("fake device missing");
                }
                Ok(RecordingSink {
                    writes: writes.clone(),
                    fail_blackout: false,
                })
            }
        };
        let mut sink = RecoveringLightSink::new(factory, CancellationToken::new());
        let captured_at = std::time::Instant::now() - CAPTURE_STALL_TIMEOUT;

        let rendered = sink
            .write_update(
                ZoneColors(
                    [Rgb8 {
                        r: 90,
                        g: 80,
                        b: 70,
                    }; 4],
                ),
                captured_at,
            )
            .await
            .unwrap();

        assert_eq!(rendered, LightUpdateStatus::Expired);
        assert_eq!(*writes.lock().unwrap(), vec![ZoneColors::BLACK]);
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RecoveryEvent {
        Open(usize),
        Write(usize),
        Blackout(usize),
    }

    struct FailingDevice {
        id: usize,
        events: Arc<Mutex<Vec<RecoveryEvent>>>,
    }

    struct NotifyingSink {
        writes: Arc<Mutex<Vec<ZoneColors>>>,
        wrote: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl LightSink for NotifyingSink {
        async fn write(&mut self, colors: ZoneColors) -> anyhow::Result<()> {
            self.writes.lock().unwrap().push(colors);
            self.wrote.notify_one();
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl LightSink for FailingDevice {
        async fn write(&mut self, _: ZoneColors) -> anyhow::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(RecoveryEvent::Write(self.id));
            if self.id == 0 {
                anyhow::bail!("fake USB write failure");
            }
            Ok(())
        }

        async fn blackout(&mut self) -> anyhow::Result<()> {
            self.events
                .lock()
                .unwrap()
                .push(RecoveryEvent::Blackout(self.id));
            Ok(())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn three_usb_failures_attempt_blackout_then_reopen() {
        let opens = Arc::new(AtomicUsize::new(0));
        let events = Arc::new(Mutex::new(Vec::new()));
        let factory = {
            let opens = opens.clone();
            let events = events.clone();
            move || {
                let id = opens.fetch_add(1, Ordering::SeqCst);
                events.lock().unwrap().push(RecoveryEvent::Open(id));
                Ok(FailingDevice {
                    id,
                    events: events.clone(),
                })
            }
        };
        let mut sink = RecoveringLightSink::new(factory, CancellationToken::new());

        sink.write(ZoneColors([Rgb8 { r: 9, g: 8, b: 7 }; 4]))
            .await
            .unwrap();

        assert_eq!(
            *events.lock().unwrap(),
            vec![
                RecoveryEvent::Open(0),
                RecoveryEvent::Write(0),
                RecoveryEvent::Write(0),
                RecoveryEvent::Write(0),
                RecoveryEvent::Blackout(0),
                RecoveryEvent::Open(1),
                RecoveryEvent::Write(1),
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn capture_stall_blacks_out_until_a_fresh_frame_arrives() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let wrote = Arc::new(Notify::new());
        let cancellation = CancellationToken::new();
        let engine = tokio::spawn(run_engine_with_sampler(
            TestSource {
                frames: vec![frame(255)].into_iter(),
                error: false,
                pending_at_end: true,
                release_after_first: None,
                ended: None,
                shutdowns: None,
            },
            NotifyingSink {
                writes: writes.clone(),
                wrote: wrote.clone(),
            },
            layout(),
            SamplerConfig::default(),
            cancellation.clone().cancelled_owned(),
            |frame, _, _| {
                ZoneColors(
                    [Rgb8 {
                        r: frame.pixels[0],
                        g: 0,
                        b: 0,
                    }; 4],
                )
            },
        ));
        wrote.notified().await;
        tokio::time::advance(CAPTURE_STALL_TIMEOUT).await;
        wrote.notified().await;

        let writes_before_cancel = writes.lock().unwrap().clone();
        assert_ne!(writes_before_cancel[0], ZoneColors::BLACK);
        assert_eq!(*writes_before_cancel.last().unwrap(), ZoneColors::BLACK);
        cancellation.cancel();
        engine.await.unwrap().unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn capture_stream_failure_blacks_out_then_resumes_from_a_reopened_source() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let wrote = Arc::new(Notify::new());
        let opens = Arc::new(AtomicUsize::new(0));
        let allow_reopen = Arc::new(Notify::new());
        let shutdowns = Arc::new(AtomicUsize::new(0));
        let source = RecoveringFrameSource::new(ReopeningTestSourceFactory {
            sources: [
                TestSource {
                    frames: vec![frame(255)].into_iter(),
                    error: true,
                    pending_at_end: false,
                    release_after_first: None,
                    ended: None,
                    shutdowns: Some(shutdowns.clone()),
                },
                TestSource {
                    frames: vec![frame(200)].into_iter(),
                    error: false,
                    pending_at_end: true,
                    release_after_first: None,
                    ended: None,
                    shutdowns: Some(shutdowns.clone()),
                },
            ]
            .into(),
            opens: opens.clone(),
            allow_reopen: allow_reopen.clone(),
        });
        let cancellation = CancellationToken::new();
        let engine = tokio::spawn(run_engine_with_sampler(
            source,
            NotifyingSink {
                writes: writes.clone(),
                wrote: wrote.clone(),
            },
            layout(),
            SamplerConfig::default(),
            cancellation.clone().cancelled_owned(),
            |frame, _, _| {
                ZoneColors(
                    [Rgb8 {
                        r: frame.pixels[0],
                        g: 0,
                        b: 0,
                    }; 4],
                )
            },
        ));

        wrote.notified().await;
        tokio::time::advance(CAPTURE_STALL_TIMEOUT).await;
        wrote.notified().await;
        assert_eq!(*writes.lock().unwrap().last().unwrap(), ZoneColors::BLACK);
        let before_reopen = writes.lock().unwrap().len();
        allow_reopen.notify_one();
        while writes.lock().unwrap().len() == before_reopen {
            wrote.notified().await;
        }
        let first_recovered = writes.lock().unwrap()[before_reopen];
        assert_ne!(first_recovered, ZoneColors::BLACK);
        assert_ne!(
            first_recovered,
            ZoneColors([Rgb8 { r: 200, g: 0, b: 0 }; 4])
        );
        cancellation.cancel();
        engine.await.unwrap().unwrap();

        assert_eq!(opens.load(Ordering::SeqCst), 2);
        assert_eq!(shutdowns.load(Ordering::SeqCst), 2);
        assert_eq!(*writes.lock().unwrap().last().unwrap(), ZoneColors::BLACK);
    }

    #[tokio::test]
    async fn capture_eos_blacks_out_and_exits_cleanly() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let result = run_engine_with_sampler(
            TestSource {
                frames: Vec::new().into_iter(),
                error: false,
                pending_at_end: false,
                release_after_first: None,
                ended: None,
                shutdowns: None,
            },
            RecordingSink {
                writes: writes.clone(),
                fail_blackout: false,
            },
            layout(),
            SamplerConfig::default(),
            std::future::pending(),
            |_, _, _| ZoneColors::BLACK,
        )
        .await;

        result.unwrap();
        assert_eq!(*writes.lock().unwrap(), vec![ZoneColors::BLACK]);
    }

    #[tokio::test]
    async fn cancellation_token_blacks_out_and_exits_cleanly() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let result = run_engine_with_sampler(
            TestSource {
                frames: Vec::new().into_iter(),
                error: false,
                pending_at_end: true,
                release_after_first: None,
                ended: None,
                shutdowns: None,
            },
            RecordingSink {
                writes: writes.clone(),
                fail_blackout: false,
            },
            layout(),
            SamplerConfig::default(),
            cancellation.cancelled_owned(),
            |_, _, _| ZoneColors::BLACK,
        )
        .await;

        result.unwrap();
        assert_eq!(*writes.lock().unwrap(), vec![ZoneColors::BLACK]);
    }

    #[tokio::test]
    async fn live_metrics_snapshot_reports_counts_and_latency_without_color_data() {
        let metrics = EngineMetrics::new();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let wrote = Arc::new(Notify::new());
        let cancellation = CancellationToken::new();
        let engine = tokio::spawn(run_engine_with_metrics(
            TestSource {
                frames: vec![frame(3)].into_iter(),
                error: false,
                pending_at_end: true,
                release_after_first: None,
                ended: None,
                shutdowns: None,
            },
            NotifyingSink {
                writes,
                wrote: wrote.clone(),
            },
            layout(),
            SamplerConfig::default(),
            cancellation.clone().cancelled_owned(),
            metrics.clone(),
        ));
        wrote.notified().await;
        cancellation.cancel();
        engine.await.unwrap().unwrap();

        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.captured_frames, 1);
        assert_eq!(snapshot.rendered_updates, 1);
        assert_eq!(snapshot.dropped_frames, 0);
        assert_eq!(snapshot.capture_stalls, 0);
        assert!(snapshot.capture_to_write_p50_us > 0);
        assert!(snapshot.capture_to_write_p95_us >= snapshot.capture_to_write_p50_us);
        assert!(snapshot.capture_to_write_p99_us >= snapshot.capture_to_write_p95_us);
    }

    #[tokio::test]
    async fn capture_replaces_pending_frames_while_sampling_is_blocked() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::new(Notify::new());
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let sampled = Arc::new(Mutex::new(Vec::new()));
        let ended = Arc::new(Notify::new());
        let sampler = {
            let started = started.clone();
            let gate = gate.clone();
            let sampled = sampled.clone();
            move |frame: &RgbFrame, _: &ZoneMasks, _: SamplerConfig| {
                let value = frame.pixels[0];
                sampled.lock().unwrap().push(value);
                if value == 1 {
                    started.notify_waiters();
                    let (lock, ready) = &*gate;
                    let guard = lock.lock().unwrap();
                    drop(ready.wait_while(guard, |open| !*open).unwrap());
                }
                ZoneColors(
                    [Rgb8 {
                        r: value,
                        g: 0,
                        b: 0,
                    }; 4],
                )
            }
        };
        let engine = tokio::spawn(run_engine_with_sampler(
            TestSource {
                frames: vec![frame(1), frame(2), frame(3)].into_iter(),
                error: false,
                pending_at_end: false,
                release_after_first: Some(started.clone()),
                ended: Some(ended.clone()),
                shutdowns: None,
            },
            RecordingSink {
                writes,
                fail_blackout: false,
            },
            layout(),
            SamplerConfig::default(),
            std::future::pending(),
            sampler,
        ));
        started.notified().await;
        ended.notified().await;
        let (lock, ready) = &*gate;
        *lock.lock().unwrap() = true;
        ready.notify_one();
        let stats = engine.await.unwrap().unwrap();

        assert_eq!(*sampled.lock().unwrap(), vec![1, 3]);
        assert_eq!(
            stats.dropped_frames, 3,
            "one capture replacement and two consumed-but-unrendered updates must be counted"
        );
    }

    #[tokio::test]
    async fn cancellation_during_sampling_never_publishes_the_sample() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::new(Notify::new());
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let cancellation = Arc::new(Notify::new());
        let sampler = {
            let started = started.clone();
            let gate = gate.clone();
            move |_: &RgbFrame, _: &ZoneMasks, _: SamplerConfig| {
                started.notify_waiters();
                let (lock, ready) = &*gate;
                let guard = lock.lock().unwrap();
                drop(ready.wait_while(guard, |open| !*open).unwrap());
                ZoneColors([Rgb8 { r: 9, g: 0, b: 0 }; 4])
            }
        };
        let engine = tokio::spawn(run_engine_with_sampler(
            TestSource {
                frames: vec![frame(1)].into_iter(),
                error: false,
                pending_at_end: true,
                release_after_first: None,
                ended: None,
                shutdowns: None,
            },
            RecordingSink {
                writes: writes.clone(),
                fail_blackout: false,
            },
            layout(),
            SamplerConfig::default(),
            cancellation.clone().notified_owned(),
            sampler,
        ));
        started.notified().await;
        cancellation.notify_one();
        tokio::task::yield_now().await;
        let (lock, ready) = &*gate;
        *lock.lock().unwrap() = true;
        ready.notify_one();
        engine.await.unwrap().unwrap();

        assert_eq!(*writes.lock().unwrap(), vec![ZoneColors::BLACK]);
    }

    #[tokio::test]
    async fn source_error_still_awaits_blackout() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let result = run_engine_with_sampler(
            TestSource {
                frames: Vec::new().into_iter(),
                error: true,
                pending_at_end: false,
                release_after_first: None,
                ended: None,
                shutdowns: None,
            },
            RecordingSink {
                writes: writes.clone(),
                fail_blackout: false,
            },
            layout(),
            SamplerConfig::default(),
            std::future::pending(),
            |_, _, _| ZoneColors::BLACK,
        )
        .await;

        assert!(result.unwrap_err().to_string().contains("capture"));
        assert_eq!(*writes.lock().unwrap(), vec![ZoneColors::BLACK]);
    }

    #[tokio::test]
    async fn sampling_panic_still_awaits_blackout() {
        let writes = Arc::new(Mutex::new(Vec::new()));
        let result = run_engine_with_sampler(
            TestSource {
                frames: vec![frame(1)].into_iter(),
                error: false,
                pending_at_end: false,
                release_after_first: None,
                ended: None,
                shutdowns: None,
            },
            RecordingSink {
                writes: writes.clone(),
                fail_blackout: false,
            },
            layout(),
            SamplerConfig::default(),
            std::future::pending(),
            |_, _, _| panic!("fake sampling panic"),
        )
        .await;

        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("sampling task failed")
        );
        assert_eq!(*writes.lock().unwrap(), vec![ZoneColors::BLACK]);
    }

    #[tokio::test]
    async fn blackout_failure_is_propagated() {
        let result = run_engine_with_sampler(
            TestSource {
                frames: Vec::new().into_iter(),
                error: false,
                pending_at_end: false,
                release_after_first: None,
                ended: None,
                shutdowns: None,
            },
            RecordingSink {
                writes: Arc::new(Mutex::new(Vec::new())),
                fail_blackout: true,
            },
            layout(),
            SamplerConfig::default(),
            std::future::pending(),
            |_, _, _| ZoneColors::BLACK,
        )
        .await;

        assert!(result.unwrap_err().to_string().contains("blackout failed"));
    }

    #[tokio::test]
    async fn engine_invokes_frame_source_shutdown_on_teardown() {
        let shutdowns = Arc::new(AtomicUsize::new(0));
        run_engine_with_sampler(
            TestSource {
                frames: Vec::new().into_iter(),
                error: false,
                pending_at_end: false,
                release_after_first: None,
                ended: None,
                shutdowns: Some(shutdowns.clone()),
            },
            RecordingSink {
                writes: Arc::new(Mutex::new(Vec::new())),
                fail_blackout: false,
            },
            layout(),
            SamplerConfig::default(),
            std::future::pending(),
            |_, _, _| ZoneColors::BLACK,
        )
        .await
        .unwrap();

        assert_eq!(shutdowns.load(Ordering::SeqCst), 1);
    }
}
