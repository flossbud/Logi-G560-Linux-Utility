use std::os::fd::AsRawFd;

use gstreamer::{self as gst, prelude::*};
use gstreamer_app as gst_app;
use gstreamer_video::{VideoFormat, VideoFrameExt, VideoFrameRef, VideoInfo};

use super::{CaptureError, portal::PortalGrant};
use crate::{FrameSource, RgbFrame};

pub struct GStreamerFrameSource {
    resources: CaptureResources,
    appsink: gst_app::AppSink,
    shutdown: ShutdownState,
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
        let appsink = match build_pipeline(&pipeline, &grant) {
            Ok(appsink) => appsink,
            Err(primary) => {
                let mut resources = CaptureResources { pipeline, grant };
                return Err(cleanup_failed_setup(primary, &mut resources).await);
            }
        };

        Ok(Self {
            resources: CaptureResources { pipeline, grant },
            appsink,
            shutdown: ShutdownState::default(),
        })
    }

    pub async fn shutdown(&mut self) -> Result<(), CaptureError> {
        shutdown_resources(&mut self.resources, &mut self.shutdown).await
    }
}

fn build_pipeline(
    pipeline: &gst::Pipeline,
    grant: &PortalGrant,
) -> Result<gst_app::AppSink, CaptureError> {
    let pipewire = gst::ElementFactory::make("pipewiresrc")
        .property("fd", grant.remote_fd.as_raw_fd())
        .property("path", grant.stream.pipe_wire_node_id().to_string())
        .property("do-timestamp", true)
        .property_from_str("on-disconnect", "error")
        .build()
        .map_err(|error| gst_error("pipewiresrc construction", error))?;
    let appsink = attach_rgb_tail(pipeline, &pipewire)?;
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|error| gst_error("pipeline start", error))?;
    Ok(appsink)
}

fn attach_rgb_tail(
    pipeline: &gst::Pipeline,
    source: &gst::Element,
) -> Result<gst_app::AppSink, CaptureError> {
    let queue = gst::ElementFactory::make("queue")
        .property_from_str("leaky", "downstream")
        .property("max-size-buffers", 1_u32)
        .build()
        .map_err(|error| gst_error("queue construction", error))?;
    let convert = gst::ElementFactory::make("videoconvert")
        .build()
        .map_err(|error| gst_error("videoconvert construction", error))?;
    let caps = gst::Caps::builder("video/x-raw")
        .field("format", "RGB")
        .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
        .build();
    let caps_filter = gst::ElementFactory::make("capsfilter")
        .property("caps", &caps)
        .build()
        .map_err(|error| gst_error("capsfilter construction", error))?;
    let appsink = latest_frame_appsink();

    pipeline
        .add_many([source, &queue, &convert, &caps_filter, appsink.upcast_ref()])
        .map_err(|error| gst_error("pipeline assembly", error))?;
    gst::Element::link_many([source, &queue, &convert, &caps_filter, appsink.upcast_ref()])
        .map_err(|error| gst_error("pipeline linking", error))?;
    Ok(appsink)
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
        next_frame_from_sink(&self.appsink, &bus)
            .await
            .map(Some)
            .map_err(Into::into)
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
) -> Result<RgbFrame, CaptureError> {
    loop {
        let appsink = appsink.clone();
        let bus = bus.clone();
        let frame = tokio::task::spawn_blocking(move || poll_frame(&appsink, &bus))
            .await
            .map_err(|error| CaptureError::GStreamer {
                stage: "sample polling task",
                message: error.to_string(),
            })??;
        if let Some(frame) = frame {
            return Ok(frame);
        }
    }
}

fn poll_frame(
    appsink: &gst_app::AppSink,
    bus: &gst::Bus,
) -> Result<Option<RgbFrame>, CaptureError> {
    if let Some(error) = terminal_pipeline_error(bus) {
        return Err(error);
    }
    if let Some(sample) = appsink.try_pull_sample(gst::ClockTime::from_mseconds(50)) {
        return frame_from_sample(&sample).map(Some);
    }
    if let Some(error) = terminal_pipeline_error(bus) {
        return Err(error);
    }
    if appsink.is_eos() {
        return Err(CaptureError::EndOfStream);
    }
    Ok(None)
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
    use std::time::Duration;

    use gst::prelude::*;
    use gstreamer as gst;
    use gstreamer_app as gst_app;
    use gstreamer_video::{VideoFormat, VideoFrameFlags, VideoInfo, VideoMeta};

    use super::{CaptureError, frame_from_sample};
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
    fn upstream_caps_win_when_portal_metadata_has_a_different_aspect() {
        gst::init().unwrap();
        let portal_metadata = (1920, 1080);
        let upstream_dimensions = (90, 160);
        assert_ne!(
            portal_metadata.0 * upstream_dimensions.1,
            portal_metadata.1 * upstream_dimensions.0
        );

        let frame = frame_through_rgb_pipeline(upstream_dimensions);

        assert_eq!((frame.width, frame.height), upstream_dimensions);
    }

    #[test]
    fn upstream_caps_preserve_portrait_and_ultrawide_without_portal_metadata() {
        for dimensions in [(90, 160), (210, 90)] {
            let frame = frame_through_rgb_pipeline(dimensions);

            assert_eq!((frame.width, frame.height), dimensions);
        }
    }

    fn frame_through_rgb_pipeline((width, height): (usize, usize)) -> RgbFrame {
        gst::init().unwrap();
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGB")
            .field("width", i32::try_from(width).unwrap())
            .field("height", i32::try_from(height).unwrap())
            .field("framerate", gst::Fraction::new(60, 1))
            .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
            .build();
        let info = VideoInfo::from_caps(&caps).unwrap();
        let appsrc = gst_app::AppSrc::builder()
            .caps(&caps)
            .format(gst::Format::Time)
            .build();
        let pipeline = gst::Pipeline::default();
        let appsink = super::attach_rgb_tail(&pipeline, appsrc.upcast_ref()).unwrap();
        pipeline.set_state(gst::State::Playing).unwrap();
        appsrc
            .push_buffer(gst::Buffer::from_slice(vec![0; info.size()]))
            .unwrap();
        let sample = appsink
            .try_pull_sample(gst::ClockTime::from_seconds(1))
            .expect("pipeline should produce one RGB sample");
        let frame = frame_from_sample(&sample).unwrap();
        pipeline.set_state(gst::State::Null).unwrap();
        frame
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

        let error = tokio::time::timeout(
            Duration::from_millis(250),
            super::next_frame_from_sink(&appsink, &bus),
        )
        .await
        .expect("bus errors must not wait indefinitely")
        .unwrap_err();

        assert!(matches!(error, CaptureError::Pipeline { .. }));
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
