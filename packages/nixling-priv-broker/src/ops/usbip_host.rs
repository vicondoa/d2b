//! Broker-side USBIP host inspection.
//!
//! The daemon names only bundle intent ids. The broker resolves those
//! ids to a trusted USBIP bind intent, then uses this module to inspect
//! the live host before taking or replaying a claim. Matching is based
//! on vendor/product plus physical bus/port/sysfs topology; serial-like
//! descriptors are intentionally ignored.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use nixling_core::bundle_resolver::ResolvedUsbipBindIntent;
use nixling_core::host::VendorProductPair;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbipHostInspectionError {
    InvalidBusId {
        bus_id: String,
    },
    DeviceMissing {
        bus_id: String,
    },
    MissingRequiredAttr {
        bus_id: String,
        attr: &'static str,
    },
    AttrIo {
        bus_id: String,
        attr: &'static str,
        kind: io::ErrorKind,
        raw_os_error: Option<i32>,
    },
    AttrParse {
        bus_id: String,
        attr: &'static str,
        value: String,
    },
    AllowlistMissing {
        intent_id: String,
    },
    AllowlistMismatch {
        bus_id: String,
        vendor: u16,
        product: u16,
    },
    TopologyIncomplete {
        bus_id: String,
        reason: &'static str,
    },
    TopologyMismatch {
        bus_id: String,
        expected_bus: u16,
        expected_ports: Vec<u8>,
        observed_bus: u16,
        observed_ports: Vec<u8>,
    },
    DeviceIdentityChanged {
        bus_id: String,
        initial_devnum: u16,
        observed_devnum: u16,
    },
    DeviceDepartedDuringInspection {
        bus_id: String,
    },
    PathSafetyViolation {
        detail: String,
    },
    DriverUnbindUnsupported {
        detail: String,
    },
}

impl std::fmt::Display for UsbipHostInspectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBusId { bus_id } => write!(f, "invalid USBIP bus id {bus_id:?}"),
            Self::DeviceMissing { bus_id } => write!(f, "USB device {bus_id} is not present"),
            Self::MissingRequiredAttr { bus_id, attr } => {
                write!(
                    f,
                    "USB device {bus_id} is missing required sysfs attr {attr}"
                )
            }
            Self::AttrIo {
                bus_id,
                attr,
                kind,
                raw_os_error,
            } => write!(
                f,
                "USB device {bus_id} sysfs attr {attr} read failed: kind={kind:?} errno={raw_os_error:?}"
            ),
            Self::AttrParse {
                bus_id,
                attr,
                value,
            } => write!(
                f,
                "USB device {bus_id} sysfs attr {attr} has invalid value {value:?}"
            ),
            Self::AllowlistMissing { intent_id } => {
                write!(
                    f,
                    "USBIP intent {intent_id} has no vendor/product allowlist"
                )
            }
            Self::AllowlistMismatch {
                bus_id,
                vendor,
                product,
            } => write!(
                f,
                "USB device {bus_id} vendor/product {vendor:04x}:{product:04x} is outside the trusted allowlist"
            ),
            Self::TopologyIncomplete { bus_id, reason } => {
                write!(f, "USB device {bus_id} topology is incomplete: {reason}")
            }
            Self::TopologyMismatch {
                bus_id,
                expected_bus,
                expected_ports,
                observed_bus,
                observed_ports,
            } => write!(
                f,
                "USB device {bus_id} topology mismatch: expected bus {expected_bus} ports {expected_ports:?}, observed bus {observed_bus} ports {observed_ports:?}"
            ),
            Self::DeviceIdentityChanged {
                bus_id,
                initial_devnum,
                observed_devnum,
            } => write!(
                f,
                "USB device {bus_id} changed during sysfs inspection: devnum {initial_devnum} became {observed_devnum}"
            ),
            Self::DeviceDepartedDuringInspection { bus_id } => {
                write!(f, "USB device {bus_id} departed during sysfs inspection")
            }
            Self::PathSafetyViolation { detail } => {
                write!(f, "path-safety-violation: {detail}")
            }
            Self::DriverUnbindUnsupported { detail } => {
                write!(f, "usbip-host driver unbind unsupported: {detail}")
            }
        }
    }
}

impl std::error::Error for UsbipHostInspectionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsbipDriverBinding {
    Unbound,
    BoundToUsbipHost,
    BoundToOtherDriver { driver: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbipHostDeviceInspection {
    pub bus_id: String,
    pub vendor: u16,
    pub product: u16,
    pub bus_number: u16,
    pub port_chain: Vec<u8>,
    pub device_node: PathBuf,
    pub driver: UsbipDriverBinding,
}

pub fn enforce_usbip_physical_policy(
    intent: &ResolvedUsbipBindIntent,
    sysfs_root: &Path,
) -> Result<UsbipHostDeviceInspection, UsbipHostInspectionError> {
    let inspection = inspect_usbip_host_device(sysfs_root, &intent.bus_id)?;
    enforce_allowlist(intent, inspection.vendor, inspection.product)?;
    Ok(inspection)
}

pub fn inspect_usbip_host_device(
    sysfs_root: &Path,
    bus_id: &str,
) -> Result<UsbipHostDeviceInspection, UsbipHostInspectionError> {
    inspect_usbip_host_device_with_reader(sysfs_root, bus_id, &FsSysfsAttrReader)
}

fn inspect_usbip_host_device_with_reader<R: SysfsAttrReader>(
    sysfs_root: &Path,
    bus_id: &str,
    reader: &R,
) -> Result<UsbipHostDeviceInspection, UsbipHostInspectionError> {
    validate_bus_id_for_path(bus_id)?;
    let expected = parse_bus_id_topology(bus_id)?;
    if expected.1.is_empty() {
        return Err(UsbipHostInspectionError::TopologyIncomplete {
            bus_id: bus_id.to_owned(),
            reason: "declared bus id has no downstream port chain",
        });
    }

    let device_dir = sysfs_root.join(bus_id);
    match fs::metadata(&device_dir) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return Err(UsbipHostInspectionError::PathSafetyViolation {
                detail: format!("USB sysfs entry for {bus_id} is not a directory"),
            });
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(UsbipHostInspectionError::DeviceMissing {
                bus_id: bus_id.to_owned(),
            });
        }
        Err(error) => {
            return Err(UsbipHostInspectionError::AttrIo {
                bus_id: bus_id.to_owned(),
                attr: "device",
                kind: error.kind(),
                raw_os_error: error.raw_os_error(),
            });
        }
    }

    let initial_devnum = read_decimal_attr(reader, &device_dir, bus_id, "devnum")?;
    let vendor = read_hex_attr(reader, &device_dir, bus_id, "idVendor")?;
    let product = read_hex_attr(reader, &device_dir, bus_id, "idProduct")?;
    let observed_bus = read_decimal_attr(reader, &device_dir, bus_id, "busnum")?;
    let observed_ports =
        parse_port_chain(&read_required_attr(reader, &device_dir, bus_id, "devpath")?).map_err(
            |value| UsbipHostInspectionError::AttrParse {
                bus_id: bus_id.to_owned(),
                attr: "devpath",
                value,
            },
        )?;
    let observed_devnum =
        read_decimal_attr(reader, &device_dir, bus_id, "devnum").map_err(|error| match error {
            UsbipHostInspectionError::MissingRequiredAttr { attr: "devnum", .. } => {
                UsbipHostInspectionError::DeviceDepartedDuringInspection {
                    bus_id: bus_id.to_owned(),
                }
            }
            other => other,
        })?;

    if initial_devnum != observed_devnum {
        return Err(UsbipHostInspectionError::DeviceIdentityChanged {
            bus_id: bus_id.to_owned(),
            initial_devnum,
            observed_devnum,
        });
    }

    if expected.0 != observed_bus || expected.1 != observed_ports {
        return Err(UsbipHostInspectionError::TopologyMismatch {
            bus_id: bus_id.to_owned(),
            expected_bus: expected.0,
            expected_ports: expected.1,
            observed_bus,
            observed_ports,
        });
    }

    Ok(UsbipHostDeviceInspection {
        bus_id: bus_id.to_owned(),
        vendor,
        product,
        bus_number: observed_bus,
        port_chain: observed_ports,
        device_node: PathBuf::from(format!(
            "/dev/bus/usb/{observed_bus:03}/{observed_devnum:03}"
        )),
        driver: inspect_usbip_driver_binding(sysfs_root, bus_id)?,
    })
}

pub fn inspect_usbip_driver_binding(
    sysfs_root: &Path,
    bus_id: &str,
) -> Result<UsbipDriverBinding, UsbipHostInspectionError> {
    validate_bus_id_for_path(bus_id)?;
    let driver_link = sysfs_root.join(bus_id).join("driver");
    match fs::read_link(&driver_link) {
        Ok(target) => {
            let driver = target
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| UsbipHostInspectionError::PathSafetyViolation {
                    detail: format!("USB driver symlink for {bus_id} has no safe basename"),
                })?
                .to_owned();
            if driver == "usbip-host" {
                Ok(UsbipDriverBinding::BoundToUsbipHost)
            } else {
                Ok(UsbipDriverBinding::BoundToOtherDriver { driver })
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(UsbipDriverBinding::Unbound),
        Err(error) => Err(UsbipHostInspectionError::AttrIo {
            bus_id: bus_id.to_owned(),
            attr: "driver",
            kind: error.kind(),
            raw_os_error: error.raw_os_error(),
        }),
    }
}

pub fn ensure_usbip_host_driver_unbind_supported(
    sysfs_root: &Path,
) -> Result<(), UsbipHostInspectionError> {
    let unbind_path = usbip_host_driver_attr_path(sysfs_root, "unbind")?;
    match fs::metadata(&unbind_path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(UsbipHostInspectionError::DriverUnbindUnsupported {
            detail: format!("{} is not a sysfs attribute file", unbind_path.display()),
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(UsbipHostInspectionError::DriverUnbindUnsupported {
                detail: format!(
                    "{} is missing; operator must unbind or recover the USB device manually",
                    unbind_path.display()
                ),
            })
        }
        Err(error) => Err(UsbipHostInspectionError::AttrIo {
            bus_id: "usbip-host".to_owned(),
            attr: "drivers/usbip-host/unbind",
            kind: error.kind(),
            raw_os_error: error.raw_os_error(),
        }),
    }
}

fn usbip_host_driver_attr_path(
    sysfs_root: &Path,
    attr: &'static str,
) -> Result<PathBuf, UsbipHostInspectionError> {
    let bus_root =
        sysfs_root
            .parent()
            .ok_or_else(|| UsbipHostInspectionError::DriverUnbindUnsupported {
                detail: format!(
                    "USB sysfs device root {} has no bus parent",
                    sysfs_root.display()
                ),
            })?;
    Ok(bus_root.join("drivers").join("usbip-host").join(attr))
}

fn enforce_allowlist(
    intent: &ResolvedUsbipBindIntent,
    vendor: u16,
    product: u16,
) -> Result<(), UsbipHostInspectionError> {
    if intent.vendor_product_allowlist.is_empty() {
        return Err(UsbipHostInspectionError::AllowlistMissing {
            intent_id: intent.intent_id.clone(),
        });
    }
    if vendor_product_allowed(&intent.vendor_product_allowlist, vendor, product) {
        Ok(())
    } else {
        Err(UsbipHostInspectionError::AllowlistMismatch {
            bus_id: intent.bus_id.clone(),
            vendor,
            product,
        })
    }
}

fn vendor_product_allowed(allowlist: &[VendorProductPair], vendor: u16, product: u16) -> bool {
    allowlist
        .iter()
        .any(|pair| pair.vendor == vendor && pair.product == product)
}

fn validate_bus_id_for_path(bus_id: &str) -> Result<(), UsbipHostInspectionError> {
    nixling_ipc::usbip::validate_bus_id(bus_id).map_err(|_| {
        UsbipHostInspectionError::InvalidBusId {
            bus_id: bus_id.to_owned(),
        }
    })
}

fn parse_bus_id_topology(bus_id: &str) -> Result<(u16, Vec<u8>), UsbipHostInspectionError> {
    validate_bus_id_for_path(bus_id)?;
    let (bus, ports) = bus_id
        .split_once('-')
        .map_or((bus_id, ""), |(bus, ports)| (bus, ports));
    let bus = bus
        .parse::<u16>()
        .map_err(|_| UsbipHostInspectionError::InvalidBusId {
            bus_id: bus_id.to_owned(),
        })?;
    let ports = parse_port_chain(ports).map_err(|value| UsbipHostInspectionError::AttrParse {
        bus_id: bus_id.to_owned(),
        attr: "busid",
        value,
    })?;
    Ok((bus, ports))
}

fn parse_port_chain(raw: &str) -> Result<Vec<u8>, String> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    raw.split('.')
        .map(|segment| segment.parse::<u8>().map_err(|_| raw.to_owned()))
        .collect()
}

trait SysfsAttrReader {
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
}

struct FsSysfsAttrReader;

impl SysfsAttrReader for FsSysfsAttrReader {
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }
}

fn read_hex_attr<R: SysfsAttrReader>(
    reader: &R,
    device_dir: &Path,
    bus_id: &str,
    attr: &'static str,
) -> Result<u16, UsbipHostInspectionError> {
    let raw = read_required_attr(reader, device_dir, bus_id, attr)?;
    u16::from_str_radix(raw.trim_start_matches("0x"), 16).map_err(|_| {
        UsbipHostInspectionError::AttrParse {
            bus_id: bus_id.to_owned(),
            attr,
            value: raw,
        }
    })
}

fn read_decimal_attr<R: SysfsAttrReader>(
    reader: &R,
    device_dir: &Path,
    bus_id: &str,
    attr: &'static str,
) -> Result<u16, UsbipHostInspectionError> {
    let raw = read_required_attr(reader, device_dir, bus_id, attr)?;
    raw.parse::<u16>()
        .map_err(|_| UsbipHostInspectionError::AttrParse {
            bus_id: bus_id.to_owned(),
            attr,
            value: raw,
        })
}

fn read_required_attr<R: SysfsAttrReader>(
    reader: &R,
    device_dir: &Path,
    bus_id: &str,
    attr: &'static str,
) -> Result<String, UsbipHostInspectionError> {
    let path = device_dir.join(attr);
    match reader.read_to_string(&path) {
        Ok(raw) => Ok(raw.trim_end_matches(&['\r', '\n'][..]).to_owned()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(UsbipHostInspectionError::MissingRequiredAttr {
                bus_id: bus_id.to_owned(),
                attr,
            })
        }
        Err(error) => Err(UsbipHostInspectionError::AttrIo {
            bus_id: bus_id.to_owned(),
            attr,
            kind: error.kind(),
            raw_os_error: error.raw_os_error(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_core::host::VendorProductPair;
    use std::cell::Cell;
    use std::os::unix::fs::symlink;

    fn temp_root(name: &str) -> PathBuf {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("usbip-host-tests");
        let root = base.join(format!("{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create temp root");
        root
    }

    fn write_device(root: &Path, bus_id: &str, vendor: &str, product: &str, devpath: &str) {
        let dir = root.join(bus_id);
        fs::create_dir_all(&dir).expect("create fake device");
        fs::write(dir.join("idVendor"), format!("{vendor}\n")).expect("vendor");
        fs::write(dir.join("idProduct"), format!("{product}\n")).expect("product");
        fs::write(dir.join("busnum"), b"1\n").expect("busnum");
        fs::write(dir.join("devnum"), b"7\n").expect("devnum");
        fs::write(dir.join("devpath"), format!("{devpath}\n")).expect("devpath");
    }

    enum SecondDevnumRead {
        Value(&'static str),
        Error(io::ErrorKind),
    }

    struct SequencedDevnumReader {
        first: &'static str,
        second: SecondDevnumRead,
        devnum_reads: Cell<usize>,
    }

    impl SequencedDevnumReader {
        fn changing(first: &'static str, second: &'static str) -> Self {
            Self {
                first,
                second: SecondDevnumRead::Value(second),
                devnum_reads: Cell::new(0),
            }
        }

        fn departing_after_first_read(first: &'static str) -> Self {
            Self {
                first,
                second: SecondDevnumRead::Error(io::ErrorKind::NotFound),
                devnum_reads: Cell::new(0),
            }
        }
    }

    impl SysfsAttrReader for SequencedDevnumReader {
        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            if path.file_name().and_then(|name| name.to_str()) != Some("devnum") {
                return fs::read_to_string(path);
            }

            let read_index = self.devnum_reads.get();
            self.devnum_reads.set(read_index + 1);
            match read_index {
                0 => Ok(format!("{}\n", self.first)),
                1 => match self.second {
                    SecondDevnumRead::Value(value) => Ok(format!("{value}\n")),
                    SecondDevnumRead::Error(kind) => Err(io::Error::from(kind)),
                },
                _ => panic!("unexpected extra devnum read"),
            }
        }
    }

    fn intent(allowlist: Vec<VendorProductPair>) -> ResolvedUsbipBindIntent {
        ResolvedUsbipBindIntent {
            intent_id: "usbip-bind:env:work:vm:corp-vm:bus:1-2.3".to_owned(),
            bus_id: "1-2.3".to_owned(),
            vm_name: "corp-vm".to_owned(),
            env: "work".to_owned(),
            lock_path: PathBuf::from("/run/nixling/locks/usbip/1-2.3"),
            vendor_product_allowlist: allowlist,
            dynamic_bus_id: false,
        }
    }

    #[test]
    fn enforces_vendor_product_and_physical_topology() {
        let root = temp_root("match");
        write_device(&root, "1-2.3", "1050", "0407", "2.3");
        symlink(
            "/sys/bus/usb/drivers/usbip-host",
            root.join("1-2.3").join("driver"),
        )
        .expect("driver symlink");

        let inspection = enforce_usbip_physical_policy(
            &intent(vec![VendorProductPair {
                vendor: 0x1050,
                product: 0x0407,
            }]),
            &root,
        )
        .expect("policy matches");
        assert_eq!(
            inspection.device_node,
            PathBuf::from("/dev/bus/usb/001/007")
        );
        assert_eq!(inspection.driver, UsbipDriverBinding::BoundToUsbipHost);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_serial_only_or_empty_allowlist_policy() {
        let root = temp_root("allowlist-missing");
        write_device(&root, "1-2.3", "1050", "0407", "2.3");
        fs::write(root.join("1-2.3").join("serial"), b"spoofable\n").expect("serial");

        let error = enforce_usbip_physical_policy(&intent(Vec::new()), &root)
            .expect_err("missing allowlist fails closed");
        assert!(matches!(
            error,
            UsbipHostInspectionError::AllowlistMissing { .. }
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_topology_mismatch_even_when_vid_pid_match() {
        let root = temp_root("topology-mismatch");
        write_device(&root, "1-2.3", "1050", "0407", "2.4");

        let error = enforce_usbip_physical_policy(
            &intent(vec![VendorProductPair {
                vendor: 0x1050,
                product: 0x0407,
            }]),
            &root,
        )
        .expect_err("topology mismatch fails");
        assert!(matches!(
            error,
            UsbipHostInspectionError::TopologyMismatch { .. }
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_devnum_change_during_identity_inspection() {
        let root = temp_root("devnum-change");
        write_device(&root, "1-2.3", "1050", "0407", "2.3");
        let reader = SequencedDevnumReader::changing("7", "8");

        let error = inspect_usbip_host_device_with_reader(&root, "1-2.3", &reader)
            .expect_err("devnum change fails closed");
        assert!(matches!(
            error,
            UsbipHostInspectionError::DeviceIdentityChanged {
                initial_devnum: 7,
                observed_devnum: 8,
                ..
            }
        ));
        assert_eq!(reader.devnum_reads.get(), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_device_departure_during_identity_inspection() {
        let root = temp_root("devnum-departed");
        write_device(&root, "1-2.3", "1050", "0407", "2.3");
        let reader = SequencedDevnumReader::departing_after_first_read("7");

        let error = inspect_usbip_host_device_with_reader(&root, "1-2.3", &reader)
            .expect_err("device departure fails closed");
        assert!(matches!(
            error,
            UsbipHostInspectionError::DeviceDepartedDuringInspection { .. }
        ));
        assert_eq!(reader.devnum_reads.get(), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_invalid_busid_before_path_join() {
        let root = temp_root("invalid-busid");
        let error = inspect_usbip_host_device(&root, "../1-2").expect_err("invalid bus id");
        assert!(matches!(
            error,
            UsbipHostInspectionError::InvalidBusId { .. }
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn checks_usbip_host_driver_unbind_support_explicitly() {
        let root = temp_root("unbind-support")
            .join("sys")
            .join("bus")
            .join("usb")
            .join("devices");
        fs::create_dir_all(&root).expect("devices root");
        let driver = root.parent().unwrap().join("drivers").join("usbip-host");
        fs::create_dir_all(&driver).expect("driver root");
        fs::write(driver.join("unbind"), b"").expect("unbind attr");
        ensure_usbip_host_driver_unbind_supported(&root).expect("unbind supported");

        fs::remove_file(driver.join("unbind")).expect("remove attr");
        let error =
            ensure_usbip_host_driver_unbind_supported(&root).expect_err("missing attr fails");
        assert!(matches!(
            error,
            UsbipHostInspectionError::DriverUnbindUnsupported { .. }
        ));
        let _ = fs::remove_dir_all(root.ancestors().nth(4).unwrap());
    }
}
