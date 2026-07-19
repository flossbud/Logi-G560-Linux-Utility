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
    pub fn open(grant: PortalGrant) -> Result<Self, CaptureError> {
        gst::init().map_err(|error| gst_error("initialization", error))?;
        let (output_width, output_height) = output_dimensions_hint(grant.stream.size());

        let pipewire = gst::ElementFactory::make("pipewiresrc")
            .property("fd", grant.remote_fd.as_raw_fd())
            .property("path", grant.stream.pipe_wire_node_id().to_string())
            .property("do-timestamp", true)
            .property_from_str("on-disconnect", "error")
            .build()
            .map_err(|error| gst_error("pipewiresrc construction", error))?;
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
        let caps = gst::Caps::builder("video/x-raw")
            .field("format", "RGB")
            .field("width", output_width)
            .field("height", output_height)
            .field("pixel-aspect-ratio", gst::Fraction::new(1, 1))
            .build();
        let caps_filter = gst::ElementFactory::make("capsfilter")
            .property("caps", &caps)
            .build()
            .map_err(|error| gst_error("capsfilter construction", error))?;
        let appsink = latest_frame_appsink();

        let pipeline = gst::Pipeline::default();
        pipeline
            .add_many([
                &pipewire,
                &queue,
                &convert,
                &scale,
                &caps_filter,
                appsink.upcast_ref(),
            ])
            .map_err(|error| gst_error("pipeline assembly", error))?;
        gst::Element::link_many([
            &pipewire,
            &queue,
            &convert,
            &scale,
            &caps_filter,
            appsink.upcast_ref(),
        ])
        .map_err(|error| gst_error("pipeline linking", error))?;
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|error| gst_error("pipeline start", error))?;

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

// Portal dimensions are compositor-coordinate hints, not captured-buffer layout.
// Actual frame dimensions and row layout always come from each negotiated sample.
fn output_dimensions_hint(size: Option<(i32, i32)>) -> (i32, i32) {
    const OUTPUT_WIDTH: i32 = 160;
    let Some((width, height)) = size.filter(|(width, height)| *width > 0 && *height > 0) else {
        return (OUTPUT_WIDTH, 90);
    };
    let scaled_height =
        (i64::from(height) * i64::from(OUTPUT_WIDTH) + i64::from(width) / 2) / i64::from(width);
    let scaled_height = i32::try_from(scaled_height).unwrap_or(90).max(1);
    (OUTPUT_WIDTH, scaled_height)
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
    use gstreamer_video::{VideoFormat, VideoFrameFlags, VideoMeta};

    use super::{CaptureError, frame_from_sample};

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
    fn output_dimensions_preserve_selected_monitor_aspect_ratio() {
        assert_eq!(super::output_dimensions_hint(Some((1920, 1080))), (160, 90));
        assert_eq!(super::output_dimensions_hint(Some((3440, 1440))), (160, 67));
        assert_eq!(
            super::output_dimensions_hint(Some((1080, 1920))),
            (160, 284)
        );
    }

    #[test]
    fn absent_monitor_metadata_uses_safe_output_dimensions() {
        assert_eq!(super::output_dimensions_hint(None), (160, 90));
    }

    #[test]
    fn negotiated_sample_caps_win_over_monitor_metadata() {
        assert_eq!(super::output_dimensions_hint(Some((1920, 1080))), (160, 90));
        let frame = frame_from_sample(&sample("RGB", vec![0; 16])).unwrap();

        assert_eq!((frame.width, frame.height), (2, 2));
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
