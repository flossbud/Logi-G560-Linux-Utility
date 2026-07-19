use std::time::Duration;

use crate::{LightSink, Zone, ZoneColors};

use super::{encode_solid, protocol::encode_solid_index};

const INTERFACE: u8 = 2;
const REPORT_LEN: usize = 20;
const REPORT_DELAY: Duration = Duration::from_millis(20);

#[derive(Debug, thiserror::Error)]
pub enum UsbError {
    #[error("G560 USB device not found")]
    DeviceNotFound,
    #[error("G560 USB interface {0} is missing from the active configuration")]
    MissingInterface(u8),
    #[error("refusing to detach interface {interface}: USB class is 0x{class:02x}, not HID")]
    UnsafeInterface { interface: u8, class: u8 },
    #[error(
        "failed to claim interface after detach ({claim}) and failed to reattach it ({reattach})"
    )]
    ClaimRecovery {
        claim: rusb::Error,
        reattach: rusb::Error,
    },
    #[error("USB operation failed: {0}")]
    Usb(#[from] rusb::Error),
    #[error("short USB control write: expected {expected} bytes, wrote {actual}")]
    ShortWrite { expected: usize, actual: usize },
    #[error("invalid G560 protocol zone index {0}; expected 0 through 3")]
    InvalidZoneIndex(u8),
}

pub trait UsbTransport: Send {
    fn write_report(&mut self, report: &[u8; REPORT_LEN]) -> Result<(), UsbError>;
}

pub struct LibUsbTransport {
    handle: rusb::DeviceHandle<rusb::GlobalContext>,
    detached_kernel_driver: bool,
}

impl LibUsbTransport {
    pub fn open() -> Result<Self, UsbError> {
        let handle =
            rusb::open_device_with_vid_pid(0x046d, 0x0a78).ok_or(UsbError::DeviceNotFound)?;
        let descriptor = handle.device().active_config_descriptor()?;
        let interface = descriptor
            .interfaces()
            .find(|interface| interface.number() == INTERFACE)
            .ok_or(UsbError::MissingInterface(INTERFACE))?;
        validate_interface_classes(
            interface
                .descriptors()
                .map(|descriptor| descriptor.class_code()),
        )?;
        let detached_kernel_driver = handle.kernel_driver_active(INTERFACE)?;
        if detached_kernel_driver {
            handle.detach_kernel_driver(INTERFACE)?;
        }
        if let Err(error) = handle.claim_interface(INTERFACE) {
            return Err(claim_failure_with_recovery(
                detached_kernel_driver,
                error,
                || handle.attach_kernel_driver(INTERFACE),
            ));
        }
        Ok(Self {
            handle,
            detached_kernel_driver,
        })
    }
}

fn claim_failure_with_recovery(
    detached_kernel_driver: bool,
    claim: rusb::Error,
    reattach: impl FnOnce() -> Result<(), rusb::Error>,
) -> UsbError {
    if detached_kernel_driver && let Err(reattach) = reattach() {
        return UsbError::ClaimRecovery { claim, reattach };
    }
    claim.into()
}

fn validate_interface_classes(classes: impl IntoIterator<Item = u8>) -> Result<(), UsbError> {
    let mut found = false;
    for class in classes {
        found = true;
        if class != rusb::constants::LIBUSB_CLASS_HID {
            return Err(UsbError::UnsafeInterface {
                interface: INTERFACE,
                class,
            });
        }
    }
    if !found {
        return Err(UsbError::MissingInterface(INTERFACE));
    }
    Ok(())
}

impl Drop for LibUsbTransport {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(INTERFACE);
        if self.detached_kernel_driver {
            let _ = self.handle.attach_kernel_driver(INTERFACE);
        }
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

pub trait ReportDelay {
    fn wait(&mut self, duration: Duration);
}

pub struct ThreadDelay;

impl ReportDelay for ThreadDelay {
    fn wait(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

pub struct G560<T, D = ThreadDelay> {
    transport: T,
    delay: D,
    previous: Option<ZoneColors>,
    has_sent_report: bool,
}

impl<T: UsbTransport> G560<T, ThreadDelay> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            delay: ThreadDelay,
            previous: None,
            has_sent_report: false,
        }
    }
}

impl<T: UsbTransport, D: ReportDelay> G560<T, D> {
    #[cfg(test)]
    fn with_delay(transport: T, delay: D) -> Self {
        Self {
            transport,
            delay,
            previous: None,
            has_sent_report: false,
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

    /// Pulse one raw protocol zone for hardware mapping while retaining a
    /// single interface claim for the entire observation window.
    pub fn pulse_protocol_zone(&mut self, index: u8, duration: Duration) -> Result<(), UsbError> {
        if index > 3 {
            return Err(UsbError::InvalidZoneIndex(index));
        }
        let result = (|| {
            self.blackout()?;
            self.send_report(&encode_solid_index(
                index,
                crate::Rgb8 {
                    r: 255,
                    g: 255,
                    b: 255,
                },
            ))?;
            std::thread::sleep(duration);
            self.blackout()
        })();
        if result.is_err() {
            let _ = self.blackout();
        }
        result
    }

    fn write_all(&mut self, colors: ZoneColors) -> Result<(), UsbError> {
        for zone in [
            Zone::LeftRear,
            Zone::LeftFront,
            Zone::RightFront,
            Zone::RightRear,
        ]
        .into_iter()
        {
            self.send_report(&encode_solid(zone, colors.get(zone)))?;
        }
        Ok(())
    }

    fn send_report(&mut self, report: &[u8; REPORT_LEN]) -> Result<(), UsbError> {
        if self.has_sent_report {
            self.delay.wait(REPORT_DELAY);
        }
        self.transport.write_report(report)?;
        self.has_sent_report = true;
        Ok(())
    }
}

#[async_trait::async_trait]
impl<T: UsbTransport, D: ReportDelay + Send> LightSink for G560<T, D> {
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

    #[derive(Clone)]
    struct RecordingDelay(Arc<Mutex<Vec<Duration>>>);

    impl ReportDelay for RecordingDelay {
        fn wait(&mut self, duration: Duration) {
            self.0.lock().unwrap().push(duration);
        }
    }

    #[test]
    fn write_emits_four_reports_in_zone_order_and_skips_unchanged_colors() {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let delays = Arc::new(Mutex::new(Vec::new()));
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
        let mut device = G560::with_delay(
            FakeTransport(reports.clone()),
            RecordingDelay(delays.clone()),
        );

        device.write(colors).unwrap();
        device.write(colors).unwrap();

        let reports = reports.lock().unwrap();
        assert_eq!(reports.len(), 4);
        assert_eq!(
            reports.iter().map(|report| report[4]).collect::<Vec<_>>(),
            [2, 0, 1, 3]
        );
        assert_eq!(*delays.lock().unwrap(), [REPORT_DELAY; 3]);
        assert_eq!(
            reports
                .iter()
                .map(|report| &report[6..9])
                .collect::<Vec<_>>(),
            [&[1, 2, 3], &[4, 5, 6], &[7, 8, 9], &[10, 11, 12]]
        );
    }

    #[test]
    fn two_changed_writes_are_paced_across_the_call_boundary() {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let delays = Arc::new(Mutex::new(Vec::new()));
        let mut device = G560::with_delay(
            FakeTransport(reports.clone()),
            RecordingDelay(delays.clone()),
        );

        device
            .write(ZoneColors([Rgb8 { r: 1, g: 2, b: 3 }; 4]))
            .unwrap();
        device
            .write(ZoneColors([Rgb8 { r: 4, g: 5, b: 6 }; 4]))
            .unwrap();

        assert_eq!(reports.lock().unwrap().len(), 8);
        assert_eq!(*delays.lock().unwrap(), [REPORT_DELAY; 7]);
    }

    #[test]
    fn write_then_blackout_is_paced_across_the_call_boundary() {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let delays = Arc::new(Mutex::new(Vec::new()));
        let mut device = G560::with_delay(
            FakeTransport(reports.clone()),
            RecordingDelay(delays.clone()),
        );

        device
            .write(ZoneColors([Rgb8 { r: 1, g: 2, b: 3 }; 4]))
            .unwrap();
        device.blackout().unwrap();

        assert_eq!(reports.lock().unwrap().len(), 8);
        assert_eq!(*delays.lock().unwrap(), [REPORT_DELAY; 7]);
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

    #[test]
    fn mapping_pulse_blacks_all_then_whites_one_raw_index_then_blacks_all() {
        let reports = Arc::new(Mutex::new(Vec::new()));
        let delays = Arc::new(Mutex::new(Vec::new()));
        let mut device = G560::with_delay(
            FakeTransport(reports.clone()),
            RecordingDelay(delays.clone()),
        );

        device
            .pulse_protocol_zone(1, Duration::from_millis(0))
            .unwrap();

        let reports = reports.lock().unwrap();
        assert_eq!(reports.len(), 9);
        assert!(reports[..4].iter().all(|report| report[6..9] == [0, 0, 0]));
        assert_eq!(reports[4][4], 1);
        assert_eq!(reports[4][6..9], [255, 255, 255]);
        assert!(reports[5..].iter().all(|report| report[6..9] == [0, 0, 0]));
        assert_eq!(*delays.lock().unwrap(), [REPORT_DELAY; 8]);
    }

    struct FailOnAttemptTransport {
        attempts: usize,
        fail_on: usize,
        successful: Arc<Mutex<Vec<[u8; REPORT_LEN]>>>,
    }

    impl UsbTransport for FailOnAttemptTransport {
        fn write_report(&mut self, report: &[u8; REPORT_LEN]) -> Result<(), UsbError> {
            self.attempts += 1;
            if self.attempts == self.fail_on {
                return Err(UsbError::ShortWrite {
                    expected: REPORT_LEN,
                    actual: 0,
                });
            }
            self.successful.lock().unwrap().push(*report);
            Ok(())
        }
    }

    #[test]
    fn failed_mapping_pulse_attempts_an_all_black_cleanup() {
        let successful = Arc::new(Mutex::new(Vec::new()));
        let mut device = G560::with_delay(
            FailOnAttemptTransport {
                attempts: 0,
                fail_on: 5,
                successful: successful.clone(),
            },
            RecordingDelay(Arc::new(Mutex::new(Vec::new()))),
        );

        assert!(device.pulse_protocol_zone(1, Duration::ZERO).is_err());

        assert_eq!(device.transport.attempts, 9);
        let reports = successful.lock().unwrap();
        assert_eq!(reports.len(), 8);
        assert!(reports.iter().all(|report| report[6..9] == [0, 0, 0]));
    }

    #[test]
    fn interface_safety_rejects_a_mixed_alternate_setting() {
        assert!(validate_interface_classes([rusb::constants::LIBUSB_CLASS_HID]).is_ok());
        assert!(matches!(
            validate_interface_classes([rusb::constants::LIBUSB_CLASS_HID, 0x01]),
            Err(UsbError::UnsafeInterface {
                interface: INTERFACE,
                class: 0x01
            })
        ));
    }

    #[test]
    fn claim_failure_surfaces_a_failed_kernel_driver_reattach() {
        let error =
            claim_failure_with_recovery(true, rusb::Error::Busy, || Err(rusb::Error::NotFound));

        assert!(matches!(
            error,
            UsbError::ClaimRecovery {
                claim: rusb::Error::Busy,
                reattach: rusb::Error::NotFound,
            }
        ));
    }
}
