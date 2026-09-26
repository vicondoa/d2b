//! d2b guest-side virtual FIDO/security-key frontend entrypoint.
//!
//! The process reads its placement and device facts from the environment,
//! creates the virtual HID device, and hands both to the toolkit's Guest base:
//!
//! ```text
//! link connect -> zone-bootstrap -> zone-enroll -> serve CTAPHID frames ->
//! drain
//! ```
//!
//! No session, admission, or service loop lives here. Everything this binary
//! adds to the base is the UHID device it owns.
//!
//! Required environment (injected by whoever places the agent):
//!
//! ```text
//! D2B_SK_VM_ID                    the VM this frontend runs in
//! D2B_SK_ZONE_LINK_UID            the committed ZoneLink this enrollment is for
//! D2B_SK_PARENT_ZONE              the parent Zone's label path
//! D2B_SK_GUEST_ZONE               this Guest's label path, a direct child
//! D2B_SK_CONTROLLER_GENERATION    the ZoneLink controller generation
//! D2B_SK_RECONNECT_GENERATION     the link identity generation
//! D2B_SK_SCHEMA_FINGERPRINT       the compiled session schema fingerprint
//! D2B_SK_PSK_ISSUANCE             the allocator's PSK issuance ordinal
//! D2B_SK_PSK_TTL_MS               that issuance's declared lifetime
//! D2B_SK_PSK_ISSUED_AT_UNIX_MS    when the allocator issued it
//! D2B_SK_STATIC_KEY_FINGERPRINT   this frontend's pinned static-key fingerprint
//! ```
//!
//! Optional: `D2B_SK_VSOCK_CID` (default: the hypervisor host),
//! `D2B_SK_VSOCK_PORT` (default 14320), `D2B_SK_UHID_PATH` (default
//! `/dev/uhid`).
//!
//! A frontend that was started without a placement refuses to start rather
//! than enroll against a guess; the unit that starts it owns the restart.

use std::fmt::Display;
use std::sync::Arc;

use d2b_provider_toolkit::{AllocatorEnrollment, run_guest};
use d2b_sk_frontend::{Config, SecurityKeyFrontend, UhidDevice, VsockAllocatorLink};

fn exit_on_error<T, E: Display>(result: Result<T, E>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => {
            eprintln!("[d2b-sk-frontend] fatal: {error}");
            std::process::exit(1);
        }
    }
}

fn main() {
    let Config {
        vm_id,
        link,
        uhid_path,
        placement,
    } = exit_on_error(Config::from_env());
    let placement = exit_on_error(placement.into_placement());

    eprintln!(
        "[d2b-sk-frontend/{}] starting; uhid={}, allocator=vsock:{}:{}",
        vm_id,
        uhid_path.display(),
        link.cid(),
        link.port(),
    );

    let agent = SecurityKeyFrontend::<UhidDevice>::open(&uhid_path, &vm_id);
    let link = Box::new(VsockAllocatorLink::new(link.cid(), link.port()));
    let status = run_guest(agent, link, Arc::new(AllocatorEnrollment::new(placement)));
    std::process::exit(status);
}
