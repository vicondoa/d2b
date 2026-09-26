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

use std::fmt;
use std::io;
use std::path::Path;

use tokio::io::unix::AsyncFd;



// ---------------------------------------------------------------------------
// UHID event type constants (from include/uapi/linux/uhid.h)
// ---------------------------------------------------------------------------

/// Create the virtual HID device (UHID_CREATE2).
const UHID_CREATE2: u32 = 11;
/// Inject an input report (device→kernel→userspace) (UHID_INPUT2).
const UHID_INPUT2: u32 = 12;
/// Kernel sends an output report (userspace→kernel→device) to us.
const UHID_OUTPUT: u32 = 6;
/// Kernel signals device start (first client opened it).
const UHID_START: u32 = 2;
/// Kernel signals device stop (last client closed it).
const UHID_STOP: u32 = 3;
/// A userspace client opened the device.
const UHID_OPEN: u32 = 4;
/// A userspace client closed the device.
const UHID_CLOSE: u32 = 5;
/// Kernel requests a GET_REPORT from the device.
const UHID_GET_REPORT: u32 = 9;
/// Reply to a GET_REPORT request (UHID_GET_REPORT_REPLY).
const UHID_GET_REPORT_REPLY: u32 = 10;

/// Fixed size of a CTAPHID HID report (input or output).
pub const CTAPHID_REPORT_LEN: usize = 64;

// ---------------------------------------------------------------------------
// HID descriptor constants
// ---------------------------------------------------------------------------

/// USB bus type code.
const BUS_USB: u16 = 3;
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
//                    2(bus) + 4(vendor) + 4(product) + 4(version) +
//                    4(country) + 4096(rd_data) + 1(dev_flags) + 7(pad)
//                    = 4380 bytes
// uhid_input2_req:   2(size) + 4096(data) = 4098 bytes
// uhid_output_req:   4096(data) + 2(size) + 1(rtype) = 4099 bytes
//
// Total uhid_event:  4(type) + max(union) = 4(type) + 4371 = 4375 bytes
// ---------------------------------------------------------------------------

const UHID_CREATE2_PAYLOAD_LEN: usize = 128 + 64 + 64 + 2 + 2 + 4 + 4 + 4 + 4 + 4096 + 1 + 7;
const UHID_INPUT2_PAYLOAD_LEN: usize = 2 + 4096;
/// Full uhid_event size (type + union max).
const UHID_EVENT_SIZE: usize = 4 + UHID_CREATE2_PAYLOAD_LEN;

/// A received event from /dev/uhid.
#[derive(Clone)]
pub enum UhidEvent {
    /// Output report (userspace → virtual device): CTAPHID command from browser.
    Output {
        /// The 64-byte CTAPHID report data.
        data: [u8; CTAPHID_REPORT_LEN],
    },
    /// Device start/stop/open/close lifecycle signal.
    Lifecycle(()),
    /// Get-report request from the kernel (feature reports).
    GetReport {
        /// The kernel's request id, echoed back on the reply.
        id: u32,
    },
    /// Other/unhandled event type.
    Other(u32),
}

impl fmt::Debug for UhidEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Output { .. } => formatter.write_str("UhidEvent::Output(<redacted>)"),
            Self::Lifecycle(_) => formatter.write_str("UhidEvent::Lifecycle"),
            Self::GetReport { id, .. } => formatter
                .debug_struct("UhidEvent::GetReport")
                .field("id", id)
                .finish(),
            Self::Other(event_type) => formatter
                .debug_tuple("UhidEvent::Other")
                .field(event_type)
                .finish(),
        }
    }
}

/// Manages the lifecycle of a virtual FIDO2 HID device via /dev/uhid.
pub struct UhidDevice {
    file: AsyncFd<std::fs::File>,
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
            .custom_flags(libc::O_NONBLOCK)
            .open(uhid_path)
            .await?
            .into_std()
            .await;
        let mut dev = UhidDevice {
            file: AsyncFd::new(file)?,
        };
        dev.write_create2(vm_id).await?;
        Ok(dev)
    }

    /// Read and parse one event from /dev/uhid.
    ///
    /// Blocks until an event is available. Returns `None` on clean EOF
    /// (e. g. the kernel closed the device).
    pub async fn read_event(&mut self) -> io::Result<Option<UhidEvent>> {
        let mut buf = [0u8; UHID_EVENT_SIZE];
        let n = self.read_nonblocking(&mut buf).await?;
        parse_event(&buf[..n])
    }

    /// Inject a 64-byte CTAPHID input report (token response → browser).
    pub async fn send_input_report(&mut self, data: &[u8; CTAPHID_REPORT_LEN]) -> io::Result<()> {
        let buf = build_input2_event(data);
        self.write_all_nonblocking(&buf).await
    }

    /// Write a GET_REPORT_REPLY with an error status (no data) for unsolicited
    /// get-report requests we cannot serve.
    pub async fn send_get_report_reply_error(&mut self, id: u32) -> io::Result<()> {
        let buf = build_get_report_reply_error(id);
        self.write_all_nonblocking(&buf).await
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    async fn write_create2(&mut self, vm_id: &str) -> io::Result<()> {
        let buf = build_create2_event(vm_id);
        self.write_all_nonblocking(&buf).await
    }

    async fn read_nonblocking(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut guard = self.file.readable().await?;
            match guard.try_io(|inner| {
                rustix::io::read(inner.get_ref(), buf)
                    .map_err(|errno| io::Error::from_raw_os_error(errno.raw_os_error()))
            }) {
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    }

    async fn write_all_nonblocking(&self, mut buf: &[u8]) -> io::Result<()> {
        while !buf.is_empty() {
            let mut guard = self.file.writable().await?;
            match guard.try_io(|inner| {
                rustix::io::write(inner.get_ref(), buf)
                    .map_err(|errno| io::Error::from_raw_os_error(errno.raw_os_error()))
            }) {
                Ok(Ok(0)) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(Ok(n)) => buf = &buf[n..],
                Ok(Err(error)) => return Err(error),
                Err(_would_block) => continue,
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Event parsing
// ---------------------------------------------------------------------------

/// Parse one raw event buffer read from /dev/uhid.
///
/// Returns `None` for an empty buffer (clean EOF) and errors on a short
/// event header (fewer than 4 bytes). The kernel always delivers full-size
/// events, so payload fields are read at their fixed packed offsets.
fn parse_event(buf: &[u8]) -> io::Result<Option<UhidEvent>> {
    if buf.is_empty() {
        return Ok(None);
    }
    if buf.len() < 4 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("short uhid event header: {} bytes", buf.len()),
        ));
    }
    let event_type = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let payload = &buf[4..];
    let event = match event_type {
        UHID_OUTPUT => {
            // uhid_output_req layout (packed):
            //   data[4096], size(__u16), rtype(__u8)
            let size = u16::from_le_bytes([payload[4096], payload[4097]]) as usize;
            let data = parse_output_report(payload, size);
            UhidEvent::Output { data }
        }
        UHID_GET_REPORT => {
            // uhid_get_report_req: id(__u32), rnum(__u8), rtype(__u8)
            let id = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
            UhidEvent::GetReport { id }
        }
        UHID_START | UHID_STOP | UHID_OPEN | UHID_CLOSE => UhidEvent::Lifecycle(()),
        other => UhidEvent::Other(other),
    };
    Ok(Some(event))
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
    // bus (__u16 LE)
    buf.extend_from_slice(&BUS_USB.to_le_bytes());
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
    // dev_flags (__u8) + __pad[7]
    buf.push(0);
    buf.extend_from_slice(&[0u8; 7]);

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

fn parse_output_report(payload: &[u8], size: usize) -> [u8; CTAPHID_REPORT_LEN] {
    let start = if size == CTAPHID_REPORT_LEN + 1 && payload.first() == Some(&0) {
        1
    } else {
        0
    };
    let available = payload.len().saturating_sub(start);
    let copy_len = size
        .saturating_sub(start)
        .min(CTAPHID_REPORT_LEN)
        .min(available);
    let mut data = [0u8; CTAPHID_REPORT_LEN];
    data[..copy_len].copy_from_slice(&payload[start..start + copy_len]);
    data
}

fn build_get_report_reply_error(id: u32) -> Vec<u8> {
    // uhid_get_report_reply_req: id(__u32), err(__u16), size(__u16), data[4096]
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
        // type(4) + name(128) + phys(64) + uniq(64) + rd_size(2) + bus(2)
        // + vendor(4) + product(4) + version(4) + country(4) + rd_data(4096)
        // + dev_flags(1) + pad(7)
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
    fn create2_identity_fields_are_aligned() {
        let buf = build_create2_event("test-vm");
        let bus_offset = 4 + 128 + 64 + 64 + 2;
        let vendor_offset = bus_offset + 2;
        let product_offset = vendor_offset + 4;

        let bus = u16::from_le_bytes([buf[bus_offset], buf[bus_offset + 1]]);
        let vendor = u32::from_le_bytes([
            buf[vendor_offset],
            buf[vendor_offset + 1],
            buf[vendor_offset + 2],
            buf[vendor_offset + 3],
        ]);
        let product = u32::from_le_bytes([
            buf[product_offset],
            buf[product_offset + 1],
            buf[product_offset + 2],
            buf[product_offset + 3],
        ]);

        assert_eq!(bus, BUS_USB);
        assert_eq!(vendor, FIDO_VENDOR_ID);
        assert_eq!(product, FIDO_PRODUCT_ID);
    }

    #[test]
    fn create2_descriptor_data_matches() {
        let buf = build_create2_event("test-vm");
        let rd_start = 4 + 128 + 64 + 64 + 2 + 2 + 4 + 4 + 4 + 4;
        assert_eq!(
            &buf[rd_start..rd_start + FIDO_HID_DESCRIPTOR.len()],
            FIDO_HID_DESCRIPTOR
        );
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
    fn output_report_preserves_plain_64_byte_report() {
        let mut payload = [0u8; 4099];
        payload[0] = 0xff;
        payload[1] = 0xff;
        payload[2] = 0xff;
        payload[3] = 0xff;
        payload[4] = 0x86;
        payload[63] = 0xee;

        let report = parse_output_report(&payload, CTAPHID_REPORT_LEN);

        assert_eq!(&report[..5], &[0xff, 0xff, 0xff, 0xff, 0x86]);
        assert_eq!(report[63], 0xee);
    }

    #[test]
    fn output_report_strips_zero_report_id_prefix() {
        let mut payload = [0u8; 4099];
        payload[0] = 0x00;
        payload[1] = 0xff;
        payload[2] = 0xff;
        payload[3] = 0xff;
        payload[4] = 0xff;
        payload[5] = 0x86;
        payload[64] = 0xee;

        let report = parse_output_report(&payload, CTAPHID_REPORT_LEN + 1);

        assert_eq!(&report[..5], &[0xff, 0xff, 0xff, 0xff, 0x86]);
        assert_eq!(report[63], 0xee);
    }

    fn open_uhid_pair() -> (UhidDevice, tokio::net::UnixStream) {
        use std::os::fd::OwnedFd;

        let (read, write) = tokio::net::UnixStream::pair().expect("uhid test socketpair");
        let read = read.into_std().expect("uhid test read end");
        let read_file: std::fs::File = OwnedFd::from(read).into();
        (
            UhidDevice {
                file: AsyncFd::new(read_file).expect("nonblocking uhid fd"),
            },
            write,
        )
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn read_event_dispatches_by_type_and_returns_none_on_clean_eof() {
        use tokio::io::AsyncWriteExt as _;

        let (mut dev, mut write) = open_uhid_pair();

        // Output: type(4) + data[4096] + size(2) + rtype(1)
        let mut output = vec![0u8; 4 + 4096 + 2 + 1];
        output[..4].copy_from_slice(&UHID_OUTPUT.to_le_bytes());
        output[4] = 0x5a;
        output[4 + 4096..4 + 4098].copy_from_slice(&(CTAPHID_REPORT_LEN as u16).to_le_bytes());
        write.write_all(&output).await.expect("write output event");
        match dev.read_event().await.expect("output event") {
            Some(UhidEvent::Output { data }) => {
                assert_eq!(data[0], 0x5a);
                assert_eq!(data[63], 0);
            }
            other => panic!("expected Output, got {other:?}"),
        }

        // GetReport: type(4) + id(4)
        let mut get_report = vec![0u8; 8];
        get_report[..4].copy_from_slice(&UHID_GET_REPORT.to_le_bytes());
        get_report[4..8].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        write.write_all(&get_report).await.expect("write get-report event");
        match dev.read_event().await.expect("get-report event") {
            Some(UhidEvent::GetReport { id }) => assert_eq!(id, 0x1234_5678),
            other => panic!("expected GetReport, got {other:?}"),
        }

        // Lifecycle: start/stop/open/close all map to Lifecycle
        write
            .write_all(&UHID_START.to_le_bytes())
            .await
            .expect("write lifecycle event");
        assert!(matches!(
            dev.read_event().await.expect("lifecycle event"),
            Some(UhidEvent::Lifecycle(()))
        ));

        // Unknown types are surfaced verbatim
        write
            .write_all(&999u32.to_le_bytes())
            .await
            .expect("write unknown event");
        assert!(matches!(
            dev.read_event().await.expect("unknown event"),
            Some(UhidEvent::Other(999))
        ));

        // Clean EOF (all bytes consumed) reads as None
        drop(write);
        assert!(dev.read_event().await.expect("clean eof").is_none());
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn read_event_rejects_a_short_header_as_unexpected_eof() {
        use tokio::io::AsyncWriteExt as _;

        let (mut dev, mut write) = open_uhid_pair();
        write.write_all(&[1u8, 2]).await.expect("write short header");
        let error = dev.read_event().await.expect_err("short header");
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn get_report_reply_error_layout_is_type_ten_id_echo_epipe_and_zero_size() {
        let buf = build_get_report_reply_error(0xdead_beef);
        assert_eq!(buf.len(), 4 + 4 + 2 + 2 + 4096);
        assert_eq!(
            u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]),
            10,
            "UHID_GET_REPORT_REPLY type"
        );
        assert_eq!(
            u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]),
            0xdead_beef,
            "request id echoed"
        );
        assert_eq!(
            u16::from_le_bytes([buf[8], buf[9]]),
            32,
            "err = EPIPE"
        );
        assert_eq!(u16::from_le_bytes([buf[10], buf[11]]), 0, "size = 0");
        assert!(buf[12..].iter().all(|byte| *byte == 0), "data zeroed");
    }

    #[test]
    fn output_report_debug_redacts_ctaphid_bytes() {
        let event = UhidEvent::Output {
            data: [0xa5; CTAPHID_REPORT_LEN],
        };
        let rendered = format!("{event:?}");
        assert_eq!(rendered, "UhidEvent::Output(<redacted>)");
        assert!(!rendered.contains("a5"));
    }

    /// Full-size uhid_event buffer with the given event type in the header.
    fn event_buffer(event_type: u32) -> Vec<u8> {
        let mut buf = vec![0u8; UHID_EVENT_SIZE];
        buf[..4].copy_from_slice(&event_type.to_le_bytes());
        buf
    }

    #[test]
    fn parse_event_dispatch_table() {
        type Case = (u32, fn(&UhidEvent) -> bool);
        let cases: &[Case] = &[
            (UHID_OUTPUT, |e| matches!(e, UhidEvent::Output { .. })),
            (UHID_GET_REPORT, |e| matches!(e, UhidEvent::GetReport { .. })),
            (UHID_START, |e| matches!(e, UhidEvent::Lifecycle(()))),
            (UHID_STOP, |e| matches!(e, UhidEvent::Lifecycle(()))),
            (UHID_OPEN, |e| matches!(e, UhidEvent::Lifecycle(()))),
            (UHID_CLOSE, |e| matches!(e, UhidEvent::Lifecycle(()))),
            (0xdead_beef, |e| matches!(e, UhidEvent::Other(0xdead_beef))),
        ];
        for (event_type, expect) in cases {
            let buf = event_buffer(*event_type);
            let event = parse_event(&buf).unwrap().unwrap();
            assert!(expect(&event), "type {event_type:#x} parsed as {event:?}");
        }
    }

    #[test]
    fn parse_event_output_reads_data_and_size_at_payload_offsets() {
        let mut buf = event_buffer(UHID_OUTPUT);
        // data lives at payload[0..64] (event offset 4)
        for (i, byte) in buf[4..4 + CTAPHID_REPORT_LEN].iter_mut().enumerate() {
            *byte = i as u8;
        }
        // size field sits at payload[4096..4098], rtype at payload[4098]
        buf[4 + 4096..4 + 4098].copy_from_slice(&(CTAPHID_REPORT_LEN as u16).to_le_bytes());

        let event = parse_event(&buf).unwrap().unwrap();
        match event {
            UhidEvent::Output { data } => {
                for (i, byte) in data.iter().enumerate() {
                    assert_eq!(*byte, i as u8);
                }
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn parse_event_output_strips_zero_report_id_prefix() {
        let mut buf = event_buffer(UHID_OUTPUT);
        // report id prefix: payload[0] = 0, data at payload[1..65]
        buf[4 + 1..4 + 65].fill(0x5a);
        buf[4 + 64] = 0xee;
        buf[4 + 4096..4 + 4098].copy_from_slice(&((CTAPHID_REPORT_LEN + 1) as u16).to_le_bytes());

        let event = parse_event(&buf).unwrap().unwrap();
        match event {
            UhidEvent::Output { data } => {
                assert_eq!(data[0], 0x5a);
                assert_eq!(data[63], 0xee);
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn parse_event_get_report_reads_id_at_payload_start() {
        let mut buf = event_buffer(UHID_GET_REPORT);
        // uhid_get_report_req: id(__u32) at payload[0..4]
        buf[4..8].copy_from_slice(&0x1122_3344u32.to_le_bytes());

        let event = parse_event(&buf).unwrap().unwrap();
        match event {
            UhidEvent::GetReport { id } => assert_eq!(id, 0x1122_3344),
            other => panic!("expected GetReport, got {other:?}"),
        }
    }

    #[test]
    fn parse_event_empty_buffer_is_eof() {
        assert!(parse_event(&[]).unwrap().is_none());
    }

    #[test]
    fn parse_event_short_header_errors() {
        let err = parse_event(&[UHID_OUTPUT as u8, 0]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn get_report_reply_error_layout() {
        let buf = build_get_report_reply_error(0x1020_3040);
        // type(4) + id(4) + err(2) + size(2) + data(4096)
        assert_eq!(buf.len(), 4 + 4 + 2 + 2 + 4096);
        let event_type = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        assert_eq!(event_type, UHID_GET_REPORT_REPLY);
        let id = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        assert_eq!(id, 0x1020_3040);
        let err = u16::from_le_bytes([buf[8], buf[9]]);
        assert_eq!(err, 32); // EPIPE: report unavailable
        let size = u16::from_le_bytes([buf[10], buf[11]]);
        assert_eq!(size, 0);
        assert!(buf[12..].iter().all(|&b| b == 0));
    }
}
