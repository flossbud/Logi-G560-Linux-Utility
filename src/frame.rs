use anyhow::{Result, ensure};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RgbFrame {
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub pixels: Vec<u8>,
}

impl RgbFrame {
    pub fn new(width: usize, height: usize, stride: usize, pixels: Vec<u8>) -> Result<Self> {
        ensure!(width > 0, "frame width must be nonzero");
        ensure!(height > 0, "frame height must be nonzero");
        let packed_stride = width
            .checked_mul(3)
            .ok_or_else(|| anyhow::anyhow!("packed frame stride overflow"))?;
        ensure!(stride >= packed_stride, "frame stride is too small");
        let expected_len = stride
            .checked_mul(height)
            .ok_or_else(|| anyhow::anyhow!("frame byte length overflow"))?;
        ensure!(pixels.len() == expected_len, "frame byte length is invalid");

        Ok(Self {
            width,
            height,
            stride,
            pixels,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Region {
    pub fn pixel_bounds(
        self,
        frame_width: usize,
        frame_height: usize,
    ) -> (usize, usize, usize, usize) {
        assert!(frame_width > 0 && frame_height > 0);

        let x0 = normalized_start(self.x, frame_width);
        let y0 = normalized_start(self.y, frame_height);
        let x1 = normalized_end(self.x + self.width, frame_width, x0);
        let y1 = normalized_end(self.y + self.height, frame_height, y0);
        (x0, y0, x1, y1)
    }
}

fn normalized_start(value: f32, extent: usize) -> usize {
    let boundary = (value.clamp(0.0, 1.0) * extent as f32).floor() as usize;
    boundary.min(extent - 1)
}

fn normalized_end(value: f32, extent: usize, start: usize) -> usize {
    let boundary = (value.clamp(0.0, 1.0) * extent as f32).ceil() as usize;
    boundary.clamp(start + 1, extent)
}

pub fn default_regions() -> [Region; 4] {
    [
        Region {
            x: 0.0,
            y: 0.0,
            width: 0.25,
            height: 0.25,
        },
        Region {
            x: 0.0,
            y: 0.75,
            width: 0.25,
            height: 0.25,
        },
        Region {
            x: 0.75,
            y: 0.75,
            width: 0.25,
            height: 0.25,
        },
        Region {
            x: 0.75,
            y: 0.0,
            width: 0.25,
            height: 0.25,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::{Region, RgbFrame, default_regions};

    #[test]
    fn packed_rgb_frame_rejects_wrong_byte_length() {
        assert!(RgbFrame::new(2, 2, 6, vec![0; 11]).is_err());
    }

    #[test]
    fn normalized_regions_clamp_to_frame_boundaries() {
        let region = Region {
            x: -0.25,
            y: 0.75,
            width: 1.5,
            height: 0.5,
        };

        assert_eq!(region.pixel_bounds(100, 80), (0, 60, 100, 80));
    }

    #[test]
    fn defaults_select_non_overlapping_edge_areas_in_zone_order() {
        let regions = default_regions();
        let bounds = regions.map(|region| region.pixel_bounds(100, 100));

        assert_eq!(bounds[0], (0, 0, 25, 25));
        assert_eq!(bounds[1], (0, 75, 25, 100));
        assert_eq!(bounds[2], (75, 75, 100, 100));
        assert_eq!(bounds[3], (75, 0, 100, 25));

        for (index, first) in bounds.iter().enumerate() {
            for second in &bounds[index + 1..] {
                let overlaps = first.0 < second.2
                    && second.0 < first.2
                    && first.1 < second.3
                    && second.1 < first.3;
                assert!(!overlaps);
            }
        }
    }
}
