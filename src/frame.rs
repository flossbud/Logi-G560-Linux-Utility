use anyhow::{Result, ensure};

use crate::Zone;

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
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Polygon {
    vertices: Vec<Point>,
}

impl Polygon {
    pub fn new(vertices: Vec<Point>) -> Result<Self> {
        ensure!(
            vertices.len() >= 3,
            "a polygon must have at least three vertices"
        );
        ensure!(
            vertices
                .iter()
                .all(|point| point.x.is_finite() && point.y.is_finite()),
            "polygon coordinates must be finite"
        );
        Ok(Self { vertices })
    }

    pub fn vertices(&self) -> &[Point] {
        &self.vertices
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ZoneLayout {
    polygons: [Polygon; 4],
}

impl ZoneLayout {
    pub fn new(polygons: [Polygon; 4]) -> Self {
        Self { polygons }
    }

    pub fn g560_default() -> Self {
        let polygon =
            |vertices| Polygon::new(vertices).expect("the built-in G560 polygon must be valid");
        Self::new([
            polygon(G560_LEFT_REAR.to_vec()),
            polygon(G560_LEFT_FRONT.to_vec()),
            polygon(G560_RIGHT_FRONT.to_vec()),
            polygon(G560_RIGHT_REAR.to_vec()),
        ])
    }

    pub fn polygon(&self, zone: Zone) -> &Polygon {
        &self.polygons[zone as usize]
    }

    fn has_g560_geometry(&self) -> bool {
        self.polygon(Zone::LeftRear).vertices() == G560_LEFT_REAR
            && self.polygon(Zone::LeftFront).vertices() == G560_LEFT_FRONT
            && self.polygon(Zone::RightFront).vertices() == G560_RIGHT_FRONT
            && self.polygon(Zone::RightRear).vertices() == G560_RIGHT_REAR
    }
}

const G560_LEFT_REAR: [Point; 5] = [
    Point { x: 0.00, y: 0.00 },
    Point { x: 0.50, y: 0.00 },
    Point { x: 0.17, y: 0.70 },
    Point { x: 0.14, y: 1.00 },
    Point { x: 0.00, y: 1.00 },
];
const G560_LEFT_FRONT: [Point; 4] = [
    Point { x: 0.50, y: 0.00 },
    Point { x: 0.50, y: 1.00 },
    Point { x: 0.14, y: 1.00 },
    Point { x: 0.17, y: 0.70 },
];
const G560_RIGHT_FRONT: [Point; 4] = [
    Point { x: 0.50, y: 0.00 },
    Point { x: 0.83, y: 0.70 },
    Point { x: 0.86, y: 1.00 },
    Point { x: 0.50, y: 1.00 },
];
const G560_RIGHT_REAR: [Point; 5] = [
    Point { x: 0.50, y: 0.00 },
    Point { x: 1.00, y: 0.00 },
    Point { x: 1.00, y: 1.00 },
    Point { x: 0.86, y: 1.00 },
    Point { x: 0.83, y: 0.70 },
];

#[derive(Clone, Debug, PartialEq)]
pub struct ZoneMasks {
    width: usize,
    height: usize,
    indices: [Vec<usize>; 4],
}

impl ZoneMasks {
    pub fn compile(layout: &ZoneLayout, width: usize, height: usize) -> Result<Self> {
        ensure!(width > 0, "mask width must be nonzero");
        ensure!(height > 0, "mask height must be nonzero");
        let pixel_count = width
            .checked_mul(height)
            .ok_or_else(|| anyhow::anyhow!("mask pixel count overflow"))?;
        let mut indices: [Vec<usize>; 4] = std::array::from_fn(|_| Vec::new());
        let use_g560_half_open_rule = layout.has_g560_geometry();

        for index in 0..pixel_count {
            let x = index % width;
            let y = index / width;
            let zone = if use_g560_half_open_rule {
                g560_zone_at_pixel(x, y, width, height)
            } else {
                let point = Point {
                    x: (x as f32 + 0.5) / width as f32,
                    y: (y as f32 + 0.5) / height as f32,
                };
                ZONES
                    .into_iter()
                    .find(|&zone| contains(layout.polygon(zone), point))
                    .unwrap_or_else(|| fallback_zone(point))
            };
            indices[zone as usize].push(index);
        }

        Ok(Self {
            width,
            height,
            indices,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn indices(&self, zone: Zone) -> &[usize] {
        &self.indices[zone as usize]
    }

    pub fn zone_at(&self, x: usize, y: usize) -> Zone {
        assert!(x < self.width && y < self.height);
        let index = y * self.width + x;
        ZONES
            .into_iter()
            .find(|&zone| self.indices(zone).binary_search(&index).is_ok())
            .expect("compiled masks cover every pixel")
    }
}

const ZONES: [Zone; 4] = [
    Zone::LeftRear,
    Zone::LeftFront,
    Zone::RightFront,
    Zone::RightRear,
];

fn g560_zone_at_pixel(x: usize, y: usize, width: usize, height: usize) -> Zone {
    let right_half = x >= width / 2;
    let left_x = x.min(width - 1 - x);
    let doubled_x_center = 2 * left_x as u128 + 1;
    let doubled_y_center = 2 * y as u128 + 1;
    let width = width as u128;
    let height = height as u128;

    let above_knee = 10 * doubled_y_center <= 14 * height;
    let front = if above_knee {
        70 * height * (width - doubled_x_center) <= 33 * width * doubled_y_center
    } else {
        100 * height * doubled_x_center + 10 * width * doubled_y_center >= 48 * width * height
    };

    match (right_half, front) {
        (false, false) => Zone::LeftRear,
        (false, true) => Zone::LeftFront,
        (true, true) => Zone::RightFront,
        (true, false) => Zone::RightRear,
    }
}

fn contains(polygon: &Polygon, point: Point) -> bool {
    let vertices = polygon.vertices();
    let mut inside = false;
    let mut previous = vertices[vertices.len() - 1];

    for &current in vertices {
        if (current.y > point.y) != (previous.y > point.y)
            && point.x
                < (previous.x - current.x) * (point.y - current.y) / (previous.y - current.y)
                    + current.x
        {
            inside = !inside;
        }
        previous = current;
    }

    inside
}

fn fallback_zone(point: Point) -> Zone {
    let left_boundary = if point.y <= 0.70 {
        0.50 + (0.17 - 0.50) * (point.y / 0.70)
    } else {
        0.17 + (0.14 - 0.17) * ((point.y - 0.70) / 0.30)
    };

    if point.x < 0.50 {
        if point.x < left_boundary {
            Zone::LeftRear
        } else {
            Zone::LeftFront
        }
    } else if point.x < 1.0 - left_boundary {
        Zone::RightFront
    } else {
        Zone::RightRear
    }
}

#[cfg(test)]
mod tests {
    use crate::Zone;

    use super::{Point, Polygon, RgbFrame, ZoneLayout, ZoneMasks};

    const ZONES: [Zone; 4] = [
        Zone::LeftRear,
        Zone::LeftFront,
        Zone::RightFront,
        Zone::RightRear,
    ];

    fn mirrored(zone: Zone) -> Zone {
        match zone {
            Zone::LeftRear => Zone::RightRear,
            Zone::LeftFront => Zone::RightFront,
            Zone::RightFront => Zone::LeftFront,
            Zone::RightRear => Zone::LeftRear,
        }
    }

    fn assert_complete_disjoint_masks(width: usize, height: usize) {
        let masks = ZoneMasks::compile(&ZoneLayout::g560_default(), width, height).unwrap();
        let mut memberships = vec![0_u8; width * height];
        for zone in ZONES {
            for &index in masks.indices(zone) {
                assert!(index < memberships.len());
                memberships[index] += 1;
            }
        }

        assert!(memberships.into_iter().all(|count| count == 1));

        for y in 0..height {
            for x in 0..width / 2 {
                assert_eq!(
                    masks.zone_at(width - 1 - x, y),
                    mirrored(masks.zone_at(x, y)),
                    "mask is not horizontally symmetric at ({x}, {y}) for {width}x{height}"
                );
            }
            if width % 2 == 1 {
                assert_eq!(
                    masks.zone_at(width / 2, y),
                    Zone::RightFront,
                    "center pixel is not owned by RightFront at y={y} for {width}x{height}"
                );
            }
        }
    }

    #[test]
    fn packed_rgb_frame_rejects_wrong_byte_length() {
        assert!(RgbFrame::new(2, 2, 6, vec![0; 11]).is_err());
    }

    #[test]
    fn g560_masks_cover_each_pixel_once_at_varied_resolutions() {
        for (width, height) in [(160, 90), (1, 1), (2, 2), (90, 160), (320, 90)] {
            assert_complete_disjoint_masks(width, height);
        }
    }

    #[test]
    fn g560_masks_assign_representative_pixels() {
        let masks = ZoneMasks::compile(&ZoneLayout::g560_default(), 160, 90).unwrap();

        assert_eq!(masks.zone_at(0, 0), Zone::LeftRear);
        assert_eq!(masks.zone_at(159, 0), Zone::RightRear);
        assert_eq!(masks.zone_at(80, 1), Zone::RightFront);
        assert_eq!(masks.zone_at(79, 89), Zone::LeftFront);
        assert_eq!(masks.zone_at(80, 89), Zone::RightFront);
    }

    #[test]
    fn shared_diagonal_boundary_is_owned_by_mirrored_front_zones() {
        let masks = ZoneMasks::compile(&ZoneLayout::g560_default(), 256, 128).unwrap();

        assert_eq!(masks.zone_at(111, 17), Zone::LeftFront);
        assert_eq!(masks.zone_at(144, 17), Zone::RightFront);
    }

    #[test]
    fn g560_masks_remain_mirrored_across_resolution_sweep() {
        for width in 1..=128 {
            for height in [1, 2, 3, 17, 90, 128] {
                assert_complete_disjoint_masks(width, height);
            }
        }
    }

    #[test]
    fn polygons_require_three_finite_vertices() {
        assert!(Polygon::new(vec![Point { x: 0.0, y: 0.0 }, Point { x: 1.0, y: 1.0 }]).is_err());
        assert!(
            Polygon::new(vec![
                Point { x: 0.0, y: 0.0 },
                Point {
                    x: f32::NAN,
                    y: 0.5,
                },
                Point { x: 1.0, y: 1.0 },
            ])
            .is_err()
        );
    }
}
