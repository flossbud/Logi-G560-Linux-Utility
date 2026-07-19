use std::os::fd::AsRawFd;

use gstreamer::{self as gst, prelude::*};
use gstreamer_app as gst_app;
use gstreamer_video::{VideoFormat, VideoInfo};

use super::{CaptureError, portal::PortalGrant};
use crate::{FrameSource, RgbFrame};

pub struct GStreamerFrameSource {
    pipeline: gst::Pipeline,
    appsink: gst_app::AppSink,
    _grant: PortalGrant,
}

impl GStreamerFrameSource {
    pub fn open(grant: PortalGrant) -> Result<Self, CaptureError> {
        gst::init().map_err(|error| gst_error("initialization", error))?;
        let (output_width, output_height) = output_dimensions(grant.stream.size())?;

        let pipewire = gst::ElementFactory::make("pipewiresrc")
            .property("fd", grant.remote_fd.as_raw_fd())
            .property("path", grant.stream.pipe_wire_node_id().to_string())
            .property("do-timestamp", true)
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
            pipeline,
            appsink,
            _grant: grant,
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

fn output_dimensions(size: Option<(i32, i32)>) -> Result<(i32, i32), CaptureError> {
    const OUTPUT_WIDTH: i32 = 160;
    let Some((width, height)) = size.filter(|(width, height)| *width > 0 && *height > 0) else {
        return Err(CaptureError::InvalidMonitorSize { size });
    };
    let scaled_height =
        (i64::from(height) * i64::from(OUTPUT_WIDTH) + i64::from(width) / 2) / i64::from(width);
    let scaled_height = i32::try_from(scaled_height)
        .map_err(|_| CaptureError::InvalidMonitorSize { size })?
        .max(1);
    Ok((OUTPUT_WIDTH, scaled_height))
}

impl Drop for GStreamerFrameSource {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[async_trait::async_trait]
impl FrameSource for GStreamerFrameSource {
    async fn next_frame(&mut self) -> anyhow::Result<Option<RgbFrame>> {
        let appsink = self.appsink.clone();
        let bus = self.pipeline.bus().ok_or_else(|| CaptureError::GStreamer {
            stage: "bus lookup",
            message: "pipeline has no bus".to_owned(),
        })?;
        tokio::task::spawn_blocking(move || match appsink.pull_sample() {
            Ok(sample) => frame_from_sample(&sample),
            Err(_) => Err(terminal_pipeline_error(&bus)),
        })
        .await
        .map_err(|error| CaptureError::GStreamer {
            stage: "sample task",
            message: error.to_string(),
        })?
        .map(Some)
        .map_err(Into::into)
    }
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
    let stride =
        usize::try_from(info.stride()[0]).map_err(|_| invalid_layout("RGB stride is negative"))?;
    let offset = info.offset()[0];
    let byte_len = stride
        .checked_mul(height)
        .ok_or_else(|| invalid_layout("frame byte length overflow"))?;
    let end = offset
        .checked_add(byte_len)
        .ok_or_else(|| invalid_layout("frame buffer offset overflow"))?;

    let buffer = sample
        .buffer()
        .ok_or(CaptureError::MissingSampleData { missing: "buffer" })?;
    let mapped = buffer
        .map_readable()
        .map_err(|error| invalid_layout(error.to_string()))?;
    let pixels = mapped
        .as_slice()
        .get(offset..end)
        .ok_or_else(|| {
            invalid_layout(format!(
                "buffer has {} bytes but layout requires {end}",
                mapped.size()
            ))
        })?
        .to_vec();

    RgbFrame::new(width, height, stride, pixels).map_err(|error| invalid_layout(error.to_string()))
}

fn terminal_pipeline_error(bus: &gst::Bus) -> CaptureError {
    let Some(message) = bus.pop_filtered(&[gst::MessageType::Error, gst::MessageType::Eos]) else {
        return CaptureError::EndOfStream;
    };
    match message.view() {
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
    }
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
    use gst::prelude::*;
    use gstreamer as gst;

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
        assert_eq!(
            super::output_dimensions(Some((1920, 1080))).unwrap(),
            (160, 90)
        );
        assert_eq!(
            super::output_dimensions(Some((3440, 1440))).unwrap(),
            (160, 67)
        );
        assert_eq!(
            super::output_dimensions(Some((1080, 1920))).unwrap(),
            (160, 284)
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
