//! Provider-neutral broker request transport and bounded error projection.

use std::path::Path;
use std::time::{Duration, Instant};

use d2b_contracts_broker::broker_wire::{
    AuditJoinContext, BrokerCallerRole, BrokerProfile, BrokerRequest, BrokerRequestEnvelope,
    BrokerResponse, CanonicalAuditDigest,
};

use crate::target_runtime::DaemonMode;
use crate::typed_error::TypedError;
use crate::unix_transport::{
    connect_seqpacket, connect_seqpacket_with_timeout, read_frame, write_json_frame,
};
use socket2::Socket;

/// NixOS gives every workload Guest its own fixed broker socket basename.
pub const GUEST_BROKER_SOCKET_BASENAME: &str = "guest-broker.sock";

/// Dispatch one broker request over an owned socket path.
///
/// The optional timeout bounds the complete connect, write, and read round
/// trip with one absolute deadline.
pub fn dispatch_broker_request_to_socket(
    socket_path: &Path,
    request: BrokerRequest,
    caller_role: BrokerCallerRole,
    timeout: Option<Duration>,
) -> Result<BrokerResponse, TypedError> {
    let audit_join = default_audit_join_context(&request);
    let envelope = BrokerRequestEnvelope {
        request,
        caller_role,
        test_peer_uid: None,
        audit_join,
    };
    let Some(timeout) = timeout else {
        let socket = connect_seqpacket(socket_path)?;
        write_json_frame(&socket, &envelope)?;
        let response = read_frame(&socket)?;
        return serde_json::from_slice(&response).map_err(|err| {
            TypedError::InternalBrokerUnavailable {
                path: socket_path.to_path_buf(),
                detail: err.to_string(),
            }
        });
    };

    let deadline = Instant::now() + timeout;
    match broker_round_trip_within_deadline(socket_path, &envelope, deadline) {
        Ok(response) => Ok(response),
        Err(_) if Instant::now() >= deadline => Err(TypedError::InternalBrokerTimeout {
            path: socket_path.to_path_buf(),
        }),
        Err(error) => Err(error),
    }
}

pub fn default_audit_join_context(request: &BrokerRequest) -> Option<AuditJoinContext> {
    let (zone_id, operation_identity) = request.authoritative_audit_join()?;
    Some(AuditJoinContext {
        zone_id: CanonicalAuditDigest::parse(zone_id).expect("canonical broker zone digest"),
        operation_identity: CanonicalAuditDigest::parse(operation_identity)
            .expect("canonical broker operation digest"),
    })
}

pub fn broker_remaining_before_op(
    deadline: Instant,
    socket_path: &Path,
) -> Result<Duration, TypedError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(TypedError::InternalBrokerTimeout {
            path: socket_path.to_path_buf(),
        });
    }
    Ok(remaining)
}

fn broker_round_trip_within_deadline(
    socket_path: &Path,
    envelope: &BrokerRequestEnvelope,
    deadline: Instant,
) -> Result<BrokerResponse, TypedError> {
    let remaining = broker_remaining_before_op(deadline, socket_path)?;
    let socket = Socket::from(connect_seqpacket_with_timeout(
        socket_path,
        Some(remaining),
    )?);

    let remaining = broker_remaining_before_op(deadline, socket_path)?;
    socket
        .set_write_timeout(Some(remaining))
        .map_err(|error| TypedError::InternalIo {
            context: format!("set broker write timeout to {remaining:?}"),
            detail: error.to_string(),
        })?;
    write_json_frame(&socket, envelope)?;

    let remaining = broker_remaining_before_op(deadline, socket_path)?;
    socket
        .set_read_timeout(Some(remaining))
        .map_err(|error| TypedError::InternalIo {
            context: format!("set broker read timeout to {remaining:?}"),
            detail: error.to_string(),
        })?;
    let response = read_frame(&socket)?;
    serde_json::from_slice(&response).map_err(|error| TypedError::InternalBrokerUnavailable {
        path: socket_path.to_path_buf(),
        detail: error.to_string(),
    })
}

pub fn broker_response_kind(response: &BrokerResponse) -> String {
    serde_json::to_value(response)
        .ok()
        .and_then(|value| {
            value
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "unknown".to_owned())
}

pub fn redact_broker_error_for_launcher(
    op_name: &str,
    target_wave: Option<&str>,
    broker_error_kind: &str,
) -> (String, String) {
    let _ = target_wave;
    let summary = format!("{op_name} failed");
    let remediation = match broker_error_kind {
        "Broker.BundleResolverUnavailable" => {
            "broker is starting up / bundle not yet loaded; retry shortly. Admin: confirm the bundle path is populated.".to_owned()
        }
        "Broker.BundleIntentMissing" => format!(
            "{op_name} references a bundle intent that the broker did not find. Admin: ask `journalctl -u d2b-broker` for the intent id."
        ),
        "Broker.StoreViewFilesystemMismatch" => format!(
            "{op_name} refused: the per-VM store view is not on the same filesystem as /nix/store. Admin: check the VM state dir layout and retry."
        ),
        "Broker.StoreViewMarkerMissing" => format!(
            "{op_name} refused: the prepared store-view generation is missing its marker. Admin: rebuild the store view and retry."
        ),
        "Broker.LiveHandlerFailed" => format!(
            "{op_name} failed at the broker live handler. Admin: inspect `journalctl -u d2b-broker` for the underlying syscall/exit code."
        ),
        "Broker.CoexistenceRefused" => "{op_name} refused: another firewall manager owns the table per FirewallCoexistencePolicy. Admin: check d2b.site.firewallCoexistencePolicy."
            .replace("{op_name}", op_name),
        "Broker.NftScriptParseFailed" => "{op_name} failed: bundle nft script could not be parsed. Admin: inspect `journalctl -u d2b-broker` for the parse error."
            .replace("{op_name}", op_name),
        "Broker.CarveoutOrderingViolation" => "{op_name} refused: USBIP firewall carve-out rules are out of order relative to broad allow/drop. Admin: inspect the bundle's nft batch ordering."
            .replace("{op_name}", op_name),
        "Broker.NftablesDriftDetected" => "{op_name} refused: the live nft table hash differs from the bundle's expected hash; someone modified the table out-of-band. Admin: investigate before reapplying."
            .replace("{op_name}", op_name),
        "Broker.ValidateBundleFailed" => {
            "trusted bundle validation failed; Admin: re-render the bundle and retry.".to_owned()
        }
        "Broker.Protocol" => {
            "broker protocol error; retry after admin checks broker logs".to_owned()
        }
        "Broker.UsbipLockConflict" => format!(
            "{op_name} refused: USB busid is already claimed by another VM. Admin: stop the other VM or use `d2b usb detach` to release the claim."
        ),
        "Broker.UsbipDeviceAbsent" => format!(
            "{op_name} refused: USB device is not present in sysfs. Admin: confirm the physical device is connected and recognized by the kernel, then retry."
        ),
        "Broker.Unimplemented" => {
            "broker operation is not implemented in this build; Admin: use the supported fallback path for this release.".to_owned()
        }
        "unknown-operation" => {
            "broker rejected an unknown operation; Admin: verify daemon and broker versions match.".to_owned()
        }
        "authz-audit-requires-admin" => {
            "broker audit export requires an authorized admin user.".to_owned()
        }
        _ => format!(
            "{op_name} failed; admin should inspect `journalctl -u d2b-broker` for details"
        ),
    };
    (summary, remediation)
}

pub fn redact_broker_dispatch_failure_for_launcher(op_name: &str) -> (String, String) {
    (
        format!("{op_name} failed"),
        format!(
            "{op_name} could not reach the broker. Admin: inspect `journalctl -u d2bd` for the daemon-side diagnostic."
        ),
    )
}

/// A fixed, mode-bound broker client.
///
/// The profile is selected when the daemon composition is created. Requests
/// carry no profile field and are checked against the closed catalog before a
/// socket is opened.
#[derive(Debug, Clone)]
pub struct ModeBoundBrokerAdapter {
    mode: DaemonMode,
    profile: BrokerProfile,
    socket_path: std::path::PathBuf,
    caller_role: BrokerCallerRole,
}

impl ModeBoundBrokerAdapter {
    pub fn for_mode(
        mode: DaemonMode,
        socket_path: impl Into<std::path::PathBuf>,
        daemon_uid: u32,
    ) -> Self {
        Self {
            mode,
            profile: mode.broker_profile(),
            socket_path: socket_path.into(),
            caller_role: BrokerCallerRole::AdminUid { uid: daemon_uid },
        }
    }

    pub fn host(socket_path: impl Into<std::path::PathBuf>, daemon_uid: u32) -> Self {
        Self::for_mode(DaemonMode::Host, socket_path, daemon_uid)
    }

    pub fn guest(socket_path: impl Into<std::path::PathBuf>, daemon_uid: u32) -> Self {
        Self::for_mode(DaemonMode::Guest, socket_path, daemon_uid)
    }

    pub const fn mode(&self) -> DaemonMode {
        self.mode
    }

    pub const fn profile(&self) -> BrokerProfile {
        self.profile
    }

    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    pub fn validate_instance(&self) -> Result<(), ModeBoundBrokerError> {
        if !self.socket_path.is_absolute() {
            return Err(ModeBoundBrokerError::SocketPath);
        }
        if self.mode == DaemonMode::Guest
            && self.socket_path.file_name().and_then(|name| name.to_str())
                != Some(GUEST_BROKER_SOCKET_BASENAME)
        {
            return Err(ModeBoundBrokerError::InstanceMismatch);
        }
        Ok(())
    }

    /// Dispatch a request only through this adapter's fixed profile.
    pub fn dispatch(
        &self,
        request: BrokerRequest,
        timeout: Option<Duration>,
    ) -> Result<BrokerResponse, ModeBoundBrokerError> {
        if !self.profile.allows_request(&request) {
            return Err(ModeBoundBrokerError::RequestDenied {
                profile: self.profile,
                operation: request.op_name(),
            });
        }
        self.validate_instance()?;
        dispatch_broker_request_to_socket(
            &self.socket_path,
            request,
            self.caller_role.clone(),
            timeout,
        )
        .map_err(ModeBoundBrokerError::Transport)
    }
}

/// Fixed broker adapter refusal.
#[derive(Debug)]
pub enum ModeBoundBrokerError {
    SocketPath,
    InstanceMismatch,
    RequestDenied {
        profile: BrokerProfile,
        operation: &'static str,
    },
    Transport(TypedError),
}

impl std::fmt::Display for ModeBoundBrokerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SocketPath => formatter.write_str("mode-bound-broker-socket-path"),
            Self::InstanceMismatch => formatter.write_str("mode-bound-broker-instance-mismatch"),
            Self::RequestDenied { profile, operation } => {
                write!(
                    formatter,
                    "{}-broker-operation-denied:{operation}",
                    profile.as_str()
                )
            }
            Self::Transport(error) => formatter.write_str(error.kind()),
        }
    }
}

impl std::error::Error for ModeBoundBrokerError {}
