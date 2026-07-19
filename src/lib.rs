pub mod color;
pub mod frame;
pub mod sampler;

pub use color::{Rgb8, Zone, ZoneColors};
pub use frame::{Region, RgbFrame, default_regions};
pub use sampler::{SamplerConfig, sample_zones};
