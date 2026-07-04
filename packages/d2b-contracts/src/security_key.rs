//! Typed DTOs for the d2b USB security-key proxy feature.
//!
//! # Overview
//!
//! The security-key proxy relays CTAP HID traffic from a host-attached
//! FIDO2 security key (YubiKey, Feitian key, Google Titan, etc.) to one
//! or more guest VMs over AF_VSOCK. Each enabled guest sees a virtual
//! `/dev/hidraw*` via `/dev/uhid`; Firefox and other CTAP2-capable
//! browsers use the normal WebAuthn UI without any in-guest hardware.
//!
//! # Wire contract boundary
//!
//! Types in this module are used on three boundaries:
//!   - **Public wire** (`public_wire`): `usb security-key status`,
//!     `usb security-key sessions`, `usb security-key cancel`
//!     subcommands. See [`SecurityKeyStatusResponse`],
//!     [`SecurityKeySessionsResponse`], [`SecurityKeySessionId`].
//!   - **Broker wire** (`broker_wire`): broker operations for opening
//!     a FIDO-class hidraw node and applying udev group grants.
//!     See [`SecurityKeyOpenDeviceRequest`].
//!   - **Notification events**: host → desktop notification payloads
//!     for CTAP session lifecycle. See [`SecurityKeyEvent`].
//!
//! # Phase 1 constraints
//!
//! Phase 1 enforces these invariants:
//!   1. Security-key proxy and USBIP YubiKey are mutually exclusive for
//!      the same VM (enforced at NixOS eval time in `assertions.nix`).
//!   2. At most one VM holds the host security-key lease at a time.
//!   3. The broker opens ONLY the configured FIDO-class hidraw node;
//!      no blanket `/dev/hidraw*` access is granted.
//!
//! Approval/hybrid prompt modes are reserved for future policy and are
//! **not** part of phase 1.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A stable per-session identifier for a CTAP HID relay session.
///
/// Sessions are created when a guest requests the host security-key
/// proxy to open a CTAP connection, and retired on completion,
/// cancellation, or timeout. The opaque string format is
/// `sk-<vm>-<monotonic-counter>` on the host side; callers must
/// treat it as opaque.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct SecurityKeySessionId(pub String);

impl SecurityKeySessionId {
    /// Construct a session ID from an already-validated string.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable label identifying a configured FIDO device selector.
///
/// Must match `^[a-z][a-z0-9-]{0,62}$` — same constraint as the NixOS
/// option `d2b.host.usb.securityKey.devices[].label`.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct SecurityKeyDeviceLabel(pub String);

impl SecurityKeyDeviceLabel {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ---------------------------------------------------------------------------
// Host proxy status
// ---------------------------------------------------------------------------

/// Top-level response for `d2b usb security-key status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityKeyStatusResponse {
    /// Whether the host security-key proxy is configured and the broker
    /// can access the FIDO hidraw device(s).
    pub host_proxy_enabled: bool,
    /// Configured FIDO device selectors and their current host-side
    /// reachability.
    pub devices: Vec<SecurityKeyDeviceStatus>,
    /// Current lease holder, if any VM holds the exclusive CTAP relay
    /// lease.
    pub current_lease: Option<SecurityKeyLeaseState>,
    /// Per-VM virtual device health summary for all opted-in VMs.
    pub vm_states: Vec<SecurityKeyVmState>,
}

/// Host-side reachability status for one configured FIDO device
/// selector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityKeyDeviceStatus {
    /// Stable selector label from `d2b.host.usb.securityKey.devices`.
    pub label: SecurityKeyDeviceLabel,
    /// USB vendor ID (decimal).
    pub vendor_id: u16,
    /// USB product ID (decimal).
    pub product_id: u16,
    /// Optional serial (from the NixOS selector configuration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
    /// Whether the device is currently present (sysfs-visible) on the host.
    pub present: bool,
    /// Whether the broker has successfully opened the hidraw node.
    pub broker_accessible: bool,
    /// Whether the device is currently bound to the USBIP subsystem
    /// (diagnostic; security-key proxy and USBIP should not coexist
    /// for this device in phase 1).
    pub usbip_bound: bool,
}

/// Current exclusive CTAP relay lease state.
///
/// At most one VM holds the lease at any time in phase 1. The lease is
/// advisory inside this DTO; the kernel OFD lock on the per-busid lock
/// file is the authoritative mutual-exclusion mechanism.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityKeyLeaseState {
    /// VM that currently holds the security-key relay lease.
    pub vm: String,
    /// Stable label of the FIDO device selector in use.
    pub device_label: SecurityKeyDeviceLabel,
    /// Active session ID, if a CTAP session is in progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SecurityKeySessionId>,
    /// ISO 8601 timestamp when the lease was acquired.
    pub acquired_at: String,
}

/// Per-VM security-key proxy health state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityKeyVmState {
    /// VM name.
    pub vm: String,
    /// Whether the VM is opted in (`d2b.vms.<name>.usb.securityKey.enable`).
    pub enabled: bool,
    /// Whether the virtual FIDO HID device is present inside the guest
    /// (`/dev/uhid`-based virtual hidraw device created by d2b-fido-front).
    pub virtual_device_present: bool,
    /// Current CTAP session state for this VM.
    pub session_state: SecurityKeyVmSessionState,
}

/// CTAP session state for a single VM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SecurityKeyVmSessionState {
    /// No CTAP session is active or pending for this VM.
    Idle,
    /// The guest has issued a CTAP HID operation; the host broker is
    /// waiting to acquire the physical device lease.
    AwaitingLease,
    /// The host broker holds the device lease and is relaying CTAP HID
    /// traffic for this VM.
    Active,
    /// The CTAP session completed (success or CTAP error); the virtual
    /// device is awaiting the next operation.
    Completed,
    /// The CTAP session was cancelled (by the operator or by timeout).
    Cancelled,
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// Response for `d2b usb security-key sessions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityKeySessionsResponse {
    /// Recent and active sessions, newest first.
    pub sessions: Vec<SecurityKeySession>,
}

/// One CTAP HID relay session record.
///
/// Sessions are created when a guest begins a WebAuthn operation and
/// retired on completion, cancellation, or timeout. The host daemon
/// keeps a bounded ring-buffer of recent sessions for the
/// `usb security-key sessions` surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityKeySession {
    /// Opaque session identifier.
    pub session_id: SecurityKeySessionId,
    /// VM that initiated this session.
    pub vm: String,
    /// Stable label of the FIDO device selector used.
    pub device_label: SecurityKeyDeviceLabel,
    /// RP ID (relying-party domain, e.g. `login.example.com`) if safely
    /// parsed from the CTAP2 `authenticatorMakeCredential` or
    /// `authenticatorGetAssertion` command. `None` if the session is
    /// still in progress, if RP ID extraction failed, or for CTAP1/U2F
    /// sessions where the AppID is not surfaced here (privacy-sensitive
    /// detail; see audit records for authoritative RP data).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rp_id: Option<String>,
    /// Session outcome.
    pub result: SecurityKeySessionResult,
    /// ISO 8601 timestamp when the session was created (lease acquired or
    /// requested).
    pub started_at: String,
    /// ISO 8601 timestamp when the session ended, or `None` if still
    /// active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
}

/// Terminal outcome of a CTAP HID relay session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SecurityKeySessionResult {
    /// Session is still active (no terminal outcome yet).
    InProgress,
    /// CTAP operation completed successfully.
    Success,
    /// CTAP operation returned a CTAP error code.
    CtapError,
    /// CTAP operation timed out (browser timeout or touch timeout).
    Timeout,
    /// Session was cancelled by the operator (`d2b usb security-key
    /// cancel`).
    Cancelled,
    /// Internal error (broker or vsock transport failure).
    InternalError,
}

// ---------------------------------------------------------------------------
// Cancel request / response
// ---------------------------------------------------------------------------

/// Request payload for `d2b usb security-key cancel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityKeyCancelRequest {
    /// Session ID to cancel, or `None` to cancel the current active
    /// session (if any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SecurityKeySessionId>,
    /// If true, cancel the current active session regardless of session
    /// ID (equivalent to `--current` on the CLI).
    #[serde(default)]
    pub cancel_current: bool,
}

/// Response to `d2b usb security-key cancel`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityKeyCancelResponse {
    /// Whether a session was found and cancelled.
    pub cancelled: bool,
    /// Session ID that was cancelled, or `None` if no session was found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SecurityKeySessionId>,
}

// ---------------------------------------------------------------------------
// Notification events
// ---------------------------------------------------------------------------

/// Notification/event payload for security-key lifecycle transitions.
///
/// These events are emitted by the host daemon and routed through the
/// d2b notification subsystem to the desktop (KDE Plasma). All variants
/// carry sufficient context for both human-readable notifications and
/// machine-readable JSON event logs.
///
/// User-facing terminology uses "security key" throughout; FIDO/CTAP
/// appear only in technical/diagnostic fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", tag = "event", deny_unknown_fields)]
pub enum SecurityKeyEvent {
    /// A guest VM has begun a WebAuthn/CTAP operation and is waiting for
    /// the security key.
    SessionStarted {
        session_id: SecurityKeySessionId,
        vm: String,
        device_label: SecurityKeyDeviceLabel,
        started_at: String,
    },
    /// A CTAP session completed successfully.
    SessionSucceeded {
        session_id: SecurityKeySessionId,
        vm: String,
        device_label: SecurityKeyDeviceLabel,
        ended_at: String,
    },
    /// A CTAP session ended with a CTAP error or timeout.
    SessionFailed {
        session_id: SecurityKeySessionId,
        vm: String,
        device_label: SecurityKeyDeviceLabel,
        result: SecurityKeySessionResult,
        ended_at: String,
    },
    /// A CTAP session was cancelled by the operator.
    SessionCancelled {
        session_id: SecurityKeySessionId,
        vm: String,
        device_label: SecurityKeyDeviceLabel,
        ended_at: String,
    },
    /// The physical security key was removed from the host while a
    /// session was active or queued.
    DeviceRemoved {
        device_label: SecurityKeyDeviceLabel,
        /// Active session that was interrupted, if any.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        interrupted_session_id: Option<SecurityKeySessionId>,
    },
    /// The physical security key was re-inserted and the broker can open
    /// it again.
    DeviceReinserted {
        device_label: SecurityKeyDeviceLabel,
    },
    /// A VM queued a CTAP request and is waiting for the current lease
    /// holder to finish (lease contention).
    SessionQueued {
        session_id: SecurityKeySessionId,
        vm: String,
        device_label: SecurityKeyDeviceLabel,
        queued_at: String,
        /// VM that currently holds the lease.
        blocking_vm: String,
    },
}

// ---------------------------------------------------------------------------
// Broker wire requests for the security-key proxy
// ---------------------------------------------------------------------------

/// Broker request: open the FIDO hidraw node for the named device
/// selector.
///
/// The broker resolves the stable device label against the trusted
/// `host.json` security-key device table, performs the sysfs-presence
/// and FIDO-class checks, opens the exact hidraw node, and returns the
/// open fd via `SCM_RIGHTS`. The daemon receives the fd and holds it
/// for the lifetime of the CTAP relay session.
///
/// This is the ONLY path by which the daemon may obtain a hidraw fd;
/// no inline path, busid, or raw device node is accepted on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityKeyOpenDeviceRequest {
    /// Stable device selector label (matches `d2b.host.usb.securityKey
    /// .devices[].label`). The broker resolves the physical hidraw node
    /// from the trusted bundle's device table using only this label.
    pub device_label: SecurityKeyDeviceLabel,
    /// Session ID for audit correlation. The broker records this in the
    /// session-open audit event alongside the redacted device identity.
    pub session_id: SecurityKeySessionId,
}

/// Broker request: apply udev group grants for configured FIDO hidraw
/// nodes.
///
/// Called once during host activation (or on-demand when the device
/// selector list changes) to update the udev rules that grant the
/// `d2b-security-key` group ownership of the exact hidraw nodes
/// matching the configured vendor/product/serial selectors.
///
/// The broker writes the generated rules to the designated path, does
/// NOT issue a blanket udev reload, and records a targeted audit event.
/// Udev rule application is deferred to the activation helper's normal
/// `udevadm trigger` pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecurityKeyApplyUdevRulesRequest {
    /// Opaque bundle reference for the security-key udev intent.
    /// The broker resolves the exact rule text from the trusted bundle
    /// copy; no inline rule text is accepted on the wire.
    pub bundle_udev_intent_ref: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_round_trips_via_serde() {
        let id = SecurityKeySessionId::new("sk-corp-vm-42");
        let json = serde_json::to_string(&id).expect("serialize");
        let decoded: SecurityKeySessionId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, decoded);
    }

    #[test]
    fn device_label_round_trips_via_serde() {
        let label = SecurityKeyDeviceLabel::new("yubikey-primary");
        let json = serde_json::to_string(&label).expect("serialize");
        let decoded: SecurityKeyDeviceLabel = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(label, decoded);
    }

    #[test]
    fn status_response_round_trips_via_serde() {
        let resp = SecurityKeyStatusResponse {
            host_proxy_enabled: true,
            devices: vec![SecurityKeyDeviceStatus {
                label: SecurityKeyDeviceLabel::new("yubikey-primary"),
                vendor_id: 0x1050,
                product_id: 0x0407,
                serial: None,
                present: true,
                broker_accessible: true,
                usbip_bound: false,
            }],
            current_lease: Some(SecurityKeyLeaseState {
                vm: "corp-vm".to_owned(),
                device_label: SecurityKeyDeviceLabel::new("yubikey-primary"),
                session_id: Some(SecurityKeySessionId::new("sk-corp-vm-1")),
                acquired_at: "2026-07-03T22:00:00Z".to_owned(),
            }),
            vm_states: vec![SecurityKeyVmState {
                vm: "corp-vm".to_owned(),
                enabled: true,
                virtual_device_present: true,
                session_state: SecurityKeyVmSessionState::Active,
            }],
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        let decoded: SecurityKeyStatusResponse = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(resp, decoded);
    }

    #[test]
    fn session_result_enum_covers_all_terminal_states() {
        for result in [
            SecurityKeySessionResult::InProgress,
            SecurityKeySessionResult::Success,
            SecurityKeySessionResult::CtapError,
            SecurityKeySessionResult::Timeout,
            SecurityKeySessionResult::Cancelled,
            SecurityKeySessionResult::InternalError,
        ] {
            let json = serde_json::to_string(&result).expect("serialize");
            let decoded: SecurityKeySessionResult =
                serde_json::from_str(&json).expect("deserialize");
            assert_eq!(result, decoded);
        }
    }

    #[test]
    fn event_session_started_round_trips_via_serde() {
        let event = SecurityKeyEvent::SessionStarted {
            session_id: SecurityKeySessionId::new("sk-corp-vm-1"),
            vm: "corp-vm".to_owned(),
            device_label: SecurityKeyDeviceLabel::new("yubikey-primary"),
            started_at: "2026-07-03T22:00:00Z".to_owned(),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        let decoded: SecurityKeyEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(event, decoded);
    }

    #[test]
    fn event_device_removed_round_trips_via_serde() {
        let event = SecurityKeyEvent::DeviceRemoved {
            device_label: SecurityKeyDeviceLabel::new("yubikey-primary"),
            interrupted_session_id: Some(SecurityKeySessionId::new("sk-corp-vm-1")),
        };
        let json = serde_json::to_string(&event).expect("serialize");
        let decoded: SecurityKeyEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(event, decoded);
    }

    #[test]
    fn open_device_request_round_trips_via_serde() {
        let req = SecurityKeyOpenDeviceRequest {
            device_label: SecurityKeyDeviceLabel::new("yubikey-primary"),
            session_id: SecurityKeySessionId::new("sk-corp-vm-1"),
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let decoded: SecurityKeyOpenDeviceRequest =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(req, decoded);
    }

    #[test]
    fn open_device_request_rejects_unknown_fields() {
        let json = r#"{
            "deviceLabel": "yubikey-primary",
            "sessionId": "sk-corp-vm-1",
            "unexpected": true
        }"#;
        let err = serde_json::from_str::<SecurityKeyOpenDeviceRequest>(json)
            .expect_err("unknown field must fail");
        assert!(err.to_string().contains("unknown field"), "error: {err}");
    }

    #[test]
    fn cancel_request_defaults_to_not_cancel_current() {
        let json = r#"{"sessionId": "sk-corp-vm-1"}"#;
        let req: SecurityKeyCancelRequest = serde_json::from_str(json).expect("deserialize");
        assert!(!req.cancel_current);
    }
}
