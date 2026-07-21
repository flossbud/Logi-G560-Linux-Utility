use serde::{Deserialize, Serialize};
use thiserror::Error;
use zvariant::Type;

pub mod protocol;

pub const API_VERSION: u32 = 1;
pub const BUS_NAME: &str = "org.logilightshow.Service1";
pub const OBJECT_PATH: &str = "/org/logilightshow/Service1";
pub const INTERFACE_NAME: &str = "org.logilightshow.Service1";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
#[zvariant(signature = "s", rename_all = "kebab-case")]
pub enum ZoneId {
    LeftRear,
    LeftFront,
    RightFront,
    RightRear,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
#[zvariant(signature = "s", rename_all = "kebab-case")]
pub enum LightingMode {
    Manual,
    ContentAware,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
#[zvariant(signature = "s", rename_all = "kebab-case")]
pub enum CaptureBackend {
    DesktopPortal,
    Gamescope,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
#[zvariant(signature = "s", rename_all = "kebab-case")]
pub enum HealthState {
    Unknown,
    Starting,
    Ready,
    Recovering,
    Unavailable,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
pub struct RgbColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
pub struct ManualZone {
    pub zone: ZoneId,
    pub color: RgbColor,
    pub brightness: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
pub struct ZoneColor {
    pub zone: ZoneId,
    pub color: RgbColor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
pub struct ManualZoneUpdate {
    pub zone: ZoneId,
    pub color: RgbColor,
    pub brightness: u8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
pub struct DiagnosticCounters {
    pub captured_frames: u64,
    pub newest_value_replacements: u64,
    pub capture_stalls: u64,
    pub usb_recoveries: u64,
    pub usb_report_failures: u64,
    pub capture_rate_millihertz: u64,
    pub last_successful_write_age_ms: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Type)]
pub struct ServiceSnapshot {
    pub api_version: u32,
    pub revision: u64,
    pub lights_enabled: bool,
    pub mode: LightingMode,
    pub manual_zones: Vec<ManualZone>,
    pub pending: bool,
    pub capture_backend: Option<CaptureBackend>,
    pub service_health: HealthState,
    pub device_health: HealthState,
    pub capture_health: HealthState,
    pub writer_health: HealthState,
    pub diagnostics: DiagnosticCounters,
    pub confirmed_colors: Vec<ZoneColor>,
    pub setup_complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq, Serialize, Deserialize)]
pub enum ApiError {
    #[error("manual zone update must contain at least one zone")]
    EmptyManualUpdate,
    #[error("manual zone update contains duplicate zone {0:?}")]
    DuplicateZone(ZoneId),
    #[error("brightness {0} is outside the range 0..=100")]
    InvalidBrightness(u8),
    #[error("the requested operation is unavailable in the current lighting mode")]
    ModeConflict,
}

pub fn validate_manual_updates(updates: &[ManualZoneUpdate]) -> Result<(), ApiError> {
    if updates.is_empty() {
        return Err(ApiError::EmptyManualUpdate);
    }

    let mut seen = [false; 4];
    for update in updates {
        if update.brightness > 100 {
            return Err(ApiError::InvalidBrightness(update.brightness));
        }

        let index = update.zone as usize;
        if seen[index] {
            return Err(ApiError::DuplicateZone(update.zone));
        }
        seen[index] = true;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_update_rejects_duplicate_zones() {
        let red = RgbColor {
            red: 255,
            green: 0,
            blue: 0,
        };
        let updates = vec![
            ManualZoneUpdate {
                zone: ZoneId::LeftFront,
                color: red,
                brightness: 100,
            },
            ManualZoneUpdate {
                zone: ZoneId::LeftFront,
                color: red,
                brightness: 50,
            },
        ];
        assert_eq!(
            validate_manual_updates(&updates),
            Err(ApiError::DuplicateZone(ZoneId::LeftFront))
        );
    }

    #[test]
    fn brightness_must_be_at_most_one_hundred() {
        let update = ManualZoneUpdate {
            zone: ZoneId::RightRear,
            color: RgbColor {
                red: 1,
                green: 2,
                blue: 3,
            },
            brightness: 101,
        };
        assert_eq!(
            validate_manual_updates(&[update]),
            Err(ApiError::InvalidBrightness(101))
        );
    }

    #[test]
    fn manual_update_rejects_an_empty_list() {
        assert_eq!(
            validate_manual_updates(&[]),
            Err(ApiError::EmptyManualUpdate)
        );
    }

    #[test]
    fn manual_update_accepts_a_nonempty_unique_subset() {
        let updates = vec![
            ManualZoneUpdate {
                zone: ZoneId::LeftRear,
                color: RgbColor {
                    red: 1,
                    green: 2,
                    blue: 3,
                },
                brightness: 0,
            },
            ManualZoneUpdate {
                zone: ZoneId::RightFront,
                color: RgbColor {
                    red: 4,
                    green: 5,
                    blue: 6,
                },
                brightness: 100,
            },
        ];

        assert_eq!(validate_manual_updates(&updates), Ok(()));
    }
}
