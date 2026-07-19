use crate::{Rgb8, Zone};

const REPORT_LEN: usize = 20;

pub fn encode_solid(zone: Zone, color: Rgb8) -> [u8; REPORT_LEN] {
    let mut report = [0; REPORT_LEN];
    report[..9].copy_from_slice(&[
        0x11,
        0xff,
        0x04,
        0x3a,
        zone_index(zone),
        0x01,
        color.r,
        color.g,
        color.b,
    ]);
    report
}

const fn zone_index(zone: Zone) -> u8 {
    match zone {
        Zone::LeftRear => 0x00,
        Zone::LeftFront => 0x02,
        Zone::RightFront => 0x03,
        Zone::RightRear => 0x01,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_known_left_rear_red_report() {
        assert_eq!(
            encode_solid(Zone::LeftRear, Rgb8 { r: 255, g: 0, b: 0 }),
            [
                0x11, 0xff, 0x04, 0x3a, 0x00, 0x01, 0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ]
        );
    }

    #[test]
    fn uses_provisional_protocol_indexes_in_logical_zone_order() {
        let indexes = [
            Zone::LeftRear,
            Zone::LeftFront,
            Zone::RightFront,
            Zone::RightRear,
        ]
        .map(|zone| encode_solid(zone, Rgb8::BLACK)[4]);

        assert_eq!(indexes, [0x00, 0x02, 0x03, 0x01]);
    }
}
