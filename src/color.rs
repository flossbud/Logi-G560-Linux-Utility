#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rgb8 {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb8 {
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0 };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Zone {
    LeftRear,
    LeftFront,
    RightFront,
    RightRear,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ZoneColors(pub [Rgb8; 4]);

impl ZoneColors {
    pub const BLACK: Self = Self([Rgb8 { r: 0, g: 0, b: 0 }; 4]);

    pub fn get(self, zone: Zone) -> Rgb8 {
        self.0[zone as usize]
    }
}
