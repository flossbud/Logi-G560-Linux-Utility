use std::time::Duration;

use crate::{LightSink, Zone, ZoneColors};

use super::encode_solid;

const INTERFACE: u8 = 2;
const REPORT_LEN: usize = 20;

#[derive(Debug, thiserror::Error)]
pub enum UsbError {
    #[error("G560 USB device not found")]
    DeviceNotFound,
    #[error("USB interface {0} has an active kernel driver")]
    InterfaceBusy(u8),
    #[error("USB operation failed: {0}")]
    Usb(#[from] rusb::Error),
    #[error("short USB control write: expected {expected} bytes, wrote {actual}")]
    ShortWrite { expected: usize, actual: usize },
}

pub trait UsbTransport: Send {
    fn write_report(&mut self, report: &[u8; REPORT_LEN]) -> Result<(), UsbError>;
}

pub struct LibUsbTransport {
    handle: rusb::DeviceHandle<rusb::GlobalContext>,
}

impl LibUsbTransport {
    pub fn open() -> Result<Self, UsbError> {
        let handle =
            rusb::open_device_with_vid_pid(0x046d, 0x0a78).ok_or(UsbError::DeviceNotFound)?;
        if handle.kernel_driver_active(INTERFACE)? {
            return Err(UsbError::InterfaceBusy(INTERFACE));
        }
        handle.claim_interface(INTERFACE)?;
        Ok(Self { handle })
    }
}

impl UsbTransport for LibUsbTransport {
    fn write_report(&mut self, report: &[u8; REPORT_LEN]) -> Result<(), UsbError> {
        let written = self.handle.write_control(
            0x21,
            0x09,
            0x0211,
            0x0002,
            report,
            Duration::from_millis(100),
        )?;
        if written != REPORT_LEN {
            return Err(UsbError::ShortWrite {
                expected: REPORT_LEN,
                actual: written,
            });
        }
        Ok(())
    }
}

pub struct G560<T> {
    transport: T,
    previous: Option<ZoneColors>,
}

impl<T: UsbTransport> G560<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            previous: None,
        }
    }

    pub fn write(&mut self, colors: ZoneColors) -> Result<(), UsbError> {
        if self.previous == Some(colors) {
            return Ok(());
        }
        self.write_all(colors)?;
        self.previous = Some(colors);
        Ok(())
    }

    pub fn blackout(&mut self) -> Result<(), UsbError> {
        self.write_all(ZoneColors::BLACK)?;
        self.previous = Some(ZoneColors::BLACK);
        Ok(())
    }

    fn write_all(&mut self, colors: ZoneColors) -> Result<(), UsbError> {
        for zone in [
            Zone::LeftRear,
            Zone::LeftFront,
            Zone::RightFront,
            Zone::RightRear,
        ] {
            self.transport
                .write_report(&encode_solid(zone, colors.get(zone)))?;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl<T: UsbTransport> LightSink for G560<T> {
    async fn write(&mut self, colors: ZoneColors) -> anyhow::Result<()> {
        G560::write(self, colors).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::Rgb8;

    struct FakeTransport(Arc<Mutex<Vec<[u8; REPORT_LEN]>>>);

    impl UsbTransport for FakeTransport {
        fn write_report(&mut self, report: &[u8; REPORT_LEN]) -> Result<(), UsbError> {
            self.0.lock().unwrap().push(*report);
            Ok(())
        }
    }

    #[test]
    fn write_emits_four_reports_in_zone_order_and_skips_unchanged_colors() {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let colors = ZoneColors([
            Rgb8 { r: 1, g: 2, b: 3 },
            Rgb8 { r: 4, g: 5, b: 6 },
            Rgb8 { r: 7, g: 8, b: 9 },
            Rgb8 {
                r: 10,
                g: 11,
                b: 12,
            },
        ]);
        let mut device = G560::new(FakeTransport(reports.clone()));

        device.write(colors).unwrap();
        device.write(colors).unwrap();

        let reports = reports.lock().unwrap();
        assert_eq!(reports.len(), 4);
        assert_eq!(
            reports.iter().map(|report| report[4]).collect::<Vec<_>>(),
            [0, 2, 3, 1]
        );
        assert_eq!(
            reports
                .iter()
                .map(|report| &report[6..9])
                .collect::<Vec<_>>(),
            [&[1, 2, 3], &[4, 5, 6], &[7, 8, 9], &[10, 11, 12]]
        );
    }

    #[test]
    fn blackout_always_emits_four_black_reports() {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let mut device = G560::new(FakeTransport(reports.clone()));

        device.blackout().unwrap();

        let reports = reports.lock().unwrap();
        assert_eq!(reports.len(), 4);
        assert!(reports.iter().all(|report| report[6..9] == [0, 0, 0]));
    }

    struct FailOnceTransport {
        attempts: usize,
    }

    impl UsbTransport for FailOnceTransport {
        fn write_report(&mut self, _: &[u8; REPORT_LEN]) -> Result<(), UsbError> {
            self.attempts += 1;
            if self.attempts == 1 {
                return Err(UsbError::ShortWrite {
                    expected: REPORT_LEN,
                    actual: 0,
                });
            }
            Ok(())
        }
    }

    #[test]
    fn failed_set_is_not_cached() {
        let mut device = G560::new(FailOnceTransport { attempts: 0 });
        let colors = ZoneColors([Rgb8 { r: 1, g: 2, b: 3 }; 4]);

        assert!(device.write(colors).is_err());
        assert!(device.write(colors).is_ok());
        assert_eq!(device.transport.attempts, 5);
    }
}
