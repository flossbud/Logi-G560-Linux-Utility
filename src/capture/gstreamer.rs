use std::{
    os::fd::AsRawFd,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use gstreamer::{self as gst, prelude::*};
use gstreamer_app as gst_app;
use gstreamer_video::{VideoFormat, VideoFrameExt, VideoFrameRef, VideoInfo};

use super::{CaptureError, portal::PortalGrant};
use crate::{FrameSource, RgbFrame};

const STATIC_FRAME_HOLD_INTERVAL: Duration = Duration::from_millis(200);

pub struct GStreamerFrameSource {
    resources: CaptureResources,
    appsink: gst_app::AppSink,
    expected_dimensions: Arc<AtomicU64>,
    last_frame: Option<RgbFrame>,
    shutdown: ShutdownState,
}

struct RgbTail {
    appsink: gst_app::AppSink,
    expected_dimensions: Arc<AtomicU64>,
}

struct OutputCapsUpdate {
    revision: u64,
    dimensions: (u32, u32),
    caps: gst::Caps,
}

impl OutputCapsUpdate {
    fn new(dimensions: (u32, u32)) -> Result<Self, String> {
        let width = i32::try_from(dimensions.0)
            .map_err(|_| format!("scaled width does not fit GStreamer caps: {}", dimensions.0))?;
        let height = i32::try_from(dimensions.1).map_err(|_| {
            format!(
                "scaled height does not fit GStreamer caps: {}",
                dimensions.1
            )
        })?;
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGB")
            .field("width", width)
            .field("height", height)
            .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
            .build();
        Ok(Self {
            revision: 0,
            dimensions,
            caps,
        })
    }
}

#[derive(Default)]
struct OutputCapsState {
    revision: u64,
    pending: Option<OutputCapsUpdate>,
    worker_running: bool,
}

struct OutputCapsCoordinator {
    state: Mutex<OutputCapsState>,
    expected_dimensions: Arc<AtomicU64>,
}

impl OutputCapsCoordinator {
    fn new(expected_dimensions: Arc<AtomicU64>) -> Self {
        Self {
            state: Mutex::new(OutputCapsState::default()),
            expected_dimensions,
        }
    }

    fn request(&self, mut update: OutputCapsUpdate) -> bool {
        let mut state = self.lock_state();
        state.revision = state.revision.wrapping_add(1);
        update.revision = state.revision;
        state.pending = Some(update);
        self.expected_dimensions.store(0, Ordering::Release);
        if state.worker_running {
            false
        } else {
            state.worker_running = true;
            true
        }
    }

    fn invalidate(&self) {
        let mut state = self.lock_state();
        state.revision = state.revision.wrapping_add(1);
        state.pending = None;
        self.expected_dimensions.store(0, Ordering::Release);
    }

    fn drain(&self, mut install: impl FnMut(&OutputCapsUpdate)) {
        loop {
            let update = {
                let mut state = self.lock_state();
                let Some(update) = state.pending.take() else {
                    state.worker_running = false;
                    return;
                };
                update
            };

            install(&update);

            let mut state = self.lock_state();
            if state.revision == update.revision {
                debug_assert!(state.pending.is_none());
                self.expected_dimensions
                    .store(pack_dimensions(update.dimensions), Ordering::Release);
                state.worker_running = false;
                return;
            }
            if state.pending.is_none() {
                state.worker_running = false;
                return;
            }
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, OutputCapsState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct CaptureResources {
    pipeline: gst::Pipeline,
    grant: PortalGrant,
}

#[derive(Default)]
struct ShutdownState {
    pipeline_stopped: bool,
    portal_closed: bool,
}

#[async_trait::async_trait]
trait ShutdownOperations {
    fn stop_pipeline(&mut self) -> Result<(), CaptureError>;
    async fn close_portal(&mut self) -> Result<(), CaptureError>;
}

#[async_trait::async_trait]
impl ShutdownOperations for CaptureResources {
    fn stop_pipeline(&mut self) -> Result<(), CaptureError> {
        self.pipeline
            .set_state(gst::State::Null)
            .map(|_| ())
            .map_err(|error| gst_error("pipeline shutdown", error))
    }

    async fn close_portal(&mut self) -> Result<(), CaptureError> {
        self.grant.close().await
    }
}

impl GStreamerFrameSource {
    pub async fn open(grant: PortalGrant) -> Result<Self, CaptureError> {
        if let Err(error) = gst::init().map_err(|error| gst_error("initialization", error)) {
            return Err(cleanup_grant_after_failure(error, &grant).await);
        }
        let pipeline = gst::Pipeline::default();
        let tail = match build_pipeline(&pipeline, &grant) {
            Ok(tail) => tail,
            Err(primary) => {
                let mut resources = CaptureResources { pipeline, grant };
                return Err(cleanup_failed_setup(primary, &mut resources).await);
            }
        };

        Ok(Self {
            resources: CaptureResources { pipeline, grant },
            appsink: tail.appsink,
            expected_dimensions: tail.expected_dimensions,
            last_frame: None,
            shutdown: ShutdownState::default(),
        })
    }

    pub async fn shutdown(&mut self) -> Result<(), CaptureError> {
        shutdown_resources(&mut self.resources, &mut self.shutdown).await
    }
}

fn build_pipeline(pipeline: &gst::Pipeline, grant: &PortalGrant) -> Result<RgbTail, CaptureError> {
    let pipewire = gst::ElementFactory::make("pipewiresrc")
        .property("fd", grant.remote_fd.as_raw_fd())
        .property("path", grant.stream.pipe_wire_node_id().to_string())
        .property("do-timestamp", true)
        .build()
        .map_err(|error| gst_error("pipewiresrc construction", error))?;
    // Some PipeWire plugin builds (including Bazzite/Fedora 43) do not expose
    // this newer property. Those builds still report capture loss through EOS,
    // which the engine handles with the same blackout/cleanup path.
    if pipewire.find_property("on-disconnect").is_some() {
        pipewire.set_property_from_str("on-disconnect", "error");
    }
    // Bazzite's current Mesa/GStreamer stack negotiates the GL bridge but
    // downloads completely black frames. System-memory PipeWire buffers are
    // reliable there and also keep GNOME compositing while capture is active.
    // Retain the Fedora-tested bridge as an explicit compatibility option.
    let require_dmabuf = std::env::var_os("LOGIG560_ENABLE_DMABUF").is_some();
    let appsink = attach_rgb_tail_with_direct_scanout(pipeline, &pipewire, require_dmabuf)?;
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|error| gst_error("pipeline start", error))?;
    Ok(appsink)
}

#[cfg(test)]
fn attach_rgb_tail(
    pipeline: &gst::Pipeline,
    source: &gst::Element,
) -> Result<RgbTail, CaptureError> {
    attach_rgb_tail_with_direct_scanout(pipeline, source, false)
}

fn direct_scanout_input_caps() -> gst::Caps {
    gst::Caps::builder("video/x-raw")
        .features(["memory:DMABuf"])
        .field("format", "DMA_DRM")
        .field(
            "drm-format",
            gst::List::new(["XR24", "AR24", "XB24", "AB24", "NV12"]),
        )
        .build()
}

struct DirectScanoutBridge {
    input_filter: gst::Element,
    wall_clock_pacer: gst::Element,
    rate_limit: gst::Element,
    rate_filter: gst::Element,
    upload: gst::Element,
    upload_filter: gst::Element,
    color_convert: gst::Element,
    gl_output_filter: gst::Element,
    download: gst::Element,
    output_filter: gst::Element,
}

fn direct_scanout_bridge() -> Result<DirectScanoutBridge, CaptureError> {
    let make = |name, stage| {
        gst::ElementFactory::make(name)
            .build()
            .map_err(|error| gst_error(stage, error))
    };
    let input_filter = gst::ElementFactory::make("capsfilter")
        .property("caps", direct_scanout_input_caps())
        .build()
        .map_err(|error| gst_error("DMA-BUF capsfilter construction", error))?;
    let output_caps = gst::Caps::builder("video/x-raw")
        .field("format", "RGBA")
        .build();
    let output_filter = gst::ElementFactory::make("capsfilter")
        .property("caps", output_caps)
        .build()
        .map_err(|error| gst_error("system-memory capsfilter construction", error))?;
    let gl_output_caps = gst::Caps::builder("video/x-raw")
        .features(["memory:GLMemory"])
        .field("format", "RGBA")
        .field("texture-target", "2D")
        .build();
    let gl_output_filter = gst::ElementFactory::make("capsfilter")
        .property("caps", gl_output_caps)
        .build()
        .map_err(|error| gst_error("GL-memory capsfilter construction", error))?;
    let upload_caps = gst::Caps::builder("video/x-raw")
        .features(["memory:GLMemory"])
        .build();
    let upload_filter = gst::ElementFactory::make("capsfilter")
        .property("caps", upload_caps)
        .build()
        .map_err(|error| gst_error("GL-upload capsfilter construction", error))?;
    let rate_caps = gst::Caps::builder("video/x-raw")
        .features(["memory:DMABuf"])
        .field("format", "DMA_DRM")
        .field("framerate", gst::Fraction::new(20, 1))
        .build();
    Ok(DirectScanoutBridge {
        input_filter,
        // The upstream one-buffer leaky queue retains the newest frame while
        // this wall-clock delay prevents bad compositor timestamps from
        // bursting expensive GL conversions above the sampler's useful rate.
        wall_clock_pacer: gst::ElementFactory::make("identity")
            .property("sleep-time", 50_000_u32)
            .build()
            .map_err(|error| gst_error("DMA-BUF wall-clock pacer construction", error))?,
        rate_limit: gst::ElementFactory::make("videorate")
            .property("drop-only", true)
            .property("max-rate", 20_i32)
            .build()
            .map_err(|error| gst_error("DMA-BUF rate limiter construction", error))?,
        rate_filter: gst::ElementFactory::make("capsfilter")
            .property("caps", rate_caps)
            .build()
            .map_err(|error| gst_error("DMA-BUF rate capsfilter construction", error))?,
        upload: make("glupload", "GL upload construction")?,
        upload_filter,
        color_convert: make("glcolorconvert", "GL color conversion construction")?,
        gl_output_filter,
        download: make("gldownload", "GL download construction")?,
        output_filter,
    })
}

fn attach_rgb_tail_with_direct_scanout(
    pipeline: &gst::Pipeline,
    source: &gst::Element,
    require_dmabuf: bool,
) -> Result<RgbTail, CaptureError> {
    let direct_scanout = require_dmabuf.then(direct_scanout_bridge).transpose()?;
    let system_memory_pacer = direct_scanout
        .is_none()
        .then(|| {
            gst::ElementFactory::make("identity")
                .property("sleep-time", 50_000_u32)
                .build()
        })
        .transpose()
        .map_err(|error| gst_error("system-memory wall-clock pacer construction", error))?;
    let queue = gst::ElementFactory::make("queue")
        .property_from_str("leaky", "downstream")
        .property("max-size-buffers", 1_u32)
        .build()
        .map_err(|error| gst_error("queue construction", error))?;
    let convert = gst::ElementFactory::make("videoconvert")
        .build()
        .map_err(|error| gst_error("videoconvert construction", error))?;
    let scale = gst::ElementFactory::make("videoscale")
        .build()
        .map_err(|error| gst_error("videoscale construction", error))?;
    let initial_caps = gst::Caps::builder("video/x-raw")
        .field("format", "RGB")
        .field("width", 160_i32)
        .field("height", 90_i32)
        .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
        .build();
    let caps_filter = gst::ElementFactory::make("capsfilter")
        .property("caps", &initial_caps)
        .property_from_str("caps-change-mode", "delayed")
        .build()
        .map_err(|error| gst_error("capsfilter construction", error))?;
    let appsink = latest_frame_appsink();
    let expected_dimensions = Arc::new(AtomicU64::new(0));

    pipeline
        .add(source)
        .map_err(|error| gst_error("pipeline assembly", error))?;
    if let Some(bridge) = &direct_scanout {
        pipeline
            .add_many([
                &bridge.input_filter,
                &bridge.wall_clock_pacer,
                &bridge.rate_limit,
                &bridge.rate_filter,
                &bridge.upload,
                &bridge.upload_filter,
                &bridge.color_convert,
                &bridge.gl_output_filter,
                &bridge.download,
                &bridge.output_filter,
            ])
            .map_err(|error| gst_error("pipeline assembly", error))?;
    }
    if let Some(pacer) = &system_memory_pacer {
        pipeline
            .add(pacer)
            .map_err(|error| gst_error("pipeline assembly", error))?;
    }
    pipeline
        .add_many([&queue, &convert, &scale, &caps_filter, appsink.upcast_ref()])
        .map_err(|error| gst_error("pipeline assembly", error))?;
    if let Some(bridge) = &direct_scanout {
        // Link the GL bridge from downstream to upstream. GStreamer 1.26 can
        // otherwise propagate the DMA-BUF input caps through glupload before
        // its GLMemory output has been constrained, making a valid bridge
        // appear unlinkable during construction.
        gst::Element::link_many([
            &bridge.download,
            &bridge.output_filter,
            &convert,
            &scale,
            &caps_filter,
            appsink.upcast_ref(),
        ])
        .map_err(|error| gst_error("pipeline DMA-BUF bridge linking", error))?;
        gst::Element::link_many([
            &bridge.color_convert,
            &bridge.gl_output_filter,
            &bridge.download,
        ])
        .map_err(|error| gst_error("pipeline DMA-BUF bridge linking", error))?;
        gst::Element::link_many([&bridge.upload, &bridge.upload_filter, &bridge.color_convert])
            .map_err(|error| gst_error("pipeline DMA-BUF bridge linking", error))?;
        gst::Element::link_many([
            source,
            &bridge.input_filter,
            &queue,
            &bridge.wall_clock_pacer,
            &bridge.rate_limit,
            &bridge.rate_filter,
            &bridge.upload,
        ])
        .map_err(|error| gst_error("pipeline DMA-BUF bridge linking", error))?;
    } else {
        let pacer = system_memory_pacer
            .as_ref()
            .expect("system-memory capture has a wall-clock pacer");
        gst::Element::link_many([
            source,
            &queue,
            pacer,
            &convert,
            &scale,
            &caps_filter,
            appsink.upcast_ref(),
        ])
        .map_err(|error| gst_error("pipeline linking", error))?;
    }
    install_output_caps_probe(source, &caps_filter, &expected_dimensions)?;
    Ok(RgbTail {
        appsink,
        expected_dimensions,
    })
}

fn install_output_caps_probe(
    source: &gst::Element,
    caps_filter: &gst::Element,
    expected_dimensions: &Arc<AtomicU64>,
) -> Result<(), CaptureError> {
    let source_pad = source
        .static_pad("src")
        .ok_or_else(|| gst_error("source pad lookup", "source has no static src pad"))?;
    let caps_filter = caps_filter.clone();
    let coordinator = Arc::new(OutputCapsCoordinator::new(expected_dimensions.clone()));
    source_pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
        let Some(event) = info.event() else {
            return gst::PadProbeReturn::Ok;
        };
        let gst::EventView::Caps(caps_event) = event.view() else {
            return gst::PadProbeReturn::Ok;
        };

        match scaled_output_caps(caps_event.caps()) {
            Ok(update) => {
                if coordinator.request(update) {
                    let coordinator = coordinator.clone();
                    caps_filter.call_async(move |caps_filter| {
                        coordinator.drain(|update| {
                            if caps_filter.property::<gst::Caps>("caps") != update.caps {
                                caps_filter.set_property("caps", &update.caps);
                            }
                        });
                    });
                }
                gst::PadProbeReturn::Ok
            }
            Err(message) => {
                coordinator.invalidate();
                gst::element_error!(
                    caps_filter,
                    gst::StreamError::Format,
                    ("upstream video caps have no usable dimensions"),
                    ["{message}"]
                );
                gst::PadProbeReturn::Drop
            }
        }
    });
    Ok(())
}

fn scaled_output_caps(caps: &gst::CapsRef) -> Result<OutputCapsUpdate, String> {
    let info = VideoInfo::from_caps(caps).map_err(|_| format!("unsupported caps: {caps}"))?;
    let dimensions = proportional_dimensions(info.width(), info.height(), 160)
        .ok_or_else(|| format!("invalid dimensions in caps: {caps}"))?;
    OutputCapsUpdate::new(dimensions)
}

fn pack_dimensions((width, height): (u32, u32)) -> u64 {
    (u64::from(width) << 32) | u64::from(height)
}

async fn shutdown_resources<O: ShutdownOperations>(
    operations: &mut O,
    state: &mut ShutdownState,
) -> Result<(), CaptureError> {
    let mut failures = Vec::new();
    if !state.pipeline_stopped {
        match operations.stop_pipeline() {
            Ok(()) => state.pipeline_stopped = true,
            Err(error) => failures.push(format!("pipeline: {error}")),
        }
    }
    if !state.portal_closed {
        match operations.close_portal().await {
            Ok(()) => state.portal_closed = true,
            Err(error) => failures.push(format!("portal: {error}")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(CaptureError::Shutdown {
            message: failures.join("; "),
        })
    }
}

async fn cleanup_failed_setup<O: ShutdownOperations>(
    primary: CaptureError,
    operations: &mut O,
) -> CaptureError {
    let mut state = ShutdownState::default();
    match shutdown_resources(operations, &mut state).await {
        Ok(()) => primary,
        Err(cleanup) => with_cleanup_failure(primary, cleanup),
    }
}

async fn cleanup_grant_after_failure(primary: CaptureError, grant: &PortalGrant) -> CaptureError {
    match grant.close().await {
        Ok(()) => primary,
        Err(cleanup) => with_cleanup_failure(primary, cleanup),
    }
}

fn with_cleanup_failure(primary: CaptureError, cleanup: CaptureError) -> CaptureError {
    CaptureError::Cleanup {
        primary: Box::new(primary),
        cleanup: Box::new(cleanup),
    }
}

#[allow(deprecated)]
fn latest_frame_appsink() -> gst_app::AppSink {
    gst_app::AppSink::builder()
        .max_buffers(1)
        .drop(true)
        .sync(false)
        .enable_last_sample(false)
        .wait_on_eos(false)
        .build()
}

pub fn proportional_dimensions(
    source_width: u32,
    source_height: u32,
    target_width: u32,
) -> Option<(u32, u32)> {
    if source_width == 0 || source_height == 0 || target_width == 0 {
        return None;
    }
    let numerator = u64::from(source_height)
        .checked_mul(u64::from(target_width))?
        .checked_add(u64::from(source_width) / 2)?;
    let target_height = u32::try_from(numerator / u64::from(source_width)).ok()?;
    Some((target_width, target_height.max(1)))
}

impl Drop for GStreamerFrameSource {
    fn drop(&mut self) {
        if !self.shutdown.pipeline_stopped {
            let _ = self.resources.pipeline.set_state(gst::State::Null);
        }
    }
}

#[async_trait::async_trait]
impl FrameSource for GStreamerFrameSource {
    async fn next_frame(&mut self) -> anyhow::Result<Option<RgbFrame>> {
        let bus = self
            .resources
            .pipeline
            .bus()
            .ok_or_else(|| CaptureError::GStreamer {
                stage: "bus lookup",
                message: "pipeline has no bus".to_owned(),
            })?;
        let next = next_frame_from_sink(
            &self.appsink,
            &bus,
            &self.expected_dimensions,
            &mut self.last_frame,
        )
        .await;
        match next {
            Ok(frame) => Ok(Some(frame)),
            Err(CaptureError::EndOfStream) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    async fn shutdown(&mut self) -> anyhow::Result<()> {
        GStreamerFrameSource::shutdown(self)
            .await
            .map_err(Into::into)
    }
}

async fn next_frame_from_sink(
    appsink: &gst_app::AppSink,
    bus: &gst::Bus,
    expected_dimensions: &Arc<AtomicU64>,
    last_frame: &mut Option<RgbFrame>,
) -> Result<RgbFrame, CaptureError> {
    let waiting_since = Instant::now();
    loop {
        let appsink = appsink.clone();
        let bus = bus.clone();
        let poll_expected_dimensions = expected_dimensions.clone();
        let frame = tokio::task::spawn_blocking(move || {
            poll_frame(&appsink, &bus, &poll_expected_dimensions)
        })
        .await
        .map_err(|error| CaptureError::GStreamer {
            stage: "sample polling task",
            message: error.to_string(),
        })??;
        if let Some(frame) = frame {
            *last_frame = Some(frame.clone());
            return Ok(frame);
        }
        if waiting_since.elapsed() >= STATIC_FRAME_HOLD_INTERVAL {
            // GNOME's screencast source is damage-driven and can legitimately
            // stop delivering buffers while the monitor is static. Keep the
            // latest valid frame alive below the engine's 500 ms safety limit,
            // without racing or duplicating the normal ~20 FPS stream. Bus
            // errors and EOS are checked inside poll_frame first.
            if let Some(frame) = held_frame(last_frame, expected_dimensions) {
                return Ok(frame);
            }
        }
    }
}

fn held_frame(
    last_frame: &Option<RgbFrame>,
    expected_dimensions: &Arc<AtomicU64>,
) -> Option<RgbFrame> {
    let frame = last_frame.as_ref()?;
    let width = u32::try_from(frame.width).ok()?;
    let height = u32::try_from(frame.height).ok()?;
    let expected = expected_dimensions.load(Ordering::Acquire);
    (expected != 0 && pack_dimensions((width, height)) == expected).then(|| frame.clone())
}

fn poll_frame(
    appsink: &gst_app::AppSink,
    bus: &gst::Bus,
    expected_dimensions: &Arc<AtomicU64>,
) -> Result<Option<RgbFrame>, CaptureError> {
    if let Some(error) = terminal_pipeline_error(bus) {
        return Err(error);
    }
    if let Some(sample) = appsink.try_pull_sample(gst::ClockTime::from_mseconds(50)) {
        return frame_from_expected_sample(&sample, expected_dimensions);
    }
    if let Some(error) = terminal_pipeline_error(bus) {
        return Err(error);
    }
    if appsink.is_eos() {
        return Err(CaptureError::EndOfStream);
    }
    Ok(None)
}

fn frame_from_expected_sample(
    sample: &gst::Sample,
    expected_dimensions: &Arc<AtomicU64>,
) -> Result<Option<RgbFrame>, CaptureError> {
    let expected = expected_dimensions.load(Ordering::Acquire);
    if expected == 0 {
        return Ok(None);
    }
    let caps = sample
        .caps()
        .ok_or(CaptureError::MissingSampleData { missing: "caps" })?;
    let info = VideoInfo::from_caps(caps).map_err(|_| CaptureError::UnsupportedCaps {
        caps: caps.to_string(),
    })?;
    if pack_dimensions((info.width(), info.height())) != expected {
        return Ok(None);
    }
    let frame = frame_from_sample(sample)?;
    if expected_dimensions.load(Ordering::Acquire) != expected {
        return Ok(None);
    }
    Ok(Some(frame))
}

fn frame_from_sample(sample: &gst::Sample) -> Result<RgbFrame, CaptureError> {
    let caps = sample
        .caps()
        .ok_or(CaptureError::MissingSampleData { missing: "caps" })?;
    let caps_text = caps.to_string();
    let info = VideoInfo::from_caps(caps).map_err(|_| CaptureError::UnsupportedCaps {
        caps: caps_text.clone(),
    })?;
    if info.format() != VideoFormat::Rgb {
        return Err(CaptureError::UnsupportedCaps { caps: caps_text });
    }

    let width = usize::try_from(info.width()).map_err(|error| invalid_layout(error.to_string()))?;
    let height =
        usize::try_from(info.height()).map_err(|error| invalid_layout(error.to_string()))?;
    let buffer = sample
        .buffer()
        .ok_or(CaptureError::MissingSampleData { missing: "buffer" })?;
    let video_frame = VideoFrameRef::from_buffer_ref_readable(buffer, &info)
        .map_err(|error| invalid_layout(error.to_string()))?;
    let stride = usize::try_from(video_frame.plane_stride()[0])
        .map_err(|_| invalid_layout("RGB stride is negative"))?;
    let byte_len = stride
        .checked_mul(height)
        .ok_or_else(|| invalid_layout("frame byte length overflow"))?;
    let plane = video_frame
        .plane_data(0)
        .map_err(|error| invalid_layout(error.to_string()))?;
    let pixels = plane
        .get(..byte_len)
        .ok_or_else(|| {
            invalid_layout(format!(
                "mapped RGB plane has {} bytes but layout requires {byte_len}",
                plane.len()
            ))
        })?
        .to_vec();

    RgbFrame::new(width, height, stride, pixels).map_err(|error| invalid_layout(error.to_string()))
}

fn terminal_pipeline_error(bus: &gst::Bus) -> Option<CaptureError> {
    let message = bus.pop_filtered(&[gst::MessageType::Error, gst::MessageType::Eos])?;
    Some(match message.view() {
        gst::MessageView::Error(error) => CaptureError::Pipeline {
            element: message
                .src()
                .map(|source| source.path_string().to_string())
                .unwrap_or_else(|| "unknown".to_owned()),
            message: error.error().to_string(),
            debug: error
                .debug()
                .map(|debug| format!(" ({debug})"))
                .unwrap_or_default(),
        },
        gst::MessageView::Eos(_) => CaptureError::EndOfStream,
        _ => unreachable!("bus was filtered to terminal messages"),
    })
}

fn invalid_layout(message: impl Into<String>) -> CaptureError {
    CaptureError::InvalidFrameLayout {
        message: message.into(),
    }
}

fn gst_error(stage: &'static str, error: impl std::fmt::Display) -> CaptureError {
    CaptureError::GStreamer {
        stage,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicU64, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };

    use gst::prelude::*;
    use gstreamer as gst;
    use gstreamer_app as gst_app;
    use gstreamer_video::{VideoFormat, VideoFrameFlags, VideoInfo, VideoMeta};

    use super::{CaptureError, direct_scanout_input_caps, frame_from_sample};

    #[test]
    fn direct_scanout_input_requires_dmabuf_memory() {
        gst::init().unwrap();
        let caps = direct_scanout_input_caps();

        assert_eq!(caps.size(), 1);
        assert_eq!(caps.structure(0).unwrap().name(), "video/x-raw");
        assert!(caps.features(0).unwrap().contains("memory:DMABuf"));
        assert_eq!(
            caps.structure(0).unwrap().get::<&str>("format").unwrap(),
            "DMA_DRM"
        );
        let formats = caps
            .structure(0)
            .unwrap()
            .get::<gst::List>("drm-format")
            .unwrap();
        assert!(formats.as_slice().iter().all(|format| {
            matches!(
                format.get::<&str>(),
                Ok("XR24" | "AR24" | "XB24" | "AB24" | "NV12")
            )
        }));
    }

    #[test]
    fn direct_scanout_bridge_links_dmabuf_input_to_system_rgb_output() {
        gst::init().unwrap();
        let pipeline = gst::Pipeline::default();
        let source = gst::ElementFactory::make("fakesrc").build().unwrap();

        let tail = super::attach_rgb_tail_with_direct_scanout(&pipeline, &source, true).unwrap();

        assert!(pipeline.children().contains(&tail.appsink.upcast()));
    }

    #[test]
    fn direct_scanout_bridge_drops_before_conversion_at_sampler_rate() {
        gst::init().unwrap();
        let pipeline = gst::Pipeline::default();
        let source = gst::ElementFactory::make("fakesrc").build().unwrap();

        super::attach_rgb_tail_with_direct_scanout(&pipeline, &source, true).unwrap();

        let rate_limit = pipeline
            .children()
            .into_iter()
            .find(|element| {
                element
                    .factory()
                    .is_some_and(|factory| factory.name() == "videorate")
            })
            .expect("the live DMA-BUF path must rate-limit before conversion");
        let wall_clock_pacer = pipeline
            .children()
            .into_iter()
            .find(|element| {
                element
                    .factory()
                    .is_some_and(|factory| factory.name() == "identity")
            })
            .expect("the live DMA-BUF path must pace timestamp bursts before conversion");
        assert_eq!(wall_clock_pacer.property::<u32>("sleep-time"), 50_000);
        assert!(rate_limit.property::<bool>("drop-only"));
        assert_eq!(rate_limit.property::<i32>("max-rate"), 20);
        let rate_caps = pipeline
            .children()
            .into_iter()
            .filter(|element| {
                element
                    .factory()
                    .is_some_and(|factory| factory.name() == "capsfilter")
            })
            .filter_map(|element| element.property::<Option<gst::Caps>>("caps"))
            .find(|caps| {
                caps.structure(0).is_some_and(|structure| {
                    structure.get::<gst::Fraction>("framerate") == Ok(gst::Fraction::new(20, 1))
                })
            })
            .expect("videorate must negotiate a fixed 20 FPS output cap");
        assert!(rate_caps.features(0).unwrap().contains("memory:DMABuf"));
        assert_eq!(
            rate_caps
                .structure(0)
                .unwrap()
                .get::<&str>("format")
                .unwrap(),
            "DMA_DRM"
        );
        assert_eq!(
            source
                .static_pad("src")
                .unwrap()
                .peer()
                .unwrap()
                .parent_element()
                .unwrap()
                .factory()
                .unwrap()
                .name(),
            "capsfilter"
        );
        assert_eq!(
            rate_limit
                .static_pad("sink")
                .unwrap()
                .peer()
                .unwrap()
                .parent_element()
                .unwrap()
                .factory()
                .unwrap()
                .name(),
            "identity"
        );
        assert_eq!(
            wall_clock_pacer
                .static_pad("sink")
                .unwrap()
                .peer()
                .unwrap()
                .parent_element()
                .unwrap()
                .factory()
                .unwrap()
                .name(),
            "queue"
        );
    }
    use crate::RgbFrame;

    fn sample(format: &str, bytes: Vec<u8>) -> gst::Sample {
        gst::init().unwrap();
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", format)
            .field("width", 2_i32)
            .field("height", 2_i32)
            .field("framerate", gst::Fraction::new(60, 1))
            .build();
        let buffer = gst::Buffer::from_slice(bytes);
        gst::Sample::builder().buffer(&buffer).caps(&caps).build()
    }

    #[test]
    fn copies_padded_rgb_frame_with_reported_stride() {
        let pixels = vec![
            1, 2, 3, 4, 5, 6, 0xaa, 0xbb, 7, 8, 9, 10, 11, 12, 0xcc, 0xdd,
        ];

        let frame = frame_from_sample(&sample("RGB", pixels.clone())).unwrap();

        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.stride, 8);
        assert_eq!(frame.pixels, pixels);
    }

    #[test]
    fn rejects_unsupported_video_format_with_typed_error() {
        let error = frame_from_sample(&sample("I420", vec![0; 16])).unwrap_err();

        assert!(matches!(error, CaptureError::UnsupportedCaps { .. }));
    }

    #[test]
    fn proportional_dimensions_use_actual_portrait_and_ultrawide_caps() {
        assert_eq!(
            super::proportional_dimensions(1080, 1920, 160),
            Some((160, 284))
        );
        assert_eq!(
            super::proportional_dimensions(2100, 900, 160),
            Some((160, 69))
        );
        assert_eq!(super::proportional_dimensions(0, 900, 160), None);
    }

    #[test]
    fn actual_upstream_portrait_caps_scale_to_width_160() {
        let frame = frame_through_scaled_rgb_pipeline((90, 160));

        assert_eq!((frame.width, frame.height), (160, 284));
    }

    #[test]
    fn actual_upstream_ultrawide_caps_scale_to_width_160() {
        let frame = frame_through_scaled_rgb_pipeline((210, 90));

        assert_eq!((frame.width, frame.height), (160, 69));
    }

    #[test]
    fn portal_metadata_mismatch_does_not_affect_scaled_output() {
        let portal_metadata = (1920, 1080);
        let upstream_dimensions = (90, 160);
        assert_ne!(
            portal_metadata.0 * upstream_dimensions.1,
            portal_metadata.1 * upstream_dimensions.0
        );

        let frame = frame_through_scaled_rgb_pipeline(upstream_dimensions);

        assert_eq!((frame.width, frame.height), (160, 284));
    }

    #[test]
    fn caps_renegotiation_recalculates_scaled_output() {
        let (pipeline, appsrc, tail) = scaled_rgb_pipeline();
        pipeline.set_state(gst::State::Playing).unwrap();

        push_source_frame(&appsrc, (90, 160));
        let portrait = pull_gated_frame(&tail);
        push_source_frame(&appsrc, (210, 90));
        let ultrawide = pull_gated_frame(&tail);

        pipeline.set_state(gst::State::Null).unwrap();
        assert_eq!((portrait.width, portrait.height), (160, 284));
        assert_eq!((ultrawide.width, ultrawide.height), (160, 69));
    }

    #[test]
    fn output_gate_drops_a_sample_that_does_not_match_calculated_caps() {
        let expected = Arc::new(AtomicU64::new(super::pack_dimensions((160, 90))));
        let full_resolution = sample_with_dimensions("RGB", 1920, 1080, vec![0; 1920 * 1080 * 3]);

        let frame = super::frame_from_expected_sample(&full_resolution, &expected).unwrap();

        assert!(frame.is_none());
    }

    #[test]
    fn newer_caps_request_prevents_delayed_update_from_reopening_stale_gate() {
        gst::init().unwrap();
        let expected_dimensions = Arc::new(AtomicU64::new(0));
        let coordinator = Arc::new(super::OutputCapsCoordinator::new(
            expected_dimensions.clone(),
        ));
        let portrait = super::OutputCapsUpdate::new((160, 284)).unwrap();
        let ultrawide = super::OutputCapsUpdate::new((160, 69)).unwrap();
        assert!(coordinator.request(portrait));

        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let applied = Arc::new(Mutex::new(Vec::new()));
        let worker = coordinator.clone();
        let worker_expected = expected_dimensions.clone();
        let worker_applied = applied.clone();
        let handle = thread::spawn(move || {
            worker.drain(|update| {
                if update.dimensions == (160, 284) {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    assert_eq!(worker_expected.load(Ordering::Acquire), 0);
                } else {
                    assert_eq!(update.dimensions, (160, 69));
                    assert_eq!(worker_expected.load(Ordering::Acquire), 0);
                }
                worker_applied.lock().unwrap().push(update.dimensions);
            });
        });

        started_rx.recv().unwrap();
        assert!(!coordinator.request(ultrawide));
        assert_eq!(expected_dimensions.load(Ordering::Acquire), 0);
        release_tx.send(()).unwrap();
        handle.join().unwrap();

        assert_eq!(applied.lock().unwrap().as_slice(), &[(160, 284), (160, 69)]);
        assert_eq!(
            expected_dimensions.load(Ordering::Acquire),
            super::pack_dimensions((160, 69))
        );
    }

    fn frame_through_scaled_rgb_pipeline(dimensions: (usize, usize)) -> RgbFrame {
        let (pipeline, appsrc, tail) = scaled_rgb_pipeline();
        pipeline.set_state(gst::State::Playing).unwrap();
        push_source_frame(&appsrc, dimensions);
        let frame = pull_gated_frame(&tail);
        pipeline.set_state(gst::State::Null).unwrap();
        frame
    }

    fn scaled_rgb_pipeline() -> (gst::Pipeline, gst_app::AppSrc, super::RgbTail) {
        gst::init().unwrap();
        let appsrc = gst_app::AppSrc::builder().format(gst::Format::Time).build();
        let pipeline = gst::Pipeline::default();
        let tail = super::attach_rgb_tail(&pipeline, appsrc.upcast_ref()).unwrap();
        (pipeline, appsrc, tail)
    }

    fn push_source_frame(appsrc: &gst_app::AppSrc, (width, height): (usize, usize)) {
        let caps = rgb_caps(width, height);
        let info = VideoInfo::from_caps(&caps).unwrap();
        appsrc.set_caps(Some(&caps));
        appsrc
            .push_buffer(gst::Buffer::from_slice(vec![0; info.size()]))
            .unwrap();
    }

    fn pull_gated_frame(tail: &super::RgbTail) -> RgbFrame {
        for _ in 0..10 {
            let sample = tail
                .appsink
                .try_pull_sample(gst::ClockTime::from_mseconds(100))
                .expect("pipeline should produce one RGB sample");
            if let Some(frame) =
                super::frame_from_expected_sample(&sample, &tail.expected_dimensions).unwrap()
            {
                return frame;
            }
        }
        panic!("pipeline did not produce a sample with the calculated output caps");
    }

    fn rgb_caps(width: usize, height: usize) -> gst::Caps {
        gst::Caps::builder("video/x-raw")
            .field("format", "RGB")
            .field("width", i32::try_from(width).unwrap())
            .field("height", i32::try_from(height).unwrap())
            .field("framerate", gst::Fraction::new(60, 1))
            .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
            .build()
    }

    fn sample_with_dimensions(
        format: &str,
        width: usize,
        height: usize,
        bytes: Vec<u8>,
    ) -> gst::Sample {
        gst::init().unwrap();
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", format)
            .field("width", i32::try_from(width).unwrap())
            .field("height", i32::try_from(height).unwrap())
            .field("framerate", gst::Fraction::new(60, 1))
            .build();
        let buffer = gst::Buffer::from_slice(bytes);
        gst::Sample::builder().buffer(&buffer).caps(&caps).build()
    }

    #[test]
    fn video_meta_stride_and_offset_override_canonical_caps_layout() {
        gst::init().unwrap();
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGB")
            .field("width", 2_i32)
            .field("height", 2_i32)
            .field("framerate", gst::Fraction::new(60, 1))
            .build();
        let expected = vec![
            1, 2, 3, 4, 5, 6, 0xaa, 0xbb, 0xcc, 0xdd, 7, 8, 9, 10, 11, 12, 0xee, 0xff, 0x88, 0x99,
        ];
        let mut bytes = vec![0x42; 4];
        bytes.extend_from_slice(&expected);
        let mut buffer = gst::Buffer::from_slice(bytes);
        VideoMeta::add_full(
            buffer.get_mut().unwrap(),
            VideoFrameFlags::empty(),
            VideoFormat::Rgb,
            2,
            2,
            &[4],
            &[10],
        )
        .unwrap();
        let sample = gst::Sample::builder().buffer(&buffer).caps(&caps).build();

        let frame = frame_from_sample(&sample).unwrap();

        assert_eq!(frame.stride, 10);
        assert_eq!(frame.pixels, expected);
    }

    #[tokio::test]
    async fn bus_error_without_a_sample_returns_within_poll_interval() {
        gst::init().unwrap();
        let appsink = super::latest_frame_appsink();
        let pipeline = gst::Pipeline::default();
        pipeline.add(&appsink).unwrap();
        let bus = pipeline.bus().unwrap();
        bus.post(
            gst::message::Error::builder(gst::CoreError::Failed, "fake pipeline failure").build(),
        )
        .unwrap();
        let mut last_frame = None;

        let error = tokio::time::timeout(
            Duration::from_millis(250),
            super::next_frame_from_sink(
                &appsink,
                &bus,
                &Arc::new(AtomicU64::new(0)),
                &mut last_frame,
            ),
        )
        .await
        .expect("bus errors must not wait indefinitely")
        .unwrap_err();

        assert!(matches!(error, CaptureError::Pipeline { .. }));
    }

    #[tokio::test]
    async fn healthy_static_pipeline_repeats_its_last_valid_frame() {
        let (pipeline, appsrc, tail) = scaled_rgb_pipeline();
        pipeline.set_state(gst::State::Playing).unwrap();
        push_source_frame(&appsrc, (160, 90));
        let bus = pipeline.bus().unwrap();
        let mut last_frame = None;

        let first = tokio::time::timeout(
            Duration::from_millis(500),
            super::next_frame_from_sink(
                &tail.appsink,
                &bus,
                &tail.expected_dimensions,
                &mut last_frame,
            ),
        )
        .await
        .expect("the initial source frame should arrive")
        .unwrap();
        let repeat_started = std::time::Instant::now();
        let repeated = tokio::time::timeout(
            Duration::from_millis(400),
            super::next_frame_from_sink(
                &tail.appsink,
                &bus,
                &tail.expected_dimensions,
                &mut last_frame,
            ),
        )
        .await
        .expect("a connected static source should repeat before the stall timeout")
        .unwrap();

        pipeline.set_state(gst::State::Null).unwrap();
        assert_eq!(repeated, first);
        assert!(
            repeat_started.elapsed() >= super::STATIC_FRAME_HOLD_INTERVAL,
            "a held frame must not race the normal capture cadence"
        );
    }

    #[test]
    fn held_frame_is_blocked_during_caps_renegotiation() {
        let frame = RgbFrame::new(160, 90, 480, vec![0; 160 * 90 * 3]).unwrap();
        let expected = Arc::new(AtomicU64::new(0));

        assert!(super::held_frame(&Some(frame.clone()), &expected).is_none());
        expected.store(super::pack_dimensions((160, 69)), Ordering::Release);
        assert!(super::held_frame(&Some(frame.clone()), &expected).is_none());
        expected.store(super::pack_dimensions((160, 90)), Ordering::Release);
        assert_eq!(
            super::held_frame(&Some(frame.clone()), &expected),
            Some(frame)
        );
    }

    #[derive(Default)]
    struct FakeShutdownOperations {
        pipeline_stops: usize,
        portal_closes: usize,
        fail_pipeline_once: bool,
    }

    #[async_trait::async_trait]
    impl super::ShutdownOperations for FakeShutdownOperations {
        fn stop_pipeline(&mut self) -> Result<(), CaptureError> {
            self.pipeline_stops += 1;
            if std::mem::take(&mut self.fail_pipeline_once) {
                return Err(CaptureError::GStreamer {
                    stage: "fake shutdown",
                    message: "failed once".to_owned(),
                });
            }
            Ok(())
        }

        async fn close_portal(&mut self) -> Result<(), CaptureError> {
            self.portal_closes += 1;
            Ok(())
        }
    }

    #[tokio::test]
    async fn shutdown_attempts_both_operations_and_is_safe_to_repeat() {
        let mut operations = FakeShutdownOperations {
            fail_pipeline_once: true,
            ..Default::default()
        };
        let mut state = super::ShutdownState::default();

        assert!(
            super::shutdown_resources(&mut operations, &mut state)
                .await
                .is_err()
        );
        assert_eq!(
            (operations.pipeline_stops, operations.portal_closes),
            (1, 1)
        );

        super::shutdown_resources(&mut operations, &mut state)
            .await
            .unwrap();
        super::shutdown_resources(&mut operations, &mut state)
            .await
            .unwrap();
        assert_eq!(
            (operations.pipeline_stops, operations.portal_closes),
            (2, 1)
        );
    }

    #[tokio::test]
    async fn failed_pipeline_setup_stops_partial_pipeline_and_closes_portal() {
        let mut operations = FakeShutdownOperations::default();
        let primary = CaptureError::GStreamer {
            stage: "injected pipeline setup",
            message: "failed".to_owned(),
        };

        let error = super::cleanup_failed_setup(primary, &mut operations).await;

        assert!(matches!(error, CaptureError::GStreamer { .. }));
        assert_eq!(
            (operations.pipeline_stops, operations.portal_closes),
            (1, 1)
        );
    }

    #[test]
    #[allow(deprecated)]
    fn appsink_keeps_only_latest_frame_without_retaining_a_last_sample() {
        gst::init().unwrap();
        let appsink = super::latest_frame_appsink();

        assert_eq!(appsink.property::<u32>("max-buffers"), 1);
        assert!(appsink.property::<bool>("drop"));
        assert!(!appsink.property::<bool>("sync"));
        assert!(!appsink.property::<bool>("enable-last-sample"));
        assert!(!appsink.property::<bool>("wait-on-eos"));
    }
}
