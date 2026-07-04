//! Linux UHID virtual HID device management.
//!
//! Opens /dev/uhid, creates a virtual FIDO2/CTAPHID HID device, and provides
//! send/receive primitives for 64-byte CTAPHID reports.
//!
//! # UHID kernel interface
//!
//! The kernel UHID interface (`/dev/uhid`) uses a simple binary protocol:
//! - Write a `uhid_event` to create/update the virtual device or inject
//!   input reports (data from the token toward userspace).
//! - Read a `uhid_event` to receive output reports (data from userspace
//!   toward the token) or lifecycle events (start, stop, open, close).
//!
//! All structs are `__attribute__((__packed__))` in the kernel headers, so
//! field offsets are as documented here with no padding.
//!
//! # Safety
//!
//! No `unsafe` code. Struct bytes are constructed/parsed manually using the
//! documented packed C layout.

use std::io;
use std::path::Path;

use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::framing::CTAPHID_REPORT_LEN;

// ---------------------------------------------------------------------------
// UHID event type constants (from include/uapi/linux/uhid.h)
// ---------------------------------------------------------------------------

/// Create the virtual HID device (UHID_CREATE2).
const UHID_CREATE2: u32 = 12;
/// Inject an input report (device→kernel→userspace) (UHID_INPUT2).
const UHID_INPUT2: u32 = 11;
/// Kernel sends an output report (userspace→kernel→device) to us.
const UHID_OUTPUT: u32 = 6;
/// Kernel signals device start (first client opened it).
const UHID_START: u32 = 4;
/// Kernel signals device stop (last client closed it).
const UHID_STOP: u32 = 5;
/// A userspace client opened the device.
const UHID_OPEN: u32 = 7;
/// A userspace client closed the device.
const UHID_CLOSE: u32 = 8;
/// Kernel requests a GET_REPORT from the device.
const UHID_GET_REPORT: u32 = 9;

// ---------------------------------------------------------------------------
// HID descriptor constants
// ---------------------------------------------------------------------------

/// USB bus type code.
const BUS_USB: u8 = 3;
/// Yubico USB vendor ID.
const FIDO_VENDOR_ID: u32 = 0x1050;
/// YubiKey 5 USB product ID (virtual HID interface, FIDO2-only).
const FIDO_PRODUCT_ID: u32 = 0x0407;

/// Standard FIDO Alliance CTAPHID HID report descriptor.
///
/// Declares a single-report-ID HID device with one 64-byte input report
/// (token→browser) and one 64-byte output report (browser→token). This is
/// the canonical descriptor required by the FIDO CTAPHID specification and
/// recognized by libfido2 and Firefox.
const FIDO_HID_DESCRIPTOR: &[u8] = &[
    0x06, 0xd0, 0xf1, // Usage Page (FIDO Alliance, 0xf1d0)
    0x09, 0x01, // Usage (CTAPHID Authenticator, 0x01)
    0xa1, 0x01, // Collection (Application)
    0x09, 0x20, //   Usage (Input Report Data, 0x20)
    0x15, 0x00, //   Logical Minimum (0)
    0x26, 0xff, 0x00, //   Logical Maximum (255)
    0x75, 0x08, //   Report Size (8)
    0x95, 0x40, //   Report Count (64)
    0x81, 0x02, //   Input (Data, Variable, Absolute)
    0x09, 0x21, //   Usage (Output Report Data, 0x21)
    0x15, 0x00, //   Logical Minimum (0)
    0x26, 0xff, 0x00, //   Logical Maximum (255)
    0x75, 0x08, //   Report Size (8)
    0x95, 0x40, //   Report Count (64)
    0x91, 0x02, //   Output (Data, Variable, Absolute)
    0xc0, // End Collection
];

// ---------------------------------------------------------------------------
// UHID event sizes (all __packed__ in the kernel header)
//
// uhid_create2_req:  128(name) + 64(phys) + 64(uniq) + 2(rd_size) +
//                    1(bus) + 4(vendor) + 4(product) + 4(version) +
//                    4(country) + 4096(rd_data) = 4371 bytes
// uhid_input2_req:   2(size) + 4096(data) = 4098 bytes
// uhid_output_req:   4096(data) + 2(size) + 1(rtype) = 4099 bytes
//
// Total uhid_event:  4(type) + max(union) = 4(type) + 4371 = 4375 bytes
// ---------------------------------------------------------------------------

const UHID_CREATE2_PAYLOAD_LEN: usize = 128 + 64 + 64 + 2 + 1 + 4 + 4 + 4 + 4 + 4096;
const UHID_INPUT2_PAYLOAD_LEN: usize = 2 + 4096;
/// Full uhid_event size (type + union max).
const UHID_EVENT_SIZE: usize = 4 + UHID_CREATE2_PAYLOAD_LEN;

/// A received event from /dev/uhid.
#[derive(Debug, Clone)]
pub enum UhidEvent {
    /// Output report (userspace → virtual device): CTAPHID command from browser.
    Output {
        /// Report type (HID_OUTPUT_REPORT = 0x02 for CTAPHID).
        _rtype: u8,
        /// The 64-byte CTAPHID report data.
        data: [u8; CTAPHID_REPORT_LEN],
    },
    /// Device start/stop/open/close lifecycle signal.
    Lifecycle(()),
    /// Get-report request from the kernel (feature reports).
    GetReport { id: u32, _rtype: u8, _rnum: u8 },
    /// Other/unhandled event type.
    Other(u32),
}

/// Manages the lifecycle of a virtual FIDO2 HID device via /dev/uhid.
pub struct UhidDevice {
    file: File,
}

impl UhidDevice {
    /// Open /dev/uhid and create the virtual FIDO2 CTAPHID device.
    ///
    /// The device is registered with the kernel and visible to libfido2/Firefox
    /// immediately after this returns. The caller is responsible for the relay
    /// loop (see [`Self::read_event`] and [`Self::send_input_report`]).
    pub async fn create(uhid_path: &Path, vm_id: &str) -> io::Result<Self> {
        let file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(uhid_path)
            .await?;
        let mut dev = UhidDevice { file };
        dev.write_create2(vm_id).await?;
        Ok(dev)
    }

    /// Read and parse one event from /dev/uhid.
    ///
    /// Blocks until an event is available. Returns `None` on clean EOF
    /// (e.g. the kernel closed the device).
    pub async fn read_event(&mut self) -> io::Result<Option<UhidEvent>> {
        let mut buf = [0u8; UHID_EVENT_SIZE];
        match self.file.read_exact(&mut buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let event_type = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let payload = &buf[4..];
        let event = match event_type {
            UHID_OUTPUT => {
                // uhid_output_req layout (packed):
                //   data[4096], size(__u16), rtype(__u8)
                let size = u16::from_le_bytes([payload[4096], payload[4097]]) as usize;
                let rtype = payload[4098];
                let clamped = size.min(CTAPHID_REPORT_LEN);
                let mut data = [0u8; CTAPHID_REPORT_LEN];
                data[..clamped].copy_from_slice(&payload[..clamped]);
                UhidEvent::Output {
                    _rtype: rtype,
                    data,
                }
            }
            UHID_GET_REPORT => {
                // uhid_get_report_req: id(__u32), rnum(__u8), rtype(__u8)
                let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                let rnum = payload[4];
                let rtype = payload[5];
                UhidEvent::GetReport {
                    id,
                    _rtype: rtype,
                    _rnum: rnum,
                }
            }
            UHID_START | UHID_STOP | UHID_OPEN | UHID_CLOSE => {
                let _ = event_type;
                UhidEvent::Lifecycle(())
            }
            other => UhidEvent::Other(other),
        };
        Ok(Some(event))
    }

    /// Inject a 64-byte CTAPHID input report (token response → browser).
    pub async fn send_input_report(
        &mut self,
        data: &[u8; CTAPHID_REPORT_LEN],
    ) -> io::Result<()> {
        let buf = build_input2_event(data);
        self.file.write_all(&buf).await
    }

    /// Write a GET_REPORT_REPLY with an error status (no data) for unsolicited
    /// get-report requests we cannot serve.
    pub async fn send_get_report_reply_error(&mut self, id: u32) -> io::Result<()> {
        let buf = build_get_report_reply_error(id);
        self.file.write_all(&buf).await
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    async fn write_create2(&mut self, vm_id: &str) -> io::Result<()> {
        let buf = build_create2_event(vm_id);
        self.file.write_all(&buf).await
    }
}

// ---------------------------------------------------------------------------
// Event builders (byte-exact, no unsafe)
// ---------------------------------------------------------------------------

fn build_create2_event(vm_id: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + UHID_CREATE2_PAYLOAD_LEN);

    // type
    buf.extend_from_slice(&UHID_CREATE2.to_le_bytes());

    // name[128]: "d2b-sk-<vm_id>\0..."
    let mut name = [0u8; 128];
    let label = format!("d2b-sk-{vm_id}");
    let label_bytes = label.as_bytes();
    let copy_len = label_bytes.len().min(127);
    name[..copy_len].copy_from_slice(&label_bytes[..copy_len]);
    buf.extend_from_slice(&name);

    // phys[64]: empty
    buf.extend_from_slice(&[0u8; 64]);
    // uniq[64]: empty
    buf.extend_from_slice(&[0u8; 64]);
    // rd_size (__u16 LE)
    let rd_size = FIDO_HID_DESCRIPTOR.len() as u16;
    buf.extend_from_slice(&rd_size.to_le_bytes());
    // bus (__u8)
    buf.push(BUS_USB);
    // vendor (__u32 LE)
    buf.extend_from_slice(&FIDO_VENDOR_ID.to_le_bytes());
    // product (__u32 LE)
    buf.extend_from_slice(&FIDO_PRODUCT_ID.to_le_bytes());
    // version (__u32 LE): 0 = no specific HID version
    buf.extend_from_slice(&0u32.to_le_bytes());
    // country (__u32 LE): 0 = not localized
    buf.extend_from_slice(&0u32.to_le_bytes());
    // rd_data[4096]: descriptor padded to 4096
    let mut rd_data = [0u8; 4096];
    rd_data[..FIDO_HID_DESCRIPTOR.len()].copy_from_slice(FIDO_HID_DESCRIPTOR);
    buf.extend_from_slice(&rd_data);

    buf
}

fn build_input2_event(data: &[u8; CTAPHID_REPORT_LEN]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(4 + UHID_INPUT2_PAYLOAD_LEN);
    // type
    buf.extend_from_slice(&UHID_INPUT2.to_le_bytes());
    // size (__u16 LE)
    buf.extend_from_slice(&(CTAPHID_REPORT_LEN as u16).to_le_bytes());
    // data[4096]: report padded to 4096
    let mut payload = [0u8; 4096];
    payload[..CTAPHID_REPORT_LEN].copy_from_slice(data);
    buf.extend_from_slice(&payload);
    buf
}

fn build_get_report_reply_error(id: u32) -> Vec<u8> {
    // uhid_get_report_reply_req: id(__u32), err(__u16), size(__u16), data[4096]
    // UHID_GET_REPORT_REPLY = 10
    const UHID_GET_REPORT_REPLY: u32 = 10;
    let mut buf = Vec::with_capacity(4 + 4 + 2 + 2 + 4096);
    buf.extend_from_slice(&UHID_GET_REPORT_REPLY.to_le_bytes());
    buf.extend_from_slice(&id.to_le_bytes());
    // err = EPIPE (32) to signal unavailability; size = 0
    buf.extend_from_slice(&32u16.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&[0u8; 4096]);
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create2_event_length() {
        let buf = build_create2_event("test-vm");
        // type(4) + name(128) + phys(64) + uniq(64) + rd_size(2) + bus(1)
        // + vendor(4) + product(4) + version(4) + country(4) + rd_data(4096)
        assert_eq!(buf.len(), 4 + UHID_CREATE2_PAYLOAD_LEN);
    }

    #[test]
    fn create2_event_type_field() {
        let buf = build_create2_event("test-vm");
        let event_type = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        assert_eq!(event_type, UHID_CREATE2);
    }

    #[test]
    fn create2_descriptor_length_field() {
        let buf = build_create2_event("test-vm");
        // rd_size field is at offset: 4(type) + 128(name) + 64(phys) + 64(uniq) = 260
        let rd_size = u16::from_le_bytes([buf[260], buf[261]]);
        assert_eq!(rd_size as usize, FIDO_HID_DESCRIPTOR.len());
    }

    #[test]
    fn create2_descriptor_data_matches() {
        let buf = build_create2_event("test-vm");
        // rd_data starts at offset: 260(rd_size offset) + 2(rd_size) + 1(bus) + 4(vendor)
        //   + 4(product) + 4(version) + 4(country) = 279
        let rd_start = 4 + 128 + 64 + 64 + 2 + 1 + 4 + 4 + 4 + 4;
        assert_eq!(&buf[rd_start..rd_start + FIDO_HID_DESCRIPTOR.len()], FIDO_HID_DESCRIPTOR);
    }

    #[test]
    fn input2_event_length() {
        let data = [0xabu8; CTAPHID_REPORT_LEN];
        let buf = build_input2_event(&data);
        // type(4) + size(2) + data(4096)
        assert_eq!(buf.len(), 4 + UHID_INPUT2_PAYLOAD_LEN);
    }

    #[test]
    fn input2_event_type_field() {
        let data = [0u8; CTAPHID_REPORT_LEN];
        let buf = build_input2_event(&data);
        let event_type = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        assert_eq!(event_type, UHID_INPUT2);
    }

    #[test]
    fn input2_event_payload_preserved() {
        let mut data = [0u8; CTAPHID_REPORT_LEN];
        data[0] = 0xde;
        data[63] = 0xad;
        let buf = build_input2_event(&data);
        // data starts at offset 4(type) + 2(size) = 6
        assert_eq!(buf[6], 0xde);
        assert_eq!(buf[6 + 63], 0xad);
    }

    #[test]
    fn fido_descriptor_is_valid_length() {
        assert_eq!(FIDO_HID_DESCRIPTOR.len(), 34);
    }
}
