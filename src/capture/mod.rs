pub mod gstreamer;
pub mod portal;

pub use gstreamer::GStreamerFrameSource;
pub use portal::{PortalCapture, PortalGrant};

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("screen capture selection was cancelled")]
    CaptureCancelled,
    #[error("screen capture portal failed: {message}")]
    Portal { message: String },
    #[error("screen capture portal returned {count} streams; expected exactly one monitor")]
    UnexpectedStreamCount { count: usize },
    #[error("GStreamer {stage} failed: {message}")]
    GStreamer {
        stage: &'static str,
        message: String,
    },
    #[error("captured sample has no {missing}")]
    MissingSampleData { missing: &'static str },
    #[error("unsupported captured video caps: {caps}")]
    UnsupportedCaps { caps: String },
    #[error("captured RGB frame has invalid layout: {message}")]
    InvalidFrameLayout { message: String },
    #[error("screen capture stream ended")]
    EndOfStream,
    #[error("screen capture pipeline failed in {element}: {message}{debug}")]
    Pipeline {
        element: String,
        message: String,
        debug: String,
    },
    #[error("capture shutdown failed: {message}")]
    Shutdown { message: String },
    #[error("{primary}; cleanup after that failure also failed: {cleanup}")]
    Cleanup {
        primary: Box<CaptureError>,
        cleanup: Box<CaptureError>,
    },
}
