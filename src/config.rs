use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};
use directories::BaseDirs;
use logig560_api::{LightingMode, ManualZone, RgbColor, ZoneId};
use serde::{Deserialize, Serialize};

pub const CONFIG_VERSION: u32 = 2;

const LOGICAL_ZONES: [ZoneId; 4] = [
    ZoneId::LeftRear,
    ZoneId::LeftFront,
    ZoneId::RightFront,
    ZoneId::RightRear,
];

const DEFAULT_COLOR: RgbColor = RgbColor {
    red: 0x14,
    green: 0xc8,
    blue: 0xf4,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    pub version: u32,
    pub lights_enabled: bool,
    pub mode: LightingMode,
    pub manual_zones: [ManualZone; 4],
    pub restore_token: Option<String>,
    pub setup_complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ConfigValidationError {
    #[error("unsupported configuration version {actual}; expected {expected}")]
    UnsupportedVersion { expected: u32, actual: u32 },
    #[error("configuration must contain exactly one {zone:?} zone; found {count}")]
    InvalidZoneCount { zone: ZoneId, count: u8 },
    #[error("brightness {brightness} for {zone:?} is outside the range 0..=100")]
    InvalidBrightness { zone: ZoneId, brightness: u8 },
}

impl AppConfig {
    pub fn validate(&self) -> Result<(), ConfigValidationError> {
        if self.version != CONFIG_VERSION {
            return Err(ConfigValidationError::UnsupportedVersion {
                expected: CONFIG_VERSION,
                actual: self.version,
            });
        }

        for zone in LOGICAL_ZONES {
            let count = self
                .manual_zones
                .iter()
                .filter(|setting| setting.zone == zone)
                .count() as u8;
            if count != 1 {
                return Err(ConfigValidationError::InvalidZoneCount { zone, count });
            }
        }

        for setting in self.manual_zones {
            if setting.brightness > 100 {
                return Err(ConfigValidationError::InvalidBrightness {
                    zone: setting.zone,
                    brightness: setting.brightness,
                });
            }
        }

        Ok(())
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            lights_enabled: true,
            mode: LightingMode::Manual,
            manual_zones: [
                default_zone(ZoneId::LeftRear),
                default_zone(ZoneId::LeftFront),
                default_zone(ZoneId::RightFront),
                default_zone(ZoneId::RightRear),
            ],
            restore_token: None,
            setup_complete: false,
        }
    }
}

fn default_zone(zone: ZoneId) -> ManualZone {
    ManualZone {
        zone,
        color: DEFAULT_COLOR,
        brightness: 100,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigLoad {
    Fresh(AppConfig),
    Loaded(AppConfig),
    Migrated {
        config: AppConfig,
        source: PathBuf,
    },
    Recovered {
        config: AppConfig,
        invalid_path: PathBuf,
    },
}

impl ConfigLoad {
    pub fn into_config(self) -> AppConfig {
        match self {
            Self::Fresh(config)
            | Self::Loaded(config)
            | Self::Migrated { config, .. }
            | Self::Recovered { config, .. } => config,
        }
    }
}

pub trait ConfigStore {
    fn load(&self) -> Result<ConfigLoad>;
    fn save(&self, config: &AppConfig) -> Result<()>;
}

#[derive(Clone, Debug)]
pub struct FileConfigStore {
    path: PathBuf,
}

impl FileConfigStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn legacy_path(&self) -> Result<PathBuf> {
        Ok(parent_directory(&self.path)?.join("capture.toml"))
    }

    fn load_legacy(&self) -> Result<Option<ConfigLoad>> {
        let source = self.legacy_path()?;
        let contents = match fs::read(&source) {
            Ok(contents) => contents,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).with_context(|| format!("read {}", source.display())),
        };
        let Ok(text) = std::str::from_utf8(&contents) else {
            return Ok(None);
        };
        let Ok(legacy) = toml::from_str::<LegacyCaptureConfig>(text) else {
            return Ok(None);
        };
        if legacy.version != 1 {
            return Ok(None);
        }

        let config = AppConfig {
            restore_token: legacy.restore_token,
            ..AppConfig::default()
        };
        Ok(Some(ConfigLoad::Migrated { config, source }))
    }

    fn preserve_invalid(&self) -> Result<PathBuf> {
        let parent = parent_directory(&self.path)?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before the Unix epoch")?
            .as_secs();

        for attempt in 0..100_u8 {
            let suffix = if attempt == 0 {
                String::new()
            } else {
                format!("-{attempt}")
            };
            let invalid_path = parent.join(format!("config.invalid-{timestamp}{suffix}.toml"));
            if invalid_path
                .try_exists()
                .with_context(|| format!("inspect {}", invalid_path.display()))?
            {
                continue;
            }
            fs::rename(&self.path, &invalid_path).with_context(|| {
                format!(
                    "preserve invalid configuration {} as {}",
                    self.path.display(),
                    invalid_path.display()
                )
            })?;
            return Ok(invalid_path);
        }

        bail!("could not allocate an invalid configuration path")
    }

    fn create_temporary(&self) -> Result<(PathBuf, fs::File)> {
        let parent = parent_directory(&self.path)?;
        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .context("configuration path has no UTF-8 file name")?;

        for attempt in 0..100_u8 {
            let suffix = if attempt == 0 {
                String::new()
            } else {
                format!("-{attempt}")
            };
            let temporary = parent.join(format!(".{file_name}.tmp-{}{suffix}", std::process::id()));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
            {
                Ok(file) => return Ok((temporary, file)),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(error).with_context(|| format!("open {}", temporary.display()));
                }
            }
        }

        bail!("could not allocate a private temporary configuration file")
    }
}

impl ConfigStore for FileConfigStore {
    fn load(&self) -> Result<ConfigLoad> {
        let contents = match fs::read(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(self
                    .load_legacy()?
                    .unwrap_or_else(|| ConfigLoad::Fresh(AppConfig::default())));
            }
            Err(error) => {
                return Err(error).with_context(|| format!("read {}", self.path.display()));
            }
        };

        let config = std::str::from_utf8(&contents)
            .ok()
            .and_then(|contents| toml::from_str::<AppConfig>(contents).ok())
            .filter(|config| config.validate().is_ok());
        match config {
            Some(config) => Ok(ConfigLoad::Loaded(config)),
            None => Ok(ConfigLoad::Recovered {
                config: AppConfig::default(),
                invalid_path: self.preserve_invalid()?,
            }),
        }
    }

    fn save(&self, config: &AppConfig) -> Result<()> {
        let parent = parent_directory(&self.path)?;
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        let contents = toml::to_string(config).context("serialize application configuration")?;
        let (temporary, mut file) = self.create_temporary()?;
        let save_result = (|| -> Result<()> {
            file.write_all(contents.as_bytes())
                .with_context(|| format!("write {}", temporary.display()))?;
            file.sync_all()
                .with_context(|| format!("sync {}", temporary.display()))?;
            fs::rename(&temporary, &self.path)
                .with_context(|| format!("replace {}", self.path.display()))?;
            fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .with_context(|| format!("sync {}", parent.display()))?;
            Ok(())
        })();
        if save_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        save_result
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyCaptureConfig {
    version: u32,
    restore_token: Option<String>,
}

fn parent_directory(path: &Path) -> Result<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("configuration path has no parent")
}

pub fn config_path() -> Result<PathBuf> {
    let base = BaseDirs::new().context("could not determine the user configuration directory")?;
    Ok(base.config_dir().join("logig560/config.toml"))
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use logig560_api::{LightingMode, ManualZone, RgbColor, ZoneId};

    use super::*;

    #[test]
    fn fresh_config_is_manual_cyan_and_lights_on() {
        let directory = tempfile::tempdir().unwrap();
        let store = FileConfigStore::new(directory.path().join("config.toml"));

        let ConfigLoad::Fresh(config) = store.load().unwrap() else {
            panic!("missing configuration should return fresh defaults");
        };

        let cyan = RgbColor {
            red: 0x14,
            green: 0xc8,
            blue: 0xf4,
        };
        assert_eq!(config.version, 2);
        assert!(config.lights_enabled);
        assert_eq!(config.mode, LightingMode::Manual);
        assert_eq!(
            config.manual_zones,
            [
                ManualZone {
                    zone: ZoneId::LeftRear,
                    color: cyan,
                    brightness: 100,
                },
                ManualZone {
                    zone: ZoneId::LeftFront,
                    color: cyan,
                    brightness: 100,
                },
                ManualZone {
                    zone: ZoneId::RightFront,
                    color: cyan,
                    brightness: 100,
                },
                ManualZone {
                    zone: ZoneId::RightRear,
                    color: cyan,
                    brightness: 100,
                },
            ]
        );
        assert_eq!(config.restore_token, None);
        assert!(!config.setup_complete);
    }

    #[test]
    fn app_config_validation_accepts_defaults() {
        assert_eq!(AppConfig::default().validate(), Ok(()));
    }

    #[test]
    fn app_config_validation_rejects_wrong_schema_version() {
        let config = AppConfig {
            version: 1,
            ..AppConfig::default()
        };

        assert_eq!(
            config.validate(),
            Err(ConfigValidationError::UnsupportedVersion {
                expected: CONFIG_VERSION,
                actual: 1,
            })
        );
    }

    #[test]
    fn app_config_validation_rejects_duplicate_logical_zone() {
        let mut config = AppConfig::default();
        config.manual_zones[3].zone = ZoneId::LeftRear;

        assert_eq!(
            config.validate(),
            Err(ConfigValidationError::InvalidZoneCount {
                zone: ZoneId::LeftRear,
                count: 2,
            })
        );
    }

    #[test]
    fn app_config_validation_rejects_missing_logical_zone() {
        let mut config = AppConfig::default();
        config.manual_zones[1].zone = ZoneId::RightRear;

        assert_eq!(
            config.validate(),
            Err(ConfigValidationError::InvalidZoneCount {
                zone: ZoneId::LeftFront,
                count: 0,
            })
        );
    }

    #[test]
    fn app_config_validation_rejects_brightness_above_one_hundred() {
        let mut config = AppConfig::default();
        config.manual_zones[2].brightness = 101;

        assert_eq!(
            config.validate(),
            Err(ConfigValidationError::InvalidBrightness {
                zone: ZoneId::RightFront,
                brightness: 101,
            })
        );
    }

    #[test]
    fn save_replaces_atomically_with_mode_0600() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let store = FileConfigStore::new(path.clone());
        let expected = AppConfig::default();

        store.save(&expected).unwrap();

        assert_eq!(store.load().unwrap(), ConfigLoad::Loaded(expected));
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn failed_parse_preserves_invalid_file_and_returns_safe_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let invalid = b"not = [valid";
        fs::write(&path, invalid).unwrap();
        let store = FileConfigStore::new(path);

        let ConfigLoad::Recovered {
            config,
            invalid_path,
        } = store.load().unwrap()
        else {
            panic!("malformed configuration should be recovered");
        };

        assert_eq!(fs::read(&invalid_path).unwrap(), invalid);
        assert_eq!(config, AppConfig::default());
    }

    #[test]
    fn failed_validation_preserves_exact_file_and_returns_safe_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let mut invalid_config = AppConfig::default();
        invalid_config.manual_zones[2].brightness = 101;
        let invalid = toml::to_string(&invalid_config).unwrap().into_bytes();
        fs::write(&path, &invalid).unwrap();
        let store = FileConfigStore::new(path);

        let ConfigLoad::Recovered {
            config,
            invalid_path,
        } = store.load().unwrap()
        else {
            panic!("semantically invalid configuration should be recovered");
        };

        assert_eq!(fs::read(&invalid_path).unwrap(), invalid);
        assert_eq!(config, AppConfig::default());
    }

    #[test]
    fn v1_capture_config_migrates_restore_token_without_losing_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let legacy_path = directory.path().join("capture.toml");
        fs::write(&legacy_path, "version = 1\nrestore_token = \"token\"").unwrap();
        let store = FileConfigStore::new(path);

        let ConfigLoad::Migrated { config, source } = store.load().unwrap() else {
            panic!("legacy capture configuration should be migrated");
        };

        assert_eq!(config.version, 2);
        assert_eq!(config.restore_token.as_deref(), Some("token"));
        assert_eq!(source, legacy_path);
    }
}
