use crate::{
    API_VERSION, ApiError, AppConfig, ConfigValidationError, DiagnosticCounters, HealthState,
    LightingMode, ManualZone, ManualZoneUpdate, Rgb8, ServiceSnapshot, Zone, ZoneColor, ZoneColors,
    validate_manual_updates,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelEffect {
    NoWrite,
    Write(ZoneColors),
    StartContentAware,
    StopContentAwareAndWrite(ZoneColors),
    PriorityBlackout,
}

#[derive(Clone, Debug)]
pub struct ControllerModel {
    config: AppConfig,
    confirmed_colors: ZoneColors,
    pending: bool,
    revision: u64,
    service_health: HealthState,
    device_health: HealthState,
    capture_health: HealthState,
    writer_health: HealthState,
    diagnostics: DiagnosticCounters,
}

impl ControllerModel {
    pub fn new(config: AppConfig) -> Result<Self, ConfigValidationError> {
        config.validate()?;
        Ok(Self {
            config,
            confirmed_colors: ZoneColors::BLACK,
            pending: false,
            revision: 0,
            service_health: HealthState::Starting,
            device_health: HealthState::Unknown,
            capture_health: HealthState::Unknown,
            writer_health: HealthState::Unknown,
            diagnostics: DiagnosticCounters {
                captured_frames: 0,
                newest_value_replacements: 0,
                capture_stalls: 0,
                usb_recoveries: 0,
                usb_report_failures: 0,
                capture_rate_millihertz: 0,
                last_successful_write_age_ms: None,
            },
        })
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn apply_manual_updates(
        &mut self,
        updates: &[ManualZoneUpdate],
    ) -> Result<ModelEffect, ApiError> {
        validate_manual_updates(updates)?;
        if self.config.mode == LightingMode::ContentAware {
            return Err(ApiError::ModeConflict);
        }

        let mut changed = false;
        for update in updates {
            let setting = self
                .config
                .manual_zones
                .iter_mut()
                .find(|setting| setting.zone == update.zone)
                .expect("application configuration must contain every logical zone");
            let replacement = ManualZone {
                zone: update.zone,
                color: update.color,
                brightness: update.brightness,
            };
            changed |= *setting != replacement;
            *setting = replacement;
        }
        self.advance_if(changed);

        if self.config.lights_enabled {
            Ok(ModelEffect::Write(self.manual_target()))
        } else {
            Ok(ModelEffect::NoWrite)
        }
    }

    pub fn set_mode(&mut self, mode: LightingMode) -> ModelEffect {
        if self.config.mode == mode {
            return ModelEffect::NoWrite;
        }

        let previous = self.config.mode;
        self.config.mode = mode;
        self.advance();

        match (previous, mode, self.config.lights_enabled) {
            (LightingMode::Manual, LightingMode::ContentAware, true) => {
                ModelEffect::StartContentAware
            }
            (LightingMode::Manual, LightingMode::ContentAware, false) => ModelEffect::NoWrite,
            (LightingMode::ContentAware, LightingMode::Manual, true) => {
                ModelEffect::StopContentAwareAndWrite(self.manual_target())
            }
            (LightingMode::ContentAware, LightingMode::Manual, false) => ModelEffect::NoWrite,
            _ => ModelEffect::NoWrite,
        }
    }

    pub fn set_lights_enabled(&mut self, enabled: bool) -> ModelEffect {
        if self.config.lights_enabled == enabled {
            return ModelEffect::NoWrite;
        }

        self.config.lights_enabled = enabled;
        self.advance();

        if !enabled {
            ModelEffect::PriorityBlackout
        } else {
            match self.config.mode {
                LightingMode::Manual => ModelEffect::Write(self.manual_target()),
                LightingMode::ContentAware => ModelEffect::StartContentAware,
            }
        }
    }

    pub fn mark_pending(&mut self) -> ModelEffect {
        if !self.pending {
            self.pending = true;
            self.advance();
        }
        ModelEffect::NoWrite
    }

    pub fn confirm_write(&mut self, colors: ZoneColors) -> ModelEffect {
        let changed = self.confirmed_colors != colors
            || self.pending
            || self.writer_health != HealthState::Ready;
        self.confirmed_colors = colors;
        self.pending = false;
        self.writer_health = HealthState::Ready;
        self.advance_if(changed);
        ModelEffect::NoWrite
    }

    pub fn record_failure(&mut self) -> ModelEffect {
        let changed = self.writer_health != HealthState::Failed;
        self.writer_health = HealthState::Failed;
        self.advance_if(changed);
        ModelEffect::NoWrite
    }

    pub fn mark_setup_complete(&mut self) -> ModelEffect {
        if !self.config.setup_complete {
            self.config.setup_complete = true;
            self.advance();
        }
        ModelEffect::NoWrite
    }

    pub fn snapshot(&self) -> ServiceSnapshot {
        ServiceSnapshot {
            api_version: API_VERSION,
            revision: self.revision,
            lights_enabled: self.config.lights_enabled,
            mode: self.config.mode,
            manual_zones: Zone::ALL
                .into_iter()
                .map(|zone| *self.manual_setting(zone))
                .collect(),
            pending: self.pending,
            capture_backend: None,
            service_health: self.service_health,
            device_health: self.device_health,
            capture_health: self.capture_health,
            writer_health: self.writer_health,
            diagnostics: self.diagnostics.clone(),
            confirmed_colors: Zone::ALL
                .into_iter()
                .map(|zone| ZoneColor {
                    zone: zone.into(),
                    color: self.confirmed_colors.get(zone).into(),
                })
                .collect(),
            setup_complete: self.config.setup_complete,
        }
    }

    pub fn manual_target(&self) -> ZoneColors {
        if !self.config.lights_enabled {
            return ZoneColors::BLACK;
        }

        ZoneColors(Zone::ALL.map(|zone| {
            let setting = self.manual_setting(zone);
            scale(setting.color.into(), setting.brightness)
        }))
    }

    fn manual_setting(&self, zone: Zone) -> &ManualZone {
        let zone = zone.into();
        self.config
            .manual_zones
            .iter()
            .find(|setting| setting.zone == zone)
            .expect("application configuration must contain every logical zone")
    }

    fn advance(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn advance_if(&mut self, changed: bool) {
        if changed {
            self.advance();
        }
    }
}

fn scale(color: Rgb8, brightness: u8) -> Rgb8 {
    fn channel(value: u8, brightness: u8) -> u8 {
        let scaled = (u16::from(value) * u16::from(brightness) + 50) / 100;
        scaled.min(u16::from(u8::MAX)) as u8
    }

    Rgb8 {
        r: channel(color.r, brightness),
        g: channel(color.g, brightness),
        b: channel(color.b, brightness),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::{
        ApiError, AppConfig, ConfigValidationError, HealthState, LightingMode, ManualZoneUpdate,
        Rgb8, RgbColor, Zone, ZoneColors, ZoneId,
    };

    const CYAN: Rgb8 = Rgb8 {
        r: 0x14,
        g: 0xc8,
        b: 0xf4,
    };
    const RED: Rgb8 = Rgb8 { r: 255, g: 0, b: 0 };

    fn red_update(zone: ZoneId) -> ManualZoneUpdate {
        ManualZoneUpdate {
            zone,
            color: RgbColor {
                red: 255,
                green: 0,
                blue: 0,
            },
            brightness: 100,
        }
    }

    #[test]
    fn brightness_scales_channels_with_rounding() {
        assert_eq!(
            scale(
                Rgb8 {
                    r: 255,
                    g: 127,
                    b: 1,
                },
                50,
            ),
            Rgb8 {
                r: 128,
                g: 64,
                b: 1
            }
        );
    }

    #[test]
    fn new_model_starts_black_unpending_and_with_initial_health() {
        let model = ControllerModel::new(AppConfig::default()).unwrap();

        let snapshot = model.snapshot();
        assert_eq!(snapshot.revision, 0);
        assert!(!snapshot.pending);
        assert_eq!(snapshot.confirmed_colors.len(), 4);
        assert!(
            snapshot
                .confirmed_colors
                .iter()
                .all(|zone| zone.color == RgbColor::from(Rgb8::BLACK))
        );
        assert_eq!(snapshot.service_health, HealthState::Starting);
        assert_eq!(snapshot.device_health, HealthState::Unknown);
        assert_eq!(snapshot.capture_health, HealthState::Unknown);
        assert_eq!(snapshot.writer_health, HealthState::Unknown);
    }

    #[test]
    fn new_model_rejects_invalid_config_instead_of_panicking_later() {
        let mut config = AppConfig::default();
        config.manual_zones[3].zone = ZoneId::LeftRear;

        assert_eq!(
            ControllerModel::new(config).unwrap_err(),
            ConfigValidationError::InvalidZoneCount {
                zone: ZoneId::LeftRear,
                count: 2,
            }
        );
    }

    #[test]
    fn grouped_update_changes_only_named_zones_atomically() {
        let mut model = ControllerModel::new(AppConfig::default()).unwrap();
        let effect = model
            .apply_manual_updates(&[
                red_update(ZoneId::LeftFront),
                red_update(ZoneId::RightFront),
            ])
            .unwrap();

        assert_eq!(
            effect,
            ModelEffect::Write(ZoneColors([CYAN, RED, RED, CYAN]))
        );
        assert_eq!(model.manual_target(), ZoneColors([CYAN, RED, RED, CYAN]));
        assert_eq!(model.snapshot().revision, 1);
    }

    #[test]
    fn invalid_grouped_update_changes_nothing() {
        let mut model = ControllerModel::new(AppConfig::default()).unwrap();
        let before = model.snapshot();
        let duplicate = [red_update(ZoneId::LeftFront), red_update(ZoneId::LeftFront)];

        assert_eq!(
            model.apply_manual_updates(&duplicate),
            Err(ApiError::DuplicateZone(ZoneId::LeftFront))
        );
        assert_eq!(model.snapshot(), before);
    }

    #[test]
    fn content_aware_rejects_manual_updates() {
        let config = AppConfig {
            mode: LightingMode::ContentAware,
            ..AppConfig::default()
        };
        let mut model = ControllerModel::new(config).unwrap();
        let before = model.snapshot();

        assert_eq!(
            model.apply_manual_updates(&[red_update(ZoneId::LeftFront)]),
            Err(ApiError::ModeConflict)
        );
        assert_eq!(model.snapshot(), before);
    }

    #[test]
    fn lights_off_saves_manual_updates_without_writing() {
        let config = AppConfig {
            lights_enabled: false,
            ..AppConfig::default()
        };
        let mut model = ControllerModel::new(config).unwrap();

        assert_eq!(
            model
                .apply_manual_updates(&[red_update(ZoneId::LeftFront)])
                .unwrap(),
            ModelEffect::NoWrite
        );
        assert_eq!(model.config().manual_zones[1].color, RgbColor::from(RED));
        assert_eq!(model.snapshot().revision, 1);
    }

    #[test]
    fn lights_off_retains_mode_and_manual_state_but_targets_black() {
        let mut model = ControllerModel::new(AppConfig::default()).unwrap();
        let saved_mode = model.config().mode;
        let saved_manual = model.config().manual_zones;

        assert_eq!(
            model.set_lights_enabled(false),
            ModelEffect::PriorityBlackout
        );

        assert_eq!(model.config().mode, saved_mode);
        assert_eq!(model.config().manual_zones, saved_manual);
        assert_eq!(model.manual_target(), ZoneColors::BLACK);
        assert_eq!(model.snapshot().revision, 1);
    }

    #[test]
    fn mode_switches_choose_the_required_runtime_effect() {
        let mut model = ControllerModel::new(AppConfig::default()).unwrap();

        assert_eq!(
            model.set_mode(LightingMode::ContentAware),
            ModelEffect::StartContentAware
        );
        assert_eq!(model.snapshot().revision, 1);
        assert_eq!(
            model.set_mode(LightingMode::Manual),
            ModelEffect::StopContentAwareAndWrite(ZoneColors([CYAN; 4]))
        );
        assert_eq!(model.snapshot().revision, 2);
    }

    #[test]
    fn no_op_mode_and_light_changes_do_not_advance_revision() {
        let mut model = ControllerModel::new(AppConfig::default()).unwrap();

        assert_eq!(model.set_mode(LightingMode::Manual), ModelEffect::NoWrite);
        assert_eq!(model.set_lights_enabled(true), ModelEffect::NoWrite);
        assert_eq!(model.snapshot().revision, 0);
    }

    #[test]
    fn lights_resume_the_saved_mode() {
        let mut manual = ControllerModel::new(AppConfig::default()).unwrap();
        manual.set_lights_enabled(false);
        assert_eq!(
            manual.set_lights_enabled(true),
            ModelEffect::Write(ZoneColors([CYAN; 4]))
        );

        let content_config = AppConfig {
            lights_enabled: false,
            mode: LightingMode::ContentAware,
            ..AppConfig::default()
        };
        let mut content = ControllerModel::new(content_config).unwrap();
        assert_eq!(
            content.set_lights_enabled(true),
            ModelEffect::StartContentAware
        );
    }

    #[test]
    fn switching_to_manual_while_off_stops_no_runtime_and_preserves_target() {
        let config = AppConfig {
            lights_enabled: false,
            mode: LightingMode::ContentAware,
            ..AppConfig::default()
        };
        let mut model = ControllerModel::new(config).unwrap();

        assert_eq!(model.set_mode(LightingMode::Manual), ModelEffect::NoWrite);
        assert_eq!(model.manual_target(), ZoneColors::BLACK);
        assert_eq!(model.snapshot().revision, 1);
    }

    #[test]
    fn switching_to_content_aware_while_off_saves_mode_without_starting_capture() {
        let config = AppConfig {
            lights_enabled: false,
            ..AppConfig::default()
        };
        let mut model = ControllerModel::new(config).unwrap();

        assert_eq!(
            model.set_mode(LightingMode::ContentAware),
            ModelEffect::NoWrite
        );
        assert_eq!(model.config().mode, LightingMode::ContentAware);
        assert!(!model.config().lights_enabled);
        assert_eq!(model.snapshot().revision, 1);
    }

    #[test]
    fn failed_write_keeps_requested_state_pending_and_confirmed_state_unchanged() {
        let mut model = ControllerModel::new(AppConfig::default()).unwrap();
        assert_eq!(
            model.confirm_write(ZoneColors([CYAN; 4])),
            ModelEffect::NoWrite
        );

        assert_eq!(
            model
                .apply_manual_updates(&[
                    red_update(ZoneId::LeftRear),
                    red_update(ZoneId::LeftFront),
                    red_update(ZoneId::RightFront),
                    red_update(ZoneId::RightRear),
                ])
                .unwrap(),
            ModelEffect::Write(ZoneColors([RED; 4]))
        );
        assert_eq!(model.mark_pending(), ModelEffect::NoWrite);
        assert!(model.snapshot().pending);
        assert_eq!(model.record_failure(), ModelEffect::NoWrite);

        let snapshot = model.snapshot();
        assert!(snapshot.pending);
        assert_eq!(
            snapshot.confirmed_colors,
            Zone::ALL
                .into_iter()
                .map(|zone| crate::ZoneColor {
                    zone: zone.into(),
                    color: RgbColor::from(CYAN),
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(snapshot.writer_health, HealthState::Failed);
        assert_eq!(snapshot.revision, 4);
        assert_eq!(model.manual_target(), ZoneColors([RED; 4]));
    }

    #[test]
    fn only_confirmation_changes_confirmed_colors() {
        let mut model = ControllerModel::new(AppConfig::default()).unwrap();
        model.mark_pending();

        assert_eq!(
            model.confirm_write(ZoneColors([RED; 4])),
            ModelEffect::NoWrite
        );

        let snapshot = model.snapshot();
        assert!(!snapshot.pending);
        assert_eq!(snapshot.writer_health, HealthState::Ready);
        assert_eq!(
            snapshot.confirmed_colors,
            Zone::ALL
                .into_iter()
                .map(|zone| crate::ZoneColor {
                    zone: zone.into(),
                    color: RgbColor::from(RED),
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(snapshot.revision, 2);
    }

    proptest! {
        #[test]
        fn scale_stays_in_range_and_zero_brightness_is_black(
            red in any::<u8>(),
            green in any::<u8>(),
            blue in any::<u8>(),
            brightness in 0_u8..=100,
        ) {
            let scaled = scale(Rgb8 { r: red, g: green, b: blue }, brightness);
            prop_assert!(u16::from(scaled.r) <= 255);
            prop_assert!(u16::from(scaled.g) <= 255);
            prop_assert!(u16::from(scaled.b) <= 255);
            prop_assert_eq!(scale(Rgb8 { r: red, g: green, b: blue }, 0), Rgb8::BLACK);
        }
    }
}
