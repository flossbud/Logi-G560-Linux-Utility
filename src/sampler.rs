use std::collections::HashMap;

use palette::{FromColor, Lab, LinSrgb, Srgb};

use crate::{Region, Rgb8, RgbFrame, ZoneColors};

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

pub fn sample_zones(frame: &RgbFrame, regions: &[Region; 4], config: SamplerConfig) -> ZoneColors {
    ZoneColors(regions.map(|region| sample_region(frame, region, config)))
}

fn sample_region(frame: &RgbFrame, region: Region, config: SamplerConfig) -> Rgb8 {
    let (x0, y0, x1, y1) = region.pixel_bounds(frame.width, frame.height);
    let region_pixel_count = (x1 - x0) * (y1 - y0);
    let mut surviving_pixel_count = 0usize;
    let mut bins = HashMap::<(i16, i16, i16), Accumulator>::new();

    for y in y0..y1 {
        for x in x0..x1 {
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

    use crate::{Region, Rgb8, RgbFrame, ZoneColors};

    use super::{SamplerConfig, sample_region, sample_zones};

    const FULL: Region = Region {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
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

    fn quadrant_frame() -> RgbFrame {
        let mut frame = solid(4, 4, Rgb8::BLACK);
        for y in 0..4 {
            for x in 0..4 {
                let color = match (x < 2, y < 2) {
                    (true, true) => RED,
                    (true, false) => GREEN,
                    (false, false) => BLUE,
                    (false, true) => YELLOW,
                };
                set_test_pixel(&mut frame, x, y, color);
            }
        }
        frame
    }

    fn quadrants() -> [Region; 4] {
        [
            Region {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
            Region {
                x: 0.0,
                y: 0.5,
                width: 0.5,
                height: 0.5,
            },
            Region {
                x: 0.5,
                y: 0.5,
                width: 0.5,
                height: 0.5,
            },
            Region {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 0.5,
            },
        ]
    }

    #[test]
    fn black_region_turns_fully_off() {
        assert_eq!(
            sample_region(&solid(8, 8, Rgb8::BLACK), FULL, cfg()),
            Rgb8::BLACK
        );
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
        let got = sample_region(&frame, FULL, cfg());
        assert!(got.r > 180 && got.g < 50 && got.b < 50);
    }

    #[test]
    fn four_regions_are_independent() {
        let frame = quadrant_frame();
        assert_eq!(
            sample_zones(&frame, &quadrants(), cfg()),
            ZoneColors([RED, GREEN, BLUE, YELLOW])
        );
    }

    proptest! {
        #[test]
        fn valid_frames_and_normalized_regions_never_panic(
            width in 1usize..24,
            height in 1usize..24,
            padding in 0usize..8,
            bytes in proptest::collection::vec(any::<u8>(), 1..256),
            x in 0.0f32..=1.0,
            y in 0.0f32..=1.0,
            region_width in 0.0f32..=1.0,
            region_height in 0.0f32..=1.0,
        ) {
            let stride = width * 3 + padding;
            let pixels = (0..stride * height)
                .map(|index| bytes[index % bytes.len()])
                .collect();
            let frame = RgbFrame::new(width, height, stride, pixels).unwrap();
            let region = Region { x, y, width: region_width, height: region_height };

            let _ = sample_region(&frame, region, cfg());
            let _ = sample_zones(&frame, &[region; 4], cfg());
        }
    }
}
