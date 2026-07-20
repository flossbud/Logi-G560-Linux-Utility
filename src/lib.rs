pub mod capture;
pub mod color;
pub mod config;
pub mod engine;
pub mod frame;
pub mod latest;
pub mod sampler;
pub mod transition;
pub mod usb;

pub use color::{Rgb8, Zone, ZoneColors};
pub use config::{AppConfig, ConfigLoad, ConfigStore, FileConfigStore, config_path};
pub use engine::{
    CaptureRecoveryMetrics, CaptureRecoverySnapshot, EngineMetrics, EngineSnapshot, EngineStats,
    FrameSource, FrameSourceFactory, LightSink, LightUpdateStatus, RecoveringFrameSource,
    RecoveringLightSink, RecoveryMetrics, RecoverySnapshot, run_engine, run_engine_with_metrics,
};
pub use frame::{Point, Polygon, RgbFrame, ZoneLayout, ZoneMasks};
pub use latest::{LatestReceiver, LatestSender, latest_channel};
pub use logilightshow_api::{
    API_VERSION, ApiError, BUS_NAME, CaptureBackend, DiagnosticCounters, HealthState,
    INTERFACE_NAME, LightingMode, ManualZone, ManualZoneUpdate, OBJECT_PATH, RgbColor,
    ServiceSnapshot, ZoneColor, ZoneId, validate_manual_updates,
};
pub use sampler::{SamplerConfig, sample_zones};
pub use transition::{DEFAULT_TRANSITION_DURATION, TransitionController};
