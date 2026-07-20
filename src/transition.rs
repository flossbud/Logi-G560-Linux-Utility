use std::array;
use std::time::{Duration, Instant};

use palette::{FromColor, LinSrgb, Oklab, Srgb};

use crate::{Rgb8, ZoneColors};

pub const DEFAULT_TRANSITION_DURATION: Duration = Duration::from_millis(90);

#[derive(Clone, Copy, Debug)]
pub struct TransitionController {
    start: ZoneColors,
    target: ZoneColors,
    started_at: Option<Instant>,
    durations: [Duration; 4],
}

impl TransitionController {
    pub fn new(initial: ZoneColors, duration: Duration) -> Self {
        Self {
            start: initial,
            target: initial,
            started_at: None,
            durations: [duration; 4],
        }
    }

    pub fn retarget(&mut self, target: ZoneColors, now: Instant) {
        self.retarget_with_zone_durations(target, now, self.durations);
    }

    pub fn retarget_with_zone_durations(
        &mut self,
        target: ZoneColors,
        now: Instant,
        durations: [Duration; 4],
    ) {
        self.start = self.colors_at(now);
        self.target = target;
        self.started_at = Some(now);
        self.durations = durations;
    }

    pub fn colors_at(&self, now: Instant) -> ZoneColors {
        let Some(started_at) = self.started_at else {
            return self.target;
        };
        let elapsed = now.saturating_duration_since(started_at);

        ZoneColors(array::from_fn(|index| {
            let duration = self.durations[index];
            if elapsed >= duration {
                return self.target.0[index];
            }
            let time = elapsed.as_secs_f32() / duration.as_secs_f32();
            let progress = time * time * (3.0 - 2.0 * time);
            interpolate_oklab(self.start.0[index], self.target.0[index], progress)
        }))
    }

    pub fn is_complete(&self, now: Instant) -> bool {
        self.started_at.is_none_or(|started_at| {
            let elapsed = now.saturating_duration_since(started_at);
            self.durations.iter().all(|duration| elapsed >= *duration)
        })
    }
}

fn interpolate_oklab(start: Rgb8, target: Rgb8, progress: f32) -> Rgb8 {
    let start = to_oklab(start);
    let target = to_oklab(target);
    let mixed = Oklab::new(
        start.l + (target.l - start.l) * progress,
        start.a + (target.a - start.a) * progress,
        start.b + (target.b - start.b) * progress,
    );
    from_oklab(mixed)
}

fn to_oklab(color: Rgb8) -> Oklab {
    let encoded = Srgb::new(
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
    );
    Oklab::from_color(encoded.into_linear())
}

fn from_oklab(color: Oklab) -> Rgb8 {
    let linear = LinSrgb::from_color(color);
    let encoded = Srgb::from_linear(linear);
    Rgb8 {
        r: encode_channel(encoded.red),
        g: encode_channel(encoded.green),
        b: encode_channel(encoded.blue),
    }
}

fn encode_channel(channel: f32) -> u8 {
    (channel.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use proptest::prelude::*;

    use super::{DEFAULT_TRANSITION_DURATION, TransitionController};
    use crate::{Rgb8, ZoneColors};

    const RED: Rgb8 = Rgb8 { r: 255, g: 0, b: 0 };
    const BLUE: Rgb8 = Rgb8 { r: 0, g: 0, b: 255 };
    const RED_ZONES: ZoneColors = ZoneColors([RED; 4]);
    const BLUE_ZONES: ZoneColors = ZoneColors([BLUE; 4]);

    #[test]
    fn default_duration_is_90_milliseconds() {
        assert_eq!(DEFAULT_TRANSITION_DURATION, Duration::from_millis(90));
    }

    #[test]
    fn reaches_exact_endpoints_and_reports_completion() {
        let start = Instant::now();
        let mut transition =
            TransitionController::new(ZoneColors::BLACK, Duration::from_millis(90));

        transition.retarget(RED_ZONES, start);

        assert_eq!(transition.colors_at(start), ZoneColors::BLACK);
        let midpoint = transition.colors_at(start + Duration::from_millis(45));
        for color in midpoint.0 {
            assert!(color.r > 0 && color.r < 255);
        }
        assert!(!transition.is_complete(start));
        assert!(!transition.is_complete(start + Duration::from_millis(89)));
        assert_eq!(
            transition.colors_at(start + Duration::from_millis(90)),
            RED_ZONES
        );
        assert!(transition.is_complete(start + Duration::from_millis(90)));
    }

    #[test]
    fn quarter_time_uses_smoothstep_easing_not_linear_time() {
        let start = Instant::now();
        let mut transition =
            TransitionController::new(ZoneColors::BLACK, Duration::from_millis(90));
        transition.retarget(RED_ZONES, start);

        let quarter = transition.colors_at(start + Duration::from_micros(22_500));
        let expected = super::interpolate_oklab(Rgb8::BLACK, RED, 0.15625);
        let linear = super::interpolate_oklab(Rgb8::BLACK, RED, 0.25);

        assert_eq!(quarter, ZoneColors([expected; 4]));
        assert_ne!(quarter, ZoneColors([linear; 4]));
    }

    #[test]
    fn retargeting_interrupts_from_the_current_visible_color() {
        let start = Instant::now();
        let mut transition =
            TransitionController::new(ZoneColors::BLACK, Duration::from_millis(90));
        transition.retarget(RED_ZONES, start);

        let interruption = start + Duration::from_millis(45);
        let midway = transition.colors_at(interruption);
        transition.retarget(BLUE_ZONES, interruption);

        assert_eq!(transition.colors_at(interruption), midway);
        assert_eq!(
            transition.colors_at(interruption + Duration::from_millis(90)),
            BLUE_ZONES
        );
    }

    #[test]
    fn zone_specific_durations_finish_independently() {
        let start = Instant::now();
        let mut transition = TransitionController::new(RED_ZONES, Duration::from_millis(90));
        transition.retarget_with_zone_durations(
            ZoneColors([Rgb8::BLACK, BLUE, BLUE, BLUE]),
            start,
            [
                Duration::from_millis(200),
                Duration::from_millis(90),
                Duration::from_millis(90),
                Duration::from_millis(90),
            ],
        );

        let at_ninety = transition.colors_at(start + Duration::from_millis(90));
        assert_ne!(at_ninety.0[0], Rgb8::BLACK);
        assert_eq!(&at_ninety.0[1..], &[BLUE, BLUE, BLUE]);
        assert!(!transition.is_complete(start + Duration::from_millis(90)));
        assert_eq!(
            transition.colors_at(start + Duration::from_millis(200)),
            ZoneColors([Rgb8::BLACK, BLUE, BLUE, BLUE])
        );
    }

    #[test]
    fn midpoint_uses_oklab_instead_of_raw_srgb() {
        let start = Instant::now();
        let mut transition = TransitionController::new(RED_ZONES, Duration::from_millis(90));
        transition.retarget(BLUE_ZONES, start);

        let midpoint = transition.colors_at(start + Duration::from_millis(45));

        assert_ne!(
            midpoint,
            ZoneColors(
                [Rgb8 {
                    r: 128,
                    g: 0,
                    b: 128,
                }; 4]
            )
        );
        assert!(midpoint.0.iter().all(|color| color.g > 0));
    }

    fn arbitrary_zone_colors() -> impl Strategy<Value = ZoneColors> {
        prop::array::uniform4((any::<u8>(), any::<u8>(), any::<u8>()))
            .prop_map(|colors| ZoneColors(colors.map(|(r, g, b)| Rgb8 { r, g, b })))
    }

    proptest! {
        #[test]
        fn arbitrary_transition_outputs_stay_in_rgb8_range(
            initial in arbitrary_zone_colors(),
            target in arbitrary_zone_colors(),
            elapsed_ms in 0_u64..=180,
        ) {
            let start = Instant::now();
            let mut transition = TransitionController::new(initial, Duration::from_millis(90));
            transition.retarget(target, start);

            let colors = transition.colors_at(start + Duration::from_millis(elapsed_ms));

            for color in colors.0 {
                for channel in [color.r, color.g, color.b] {
                    let channel = i16::from(channel);
                    prop_assert!((i16::from(u8::MIN)..=i16::from(u8::MAX)).contains(&channel));
                }
            }
        }
    }
}
