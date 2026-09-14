//! The type-specific half of the security-key frontend.
//!
//! One [`SecurityKeyFrontend`] is a
//! [`d2b_provider_toolkit::GuestAgent`]: it serves CTAPHID report frames on the
//! enrolled Guest session and raises the reports its virtual HID device
//! produces. The session, the enrollment, the frame bound, and the drain
//! ordering are the toolkit's; the report translation and the HID device are
//! this module's.

use std::io;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use d2b_provider_toolkit::{
    Cardinality, DrainDeadline, DrainError, DriverDescriptor, GuestAgent, GuestError, GuestFrame,
    IsolationPosture, ProviderDeclaration,
};
use tokio::sync::{Mutex, OnceCell};

use crate::uhid::{CTAPHID_REPORT_LEN, UhidDevice, UhidEvent};

/// The binary's declared provider identity.
pub const PROVIDER_REF: &str = "device-security-key";

/// The minimal declaration a Guest agent states.
///
/// A Guest agent declares identity like any Provider agent, but it binds no
/// principal, no storage root, and no plane adapter: those are host-plane
/// facts and none of them crosses into the Guest. Its drivers are empty by
/// construction: what this crate serves is CTAPHID report frames on an
/// enrolled Guest session, while the `Device` resource driver is declared by
/// the host-side provider crate for the same type.
pub static DECLARATION: ProviderDeclaration = ProviderDeclaration {
    provider_ref: PROVIDER_REF,
    self_bindings: &[],
    required: false,
    cardinality: Cardinality::AtMostOne,
    isolation_posture: IsolationPosture::Standard,
    plane_adapters: &[],
    principals: &[],
    storage_roots: &[],
};

/// The virtual HID device one frontend drives.
///
/// The trait is the seam a test substitutes; production always runs
/// [`UhidDevice`]. `open` is part of it because a device must be created
/// inside the runtime that will poll it, which is the runtime the toolkit's
/// Guest base owns, not the process entrypoint's.
#[async_trait]
pub trait HidDevice: Send + Sized + 'static {
    /// Create the device this frontend drives.
    async fn open(path: &Path, vm_id: &str) -> io::Result<Self>;

    /// Await the next output report the browser sent to the virtual device.
    ///
    /// MUST be cancel-safe. Lifecycle events are consumed here; `None` means
    /// the device is gone.
    async fn read_report(&mut self) -> io::Result<Option<[u8; CTAPHID_REPORT_LEN]>>;

    /// Inject one input report (token response to the browser).
    async fn send_report(&mut self, report: &[u8; CTAPHID_REPORT_LEN]) -> io::Result<()>;
}

#[async_trait]
impl HidDevice for UhidDevice {
    async fn open(path: &Path, vm_id: &str) -> io::Result<Self> {
        UhidDevice::create(path, vm_id).await
    }

    async fn read_report(&mut self) -> io::Result<Option<[u8; CTAPHID_REPORT_LEN]>> {
        loop {
            match self.read_event().await? {
                Some(UhidEvent::Output { data, .. }) => return Ok(Some(data)),
                Some(UhidEvent::GetReport { id, .. }) => {
                    // Feature reports are the host relay's to serve, not the
                    // Guest frontend's; answer with an error and keep reading.
                    self.send_get_report_reply_error(id).await?;
                }
                // Lifecycle and unknown events are not report traffic.
                Some(UhidEvent::Lifecycle(())) | Some(UhidEvent::Other(_)) => continue,
                None => return Ok(None),
            }
        }
    }

    async fn send_report(&mut self, report: &[u8; CTAPHID_REPORT_LEN]) -> io::Result<()> {
        self.send_input_report(report).await
    }
}

/// Where this frontend's device comes from.
enum DeviceRequest {
    /// The device is created inside the serving runtime.
    Create { path: PathBuf, vm_id: String },
    /// The device was supplied ready, as a test does.
    Ready,
}

/// The security-key frontend: the type-specific half of the Guest agent.
///
/// The device is created once, on first use, inside the runtime the base
/// serves on; the base owns that runtime, so this type never builds one.
pub struct SecurityKeyFrontend<D: HidDevice> {
    device: OnceCell<Mutex<D>>,
    request: DeviceRequest,
}

impl<D: HidDevice> SecurityKeyFrontend<D> {
    /// Bind one frontend to an already created virtual HID device.
    pub fn new(device: D) -> Self {
        let cell = OnceCell::new();
        let ready = cell.set(Mutex::new(device));
        debug_assert!(ready.is_ok(), "a fresh cell accepts its first value");
        Self {
            device: cell,
            request: DeviceRequest::Ready,
        }
    }

    /// Bind one frontend that creates its own device on first use.
    pub fn open(path: impl Into<PathBuf>, vm_id: impl Into<String>) -> Self {
        Self {
            device: OnceCell::new(),
            request: DeviceRequest::Create {
                path: path.into(),
                vm_id: vm_id.into(),
            },
        }
    }

    /// The device, created on first use when this frontend owns creation.
    async fn device(&self) -> Result<&Mutex<D>, GuestError> {
        match &self.request {
            DeviceRequest::Ready => self.device.get().ok_or(GuestError::ServeRefused),
            DeviceRequest::Create { path, vm_id } => self
                .device
                .get_or_try_init(|| async move { D::open(path, vm_id).await.map(Mutex::new) })
                .await
                .map_err(|_| GuestError::ServeRefused),
        }
    }
}

impl<D: HidDevice> std::fmt::Debug for SecurityKeyFrontend<D> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecurityKeyFrontend(<redacted>)")
    }
}

#[async_trait]
impl<D: HidDevice> GuestAgent for SecurityKeyFrontend<D> {
    fn declaration(&self) -> &ProviderDeclaration {
        &DECLARATION
    }

    fn drivers(&self) -> &'static [DriverDescriptor] {
        &[]
    }

    async fn serve(&self, frame: GuestFrame) -> Result<Vec<GuestFrame>, GuestError> {
        let Ok(report) = <[u8; CTAPHID_REPORT_LEN]>::try_from(frame.as_bytes()) else {
            return Err(GuestError::ServeRefused);
        };
        self.device()
            .await?
            .lock()
            .await
            .send_report(&report)
            .await
            .map_err(|_| GuestError::ServeRefused)?;
        Ok(Vec::new())
    }

    async fn next_event(&self) -> Option<GuestFrame> {
        let device = self.device().await.ok()?;
        match device.lock().await.read_report().await {
            Ok(Some(report)) => GuestFrame::new(report.to_vec()).ok(),
            // A device error or a closed device ends this agent's events; the
            // session itself is the base's to drain.
            Ok(None) | Err(_) => None,
        }
    }

    async fn drain(&self, deadline: DrainDeadline) -> Result<(), DrainError> {
        if deadline.expired() {
            return Err(DrainError::DeadlineExpired);
        }
        // Closing the frontend is releasing the virtual HID device: dropping
        // the file descriptor removes the device from the Guest kernel, so
        // there is no further state to unwind and nothing to wait for.
        Ok(())
    }
}
