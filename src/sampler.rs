use std::collections::HashMap;

use palette::{FromColor, Lab, LinSrgb, Srgb};

use crate::{Rgb8, RgbFrame, Zone, ZoneColors, ZoneMasks};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SamplerConfig {
    pub darkness_luma: f32,
    pub chroma_bin: f32,
    pub lightness_bin: f32,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            darkness_luma: 0.015,
            chroma_bin: 12.0,
            lightness_bin: 10.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Accumulator {
    weight: f32,
    red: f32,
    green: f32,
    blue: f32,
    count: usize,
}

pub fn sample_zones(frame: &RgbFrame, masks: &ZoneMasks, config: SamplerConfig) -> ZoneColors {
    assert_eq!(frame.width, masks.width());
    assert_eq!(frame.height, masks.height());
    ZoneColors(ZONES.map(|zone| sample_mask(frame, masks.indices(zone), config)))
}

const ZONES: [Zone; 4] = [
    Zone::LeftRear,
    Zone::LeftFront,
    Zone::RightFront,
    Zone::RightRear,
];

fn sample_mask(frame: &RgbFrame, indices: &[usize], config: SamplerConfig) -> Rgb8 {
    let region_pixel_count = indices.len();
    let mut surviving_pixel_count = 0usize;
    let mut bins = HashMap::<(i16, i16, i16), Accumulator>::new();

    for &index in indices {
        let x = index % frame.width;
        let y = index / frame.width;
        let offset = y * frame.stride + x * 3;
        let encoded = Srgb::new(
            frame.pixels[offset] as f32 / 255.0,
            frame.pixels[offset + 1] as f32 / 255.0,
            frame.pixels[offset + 2] as f32 / 255.0,
        );
        let linear = encoded.into_linear();
        let luma = 0.2126 * linear.red + 0.7152 * linear.green + 0.0722 * linear.blue;
        if luma < config.darkness_luma {
            continue;
        }

        surviving_pixel_count += 1;
        let lab = Lab::from_color(linear);
        let chroma = lab.a.hypot(lab.b);
        let weight = (0.25 + chroma / 128.0).min(2.0);
        let key = (
            (lab.l / config.lightness_bin).floor() as i16,
            (lab.a / config.chroma_bin).floor() as i16,
            (lab.b / config.chroma_bin).floor() as i16,
        );
        let accumulator = bins.entry(key).or_default();
        accumulator.weight += weight;
        accumulator.red += linear.red * weight;
        accumulator.green += linear.green * weight;
        accumulator.blue += linear.blue * weight;
        accumulator.count += 1;
    }

    if surviving_pixel_count * 100 < region_pixel_count * 2 {
        return Rgb8::BLACK;
    }

    let Some(dominant) = bins
        .values()
        .max_by(|left, right| left.weight.total_cmp(&right.weight))
    else {
        return Rgb8::BLACK;
    };
    let mean = LinSrgb::new(
        dominant.red / dominant.weight,
        dominant.green / dominant.weight,
        dominant.blue / dominant.weight,
    );
    let encoded: Srgb<f32> = Srgb::from_linear(mean);
    let encoded: Srgb<u8> = encoded.into_format();
    Rgb8 {
        r: encoded.red,
        g: encoded.green,
        b: encoded.blue,
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::{Rgb8, RgbFrame, Zone, ZoneColors, ZoneLayout, ZoneMasks};

    use super::{SamplerConfig, sample_mask, sample_zones};

    const RED: Rgb8 = Rgb8 { r: 255, g: 0, b: 0 };
    const GREEN: Rgb8 = Rgb8 { r: 0, g: 255, b: 0 };
    const BLUE: Rgb8 = Rgb8 { r: 0, g: 0, b: 255 };
    const YELLOW: Rgb8 = Rgb8 {
        r: 255,
        g: 255,
        b: 0,
    };

    fn cfg() -> SamplerConfig {
        SamplerConfig::default()
    }

    fn solid(width: usize, height: usize, color: Rgb8) -> RgbFrame {
        let mut pixels = Vec::with_capacity(width * height * 3);
        for _ in 0..width * height {
            pixels.extend_from_slice(&[color.r, color.g, color.b]);
        }
        RgbFrame::new(width, height, width * 3, pixels).unwrap()
    }

    fn set_test_pixel(frame: &mut RgbFrame, x: usize, y: usize, color: Rgb8) {
        let offset = y * frame.stride + x * 3;
        frame.pixels[offset..offset + 3].copy_from_slice(&[color.r, color.g, color.b]);
    }

    #[test]
    fn black_region_turns_fully_off() {
        let frame = solid(8, 8, Rgb8::BLACK);
        let indices = (0..frame.width * frame.height).collect::<Vec<_>>();
        assert_eq!(sample_mask(&frame, &indices, cfg()), Rgb8::BLACK);
    }

    #[test]
    fn dominant_red_ignores_one_white_pixel() {
        let mut frame = solid(
            10,
            10,
            Rgb8 {
                r: 220,
                g: 10,
                b: 10,
            },
        );
        set_test_pixel(
            &mut frame,
            0,
            0,
            Rgb8 {
                r: 255,
                g: 255,
                b: 255,
            },
        );
        let indices = (0..frame.width * frame.height).collect::<Vec<_>>();
        let got = sample_mask(&frame, &indices, cfg());
        assert!(got.r > 180 && got.g < 50 && got.b < 50);
    }

    #[test]
    fn compiled_polygon_masks_sample_in_logical_zone_order() {
        let width = 160;
        let height = 90;
        let masks = ZoneMasks::compile(&ZoneLayout::g560_default(), width, height).unwrap();
        let assigned = [RED, GREEN, BLUE, YELLOW];
        let mut frame = solid(width, height, Rgb8::BLACK);

        for zone in [
            Zone::LeftRear,
            Zone::LeftFront,
            Zone::RightFront,
            Zone::RightRear,
        ] {
            for &index in masks.indices(zone) {
                set_test_pixel(
                    &mut frame,
                    index % width,
                    index / width,
                    assigned[zone as usize],
                );
            }
        }

        assert_eq!(sample_zones(&frame, &masks, cfg()), ZoneColors(assigned));
    }

    proptest! {
        #[test]
        fn valid_frames_and_compiled_masks_never_panic(
            width in 1usize..24,
            height in 1usize..24,
            padding in 0usize..8,
            bytes in proptest::collection::vec(any::<u8>(), 1..256),
        ) {
            let stride = width * 3 + padding;
            let pixels = (0..stride * height)
                .map(|index| bytes[index % bytes.len()])
                .collect();
            let frame = RgbFrame::new(width, height, stride, pixels).unwrap();
            let masks = ZoneMasks::compile(&ZoneLayout::g560_default(), width, height).unwrap();

            let _ = sample_zones(&frame, &masks, cfg());
        }
    }
}
