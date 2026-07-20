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

impl Zone {
    pub const ALL: [Self; 4] = [
        Self::LeftRear,
        Self::LeftFront,
        Self::RightFront,
        Self::RightRear,
    ];
}

impl From<logilightshow_api::ZoneId> for Zone {
    fn from(zone: logilightshow_api::ZoneId) -> Self {
        match zone {
            logilightshow_api::ZoneId::LeftRear => Self::LeftRear,
            logilightshow_api::ZoneId::LeftFront => Self::LeftFront,
            logilightshow_api::ZoneId::RightFront => Self::RightFront,
            logilightshow_api::ZoneId::RightRear => Self::RightRear,
        }
    }
}

impl From<Zone> for logilightshow_api::ZoneId {
    fn from(zone: Zone) -> Self {
        match zone {
            Zone::LeftRear => Self::LeftRear,
            Zone::LeftFront => Self::LeftFront,
            Zone::RightFront => Self::RightFront,
            Zone::RightRear => Self::RightRear,
        }
    }
}

impl From<logilightshow_api::RgbColor> for Rgb8 {
    fn from(color: logilightshow_api::RgbColor) -> Self {
        Self {
            r: color.red,
            g: color.green,
            b: color.blue,
        }
    }
}

impl From<Rgb8> for logilightshow_api::RgbColor {
    fn from(color: Rgb8) -> Self {
        Self {
            red: color.r,
            green: color.g,
            blue: color.b,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ZoneColors(pub [Rgb8; 4]);

impl ZoneColors {
    pub const BLACK: Self = Self([Rgb8 { r: 0, g: 0, b: 0 }; 4]);

    pub fn get(self, zone: Zone) -> Rgb8 {
        self.0[zone as usize]
    }
}

#[cfg(test)]
mod tests {
    use logilightshow_api::{RgbColor, ZoneId};

    use super::{Rgb8, Zone};

    #[test]
    fn all_zones_follow_logical_array_order() {
        assert_eq!(
            Zone::ALL,
            [
                Zone::LeftRear,
                Zone::LeftFront,
                Zone::RightFront,
                Zone::RightRear,
            ]
        );
    }

    #[test]
    fn logical_zone_conversions_preserve_every_zone() {
        let pairs = [
            (ZoneId::LeftRear, Zone::LeftRear),
            (ZoneId::LeftFront, Zone::LeftFront),
            (ZoneId::RightFront, Zone::RightFront),
            (ZoneId::RightRear, Zone::RightRear),
        ];

        for (api, core) in pairs {
            assert_eq!(Zone::from(api), core);
            assert_eq!(ZoneId::from(core), api);
        }
    }

    #[test]
    fn rgb_conversions_preserve_channel_values() {
        let api = RgbColor {
            red: 17,
            green: 34,
            blue: 51,
        };
        let core = Rgb8 {
            r: 17,
            g: 34,
            b: 51,
        };

        assert_eq!(Rgb8::from(api), core);
        assert_eq!(RgbColor::from(core), api);
    }
}
