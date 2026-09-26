//! The broker process entry point.
//!
//! [`parse_command`] turns the process arguments into a [`BrokerMode`]
//! carrying a [`ServerConfig`]; [`run`] executes that mode, serving the
//! broker's socket until termination or returning a [`RunError`] on
//! failure.

use std::env;
use std::fs;
#[cfg(not(feature = "layer1-bootstrap"))]
use std::future::Future;
use std::io;
#[cfg(not(feature = "layer1-bootstrap"))]
use std::pin::Pin;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
#[cfg(not(feature = "layer1-bootstrap"))]
use std::os::unix::fs::FileTypeExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
// Test-only sync fakes keep `std::sync::Mutex` per the plan's test-helper
// assumption (sanctioned inline allows at flip time); production state is
// `tokio::sync` (plan U8). The fakes exist only on the non-bootstrap wire.
#[cfg(all(test, not(feature = "layer1-bootstrap")))]
use std::sync::Mutex;
use std::time::{Duration, Instant};
#[cfg(not(feature = "layer1-bootstrap"))]
use std::{
    collections::{BTreeSet, HashMap},
    sync::{LazyLock, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

// The accepted descriptor is wrapped where it is accepted (the reactor's
// listener), so the bootstrap build - which serves the same listener - has no
// direct user of the raw-fd helper.
#[cfg(not(feature = "layer1-bootstrap"))]
use crate::sys::owned_fd_from_raw;
use crate::sys::{path_safe, peer_credentials};
#[cfg(not(feature = "layer1-bootstrap"))]
use hmac::{Hmac, Mac};
#[cfg(not(feature = "layer1-bootstrap"))]
use nix::libc;
#[cfg(not(feature = "layer1-bootstrap"))]
use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
#[cfg(not(feature = "layer1-bootstrap"))]
use nix::unistd::dup;
use serde_json::Value;
#[cfg(not(feature = "layer1-bootstrap"))]
use sha2::{Digest, Sha256};
#[cfg(not(feature = "layer1-bootstrap"))]
use tracing::info;
use tracing::warn;

use crate::audit::{AuditLog, AuditWriteClass};
#[cfg(not(feature = "layer1-bootstrap"))]
use crate::audit::{BROKER_VERSION, new_event_id, result_for_decision};
#[cfg(not(feature = "layer1-bootstrap"))]
use crate::ops::audit_op::{
    BrokerAuditRecordClass, OpAuditRecord, OperationFields, UsbAuditDeviceIdentity,
    UsbSerialCorrelation, UsbSerialCorrelationKeyRotationAudit,
};
#[cfg(not(feature = "layer1-bootstrap"))]
use crate::protocol::{AsyncSeqpacket, AsyncSeqpacketListener, bind_seqpacket};
#[cfg(feature = "layer1-bootstrap")]
use crate::protocol::{
    AsyncSeqpacket, AsyncSeqpacketListener, bind_seqpacket, connect_seqpacket, recv_json_frame,
    send_json_frame,
};

#[cfg(feature = "layer1-bootstrap")]
use crate::bootstrap::wire::{BrokerRequest, BrokerResponse, CallerRole, RequestEnvelope};
#[cfg(feature = "layer1-bootstrap")]
use d2b_contracts_broker::broker_wire::BrokerProfile;
#[cfg(not(feature = "layer1-bootstrap"))]
use d2b_contracts_broker::broker_wire::{
    AuditJoinContext, BrokerCallerRole as CallerRole, BrokerErrorResponse, BrokerProfile,
    BrokerRequest, BrokerRequestEnvelope as RequestEnvelope, BrokerResponse, CanonicalAuditDigest,
    RunnerRole,
};
#[cfg(feature = "layer1-bootstrap")]
type AuditJoinContext = ();

#[cfg(not(feature = "layer1-bootstrap"))]
use d2b_core::bundle_resolver::BundleResolver;

/// Default socket path.  When `LISTEN_FDS=1` (socket activation) this path
/// is informational only; the broker adopts fd 3 from systemd and MUST NOT
/// bind or re-chown the path.  Pass `--socket-path` (or set
/// `D2B_BROKER_SOCKET_PATH`) to override the path used in non-activated
/// (test / legacy) mode.
const DEFAULT_SOCKET_PATH: &str = "/run/d2b/priv.sock";
/// The directory the broker's own socket lives in. The daemon derives a
/// runner's private socket paths from the configured broker socket's parent
/// (`socket_runtime_dir`), so this is the fallback fence for
/// [`crate::live_handlers::grant_serving_worker_launch_acls`] when the
/// configured path has no parent.
const DEFAULT_BROKER_RUNTIME_DIR: &str = "/run/d2b";
const DEFAULT_GUEST_SOCKET_PATH: &str = "/run/d2b/guest-broker.sock";
/// Audit records land under
/// `/var/lib/d2b/audit/broker-<utc-date>.jsonl` (no more legacy
/// single `broker-audit.log` file). Override via `--audit-dir`.
const DEFAULT_AUDIT_DIR: &str = "/var/lib/d2b/audit";
const DEFAULT_GUEST_AUDIT_DIR: &str = "/var/lib/d2b/guest-audit";
/// Default audit retention. Matches the docs claim in
/// `docs/reference/daemon-api.md` "Audit" and `AGENTS.md` "Control
/// plane". Override via `--audit-retention-days` (broker flag) or the
/// NixOS module's `d2b.site.audit.retentionDays` option. Set to 0
/// to disable pruning.
const DEFAULT_AUDIT_RETENTION_DAYS: u32 = 30;
const DEFAULT_BUNDLE_PATH: &str = "/var/lib/d2b/current-bundle/manifest.json";
const DEFAULT_GUEST_BUNDLE_PATH: &str = "/etc/d2b/guest-bundle.json";
const DEFAULT_STATE_DIR: &str = "/var/lib/d2b";
const DEFAULT_GUEST_STATE_DIR: &str = "/var/lib/d2b/guest-broker";
const DEFAULT_ACTIVATION_HELPER_PATH: &str = "/run/current-system/sw/bin/d2b-activation-helper";
const CAPABILITIES: &[&str] = &["Hello", "ExportBrokerAudit", "ApplyHostGenerationHandoff"];
const DEFAULT_IPC_REQUESTS_PER_UID_PER_SECOND: u32 = 512;
const IPC_RATE_LIMIT_WINDOW: Duration = Duration::from_secs(1);
/// Bound on one PipeWire probe subprocess (`pw_dump` / `wpctl`) wait.
///
/// The typed `PipeWireAudio` arm carries no envelope context deadline, so
/// the probe wait is bounded by this default, mirroring the systemd
/// family's method-timeout precedent (5s). A probe that exceeds the bound
/// is treated exactly like a failed probe (host not ready / effect not
/// applied), never a stall of the dispatch worker.
const PW_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_IPC_RATE_LIMIT_MAX_BUCKETS: usize = 4096;
const MAX_MODULE_NAME_LEN: usize = 64;

#[cfg(not(feature = "layer1-bootstrap"))]
/// One wire variant every current broker must answer with the
/// stale-wire-version refusal, never with a pre-dispatch
/// wire-malformed-json drop (KTD10).
///
/// A retirement keeps the variant's name here as a closed entry so a
/// straggler peer's call to it is recognized before dispatch: the gate
/// walks the negotiated-wire table
/// ([`ServerConfig::retired_wire_variants`]) and refuses the call with a
/// typed code plus an audit record. The entry
/// records the negotiated wire version the variant was retired in, so the
/// refusal and its audit record tell the operator which version boundary
/// the caller has not moved past.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetiredWireVariant {
    /// The wire variant name, exactly as the frame's `request.kind` spells
    /// it.
    pub variant: &'static str,
    /// The negotiated wire version the variant was retired in: a straggler
    /// peer negotiated a version before this boundary, and the refusal names
    /// it.
    pub retired_in_version: u32,
}

#[cfg(not(feature = "layer1-bootstrap"))]
/// The production retirement table.
///
/// U10 retired the process-family wire variants here, each entry added in
/// the same change that removed the variant's dispatch arm; a straggler
/// peer's call is refused with the typed stale-wire-version code plus an
/// audit record before the typed decode could drop it as malformed wire
/// (KTD10). The mixed-version fixture table in the runtime tests keeps the
/// gate machinery exercised independently of the production entries.
pub const RETIRED_WIRE_VARIANTS: &[RetiredWireVariant] = &[
    RetiredWireVariant {
        variant: "OpenPidfd",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "OpenPeerPidfdFromAcceptedSocket",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "ObserveRunner",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "PollChildReaped",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "PrepareRuntimeDir",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "PrepareStateDir",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "CgroupKill",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "SignalRunner",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "DeregisterRunnerPidfd",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "SpawnRunner",
        retired_in_version: 6,
    },
    // U11 retired the guest lifecycle lease arm with its row: the lease
    // rides the generic consume-cell/complete-cell kernels through the
    // EnvelopeInvoke surface, and a straggler's typed lease frame is
    // refused by this gate.
    RetiredWireVariant {
        variant: "ConsumeLifecycleLease",
        retired_in_version: 6,
    },
    // U12 retired the network-fds family wire variants: the thirteen
    // network operations ride the broker-generic kernels through the
    // EnvelopeInvoke surface, and a straggler's typed frame is refused by
    // this gate.
    RetiredWireVariant {
        variant: "ApplyNftables",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "ApplyNftablesProjection",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "ApplyNmUnmanaged",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "ApplyRoute",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "ApplySysctl",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "CreateBridge",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "DeleteBridge",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "CreatePersistentTap",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "DeletePersistentTap",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "CreateTapFd",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "SetBridgePortFlags",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "UpdateHostsFile",
        retired_in_version: 6,
    },
    RetiredWireVariant {
        variant: "SeedDnsmasqLease",
        retired_in_version: 6,
    },
];

#[cfg(not(feature = "layer1-bootstrap"))]
/// The variant one frame's `request.kind` names, when the envelope's request
/// is shaped the way the closed wire spells it.
///
/// The frame is inspected before the typed decode precisely so a variant the
/// current enum no longer carries can still be recognized: an internally
/// tagged request serializes as `{"request": {"kind": ..., "payload": ...}}`
/// (or the bare `"kind"` string for a unit variant), and the name alone is
/// enough to gate.
fn request_kind(envelope: &Value) -> Option<&str> {
    match envelope.get("request")? {
        Value::String(kind) => Some(kind.as_str()),
        Value::Object(object) => object.get("kind").and_then(Value::as_str),
        _ => None,
    }
}

/// The retired-variant entry one wire variant name matches, when the table
/// carries it.
///
#[cfg(not(feature = "layer1-bootstrap"))]
/// The gate's lookup, public so a mixed-version fixture can exercise it the
/// way the production accept loop does; the variant name is the frame's
/// `request.kind` ([`request_kind`]).
pub fn retired_wire_variant<'a>(
    kind: &str,
    retired: &'a [RetiredWireVariant],
) -> Option<&'a RetiredWireVariant> {
    retired.iter().find(|entry| entry.variant == kind)
}

#[cfg(not(feature = "layer1-bootstrap"))]
/// Whether one decoded-as-JSON request names a retired wire variant of
/// `retired`, for the gate in [`handle_connection`].
fn retired_variant_for<'a>(
    envelope: &Value,
    retired: &'a [RetiredWireVariant],
) -> Option<&'a RetiredWireVariant> {
    retired_wire_variant(request_kind(envelope)?, retired)
}

#[cfg(not(feature = "layer1-bootstrap"))]
/// The typed refusal one retired-variant call is answered with.
///
/// The `kind` is the envelope's stale-wire-version code (`STALE_WIRE_VERSION`,
/// a member of the envelope's closed refusal set), so a daemon that reads the
/// error response sees the same vocabulary the envelope's own refusals use.
fn stale_wire_refusal(retired: &RetiredWireVariant) -> BrokerResponse {
    BrokerResponse::Error(BrokerErrorResponse {
        kind: crate::envelope::STALE_WIRE_VERSION.to_owned(),
        operation: retired.variant.to_owned(),
        target_wave: None,
        message: format!(
            "wire variant {} was retired in wire version {}; the negotiated wire version of this call does not serve it any longer",
            retired.variant, retired.retired_in_version
        ),
        action: "upgrade the calling binary so its Hello-negotiated wire version no longer sends retired variants".to_owned(),
    })
}

/// Process-start configuration for one broker run, resolved from CLI
/// flags and environment defaults by [`parse_command`].
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Fixed process-start authority profile. Requests cannot change it.
    pub profile: BrokerProfile,
    /// Stable authority label binding this process instance's state, socket,
    /// audit root, and caller identity.
    pub authority_id: String,
    pub socket_path: PathBuf,
    pub audit_dir: PathBuf,
    pub audit_retention_days: u32,
    /// The broker reads the bundle manifest from this server-configured
    /// path. The daemon never names a bundle path on the wire (security:
    /// prevents path-traversal + symlink-confusion). Defaults to
    /// `/var/lib/d2b/current-bundle/manifest.json`; the NixOS module's
    /// `d2b.site.bundle.currentManifest` option overrides.
    pub bundle_path: PathBuf,
    pub state_dir: PathBuf,
    /// Trusted target-local activation helper. The daemon never supplies
    /// this path on the wire.
    pub activation_helper_path: PathBuf,
    pub d2bd_uid: u32,
    pub d2bd_gid: u32,
    /// Directory for the StoreSync-only observability JSONL export
    /// (ADR 0027). The broker appends a positive-allow-list projection of
    /// every terminal StoreSync audit record here; the host Nix/Alloy
    /// wiring grants the `alloy` identity focused read/traverse on this
    /// directory only. Defaults to
    /// `/var/lib/d2b/observability/store-sync`; override via
    /// `--store-sync-export-dir`.
    pub store_sync_export_dir: PathBuf,
    /// The process that serves the declared handlers of family-owned
    /// operations.
    ///
    /// The broker links no provider crate, so a committed family row's
    /// handler runs in the declaring crate's process: the dispatch step
    /// dials this socket and crosses the validated invocation to whoever
    /// answers there. `None` means no peer is configured, and every
    /// forwarded operation is then refused fail-closed. Resolved from
    /// `--forward-socket`, else `D2B_BROKER_FORWARD_SOCKET`.
    pub forward_socket_path: Option<PathBuf>,
    pub test_mode: bool,
    #[cfg(not(feature = "layer1-bootstrap"))]
    /// The wire variants this broker refuses with the stale-wire-version
    /// code, gated on the Hello-negotiated wire version (KTD10).
    ///
    /// Production serves [`RETIRED_WIRE_VARIANTS`]; a test injects a fixture
    /// table so the gate is exercised while the production table stays
    /// empty (no variant is retired until U10).
    pub retired_wire_variants: &'static [RetiredWireVariant],
}

/// The process mode selected by [`parse_command`]: host or guest serving,
/// or a bootstrap probe.
#[derive(Debug, Clone)]
pub enum BrokerMode {
    Host(ServerConfig),
    Guest(ServerConfig),
    #[cfg(feature = "layer1-bootstrap")]
    ProbeHello {
        socket_path: PathBuf,
        test_uid: Option<u32>,
    },
    #[cfg(feature = "layer1-bootstrap")]
    ProbeStub {
        socket_path: PathBuf,
        test_uid: Option<u32>,
        operation: String,
    },
    #[cfg(feature = "layer1-bootstrap")]
    ProbeExportAudit {
        socket_path: PathBuf,
        test_uid: Option<u32>,
        caller_role: CallerRole,
    },
}

/// A process-entry failure surfaced by [`parse_command`] or [`run`]:
/// usage, I/O, or protocol.
#[derive(Debug)]
pub enum RunError {
    Usage(String),
    Io(io::Error),
    Protocol(String),
}

impl From<io::Error> for RunError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub(crate) enum BrokerError {
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    MinijailValidation {
        reason: String,
    },
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    NoPidfd {
        runner_id: String,
    },
    /// The privileged implementation is staged but the bootstrap wire
    /// path does not carry the typed intent data required to call into
    /// `ops::*` yet. Emits a typed [`OpAuditRecord`] with
    /// `decision = "errored"` + `error_kind = "w3-pending-typed-wire"`.
    Unimplemented {
        operation: &'static str,
        target_wave: &'static str,
    },
    /// USBIP live device routing ops (`UsbipBind`, `UsbipUnbind`,
    /// `UsbipProxyReconcile`) were out of scope for the initial broker
    /// and were wired into the non-bootstrap real-wire dispatch later,
    /// so this variant is only constructed by the bootstrap dispatch arm.
    #[cfg_attr(not(feature = "layer1-bootstrap"), allow(dead_code))]
    UnknownOperation {
        operation: &'static str,
    },
    AuditRequiresAdmin,
    HostShutdownRestricted,
    /// Broker started without a loadable bundle at
    /// `ServerConfig.bundle_path`; bundle-dependent real-wire ops cannot
    /// resolve their `BundleOpId` refs and refuse fail-closed.
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    BundleResolverUnavailable,
    /// Bundle artifact at `ServerConfig.bundle_path` failed the
    /// tamper-resistance check (symlink / owner / mode / hash). Every
    /// incoming operation surfaces this error until the broker is
    /// restarted with a clean bundle.
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    BundleTampered {
        path: String,
        reason: String,
    },
    /// The daemon-supplied `bundle_*_intent_ref` did not resolve
    /// against the bundle's intent table.
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    BundleIntentMissing {
        kind: &'static str,
        intent_id: String,
    },
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    UsbipDeviceNotAllowed {
        busid: String,
        vendor: u16,
        product: u16,
    },
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    UsbipPolicyMismatch {
        busid: String,
        reason: &'static str,
    },
    /// `UsbipBind` refused because the busid lock is already held by a
    /// different VM. The daemon maps this to `LockConflict` via the
    /// wire kind `"Broker.UsbipLockConflict"` without string-matching on
    /// a redacted `LiveHandler` message.
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    UsbipLockConflict {
        busid: String,
        owner: String,
    },
    /// `UsbipBind` refused because the physical USB device is absent in
    /// sysfs. The daemon maps this to `RuntimeAbsent` via the wire kind
    /// `"Broker.UsbipDeviceAbsent"`.
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    UsbipDeviceAbsent {
        busid: String,
    },
    /// The live executor reported an error (nft/route/sysctl shellout
    /// failed, pidfd open failed, spawn preflight failed, etc).
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    LiveHandler(String),
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    CoexistenceRefused {
        manager: d2b_core::host_w3::FirewallManager,
        rationale: String,
    },
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    NftScriptParseFailed(String),
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    CarveoutOrderingViolation(String),
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    NftablesDriftDetected {
        expected: String,
        observed: String,
    },
    /// `SpawnRunner` was called with `RunnerRole::OtelHostBridge`, but
    /// the bundle-resolved intent points at a VM whose name does not
    /// match `manifest._observability.vmName`. The bridge MUST forward
    /// only into the obs VM declared in the trusted bundle; any other
    /// target is a closed-set violation and the broker refuses
    /// fail-closed.
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    OtelHostBridgeIntentInvalid {
        intent_vm: String,
        expected_obs_vm: String,
    },
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    SpawnRunnerIntentMismatch {
        field: &'static str,
        requested: String,
        resolved: String,
    },
    /// A `StoreSync` attempt failed (or was denied) after the dispatch
    /// arm already emitted the signed ADR 0027 terminal
    /// `OperationFields::StoreSync` audit record. This variant carries the
    /// classified `error_stage` slug for the wire error envelope; its
    /// [`BrokerError::audit`] is a deliberate no-op so the generic dispatch
    /// error path never writes a SECOND record for the same attempt
    /// (exactly one terminal StoreSync record per attempt).
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    StoreSyncFailed {
        error_stage: &'static str,
        message: String,
    },
    Protocol(String),
    PeerCredentialRefused {
        operation: &'static str,
    },
    ProfileOperationRefused {
        profile: BrokerProfile,
        operation: &'static str,
    },
    /// swtpm-dir first-run hardening (issue #64) refused to proceed.
    /// Carries the path-free [`OperationFields::PrepareSwtpmDir`] audit
    /// so the SpawnRunner dispatch arm emits exactly one terminal
    /// `PrepareSwtpmDir` record (its [`BrokerError::audit`] is a no-op,
    /// mirroring `StoreSyncFailed`). The wire envelope surfaces only the
    /// closed-set, path-free `reason` slug.
    #[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
    SwtpmDirHardening {
        audit: crate::ops::audit_op::SwtpmDirAudit,
        reason: &'static str,
    },
    RequestValidation {
        operation: &'static str,
        reason: &'static str,
    },
    /// A generic envelope invocation the committed rows do not admit.
    IpcRateLimited,
}

impl core::fmt::Debug for BrokerError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("BrokerError(<redacted>)")
    }
}

/// Parse process arguments into the [`BrokerMode`] to run.
///
/// # Errors
///
/// Returns [`RunError::Usage`] when the first argument is not a known
/// profile (or probe subcommand), a flag is missing or malformed, or the
/// d2bd uid/gid cannot be resolved.
pub fn parse_command<I>(args: I) -> Result<BrokerMode, RunError>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let subcommand = args
        .next()
        .ok_or_else(|| RunError::Usage("usage: d2b-broker [host|guest] [options]".to_owned()))?;
    #[cfg(feature = "layer1-bootstrap")]
    match subcommand.as_str() {
        "probe-hello" => {
            let (socket_path, test_uid) = parse_probe_flags(args.collect())?;
            return Ok(BrokerMode::ProbeHello {
                socket_path,
                test_uid,
            });
        }
        "probe-stub" => {
            let rest: Vec<String> = args.collect();
            let (socket_path, test_uid, operation) = parse_stub_flags(&rest)?;
            return Ok(BrokerMode::ProbeStub {
                socket_path,
                test_uid,
                operation,
            });
        }
        "probe-export-audit" => {
            let rest: Vec<String> = args.collect();
            let (socket_path, test_uid, caller_role) = parse_export_flags(&rest)?;
            return Ok(BrokerMode::ProbeExportAudit {
                socket_path,
                test_uid,
                caller_role,
            });
        }
        _ => {}
    }
    let profile = match subcommand.as_str() {
        "host" => BrokerProfile::Host,
        "guest" => BrokerProfile::Guest,
        other => {
            return Err(RunError::Usage(format!(
                "usage: d2b-broker [host|guest] [options] (unknown profile: {other})"
            )));
        }
    };
    // --socket-path is optional.  Resolution order:
    //   1. --socket-path flag (explicit override)
    //   2. D2B_BROKER_SOCKET_PATH env var
    //   3. DEFAULT_SOCKET_PATH constant ("/run/d2b/priv.sock")
    // Under SD_LISTEN_FDS=1 (socket activation) the resolved path is
    // informational only; the broker adopts fd 3 from systemd and
    // MUST NOT bind, fchmod, or fchown the socket path.
    let mut socket_path_override: Option<PathBuf> = None;
    let mut audit_dir = PathBuf::from(match profile {
        BrokerProfile::Host => DEFAULT_AUDIT_DIR,
        BrokerProfile::Guest => DEFAULT_GUEST_AUDIT_DIR,
    });
    let mut audit_retention_days = DEFAULT_AUDIT_RETENTION_DAYS;
    let mut bundle_path = PathBuf::from(match profile {
        BrokerProfile::Host => DEFAULT_BUNDLE_PATH,
        BrokerProfile::Guest => DEFAULT_GUEST_BUNDLE_PATH,
    });
    let mut state_dir = PathBuf::from(match profile {
        BrokerProfile::Host => DEFAULT_STATE_DIR,
        BrokerProfile::Guest => DEFAULT_GUEST_STATE_DIR,
    });
    let mut activation_helper_path = PathBuf::from(DEFAULT_ACTIVATION_HELPER_PATH);
    let mut store_sync_export_dir = match profile {
        BrokerProfile::Host => {
            PathBuf::from(crate::ops::store_sync_export::DEFAULT_STORE_SYNC_EXPORT_DIR)
        }
        BrokerProfile::Guest => PathBuf::from("/var/lib/d2b/guest-audit/store-sync"),
    };
    let mut authority_id = profile.as_str().to_owned();
    let mut forward_socket_override: Option<PathBuf> = None;
    let mut d2bd_uid = None;
    let mut d2bd_gid = None;
    let mut test_mode = false;
    let rest: Vec<String> = args.collect();
    let mut index = 0;
    while index < rest.len() {
        match rest[index].as_str() {
            "--socket-path" => {
                index += 1;
                socket_path_override =
                    Some(PathBuf::from(expect_arg(&rest, index, "--socket-path")?));
            }
            "--audit-dir" => {
                index += 1;
                audit_dir = PathBuf::from(expect_arg(&rest, index, "--audit-dir")?);
            }
            "--audit-retention-days" => {
                index += 1;
                audit_retention_days = expect_arg(&rest, index, "--audit-retention-days")?
                    .parse()
                    .map_err(|_| {
                        RunError::Usage(
                            "invalid --audit-retention-days (expected a non-negative integer; 0 disables pruning)"
                                .to_owned(),
                        )
                    })?;
            }
            "--bundle-path" => {
                // Broker reads the bundle manifest from this
                // server-configured path so the daemon never
                // names a bundle path on the wire.
                index += 1;
                bundle_path = PathBuf::from(expect_arg(&rest, index, "--bundle-path")?);
            }
            "--state-dir" => {
                index += 1;
                state_dir = PathBuf::from(expect_arg(&rest, index, "--state-dir")?);
            }
            "--activation-helper-path" => {
                index += 1;
                activation_helper_path =
                    PathBuf::from(expect_arg(&rest, index, "--activation-helper-path")?);
            }
            "--store-sync-export-dir" => {
                index += 1;
                store_sync_export_dir =
                    PathBuf::from(expect_arg(&rest, index, "--store-sync-export-dir")?);
            }
            "--forward-socket" => {
                index += 1;
                forward_socket_override =
                    Some(PathBuf::from(expect_arg(&rest, index, "--forward-socket")?));
            }
            "--authority-id" => {
                index += 1;
                authority_id = expect_arg(&rest, index, "--authority-id")?.to_owned();
                if authority_id.is_empty() {
                    return Err(RunError::Usage(
                        "--authority-id must not be empty".to_owned(),
                    ));
                }
            }
            "--d2bd-uid" => {
                index += 1;
                d2bd_uid = Some(
                    expect_arg(&rest, index, "--d2bd-uid")?
                        .parse()
                        .map_err(|_| RunError::Usage("invalid --d2bd-uid".to_owned()))?,
                );
            }
            "--d2bd-gid" => {
                index += 1;
                d2bd_gid = Some(
                    expect_arg(&rest, index, "--d2bd-gid")?
                        .parse()
                        .map_err(|_| RunError::Usage("invalid --d2bd-gid".to_owned()))?,
                );
            }
            "--test-mode" => test_mode = true,
            other => {
                return Err(RunError::Usage(format!(
                    "unknown {} flag: {other}",
                    profile.as_str()
                )));
            }
        }
        index += 1;
    }

    // Resolve socket path: flag > env var > built-in default.
    let socket_path = socket_path_override
        .or_else(|| {
            let variable = match profile {
                BrokerProfile::Host => "D2B_BROKER_SOCKET_PATH",
                BrokerProfile::Guest => "D2B_GUEST_BROKER_SOCKET_PATH",
            };
            env::var(variable).ok().map(PathBuf::from)
        })
        .unwrap_or_else(|| {
            PathBuf::from(match profile {
                BrokerProfile::Host => DEFAULT_SOCKET_PATH,
                BrokerProfile::Guest => DEFAULT_GUEST_SOCKET_PATH,
            })
        });

    let fallback_uid = if test_mode {
        nix::unistd::Uid::current().as_raw()
    } else {
        env::var("D2BD_UID")
            .ok()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| {
                RunError::Usage("missing d2bd uid: pass --d2bd-uid or set D2BD_UID".to_owned())
            })?
    };
    let fallback_gid = if test_mode {
        nix::unistd::Gid::current().as_raw()
    } else {
        env::var("D2BD_GID")
            .ok()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| {
                RunError::Usage("missing d2bd gid: pass --d2bd-gid or set D2BD_GID".to_owned())
            })?
    };

    let config = ServerConfig {
        profile,
        authority_id,
        socket_path,
        audit_dir,
        audit_retention_days,
        bundle_path,
        state_dir,
        activation_helper_path,
        d2bd_uid: d2bd_uid.unwrap_or(fallback_uid),
        d2bd_gid: d2bd_gid.unwrap_or(fallback_gid),
        store_sync_export_dir,
        // The declaring process that serves family handlers: the flag wins,
        // otherwise the environment names it, otherwise no peer is wired and
        // every forwarded operation refuses fail-closed.
        forward_socket_path: forward_socket_override.or_else(|| {
            env::var(crate::forwarding::FORWARD_SOCKET_ENV)
                .ok()
                .map(PathBuf::from)
        }),
        test_mode,
#[cfg(not(feature = "layer1-bootstrap"))]
        // No variant is retired on this tree yet; the gate ships with the
        // production table empty and a fixture table exercising it.
        retired_wire_variants: RETIRED_WIRE_VARIANTS,
    };
    Ok(match profile {
        BrokerProfile::Host => BrokerMode::Host(config),
        BrokerProfile::Guest => BrokerMode::Guest(config),
    })
}

/// Run the broker in the parsed [`BrokerMode`], serving until termination.
///
/// # Errors
///
/// Returns [`RunError::Io`] when the socket cannot be adopted or bound,
/// [`RunError::Protocol`] when the wire negotiation or a request fails
/// fatally, and [`RunError::Usage`] for a malformed probe invocation.
pub fn run(command: BrokerMode) -> Result<(), RunError> {
    match command {
        BrokerMode::Host(config) | BrokerMode::Guest(config) => run_server(config),
        #[cfg(feature = "layer1-bootstrap")]
        BrokerMode::ProbeHello {
            socket_path,
            test_uid,
        } => run_probe(
            socket_path,
            crate::bootstrap::wire::probe_hello(test_uid),
            true,
        ),
        #[cfg(feature = "layer1-bootstrap")]
        BrokerMode::ProbeStub {
            socket_path,
            test_uid,
            operation,
        } => {
            let request = crate::bootstrap::wire::probe_stub(&operation, test_uid)
                .ok_or_else(|| RunError::Usage(format!("unknown stub operation: {operation}")))?;
            run_probe(socket_path, request, true)
        }
        #[cfg(feature = "layer1-bootstrap")]
        BrokerMode::ProbeExportAudit {
            socket_path,
            test_uid,
            caller_role,
        } => run_probe(
            socket_path,
            crate::bootstrap::wire::probe_export_audit(test_uid, caller_role),
            false,
        ),
    }
}

/// Attempt to adopt a socket-activated listen fd from systemd's
/// `SD_LISTEN_FDS` protocol.
///
/// Returns:
/// - `None` if `LISTEN_PID` is absent or does not match this process's PID,
///   or if `LISTEN_FDS` is absent or not `"1"` - not socket-activated.
/// - `Some(Ok(fd))` when socket activation is valid and fd 3 has been
///   verified as an `AF_UNIX SOCK_SEQPACKET` listen socket.
/// - `Some(Err(_))` if `LISTEN_FDNAMES` is present but is not `"priv.sock"`,
///   or if the fd-level validation in `sys::adopt_listen_fd_from_fd3` fails.
///
/// The `LISTEN_*` vars are NOT unset after adoption. The `sd_listen_fds(3)`
/// protocol is self-scoping: a reader only honours the vars when
/// `LISTEN_PID` equals its own PID, so any spawned child (a different PID)
/// ignores inherited `LISTEN_*` regardless. The broker also never re-reads
/// them after this function, and per-runner processes receive an explicit
/// (non-inherited) environment via `execve`, so leaving the vars in the
/// broker's own short-lived environment is inert.
fn adopt_listen_fd() -> Option<Result<OwnedFd, RunError>> {
    // Step 1: LISTEN_PID must match this process.
    let listen_pid = env::var("LISTEN_PID").ok()?;
    if listen_pid != std::process::id().to_string() {
        return None;
    }

    // Step 2: LISTEN_FDS must be exactly "1".
    let listen_fds = env::var("LISTEN_FDS").ok()?;
    if listen_fds != "1" {
        return None;
    }

    // Step 3: If LISTEN_FDNAMES is present it must equal "priv.sock".
    if let Ok(fdnames) = env::var("LISTEN_FDNAMES")
        && fdnames != "priv.sock"
    {
        return Some(Err(RunError::Usage(format!(
            "socket activation: expected LISTEN_FDNAMES=priv.sock, \
                 got {fdnames:?}"
        ))));
    }

    // Steps 4-5: verify fd 3 + set CLOEXEC + wrap in OwnedFd (sys.rs). The
    // `LISTEN_*` vars are intentionally left in place; see the fn docs for
    // why that is inert (LISTEN_PID self-scoping + explicit runner env).
    Some(crate::sys::adopt_listen_fd_from_fd3().map_err(RunError::Io))
}

/// Send `READY=1` (and `MAINPID=<pid>`) to `$NOTIFY_SOCKET` via the
/// `sd_notify(3)` protocol.
///
/// Failures are logged at WARN level but are not fatal - the broker
/// continues serving even if the notification cannot be delivered.
/// This preserves behaviour in environments that do not use systemd
/// supervision (tests, containers).
fn sd_notify_ready() {
    use nix::sys::socket::{AddressFamily, MsgFlags, SockFlag, SockType, UnixAddr, sendto, socket};

    let notify_socket = match env::var("NOTIFY_SOCKET") {
        Ok(s) if !s.is_empty() => s,
        _ => return, // not under systemd supervision - skip silently
    };

    let addr: UnixAddr = if let Some(abstract_name) = notify_socket.strip_prefix('@') {
        // Abstract namespace: sd_notify passes "@ <name>" where the kernel
        // address has a leading NUL byte.
        match UnixAddr::new_abstract(abstract_name.as_bytes()) {
            Ok(a) => a,
            Err(err) => {
                warn!(
                    error = %err,
                    notify_result = "invalid",
                    "sd_notify: invalid abstract socket address; skipping"
                );
                return;
            }
        }
    } else {
        match UnixAddr::new(std::path::Path::new(&notify_socket)) {
            Ok(a) => a,
            Err(err) => {
                warn!(
                    error = %err,
                    notify_result = "invalid",
                    "sd_notify: invalid NOTIFY_SOCKET path; skipping"
                );
                return;
            }
        }
    };

    let sock = match socket(
        AddressFamily::Unix,
        SockType::Datagram,
        SockFlag::SOCK_CLOEXEC,
        None,
    ) {
        Ok(fd) => fd,
        Err(err) => {
            warn!(error = %err, "sd_notify: failed to create datagram socket; skipping");
            return;
        }
    };

    let msg = format!("READY=1\nMAINPID={}\n", std::process::id());
    match sendto(sock.as_raw_fd(), msg.as_bytes(), &addr, MsgFlags::empty()) {
        Ok(_) => tracing::info!(notify_result = "sent", "sd_notify: READY=1 sent"),
        Err(err) => warn!(error = %err, notify_result = "failed", "sd_notify: sendto failed"),
    }
}

/// Reactor worker threads: the accept loop, the frame I/O, the reap loop, and
/// the background retries. No request body runs here, so the count is small
/// and fixed rather than a cap on how much work is in flight.
const SERVER_WORKER_THREADS: usize = 4;

/// Connections the broker admits before the accept loop waits for one to
/// finish. The listen backlog holds the dials this process has not admitted.
const MAX_INFLIGHT_CONNECTIONS: usize = 64;

/// Worker threads behind the dispatch pool, at least one more than one so two
/// requests never queue behind one another.
///
/// The floor must cover the worst concurrent nesting the seam produces:
/// outer forwarded invocations (a supervisor's SpawnRunner family) park a
/// worker on `block_on` for their whole forward round trip, and the nested
/// EnvelopeInvoke calls their handlers present arrive as new requests on
/// this same pool. A floor below (concurrent outer calls + their nested
/// legs) starves the nested legs until the outer io budgets expire - the
/// 10s all-wave spawn stall. Eight matches the envelope-call runtime's
/// worker count, so a worker-parked outer call never starves its own
/// nested leg's dispatch.
const MIN_DISPATCH_WORKERS: usize = 8;

/// Upper bound on the dispatch pool: a stalled peer or a slow subprocess must
/// not grow the broker's thread count with its callers.
const MAX_DISPATCH_WORKERS: usize = 16;
/// Jobs one dispatch worker queues before its callers wait.
const DISPATCH_QUEUE_PER_WORKER: usize = 1;
/// Worker threads behind the nested-invocation dispatch pool.
///
/// A handler's nested (sandwich) EnvelopeInvoke presents its evidence chain
/// and must never wait on a worker parked by an outer forwarded call: the
/// outer call blocks its worker on `block_on` until the nested leg's reply
/// crosses back, so one shared round-robin pool can land the nested leg on
/// a pinned worker's queue and deadlock both until the io budgets expire.
/// A separate pool makes that starvation impossible by construction; the
/// checks' deepest nesting is one level (family handler -> kernel row).
const NESTED_DISPATCH_WORKERS: usize = 4;

/// One job on the dispatch pool.
type DispatchJob = Box<dyn FnOnce() + Send + 'static>;

/// The bounded pool the broker runs a request's synchronous kernel path on.
///
/// The broker's request body has no async form: it reloads the trusted bundle
/// from disk, spawns and signals children, walks the filesystem under
/// `openat2`, and appends to the audit log. Running it on a reactor worker
/// would stall every other connection's frame I/O, and running it on a thread
/// per call would let a caller pick the broker's thread count. So it runs
/// here: a fixed set of worker threads, one bounded queue each, and an async
/// caller that waits for its own job's reply. A full queue is backpressure on
/// the connection task, not a blocked thread.
pub(crate) struct DispatchPool {
    queues: Vec<tokio::sync::mpsc::Sender<DispatchJob>>,
    next: std::sync::atomic::AtomicUsize,
}

impl DispatchPool {
    pub(crate) fn new(workers: usize) -> Arc<Self> {
        let mut queues = Vec::with_capacity(workers);
        for index in 0..workers {
            let (queue, mut jobs) =
                tokio::sync::mpsc::channel::<DispatchJob>(DISPATCH_QUEUE_PER_WORKER);
            queues.push(queue);
            std::thread::Builder::new()
                .name(format!("d2b-broker-dispatch-{index}"))
                .spawn(move || {
                    while let Some(job) = jobs.blocking_recv() {
                        // A panicking handler must cost its own connection,
                        // not the worker: the pool keeps its workers, and the
                        // waiting caller sees the reply channel close.
                        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).is_err() {
                            tracing::error!(
                                "broker request body panicked; the connection is closed and \
                                 the broker keeps serving"
                            );
                        }
                    }
                })
                .expect("spawn broker dispatch worker");
        }
        Arc::new(Self {
            queues,
            next: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// Run one job on a pool worker and hand back its result.
    pub(crate) async fn run<T: Send + 'static>(
        &self,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, DispatchPoolClosed> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        let job: DispatchJob = Box::new(move || {
            let _ = reply.send(job());
        });
        let index =
            self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % self.queues.len();
        self.queues[index]
            .send(job)
            .await
            .map_err(|_| DispatchPoolClosed)?;
        answer.await.map_err(|_| DispatchPoolClosed)
    }
}

impl Drop for DispatchPool {
    /// Closing every queue ends its worker's loop once the queue drains; a job
    /// already running finishes on the thread that owns it.
    fn drop(&mut self) {
        self.queues.clear();
    }
}

/// The dispatch pool is gone: the broker is shutting down, so a request in
/// flight is answered by nothing.
#[derive(Debug)]
pub(crate) struct DispatchPoolClosed;

/// How many dispatch workers this process runs.
///
/// Derived from the machine so a wide host does not serialize behind four
/// workers, clamped so a narrow host still runs two requests at once and
/// neither extreme turns the pool into an unbounded thread source.
fn dispatch_workers() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(MIN_DISPATCH_WORKERS)
        .clamp(MIN_DISPATCH_WORKERS, MAX_DISPATCH_WORKERS)
}

/// The broker's serving state: one accepted connection is handled against it.
struct Server {
    config: Arc<ServerConfig>,
    audit_log: Arc<AuditLog>,
    dispatches: Arc<DispatchPool>,
    /// Nested (sandwich) EnvelopeInvoke legs get their own pool: an outer
    /// forwarded call parks its worker until the nested leg's reply returns,
    /// so a shared pool can deadlock the pair (see NESTED_DISPATCH_WORKERS).
    nested_dispatches: Arc<DispatchPool>,
    /// The per-uid IPC admission limiter. `tokio::sync` per plan U8: the
    /// check runs on the synchronous dispatch workers, which reach it
    /// through the non-blocking `try_lock` (never held across an await).
    ipc_rate_limiter: Arc<tokio::sync::Mutex<IpcRateLimiter>>,
}
/// The process-lifetime handles background work reaches the broker's runtime
/// through.
///
/// A background step that outlives the request that scheduled it - the
/// obs-vsock ACL refresh above all - runs as a task on this reactor with an
/// async deadline, and any blocking attempt inside it goes to this pool
/// rather than to a thread of its own.
pub(crate) struct BrokerBackground {
    pub(crate) runtime: tokio::runtime::Handle,
    pub(crate) dispatches: Arc<DispatchPool>,
}

static BROKER_BACKGROUND: std::sync::OnceLock<BrokerBackground> = std::sync::OnceLock::new();

/// The running broker's background handles, absent until `run_server` starts
/// the reactor.
pub(crate) fn broker_background() -> Option<&'static BrokerBackground> {
    BROKER_BACKGROUND.get()
}

fn run_server(config: ServerConfig) -> Result<(), RunError> {
    let listener = match adopt_listen_fd() {
        Some(Ok(fd)) => {
            // Socket-activated: systemd owns bind+listen+ACL.
            // We MUST NOT touch socket_path / fchmod / fchown.
            tracing::info!(
                activation_mode = "systemd",
                socket_owner = "systemd",
                "broker adopted socket-activated listen fd"
            );
            fd
        }
        Some(Err(err)) => return Err(err),
        None => {
            // Not socket-activated: legacy / test mode - bind ourselves.
            validate_socket_parent(&config.socket_path, config.test_mode)?;
            prepare_socket_path(&config.socket_path)?;
            // fchmod() on an AF_UNIX socket fd does not change the bound
            // path's mode on some kernels/filesystems (verified: a socket
            // bound under umask 0o022 stays 0o755 after fchmod 0o660), so
            // constrain the creation umask around bind() so the socket is
            // materialized at 0o660 directly. The fchmod below stays as a
            // belt-and-suspenders for kernels where it does take effect.
            // Production uses socket activation (systemd owns the mode);
            // this is only the non-socket-activated fallback, and the
            // broker is single-threaded at startup so the transient
            // process-wide umask change is race-free.
            let prev_umask = nix::sys::stat::umask(nix::sys::stat::Mode::from_bits_truncate(0o117));
            let listener_result = bind_seqpacket(&config.socket_path);
            nix::sys::stat::umask(prev_umask);
            let listener = listener_result?;
            path_safe::fchmod(listener.as_fd(), 0o660)?;
            if !config.test_mode {
                path_safe::fchown(listener.as_fd(), Some(0), Some(config.d2bd_gid))?;
            }
            listener
        }
    };

    // Open the broker's state cell store under the daemon state root (the
    // same root the daemon's ProviderSet state lives under) and recover the
    // durable one-time records; a malformed durable file fails the broker
    // closed at startup rather than replaying grants silently.
    #[cfg(not(feature = "layer1-bootstrap"))]
    crate::state_cells::init_broker_store(&config.state_dir)
        .map_err(|error| RunError::Protocol(format!("state cell store: {error}")))?;

    // The trusted-context store is deliberately NOT opened at startup: a
    // state root that cannot host `trusted-context/` (a read-only or absent
    // daemon state dir in a constrained sandbox) must not take the whole
    // broker down. The store opens lazily on the FIRST PublishTrustedContext
    // arrival (see that dispatch arm): an open failure there refuses that
    // publication fail-closed while the broker keeps serving everything
    // else, and the rendezvous stays fail-closed on its zero epoch because
    // it only ever advances on an acknowledged epoch.

    let audit_log = Arc::new(AuditLog::open(
        &config.audit_dir,
        config.d2bd_gid,
        config.test_mode,
        config.audit_retention_days,
    )?);

    // Install the committed-operation envelope before any connection is
    // accepted: the kernel seam's handlers capture the fixed process
    // config, and every dispatch resolves its operations against this one
    // envelope (U10). The chain-audit sink writes the in-broker leg
    // records (root record per invocation, correlation record per nested
    // leg) into the same daily audit log the typed arms use, so the
    // envelope's KTD6 correlation surface is durable in production.
    #[cfg(not(feature = "layer1-bootstrap"))]
    install_live_operation_envelope(&config, &audit_log)?;

    // Signal systemd that the broker is ready to accept connections.
    // Called after the listener is established and the audit log is open,
    // before entering the accept loop.  No-op when NOTIFY_SOCKET is absent.
    sd_notify_ready();

    // One reactor serves every accepted connection, the SIGCHLD reap loop,
    // and the background retries. Nothing on it blocks: a request's
    // synchronous body runs on the dispatch pool, so the reactor stays free
    // to accept and to move frames.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(SERVER_WORKER_THREADS)
        .thread_name("d2b-broker")
        .enable_all()
        .build()
        .map_err(RunError::Io)?;
    let dispatches = DispatchPool::new(dispatch_workers());
    // Background work reaches the same reactor and the same bounded pool the
    // request path uses, so a retry is a task here rather than a thread.
    let _ = BROKER_BACKGROUND.set(BrokerBackground {
        runtime: runtime.handle().clone(),
        dispatches: Arc::clone(&dispatches),
    });

    // Start background SIGCHLD reap loop on the shared reactor.
    #[cfg(not(feature = "layer1-bootstrap"))]
    start_sigchld_reaper(&runtime, Arc::clone(&audit_log));

    let nested_dispatches = DispatchPool::new(NESTED_DISPATCH_WORKERS);
    let server = Arc::new(Server {
        config: Arc::new(config),
        audit_log,
        dispatches,
        nested_dispatches,
        ipc_rate_limiter: Arc::new(tokio::sync::Mutex::new(IpcRateLimiter::new(
            DEFAULT_IPC_REQUESTS_PER_UID_PER_SECOND,
        ))),
    });

    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    runtime.block_on(serve(server, listener))
}

/// Outcome of a bundle load attempt at broker startup.
#[cfg(not(feature = "layer1-bootstrap"))]
#[derive(Debug)]
pub(crate) enum BundleSlot {
    /// Bundle loaded and verified successfully.
    Loaded(Arc<BundleResolver>),
    /// Bundle absent or unreadable; bundle-dependent ops return
    /// `BundleResolverUnavailable`.
    Unavailable,
    /// Bundle failed tamper-resistance check; every incoming operation
    /// immediately surfaces `BundleTampered` until the broker restarts.
    Tampered { path: String, reason: String },
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn try_load_resolver(bundle_path: &Path) -> BundleSlot {
    try_load_resolver_with_policy(
        bundle_path,
        &d2b_core::bundle_resolver::BundleVerifyPolicy::production(),
    )
}

/// Load the bundle resolver for one kernel invocation. Production uses
/// the production verify policy (`root:d2bd`, mode 0640); under
/// `cfg(test)` the test policy accepts the invoking principal so a
/// per-test temp bundle loads. The kernel needs a resolver only for the
/// USBIP backend device-bind extension; every other invocation stays
/// bundle-free.
#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn load_kernel_resolver(bundle_path: &Path) -> BundleSlot {
    #[cfg(test)]
    {
        // Unit tests inject the prebuilt in-memory resolver of their
        // per-test bundle (identical to what the daemon side passes the
        // arm); the on-disk reload stays the fallback.
        if let Some(resolver) = TEST_KERNEL_BUNDLE_RESOLVER.get() {
            return BundleSlot::Loaded(resolver.clone());
        }
        try_load_resolver_with_policy(
            bundle_path,
            &d2b_core::bundle_resolver::BundleVerifyPolicy::for_tests(),
        )
    }
    #[cfg(not(test))]
    {
        try_load_resolver(bundle_path)
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn try_load_resolver_with_policy(
    bundle_path: &Path,
    policy: &d2b_core::bundle_resolver::BundleVerifyPolicy,
) -> BundleSlot {
    use d2b_core::error::{BundleError, Error as CoreError};
    // Per the tracing contract, span attributes MUST NOT include
    // filesystem paths (high cardinality + can leak host layout). The
    // bundle path is bounded operational context handled by the typed
    // error envelope + audit log, not the trace. Keep traces to bounded
    // attrs: result/outcome/reason/intent-counts.
    match BundleResolver::load_with_policy(bundle_path, policy) {
        Ok(resolver) => {
            tracing::debug!(
                load_outcome = "ok",
                nft = resolver.nft_intent_ids().count(),
                route = resolver.route_intent_ids().count(),
                sysctl = resolver.sysctl_intent_ids().count(),
                hosts = resolver.hosts_intent_ids().count(),
                runners = resolver.runner_intent_ids().count(),
                "Bundle resolver loaded"
            );
            BundleSlot::Loaded(Arc::new(resolver))
        }
        Err(CoreError::Bundle(BundleError::Tampered { path, reason })) => {
            tracing::error!(
                load_outcome = "tampered",
                reason = %reason,
                "Bundle tamper-resistance check failed; all ops will be refused until broker restarts with a clean bundle"
            );
            BundleSlot::Tampered {
                path: path.display().to_string(),
                reason,
            }
        }
        Err(err) => {
            warn!(
                load_outcome = "unavailable",
                error_kind = ?err.kind(),
                "Bundle resolver could not load; bundle-dependent ops will fail closed"
            );
            BundleSlot::Unavailable
        }
    }
}

/// Test helper: load the bundle at `bundle_path` through the same
/// `try_load_resolver` path as the broker's `serve` loop and convert the
/// resulting `BundleSlot` into a `BrokerResponse`. Returns
/// `BrokerResponse::Error { kind: "bundle-tampered" }` when the bundle
/// fails its tamper-resistance check, and
/// `BrokerResponse::Error { kind: "Broker.BundleResolverUnavailable" }` when
/// the bundle is absent or unreadable. Exposed for the
/// `bundle_tampered_broker` integration test.
#[cfg(not(feature = "layer1-bootstrap"))]
pub fn probe_bundle_load_response(bundle_path: &std::path::Path) -> BrokerResponse {
    match try_load_resolver(bundle_path) {
        BundleSlot::Loaded(_) => ack_response("BundleLoad"),
        BundleSlot::Unavailable => BrokerError::BundleResolverUnavailable.into_response(),
        BundleSlot::Tampered { path, reason } => {
            BrokerError::BundleTampered { path, reason }.into_response()
        }
    }
}

/// Like [`probe_bundle_load_response`] but uses an explicit [`BundleVerifyPolicy`].
/// Tests that need to control uid/gid/mode requirements (e.g. to avoid requiring
/// root in CI) pass `current_user_policy()` so the uid check passes and only the
/// intended tamper reason fires.
#[cfg(not(feature = "layer1-bootstrap"))]
pub fn probe_bundle_load_response_with_policy(
    bundle_path: &std::path::Path,
    policy: &d2b_core::bundle_resolver::BundleVerifyPolicy,
) -> BrokerResponse {
    match try_load_resolver_with_policy(bundle_path, policy) {
        BundleSlot::Loaded(_) => ack_response("BundleLoad"),
        BundleSlot::Unavailable => BrokerError::BundleResolverUnavailable.into_response(),
        BundleSlot::Tampered { path, reason } => {
            BrokerError::BundleTampered { path, reason }.into_response()
        }
    }
}

/// Bind a kernel-authenticated peer to this broker instance before any wire
/// bytes are decoded. Test mode keeps the existing simulated envelope UID
/// support, but still requires the actual local test process credentials.
fn peer_matches_instance(config: &ServerConfig, peer_uid: u32, peer_gid: u32) -> bool {
    if config.test_mode {
        return (peer_uid == nix::unistd::Uid::current().as_raw()
            && peer_gid == nix::unistd::Gid::current().as_raw())
            || peer_uid == 0;
    }
    (peer_uid == config.d2bd_uid && peer_gid == config.d2bd_gid)
        || (config.profile == BrokerProfile::Host && peer_uid == 0)
}

/// Accept and serve connections until the listener itself fails.
///
/// One accepted connection becomes one task, so the accept path never runs a
/// request inline and a caller that connects and then stalls holds a waiting
/// task rather than the broker. The in-flight count is capped: past the
/// ceiling the loop stops accepting, and the listen backlog - not this
/// process - holds the dials it cannot serve yet.
async fn serve(server: Arc<Server>, listener: OwnedFd) -> Result<(), RunError> {
    let listener = AsyncSeqpacketListener::from_owned(listener).map_err(RunError::Io)?;
    let gate = Arc::new(tokio::sync::Semaphore::new(MAX_INFLIGHT_CONNECTIONS));
    loop {
        // Owned, because the permit travels into the connection's task.
        let permit = match Arc::clone(&gate).acquire_owned().await {
            Ok(permit) => permit,
            // The gate is owned here and never closed.
            Err(_) => {
                return Err(RunError::Protocol(
                    "broker connection gate closed".to_owned(),
                ));
            }
        };
        let connection = match listener.accept().await {
            Ok(connection) => connection,
            Err(err) => {
                warn!(error = %err, "broker accept failed");
                return Err(RunError::Io(err));
            }
        };
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(err) = handle_connection(connection, &server).await {
                warn!(error = ?err, "broker request failed");
            }
        });
    }
}

/// Serve one accepted connection.
///
/// The order the synchronous server used is preserved: the peer is
/// authenticated before a single frame byte is decoded, and a peer this
/// instance does not admit is closed without reading its bytes. What changed
/// is where each wait happens - the frame read and write wait in async time,
/// and the request's own kernel work runs on the dispatch pool.
async fn handle_connection(connection: AsyncSeqpacket, server: &Server) -> io::Result<()> {
    let (peer_uid, peer_gid, peer_pid) = peer_credentials(connection.as_raw_fd())?;
    if !peer_matches_instance(&server.config, peer_uid, peer_gid) {
        // Do not decode or drain an unauthenticated peer's frame: closing the
        // accepted socket is the fail-closed response. The append is one of
        // the writes with no async form, so it runs on the dispatch pool.
        let audit_log = Arc::clone(&server.audit_log);
        let _ = server
            .dispatches
            .run(move || {
                write_refusal_audit_bounded(
                    &audit_log,
                    AuditWriteClass::Unprivileged,
                    "PeerAuthentication",
                    peer_uid,
                    peer_gid,
                    "peer-refused-before-decode",
                    "broker-instance",
                    "closed",
                )
            })
            .await;
        return Ok(());
    }
    #[cfg(not(feature = "layer1-bootstrap"))]
    let (envelope, request_fds) = {
        // The frame decodes as JSON first so the retired-wire gate can
        // recognize a variant the current enum no longer carries: a retired
        // variant's frame is well-formed JSON but not a current
        // `RequestEnvelope`, and the gate refuses it with the typed
        // stale-wire-version code plus an audit record before the typed
        // decode can drop it as malformed wire (KTD10).
        let Some((envelope_value, request_fds)) =
            connection.recv_json_frame_with_fds::<Value>().await?
        else {
            return Ok(());
        };
        if let Some(retired) =
            retired_variant_for(&envelope_value, server.config.retired_wire_variants)
        {
            // The closing refusal of a straggler call: the frame's
            // descriptors are closed with the frame, the audit record is
            // appended on the dispatch pool like every other append with no
            // async form, and the peer is answered with the typed refusal -
            // never a pre-dispatch malformed-wire drop.
            let audit_log = Arc::clone(&server.audit_log);
            let variant = retired.variant;
            let _ = server
                .dispatches
                .run(move || {
                    write_refusal_audit_bounded(
                        &audit_log,
                        AuditWriteClass::Privileged,
                        variant,
                        peer_uid,
                        peer_gid,
                        "stale-wire-version",
                        "broker-instance",
                        "refused",
                    )
                })
                .await;
            connection
                .send_json_frame(&stale_wire_refusal(retired))
                .await?;
            return Ok(());
        }
        let envelope: RequestEnvelope = serde_json::from_value(envelope_value)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        (envelope, request_fds)
    };
    #[cfg(feature = "layer1-bootstrap")]
    let Some((envelope, request_fds)) = connection
        .recv_json_frame::<RequestEnvelope>()
        .await
        .map(|frame| frame.map(|envelope| (envelope, Vec::new())))?
    else {
        return Ok(());
    };

    let config = Arc::clone(&server.config);
    let audit_log = Arc::clone(&server.audit_log);
    let ipc_rate_limiter = Arc::clone(&server.ipc_rate_limiter);
    // A nested (sandwich) EnvelopeInvoke - one that presents an evidence
    // chain - is served by its own pool. Its invoking handler is blocked on
    // this leg's reply inside an outer forwarded call, so routing it onto
    // the outer pool can park it behind the very call that waits on it
    // (NESTED_DISPATCH_WORKERS).
    #[cfg(not(feature = "layer1-bootstrap"))]
    let nested_leg = matches!(
        &envelope.request,
        d2b_contracts_broker::broker_wire::BrokerRequest::EnvelopeInvoke(req)
            if req.chain_root_invocation_id.is_some()
    );
    // The layer1-bootstrap protocol carries no envelope chain (its request
    // type is BootstrapCall), so no leg can present a chain root.
    #[cfg(feature = "layer1-bootstrap")]
    let nested_leg = false;
    let pool = if nested_leg {
        Arc::clone(&server.nested_dispatches)
    } else {
        Arc::clone(&server.dispatches)
    };
    let outcome = pool
        .run(move || {
            // The dispatch chain is async end-to-end (the backend trait and
            // the dispatch fns await in async time), but the pool workers are
            // plain threads with no executor. The job bridges the boundary
            // with `block_on` on the broker's dispatch runtime - the same
            // bridge the envelope arms used before the chain went async - so
            // the pool's bounded-worker semantics are unchanged.
            #[cfg(not(feature = "layer1-bootstrap"))]
            {
                #[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
                envelope_call_runtime().block_on(answer_request(
                    envelope,
                    request_fds,
                    peer_uid,
                    peer_gid,
                    peer_pid,
                    &config,
                    &audit_log,
                    &ipc_rate_limiter,
                ))
            }
            #[cfg(feature = "layer1-bootstrap")]
            {
                bootstrap_dispatch_runtime().block_on(answer_request(
                    envelope,
                    request_fds,
                    peer_uid,
                    peer_gid,
                    peer_pid,
                    &config,
                    &audit_log,
                    &ipc_rate_limiter,
                ))
            }
        })
        .await
        // A pool that is gone answered nothing: the caller is closed rather
        // than read a frame that was never produced.
        .map_err(|_| io::Error::other("broker dispatch pool is not running"))??;

    match outcome {
        RequestOutcome::Reply(response, fds) => {
            #[cfg(not(feature = "layer1-bootstrap"))]
            {
                if fds.is_empty() {
                    connection.send_json_frame(&response).await
                } else {
                    let raw_fds: Vec<i32> = fds.iter().map(AsRawFd::as_raw_fd).collect();
                    connection
                        .send_json_frame_with_fds(&response, &raw_fds)
                        .await?;
                    // Drop ownership: the SCM_RIGHTS send duplicated the fd
                    // into the receiver's table; the broker's copy is the
                    // OwnedFd in `fds` and will close on scope exit, which is
                    // the intended lifecycle.
                    drop(fds);
                    Ok(())
                }
            }
            #[cfg(feature = "layer1-bootstrap")]
            {
                let _ = fds; // layer1-bootstrap dispatch never returns fds
                connection.send_json_frame(&response).await
            }
        }
        RequestOutcome::Silence => Ok(()),
    }
}

/// What the synchronous half of one request decided the caller is owed.
enum RequestOutcome {
    /// Write this response frame back to the caller.
    Reply(BrokerResponse, Vec<OwnedFd>),
    /// Close without a frame: the caller is owed no response at all.
    Silence,
}

/// The synchronous half of one request: everything between the decoded frame
/// and the response.
///
/// These are the steps with no async form - the per-request bundle reload,
/// the handlers' subprocess and filesystem work, the audit append - so they
/// run on the dispatch pool, whose workers bound the work and whose queue
/// bounds the waiters, rather than on a reactor worker or a thread per call.
async fn answer_request(
    envelope: RequestEnvelope,
    request_fds: Vec<OwnedFd>,
    peer_uid: u32,
    peer_gid: u32,
    peer_pid: i32,
    config: &ServerConfig,
    audit_log: &AuditLog,
    ipc_rate_limiter: &tokio::sync::Mutex<IpcRateLimiter>,
) -> io::Result<RequestOutcome> {
    #[cfg(feature = "layer1-bootstrap")]
    let _ = &request_fds; // the bootstrap wire carries no request descriptors
    // Load the bundle resolver from the configured `bundle_path` for every
    // request. The broker is socket-activated but can remain alive across
    // `nixos-rebuild switch`; treating the bundle as process-lifetime
    // immutable made already-running brokers dispatch stale runner intents
    // after a switch. Per-request reload keeps broker authority aligned with
    // the current on-disk bundle while preserving fail-closed tamper
    // handling.
    #[cfg(not(feature = "layer1-bootstrap"))]
    let (resolver, bundle_tamper) = match try_load_resolver(&config.bundle_path) {
        BundleSlot::Loaded(resolver) => (Some(resolver), None),
        BundleSlot::Unavailable => (None, None),
        BundleSlot::Tampered { path, reason } => (None, Some((path, reason))),
    };
    let request = envelope.request;
    let effective_uid = if config.test_mode {
        envelope.test_peer_uid.unwrap_or(peer_uid)
    } else {
        peer_uid
    };
    let operation = request.op_name();
    let opaque_target_id = request.opaque_target_id();
    #[cfg(not(feature = "layer1-bootstrap"))]
    if !config.profile.allows_request(&request) {
        let error = BrokerError::ProfileOperationRefused {
            profile: config.profile,
            operation,
        };
        let _ = write_refusal_audit_bounded(
            audit_log,
            AuditWriteClass::Privileged,
            operation,
            peer_uid,
            peer_gid,
            "profile-operation-denied",
            opaque_target_id,
            "closed",
        );
        return Ok(RequestOutcome::Reply(error.into_response(), Vec::new()));
    }
    let (rate_role, rate_operation) = if effective_uid == config.d2bd_uid {
        (envelope.caller_role.for_display(), operation)
    } else {
        ("direct-broker-peer", "direct-broker-connect")
    };
    let rate_pool = if effective_uid == config.d2bd_uid {
        IpcRatePool::Daemon
    } else {
        IpcRatePool::Direct
    };
    // The limiter is a `tokio::sync::Mutex` (plan U8); this check runs on
    // the synchronous dispatch workers, which must never block, so it spins
    // on `try_lock` only for the short bounded `check` critical section
    // (lock_sync pattern - serialize concurrent bursts, never refuse; the
    // pre-conversion std Mutex::lock serialized the check the same way).
    // The guard MUST be dropped before any audit or dispatch work below:
    // holding it across the forward/nested dispatch would deadlock
    // concurrent requests against the daemon leg.
    let rate_allowed = {
        let mut limiter = loop {
            match ipc_rate_limiter.try_lock() {
                Ok(guard) => break guard,
                Err(_) => std::hint::spin_loop(),
            }
        };
        limiter.check(rate_pool, effective_uid, rate_role, rate_operation)
    };
    if !rate_allowed {
        if let Err(error) = write_refusal_audit_bounded(
            audit_log,
            if effective_uid == config.d2bd_uid {
                AuditWriteClass::Privileged
            } else {
                AuditWriteClass::Unprivileged
            },
            operation,
            effective_uid,
            peer_gid,
            "ipc-rate-limited",
            opaque_target_id,
            "closed",
        ) {
            tracing::error!(error = ?error, "broker rate-limit audit failed");
        }
        if effective_uid == config.d2bd_uid {
            return Ok(RequestOutcome::Reply(
                BrokerError::IpcRateLimited.into_response(),
                Vec::new(),
            ));
        }
        return Ok(RequestOutcome::Silence);
    }
    if effective_uid != config.d2bd_uid {
        if let Err(error) = write_refusal_audit_bounded(
            audit_log,
            AuditWriteClass::Unprivileged,
            operation,
            effective_uid,
            peer_gid,
            "peer-refused",
            opaque_target_id,
            "closed",
        ) {
            tracing::error!(error = ?error, "broker peer-refusal audit failed");
        }
        return Ok(RequestOutcome::Reply(
            BrokerError::PeerCredentialRefused { operation }.into_response(),
            Vec::new(),
        ));
    }

    if let Err(error) = validate_broker_request(&request) {
        #[cfg(not(feature = "layer1-bootstrap"))]
        let audit_join = envelope.audit_join.clone();
        #[cfg(feature = "layer1-bootstrap")]
        let audit_join = None;
        let audit_context = DispatchAuditContext {
            peer_pid,
            peer_role: envelope.caller_role.for_display().to_owned(),
            verb: operation.to_owned(),
            request_fields: serde_json::json!({ "validation": "failed" }),
            started_at: Instant::now(),
            audit_join,
        };
        #[cfg(not(feature = "layer1-bootstrap"))]
        if let Err(audit_error) = error.audit(
            audit_log,
            effective_uid,
            peer_gid,
            &envelope.caller_role,
            &audit_context,
            resolver.as_deref(),
            operation,
            opaque_target_id,
        ) {
            tracing::error!(error = ?audit_error, "broker validation error audit failed");
        }
        #[cfg(feature = "layer1-bootstrap")]
        if let Err(audit_error) = error.audit(
            audit_log,
            effective_uid,
            peer_gid,
            &envelope.caller_role,
            &audit_context,
            operation,
            opaque_target_id,
        ) {
            tracing::error!(error = ?audit_error, "broker validation error audit failed");
        }
        return Ok(RequestOutcome::Reply(error.into_response(), Vec::new()));
    }
    #[cfg(not(feature = "layer1-bootstrap"))]
    let audit_join = envelope.audit_join.as_ref();
    #[cfg(feature = "layer1-bootstrap")]
    let audit_join: Option<&AuditJoinContext> = None;
    let audit_context = DispatchAuditContext::from_request_with_join(
        &request,
        peer_pid,
        &envelope.caller_role,
        audit_join,
    )
    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, format!("{err:?}")))?;
    #[cfg(not(feature = "layer1-bootstrap"))]
    let dispatch_outcome = if let Some((path, reason)) = bundle_tamper {
        Err(BrokerError::BundleTampered { path, reason })
    } else {
        dispatch_request_with_request_fds(
            request,
            effective_uid,
            peer_gid,
            envelope.caller_role.clone(),
            &audit_context,
            config,
            audit_log,
            resolver.as_ref(),
            request_fds,
        )
        .await
    };
    #[cfg(feature = "layer1-bootstrap")]
    let dispatch_outcome = dispatch_request(
        request,
        effective_uid,
        peer_gid,
        envelope.caller_role.clone(),
        &audit_context,
        config,
        audit_log,
    )
    .map(DispatchResult::no_fds);

    let (response, fds) = match dispatch_outcome {
        Ok(result) => (result.response, result.fds),
        Err(error) => {
            #[cfg(not(feature = "layer1-bootstrap"))]
            if let Err(audit_error) = error.audit(
                audit_log,
                effective_uid,
                peer_gid,
                &envelope.caller_role,
                &audit_context,
                resolver.as_deref(),
                operation,
                opaque_target_id,
            ) {
                tracing::error!(error = ?audit_error, "broker terminal error audit failed");
            }
            #[cfg(feature = "layer1-bootstrap")]
            if let Err(audit_error) = error.audit(
                audit_log,
                effective_uid,
                peer_gid,
                &envelope.caller_role,
                &audit_context,
                operation,
                opaque_target_id,
            ) {
                tracing::error!(error = ?audit_error, "broker terminal error audit failed");
            }
            (error.into_response(), Vec::new())
        }
    };

    Ok(RequestOutcome::Reply(response, fds))
}
fn write_refusal_audit_bounded(
    audit_log: &AuditLog,
    audit_class: AuditWriteClass,
    operation: &str,
    caller_uid: u32,
    caller_gid: u32,
    disposition: &str,
    opaque_target_id: &str,
    outcome: &str,
) -> io::Result<()> {
    match audit_log.write_entry_with_class_and_caller_ids(
        audit_class,
        operation,
        caller_uid,
        caller_gid,
        disposition,
        opaque_target_id,
        outcome,
    ) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(()),
        Err(err) => Err(err),
    }
}

/// Real-wire dispatch results can carry zero-or-more `OwnedFd`s
/// alongside the JSON response. Bootstrap dispatch never carries fds.
#[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
#[derive(Debug)]
struct DispatchResult {
    response: BrokerResponse,
    fds: Vec<OwnedFd>,
}

#[cfg_attr(feature = "layer1-bootstrap", allow(dead_code))]
impl DispatchResult {
    fn no_fds(response: BrokerResponse) -> Self {
        Self {
            response,
            fds: Vec::new(),
        }
    }

    fn with_fd(response: BrokerResponse, fd: OwnedFd) -> Self {
        Self {
            response,
            fds: vec![fd],
        }
    }

    fn with_fds(response: BrokerResponse, fds: Vec<OwnedFd>) -> Self {
        Self { response, fds }
    }
}

#[derive(Debug)]
struct IpcRateLimiter {
    max_requests_per_window: u32,
    max_buckets_per_pool: usize,
    daemon_buckets: std::collections::HashMap<IpcRateKey, IpcRateBucket>,
    direct_buckets: std::collections::HashMap<IpcRateKey, IpcRateBucket>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IpcRatePool {
    Daemon,
    Direct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct IpcRateKey {
    uid: u32,
    role: &'static str,
    operation: &'static str,
}

#[derive(Debug)]
struct IpcRateBucket {
    window_start: Instant,
    requests_this_window: u32,
}

impl IpcRateLimiter {
    fn new(max_requests_per_window: u32) -> Self {
        Self::with_limits(max_requests_per_window, DEFAULT_IPC_RATE_LIMIT_MAX_BUCKETS)
    }

    fn with_limits(max_requests_per_window: u32, max_buckets: usize) -> Self {
        Self {
            max_requests_per_window,
            max_buckets_per_pool: max_buckets,
            daemon_buckets: std::collections::HashMap::new(),
            direct_buckets: std::collections::HashMap::new(),
        }
    }

    fn check(
        &mut self,
        pool: IpcRatePool,
        uid: u32,
        role: &'static str,
        operation: &'static str,
    ) -> bool {
        self.check_at(pool, uid, role, operation, Instant::now())
    }

    fn check_at(
        &mut self,
        pool: IpcRatePool,
        uid: u32,
        role: &'static str,
        operation: &'static str,
        now: Instant,
    ) -> bool {
        let buckets = match pool {
            IpcRatePool::Daemon => &mut self.daemon_buckets,
            IpcRatePool::Direct => &mut self.direct_buckets,
        };
        Self::check_bucket_map(
            self.max_requests_per_window,
            self.max_buckets_per_pool,
            buckets,
            uid,
            role,
            operation,
            now,
        )
    }

    fn check_bucket_map(
        max_requests_per_window: u32,
        max_buckets: usize,
        buckets: &mut std::collections::HashMap<IpcRateKey, IpcRateBucket>,
        uid: u32,
        role: &'static str,
        operation: &'static str,
        now: Instant,
    ) -> bool {
        if max_requests_per_window == 0 {
            return false;
        }
        let key = IpcRateKey {
            uid,
            role,
            operation,
        };
        if !buckets.contains_key(&key) {
            Self::evict_expired(buckets, now);
            if buckets.len() >= max_buckets {
                return false;
            }
        }
        let bucket = buckets.entry(key).or_insert(IpcRateBucket {
            window_start: now,
            requests_this_window: 0,
        });
        if now.saturating_duration_since(bucket.window_start) >= IPC_RATE_LIMIT_WINDOW {
            bucket.window_start = now;
            bucket.requests_this_window = 0;
        }
        if bucket.requests_this_window >= max_requests_per_window {
            return false;
        }
        bucket.requests_this_window += 1;
        true
    }

    fn evict_expired(
        buckets: &mut std::collections::HashMap<IpcRateKey, IpcRateBucket>,
        now: Instant,
    ) {
        buckets.retain(|_, bucket| {
            now.saturating_duration_since(bucket.window_start) < IPC_RATE_LIMIT_WINDOW
        });
    }
}

#[cfg(feature = "layer1-bootstrap")]
fn validate_broker_request(_request: &BrokerRequest) -> Result<(), BrokerError> {
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn validate_network_authority(scope_id: &str, intent_id: &str) -> Result<(), &'static str> {
    if scope_id.starts_with("env:")
        || intent_id.contains(":env:")
        || intent_id.starts_with("route:env:")
        || intent_id.starts_with("sysctl:env:")
        || intent_id.starts_with("bridge:env:")
        || intent_id.starts_with("nft-projection:env:")
        || intent_id.starts_with("bridge:zone:")
        || intent_id.starts_with("nft-projection:zone:")
        || intent_id.starts_with("route:zone:")
        || intent_id.starts_with("sysctl:zone:")
        || intent_id.starts_with("hosts:zone:")
    {
        return Err("legacy-network-authority");
    }
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn validate_uid_network_authority(
    scope_id: &str,
    intent_id: &str,
    zone_uid: &d2b_contracts_resource::v3::ResourceUid,
    network_uid: &d2b_contracts_resource::v3::ResourceUid,
    network_generation: d2b_contracts_resource::v3::ResourceGeneration,
    attachment_generation: d2b_contracts_resource::v3::ResourceGeneration,
    bundle_generation: &d2b_contracts_resource::v3::ResourceBundleGenerationId,
) -> Result<(), &'static str> {
    let expected_scope = format!("network:{}:{}", zone_uid.as_str(), network_uid.as_str());
    if scope_id != expected_scope {
        return Err("network-scope-mismatch");
    }
    let fields = intent_id.split(':').collect::<Vec<_>>();
    let is_network_intent = matches!(
        fields.first().copied(),
        Some("network-bridge")
            | Some("network-firewall")
            | Some("network-hosts")
            | Some("network-route")
            | Some("network-sysctl")
            | Some("network-marker")
    );
    if !is_network_intent
        || fields.get(1).and_then(|value| {
            d2b_contracts_resource::v3::ResourceUid::parse((*value).to_owned()).ok()
        }) != Some(zone_uid.clone())
        || fields.get(2).and_then(|value| {
            d2b_contracts_resource::v3::ResourceUid::parse((*value).to_owned()).ok()
        }) != Some(network_uid.clone())
    {
        return Err("network-admission-mismatch");
    }
    if fields.get(3).is_none_or(|value| {
        value.len() != 16 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        return Err("network-admission-mismatch");
    }
    if network_generation.get() == 0
        || attachment_generation.get() == 0
        || bundle_generation.as_str().is_empty()
    {
        return Err("network-admission-mismatch");
    }
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn validate_network_scope_provenance(
    scope_id: &str,
    zone_uid: &d2b_contracts_resource::v3::ResourceUid,
    network_uid: &d2b_contracts_resource::v3::ResourceUid,
    network_generation: d2b_contracts_resource::v3::ResourceGeneration,
    attachment_generation: d2b_contracts_resource::v3::ResourceGeneration,
    bundle_generation: &d2b_contracts_resource::v3::ResourceBundleGenerationId,
) -> Result<(), &'static str> {
    let expected_scope = format!("network:{}:{}", zone_uid.as_str(), network_uid.as_str());
    if scope_id != expected_scope
        || network_generation.get() == 0
        || attachment_generation.get() == 0
        || bundle_generation.as_str().is_empty()
    {
        return Err("network-admission-mismatch");
    }
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn validate_tap_create_provenance(
    bundle_tap_intent_ref: &d2b_contracts::types::BundleOpId,
    vm_id: &d2b_contracts::types::VmId,
    role_id: &d2b_contracts::types::RoleId,
    attachment_id: &d2b_contracts_resource::v3::ResourceUid,
    zone_uid: &d2b_contracts_resource::v3::ResourceUid,
    network_uid: &d2b_contracts_resource::v3::ResourceUid,
    network_generation: d2b_contracts_resource::v3::ResourceGeneration,
    attachment_generation: d2b_contracts_resource::v3::ResourceGeneration,
    bundle_generation: &d2b_contracts_resource::v3::ResourceBundleGenerationId,
    admitted_interface_names: &[d2b_contracts_resource::v3::IfName],
) -> Result<(), &'static str> {
    validate_bundle_op_id(bundle_tap_intent_ref.as_str())?;
    if vm_id.as_str().is_empty()
        || network_generation.get() == 0
        || attachment_generation.get() == 0
        || bundle_generation.as_str().is_empty()
    {
        return Err("network-admission-mismatch");
    }
    let canonical_role_id = d2b_core::bundle_resolver::canonical_tap_role_id(role_id.as_str());
    if !matches!(
        canonical_role_id,
        "ch" | "qemu-media"
            | "net-vm-lan"
            | "uplink"
            | "workload-lan"
            | "network-attachment"
            | "runner-lan"
    ) {
        return Err("network-admission-mismatch");
    }
    let expected = d2b_core::bundle_resolver::intent_id_network_tap(
        zone_uid,
        network_uid,
        attachment_id,
        (network_generation, attachment_generation),
        bundle_generation,
        canonical_role_id,
        vm_id.as_str(),
    );
    if bundle_tap_intent_ref.as_str() == expected {
        let (bridge_role, tap_role, tap_attachment) = if vm_id.as_str()
            == d2b_contracts_resource::v3::derive_network_child_name(network_uid, "vm")
        {
            (
                d2b_contracts_resource::v3::NetworkIfRole::LanBridge,
                d2b_contracts_resource::v3::NetworkIfRole::NetVmLanTap,
                None,
            )
        } else {
            match canonical_role_id {
                "net-vm-lan" => (
                    d2b_contracts_resource::v3::NetworkIfRole::LanBridge,
                    d2b_contracts_resource::v3::NetworkIfRole::NetVmLanTap,
                    None,
                ),
                "uplink" => (
                    d2b_contracts_resource::v3::NetworkIfRole::UplinkBridge,
                    d2b_contracts_resource::v3::NetworkIfRole::NetVmUplinkTap,
                    None,
                ),
                "ch" | "qemu-media" | "workload-lan" | "network-attachment" | "runner-lan" => (
                    d2b_contracts_resource::v3::NetworkIfRole::LanBridge,
                    d2b_contracts_resource::v3::NetworkIfRole::WorkloadGuestTap,
                    Some(attachment_id),
                ),
                _ => return Err("network-admission-mismatch"),
            }
        };
        let bridge = d2b_contracts_resource::v3::derive_network_ifname(
            zone_uid,
            network_uid,
            bridge_role,
            None,
        )
        .map_err(|_| "network-admission-mismatch")?;
        let tap = d2b_contracts_resource::v3::derive_network_ifname(
            zone_uid,
            network_uid,
            tap_role,
            tap_attachment,
        )
        .map_err(|_| "network-admission-mismatch")?;
        if admitted_interface_names
            .iter()
            .any(|ifname| ifname == &bridge)
            && admitted_interface_names.iter().any(|ifname| ifname == &tap)
        {
            Ok(())
        } else {
            Err("network-admission-mismatch")
        }
    } else {
        Err("network-admission-mismatch")
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn network_provenance(
    zone_uid: d2b_contracts_resource::v3::ResourceUid,
    network_uid: d2b_contracts_resource::v3::ResourceUid,
    network_generation: d2b_contracts_resource::v3::ResourceGeneration,
    attachment_generation: d2b_contracts_resource::v3::ResourceGeneration,
    bundle_generation: d2b_contracts_resource::v3::ResourceBundleGenerationId,
) -> d2b_contracts_resource::v3::NetworkProvenance {
    d2b_contracts_resource::v3::NetworkProvenance::new(
        zone_uid,
        network_uid,
        network_generation,
        attachment_generation,
        bundle_generation,
    )
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn require_installed_network_generation(
    resolver: &BundleResolver,
    provenance: &d2b_contracts_resource::v3::NetworkProvenance,
) -> Result<(), BrokerError> {
    let installed =
        resolver
            .installed_generation_identity()
            .ok_or(BrokerError::RequestValidation {
                operation: "NetworkEffect",
                reason: "installed-generation-unavailable",
            })?;
    if installed.as_str() != provenance.bundle_generation().as_str() {
        return Err(BrokerError::RequestValidation {
            operation: "NetworkEffect",
            reason: "stale-projection-generation",
        });
    }
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn validate_broker_request(request: &BrokerRequest) -> Result<(), BrokerError> {
    // Shape-only defense in depth after d2bd has accepted and classified
    // the local peer. Do not add role/bundle authorization here: dispatch must
    // continue to resolve opaque ids through the trusted bundle, with d2bd
    // owning lifecycle authz classification.
    match request {
        BrokerRequest::ModprobeIfAllowed(req) => {
            validate_module_name(&req.module_name).map_err(|reason| {
                BrokerError::RequestValidation {
                    operation: "ModprobeIfAllowed",
                    reason,
                }
            })
        }
        BrokerRequest::UsbipBind(req) => {
            validate_bundle_op_id(req.bundle_usbip_bind_intent_ref.as_str()).map_err(|reason| {
                BrokerError::RequestValidation {
                    operation: "UsbipBind",
                    reason,
                }
            })
        }
        BrokerRequest::UsbipUnbind(req) => {
            validate_bundle_op_id(req.bundle_usbip_bind_intent_ref.as_str()).map_err(|reason| {
                BrokerError::RequestValidation {
                    operation: "UsbipUnbind",
                    reason,
                }
            })
        }
        BrokerRequest::UsbipProxyReconcile(req) => validate_scope_like_id(req.scope_id.as_str())
            .map_err(|reason| BrokerError::RequestValidation {
                operation: "UsbipProxyReconcile",
                reason,
            }),
        BrokerRequest::UsbipBindFirewallRule(req) => {
            validate_bundle_op_id(req.bundle_usbip_firewall_intent_ref.as_str()).map_err(|reason| {
                BrokerError::RequestValidation {
                    operation: "UsbipBindFirewallRule",
                    reason,
                }
            })
        }
        BrokerRequest::UsbipExplicitBind(req) => {
            validate_usbip_busid_wire(&req.bus_id).map_err(|reason| {
                BrokerError::RequestValidation {
                    operation: "UsbipExplicitBind",
                    reason,
                }
            })
        }
        BrokerRequest::UsbipExplicitFirewallRule(req) => validate_usbip_busid_wire(&req.bus_id)
            .map_err(|reason| BrokerError::RequestValidation {
                operation: "UsbipExplicitFirewallRule",
                reason,
            }),
        BrokerRequest::PipeWireAudio(req) => {
            validate_small_wire_id(req.vm_id.as_str(), 128, "invalid-vm-id").map_err(|reason| {
                BrokerError::RequestValidation {
                    operation: "PipeWireAudio",
                    reason,
                }
            })?;
            validate_small_wire_id(req.role_id.as_str(), 128, "invalid-role-id").map_err(
                |reason| BrokerError::RequestValidation {
                    operation: "PipeWireAudio",
                    reason,
                },
            )?;
            validate_bundle_op_id(req.bundle_runner_intent_ref.as_str()).map_err(|reason| {
                BrokerError::RequestValidation {
                    operation: "PipeWireAudio",
                    reason,
                }
            })?;
            if let d2b_contracts_broker::broker_wire::PipeWireAudioAction::SetLevel { percent } =
                req.action
                && percent > 100
            {
                return Err(BrokerError::RequestValidation {
                    operation: "PipeWireAudio",
                    reason: "audio-level-out-of-range",
                });
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn validate_usbip_busid_wire(bus_id: &str) -> Result<(), &'static str> {
    d2b_contracts::usbip::validate_bus_id(bus_id).map_err(|_| "invalid-usbip-busid")
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn validate_module_name(module_name: &str) -> Result<(), &'static str> {
    if module_name.is_empty() {
        return Err("empty-module-name");
    }
    if module_name.len() > MAX_MODULE_NAME_LEN {
        return Err("module-name-too-long");
    }
    if module_name.contains('/') || module_name.contains('\\') || module_name.contains('\0') {
        return Err("invalid-module-name");
    }
    if module_name == "." || module_name == ".." || module_name.contains("..") {
        return Err("invalid-module-name");
    }
    if !module_name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err("invalid-module-name");
    }
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn validate_scope_like_id(value: &str) -> Result<(), &'static str> {
    validate_small_wire_id(value, 128, "invalid-scope-id")
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn validate_bundle_op_id(value: &str) -> Result<(), &'static str> {
    validate_small_wire_id(value, 192, "invalid-bundle-op-id")
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn validate_small_wire_id(
    value: &str,
    max_len: usize,
    error: &'static str,
) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > max_len {
        return Err(error);
    }
    if value.contains('/') || value.contains('\\') || value.contains('\0') || value.contains("..") {
        return Err(error);
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-' | b'_' | b'.'))
    {
        return Err(error);
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct DispatchAuditContext {
    peer_pid: i32,
    peer_role: String,
    verb: String,
    request_fields: Value,
    started_at: Instant,
    audit_join: Option<AuditJoinContext>,
}

impl DispatchAuditContext {
    fn from_request(
        request: &BrokerRequest,
        peer_pid: i32,
        caller_role: &CallerRole,
    ) -> Result<Self, BrokerError> {
        #[cfg(feature = "layer1-bootstrap")]
        {
            Self::from_request_with_join(request, peer_pid, caller_role, None)
        }
        #[cfg(not(feature = "layer1-bootstrap"))]
        {
            let join = match request.authoritative_audit_join() {
                Some((zone_id, operation_identity)) => Some(AuditJoinContext {
                    zone_id: CanonicalAuditDigest::parse(zone_id)
                        .map_err(|_| BrokerError::Protocol("audit zone identity invalid".to_owned()))?,
                    operation_identity: CanonicalAuditDigest::parse(operation_identity)
                        .map_err(|_| {
                            BrokerError::Protocol("audit operation identity invalid".to_owned())
                        })?,
                }),
                None => None,
            };
            Self::from_request_with_join(request, peer_pid, caller_role, join.as_ref())
        }
    }

    fn from_request_with_join(
        request: &BrokerRequest,
        peer_pid: i32,
        caller_role: &CallerRole,
        audit_join: Option<&AuditJoinContext>,
    ) -> Result<Self, BrokerError> {
        #[cfg(not(feature = "layer1-bootstrap"))]
        if audit_join.is_none() && Self::request_requires_audit_join(request) {
            return Err(BrokerError::Protocol("audit-join-required".to_owned()));
        }
        #[cfg(not(feature = "layer1-bootstrap"))]
        if let Some(supplied) = audit_join
            && let Some((zone_id, operation_identity)) = request.authoritative_audit_join()
        {
            let expected_zone = CanonicalAuditDigest::parse(zone_id)
                .map_err(|_| BrokerError::Protocol("audit zone identity invalid".to_owned()))?;
            let expected_operation =
                CanonicalAuditDigest::parse(operation_identity).map_err(|_| {
                    BrokerError::Protocol("audit operation identity invalid".to_owned())
                })?;
            if supplied.zone_id != expected_zone
                || supplied.operation_identity != expected_operation
            {
                return Err(BrokerError::Protocol("audit-join-mismatch".to_owned()));
            }
        }
        Ok(Self {
            peer_pid,
            peer_role: caller_role.for_display().to_owned(),
            verb: request.op_name().to_owned(),
            request_fields: request_fields_value(request)?,
            started_at: Instant::now(),
            audit_join: audit_join.cloned(),
        })
    }

    fn duration_us(&self) -> u64 {
        self.started_at.elapsed().as_micros() as u64
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    fn request_requires_audit_join(request: &BrokerRequest) -> bool {
        request.requires_authoritative_audit_join()
    }
}

fn request_fields_value(request: &BrokerRequest) -> Result<Value, BrokerError> {
    #[cfg(not(feature = "layer1-bootstrap"))]
    if let BrokerRequest::QemuMediaEnroll(req) = request {
        return Ok(serde_json::json!({
            "vmId": req.vm_id.as_str(),
            "mediaRef": req.media_ref.as_str(),
            "busIdProvided": true,
            "tracingSpanIdPresent": req.tracing_span_id.is_some(),
        }));
    }
    #[cfg(not(feature = "layer1-bootstrap"))]
    if let BrokerRequest::QemuMediaRefreshRegistry(req) = request {
        return Ok(serde_json::json!({
            "tracingSpanIdPresent": req.tracing_span_id.is_some(),
        }));
    }
    #[cfg(not(feature = "layer1-bootstrap"))]
    if let BrokerRequest::QemuMediaBoot(req) = request {
        return Ok(serde_json::json!({
            "vmId": req.vm_id.as_str(),
            "tracingSpanIdPresent": req.tracing_span_id.is_some(),
        }));
    }
    #[cfg(not(feature = "layer1-bootstrap"))]
    if let BrokerRequest::QemuMediaSystemPowerdown(req) | BrokerRequest::QemuMediaQuit(req) =
        request
    {
        return Ok(serde_json::json!({
            "vmId": req.vm_id.as_str(),
            "tracingSpanIdPresent": req.tracing_span_id.is_some(),
        }));
    }
    #[cfg(not(feature = "layer1-bootstrap"))]
    if let BrokerRequest::QemuMediaQueryStatus(req) = request {
        return Ok(serde_json::json!({
            "vmId": req.vm_id.as_str(),
            "shutdownContext": req.shutdown_context,
            "tracingSpanIdPresent": req.tracing_span_id.is_some(),
        }));
    }
    #[cfg(not(feature = "layer1-bootstrap"))]
    if let BrokerRequest::QemuMediaAttach(req) | BrokerRequest::QemuMediaDetach(req) = request {
        return Ok(serde_json::json!({
            "vmId": req.vm_id.as_str(),
            "busIdProvided": true,
            "tracingSpanIdPresent": req.tracing_span_id.is_some(),
        }));
    }
    #[cfg(not(feature = "layer1-bootstrap"))]
    if let BrokerRequest::UsbipBind(req) = request {
        return Ok(serde_json::json!({
            "bundleUsbipBindIntentRef": req.bundle_usbip_bind_intent_ref.as_str(),
            "tracingSpanIdPresent": req.tracing_span_id.is_some(),
        }));
    }
    #[cfg(not(feature = "layer1-bootstrap"))]
    if let BrokerRequest::UsbipUnbind(req) = request {
        return Ok(serde_json::json!({
            "bundleUsbipBindIntentRef": req.bundle_usbip_bind_intent_ref.as_str(),
            "preserveDurableClaim": req.preserve_durable_claim,
            "tracingSpanIdPresent": req.tracing_span_id.is_some(),
        }));
    }
    #[cfg(not(feature = "layer1-bootstrap"))]
    if let BrokerRequest::UsbipBindFirewallRule(req) = request {
        return Ok(serde_json::json!({
            "bundleUsbipFirewallIntentRef": req.bundle_usbip_firewall_intent_ref.as_str(),
            "tracingSpanIdPresent": req.tracing_span_id.is_some(),
        }));
    }
    #[cfg(not(feature = "layer1-bootstrap"))]
    if let BrokerRequest::UsbipProxyReconcile(req) = request {
        return Ok(serde_json::json!({
            "scopeId": req.scope_id.as_str(),
            "tracingSpanIdPresent": req.tracing_span_id.is_some(),
        }));
    }
    let mut value = serde_json::to_value(request)
        .map_err(|err| BrokerError::Protocol(format!("serialize request fields: {err}")))?;
    match &mut value {
        Value::Object(map) => {
            if let Some(payload) = map.remove("payload") {
                Ok(payload)
            } else {
                map.remove("kind");
                map.remove("request");
                Ok(value)
            }
        }
        _ => Ok(value),
    }
}

#[cfg(feature = "layer1-bootstrap")]
fn dispatch_request(
    request: BrokerRequest,
    caller_uid: u32,
    caller_gid: u32,
    caller_role: CallerRole,
    _audit_context: &DispatchAuditContext,
    _config: &ServerConfig,
    audit_log: &AuditLog,
) -> Result<BrokerResponse, BrokerError> {
    match request {
        BrokerRequest::Hello { .. } => {
            audit_log
                .write_entry_with_caller_ids(
                    "Hello",
                    caller_uid,
                    caller_gid,
                    "callable-read-only",
                    "daemon-handshake",
                    "ok",
                )
                .map_err(|err| BrokerError::Protocol(err.to_string()))?;
            Ok(hello_ok_response(_config.profile))
        }
        BrokerRequest::ExportBrokerAudit { since, filter } => handle_export_broker_audit(
            since.as_deref(),
            filter.as_deref(),
            caller_uid,
            caller_gid,
            caller_role,
            audit_log,
        ),
        BrokerRequest::ApplyNftables { .. } => Err(BrokerError::Unimplemented {
            operation: "ApplyNftables",
            target_wave: "W3",
        }),
        BrokerRequest::ApplyNmUnmanaged { .. } => Err(BrokerError::Unimplemented {
            operation: "ApplyNmUnmanaged",
            target_wave: "W3",
        }),
        BrokerRequest::ApplyRoute { .. } => Err(BrokerError::Unimplemented {
            operation: "ApplyRoute",
            target_wave: "W3",
        }),
        BrokerRequest::ApplySysctl { .. } => Err(BrokerError::Unimplemented {
            operation: "ApplySysctl",
            target_wave: "W3",
        }),
        BrokerRequest::CreateOrReconcileUsersGroups { .. } => Err(BrokerError::Unimplemented {
            operation: "CreateOrReconcileUsersGroups",
            target_wave: "W3",
        }),
        BrokerRequest::CreatePersistentTap { .. } => Err(BrokerError::Unimplemented {
            operation: "CreatePersistentTap",
            target_wave: "W3",
        }),
        BrokerRequest::CreateTapFd { .. } => Err(BrokerError::Unimplemented {
            operation: "CreateTapFd",
            target_wave: "W3",
        }),
        BrokerRequest::DelegateCgroupV2 { .. } => Err(BrokerError::Unimplemented {
            operation: "DelegateCgroupV2",
            target_wave: "W3",
        }),
        BrokerRequest::InjectSecretById { .. } => Err(BrokerError::Unimplemented {
            operation: "InjectSecretById",
            target_wave: "W8",
        }),
        BrokerRequest::LaunchMinijailChild { .. } => Err(BrokerError::Unimplemented {
            operation: "LaunchMinijailChild",
            target_wave: "W5",
        }),
        BrokerRequest::ModprobeIfAllowed { .. } => Err(BrokerError::Unimplemented {
            operation: "ModprobeIfAllowed",
            target_wave: "W3",
        }),
        BrokerRequest::OpenCgroupDir { .. } => Err(BrokerError::Unimplemented {
            operation: "OpenCgroupDir",
            target_wave: "W3",
        }),
        BrokerRequest::OpenDevice { .. } => Err(BrokerError::Unimplemented {
            operation: "OpenDevice",
            target_wave: "W3",
        }),
        BrokerRequest::OpenFuse { .. } => Err(BrokerError::Unimplemented {
            operation: "OpenFuse",
            target_wave: "W3",
        }),
        BrokerRequest::OpenKvm { .. } => Err(BrokerError::Unimplemented {
            operation: "OpenKvm",
            target_wave: "W3",
        }),
        BrokerRequest::OpenPidfd { .. } => Err(BrokerError::Unimplemented {
            operation: "OpenPidfd",
            target_wave: "W4-fu",
        }),
        BrokerRequest::OpenVhostNet { .. } => Err(BrokerError::Unimplemented {
            operation: "OpenVhostNet",
            target_wave: "W3",
        }),
        BrokerRequest::PrepareRuntimeDir { .. } => Err(BrokerError::Unimplemented {
            operation: "PrepareRuntimeDir",
            target_wave: "W3",
        }),
        BrokerRequest::PrepareStateDir { .. } => Err(BrokerError::Unimplemented {
            operation: "PrepareStateDir",
            target_wave: "W3",
        }),
        BrokerRequest::StoreSync { .. } => Err(BrokerError::Unimplemented {
            operation: "StoreSync",
            target_wave: "P2",
        }),
        BrokerRequest::ReadSecretById { .. } => Err(BrokerError::Unimplemented {
            operation: "ReadSecretById",
            target_wave: "W8",
        }),
        BrokerRequest::RotateSecretById { .. } => Err(BrokerError::Unimplemented {
            operation: "RotateSecretById",
            target_wave: "W8",
        }),
        BrokerRequest::SetBridgePortFlags { .. } => Err(BrokerError::Unimplemented {
            operation: "SetBridgePortFlags",
            target_wave: "W3",
        }),
        BrokerRequest::SpawnRunner { .. } => Err(BrokerError::Unimplemented {
            operation: "SpawnRunner",
            target_wave: "W4-fu",
        }),
        BrokerRequest::UpdateHostsFile { .. } => Err(BrokerError::Unimplemented {
            operation: "UpdateHostsFile",
            target_wave: "W3",
        }),
        BrokerRequest::UsbipBind { .. } => Err(BrokerError::UnknownOperation {
            operation: "UsbipBind",
        }),
        BrokerRequest::UsbipBindFirewallRule { .. } => Err(BrokerError::Unimplemented {
            operation: "UsbipBindFirewallRule",
            target_wave: "W3",
        }),
        BrokerRequest::UsbipExplicitBind { .. } => Err(BrokerError::Unimplemented {
            operation: "UsbipExplicitBind",
            target_wave: "W5",
        }),
        BrokerRequest::UsbipExplicitFirewallRule { .. } => Err(BrokerError::Unimplemented {
            operation: "UsbipExplicitFirewallRule",
            target_wave: "W5",
        }),
        BrokerRequest::UsbipProxyReconcile { .. } => Err(BrokerError::UnknownOperation {
            operation: "UsbipProxyReconcile",
        }),
        BrokerRequest::UsbipUnbind { .. } => Err(BrokerError::UnknownOperation {
            operation: "UsbipUnbind",
        }),
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn request_accepts_fd(request: &BrokerRequest) -> bool {
    matches!(request, BrokerRequest::EnvelopeInvoke(_))
}

/// Whether the trusted intent is the signed binding-owned serving-worker
/// template.
///
/// This is the ONE site in the broker that spells the raw condition
/// (`virtiofsd-worker` under `Provider/volume-virtiofs`).
/// [`LaunchPosture::resolve`] turns it into the carried posture for the
/// launch path; the adoption path (`OpenPidfd`, whose wire request carries no
/// runner role) reads this same evaluation directly, so a posture-dependent
/// decision cannot be omitted at one site while the others agree.
///
/// The resolver mints the same template from its own spelling of the
/// predicate (`is_serving_worker_template` in `d2b_core::bundle_resolver`,
/// where the shape also selects the ADR-0021 user namespace), and the daemon
/// classifies a binding-owned worker from that same declared template under
/// its `VolumeBinding` owner (`resolve_launch_identity` in
/// `d2bd::process_resource_runtime`, keyed on
/// `d2b_provider_volume_virtiofs::WORKER_TEMPLATE`): those are separate,
/// cross-crate decisions the broker only *reads*, and the broker must never
/// re-derive them from request fields.
#[cfg(not(feature = "layer1-bootstrap"))]
fn intent_is_serving_worker_template(
    intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent,
) -> bool {
    intent.role == d2b_core::processes::ProcessRole::ProviderController
        && intent.profile_id == "virtiofsd-worker"
        && intent.owner_ref.as_deref() == Some("Provider/volume-virtiofs")
}

/// The launch posture of one runner request, resolved exactly once from the
/// trusted intent.
///
/// SINGLE EVALUATION POINT for the serving-worker / controller posture: every
/// posture-dependent decision in the launch path reads this value - the
/// inherited-descriptor contract ([`Self::validate_request_fds`]), the
/// controller-bootstrap escrow custody
/// ([`Self::carries_controller_escrow`]) and the advertised response fd
/// indices ([`Self::bootstrap_response_index`] /
/// [`Self::console_response_index`]) - instead of re-deriving it from
/// `(req.role, intent)`.
///
/// The posture deliberately decides NO identity: a launch's executor uid/gid
/// and in-namespace root mapping are always the trusted intent's principal
/// ([`prepare_runner_launch_identity`]). The broker's only authentication factor is
/// peer identity - `peer_matches_instance` admits exactly the daemon's
/// uid/gid on the privileged socket - so a posture that handed any runner the
/// daemon identity would hand it the whole daemon API (the pre-PR security
/// review's finding against the binding-owned serving worker).
///
/// The serving-worker axis is a property of the RESOLVED trusted intent (the
/// signed `virtiofsd-worker` profile under `Provider/volume-virtiofs`), never
/// of caller-supplied request fields: `SpawnRunnerRequest::owner_ref` is only
/// a fence value checked against the intent. The request only selects the
/// controller-role contract, which the role fence checks against the trusted
/// intent.
#[cfg(not(feature = "layer1-bootstrap"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchPosture {
    /// Ordinary runner launch: no controller escrow descriptor.
    Standard,
    /// Non-serving `ProviderController` launch: must carry the controller
    /// bootstrap escrow descriptor (1-2 inherited fds, one extra attached).
    /// The broker retains the escrow as registry custody and returns no
    /// duplicate on the response (a dup of the caller's own descriptor is
    /// refused by the forward carrier's anti-replay fence; the daemon
    /// keeps its own copy of the daemon end to wait on).
    ControllerEscrow,
    /// Binding-owned serving worker: carries no broker escrow descriptor
    /// (zero-fd exemption) and advertises no bootstrap index. It runs as the
    /// trusted intent's principal like every other posture; the two path
    /// trees its launch ticket names are opened to that principal with
    /// per-runner ACLs
    /// ([`crate::live_handlers::grant_serving_worker_launch_acls`]).
    ServingWorker,
}

#[cfg(not(feature = "layer1-bootstrap"))]
impl LaunchPosture {
    /// Resolve the posture ONCE, from the trusted intent, at the point the
    /// dispatch arm resolves that intent.
    fn resolve(role: RunnerRole, intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent) -> Self {
        match (role, intent_is_serving_worker_template(intent)) {
            (RunnerRole::ProviderController, true) => Self::ServingWorker,
            (RunnerRole::ProviderController, false) => Self::ControllerEscrow,
            _ => Self::Standard,
        }
    }

    fn is_provider_controller(self) -> bool {
        !matches!(self, Self::Standard)
    }

    fn is_serving_worker(self) -> bool {
        matches!(self, Self::ServingWorker)
    }

    /// Whether this launch owns the controller bootstrap escrow descriptor:
    /// the backend retains the attached descriptor in the
    /// `controller_bootstrap_registry` custody and returns no duplicate on
    /// the response.
    fn carries_controller_escrow(self) -> bool {
        matches!(self, Self::ControllerEscrow)
    }

    /// The inherited-descriptor contract this posture admits.
    ///
    /// Only [`Self::ControllerEscrow`] admits the controller-escrow fd shape
    /// (1-2 inherited descriptors, one extra attached for the bootstrap
    /// duplicate). The binding-owned serving worker carries no inherited
    /// descriptor at all - the supervisor refuses one for its ticket
    /// (`binding_worker_launch`) - and because
    /// [`Self::carries_controller_escrow`] is false for it, an admitted
    /// escrow-shaped fd would be neither retained as custody nor returned as
    /// a duplicate: the backend pushes every supplied descriptor onto
    /// `pre_opened_device_fds`, so a caller-chosen descriptor would land on
    /// the well-known Provider bootstrap fd slot (10) inside the worker with
    /// no custody and no copy. The ServingWorker arm therefore admits only
    /// `(0, 0)`, enforcing the contract the enum documents.
    fn validate_request_fds(
        self,
        inherited_fd_count: u16,
        attached_fd_count: usize,
    ) -> Result<(), BrokerError> {
        const MAX_REQUEST_INHERITED_FDS: u16 = 256;
        if inherited_fd_count > MAX_REQUEST_INHERITED_FDS {
            return Err(BrokerError::Protocol(
                "SpawnRunner inherited fd count exceeds the bounded maximum".to_owned(),
            ));
        }
        match self {
            // A binding-owned serving worker is a controller-role launch with
            // a VolumeBinding semantic owner; it carries no broker escrow
            // descriptors. The full intent fences still apply.
            Self::ServingWorker if inherited_fd_count == 0 && attached_fd_count == 0 => Ok(()),
            Self::ServingWorker => Err(BrokerError::Protocol(
                "binding-owned serving worker must carry no inherited fds".to_owned(),
            )),
            Self::ControllerEscrow
                if (1..=2).contains(&inherited_fd_count)
                    && attached_fd_count == usize::from(inherited_fd_count) + 1 =>
            {
                Ok(())
            }
            Self::ControllerEscrow => Err(BrokerError::Protocol(
                "ProviderController requires one or two inherited fds".to_owned(),
            )),
            Self::Standard if inherited_fd_count == 0 && attached_fd_count == 0 => Ok(()),
            Self::Standard => Err(BrokerError::Protocol(
                "runner inherited fd count/attachment mismatch".to_owned(),
            )),
        }
    }

    /// Index of the controller-bootstrap escrow fd on a `SpawnRunner`
    /// response.
    ///
    /// Always `None`: the escrow descriptor is retained as registry
    /// custody and no duplicate rides the response (a dup of the caller's
    /// own descriptor is refused by the forward carrier's anti-replay
    /// fence with the fd-leg code). The daemon keeps its own copy of the
    /// daemon end to wait on, so the response fd vector holds the pidfd
    /// alone for every launch.
    fn bootstrap_response_index(self) -> Option<u32> {
        None
    }

    /// Index of the console-socket descriptor on a `SpawnRunner` response.
    /// Controller launches carry no console descriptor.
    fn console_response_index(self, extra_response_fds: usize) -> Option<u32> {
        if extra_response_fds == 0 || self.is_provider_controller() {
            None
        } else {
            Some(1)
        }
    }
}

/// The executor identity one runner launch runs with: uid, gid, and the
/// user-namespace mapping the spawn plan installs (ADR 0021), plus the
/// per-runner path access the launch posture requires.
///
/// The identity is ALWAYS the trusted intent's principal, for every posture.
/// This is the single evaluation point for runner identity, and it
/// deliberately takes no identity from the posture: the broker's only
/// authentication factor is peer identity - `peer_matches_instance` admits
/// exactly `config.d2bd_uid` / `config.d2bd_gid` on the privileged socket,
/// whose mode is `0660 d2bd:d2bd` - so a runner launched with the daemon's
/// uid/gid could connect that socket, pass the pre-decode peer check, and use
/// the whole daemon API (the pre-PR security review's finding: the
/// binding-owned serving worker was launched with the daemon identity and its
/// in-namespace root mapped to it).
///
/// The posture decides only *access*, never identity: a binding-owned serving
/// worker runs on daemon-provisioned paths, so its two ticket-named trees are
/// opened to its own principal with per-runner ACLs
/// ([`crate::live_handlers::grant_serving_worker_launch_acls`]) instead of
/// borrowing the daemon's credentials.
#[cfg(not(feature = "layer1-bootstrap"))]
fn prepare_runner_launch_identity(
    posture: LaunchPosture,
    config: &ServerConfig,
    intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent,
    argv: &[String],
) -> Result<
    (
        u32,
        u32,
        Option<crate::ops::spawn_runner::UserNamespaceSpec>,
    ),
    BrokerError,
> {
    let (uid, gid, user_namespace) = (
        intent.uid,
        intent.gid,
        intent
            .user_namespace
            .map(|spec| crate::ops::spawn_runner::UserNamespaceSpec {
                host_uid_for_zero: spec.host_uid_for_zero,
                host_gid_for_zero: spec.host_gid_for_zero,
            }),
    );
    let runtime_root = config
        .socket_path
        .parent()
        .unwrap_or_else(|| Path::new(DEFAULT_BROKER_RUNTIME_DIR));
    if posture.is_serving_worker() {
        crate::live_handlers::grant_serving_worker_launch_acls(argv, uid, runtime_root)
            .map_err(|error| BrokerError::LiveHandler(error.to_string()))?;
    }
    // Device-owned worker launches (the declared swtpm/GPU rows) also bind
    // their per-VM socket inside the broker runtime root, which the daemon
    // owns and which carries no grant for the launched principal. That grant
    // is NOT applied here: it is derived from the pinned owning Device (never
    // from the launch arguments) and applied inside `live_spawn_runner` after
    // the launch's typed fences pass, so a refused launch never leaves an ACL
    // behind (`live_handlers::DeviceWorkerSocketGrant`).
    Ok((uid, gid, user_namespace))
}

/// Real-wire dispatch. Matches the opaque-ID
/// `d2b_contracts_broker::broker_wire::BrokerRequest` tuple-newtype shape and
/// wires the live executors into the dispatch arms that have a ready
/// implementation today.
///
/// This signature takes an `Option<&Arc<BundleResolver>>` and returns
/// `DispatchResult` (response + optional fds) so the bundle-dependent
/// arms can route through `BundleResolver::find_*_intent` and
/// `live_handlers::*`, transporting fds via SCM_RIGHTS on the response
/// frame.
#[cfg(not(feature = "layer1-bootstrap"))]
async fn dispatch_request(
    request: BrokerRequest,
    caller_uid: u32,
    caller_gid: u32,
    caller_role: CallerRole,
    audit_context: &DispatchAuditContext,
    config: &ServerConfig,
    audit_log: &AuditLog,
    resolver: Option<&Arc<BundleResolver>>,
) -> Result<DispatchResult, BrokerError> {
    dispatch_request_with_request_fds(
        request,
        caller_uid,
        caller_gid,
        caller_role,
        audit_context,
        config,
        audit_log,
        resolver,
        Vec::new(),
    )
    .await
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[allow(clippy::too_many_arguments)]
async fn dispatch_request_with_request_fds(
    request: BrokerRequest,
    caller_uid: u32,
    caller_gid: u32,
    caller_role: CallerRole,
    audit_context: &DispatchAuditContext,
    config: &ServerConfig,
    audit_log: &AuditLog,
    resolver: Option<&Arc<BundleResolver>>,
    request_fds: Vec<OwnedFd>,
) -> Result<DispatchResult, BrokerError> {
    let backend = LiveDispatchBackend {
        daemon_uid: config.d2bd_uid,
        daemon_gid: config.d2bd_gid,
        profile: config.profile,
        state_dir: config.state_dir.clone(),
        forward_socket_path: config.forward_socket_path.clone(),
        // The tree the broker's own private socket lives in: the bound every
        // Device-owned worker's per-Guest socket directory is derived under.
        runtime_root: config
            .socket_path
            .parent()
            .unwrap_or_else(|| Path::new(DEFAULT_BROKER_RUNTIME_DIR))
            .to_path_buf(),
    };
    dispatch_request_with_backend_and_request_fds(
        request,
        caller_uid,
        caller_gid,
        caller_role,
        audit_context,
        config,
        audit_log,
        resolver,
        &backend,
        request_fds,
    )
    .await
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[allow(clippy::too_many_arguments)]
async fn dispatch_request_with_backend<B: DispatchBackend>(
    request: BrokerRequest,
    caller_uid: u32,
    caller_gid: u32,
    caller_role: CallerRole,
    audit_context: &DispatchAuditContext,
    config: &ServerConfig,
    audit_log: &AuditLog,
    resolver: Option<&Arc<BundleResolver>>,
    backend: &B,
) -> Result<DispatchResult, BrokerError> {
    dispatch_request_with_backend_and_request_fds(
        request,
        caller_uid,
        caller_gid,
        caller_role,
        audit_context,
        config,
        audit_log,
        resolver,
        backend,
        Vec::new(),
    )
    .await
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[allow(clippy::too_many_arguments)]
async fn dispatch_request_with_backend_and_request_fds<B: DispatchBackend>(
    request: BrokerRequest,
    caller_uid: u32,
    caller_gid: u32,
    caller_role: CallerRole,
    audit_context: &DispatchAuditContext,
    config: &ServerConfig,
    audit_log: &AuditLog,
    resolver: Option<&Arc<BundleResolver>>,
    backend: &B,
    request_fds: Vec<OwnedFd>,
) -> Result<DispatchResult, BrokerError> {
    use d2b_contracts_broker::broker_wire::BrokerRequest as RealBrokerRequest;
    let bundle_metadata = audit_bundle_metadata(resolver.map(std::sync::Arc::as_ref));
    macro_rules! write_decision_op_record {
        ($($args:tt)*) => {
            write_decision_op_record_impl($($args)* audit_context)
        };
    }
    macro_rules! write_success_op_record {
        ($($args:tt)*) => {
            write_success_op_record_impl($($args)* audit_context)
        };
    }
    if !request_accepts_fd(&request) && !request_fds.is_empty() {
        return Err(BrokerError::Protocol(
            "unexpected request SCM_RIGHTS descriptor".to_owned(),
        ));
    }
    validate_broker_request(&request)?;
    if matches!(caller_role, CallerRole::HostShutdownUid { .. })
        && !matches!(
            request,
            RealBrokerRequest::Hello(_)
                | RealBrokerRequest::EnvelopeInvoke(_)
        )
    {
        return Err(BrokerError::HostShutdownRestricted);
    }
    match request {
        RealBrokerRequest::Hello(req) => {
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "Hello",
                "daemon-handshake",
                caller_uid,
                caller_gid,
                &caller_role,
                "daemon-handshake",
                "broker",
                None,
                OperationFields::Hello {
                    client_version: req.client_version,
                },
            )?;
            Ok(DispatchResult::no_fds(hello_ok_response(config.profile)))
        }
        RealBrokerRequest::PublishTrustedContext(req) => {
            // The daemon publishes its current provider-set revision and
            // controller/guest generations over the origination leg; the
            // store caches them durably and monotonically, and the reply
            // carries the broker epoch every context minted from this point
            // on must carry. A publication that would move the cached values
            // backwards - or a store I/O failure - is refused with the
            // shared stale-context code so the daemon's receiving leg never
            // trusts an epoch that was not acknowledged.
            //
            // The store opens lazily on this first arrival rather than at
            // broker startup: a state root that cannot host the store (a
            // constrained sandbox whose daemon state dir is absent or
            // read-only) must refuse publications fail-closed, not take the
            // whole broker down. The process store is a OnceLock with the
            // epoch already claimed by whoever opened it, so only the first
            // arrival opens; an open failure here means the broker has NO
            // usable store, and the publication is refused by name while
            // the broker keeps serving everything else. Restart
            // invalidation still holds: a fresh broker process bumps the
            // persisted epoch before it can serve a publication.
            // The store's channel boundary is a blocking one by design
            // (the single writer's commands are `blocking_send` /
            // `blocking_recv`, sanctioned for a dedicated blocking worker
            // per plan R4). The dispatch chain runs on the broker's async
            // runtime, so the store calls use the async channel legs
            // (`send().await` + awaited reply) instead of parking an
            // executor worker on the blocking boundary.
            if crate::envelope::trusted_context_store().is_none() {
                crate::envelope::init_trusted_context_store_async(&config.state_dir)
                    .await
                    .map_err(|error| {
                        BrokerError::LiveHandler(format!(
                            "trusted-context store unavailable: {error}"
                        ))
                    })?;
            }
            let reply = crate::envelope::trusted_context_store()
                .expect("the lazy init above just opened the store")
                .publication_reply(&req)
                .await
                .map_err(|error| {
                BrokerError::LiveHandler(format!(
                    "trusted-context publication refused: {}",
                    error.code()
                ))
            })?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "PublishTrustedContext",
                "daemon-handshake",
                caller_uid,
                caller_gid,
                &caller_role,
                req.zone.as_str(),
                "broker",
                None,
                OperationFields::PublishTrustedContext {
                    zone: req.zone.clone(),
                    provider_set_revision: req.provider_set_revision,
                    controller_generation: req.controller_generation,
                    guest_generation: req.guest_generation,
                },
            )?;
            Ok(DispatchResult::no_fds(
                BrokerResponse::PublishTrustedContext(reply),
            ))
        }
        RealBrokerRequest::OwnershipMatrixCheck(req) => {
            // U5: the first production envelope caller. The daemon's
            // host-prep DAG dispatches the ownership-matrix preflight as the
            // typed wire request (the established origination-leg carrier);
            // this arm is the envelope's production call site. The check
            // runs as the daemon's own authority - `CallerAuthority::Daemon`
            // - under the operation's committed row (see
            // `docs/reference/policy/broker-operations.json`), and the
            // dispatch step answers through the envelope: before the daemon
            // binds its forwarding socket the envelope refuses with
            // `unregistered-handler` (the documented prebind gap, no
            // transparent queue); after the daemon binds, the declaring
            // process's handler answers. The wire request carries no Zone -
            // the preflight is host-scope and the vm_id is the wire's own
            // authoritative audit-join axis - so the call's Zone field
            // carries the same identity the audit join derives.
            let zone = req.vm_id.as_str();
            let payload = serde_json::json!({ "vm": req.vm_id.as_str() });
            let invocation = backend
                .operation_envelope()
                .call(
                    crate::envelope::CallerAuthority::Daemon,
                    "OwnershipMatrixCheck",
                    zone,
                    &payload,
                )
                .await;
            if let Err(refusal) = invocation {
                tracing::warn!(
                    broker_operation = "OwnershipMatrixCheck",
                    refusal = %refusal.code,
                    "ownership-matrix preflight refused through the envelope"
                );
                return Err(BrokerError::LiveHandler(format!(
                    "OwnershipMatrixCheck refused: {}",
                    refusal.code
                )));
            }
            // The success path records the preflight as an allowed entry
            // with the VM target. The record carries no typed `OperationFields`
            // shape yet: the row's audit field set stays empty and the
            // operation is pinned in the served-but-unaudited-shape set
            // (same marker as `PollChildReaped`, see
            // `a_served_operation_names_the_audit_shape_its_records_use`),
            // so the closed audit vocabulary in `catalog.rs` does not widen
            // for one caller.
            audit_log
                .write_entry_with_caller_ids(
                    "OwnershipMatrixCheck",
                    caller_uid,
                    caller_gid,
                    "allowed",
                    req.vm_id.as_str(),
                    "success",
                )
                .map_err(|err| BrokerError::Protocol(err.to_string()))?;
            Ok(DispatchResult::no_fds(ack_response("OwnershipMatrixCheck")))
        }
        RealBrokerRequest::EnvelopeInvoke(req) => {
            // The generic envelope invocation surface (U10, KTD10): one
            // origination-leg control path in place of one typed dispatch
            // arm per operation. The broker runs the committed operation's
            // five envelope steps - resolve the row, authorize the caller,
            // validate the payload, audit, dispatch - and the dispatch
            // answers from the declaring process (forwarded leg) or from
            // the broker's own broker-generic handlers (in-broker leg).
            // A root call is authorized as the attested caller's own class;
            // a handler's nested (sandwich) call reconstructs the evidence
            // chain from the carrier and is authorized against the chain's
            // initiating principal (KTD6). Retired per-operation wire
            // variants are refused by the wire-version gate before this
            // match, so a straggler peer gets the stale-wire-version
            // refusal plus an audit record, never a silent malformed-wire
            // drop (KTD10).
            let caller = crate::envelope::CallerAuthority::classify(&caller_role);
            let operation = req.operation.as_str();
            let zone = req.zone.as_str();
            let invocation = match (&req.chain_root_invocation_id, &req.chain_identities) {
                (Some(root), Some(identities)) => {
                    let Some((head, tail)) = identities.split_first() else {
                        return Err(BrokerError::RequestValidation {
                            operation: "EnvelopeInvoke",
                            reason: "a chain with no identities is not a chain",
                        });
                    };
                    let mut chain =
                        d2b_audit::evidence_chain::EvidenceChain::root(root.clone(), head.clone());
                    for identity in tail {
                        chain = chain.nested(identity.clone());
                    }
                    backend
                        .operation_envelope()
                        .call_nested_with_fds(
                            chain,
                            operation,
                            zone,
                            &req.payload,
                            &request_fds,
                        )
                        .await
                }
                (None, None) => {
                    backend
                        .operation_envelope()
                        .call_with_fds(caller, operation, zone, &req.payload, &request_fds)
                        .await
                }
                _ => {
                    return Err(BrokerError::RequestValidation {
                        operation: "EnvelopeInvoke",
                        reason: "the evidence chain is partial",
                    });
                }
            };
            match invocation {
                Ok(invocation) => {
                    let crate::envelope::DispatchOutcome { result, fds } = invocation.outcome;
                    let fd_indexes: Vec<u32> = (0..fds.len() as u32).collect();
                    let fd_kinds: Vec<d2b_contracts_broker::broker_wire::FdKind> =
                        match crate::catalog::BrokerOperationRow::find(operation)
                            .and_then(|row| row.fd_kind)
                        {
                            // The row's declared kind labels every returned
                            // descriptor; a row that mints fds without one
                            // still labels them `Any` so the index and kind
                            // lists never diverge (U10).
                            Some(kind) => vec![kind; fds.len()],
                            None => vec![d2b_contracts_broker::broker_wire::FdKind::Any; fds.len()],
                        };
                    Ok(DispatchResult::with_fds(
                        BrokerResponse::EnvelopeInvoke(
                            d2b_contracts_broker::broker_wire::EnvelopeInvokeResponse {
                                operation: operation.to_owned(),
                                invocation_id: invocation.invocation_id,
                                result: serde_json::to_value(&result).ok(),
                                refusal: None,
                                detail: None,
                                fd_indexes,
                                fd_kinds,
                            },
                        ),
                        fds,
                    ))
                }
                Err(refusal) => Ok(DispatchResult::no_fds(BrokerResponse::EnvelopeInvoke(
                    d2b_contracts_broker::broker_wire::EnvelopeInvokeResponse {
                        operation: operation.to_owned(),
                        invocation_id: refusal.invocation_id,
                        result: None,
                        refusal: Some(refusal.code.to_owned()),
                        detail: refusal.detail,
                        fd_indexes: Vec::new(),
                        fd_kinds: Vec::new(),
                    },
                ))),
            }
        }
        RealBrokerRequest::ExportBrokerAudit(req) => {
            // Real wire filter is a typed BrokerAuditFilter struct;
            // serialize to JSON so the daily-file export path keeps the
            // existing substring match semantics.
            let filter_json = req
                .filter
                .as_ref()
                .and_then(|f| serde_json::to_string(f).ok());
            let op_fields = OperationFields::ExportBrokerAudit {
                since: req.since.clone(),
                filter: filter_json.clone(),
            };
            if !caller_role_is_admin(&caller_role) {
                write_decision_op_record!(
                    audit_log,
                    bundle_metadata,
                    "ExportBrokerAudit",
                    "audit-log",
                    caller_uid,
                    caller_gid,
                    &caller_role,
                    "audit-log",
                    "broker",
                    None,
                    "denied-refused",
                    Some("audit-requires-admin"),
                    op_fields,
                )?;
                return Err(BrokerError::AuditRequiresAdmin);
            }
            let page = audit_log
                .export_page(
                    req.since.as_deref(),
                    filter_json.as_deref(),
                    req.cursor.as_ref(),
                    req.limit,
                )
                .map_err(|err| BrokerError::Protocol(err.to_string()))?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "ExportBrokerAudit",
                "audit-log",
                caller_uid,
                caller_gid,
                &caller_role,
                "audit-log",
                "broker",
                None,
                op_fields,
            )?;
            Ok(DispatchResult::no_fds(export_broker_audit_ok_response(
                page,
            )))
        }
        // Live bundle-dependent real-wire ops. Each one (1) resolves the
        // daemon's opaque BundleOpId via the trusted-bundle resolver,
        // (2) invokes the matching live_handlers::* executor against the
        // system executor, (3) writes the audit row, (4) returns an
        // Ack or fd-bearing response.
        RealBrokerRequest::ReconcileStorageScope(req) => {
            let resolver = require_resolver(resolver)?;
            let response = crate::ops::storage_contract::reconcile_storage_scope(
                resolver,
                &req.storage_ref,
                req.apply,
            )
            .await
            .map_err(|err| match err {
                crate::ops::storage_contract::StorageContractError::UnknownStorage(id) => {
                    BrokerError::BundleIntentMissing {
                        kind: "storage",
                        intent_id: id,
                    }
                }
                other => BrokerError::LiveHandler(other.to_string()),
            })?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "ReconcileStorageScope",
                req.storage_ref.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                response.scope.as_str(),
                req.storage_ref.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::ReconcileStorageScope {
                    storage_ref: response.storage_ref.as_str().to_owned(),
                    scope: response.scope.clone(),
                    kind: response.kind.clone(),
                    status: format!("{:?}", response.status),
                    applied: response.applied,
                    path_hash: response.path_hash.clone(),
                },
            )?;
            Ok(DispatchResult::no_fds(
                BrokerResponse::ReconcileStorageScope(response),
            ))
        }
        RealBrokerRequest::ValidateLockSpec(req) => {
            let resolver = require_resolver(resolver)?;
            let response =
                crate::ops::storage_contract::validate_lock_spec(resolver, &req.lock_ref)
                    .await
                    .map_err(
                    |err| match err {
                        crate::ops::storage_contract::StorageContractError::UnknownLock(id) => {
                            BrokerError::BundleIntentMissing {
                                kind: "sync-lock",
                                intent_id: id,
                            }
                        }
                        other => BrokerError::LiveHandler(other.to_string()),
                    },
                )?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "ValidateLockSpec",
                req.lock_ref.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                response.scope.as_str(),
                req.lock_ref.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::ValidateLockSpec {
                    lock_ref: response.lock_ref.as_str().to_owned(),
                    scope: response.scope.clone(),
                    kind: response.kind.clone(),
                    cloexec_required: response.cloexec_required,
                    fd_passing_mechanism: response.fd_passing_mechanism.clone(),
                    order_key: response.order_key.clone(),
                },
            )?;
            Ok(DispatchResult::no_fds(BrokerResponse::ValidateLockSpec(
                response,
            )))
        }

        RealBrokerRequest::PipeWireAudio(req) => {
            let resolver = require_resolver(resolver)?;
            let intent = resolver
                .find_runner_intent(req.bundle_runner_intent_ref.as_str())
                .ok_or_else(|| BrokerError::BundleIntentMissing {
                    kind: "runner",
                    intent_id: req.bundle_runner_intent_ref.as_str().to_owned(),
                })?;
            if req.vm_id.as_str() != intent.vm_name
                || req.role_id.as_str() != intent.role_id
                || req.bundle_runner_intent_ref.as_str() != intent.intent_id
                || intent.role != d2b_core::processes::ProcessRole::Audio
            {
                return Err(BrokerError::LiveHandler(
                    "audio effect intent mismatch".to_owned(),
                ));
            }

            let env_value = |key: &str| {
                intent
                    .env
                    .iter()
                    .find_map(|entry| entry.strip_prefix(&format!("{key}=")))
            };
            let response = match (
                env_value("WPCTL_PATH"),
                env_value("PW_DUMP_PATH"),
                env_value("PIPEWIRE_RUNTIME_DIR"),
            ) {
                (Some(wpctl_path), Some(pw_dump_path), Some(runtime_dir))
                    if Path::new(wpctl_path).is_absolute()
                        && Path::new(pw_dump_path).is_absolute()
                        && Path::new(runtime_dir).is_absolute() =>
                {
                    // The PipeWire probe runs in async time on the dispatch
                    // runtime. The wait is bounded by PW_PROBE_TIMEOUT (5s,
                    // mirroring the zbus SYSTEMD_METHOD_TIMEOUT precedent):
                    // this ADDS a bound where the old synchronous path had
                    // none, so a stalled pw_dump can no longer pin a
                    // dispatch worker. A timeout is treated exactly like a
                    // failed probe (host not ready), never a stall.
                    let dump = tokio::time::timeout(
                        PW_PROBE_TIMEOUT,
                        tokio::process::Command::new(pw_dump_path)
                            .env_clear()
                            .env("PIPEWIRE_RUNTIME_DIR", runtime_dir)
                            .env("XDG_RUNTIME_DIR", runtime_dir)
                            .stdin(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .output(),
                    )
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .filter(|output| output.status.success());
                    match dump {
                        None => d2b_contracts_broker::broker_wire::PipeWireAudioResponse {
                            vm_id: req.vm_id.clone(),
                            role_id: req.role_id.clone(),
                            applied: false,
                            host_ready: false,
                            node_present: false,
                        },
                        Some(dump) => {
                            let node_id = serde_json::from_slice::<Value>(&dump.stdout)
                                .ok()
                                .and_then(|document| {
                                    let expected_app = format!("d2b-{}", intent.vm_name);
                                    let expected_class = match req.channel {
                                        d2b_contracts_broker::broker_wire::PipeWireAudioChannel::Speaker => {
                                            "Stream/Output/Audio"
                                        }
                                        d2b_contracts_broker::broker_wire::PipeWireAudioChannel::Microphone => {
                                            "Stream/Input/Audio"
                                        }
                                    };
                                    let mut matches =
                                        document.as_array()?.iter().filter_map(|entry| {
                                            let props = entry.get("info")?.get("props")?;
                                            if props.get("application.name")?.as_str()?
                                                != expected_app
                                                || props.get("media.class")?.as_str()?
                                                    != expected_class
                                            {
                                                return None;
                                            }
                                            entry.get("id").and_then(Value::as_u64)
                                        });
                                    let first = matches.next()?;
                                    if matches.next().is_some() {
                                        None
                                    } else {
                                        Some(first.to_string())
                                    }
                                });
                            match node_id {
                                None => d2b_contracts_broker::broker_wire::PipeWireAudioResponse {
                                    vm_id: req.vm_id.clone(),
                                    role_id: req.role_id.clone(),
                                    applied: false,
                                    host_ready: true,
                                    node_present: false,
                                },
                                Some(node_id) => {
                                    let mut command = tokio::process::Command::new(wpctl_path);
                                    command
                                        .env_clear()
                                        .env("PIPEWIRE_RUNTIME_DIR", runtime_dir)
                                        .env("XDG_RUNTIME_DIR", runtime_dir)
                                        .stdin(std::process::Stdio::null())
                                        .stdout(std::process::Stdio::null())
                                        .stderr(std::process::Stdio::null());
                                    let level_arg;
                                    match req.action {
                                        d2b_contracts_broker::broker_wire::PipeWireAudioAction::SetGrant {
                                            on,
                                        } => {
                                            command.args([
                                                "set-mute",
                                                &node_id,
                                                if on { "0" } else { "1" },
                                            ]);
                                        }
                                        d2b_contracts_broker::broker_wire::PipeWireAudioAction::SetLevel {
                                            percent,
                                        } => {
                                            level_arg = format!("{percent}%");
                                            command.args(["set-volume", &node_id, &level_arg]);
                                        }
                                    }
                                    // Same bounded async wait as the pw_dump
                                    // probe above (PW_PROBE_TIMEOUT).
                                    let applied = tokio::time::timeout(
                                        PW_PROBE_TIMEOUT,
                                        command.output(),
                                    )
                                    .await
                                    .map(|output| output.map(|output| output.status.success()))
                                    .unwrap_or(Ok(false))
                                    .unwrap_or(false);
                                    d2b_contracts_broker::broker_wire::PipeWireAudioResponse {
                                        vm_id: req.vm_id.clone(),
                                        role_id: req.role_id.clone(),
                                        applied,
                                        host_ready: true,
                                        node_present: true,
                                    }
                                }
                            }
                        }
                    }
                }
                _ => d2b_contracts_broker::broker_wire::PipeWireAudioResponse {
                    vm_id: req.vm_id.clone(),
                    role_id: req.role_id.clone(),
                    applied: false,
                    host_ready: false,
                    node_present: false,
                },
            };
            let action = match req.action {
                d2b_contracts_broker::broker_wire::PipeWireAudioAction::SetGrant { on } => {
                    format!("grant:{}", if on { "on" } else { "off" })
                }
                d2b_contracts_broker::broker_wire::PipeWireAudioAction::SetLevel { percent } => {
                    format!("level:{percent}")
                }
            };
            let channel = match req.channel {
                d2b_contracts_broker::broker_wire::PipeWireAudioChannel::Speaker => "speaker",
                d2b_contracts_broker::broker_wire::PipeWireAudioChannel::Microphone => "microphone",
            };
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "PipeWireAudio",
                req.bundle_runner_intent_ref.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                req.vm_id.as_str(),
                req.role_id.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::PipeWireAudio {
                    vm_id: req.vm_id.as_str().to_owned(),
                    role_id: req.role_id.as_str().to_owned(),
                    channel: channel.to_owned(),
                    action,
                    applied: response.applied,
                    host_ready: response.host_ready,
                    node_present: response.node_present,
                },
            )?;
            Ok(DispatchResult::no_fds(BrokerResponse::PipeWireAudio(
                response,
            )))
        }
        RealBrokerRequest::DelegateCgroupV2(req) => {
            let resolver = require_resolver(resolver)?;
            let exec = live_exec(config);
            crate::ops::cgroup::live_delegate_cgroup_v2(&exec, resolver, &req, audit_log)
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "DelegateCgroupV2",
                req.scope_id.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                req.scope_id.as_str(),
                req.scope_id.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::DelegateCgroupV2 {
                    scope_id: req.scope_id.as_str().to_owned(),
                },
            )?;
            Ok(DispatchResult::no_fds(ack_response("DelegateCgroupV2")))
        }
        RealBrokerRequest::ModprobeIfAllowed(req) => {
            let resolver = require_resolver(resolver)?;
            let exec = live_exec(config);
            let outcome =
                crate::ops::modprobe::live_modprobe_if_allowed(&exec, resolver, &req, audit_log)
                    .await
                    .map_err(BrokerError::LiveHandler)?;
            let disposition = serde_json::to_value(outcome.disposition)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_owned());
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "ModprobeIfAllowed",
                req.module_name.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                req.module_name.as_str(),
                "host",
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::ModprobeIfAllowed {
                    module_name: outcome.module_name,
                    matrix_entry_id: outcome.matrix_entry_id,
                    modules_disabled_sysctl: outcome.modules_disabled_sysctl,
                    disposition,
                },
            )?;
            Ok(DispatchResult::no_fds(ack_response("ModprobeIfAllowed")))
        }
        RealBrokerRequest::OpenCgroupDir(req) => {
            let resolver = require_resolver(resolver)?;
            let exec = live_exec(config);
            let outcome =
                crate::ops::cgroup::live_open_cgroup_dir(&exec, resolver, &req, audit_log)
                    .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            let path_class = match req.path_class {
                d2b_contracts::types::PathClass::Runtime => "runtime",
                d2b_contracts::types::PathClass::Vm => "vm",
            };
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "OpenCgroupDir",
                req.scope_id.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                req.scope_id.as_str(),
                req.scope_id.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::OpenCgroupDir {
                    scope_id: req.scope_id.as_str().to_owned(),
                    path_class: path_class.to_owned(),
                    cgroup_path: outcome.cgroup_path.display().to_string(),
                },
            )?;
            Ok(DispatchResult::with_fd(
                ack_response("OpenCgroupDir"),
                outcome.fd,
            ))
        }
        RealBrokerRequest::OpenDevice(req) => {
            let resolver = require_resolver(resolver)?;
            let exec = live_exec(config);
            let outcome = crate::ops::device::live_open_device(&exec, resolver, &req, audit_log)
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "OpenDevice",
                req.role_id.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                req.role_id.as_str(),
                req.role_id.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::OpenDevice {
                    role_id: req.role_id.as_str().to_owned(),
                    device_class: outcome.device_class,
                    device_path: outcome.device_path.display().to_string(),
                    matrix_entry_id: outcome.matrix_entry_id,
                },
            )?;
            Ok(DispatchResult::with_fd(
                ack_response("OpenDevice"),
                outcome.fd,
            ))
        }
        RealBrokerRequest::OpenFuse(req) => {
            let resolver = require_resolver(resolver)?;
            let exec = live_exec(config);
            let outcome = crate::ops::device::live_open_fuse(&exec, resolver, &req, audit_log)
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "OpenFuse",
                req.role_id.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                req.role_id.as_str(),
                req.role_id.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::OpenFuse {
                    role_id: req.role_id.as_str().to_owned(),
                    device_class: outcome.device_class,
                    device_path: outcome.device_path.display().to_string(),
                    matrix_entry_id: outcome.matrix_entry_id,
                },
            )?;
            Ok(DispatchResult::with_fd(
                ack_response("OpenFuse"),
                outcome.fd,
            ))
        }
        RealBrokerRequest::OpenHidrawSecurityKey(req) => {
            let resolver = require_resolver(resolver)?;
            let outcome = crate::ops::security_key::live_open_hidraw_security_key(
                &req,
                &resolver.host.security_key_selectors,
                audit_log,
            )
            .await
            .map_err(|error| BrokerError::LiveHandler(error.to_string()))?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "OpenHidrawSecurityKey",
                req.vm_id.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                req.vm_id.as_str(),
                req.selector_id.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::OpenHidrawSecurityKey {
                    vm_id: req.vm_id.as_str().to_owned(),
                    selector_id: outcome.selector_label.clone(),
                    device_class: outcome.device_class.clone(),
                    resolved: true,
                },
            )?;
            Ok(DispatchResult::with_fd(
                BrokerResponse::OpenHidrawSecurityKey(
                    d2b_contracts_broker::broker_wire::OpenHidrawSecurityKeyResponse {
                        selector_resolved: outcome.selector_label,
                        device_class: outcome.device_class,
                    },
                ),
                outcome.fd,
            ))
        }
        RealBrokerRequest::OpenKvm(req) => {
            let resolver = require_resolver(resolver)?;
            let exec = live_exec(config);
            let outcome = crate::ops::device::live_open_kvm(&exec, resolver, &req, audit_log)
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "OpenKvm",
                req.role_id.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                req.role_id.as_str(),
                req.role_id.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::OpenKvm {
                    role_id: req.role_id.as_str().to_owned(),
                    device_class: outcome.device_class,
                    device_path: outcome.device_path.display().to_string(),
                    matrix_entry_id: outcome.matrix_entry_id,
                },
            )?;
            Ok(DispatchResult::with_fd(ack_response("OpenKvm"), outcome.fd))
        }
        RealBrokerRequest::QemuMediaEnroll(req) => {
            let resolver = require_resolver(resolver)?;
            let outcome = crate::ops::media::enroll(resolver, &req)
                .await
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "QemuMediaEnroll",
                req.media_ref.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                req.vm_id.as_str(),
                req.media_ref.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::QemuMediaEnroll {
                    vm_id: req.vm_id.as_str().to_owned(),
                    media_ref: req.media_ref.as_str().to_owned(),
                    read_only: outcome.response.read_only,
                    by_id_count: outcome.by_id_count,
                    udev_rule_written: outcome.response.udev_rule_written,
                    udev_reloaded: outcome.response.udev_reloaded,
                },
            )?;
            Ok(DispatchResult::no_fds(BrokerResponse::QemuMediaEnroll(
                outcome.response,
            )))
        }
        RealBrokerRequest::QemuMediaRefreshRegistry(req) => {
            let resolver = require_resolver(resolver)?;
            let outcome = crate::ops::media::refresh_registry(resolver)
                .await
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "QemuMediaRefreshRegistry",
                "qemu-media",
                caller_uid,
                caller_gid,
                &caller_role,
                "host",
                "qemu-media",
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::QemuMediaRefreshRegistry {
                    record_count: outcome.response.record_count,
                    redacted_index_written: outcome.response.redacted_index_written,
                    udev_rule_written: outcome.response.udev_rule_written,
                    udev_reloaded: outcome.response.udev_reloaded,
                },
            )?;
            Ok(DispatchResult::no_fds(
                BrokerResponse::QemuMediaRefreshRegistry(outcome.response),
            ))
        }
        RealBrokerRequest::QemuMediaBoot(req) => {
            let resolver = require_resolver(resolver)?;
            let outcome = crate::ops::media::boot(resolver, &req)
                .await
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "QemuMediaBoot",
                outcome.response.media_ref.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                outcome.response.vm_id.as_str(),
                outcome.response.media_ref.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::QemuMediaBoot {
                    vm_id: outcome.response.vm_id.as_str().to_owned(),
                    media_ref: outcome.response.media_ref.as_str().to_owned(),
                    slot: outcome.response.slot.clone(),
                    read_only: outcome.response.read_only,
                    registry_record_written: outcome.registry_record_written,
                    redacted_index_written: outcome.redacted_index_written,
                    udev_rule_written: outcome.udev_rule_written,
                    udev_reloaded: outcome.udev_reloaded,
                    qmp_commands: outcome.response.qmp_commands.clone(),
                },
            )?;
            Ok(DispatchResult::no_fds(BrokerResponse::QemuMediaBoot(
                outcome.response,
            )))
        }
        RealBrokerRequest::QemuMediaSystemPowerdown(req) => {
            let response = backend.qemu_media_system_powerdown(&req).await?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "QemuMediaSystemPowerdown",
                response.vm_id.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                response.vm_id.as_str(),
                response.vm_id.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::QemuMediaSystemPowerdown {
                    vm_id: response.vm_id.as_str().to_owned(),
                    qmp_command: "system_powerdown".to_owned(),
                },
            )?;
            Ok(DispatchResult::no_fds(
                BrokerResponse::QemuMediaSystemPowerdown(response),
            ))
        }
        RealBrokerRequest::QemuMediaQueryStatus(req) => {
            let response = backend.qemu_media_query_status(&req).await?;
            Ok(DispatchResult::no_fds(
                BrokerResponse::QemuMediaQueryStatus(response),
            ))
        }
        RealBrokerRequest::QemuMediaQuit(req) => {
            let response = backend.qemu_media_quit(&req).await?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "QemuMediaQuit",
                response.vm_id.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                response.vm_id.as_str(),
                response.vm_id.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::QemuMediaQuit {
                    vm_id: response.vm_id.as_str().to_owned(),
                    qmp_command: "quit".to_owned(),
                },
            )?;
            Ok(DispatchResult::no_fds(BrokerResponse::QemuMediaQuit(
                response,
            )))
        }
        RealBrokerRequest::QemuMediaAttach(req) => {
            let resolver = require_resolver(resolver)?;
            let outcome = crate::ops::media::attach(resolver, &req)
                .await
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "QemuMediaAttach",
                outcome.response.media_ref.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                outcome.response.vm_id.as_str(),
                outcome.response.media_ref.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::QemuMediaAttach {
                    vm_id: outcome.response.vm_id.as_str().to_owned(),
                    media_ref: outcome.response.media_ref.as_str().to_owned(),
                    slot: outcome.response.slot.clone(),
                    read_only: outcome.response.read_only,
                    qmp_commands: outcome.response.qmp_commands.clone(),
                },
            )?;
            Ok(DispatchResult::no_fds(BrokerResponse::QemuMediaAttach(
                outcome.response,
            )))
        }
        RealBrokerRequest::QemuMediaDetach(req) => {
            let resolver = require_resolver(resolver)?;
            let outcome = crate::ops::media::detach(resolver, &req)
                .await
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "QemuMediaDetach",
                outcome.response.media_ref.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                outcome.response.vm_id.as_str(),
                outcome.response.media_ref.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::QemuMediaDetach {
                    vm_id: outcome.response.vm_id.as_str().to_owned(),
                    media_ref: outcome.response.media_ref.as_str().to_owned(),
                    slot: outcome.response.slot.clone(),
                    read_only: outcome.response.read_only,
                    qmp_commands: outcome.response.qmp_commands.clone(),
                },
            )?;
            Ok(DispatchResult::no_fds(BrokerResponse::QemuMediaDetach(
                outcome.response,
            )))
        }
        RealBrokerRequest::OpenVhostNet(req) => {
            let resolver = require_resolver(resolver)?;
            let exec = live_exec(config);
            let outcome = crate::ops::device::live_open_vhost_net(&exec, resolver, &req, audit_log)
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "OpenVhostNet",
                req.role_id.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                req.role_id.as_str(),
                req.role_id.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::OpenVhostNet {
                    role_id: req.role_id.as_str().to_owned(),
                    device_class: outcome.device_class,
                    device_path: outcome.device_path.display().to_string(),
                    matrix_entry_id: outcome.matrix_entry_id,
                },
            )?;
            Ok(DispatchResult::with_fd(
                ack_response("OpenVhostNet"),
                outcome.fd,
            ))
        }

        RealBrokerRequest::StoreSync(req) => {
            let resolver = require_resolver(resolver)?;
            let vm_name = lookup_vm_name(resolver, &req.vm_id);
            let intent = resolver
                .find_store_view_intent(req.bundle_closure_ref.as_str())
                .filter(|intent| intent.vm == vm_name)
                .ok_or_else(|| BrokerError::BundleIntentMissing {
                    kind: "store-sync-closure",
                    intent_id: req.bundle_closure_ref.as_str().to_owned(),
                })?;
            let started = std::time::Instant::now();
            let result = crate::ops::store_sync::run_store_sync(intent, &vm_name, req.generation_token)
                .await;
            let total_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

            // ADR 0027: every StoreSync attempt that reaches this handler
            // emits EXACTLY ONE terminal `OperationFields::StoreSync`
            // record. The audit context is derived from the trusted
            // resolved intent (NOT the run_store_sync outcome), so a
            // failure that aborts before any link accounting still emits a
            // fully-attributed record. `generation_id` is the
            // collision-free on-disk key. Successful attempts use the
            // StoreSync handler's per-phase timings; pre-handler failures
            // still carry the dispatch-level total only.
            let hardlink_farm_path_str = intent.hardlink_farm_path.display().to_string();
            let closure_count = u32::try_from(intent.closure_paths.len()).unwrap_or(u32::MAX);
            let target_env = resolver
                .guest_vm_resources()
                .find(|(_, resource)| resource.metadata().name().as_str() == vm_name)
                .map(|(zone, _)| zone.as_str().to_owned());
            let timings = match &result {
                Ok(outcome) => outcome.timings,
                Err(_) => crate::ops::store_sync_audit::StoreSyncTimings {
                    total_ms,
                    ..Default::default()
                },
            };
            let audit_ctx = crate::ops::store_sync_audit::StoreSyncAuditContext {
                vm: vm_name.clone(),
                vm_id: req.vm_id.as_str().to_owned(),
                env: target_env,
                bundle_closure_ref: req.bundle_closure_ref.as_str().to_owned(),
                hardlink_farm_path: hardlink_farm_path_str.clone(),
                generation_id: crate::ops::store_sync::generation_id_for_intent(intent),
                generation_token: req.generation_token,
                caller_principal: Some(format!(
                    "uid:{caller_uid}/role:{}",
                    audit_context.peer_role.as_str()
                )),
                closure_count,
                timings,
            };
            let audit_fields = crate::ops::store_sync::audit_fields_for_result(audit_ctx, &result);
            debug_assert!(
                audit_fields.validate().is_ok(),
                "StoreSync terminal audit record violates the signed schema: {:?}",
                audit_fields.validate()
            );

            // ADR 0027 observability export: project the host-confidential
            // terminal record down to the signed positive-allow-list and
            // append it to the alloy-readable export directory. Emitted
            // exactly once per terminal attempt (before the success/failure
            // match consumes `audit_fields`). Best-effort: the broker audit
            // record is the source of truth, so a failed export write must
            // never fail the StoreSync operation.
            let export_record =
                crate::ops::store_sync_export::StoreSyncObservabilityRecord::from_audit_fields(
                    &audit_fields,
                );
            if let Err(err) = crate::ops::store_sync_export::append_export_record(
                &config.store_sync_export_dir,
                &export_record,
            ) {
                warn!(
                    target_vm = %export_record.target_vm,
                    error = %err,
                    "failed to write StoreSync observability export record"
                );
            }

            match result {
                Ok(outcome) => {
                    write_success_op_record!(
                        audit_log,
                        bundle_metadata,
                        "StoreSync",
                        req.vm_id.as_str(),
                        caller_uid,
                        caller_gid,
                        &caller_role,
                        outcome.vm.as_str(),
                        outcome.vm.as_str(),
                        tracing_span_id_str(req.tracing_span_id.as_ref()),
                        OperationFields::StoreSync(audit_fields),
                    )?;
                    Ok(DispatchResult::no_fds(BrokerResponse::StoreSync(
                        d2b_contracts_broker::broker_wire::StoreSyncResponse {
                            vm: outcome.vm,
                            generation_id: outcome.generation_id,
                            generation_token: outcome.generation_token,
                            hardlink_farm_path: hardlink_farm_path_str,
                            closure_count: outcome.closure_count,
                            retained_generations: outcome.retained_generations,
                            swept_count: outcome.swept_count,
                            cleanup_deferred: outcome.cleanup_deferred,
                        },
                    )))
                }
                Err(err) => {
                    // Terminal failure record (decision = "errored",
                    // result = "error"). The `failed`/`denied` audit shape
                    // is carried in `operation_fields`; the header
                    // `error_kind` is the classified stage slug.
                    let error_kind = store_sync_error_kind(err.error_stage());
                    tracing::warn!(
                        error_stage = error_kind,
                        error = %err,
                        "StoreSync execution failed",
                    );
                    write_decision_op_record!(
                        audit_log,
                        bundle_metadata,
                        "StoreSync",
                        req.vm_id.as_str(),
                        caller_uid,
                        caller_gid,
                        &caller_role,
                        req.vm_id.as_str(),
                        vm_name.as_str(),
                        tracing_span_id_str(req.tracing_span_id.as_ref()),
                        "errored",
                        Some(error_kind),
                        OperationFields::StoreSync(audit_fields),
                    )?;
                    Err(BrokerError::StoreSyncFailed {
                        error_stage: error_kind,
                        message: err.to_string(),
                    })
                }
            }
        }
        RealBrokerRequest::ApplyHostGenerationHandoff(req) => {
            let target = req.target.to_canonical_string();
            let fields = OperationFields::ApplyHostGenerationHandoff {
                target: target.clone(),
                source_generation: req.intent.source_generation,
                target_generation: req.intent.target_generation,
                state: "requested".to_owned(),
            };
            if !caller_role_is_admin(&caller_role)
                || !matches!(
                    req.caller_role,
                    d2b_contracts_broker::host_generation::HandoffCallerRole::Lifecycle
                        | d2b_contracts_broker::host_generation::HandoffCallerRole::Admin
                )
            {
                write_decision_op_record!(
                    audit_log,
                    bundle_metadata,
                    "ApplyHostGenerationHandoff",
                    &target,
                    caller_uid,
                    caller_gid,
                    &caller_role,
                    &target,
                    "host-generation",
                    None,
                    "denied-refused",
                    Some("handoff-requires-admin"),
                    fields,
                )?;
                return Err(BrokerError::AuditRequiresAdmin);
            }
            let response = backend
                .apply_host_generation_handoff(
                    &config.state_dir,
                    &config.activation_helper_path,
                    &req,
                )
                .await?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "ApplyHostGenerationHandoff",
                &response.target.to_canonical_string(),
                caller_uid,
                caller_gid,
                &caller_role,
                &response.target.to_canonical_string(),
                "host-generation",
                None,
                OperationFields::ApplyHostGenerationHandoff {
                    target,
                    source_generation: response.source_generation,
                    target_generation: response.target_generation,
                    state: format!("{:?}", response.state).to_lowercase(),
                },
            )?;
            Ok(DispatchResult::no_fds(
                BrokerResponse::ApplyHostGenerationHandoff(response),
            ))
        }
        RealBrokerRequest::UsbipBind(req) => {
            let resolver = require_resolver(resolver)?;
            let intent = find_usbip_bind_intent_or_wildcard(
                resolver,
                req.bundle_usbip_bind_intent_ref.as_str(),
            )
            .ok_or_else(|| BrokerError::BundleIntentMissing {
                kind: "usbip-bind",
                intent_id: req.bundle_usbip_bind_intent_ref.as_str().to_owned(),
            })?;
            let same_vm_replay = match crate::ops::usbip_lock::peek_owner(&intent.lock_path) {
                Some(owner) if owner == intent.vm_name => true,
                Some(owner) => {
                    return Err(BrokerError::UsbipLockConflict {
                        busid: intent.bus_id.clone(),
                        owner,
                    });
                }
                None => false,
            };
            let inspection = crate::ops::usbip_host::enforce_usbip_physical_policy(
                &intent,
                usb_device_sysfs_root(),
            )
            .await
            .map_err(|err| map_usbip_host_inspection_error_for_intent(&intent, err))?;
            let expected_identity = (inspection.vendor, inspection.product);
            let (audit_device_identity, rotation_audit) = usb_audit_device_identity_for_busid(
                usb_device_sysfs_root(),
                &intent.bus_id,
                expected_identity,
                &config.state_dir,
                config.test_mode,
            )
            .await?;
            if let Some(rotation_audit) = rotation_audit
                && let Some(rotation_audit_dedupe_key) =
                    mark_usb_audit_serial_hmac_rotation_audit_logged(&rotation_audit)
            {
                let rotation_audit_context = DispatchAuditContext {
                    peer_pid: audit_context.peer_pid,
                    peer_role: audit_context.peer_role.clone(),
                    verb: "UsbSerialCorrelationKeyRotate".to_owned(),
                    request_fields: serde_json::json!({
                        "detectedDuring": "UsbipBind",
                        "tracingSpanIdPresent": req.tracing_span_id.is_some(),
                    }),
                    started_at: audit_context.started_at,
                    audit_join: audit_context.audit_join.clone(),
                };
                if let Err(err) = write_success_op_record_impl(
                    audit_log,
                    bundle_metadata,
                    "UsbSerialCorrelationKeyRotate",
                    "usb-audit-serial-hmac",
                    caller_uid,
                    caller_gid,
                    &caller_role,
                    "usb-audit-serial-hmac",
                    "host",
                    tracing_span_id_str(req.tracing_span_id.as_ref()),
                    OperationFields::UsbSerialCorrelationKeyRotate(rotation_audit),
                    &rotation_audit_context,
                ) {
                    unmark_usb_audit_serial_hmac_rotation_audit_logged(&rotation_audit_dedupe_key);
                    return Err(err);
                }
            }
            let expected_device_node = inspection.device_node;
            backend.usbip_bind(&intent).await?;
            if let Err(grant_error) = grant_usbip_backend_device_acl(
                resolver,
                &intent,
                expected_identity,
                expected_device_node,
            )
            .await
            {
                return Err(rollback_usbip_bind_after_acl_grant_failure(
                    backend,
                    &intent,
                    same_vm_replay,
                    grant_error,
                )
                .await);
            }
            let scope_id = format!("env:{}", intent.env);
            let audit_result = write_success_op_record!(
                audit_log,
                bundle_metadata,
                "UsbipBind",
                intent.intent_id.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                intent.vm_name.as_str(),
                scope_id.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::UsbipBind {
                    bus_id: intent.bus_id.clone(),
                    vm: intent.vm_name.clone(),
                    device_identity: Some(audit_device_identity),
                },
            );
            if let Err(audit_error) = audit_result {
                rollback_usbip_bind_after_audit_failure(backend, resolver, &intent, same_vm_replay).await;
                return Err(audit_error);
            }
            Ok(DispatchResult::no_fds(ack_response("UsbipBind")))
        }
        RealBrokerRequest::UsbipUnbind(req) => {
            let resolver = require_resolver(resolver)?;
            let intent = find_usbip_bind_intent_or_wildcard(
                resolver,
                req.bundle_usbip_bind_intent_ref.as_str(),
            )
            .ok_or_else(|| BrokerError::BundleIntentMissing {
                kind: "usbip-bind",
                intent_id: req.bundle_usbip_bind_intent_ref.as_str().to_owned(),
            })?;
            let had_matching_lock = match crate::ops::usbip_lock::peek_owner(&intent.lock_path) {
                Some(owner) if owner == intent.vm_name => true,
                Some(owner) => {
                    return Err(BrokerError::UsbipLockConflict {
                        busid: intent.bus_id.clone(),
                        owner,
                    });
                }
                None => false,
            };
            backend.usbip_unbind(&intent).await?;
            if had_matching_lock {
                if let Err(revoke_error) = revoke_usbip_backend_device_acl(resolver, &intent).await {
                    return Err(handle_usbip_acl_revoke_failure_after_unbind(
                        &intent,
                        req.preserve_durable_claim,
                        revoke_error,
                    )
                    .await);
                }
                if !req.preserve_durable_claim {
                    crate::ops::usbip_lock::release_lock(&intent.lock_path, &intent.vm_name)
                        .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
                }
            }
            let scope_id = format!("env:{}", intent.env);
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "UsbipUnbind",
                intent.intent_id.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                intent.vm_name.as_str(),
                scope_id.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::UsbipUnbind {
                    bus_id: intent.bus_id.clone(),
                },
            )?;
            Ok(DispatchResult::no_fds(ack_response("UsbipUnbind")))
        }
        RealBrokerRequest::UsbipBindFirewallRule(req) => {
            // Render the carve-out through the canonical USBIP nft batch
            // helper so the existing table/chain ordering is preserved.
            let resolver = require_resolver(resolver)?;
            let intent = find_usbip_firewall_intent_or_wildcard(
                resolver,
                req.bundle_usbip_firewall_intent_ref.as_str(),
            )
            .ok_or_else(|| BrokerError::BundleIntentMissing {
                kind: "usbip-firewall",
                intent_id: req.bundle_usbip_firewall_intent_ref.as_str().to_owned(),
            })?;
            backend.usbip_bind_firewall_rule(resolver, &intent).await?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "UsbipBindFirewallRule",
                req.bundle_usbip_firewall_intent_ref.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                intent.bus_id.as_str(),
                "usbip-firewall",
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::UsbipBindFirewallRule {
                    bundle_usbip_firewall_intent_ref: req
                        .bundle_usbip_firewall_intent_ref
                        .as_str()
                        .to_owned(),
                },
            )?;
            Ok(DispatchResult::no_fds(ack_response(
                "UsbipBindFirewallRule",
            )))
        }
        RealBrokerRequest::UsbipProxyReconcile(req) => {
            let resolver = require_resolver(resolver)?;
            let expectations: Vec<(String, String, std::path::PathBuf)> = resolver
                .usbip_bind_intent_ids()
                .filter_map(|id| {
                    resolver.find_usbip_bind_intent(id).map(|intent| {
                        (
                            intent.bus_id.clone(),
                            intent.vm_name.clone(),
                            intent.lock_path.clone(),
                        )
                    })
                })
                .collect();
            backend.usbip_proxy_reconcile(&expectations).await?;
            reconcile_active_usbip_backend_acls(resolver).await?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "UsbipProxyReconcile",
                req.scope_id.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                "usbip-proxy",
                req.scope_id.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::UsbipProxyReconcile {},
            )?;
            Ok(DispatchResult::no_fds(ack_response("UsbipProxyReconcile")))
        }
        // SeedDnsmasqLease + BindMount dispatch arms. The bundle
        // resolver vouches for the per-VM intent rows; the broker
        // validates the VM exists in the trusted manifest, records the
        // typed audit row, and acks. The actual filesystem mutation
        // (writing the leases file / performing the bind mount) stays
        // out of scope here - both targets live in subtrees the daemon
        // already owns (`/var/lib/d2b/dnsmasq/`, per-VM store farm),
        // and this removes the typed-Unimplemented wall so the host-prep
        // DAG executor exercises a real broker round trip in eval-only
        // test environments. Live filesystem handlers land later.
        RealBrokerRequest::UsbipExplicitBind(req) => {
            // Explicit attach: bind a present sysfs busid to a USB-capable VM without
            // a bundle allowlist. The daemon has already performed:
            //   1. Sysfs presence check (fail-closed if device absent)
            //   2. USB-capable gate (RuntimeCapabilityGate::UsbHotplug)
            //   3. Active-claim exclusivity check (OFD lock read)
            // The broker acquires the per-busid OFD lock, runs `usbip bind`, and
            // grants the per-device ACL to the env's USBIP backend runner.
            let resolver = require_resolver(resolver)?;
            let lock_path = std::path::PathBuf::from(
                d2b_contracts::usbip::lock_path_for_busid(&req.bus_id),
            );

            // Same-VM replay: lock is already held by this VM (e.g. daemon restart).
            let same_vm_replay = match crate::ops::usbip_lock::peek_owner(&lock_path) {
                Some(owner) if owner == req.vm => true,
                Some(owner) => {
                    return Err(BrokerError::UsbipLockConflict {
                        busid: req.bus_id.clone(),
                        owner,
                    });
                }
                None => false,
            };

            // Inspect the device: no allowlist check (explicit path bypasses it).
            let inspection = crate::ops::usbip_host::inspect_usbip_host_device(
                usb_device_sysfs_root(),
                &req.bus_id,
            )
            .await
            .map_err(|err| {
                BrokerError::LiveHandler(format!(
                    "explicit usbip bind sysfs inspection failed for bus_id={}: {err}",
                    req.bus_id
                ))
            })?;

            let expected_identity = (inspection.vendor, inspection.product);
            let expected_device_node = inspection.device_node;

            // Audit device identity (same helper as declared path).
            let (audit_device_identity, rotation_audit) = usb_audit_device_identity_for_busid(
                usb_device_sysfs_root(),
                &req.bus_id,
                expected_identity,
                &config.state_dir,
                config.test_mode,
            )
            .await?;

            // Emit serial key rotation audit if needed (same policy as UsbipBind).
            if let Some(rotation_audit) = rotation_audit
                && let Some(rotation_audit_dedupe_key) =
                    mark_usb_audit_serial_hmac_rotation_audit_logged(&rotation_audit)
            {
                let rotation_audit_context = DispatchAuditContext {
                    peer_pid: audit_context.peer_pid,
                    peer_role: audit_context.peer_role.clone(),
                    verb: "UsbSerialCorrelationKeyRotate".to_owned(),
                    request_fields: serde_json::json!({
                        "detectedDuring": "UsbipExplicitBind",
                        "tracingSpanIdPresent": req.tracing_span_id.is_some(),
                    }),
                    started_at: audit_context.started_at,
                    audit_join: audit_context.audit_join.clone(),
                };
                if let Err(err) = write_success_op_record_impl(
                    audit_log,
                    bundle_metadata,
                    "UsbSerialCorrelationKeyRotate",
                    "usb-audit-serial-hmac",
                    caller_uid,
                    caller_gid,
                    &caller_role,
                    "usb-audit-serial-hmac",
                    "host",
                    tracing_span_id_str(req.tracing_span_id.as_ref()),
                    OperationFields::UsbSerialCorrelationKeyRotate(rotation_audit),
                    &rotation_audit_context,
                ) {
                    unmark_usb_audit_serial_hmac_rotation_audit_logged(&rotation_audit_dedupe_key);
                    return Err(err);
                }
            }

            // Build synthetic intent for backend calls: usbip_bind/usbip_unbind
            // only use bus_id, lock_path, and vm_name - the empty allowlist is never
            // checked on the explicit path (no enforce_usbip_physical_policy call).
            let synthetic_intent = d2b_core::bundle_resolver::ResolvedUsbipBindIntent {
                intent_id: format!("explicit:{}:{}", req.env, req.bus_id),
                bus_id: req.bus_id.clone(),
                vm_name: req.vm.clone(),
                env: req.env.clone(),
                lock_path: lock_path.clone(),
                vendor_product_allowlist: vec![],
                dynamic_bus_id: true,
            };

            // Acquire OFD lock and run `usbip bind` via the standard live handler.
            backend.usbip_bind(&synthetic_intent).await?;

            // Grant per-device ACL to the env's USBIP backend runner (no allowlist).
            if let Err(grant_error) = grant_explicit_usbip_backend_acl(
                resolver,
                &req.env,
                &req.bus_id,
                expected_identity,
                expected_device_node,
            )
            .await
            {
                if !same_vm_replay {
                    match backend.usbip_unbind(&synthetic_intent).await {
                        Ok(()) => {
                            if let Err(lock_error) =
                                crate::ops::usbip_lock::release_lock(&lock_path, &req.vm)
                            {
                                warn!(
                                    bus_id = %req.bus_id,
                                    vm = %req.vm,
                                    grant_error = ?grant_error,
                                    error = %lock_error,
                                    "UsbipExplicitBind ACL grant failed, rollback unbind succeeded, but lock rollback failed"
                                );
                            }
                        }
                        Err(rollback_error) => {
                            warn!(
                                bus_id = %req.bus_id,
                                vm = %req.vm,
                                grant_error = ?grant_error,
                                rollback_error = ?rollback_error,
                                "UsbipExplicitBind ACL grant failed and rollback unbind also failed"
                            );
                        }
                    }
                }
                return Err(grant_error);
            }

            let scope_id = format!("env:{}", req.env);
            let audit_result = write_success_op_record!(
                audit_log,
                bundle_metadata,
                "UsbipExplicitBind",
                req.bus_id.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                req.vm.as_str(),
                scope_id.as_str(),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::UsbipExplicitBind {
                    bus_id: req.bus_id.clone(),
                    vm: req.vm.clone(),
                    env: req.env.clone(),
                    device_identity: Some(audit_device_identity),
                },
            );
            if let Err(audit_error) = audit_result {
                if !same_vm_replay {
                    if let Err(revoke_err) =
                        revoke_explicit_usbip_backend_acl(resolver, &req.env, &req.bus_id).await
                    {
                        warn!(
                            bus_id = %req.bus_id,
                            vm = %req.vm,
                            error = ?revoke_err,
                            "UsbipExplicitBind audit write failed and backend ACL rollback failed"
                        );
                    }
                    match backend.usbip_unbind(&synthetic_intent).await {
                        Ok(()) => {
                            if let Err(lock_error) =
                                crate::ops::usbip_lock::release_lock(&lock_path, &req.vm)
                            {
                                warn!(
                                    bus_id = %req.bus_id,
                                    vm = %req.vm,
                                    error = %lock_error,
                                    "UsbipExplicitBind audit write failed and lock rollback failed"
                                );
                            }
                        }
                        Err(unbind_error) => {
                            warn!(
                                bus_id = %req.bus_id,
                                vm = %req.vm,
                                error = ?unbind_error,
                                "UsbipExplicitBind audit write failed and backend unbind rollback failed"
                            );
                        }
                    }
                }
                return Err(audit_error);
            }
            Ok(DispatchResult::no_fds(ack_response("UsbipExplicitBind")))
        }
        RealBrokerRequest::UsbipExplicitFirewallRule(req) => {
            // Explicit attach: install a per-busid nftables carve-out scoped to the
            // target VM's env bridge. The broker validates request IPs against the
            // env's declared host config values (cross-check to prevent rule spoofing),
            // validates anti-spoof bridge port flags, and atomically applies the
            // updated nft table preserving all currently active carve-outs.
            let resolver = require_resolver(resolver)?;

            let (_, rule_body) = build_explicit_usbip_rule_body(
                resolver,
                &req.env,
                &req.host_uplink_ip,
                &req.net_uplink_ip,
            )?;

            let host_nft_intent = resolver
                .find_nft_intent(&d2b_core::bundle_resolver::intent_id_nft_host())
                .ok_or_else(|| BrokerError::BundleIntentMissing {
                    kind: "nft",
                    intent_id: d2b_core::bundle_resolver::intent_id_nft_host(),
                })?;

            let decision = build_usbip_explicit_firewall_decision(
                resolver,
                host_nft_intent,
                &req.bus_id,
                &rule_body,
            )
            .await?;

            let nft_binary = nft_binary_path();
            let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
            let nft_script = decision.batch.render_nft_script();
            let expected_hash = persisted_nft_hash()
                .await
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))?
                .or_else(|| resolver.host.nftables.table_hash_after_apply.clone());
            crate::ops::nft::apply_with_coexistence(
                &exec,
                &nft_binary,
                &nft_script,
                resolver.host.nftables.ownership_id.as_str(),
                resolver.host.firewall_coexistence_policy.as_ref(),
                expected_hash.as_deref(),
            )
            .await
            .map_err(|err| match err {
                crate::ops::nft::ApplyWithCoexistenceError::CoexistenceRefused {
                    manager,
                    rationale,
                } => BrokerError::CoexistenceRefused { manager, rationale },
                crate::ops::nft::ApplyWithCoexistenceError::ParseFailed(err) => {
                    BrokerError::NftScriptParseFailed(err.to_string())
                }
                crate::ops::nft::ApplyWithCoexistenceError::CarveoutOrderingViolation(err) => {
                    BrokerError::CarveoutOrderingViolation(match err {
                        d2b_host::nftables::NftError::ForeignNftRuleShadowsD2b { details } => {
                            details
                        }
                        other => other.to_string(),
                    })
                }
                crate::ops::nft::ApplyWithCoexistenceError::DriftDetected {
                    expected,
                    observed,
                } => BrokerError::NftablesDriftDetected { expected, observed },
                crate::ops::nft::ApplyWithCoexistenceError::ForeignOwnership => {
                    BrokerError::LiveHandler("foreign-nft-ownership".to_owned())
                }
                crate::ops::nft::ApplyWithCoexistenceError::ReconcileExec(err) => {
                    BrokerError::LiveHandler(err.to_string())
                }
            })?;
            crate::ops::nft::persist_live_nft_hash(
                &exec,
                &nft_binary,
                &resolver.host.nftables.family,
                &resolver.host.nftables.table,
                &nft_hash_sidecar_path(),
            )
            .await
            .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;

            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "UsbipExplicitFirewallRule",
                req.bus_id.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                req.bus_id.as_str(),
                &format!("env:{}", req.env),
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::UsbipExplicitFirewallRule {
                    bus_id: req.bus_id.clone(),
                    env: req.env.clone(),
                },
            )?;
            Ok(DispatchResult::no_fds(ack_response(
                "UsbipExplicitFirewallRule",
            )))
        }
        // Disk-init dispatch. The broker resolves every `DiskInit`
        // plan-op from the trusted bundle for `vm_id` and creates the
        // disk images before the caller issues SpawnRunner. No
        // caller-supplied paths: the bundle is the only source of
        // `target_path`, `size_bytes`, `mode`, `owner_uid`, `owner_gid`.
        RealBrokerRequest::DiskInit(req) => {
            let resolver = require_resolver(resolver)?;
            let vm_name = lookup_vm_name(resolver, &req.vm_id);
            let summary = crate::ops::disk_init::live_disk_init(resolver.as_ref(), &vm_name)
                .await
                .map_err(|e| BrokerError::LiveHandler(e.to_string()))?;
            write_success_op_record!(
                audit_log,
                bundle_metadata,
                "DiskInit",
                vm_name.as_str(),
                caller_uid,
                caller_gid,
                &caller_role,
                vm_name.as_str(),
                "host",
                tracing_span_id_str(req.tracing_span_id.as_ref()),
                OperationFields::DiskInit {
                    vm_id: vm_name.clone(),
                    ops_total: summary.ops_total,
                    ops_created: summary.ops_created,
                    ops_skipped: summary.ops_skipped,
                    ops_repaired: Some(summary.ops_repaired),
                    ops_posture_repaired: Some(summary.ops_posture_repaired),
                    target_paths_hash: summary.target_paths_hash,
                },
            )?;
            Ok(DispatchResult::no_fds(ack_response("DiskInit")))
        }
        // Every remaining variant is a reserved stub. One table arm serves
        // them all, so the arm count follows the committed dispositions rather
        // than the variant list, and the refusal names the deferral marker the
        // committed row carries. A variant that is neither served nor reserved
        // is a bug in this match, not a silent success.
        request => {
            let operation = request.op_name();
            match crate::catalog::stub_target(operation) {
                Some(stub) => Err(BrokerError::Unimplemented {
                    operation,
                    target_wave: stub.as_str(),
                }),
                None => Err(BrokerError::Protocol(format!(
                    "uncommitted dispatch arm for {operation}"
                ))),
            }
        }
    }
}

#[derive(Clone, Copy)]
struct AuditBundleMetadata<'a> {
    bundle_version: &'a str,
    bundle_hash: &'a str,
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn audit_bundle_metadata(resolver: Option<&BundleResolver>) -> AuditBundleMetadata<'_> {
    match resolver {
        Some(resolver) => AuditBundleMetadata {
            bundle_version: resolver.audit_bundle_version(),
            bundle_hash: resolver.audit_bundle_hash(),
        },
        None => AuditBundleMetadata {
            bundle_version: "unknown",
            bundle_hash: "",
        },
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn caller_role_authz_result(caller_role: &CallerRole) -> &'static str {
    match caller_role {
        CallerRole::AdminUid { .. } | CallerRole::RootUid { .. } => "admin",
        CallerRole::LauncherUid { .. } => "launcher",
        CallerRole::HostShutdownUid { .. } => "host-shutdown",
        CallerRole::NotAuthorized => "deny",
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn audit_timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[allow(clippy::too_many_arguments)]
fn write_decision_op_record_impl(
    audit_log: &AuditLog,
    bundle_metadata: AuditBundleMetadata<'_>,
    operation: &str,
    public_operation_id: &str,
    peer_uid: u32,
    peer_gid: u32,
    caller_role: &CallerRole,
    subject_id: &str,
    scope_id: &str,
    tracing_span_id: Option<&str>,
    decision: &str,
    error_kind: Option<&str>,
    operation_fields: OperationFields,
    audit_context: &DispatchAuditContext,
) -> Result<(), BrokerError> {
    let operation_fields = serde_json::to_value(&operation_fields).map_err(|err| {
        BrokerError::Protocol(format!("serialize {operation} audit fields: {err}"))
    })?;
    let event_id = new_event_id()
        .map_err(|err| BrokerError::Protocol(format!("generate audit event id: {err}")))?;
    let (zone_id, operation_identity, public_operation_id) = if let Some(join) =
        audit_context.audit_join.as_ref()
    {
        let zone_id = d2b_audit::ZoneId::parse(join.zone_id.as_str())
            .map_err(|_| BrokerError::Protocol("audit zone identity invalid".to_owned()))?;
        let operation_identity = d2b_audit::OperationIdentity::parse(
            join.operation_identity.as_str(),
        )
        .map_err(|_| BrokerError::Protocol("audit operation identity invalid".to_owned()))?;
        let public_operation_id = operation_identity.as_str().to_owned();
        (zone_id, operation_identity, public_operation_id)
    } else {
        let zone_id = d2b_audit::ZoneId::derive(scope_id)
            .map_err(|_| BrokerError::Protocol("audit zone identity invalid".to_owned()))?;
        let operation_identity = d2b_audit::OperationIdentity::derive(public_operation_id)
            .map_err(|_| BrokerError::Protocol("audit operation identity invalid".to_owned()))?;
        (zone_id, operation_identity, public_operation_id.to_owned())
    };
    let record = OpAuditRecord {
        record_class: BrokerAuditRecordClass::Durability,
        ts_ms: audit_timestamp_ms(),
        broker_version: BROKER_VERSION,
        bundle_version: bundle_metadata.bundle_version,
        bundle_hash: bundle_metadata.bundle_hash,
        operation,
        public_operation_id: &public_operation_id,
        zone_id: &zone_id,
        operation_identity: &operation_identity,
        event_id: event_id.as_str(),
        peer_uid,
        peer_gid,
        peer_pid: audit_context.peer_pid,
        peer_role: audit_context.peer_role.as_str(),
        authz_result: caller_role_authz_result(caller_role),
        subject_id,
        scope_id,
        verb: audit_context.verb.as_str(),
        request_fields: audit_context.request_fields.clone(),
        decision,
        result: result_for_decision(decision),
        error_kind,
        tracing_span_id,
        duration_us: audit_context.duration_us(),
        operation_fields: Some(operation_fields),
        zone_operation_key: d2b_audit::ZoneOperationKey::new(
            zone_id.clone(),
            operation_identity.clone(),
        ),
    };
    audit_log
        .write_op_record(&record)
        .map_err(|err| BrokerError::Protocol(err.to_string()))
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[allow(clippy::too_many_arguments)]
fn write_success_op_record_impl(
    audit_log: &AuditLog,
    bundle_metadata: AuditBundleMetadata<'_>,
    operation: &str,
    public_operation_id: &str,
    peer_uid: u32,
    peer_gid: u32,
    caller_role: &CallerRole,
    subject_id: &str,
    scope_id: &str,
    tracing_span_id: Option<&str>,
    operation_fields: OperationFields,
    audit_context: &DispatchAuditContext,
) -> Result<(), BrokerError> {
    write_decision_op_record_impl(
        audit_log,
        bundle_metadata,
        operation,
        public_operation_id,
        peer_uid,
        peer_gid,
        caller_role,
        subject_id,
        scope_id,
        tracing_span_id,
        "allowed",
        None,
        operation_fields,
        audit_context,
    )
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn runner_signal_name(signal: d2b_contracts_broker::broker_wire::RunnerSignal) -> &'static str {
    match signal {
        d2b_contracts_broker::broker_wire::RunnerSignal::Term => "term",
        d2b_contracts_broker::broker_wire::RunnerSignal::Kill => "kill",
        d2b_contracts_broker::broker_wire::RunnerSignal::Quit => "quit",
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn runner_signal_number(signal: d2b_contracts_broker::broker_wire::RunnerSignal) -> i32 {
    match signal {
        d2b_contracts_broker::broker_wire::RunnerSignal::Term => libc::SIGTERM,
        d2b_contracts_broker::broker_wire::RunnerSignal::Kill => libc::SIGKILL,
        d2b_contracts_broker::broker_wire::RunnerSignal::Quit => libc::SIGQUIT,
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
const RUNNER_PIDFD_REGISTRY_CELL: &str = "runner-pidfd-registry";

#[cfg(not(feature = "layer1-bootstrap"))]
fn cell_store_error(error: crate::state_cells::CellStoreError) -> BrokerError {
    BrokerError::Protocol(error.to_string())
}

/// The runner pidfd registry as one declared ephemeral state cell.
///
/// The typed arm keeps serving registry reads from the cell backing until
/// the Process family's arm retires (U10/U11): keys are `runner_id`,
/// records hold the broker's dup of the spawn pidfd, and the cell is
/// ephemeral - in-process with reset-on-restart semantics, per the
/// committed row's durability facet. Registered records are broker-internal
/// spawn state, so the recorded principal is [`BROKER_PRINCIPAL`]; the
/// reconciler reads (`contains`/`remove`/`keys`) are principal-agnostic,
/// matching the map semantics they replace.
#[cfg(not(feature = "layer1-bootstrap"))]
#[derive(Debug, Clone, Copy)]
pub(crate) struct RunnerPidfdCell;

#[cfg(not(feature = "layer1-bootstrap"))]
impl RunnerPidfdCell {
    pub(crate) fn cell() -> (&'static str, crate::catalog::CellDurability) {
        let row = crate::catalog::BrokerOperationRow::find("DeregisterRunnerPidfd")
            .expect("DeregisterRunnerPidfd is a committed row");
        (
            row.state_cell
                .expect("DeregisterRunnerPidfd declares its state cell"),
            row.cell_durability
                .expect("DeregisterRunnerPidfd declares its cell durability"),
        )
    }

    pub(crate) fn contains_key(self, runner_id: &str) -> bool {
        let (cell, _) = Self::cell();
        crate::state_cells::broker_store().contains(cell, runner_id)
    }

    /// The registered pidfd handle, so the caller signals the live process
    /// (or dups) without racing a concurrent deregistration: the record's
    /// `Arc` keeps the descriptor valid.
    pub(crate) fn get(self, runner_id: &str) -> Option<Arc<OwnedFd>> {
        let (cell, _) = Self::cell();
        crate::state_cells::broker_store()
            .payload(cell, runner_id)
            .and_then(|payload| payload.downcast::<OwnedFd>().ok())
            .map(|pidfd| Arc::clone(&pidfd))
    }

    /// Duplicate the registered pidfd.
    pub(crate) fn duplicate(self, runner_id: &str) -> Option<OwnedFd> {
        self.get(runner_id).and_then(
            |pidfd| match dup(pidfd.as_raw_fd()).map(owned_fd_from_raw) {
                Ok(pidfd) => Some(pidfd),
                Err(error) => {
                    warn!(runner_id = %runner_id, error = %error, "duplicate runner pidfd failed");
                    None
                }
            },
        )
    }

    pub(crate) fn insert(self, runner_id: &str, pidfd: OwnedFd) -> Result<(), BrokerError> {
        let (cell, _) = Self::cell();
        crate::state_cells::broker_store()
            .insert_payload(
                cell,
                runner_id,
                crate::state_cells::BROKER_PRINCIPAL,
                Arc::new(pidfd),
            )
            .map_err(cell_store_error)
    }

    pub(crate) fn remove(self, runner_id: &str) -> bool {
        let (cell, _) = Self::cell();
        crate::state_cells::broker_store().remove(cell, runner_id)
    }

    pub(crate) fn keys(self) -> Vec<String> {
        let (cell, _) = Self::cell();
        crate::state_cells::broker_store().keys(cell)
    }

    pub(crate) fn clear(self) -> usize {
        let (cell, _) = Self::cell();
        crate::state_cells::broker_store().clear(cell)
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn runner_pidfds() -> RunnerPidfdCell {
    RunnerPidfdCell
}

/// The controller-bootstrap escrow registry.
///
/// `tokio::sync` per plan U8: every caller runs on the synchronous
/// dispatch workers or the reap task, and reaches the registry through the
/// non-blocking `try_lock` (the critical sections are single map
/// operations, never held across an await). A `Busy` collision is handled
/// per site exactly like the old poisoned path (error / skip / retry).
#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn controller_bootstrap_registry() -> &'static tokio::sync::Mutex<HashMap<String, OwnedFd>>
{
    static REGISTRY: LazyLock<tokio::sync::Mutex<HashMap<String, OwnedFd>>> =
        LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));
    &REGISTRY
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[derive(Clone)]
pub(crate) struct RunnerRegistration {
    pub(crate) vm_id: String,
    pub(crate) role_id: String,
    pub(crate) resource_ref: Option<d2b_contracts_resource::v3::ResourceRef>,
    pub(crate) resource_uid: Option<d2b_contracts_resource::v3::ResourceUid>,
    pub(crate) zone_uid: Option<d2b_contracts_resource::v3::ResourceUid>,
    pub(crate) generation: Option<u64>,
    pub(crate) runtime_scope: Option<[u8; 32]>,
    pub(crate) owner_ref: Option<d2b_contracts_resource::v3::ResourceRef>,
    pub(crate) provider_ref: Option<d2b_contracts_resource::v3::ResourceRef>,
    pub(crate) provider_identity: Option<[u8; 32]>,
    pub(crate) template_identity: Option<[u8; 32]>,
    pub(crate) role: d2b_contracts_broker::broker_wire::RunnerRole,
    pub(crate) bundle_runner_intent_ref: String,
    pub(crate) pid: i32,
    pub(crate) start_time_ticks: u64,
    pub(crate) binary_path: PathBuf,
    pub(crate) cgroup_subtree: String,
    pub(crate) guest_execution: Option<d2b_contracts_broker::broker_wire::GuestExecutionBinding>,
}

#[cfg(not(feature = "layer1-bootstrap"))]
impl std::fmt::Debug for RunnerRegistration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RunnerRegistration(<redacted>)")
    }
}

/// The runner-id-keyed metadata registry.
///
/// `tokio::sync` per plan U8 (see [`controller_bootstrap_registry`] for
/// the access model): synchronous callers use the non-blocking `try_lock`.
#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn runner_metadata_registry(
) -> &'static tokio::sync::Mutex<HashMap<String, RunnerRegistration>> {
    static REGISTRY: LazyLock<tokio::sync::Mutex<HashMap<String, RunnerRegistration>>> =
        LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));
    &REGISTRY
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn register_runner_metadata(
    runner_id: &str,
    request: &d2b_contracts_broker::broker_wire::SpawnRunnerRequest,
    intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent,
    pid: i32,
    start_time_ticks: u64,
) -> Result<(), BrokerError> {
    let cgroup_placement = private_cgroup_placement(
        &intent.cgroup_placement,
        request.vm_id.as_str(),
        request.runtime_scope,
        request.resource_ref.is_some(),
    )?;
    let mut registry = runner_metadata_registry()
        .try_lock()
        .map_err(|_| BrokerError::Protocol("runner metadata registry busy (tokio try-lock)".to_owned()))?;
    registry.insert(
        runner_id.to_owned(),
        RunnerRegistration {
            vm_id: request.vm_id.as_str().to_owned(),
            role_id: request.role_id.as_str().to_owned(),
            resource_ref: request.resource_ref.clone(),
            resource_uid: request.resource_uid.clone(),
            zone_uid: request.zone_uid.clone(),
            generation: request.generation,
            runtime_scope: request.runtime_scope,
            owner_ref: request.owner_ref.clone(),
            provider_ref: request.provider_ref.clone(),
            provider_identity: request.provider_identity,
            template_identity: request.template_identity,
            role: request.role,
            bundle_runner_intent_ref: request.bundle_runner_intent_ref.as_str().to_owned(),
            pid,
            start_time_ticks,
            binary_path: intent.binary_path.clone(),
            cgroup_subtree: cgroup_placement.subtree,
            guest_execution: request.guest_execution.clone(),
        },
    );
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn register_runner_metadata_from_open(
    runner_id: &str,
    request: &d2b_contracts_broker::broker_wire::OpenPidfdRequest,
    intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent,
) -> Result<(), BrokerError> {
    let role = runner_role_for_process_role(&intent.role).ok_or_else(|| {
        BrokerError::SpawnRunnerIntentMismatch {
            field: "role",
            requested: request.role_id.as_str().to_owned(),
            resolved: format!("{:?}", intent.role),
        }
    })?;
    let cgroup_placement = private_cgroup_placement(
        &intent.cgroup_placement,
        request.vm_id.as_str(),
        request.runtime_scope,
        request.resource_ref.is_some(),
    )?;
    let mut registry = runner_metadata_registry()
        .try_lock()
        .map_err(|_| BrokerError::Protocol("runner metadata registry busy (tokio try-lock)".to_owned()))?;
    registry.insert(
        runner_id.to_owned(),
        RunnerRegistration {
            vm_id: request.vm_id.as_str().to_owned(),
            role_id: request.role_id.as_str().to_owned(),
            resource_ref: request.resource_ref.clone(),
            resource_uid: request.resource_uid.clone(),
            zone_uid: request.zone_uid.clone(),
            generation: request.generation,
            runtime_scope: request.runtime_scope,
            owner_ref: request.owner_ref.clone(),
            provider_ref: request.provider_ref.clone(),
            provider_identity: request.provider_identity,
            template_identity: request.template_identity,
            role,
            bundle_runner_intent_ref: intent.intent_id.clone(),
            pid: request.pid,
            start_time_ticks: request.expected_start_time_ticks,
            binary_path: intent.binary_path.clone(),
            cgroup_subtree: cgroup_placement.subtree,
            guest_execution: request.guest_execution.clone(),
        },
    );
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn runner_registry_key(
    vm_id: &str,
    role_id: &str,
    resource_ref: Option<&d2b_contracts_resource::v3::ResourceRef>,
    resource_uid: Option<&d2b_contracts_resource::v3::ResourceUid>,
    zone_uid: Option<&d2b_contracts_resource::v3::ResourceUid>,
    runtime_scope: Option<[u8; 32]>,
) -> String {
    match (resource_ref, resource_uid, zone_uid, runtime_scope) {
        (Some(_), Some(_), Some(_), Some(runtime_scope)) => {
            let mut rendered = String::with_capacity(64);
            for byte in runtime_scope {
                rendered.push_str(&format!("{byte:02x}"));
            }
            format!("scope:{rendered}")
        }
        _ => format!("{vm_id}:{role_id}"),
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn runner_intent_id_for_open_pidfd(vm_id: &str, role_id: &str) -> String {
    let process_role_id = match role_id {
        "ch-runner" => "cloud-hypervisor",
        other => other,
    };
    d2b_core::bundle_resolver::intent_id_legacy_runner(vm_id, process_role_id)
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[allow(clippy::too_many_arguments)]
fn registration_matches(
    registration: &RunnerRegistration,
    resource_ref: Option<&d2b_contracts_resource::v3::ResourceRef>,
    resource_uid: Option<&d2b_contracts_resource::v3::ResourceUid>,
    pid: Option<i32>,
    start_time_ticks: Option<u64>,
    zone_uid: Option<&d2b_contracts_resource::v3::ResourceUid>,
    generation: Option<u64>,
    runtime_scope: Option<[u8; 32]>,
    owner_ref: Option<&d2b_contracts_resource::v3::ResourceRef>,
    provider_ref: Option<&d2b_contracts_resource::v3::ResourceRef>,
    provider_identity: Option<[u8; 32]>,
    template_identity: Option<[u8; 32]>,
    guest_execution: Option<&d2b_contracts_broker::broker_wire::GuestExecutionBinding>,
) -> bool {
    let strict = resource_ref.is_some();
    let owner_matches = if strict {
        registration.owner_ref.as_ref() == owner_ref
    } else {
        owner_ref.is_none_or(|owner| registration.owner_ref.as_ref() == Some(owner))
    };
    let provider_ref_matches = if strict {
        registration.provider_ref.as_ref() == provider_ref
    } else {
        provider_ref.is_none_or(|provider| registration.provider_ref.as_ref() == Some(provider))
    };
    let provider_identity_matches = if strict {
        registration.provider_identity == provider_identity
    } else {
        provider_identity.is_none_or(|identity| registration.provider_identity == Some(identity))
    };
    let template_identity_matches = if strict {
        registration.template_identity == template_identity
    } else {
        template_identity.is_none_or(|identity| registration.template_identity == Some(identity))
    };
    registration.resource_ref.as_ref() == resource_ref
        && registration.resource_uid.as_ref() == resource_uid
        && pid.is_none_or(|pid| registration.pid == pid)
        && start_time_ticks.is_none_or(|ticks| registration.start_time_ticks == ticks)
        && registration.zone_uid.as_ref() == zone_uid
        && generation.is_none_or(|generation| registration.generation == Some(generation))
        && runtime_scope.is_none_or(|scope| registration.runtime_scope == Some(scope))
        && owner_matches
        && provider_ref_matches
        && provider_identity_matches
        && template_identity_matches
        && registration.guest_execution.as_ref() == guest_execution
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn rebind_guest_execution_registration(
    registration: &mut RunnerRegistration,
    requested: Option<&d2b_contracts_broker::broker_wire::GuestExecutionBinding>,
) -> Result<bool, BrokerError> {
    if requested.is_some_and(|binding| !binding.is_valid()) {
        return Err(BrokerError::LiveHandler(
            "guest runner execution binding changed".to_owned(),
        ));
    }
    if registration.guest_execution.as_ref() == requested {
        return Ok(false);
    }

    let (Some(current), Some(next)) = (registration.guest_execution.as_ref(), requested) else {
        return Err(BrokerError::LiveHandler(
            "guest runner execution binding changed".to_owned(),
        ));
    };
    if current.target_uid != next.target_uid
        || current.boot_identity_digest != next.boot_identity_digest
        || current.assignment_epoch != next.assignment_epoch
        || current.provider_generation != next.provider_generation
        || current.controller_generation != next.controller_generation
    {
        return Err(BrokerError::LiveHandler(
            "guest runner execution binding changed".to_owned(),
        ));
    }
    if next.session_generation <= current.session_generation {
        return Err(BrokerError::LiveHandler(
            "guest runner session generation is not newer".to_owned(),
        ));
    }

    registration.guest_execution = Some(next.clone());
    Ok(true)
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn remove_runner_metadata(runner_id: &str) {
    // Non-blocking try-lock (plan U8): a Busy collision skips the removal
    // exactly like the old poisoned path; the caller retries or the next
    // reap pass clears the entry.
    if let Ok(mut registry) = runner_metadata_registry().try_lock() {
        registry.remove(runner_id);
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn observe_registered_runner(
    request: &d2b_contracts_broker::broker_wire::ObserveRunnerRequest,
    intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent,
) -> Result<d2b_contracts_broker::broker_wire::ObserveRunnerResponse, BrokerError> {
    let cgroup_placement = private_cgroup_placement(
        &intent.cgroup_placement,
        request.vm_id.as_str(),
        request.runtime_scope,
        request.resource_ref.is_some(),
    )?;
    let runner_id = runner_registry_key(
        request.vm_id.as_str(),
        request.role_id.as_str(),
        request.resource_ref.as_ref(),
        request.resource_uid.as_ref(),
        request.zone_uid.as_ref(),
        request.runtime_scope,
    );
    let registered = async move {
        // Keep the lock order aligned with deregistration: pidfd registry
        // first, metadata second. This makes the binding update atomic with
        // the live pidfd registration.
let mut metadata_registry = runner_metadata_registry().try_lock().map_err(|_| {
            BrokerError::LiveHandler("runner metadata registry busy (tokio try-lock)".to_owned())
        })?;
        let registration = match metadata_registry.get(&runner_id).cloned() {
            Some(registration) => registration,
            None => return Ok(None),
        };
        let mut observed_registration = registration.clone();
        let rebound = rebind_guest_execution_registration(
            &mut observed_registration,
            request.guest_execution.as_ref(),
        )?;
        let pidfd_registered = runner_pidfds().contains_key(&runner_id);
        if !pidfd_registered
            || observed_registration.vm_id != request.vm_id.as_str()
            || observed_registration.role_id != request.role_id.as_str()
            || observed_registration.role != request.role
            || !registration_matches(
                &observed_registration,
                request.resource_ref.as_ref(),
                request.resource_uid.as_ref(),
                None,
                None,
                request.zone_uid.as_ref(),
                request.generation,
                request.runtime_scope,
                request.owner_ref.as_ref(),
                request.provider_ref.as_ref(),
                request.provider_identity,
                request.template_identity,
                request.guest_execution.as_ref(),
            )
            || observed_registration.bundle_runner_intent_ref
                != request.bundle_runner_intent_ref.as_str()
            || observed_registration.pid <= 0
            || observed_registration.start_time_ticks == 0
        {
            // The registration records what the spawn-process kernel
            // ACTUALLY ran (the daemon's resolved plan carried in the
            // payload); the broker's re-derived intent below can disagree
            // with that across the two bundle views, so the binary and
            // cgroup placement are NOT re-checked against the re-derived
            // intent (the same tolerance the discovery path applies via
            // the registered-binary preference, U10 seam). The identity
            // fields above still bind the registration to the request.
            Ok(None)
        } else {
            let Some(start_time_ticks) = read_proc_start_time_ticks(observed_registration.pid)?
            else {
                let pidfd = runner_pidfds().duplicate(&runner_id);
                drop(metadata_registry);
                return Ok(Some(
                    reap_registered_runner_after_observation(
                        request,
                        &runner_id,
                        &observed_registration,
                        pidfd,
                    ),
                ));
            };
            if start_time_ticks != observed_registration.start_time_ticks {
                return Err(BrokerError::LiveHandler(
                    "runner process identity changed".to_owned(),
                ));
            }
            let cgroup_verified = proc_cgroup_matches(
                observed_registration.pid,
                &observed_registration.cgroup_subtree,
            )
            .await;
            let executable_observation = observe_runner_executable(
                read_runner_executable(observed_registration.pid).await,
                &observed_registration.binary_path,
            )
            .await?;
            if executable_observation == RunnerExecutableObservation::Vanished {
                let pidfd = runner_pidfds().duplicate(&runner_id);
                drop(metadata_registry);
                return Ok(Some(
                    reap_registered_runner_after_observation(
                        request,
                        &runner_id,
                        &observed_registration,
                        pidfd,
                    ),
                ));
            }
            let Some(current_start_time_ticks) =
                read_proc_start_time_ticks(observed_registration.pid)?
            else {
                let pidfd = runner_pidfds().duplicate(&runner_id);
                drop(metadata_registry);
                return Ok(Some(
                    reap_registered_runner_after_observation(
                        request,
                        &runner_id,
                        &observed_registration,
                        pidfd,
                    ),
                ));
            };
            if current_start_time_ticks != start_time_ticks {
                return Err(BrokerError::LiveHandler(
                    "runner process identity changed".to_owned(),
                ));
            }
            let executable_verified =
                executable_observation.is_verified_for_registered(pidfd_registered, cgroup_verified);
            if pidfd_registered && cgroup_verified && executable_verified {
                tracing::debug!(
                    runner_id,
                    pid = observed_registration.pid,
                    "ObserveRunner found a verified registered runner",
                );
            } else {
                tracing::warn!(
                    runner_id,
                    pid = observed_registration.pid,
                    pidfd_registered,
                    cgroup_verified,
                    executable_verified,
                    "ObserveRunner registered runner verification is incomplete",
                );
            }
            if rebound {
                if let Some(stored) = metadata_registry.get_mut(&runner_id) {
                    stored.guest_execution = observed_registration.guest_execution.clone();
                } else {
                    return Err(BrokerError::LiveHandler(
                        "runner registration disappeared".to_owned(),
                    ));
                }
            }
            Ok(Some(
                d2b_contracts_broker::broker_wire::ObserveRunnerResponse {
                    vm_id: request.vm_id.clone(),
                    role_id: request.role_id.clone(),
                    present: true,
                    pid: observed_registration.pid,
                    start_time_ticks,
                    cgroup_verified,
                    executable_verified,
                },
            ))
        }
    }.await?;
    match registered {
        Some(response) => Ok(response),
        None => discover_runner_candidate(request, intent, &cgroup_placement.subtree).await,
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn discover_runner_candidate(
    request: &d2b_contracts_broker::broker_wire::ObserveRunnerRequest,
    intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent,
    cgroup_subtree: &str,
) -> Result<d2b_contracts_broker::broker_wire::ObserveRunnerResponse, BrokerError> {
    // The spawn-process kernel registered the ACTUAL binary it exec'd
    // (the daemon's resolved plan path). The broker's re-derived intent
    // below can disagree with that path across the two bundle views, so
    // the executable check prefers the registered spawn metadata when it
    // exists (U10 seam: the observation verifies what actually ran).
    let registered_binary = runner_metadata_registry()
        .try_lock()
        .map_err(|_| {
            BrokerError::LiveHandler("runner metadata registry busy (tokio try-lock)".to_owned())
        })?
        .get(&runner_registry_key(
            request.vm_id.as_str(),
            request.role_id.as_str(),
            request.resource_ref.as_ref(),
            request.resource_uid.as_ref(),
            request.zone_uid.as_ref(),
            request.runtime_scope.as_ref().copied(),
        ))
        .map(|registration| registration.binary_path.clone());
    let mut candidates = Vec::new();
    let mut entries = tokio::fs::read_dir("/proc")
        .await
        .map_err(|error| BrokerError::LiveHandler(error.to_string()))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| BrokerError::LiveHandler(error.to_string()))?
    {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|value| value.parse::<i32>().ok()) else {
            continue;
        };
        let cgroup_verified = pid > 0 && proc_cgroup_matches(pid, cgroup_subtree).await;
        if !cgroup_verified {
            continue;
        }
        let Some(start_time_ticks) = read_proc_start_time_ticks(pid)? else {
            continue;
        };
        if start_time_ticks == 0 {
            continue;
        }
        let observed_exe = read_runner_executable(pid).await;
        let expected_binary = registered_binary.as_deref().unwrap_or(&intent.binary_path);
        let executable_observation = observe_runner_executable(observed_exe, expected_binary).await?;
        if executable_observation == RunnerExecutableObservation::Vanished {
            continue;
        }
        let Some(current_start_time_ticks) = read_proc_start_time_ticks(pid)? else {
            continue;
        };
        if current_start_time_ticks != start_time_ticks {
            continue;
        }
        let executable_verified = executable_observation.is_verified_for_discovery();
        candidates.push((pid, start_time_ticks, executable_verified));
    }
    let Some((pid, start_time_ticks, executable_verified)) = select_runner_candidate(candidates)?
    else {
        return Ok(absent_runner_response(request));
    };
    Ok(d2b_contracts_broker::broker_wire::ObserveRunnerResponse {
        vm_id: request.vm_id.clone(),
        role_id: request.role_id.clone(),
        present: true,
        pid,
        start_time_ticks,
        cgroup_verified: true,
        executable_verified,
    })
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunnerExecutableObservation {
    Matching,
    Mismatch,
    PermissionDenied,
    Vanished,
}

#[cfg(not(feature = "layer1-bootstrap"))]
impl RunnerExecutableObservation {
    fn is_verified(self, broker_owned_provenance: bool) -> bool {
        match self {
            Self::Matching => true,
            Self::PermissionDenied => broker_owned_provenance,
            Self::Mismatch | Self::Vanished => false,
        }
    }

    fn is_verified_for_discovery(self) -> bool {
        self.is_verified(false)
    }

    fn is_verified_for_registered(self, pidfd_registered: bool, cgroup_verified: bool) -> bool {
        self.is_verified(pidfd_registered && cgroup_verified)
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn observe_runner_executable(
    executable: io::Result<PathBuf>,
    expected: &Path,
) -> Result<RunnerExecutableObservation, BrokerError> {
    match executable {
        Ok(path) => Ok(classify_executable_path(&path, expected).await),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(RunnerExecutableObservation::Vanished)
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            Ok(RunnerExecutableObservation::PermissionDenied)
        }
        Err(error) => Err(BrokerError::LiveHandler(error.to_string())),
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn classify_executable_path(actual: &Path, expected: &Path) -> RunnerExecutableObservation {
    let actual = match tokio::fs::canonicalize(actual).await {
        Ok(actual) => actual,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            return RunnerExecutableObservation::PermissionDenied;
        }
        Err(_) => return RunnerExecutableObservation::Mismatch,
    };
    let canon_expected = tokio::fs::canonicalize(expected).await;
    let Some(expected) = canon_expected.as_deref().ok().map(Path::to_path_buf) else {
        return RunnerExecutableObservation::Mismatch;
    };
    if actual == expected {
        return RunnerExecutableObservation::Matching;
    }

    let Ok(script) = tokio::fs::read_to_string(&expected).await else {
        return RunnerExecutableObservation::Mismatch;
    };
    if !script.starts_with("#!") {
        return RunnerExecutableObservation::Mismatch;
    }
    let mut target = None;
    for line in script.lines().map(str::trim) {
        let Some(exec) = line.strip_prefix("exec \"$here/") else {
            continue;
        };
        let Some((relative, _)) = exec.split_once('"') else {
            return RunnerExecutableObservation::Mismatch;
        };
        let relative_path = Path::new(relative);
        if relative.is_empty()
            || relative.contains('$')
            || relative_path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return RunnerExecutableObservation::Mismatch;
        }
        let Some(parent) = expected.parent() else {
            return RunnerExecutableObservation::Mismatch;
        };
        if target.replace(parent.join(relative_path)).is_some() {
            return RunnerExecutableObservation::Mismatch;
        }
    }
    let Some(canon_target) = target else {
        return RunnerExecutableObservation::Mismatch;
    };
    let Ok(canon_target) = tokio::fs::canonicalize(&canon_target).await else {
        return RunnerExecutableObservation::Mismatch;
    };
    if canon_target == actual {
        RunnerExecutableObservation::Matching
    } else {
        RunnerExecutableObservation::Mismatch
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn executable_paths_match(actual: &Path, expected: &Path) -> bool {
    classify_executable_path(actual, expected).await == RunnerExecutableObservation::Matching
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn present_unverified_runner_response(
    request: &d2b_contracts_broker::broker_wire::ObserveRunnerRequest,
    registration: &RunnerRegistration,
) -> d2b_contracts_broker::broker_wire::ObserveRunnerResponse {
    d2b_contracts_broker::broker_wire::ObserveRunnerResponse {
        vm_id: request.vm_id.clone(),
        role_id: request.role_id.clone(),
        present: true,
        pid: registration.pid,
        start_time_ticks: registration.start_time_ticks,
        cgroup_verified: false,
        executable_verified: false,
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn reap_registered_runner_after_observation(
    request: &d2b_contracts_broker::broker_wire::ObserveRunnerRequest,
    runner_id: &str,
    registration: &RunnerRegistration,
    pidfd: Option<OwnedFd>,
) -> d2b_contracts_broker::broker_wire::ObserveRunnerResponse {
    let Some(pidfd) = pidfd else {
        return present_unverified_runner_response(request, registration);
    };
    match targeted_reap_runner(runner_id, pidfd.as_fd()) {
        TargetedReapOutcome::Reaped | TargetedReapOutcome::AlreadyReaped => {
            absent_runner_response(request)
        }
        TargetedReapOutcome::StillAlive | TargetedReapOutcome::Failed => {
            present_unverified_runner_response(request, registration)
        }
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn absent_runner_response(
    request: &d2b_contracts_broker::broker_wire::ObserveRunnerRequest,
) -> d2b_contracts_broker::broker_wire::ObserveRunnerResponse {
    d2b_contracts_broker::broker_wire::ObserveRunnerResponse {
        vm_id: request.vm_id.clone(),
        role_id: request.role_id.clone(),
        present: false,
        pid: 0,
        start_time_ticks: 0,
        cgroup_verified: false,
        executable_verified: false,
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn select_runner_candidate(
    candidates: impl IntoIterator<Item = (i32, u64, bool)>,
) -> Result<Option<(i32, u64, bool)>, BrokerError> {
    let mut candidate = None;
    for next in candidates {
        if candidate.replace(next).is_some() {
            return Err(BrokerError::LiveHandler(
                "runner adoption candidate ambiguous".to_owned(),
            ));
        }
    }
    Ok(candidate)
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn read_runner_executable(pid: i32) -> io::Result<PathBuf> {
    tokio::fs::read_link(format!("/proc/{pid}/exe")).await
}

#[cfg(not(feature = "layer1-bootstrap"))]
// R11 inventory: `read_proc_start_time_ticks` is the shared /proc stat
// reader of the runner-identity surface. It stays synchronous because the
// sync reap-test scaffolding (plain `#[test]` fns driving `Command::spawn` /
// `kill` / `waitpid` around the pidfd registry) reads it without a runtime;
// the async discovery chain calls it as a bounded single-file read. The
// allow is the sanctioned synchronous-path class, not a blanket.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn read_proc_start_time_ticks(pid: i32) -> Result<Option<u64>, BrokerError> {
    let content = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(content) => content,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(BrokerError::LiveHandler(error.to_string())),
    };
    let close = content
        .trim_end_matches('\n')
        .rfind(')')
        .ok_or_else(|| BrokerError::LiveHandler("malformed proc stat".to_owned()))?;
    let mut fields = content[close + 1..].split_whitespace();
    let state = fields
        .next()
        .ok_or_else(|| BrokerError::LiveHandler("malformed proc stat".to_owned()))?;
    if matches!(state, "Z" | "X") {
        return Ok(None);
    }
    fields
        .nth(18)
        .ok_or_else(|| BrokerError::LiveHandler("malformed proc stat".to_owned()))?
        .parse::<u64>()
        .map(Some)
        .map_err(|_| BrokerError::LiveHandler("invalid proc start time".to_owned()))
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn proc_cgroup_matches(pid: i32, expected_subtree: &str) -> bool {
    if expected_subtree.is_empty() {
        return false;
    }
    let Ok(content) = tokio::fs::read_to_string(format!("/proc/{pid}/cgroup")).await else {
        return false;
    };
    let expected = expected_subtree.trim_start_matches('/');
    content.lines().any(|line| {
        let Some((_, path)) = line.split_once("::") else {
            return false;
        };
        let actual = path.trim_start_matches('/');
        actual == expected || actual.ends_with(&format!("/{expected}"))
    })
}

/// In-memory ring buffer for `ChildReaped` notifications.
/// Capped at 256 entries (oldest dropped on overflow). Protected by a
/// `tokio::sync::Mutex` (plan U8) so both the tokio reap task and the
/// synchronous dispatch workers can access it safely: the reap task pushes
/// through the async lock (a reap notification is never dropped), while
/// synchronous readers and writers use the non-blocking `try_lock` - the
/// sanctioned shape for sync readers, never a surviving `std::sync::Mutex`
/// (plan KD1). The critical sections are single deque operations, never
/// held across an await.
#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn child_reap_buffer() -> &'static tokio::sync::Mutex<
    std::collections::VecDeque<d2b_contracts_broker::broker_wire::ChildReapedNotification>,
> {
    use std::collections::VecDeque;

    static BUFFER: LazyLock<
        tokio::sync::Mutex<VecDeque<d2b_contracts_broker::broker_wire::ChildReapedNotification>>,
    > = LazyLock::new(|| tokio::sync::Mutex::new(VecDeque::with_capacity(256)));
    &BUFFER
}

#[cfg(not(feature = "layer1-bootstrap"))]
const CHILD_REAP_BUFFER_CAP: usize = 256;

/// Wake-up for waiters on the reap buffer (the reap tests' event-driven
/// waits): `notify_waiters` is called after every push, so a waiter parks
/// on the event instead of polling on a fixed cadence. A no-op when no
/// waiter is registered.
#[cfg(not(feature = "layer1-bootstrap"))]
static CHILD_REAP_NOTIFY: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Push one notification to the ring buffer under a held guard.
/// If the buffer is full, drops the oldest entry and logs a warning.
#[cfg(not(feature = "layer1-bootstrap"))]
fn push_child_reap_notification_locked(
    buf: &mut std::collections::VecDeque<
        d2b_contracts_broker::broker_wire::ChildReapedNotification,
    >,
    notif: d2b_contracts_broker::broker_wire::ChildReapedNotification,
) {
    if buf.len() >= CHILD_REAP_BUFFER_CAP {
        let dropped = buf.pop_front();
        tracing::warn!(
            dropped_runner_id = dropped
                .as_ref()
                .map(|d| d.runner_id.as_str())
                .unwrap_or("?"),
            "child_reap_buffer overflow: dropped oldest ChildReaped event"
        );
    }
    buf.push_back(notif);
    CHILD_REAP_NOTIFY.notify_waiters();
}

/// Push one notification from an async context (the SIGCHLD reap task).
/// Waits for the lock, so the primary reaper's notification is never
/// dropped under a concurrent reader.
#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) async fn push_child_reap_notification_async(
    notif: d2b_contracts_broker::broker_wire::ChildReapedNotification,
) {
    let mut buf = child_reap_buffer().lock().await;
    push_child_reap_notification_locked(&mut buf, notif);
}

/// Push one notification from a synchronous context (kernel handlers on
/// the dispatch workers). Non-blocking `try_lock`: a notification is
/// dropped with a warning only when another push/drain momentarily holds
/// the lock (sub-microsecond critical sections); the reap outcome is still
/// carried by the ReapChild response and the audit record.
#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn push_child_reap_notification(
    notif: d2b_contracts_broker::broker_wire::ChildReapedNotification,
) {
    let Ok(mut buf) = child_reap_buffer().try_lock() else {
        tracing::warn!("child_reap_buffer busy; dropping ChildReaped notification");
        return;
    };
    push_child_reap_notification_locked(&mut buf, notif);
}

/// Drain the ring buffer (used by PollChildReaped handler).
#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn drain_child_reap_buffer()
-> Vec<d2b_contracts_broker::broker_wire::ChildReapedNotification> {
    match child_reap_buffer().try_lock() {
        Ok(mut buf) => buf.drain(..).collect(),
        Err(_) => {
            tracing::warn!("child_reap_buffer busy; returning empty drain");
            Vec::new()
        }
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn register_runner_pidfd(runner_id: &str, pidfd: &OwnedFd) -> Result<(), BrokerError> {
    let duplicated = dup(pidfd.as_raw_fd())
        .map(owned_fd_from_raw)
        .map_err(|err| BrokerError::Protocol(format!("dup pidfd for {runner_id}: {err}")))?;
    runner_pidfds().insert(runner_id, duplicated)
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn remove_runner_registries(runner_id: &str) -> bool {
    // Keep the metadata ordering aligned with observation and deregistration
    // so reap cleanup cannot deadlock with a concurrent registry update; the
    // pidfd registry lives on the cell store with its own lock. The
    // registries are `tokio::sync` (plan U8): the non-blocking `try_lock`
    // skips a pass exactly like the old poisoned path, and the caller
    // retries (the SIGCHLD loop re-passes on its next signal, the targeted
    // reap reports Failed and the caller re-probes).
    let Ok(mut metadata_registry) = runner_metadata_registry().try_lock() else {
        return false;
    };
    runner_pidfds().remove(runner_id);
    metadata_registry.remove(runner_id);
    if let Ok(mut bootstrap_registry) = controller_bootstrap_registry().try_lock() {
        bootstrap_registry.remove(runner_id);
    }
    true
}

/// Refuse to start a SECOND live runner for an already-registered
/// `runner_id` (legacy `<vm>:<role>` or typed opaque runtime scope).
/// Checked BEFORE the child is spawned so a duplicate is never created:
/// rejecting AFTER the spawn would leak an
/// orphan child (a non-blocking targeted reap is a no-op for a live
/// process), and reaping that orphan would also pollute the existing
/// same-`runner_id` registry entry + push a spurious `ChildReaped` for the
/// live runner. A `runner_id` has at most one live registration - the
/// daemon serializes per-VM lifecycle (per-VM start flock + DAG) and a live
/// entry is cleared on exit (SIGCHLD reaper) or on down/stop - so a
/// legitimate re-spawn never collides here; a collision is a
/// concurrent/duplicate spawn and must fail closed. See issue #64
/// work-review (W1fu1/fu2).
#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn reserve_runner_id_for_spawn(runner_id: &str) -> Result<(), BrokerError> {
    let Some(pidfd) = runner_pidfds().get(runner_id) else {
        return Ok(());
    };
    // A reservation is a LIVE-process guard and must not outlive the runner
    // it names. The SIGCHLD loop reaps the child under its invocation key,
    // and the runner-id-keyed duplicate entry is not guaranteed to be
    // cleared by that same pass (the reap loop can be overtaken by the next
    // spawn reservation, and a registration can also outlive the child when
    // the runner dies outside the reap window). A registration whose
    // process is gone would otherwise refuse every relaunch forever - a
    // failed launch (for example a lost spawn reply) wedges the row with no
    // recovery.
    //
    // Liveness is decided from the runner's own registered pidfd - the
    // authoritative, generation-exact handle the broker holds for exactly
    // this process - never by re-deriving identity from /proc (a registered
    // start-time and a later /proc read are not comparable across the
    // child's own user/PID namespace in general). A live pidfd refuses a
    // duplicate spawn exactly as before; a pidfd whose process has exited -
    // or been reaped (ECHILD) - is a stale registration: evict it so the
    // stored child is relaunched instead of wedging the row. An
    // unreadable/unexpected pidfd state keeps the conservative refusal.
    use nix::errno::Errno;
    use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
    match waitid(
        Id::PIDFd(pidfd.as_fd()),
        WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG,
    ) {
        // A dead-but-unreaped child, or one reaped by the SIGCHLD loop:
        // reclaim the stale registration and let the relaunch proceed.
        Ok(WaitStatus::Exited(_, _)) | Ok(WaitStatus::Signaled(_, _, _)) => {
            tracing::warn!(
                runner_id,
                "reserve: reclaiming stale registration of a dead runner before relaunch"
            );
            remove_runner_registries(runner_id);
            return Ok(());
        }
        Err(Errno::ECHILD) => {
            tracing::warn!(
                runner_id,
                "reserve: reclaiming registration of an already-reaped runner before relaunch"
            );
            remove_runner_registries(runner_id);
            return Ok(());
        }
        // Any other outcome - a live child (StillAlive), an unreadable or
        // unexpected pidfd state - keeps the refusal.
        Ok(_) | Err(_) => {}
    }
    Err(BrokerError::Protocol(format!(
        "runner {runner_id} already has an active registration; refusing duplicate spawn"
    )))
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn signal_registered_runner(
    runner_id: &str,
    signal: d2b_contracts_broker::broker_wire::RunnerSignal,
) -> Result<(), BrokerError> {
    let pidfd = runner_pidfds()
        .get(runner_id)
        .ok_or_else(|| BrokerError::NoPidfd {
            runner_id: runner_id.to_owned(),
        })?;
    crate::sys::pidfd_sys::pidfd_send_signal(pidfd.as_fd(), runner_signal_number(signal))
        .map_err(|err| BrokerError::LiveHandler(format!("pidfd_send_signal({runner_id}): {err}")))
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn tracing_span_id_str(
    tracing_span_id: Option<&d2b_contracts::types::TracingSpanId>,
) -> Option<&str> {
    tracing_span_id.map(d2b_contracts::types::TracingSpanId::as_str)
}

/// Maps a classified [`crate::ops::store_sync_audit::ErrorStage`] to the
/// stable header `error_kind` slug recorded on the terminal StoreSync
/// failure/denial audit record (ADR 0027). The full signed shape lives in
/// `operation_fields`; this slug is the coarse, greppable category.
#[cfg(not(feature = "layer1-bootstrap"))]
fn store_sync_error_kind(stage: crate::ops::store_sync_audit::ErrorStage) -> &'static str {
    use crate::ops::store_sync_audit::ErrorStage;
    match stage {
        ErrorStage::None => "store-sync-failed",
        ErrorStage::Authz => "store-sync-authz-denied",
        ErrorStage::Lock => "store-sync-lock-failed",
        ErrorStage::Probe => "store-sync-probe-failed",
        ErrorStage::Verify => "store-sync-verify-failed",
        ErrorStage::Stage => "store-sync-stage-failed",
        ErrorStage::Rename => "store-sync-rename-failed",
        ErrorStage::Metadata => "store-sync-metadata-failed",
        ErrorStage::Integrity => "store-sync-integrity-failed",
        ErrorStage::CurrentSwap => "store-sync-current-swap-failed",
        ErrorStage::Marker => "store-sync-marker-failed",
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
trait DispatchBackend {
    /// The committed-operation envelope this process serves.
    ///
    /// The broker holds the committed rows; the handler code lives in the
    /// declaring crate's process. Implementations either serve the rows they
    /// hold handlers for or refuse them, so a registered handler is never
    /// authority and an unwired operation stays unreachable.
    fn operation_envelope(&self) -> &crate::envelope::BrokerEnvelope;

    fn apply_nftables<'a>(
        &'a self,
        resolver: &'a BundleResolver,
        intent: &'a d2b_core::bundle_resolver::ResolvedNftIntent,
        desired_hash: Option<&'a str>,
        destroy: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>>;

    fn apply_route<'a>(
        &'a self,
        state_dir: &'a Path,
        intent: &'a d2b_core::bundle_resolver::ResolvedRouteIntent,
        provenance: &'a d2b_contracts_resource::v3::NetworkProvenance,
        destroy: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>>;

    fn apply_sysctl<'a>(
        &'a self,
        intent: &'a d2b_core::bundle_resolver::ResolvedSysctlIntent,
        destroy: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>>;

    fn update_hosts_file<'a>(
        &'a self,
        intent: &'a d2b_core::bundle_resolver::ResolvedHostsIntent,
        destroy: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>>;

    fn apply_nm_unmanaged<'a>(
        &'a self,
        intent: &'a d2b_core::bundle_resolver::ResolvedNmUnmanagedIntent,
        destroy: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>>;

    fn set_bridge_port_flags<'a>(
        &'a self,
        req: &'a d2b_contracts_broker::broker_wire::SetBridgePortFlagsRequest,
        resolver: &'a BundleResolver,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<d2b_contracts_broker::broker_wire::BridgePortFlagsResponse, BrokerError>>
                + Send
                + 'a,
        >,
    >;

    fn open_pidfd<'a>(
        &'a self,
        runner_id: &'a str,
        pid: i32,
        expected_start_time_ticks: u64,
    ) -> Pin<Box<dyn Future<Output = Result<crate::live_handlers::OpenPidfdResult, BrokerError>> + Send + 'a>>;

    fn signal_runner<'a>(
        &'a self,
        runner_id: &'a str,
        signal: d2b_contracts_broker::broker_wire::RunnerSignal,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>>;

    fn spawn_runner<'a>(
        &'a self,
        runner_id: &'a str,
        plan_input: &'a crate::ops::spawn_runner::SpawnRunnerPlanInput,
        resolver: &'a BundleResolver,
        req: &'a d2b_contracts_broker::broker_wire::SpawnRunnerRequest,
        // Launch posture resolved from the trusted intent by the dispatch
        // arm; the backend must not re-derive it from the request.
        posture: LaunchPosture,
        // Device-owned worker scope resolved (and pinned) by the dispatch arm
        // from the verified bundle; `default()` for every other launch.
        device_worker: &'a crate::ops::device_worker::DeviceWorkerLaunch,
        request_fds: Vec<OwnedFd>,
        audit_log: &'a crate::audit::AuditLog,
    ) -> Pin<Box<dyn Future<Output = Result<crate::live_handlers::SpawnRunnerResult, BrokerError>> + Send + 'a>>;

    fn apply_host_generation_handoff<'a>(
        &'a self,
        state_dir: &'a std::path::Path,
        helper_path: &'a std::path::Path,
        request: &'a d2b_contracts_broker::host_generation::ApplyHostGenerationHandoff,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        d2b_contracts_broker::broker_wire::ApplyHostGenerationHandoffResponse,
                        BrokerError,
                    >,
                > + Send
                + 'a,
        >,
    >;

    fn usbip_bind<'a>(
        &'a self,
        intent: &'a d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>>;

    fn usbip_unbind<'a>(
        &'a self,
        intent: &'a d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>>;

    fn usbip_bind_firewall_rule<'a>(
        &'a self,
        resolver: &'a BundleResolver,
        intent: &'a d2b_core::bundle_resolver::ResolvedUsbipFirewallIntent,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>>;

    fn usbip_proxy_reconcile<'a>(
        &'a self,
        expectations: &'a [(String, String, PathBuf)],
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>>;

    fn qemu_media_system_powerdown<'a>(
        &'a self,
        req: &'a d2b_contracts_broker::broker_wire::QemuMediaLifecycleRequest,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        d2b_contracts_broker::broker_wire::QemuMediaLifecycleResponse,
                        BrokerError,
                    >,
                > + Send
                + 'a,
        >,
    >;

    fn qemu_media_query_status<'a>(
        &'a self,
        req: &'a d2b_contracts_broker::broker_wire::QemuMediaQueryStatusRequest,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        d2b_contracts_broker::broker_wire::QemuMediaQueryStatusResponse,
                        BrokerError,
                    >,
                > + Send
                + 'a,
        >,
    >;

    fn qemu_media_quit<'a>(
        &'a self,
        req: &'a d2b_contracts_broker::broker_wire::QemuMediaLifecycleRequest,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        d2b_contracts_broker::broker_wire::QemuMediaLifecycleResponse,
                        BrokerError,
                    >,
                > + Send
                + 'a,
        >,
    >;
}

#[cfg(not(feature = "layer1-bootstrap"))]
struct LiveDispatchBackend {
    daemon_uid: u32,
    daemon_gid: u32,
    profile: BrokerProfile,
    state_dir: PathBuf,
    /// Broker runtime root (the private socket's directory): the tree a
    /// Device-owned worker's per-Guest socket directory must strictly live
    /// under before the broker opens it to the worker's principal.
    runtime_root: PathBuf,
    /// The declaring process that serves family-owned operation handlers, as
    /// the server resolved it from its configuration.
    forward_socket_path: Option<PathBuf>,
}

#[cfg(not(feature = "layer1-bootstrap"))]
struct RunnerPreopenedFds {
    child_fds: Vec<std::os::fd::OwnedFd>,
    response_fds: Vec<std::os::fd::OwnedFd>,
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn prepare_runner_preopened_fds(
    _plan_input: &crate::ops::spawn_runner::SpawnRunnerPlanInput,
    resolver: &BundleResolver,
    req: &d2b_contracts_broker::broker_wire::SpawnRunnerRequest,
    audit_log: &crate::audit::AuditLog,
    daemon_uid: u32,
    daemon_gid: u32,
) -> Result<RunnerPreopenedFds, BrokerError> {
    if req.role == d2b_contracts_broker::broker_wire::RunnerRole::CloudHypervisor {
        let typed_resource = req.resource_ref.is_some()
            && req.resource_uid.is_some()
            && req.zone_uid.is_some()
            && req.runtime_scope.is_some();
        if typed_resource && req.network_tap_context.is_none() {
            return Ok(RunnerPreopenedFds {
                child_fds: Vec::new(),
                response_fds: Vec::new(),
            });
        }
        let runner_intent = resolver
            .find_runner_intent(req.bundle_runner_intent_ref.as_str())
            .ok_or_else(|| BrokerError::BundleIntentMissing {
                kind: "runner",
                intent_id: req.bundle_runner_intent_ref.as_str().to_owned(),
            })?;
        let intents = resolver
            .resolve_macvtap_intents(req.vm_id.as_str(), runner_intent.role_id.as_str())
            .map_err(|error| BrokerError::LiveHandler(error.to_string()))?;
        if intents.is_empty() {
            return Ok(RunnerPreopenedFds {
                child_fds: Vec::new(),
                response_fds: Vec::new(),
            });
        }
        let mut child_fds = Vec::with_capacity(intents.len());
        for (offset, intent) in intents.into_iter().enumerate() {
            // The inherited-fd contract assigns each macvtap intent the fd
            // RENDER_NODE_INHERITED_FD + index, in declaration order.
            let expected_fd =
                crate::sys::pidfd_sys::RENDER_NODE_INHERITED_FD + offset as libc::c_int;
            if intent.fd != expected_fd {
                return Err(BrokerError::LiveHandler(format!(
                    "macvtap fd contract mismatch for {}: processes.json declares fd {}, broker would install fd {}",
                    intent.ifname.as_str(),
                    intent.fd,
                    expected_fd
                )));
            }
            let fd = crate::ops::tap::live_create_macvtap_fd(&intent)
                .await
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            child_fds.push(fd);
        }
        let _ = audit_log;
        let _ = daemon_uid;
        let _ = daemon_gid;
        return Ok(RunnerPreopenedFds {
            child_fds,
            response_fds: Vec::new(),
        });
    }

    if req.role != d2b_contracts_broker::broker_wire::RunnerRole::QemuMedia {
        return Ok(RunnerPreopenedFds {
            child_fds: Vec::new(),
            response_fds: Vec::new(),
        });
    }

    let tap_context = req
        .network_tap_context
        .as_ref()
        .ok_or(BrokerError::RequestValidation {
            operation: "CreateTapFd",
            reason: "network-admission-required",
        })?;
    let tap_role_id =
        d2b_core::bundle_resolver::canonical_tap_role_id(req.role_id.as_str()).to_owned();
    let exec = crate::ops::exec_reconcile::SystemLiveExec::new(daemon_uid, daemon_gid);
    let outcome = crate::ops::tap::live_create_tap_fd(
        &exec,
        resolver,
        &d2b_contracts_broker::broker_wire::CreateTapFdRequest {
            vm_id: req.vm_id.clone(),
            role_id: d2b_contracts::types::RoleId::new(tap_role_id.clone()),
            bundle_tap_intent_ref: d2b_contracts::types::BundleOpId::new(
                d2b_core::bundle_resolver::intent_id_network_tap(
                    &tap_context.zone_uid,
                    &tap_context.network_uid,
                    &tap_context.attachment_id,
                    (
                        tap_context.network_generation,
                        tap_context.attachment_generation,
                    ),
                    &tap_context.bundle_generation,
                    &tap_role_id,
                    req.vm_id.as_str(),
                ),
            ),
            attachment_id: tap_context.attachment_id.clone(),
            network_generation: tap_context.network_generation,
            attachment_generation: tap_context.attachment_generation,
            zone_uid: tap_context.zone_uid.clone(),
            network_uid: tap_context.network_uid.clone(),
            bundle_generation: tap_context.bundle_generation.clone(),
            admitted_interface_names: tap_context.admitted_interface_names.clone(),
            tracing_span_id: req.tracing_span_id.clone(),
        },
        Some(audit_log),
    )
    .await
    .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;

    let reconcile = crate::ops::exec_reconcile::SystemReconcileExecutor;
    let _bridge_flags = dispatch_set_bridge_port_flags_inner(
        &d2b_contracts_broker::broker_wire::SetBridgePortFlagsRequest {
            vm_id: req.vm_id.clone(),
            role_id: d2b_contracts::types::RoleId::new("workload-lan"),
            network_tap_context: req.network_tap_context.clone(),
            tracing_span_id: req.tracing_span_id.clone(),
        },
        resolver,
        &reconcile,
    )
    .await?;

    let tap_fd = outcome
        .fd
        .ok_or_else(|| BrokerError::LiveHandler("qemu-media tap fd missing".to_owned()))?;
    let (qemu_console_fd, daemon_console_fd) = socketpair(
        AddressFamily::Unix,
        SockType::Stream,
        None,
        SockFlag::SOCK_CLOEXEC | SockFlag::SOCK_NONBLOCK,
    )
    .map_err(|err| BrokerError::LiveHandler(format!("qemu-media console socketpair: {err}")))?;
    Ok(RunnerPreopenedFds {
        child_fds: vec![tap_fd, qemu_console_fd],
        response_fds: vec![daemon_console_fd],
    })
}

#[cfg(not(feature = "layer1-bootstrap"))]
impl DispatchBackend for LiveDispatchBackend {
    fn operation_envelope(&self) -> &crate::envelope::BrokerEnvelope {
        // The envelope is installed once at serve time with the kernel
        // seam's handlers captured over the fixed process config (U10).
        live_operation_envelope()
    }

    fn apply_nftables<'a>(
        &'a self,
        resolver: &'a BundleResolver,
        intent: &'a d2b_core::bundle_resolver::ResolvedNftIntent,
        desired_hash: Option<&'a str>,
        destroy: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
        Box::pin(async move {
            let nft_binary = nft_binary_path();
            let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
            let destroy_script;
            let script_body = if destroy {
                destroy_script = render_nft_destroy_script(
                    &resolver.host.nftables.family,
                    &resolver.host.nftables.table,
                );
                destroy_script.as_str()
            } else {
                intent.script_body.as_str()
            };
            let persisted_hash = if destroy {
                None
            } else {
                persisted_nft_hash()
                    .await
                    .map_err(|err| BrokerError::LiveHandler(err.to_string()))?
                    .or_else(|| resolver.host.nftables.table_hash_after_apply.clone())
            };
            let expected_hash = if destroy {
                None
            } else {
                desired_hash.or(persisted_hash.as_deref())
            };
            let _new_hash = crate::ops::nft::apply_with_coexistence(
                &exec,
                &nft_binary,
                script_body,
                intent.ownership_id.as_str(),
                resolver.host.firewall_coexistence_policy.as_ref(),
                expected_hash,
            )
            .await
            .map_err(|err| match err {
                crate::ops::nft::ApplyWithCoexistenceError::CoexistenceRefused {
                    manager,
                    rationale,
                } => BrokerError::CoexistenceRefused { manager, rationale },
                crate::ops::nft::ApplyWithCoexistenceError::ParseFailed(err) => {
                    BrokerError::NftScriptParseFailed(err.to_string())
                }
                crate::ops::nft::ApplyWithCoexistenceError::CarveoutOrderingViolation(err) => {
                    BrokerError::CarveoutOrderingViolation(match err {
                        d2b_host::nftables::NftError::ForeignNftRuleShadowsD2b { details } => {
                            details
                        }
                        other => other.to_string(),
                    })
                }
                crate::ops::nft::ApplyWithCoexistenceError::DriftDetected {
                    expected,
                    observed,
                } => BrokerError::NftablesDriftDetected { expected, observed },
                crate::ops::nft::ApplyWithCoexistenceError::ForeignOwnership => {
                    BrokerError::LiveHandler("foreign-nft-ownership".to_owned())
                }
                crate::ops::nft::ApplyWithCoexistenceError::ReconcileExec(err) => {
                    BrokerError::LiveHandler(err.to_string())
                }
            })?;
            crate::ops::nft::persist_live_nft_hash(
                &exec,
                &nft_binary,
                &resolver.host.nftables.family,
                &resolver.host.nftables.table,
                &nft_hash_sidecar_path(),
            )
            .await
            .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            Ok(())
        })
    }

    fn apply_route<'a>(
        &'a self,
        state_dir: &'a Path,
        intent: &'a d2b_core::bundle_resolver::ResolvedRouteIntent,
        provenance: &'a d2b_contracts_resource::v3::NetworkProvenance,
        destroy: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
        Box::pin(async move {
            let ip_binary = ip_binary_path();
            let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
            crate::ops::route::apply_with_preflight_owned(
                &exec, &ip_binary, state_dir, intent, provenance, destroy,
            )
            .await
            .map_err(|err| BrokerError::LiveHandler(err.to_string()))
        })
    }

    fn apply_sysctl<'a>(
        &'a self,
        intent: &'a d2b_core::bundle_resolver::ResolvedSysctlIntent,
        destroy: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
        Box::pin(async move {
            let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
            let value = if destroy {
                destroy_sysctl_value(&intent.key)?
            } else {
                intent.value.as_str()
            };
            crate::ops::sysctl::apply_with_readback(&exec, &intent.key, value)
                .await
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))
        })
    }

    fn update_hosts_file<'a>(
        &'a self,
        intent: &'a d2b_core::bundle_resolver::ResolvedHostsIntent,
        destroy: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
        Box::pin(async move {
            let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
            if destroy {
                crate::ops::hosts::remove_marker_block(&exec, intent)
                    .await
                    .map_err(|err| BrokerError::LiveHandler(err.to_string()))
            } else {
                crate::ops::hosts::write_marker_block(&exec, intent)
                    .await
                    .map_err(|err| BrokerError::LiveHandler(err.to_string()))
            }
        })
    }

    fn apply_nm_unmanaged<'a>(
        &'a self,
        intent: &'a d2b_core::bundle_resolver::ResolvedNmUnmanagedIntent,
        destroy: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
        Box::pin(async move {
            let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
            if destroy {
                crate::ops::nm::remove_with_reload(intent)
                    .await
                    .map_err(|err| BrokerError::LiveHandler(err.to_string()))
            } else {
                crate::ops::nm::apply_with_reload(&exec, intent)
                    .await
                    .map_err(|err| BrokerError::LiveHandler(err.to_string()))
            }
        })
    }

    fn set_bridge_port_flags<'a>(
        &'a self,
        req: &'a d2b_contracts_broker::broker_wire::SetBridgePortFlagsRequest,
        resolver: &'a BundleResolver,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<d2b_contracts_broker::broker_wire::BridgePortFlagsResponse, BrokerError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
            dispatch_set_bridge_port_flags_inner(req, resolver, &exec).await
        })
    }

    fn open_pidfd<'a>(
        &'a self,
        runner_id: &'a str,
        pid: i32,
        expected_start_time_ticks: u64,
    ) -> Pin<Box<dyn Future<Output = Result<crate::live_handlers::OpenPidfdResult, BrokerError>> + Send + 'a>> {
        Box::pin(async move {
            let outcome = crate::live_handlers::live_open_pidfd(pid, expected_start_time_ticks)
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            register_runner_pidfd(runner_id, &outcome.pidfd)?;
            Ok(outcome)
        })
    }

    fn signal_runner<'a>(
        &'a self,
        runner_id: &'a str,
        signal: d2b_contracts_broker::broker_wire::RunnerSignal,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
        Box::pin(async move { signal_registered_runner(runner_id, signal) })
    }

    fn spawn_runner<'a>(
        &'a self,
        runner_id: &'a str,
        plan_input: &'a crate::ops::spawn_runner::SpawnRunnerPlanInput,
        resolver: &'a BundleResolver,
        req: &'a d2b_contracts_broker::broker_wire::SpawnRunnerRequest,
        posture: LaunchPosture,
        device_worker: &'a crate::ops::device_worker::DeviceWorkerLaunch,
        mut request_fds: Vec<OwnedFd>,
        audit_log: &'a crate::audit::AuditLog,
    ) -> Pin<Box<dyn Future<Output = Result<crate::live_handlers::SpawnRunnerResult, BrokerError>> + Send + 'a>> {
        Box::pin(async move {
        // Reserve the runner_id BEFORE spawning the child: refuse a
        // duplicate active registration up front so we never create an
        // orphan child (see `reserve_runner_id_for_spawn`).
        #[cfg(not(feature = "layer1-bootstrap"))]
        reserve_runner_id_for_spawn(runner_id)?;
        let preopened = prepare_runner_preopened_fds(
            plan_input,
            resolver,
            req,
            audit_log,
            self.daemon_uid,
            self.daemon_gid,
        )
        .await?;
        // The escrow descriptor is retained as registry custody only: no
        // duplicate rides the answer leg (a dup of the caller's own
        // descriptor is refused by the forward carrier's anti-replay
        // fence with the fd-leg code; the daemon keeps its own copy of
        // the daemon end to wait on).
        let retained_controller_bootstrap = if posture.carries_controller_escrow() {
            let retained = request_fds.pop().ok_or_else(|| {
                BrokerError::Protocol(
                    "ProviderController bootstrap escrow fd is missing".to_owned(),
                )
            })?;
            Some(retained)
        } else {
            None
        };
        if !request_fds.is_empty() && !preopened.child_fds.is_empty() {
            return Err(BrokerError::Protocol(
                "request inherited fds cannot combine with broker-preopened fds".to_owned(),
            ));
        }
        let swtpm_identity = resource_backed_swtpm_identity(resolver, req, device_worker);
        let mut outcome = crate::live_handlers::live_spawn_runner(
            plan_input,
            preopened.child_fds,
            request_fds,
            req.activation_input.as_ref(),
            &self.state_dir,
            swtpm_identity.as_ref(),
            device_worker,
            &self.runtime_root,
        )
        .await
        .map_err(|err| {
            // Log the actual LiveHandlerError detail before wrapping it
            // in the opaque BrokerError::LiveHandler envelope so
            // operators can see WHY the spawn failed in journalctl.
            tracing::error!(
                runner_id = %runner_id,
                error = %err,
                "live_spawn_runner failed"
            );
            // swtpm-dir hardening fail-closed carries a structured,
            // path-free audit that the dispatch arm must emit as a
            // terminal PrepareSwtpmDir record; preserve it instead of
            // collapsing to the opaque LiveHandler envelope.
            match err {
                crate::live_handlers::LiveHandlerError::SwtpmDirHardening { audit, reason } => {
                    BrokerError::SwtpmDirHardening { audit, reason }
                }
                other => BrokerError::LiveHandler(other.to_string()),
            }
        })?;
        outcome.extra_response_fds = preopened.response_fds;
        register_runner_pidfd(runner_id, &outcome.pidfd).inspect_err(|_err| {
            // Registration failed: the broker is about to drop this
            // just-spawned child's pidfd. Reap it now (targeted,
            // non-blocking) so a child that has already exited cannot
            // leak as a zombie. Best-effort; the registry entry is
            // already absent on the failure path.
            #[cfg(not(feature = "layer1-bootstrap"))]
            targeted_reap_runner(runner_id, outcome.pidfd.as_fd());
        })?;
        if let Some(bootstrap) = retained_controller_bootstrap {
            controller_bootstrap_registry()
                .try_lock()
                .map_err(|_| {
                    BrokerError::Protocol(
                        "controller bootstrap registry busy (tokio try-lock)".to_owned(),
                    )
                })?
                .insert(runner_id.to_owned(), bootstrap);
        }
        // Close the registration-window race: if the child exited
        // between clone3 and the registry insertion above, its SIGCHLD
        // may have already been coalesced/consumed by a reap pass that
        // ran before the entry existed. A targeted, generation-exact
        // (pidfd-keyed) non-blocking reap here guarantees the child is
        // reaped regardless of SIGCHLD timing.
        #[cfg(not(feature = "layer1-bootstrap"))]
        targeted_reap_runner(runner_id, outcome.pidfd.as_fd());
        Ok(outcome)
        })
    }

    fn apply_host_generation_handoff<'a>(
        &'a self,
        state_dir: &'a std::path::Path,
        helper_path: &'a std::path::Path,
        request: &'a d2b_contracts_broker::host_generation::ApplyHostGenerationHandoff,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        d2b_contracts_broker::broker_wire::ApplyHostGenerationHandoffResponse,
                        BrokerError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            crate::ops::host_generation_handoff::apply_with_helper(
                state_dir,
                helper_path,
                request,
            )
            .await
            .map_err(|error| BrokerError::LiveHandler(error.to_string()))
        })
    }

    fn usbip_bind<'a>(
        &'a self,
        intent: &'a d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
        Box::pin(async move {
            let usbip_binary = usbip_binary_path();
            let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
            crate::live_handlers::live_usbip_bind(
                &exec,
                &usbip_binary,
                usb_device_sysfs_root(),
                &intent.bus_id,
                &intent.lock_path,
                &intent.vm_name,
                self.daemon_uid,
                self.daemon_gid,
            )
            .await
            .map_err(|err| BrokerError::LiveHandler(err.to_string()))
        })
    }

    fn usbip_unbind<'a>(
        &'a self,
        intent: &'a d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
        Box::pin(async move {
            let usbip_binary = usbip_binary_path();
            let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
            crate::live_handlers::live_usbip_unbind(
                &exec,
                &usbip_binary,
                usb_device_sysfs_root(),
                &intent.bus_id,
                &intent.lock_path,
                &intent.vm_name,
            )
            .await
            .map_err(|err| BrokerError::LiveHandler(err.to_string()))
        })
    }

    fn usbip_bind_firewall_rule<'a>(
        &'a self,
        resolver: &'a BundleResolver,
        intent: &'a d2b_core::bundle_resolver::ResolvedUsbipFirewallIntent,
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
        Box::pin(async move {
            let host_nft_intent = resolver
                .find_nft_intent(&d2b_core::bundle_resolver::intent_id_nft_host())
                .ok_or_else(|| BrokerError::BundleIntentMissing {
                    kind: "nft",
                    intent_id: d2b_core::bundle_resolver::intent_id_nft_host(),
                })?;
            let decision = build_usbip_firewall_decision(resolver, host_nft_intent, intent)
                .await?;
            let nft_binary = nft_binary_path();
            let exec = crate::ops::exec_reconcile::SystemReconcileExecutor;
            let nft_script = decision.batch.render_nft_script();
            let expected_hash = persisted_nft_hash()
                .await
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))?
                .or_else(|| resolver.host.nftables.table_hash_after_apply.clone());
            crate::ops::nft::apply_with_coexistence(
                &exec,
                &nft_binary,
                &nft_script,
                resolver.host.nftables.ownership_id.as_str(),
                resolver.host.firewall_coexistence_policy.as_ref(),
                expected_hash.as_deref(),
            )
            .await
            .map_err(|err| match err {
                crate::ops::nft::ApplyWithCoexistenceError::CoexistenceRefused {
                    manager,
                    rationale,
                } => BrokerError::CoexistenceRefused { manager, rationale },
                crate::ops::nft::ApplyWithCoexistenceError::ParseFailed(err) => {
                    BrokerError::NftScriptParseFailed(err.to_string())
                }
                crate::ops::nft::ApplyWithCoexistenceError::CarveoutOrderingViolation(err) => {
                    BrokerError::CarveoutOrderingViolation(match err {
                        d2b_host::nftables::NftError::ForeignNftRuleShadowsD2b { details } => {
                            details
                        }
                        other => other.to_string(),
                    })
                }
                crate::ops::nft::ApplyWithCoexistenceError::DriftDetected {
                    expected,
                    observed,
                } => BrokerError::NftablesDriftDetected { expected, observed },
                crate::ops::nft::ApplyWithCoexistenceError::ForeignOwnership => {
                    BrokerError::LiveHandler("foreign-nft-ownership".to_owned())
                }
                crate::ops::nft::ApplyWithCoexistenceError::ReconcileExec(err) => {
                    BrokerError::LiveHandler(err.to_string())
                }
            })?;
            crate::ops::nft::persist_live_nft_hash(
                &exec,
                &nft_binary,
                &resolver.host.nftables.family,
                &resolver.host.nftables.table,
                &nft_hash_sidecar_path(),
            )
            .await
            .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
            Ok(())
        })
    }

    fn usbip_proxy_reconcile<'a>(
        &'a self,
        expectations: &'a [(String, String, PathBuf)],
    ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
        Box::pin(async move {
            crate::live_handlers::live_usbip_proxy_reconcile(expectations)
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))
        })
    }

    fn qemu_media_system_powerdown<'a>(
        &'a self,
        req: &'a d2b_contracts_broker::broker_wire::QemuMediaLifecycleRequest,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        d2b_contracts_broker::broker_wire::QemuMediaLifecycleResponse,
                        BrokerError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            crate::ops::media::system_powerdown(req)
                .await
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))
        })
    }

    fn qemu_media_query_status<'a>(
        &'a self,
        req: &'a d2b_contracts_broker::broker_wire::QemuMediaQueryStatusRequest,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        d2b_contracts_broker::broker_wire::QemuMediaQueryStatusResponse,
                        BrokerError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            crate::ops::media::query_status(req)
                .await
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))
        })
    }

    fn qemu_media_quit<'a>(
        &'a self,
        req: &'a d2b_contracts_broker::broker_wire::QemuMediaLifecycleRequest,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        d2b_contracts_broker::broker_wire::QemuMediaLifecycleResponse,
                        BrokerError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            crate::ops::media::quit(req)
                .await
                .map_err(|err| BrokerError::LiveHandler(err.to_string()))
        })
    }
}

/// The runtime one request's dispatch chain from the synchronous dispatch
/// pool runs on.
///
/// The dispatch pool workers are plain threads with no executor, while the
/// dispatch chain is async end-to-end (the backend trait methods and the
/// dispatch fns await in async time): the forward leg dials and exchanges
/// frames under the forwarder's round-trip budget, the subprocess probes
/// wait in async time, and the nested kernel calls dispatch on this
/// runtime's workers. A pool job bridges the boundary with `block_on` on
/// this dedicated runtime instead of stalling a dispatch worker's executor,
/// which has no reactor by construction. The set is built the way the
/// broker's other runtimes are (`enable_all`, capped workers), and exists
/// only when a committed operation's arm actually calls the envelope.
#[cfg(not(feature = "layer1-bootstrap"))]
static ENVELOPE_CALL_RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> =
    std::sync::LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            // Bounded parallelism: forwarded legs wait on the daemon's
            // answer while their nested kernel calls must be dispatched on
            // other workers; two workers serialized those and every
            // forward+nest pair stalled for the full io timeout (the
            // host-integration lane proved it).
            .worker_threads(8)
            .thread_name("d2b-broker-envelope-call")
            .enable_all()
            .build()
            .expect("broker envelope-call runtime")
    });

#[cfg(not(feature = "layer1-bootstrap"))]
fn envelope_call_runtime() -> &'static tokio::runtime::Runtime {
    &ENVELOPE_CALL_RUNTIME
}

/// The runtime the layer1-bootstrap wire's dispatch chain runs on.
///
/// The bootstrap wire's dispatch body is synchronous (every live arm is an
/// `Unimplemented` refusal), but it shares the async `answer_request` shape
/// with the production wire, so its pool jobs bridge the same way on a
/// single-thread runtime.
#[cfg(feature = "layer1-bootstrap")]
static BOOTSTRAP_DISPATCH_RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> =
    std::sync::LazyLock::new(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("bootstrap dispatch runtime")
    });

#[cfg(feature = "layer1-bootstrap")]
fn bootstrap_dispatch_runtime() -> &'static tokio::runtime::Runtime {
    &BOOTSTRAP_DISPATCH_RUNTIME
}

/// The committed-operation envelope of this broker instance.
///
/// The rows are the committed catalog. The dispatch step answers per
/// operation from the mixed kernel seam (U10): the broker-generic kernel
/// rows run in-broker on the kernel handler table, and every other
/// committed row forwards to the declaring process that serves it. The
/// envelope is built once at serve time from the fixed process config
/// (runtime input, so a plain `OnceLock` cell rather than a `LazyLock`),
/// and an invocation identifier is unique across the instance's lifetime.
#[cfg(not(feature = "layer1-bootstrap"))]
static LIVE_OPERATION_ENVELOPE: OnceLock<crate::envelope::BrokerEnvelope> = OnceLock::new();

/// Install the envelope at serve time, before any connection is accepted.
/// The production chain-audit sink: every in-broker envelope leg's record
/// lands in the same daily audit log the typed dispatch arms write.
#[cfg(not(feature = "layer1-bootstrap"))]
struct AuditLogChainSink {
    log: Arc<AuditLog>,
}

#[cfg(not(feature = "layer1-bootstrap"))]
impl d2b_audit::evidence_chain::ChainAuditSink for AuditLogChainSink {
    fn record(&self, record: &d2b_audit::evidence_chain::ChainRecord) -> std::io::Result<()> {
        self.log.write_chain_record(record)
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn install_live_operation_envelope(
    config: &ServerConfig,
    audit_log: &Arc<AuditLog>,
) -> Result<(), RunError> {
    let profile = match config.profile {
        BrokerProfile::Host => crate::catalog::BrokerProfileId::Host,
        BrokerProfile::Guest => crate::catalog::BrokerProfileId::Guest,
    };
    let forwarder = match config.forward_socket_path.as_deref() {
        Some(path) => crate::envelope::ForwardingDispatcher::new(
            crate::forwarding::SocketForwarder::new(path),
        ),
        // No peer is configured: every forwarded operation refuses rather
        // than being served by a process that does not declare it.
        None => crate::envelope::ForwardingDispatcher::default(),
    };
    let kernels = crate::kernel_ops::kernel_table(crate::kernel_ops::KernelConfig {
        state_dir: config.state_dir.clone(),
        runtime_root: config
            .socket_path
            .parent()
            .unwrap_or_else(|| Path::new(DEFAULT_BROKER_RUNTIME_DIR))
            .to_path_buf(),
        daemon_uid: config.d2bd_uid,
        daemon_gid: config.d2bd_gid,
        bundle_path: config.bundle_path.clone(),
    });
    let dispatcher = crate::envelope::KernelDispatcher::new(kernels, forwarder);
    let envelope = crate::envelope::BrokerEnvelope::over(profile, Box::new(dispatcher))
        .commit_forwarded()
        .with_chain_audit(Arc::new(AuditLogChainSink {
            log: Arc::clone(audit_log),
        }))
        .build();
    LIVE_OPERATION_ENVELOPE
        .set(envelope)
        .map_err(|_| RunError::Protocol("live envelope installed twice".to_owned()))
}

/// The envelope this process installed at serve time.
#[cfg(not(feature = "layer1-bootstrap"))]
fn live_operation_envelope() -> &'static crate::envelope::BrokerEnvelope {
    LIVE_OPERATION_ENVELOPE
        .get()
        .expect("live envelope installed at serve time before any dispatch")
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn require_resolver_ref(resolver: Option<&BundleResolver>) -> Result<&BundleResolver, BrokerError> {
    resolver.ok_or(BrokerError::BundleResolverUnavailable)
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn require_resolver(
    resolver: Option<&Arc<BundleResolver>>,
) -> Result<&Arc<BundleResolver>, BrokerError> {
    resolver.ok_or(BrokerError::BundleResolverUnavailable)
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn live_exec(config: &ServerConfig) -> crate::ops::exec_reconcile::SystemLiveExec {
    crate::ops::exec_reconcile::SystemLiveExec::new(config.d2bd_uid, config.d2bd_gid)
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) async fn dispatch_set_bridge_port_flags_inner(
    req: &d2b_contracts_broker::broker_wire::SetBridgePortFlagsRequest,
    resolver: &BundleResolver,
    executor: &dyn crate::ops::exec_reconcile::ReconcileExecutor,
) -> Result<d2b_contracts_broker::broker_wire::BridgePortFlagsResponse, BrokerError> {
    let context = req
        .network_tap_context
        .as_ref()
        .ok_or(BrokerError::RequestValidation {
            operation: "SetBridgePortFlags",
            reason: "network-admission-required",
        })?;
    let installed =
        resolver
            .installed_generation_identity()
            .ok_or(BrokerError::RequestValidation {
                operation: "SetBridgePortFlags",
                reason: "installed-generation-unavailable",
            })?;
    if installed.as_str() != context.bundle_generation.as_str()
        || context.network_generation.get() == 0
        || context.attachment_generation.get() == 0
    {
        return Err(BrokerError::RequestValidation {
            operation: "SetBridgePortFlags",
            reason: "stale-projection-generation",
        });
    }
    crate::ops::tap::live_set_bridge_port_flags(executor, resolver, req)
        .await
        .map_err(|err| BrokerError::LiveHandler(err.to_string()))
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn nft_binary_path() -> PathBuf {
    PathBuf::from(env::var("D2B_BROKER_NFT_BINARY").unwrap_or_else(|_| "/usr/sbin/nft".to_owned()))
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn nft_hash_sidecar_path() -> PathBuf {
    PathBuf::from(
        env::var("D2B_BROKER_NFT_HASH_PATH")
            .unwrap_or_else(|_| crate::ops::nft::DEFAULT_NFT_HASH_SIDECAR_PATH.to_owned()),
    )
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) async fn persisted_nft_hash()
-> Result<Option<String>, crate::ops::exec_reconcile::ReconcileExecError> {
    crate::ops::nft::read_persisted_nft_hash(&nft_hash_sidecar_path()).await
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn ip_binary_path() -> PathBuf {
    PathBuf::from(env::var("D2B_BROKER_IP_BINARY").unwrap_or_else(|_| "/usr/sbin/ip".to_owned()))
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn render_nft_destroy_script(family: &str, table: &str) -> String {
    format!("table {family} {table} {{\n}}\n")
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn destroy_sysctl_value(key: &str) -> Result<&'static str, BrokerError> {
    crate::ops::sysctl::destroy_value_for_key(key)
        .ok_or_else(|| BrokerError::Protocol(format!("unsupported host-destroy sysctl key: {key}")))
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn usbip_binary_path() -> PathBuf {
    PathBuf::from(
        env::var("D2B_BROKER_USBIP_BINARY").unwrap_or_else(|_| "/usr/sbin/usbip".to_owned()),
    )
}

/// Best-effort lookup of the human-readable VM name carried in the
/// bundle's `processes.vms[*].vm` list. The wire `VmId` is a transparent
/// opaque string; the bundle index is the `processes.vms[*].vm` field.
/// We use the wire value as both the opaque key and the human-readable
/// name today - the daemon emits them identically.
#[cfg(not(feature = "layer1-bootstrap"))]
fn lookup_vm_name(_resolver: &Arc<BundleResolver>, vm_id: &d2b_contracts::types::VmId) -> String {
    vm_id.as_str().to_owned()
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn apply_vm_start_prerequisites(
    resolver: &BundleResolver,
    vm_name: &str,
    role_id: &str,
) -> Result<(), BrokerError> {
    for intent in resolver.resolve_vm_start_prerequisites(vm_name, role_id) {
        for action in &intent.actions {
            execute_vm_start_action(&intent, action)?;
        }
    }
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn execute_vm_start_action(
    intent: &d2b_core::bundle_resolver::ResolvedVmStartIntent,
    action: &d2b_core::bundle_resolver::ResolvedVmStartAction,
) -> Result<(), BrokerError> {
    match action {
        d2b_core::bundle_resolver::ResolvedVmStartAction::PrepareRuntimeDir(dir)
        | d2b_core::bundle_resolver::ResolvedVmStartAction::PrepareStateDir(dir) => {
            crate::ops::state_dir::prepare_dir(&crate::ops::state_dir::PrepareDirRequest {
                kind: if matches!(
                    action,
                    d2b_core::bundle_resolver::ResolvedVmStartAction::PrepareRuntimeDir(_)
                ) {
                    crate::ops::state_dir::DirKind::RuntimeDir
                } else {
                    crate::ops::state_dir::DirKind::StateDir
                },
                base_dir: dir.base_dir.clone(),
                vm_id_or_scope: intent.vm_name.clone(),
                mode: dir.mode,
                owner_uid: dir.owner_uid,
                owner_gid: dir.owner_gid,
                created_paths: Vec::new(),
                daemon_uid: None,
            })
            .map(|_| ())
            .map_err(|err| {
                BrokerError::LiveHandler(format!(
                    "prepare vm-start directory {} for {}:{} failed: {err}",
                    dir.base_dir.display(),
                    intent.vm_name,
                    intent.role_id
                ))
            })
        }
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[cfg(test)]
static TEST_USB_SYSFS_ROOT: OnceLock<PathBuf> = OnceLock::new();

#[cfg(not(feature = "layer1-bootstrap"))]
fn usb_device_sysfs_root() -> &'static Path {
    #[cfg(test)]
    if let Some(path) = TEST_USB_SYSFS_ROOT.get() {
        return path.as_path();
    }
    Path::new("/sys/bus/usb/devices")
}

/// The busid lock path one resolved USBIP bind intent is probed at. The
/// resolver bakes the production lock root (`/run/d2b/locks/usbip`) into
/// the intent at bundle-load time; under `cfg(test)` a test may redirect
/// the probe to a scratch root (mirroring [`TEST_USB_SYSFS_ROOT`]) so the
/// kernel's device-bind extension is testable without touching the
/// daemon-owned lock tree.
#[cfg(test)]
fn usbip_lock_path_for_intent(
    intent: &d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
) -> PathBuf {
    match TEST_USBIP_LOCK_ROOT.get() {
        Some(root) => root.join(&intent.bus_id),
        None => intent.lock_path.clone(),
    }
}

#[cfg(not(test))]
fn usbip_lock_path_for_intent(
    intent: &d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
) -> PathBuf {
    intent.lock_path.clone()
}

#[cfg(test)]
static TEST_USBIP_LOCK_ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Unit-test injection for the kernel's USBIP bundle resolver (see
/// [`load_kernel_resolver`]).
#[cfg(test)]
static TEST_KERNEL_BUNDLE_RESOLVER: OnceLock<std::sync::Arc<BundleResolver>> = OnceLock::new();

#[cfg(not(feature = "layer1-bootstrap"))]
async fn read_usb_device_identity(sysfs_root: &Path, bus_id: &str) -> Result<(u16, u16), BrokerError> {
    if d2b_contracts::usbip::validate_bus_id(bus_id).is_err() {
        return Err(BrokerError::Protocol(format!(
            "invalid USB bus_id for sysfs lookup: {bus_id:?}"
        )));
    }
    let device_dir = sysfs_root.join(bus_id);
    let vendor = read_hex_u16(device_dir.join("idVendor"), bus_id).await?;
    let product = read_hex_u16(device_dir.join("idProduct"), bus_id).await?;
    Ok((vendor, product))
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UsbAuditSerialHmacKeySlot {
    Current,
    Previous,
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct UsbAuditSerialHmacKey {
    slot: UsbAuditSerialHmacKeySlot,
    key_id: String,
    key: Vec<u8>,
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct UsbAuditSerialHmacKeyring {
    current: UsbAuditSerialHmacKey,
    previous: Option<UsbAuditSerialHmacKey>,
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn usb_audit_device_identity_for_busid(
    sysfs_root: &Path,
    bus_id: &str,
    identity: (u16, u16),
    state_dir: &Path,
    test_mode: bool,
) -> Result<
    (
        UsbAuditDeviceIdentity,
        Option<UsbSerialCorrelationKeyRotationAudit>,
    ),
    BrokerError,
> {
    let serial = read_usb_serial_for_audit(sysfs_root, bus_id).await;
    let keyring = match serial.as_deref() {
        Some(_) => Some(usb_audit_serial_hmac_keyring(state_dir, test_mode).await?),
        None => None,
    };
    let rotation_audit = keyring
        .as_ref()
        .and_then(usb_serial_correlation_key_rotation_audit);
    Ok((
        usb_audit_device_identity(identity, serial.as_deref(), keyring.as_ref()),
        rotation_audit,
    ))
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn usb_audit_device_identity(
    identity: (u16, u16),
    serial: Option<&str>,
    keyring: Option<&UsbAuditSerialHmacKeyring>,
) -> UsbAuditDeviceIdentity {
    let serial = serial.map(str::trim).filter(|value| !value.is_empty());
    UsbAuditDeviceIdentity {
        vendor_id: Some(d2b_contracts::usbip::format_usb_hex_id(identity.0)),
        product_id: Some(d2b_contracts::usbip::format_usb_hex_id(identity.1)),
        serial_observed: serial.is_some(),
        serial_correlation: serial.and_then(|serial| {
            keyring.and_then(|keys| usb_serial_correlation(serial, &keys.current))
        }),
        previous_serial_correlation: serial.and_then(|serial| {
            keyring.and_then(|keys| {
                keys.previous
                    .as_ref()
                    .and_then(|key| usb_serial_correlation(serial, key))
            })
        }),
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn usb_serial_correlation(
    serial: &str,
    key: &UsbAuditSerialHmacKey,
) -> Option<UsbSerialCorrelation> {
    if key.key_id.is_empty() || key.key.len() < USB_AUDIT_SERIAL_HMAC_KEY_BYTES {
        return None;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(&key.key).ok()?;
    mac.update(b"d2b-usb-audit-serial-v1\0");
    mac.update(serial.as_bytes());
    Some(UsbSerialCorrelation {
        key_id: key.key_id.clone(),
        hmac_sha256: lower_hex(&mac.finalize().into_bytes()),
    })
}

#[cfg(not(feature = "layer1-bootstrap"))]
const USB_AUDIT_SERIAL_HMAC_KEY_BYTES: usize = 32;
#[cfg(not(feature = "layer1-bootstrap"))]
const USB_AUDIT_SERIAL_HMAC_RANDOM_BYTES: usize = USB_AUDIT_SERIAL_HMAC_KEY_BYTES + 16;
#[cfg(not(feature = "layer1-bootstrap"))]
const USB_AUDIT_SERIAL_HMAC_KEY_DIR: &str = "usb-audit-serial-hmac";
#[cfg(not(feature = "layer1-bootstrap"))]
const USB_AUDIT_SERIAL_HMAC_CURRENT_KEY_FILE: &str = "current.key";
#[cfg(not(feature = "layer1-bootstrap"))]
const USB_AUDIT_SERIAL_HMAC_PREVIOUS_KEY_FILE: &str = "previous.key";
#[cfg(not(feature = "layer1-bootstrap"))]
const USB_AUDIT_SERIAL_HMAC_KEY_MAGIC: &str = "d2b-usb-audit-serial-hmac-v1";
#[cfg(not(feature = "layer1-bootstrap"))]
const USB_AUDIT_SERIAL_CORRELATION_VERSION: &str = "d2b-usb-audit-serial-v1";
#[cfg(not(feature = "layer1-bootstrap"))]
const USB_AUDIT_SERIAL_HMAC_PREVIOUS_KEY_GRACE_WINDOW_SECONDS: u64 = 30 * 24 * 60 * 60;
/// Rotation-window / rotation-audit dedupe sets.
///
/// `tokio::sync` per plan U8: the dispatch arms reach them through the
/// non-blocking `try_lock`; a `Busy` collision degrades exactly like the
/// old poisoned path (the dedupe is skipped, never a hard failure).
#[cfg(not(feature = "layer1-bootstrap"))]
static USB_AUDIT_SERIAL_HMAC_ROTATION_LOGGED: LazyLock<tokio::sync::Mutex<HashMap<String, ()>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));
#[cfg(not(feature = "layer1-bootstrap"))]
static USB_AUDIT_SERIAL_HMAC_ROTATION_AUDIT_LOGGED: LazyLock<tokio::sync::Mutex<HashMap<String, ()>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));

#[cfg(not(feature = "layer1-bootstrap"))]
fn usb_serial_correlation_key_rotation_audit(
    keyring: &UsbAuditSerialHmacKeyring,
) -> Option<UsbSerialCorrelationKeyRotationAudit> {
    keyring
        .previous
        .as_ref()
        .map(|previous| UsbSerialCorrelationKeyRotationAudit {
            previous_key_id: previous.key_id.clone(),
            current_key_id: keyring.current.key_id.clone(),
            active_key_count: 2,
            grace_window_seconds: USB_AUDIT_SERIAL_HMAC_PREVIOUS_KEY_GRACE_WINDOW_SECONDS,
            correlation_version: USB_AUDIT_SERIAL_CORRELATION_VERSION.to_owned(),
        })
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn usb_audit_serial_hmac_rotation_dedupe_key(
    audit: &UsbSerialCorrelationKeyRotationAudit,
) -> String {
    format!("{}|{}", audit.previous_key_id, audit.current_key_id)
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn mark_usb_audit_serial_hmac_rotation_audit_logged(
    audit: &UsbSerialCorrelationKeyRotationAudit,
) -> Option<String> {
    let dedupe_key = usb_audit_serial_hmac_rotation_dedupe_key(audit);
    // Non-blocking try-lock (plan U8): a Busy collision skips the dedupe
    // exactly like the old poisoned path - the rotation audit is logged
    // (possibly once more) rather than lost.
    let Ok(mut logged) = USB_AUDIT_SERIAL_HMAC_ROTATION_AUDIT_LOGGED.try_lock() else {
        return Some(dedupe_key);
    };
    if logged.insert(dedupe_key.clone(), ()).is_some() {
        return None;
    }
    Some(dedupe_key)
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn unmark_usb_audit_serial_hmac_rotation_audit_logged(dedupe_key: &str) {
    if let Ok(mut logged) = USB_AUDIT_SERIAL_HMAC_ROTATION_AUDIT_LOGGED.try_lock() {
        logged.remove(dedupe_key);
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn log_usb_audit_serial_hmac_rotation_window(keyring: &UsbAuditSerialHmacKeyring) {
    let Some(audit) = usb_serial_correlation_key_rotation_audit(keyring) else {
        return;
    };
    let dedupe_key = usb_audit_serial_hmac_rotation_dedupe_key(&audit);
    // Non-blocking try-lock (plan U8); a Busy collision skips the window
    // log exactly like the old poisoned path.
    let Ok(mut logged) = USB_AUDIT_SERIAL_HMAC_ROTATION_LOGGED.try_lock() else {
        return;
    };
    if logged.insert(dedupe_key, ()).is_some() {
        return;
    }
    info!(
        event = "usb_serial_correlation_key_rotation_window",
        previous_key_id = %audit.previous_key_id,
        current_key_id = %audit.current_key_id,
        active_key_count = audit.active_key_count,
        grace_window_seconds = audit.grace_window_seconds,
        correlation_version = %audit.correlation_version,
        "USB audit serial HMAC key rotation window active"
    );
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn usb_audit_serial_hmac_keyring(
    state_dir: &Path,
    test_mode: bool,
) -> Result<UsbAuditSerialHmacKeyring, BrokerError> {
    let key_dir = usb_audit_serial_hmac_key_dir(state_dir);
    ensure_usb_audit_serial_hmac_key_dir(&key_dir, test_mode)?;
    let current = match read_usb_audit_serial_hmac_key_file(
        &key_dir.join(USB_AUDIT_SERIAL_HMAC_CURRENT_KEY_FILE),
        UsbAuditSerialHmacKeySlot::Current,
        test_mode,
    )
    .await?
    {
        Some(key) => key,
        None => create_usb_audit_serial_hmac_key(&key_dir, test_mode).await?,
    };
    let previous = read_usb_audit_serial_hmac_key_file(
        &key_dir.join(USB_AUDIT_SERIAL_HMAC_PREVIOUS_KEY_FILE),
        UsbAuditSerialHmacKeySlot::Previous,
        test_mode,
    )
    .await?;

    let keyring = UsbAuditSerialHmacKeyring { current, previous };
    log_usb_audit_serial_hmac_rotation_window(&keyring);
    Ok(keyring)
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn usb_audit_serial_hmac_key_dir(state_dir: &Path) -> PathBuf {
    state_dir
        .join("secrets")
        .join(USB_AUDIT_SERIAL_HMAC_KEY_DIR)
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn ensure_usb_audit_serial_hmac_key_dir(
    key_dir: &Path,
    test_mode: bool,
) -> Result<(), BrokerError> {
    let state_secrets = key_dir.parent().ok_or_else(|| {
        BrokerError::LiveHandler("USB audit serial HMAC key directory has no parent".to_owned())
    })?;
    let owner = if test_mode { None } else { Some(0) };
    crate::sys::path_safe::ensure_dir(state_secrets, 0o700, owner, owner).map_err(|err| {
        BrokerError::LiveHandler(format!(
            "prepare USB audit serial HMAC secrets directory failed: {err}"
        ))
    })?;
    crate::sys::path_safe::ensure_dir(key_dir, 0o700, owner, owner).map_err(|err| {
        BrokerError::LiveHandler(format!(
            "prepare USB audit serial HMAC key directory failed: {err}"
        ))
    })?;
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn read_usb_audit_serial_hmac_key_file(
    path: &Path,
    slot: UsbAuditSerialHmacKeySlot,
    test_mode: bool,
) -> Result<Option<UsbAuditSerialHmacKey>, BrokerError> {
    let fd = match nix::fcntl::open(
        path,
        nix::fcntl::OFlag::O_RDONLY | nix::fcntl::OFlag::O_CLOEXEC | nix::fcntl::OFlag::O_NOFOLLOW,
        nix::sys::stat::Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(nix::errno::Errno::ENOENT) => return Ok(None),
        Err(err) => {
            return Err(BrokerError::LiveHandler(format!(
                "open USB audit serial HMAC key failed: {err}"
            )));
        }
    };
    use tokio::io::AsyncReadExt as _;
    let file = tokio::fs::File::from_std(fs::File::from(owned_fd_from_raw(fd)));
    validate_usb_audit_serial_hmac_key_metadata(&file, test_mode).await?;
    let mut contents = String::new();
    file.take(u64::MAX).read_to_string(&mut contents).await.map_err(|err| {
        BrokerError::LiveHandler(format!("read USB audit serial HMAC key failed: {err}"))
    })?;
    parse_usb_audit_serial_hmac_key(&contents, slot).map(Some)
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn validate_usb_audit_serial_hmac_key_metadata(
    file: &tokio::fs::File,
    test_mode: bool,
) -> Result<(), BrokerError> {
    let metadata = file.metadata().await.map_err(|err| {
        BrokerError::LiveHandler(format!("stat USB audit serial HMAC key failed: {err}"))
    })?;
    if !metadata.is_file() || metadata.mode() & 0o077 != 0 || (!test_mode && metadata.uid() != 0) {
        return Err(BrokerError::LiveHandler(
            "USB audit serial HMAC key must be a root-only regular file".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn create_usb_audit_serial_hmac_key(
    key_dir: &Path,
    test_mode: bool,
) -> Result<UsbAuditSerialHmacKey, BrokerError> {
    let key = generate_usb_audit_serial_hmac_key().await?;
    let dir_fd = crate::sys::path_safe::open_dir_path_safe(key_dir).map_err(|err| {
        BrokerError::LiveHandler(format!(
            "open USB audit serial HMAC key directory failed: {err}"
        ))
    })?;
    match write_new_usb_audit_serial_hmac_key_file(&dir_fd, &key).await {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(err) => {
            return Err(BrokerError::LiveHandler(format!(
                "create USB audit serial HMAC key failed: {err}"
            )));
        }
    }
    read_usb_audit_serial_hmac_key_file(
        &key_dir.join(USB_AUDIT_SERIAL_HMAC_CURRENT_KEY_FILE),
        UsbAuditSerialHmacKeySlot::Current,
        test_mode,
    )
    .await?
    .ok_or_else(|| {
        BrokerError::LiveHandler("USB audit serial HMAC key disappeared after creation".to_owned())
    })
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn write_new_usb_audit_serial_hmac_key_file(
    dir_fd: &OwnedFd,
    key: &UsbAuditSerialHmacKey,
) -> io::Result<()> {
    use tokio::io::AsyncWriteExt as _;
    let fd = crate::sys::path_safe::create_file_at_safe(
        dir_fd,
        USB_AUDIT_SERIAL_HMAC_CURRENT_KEY_FILE,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        0o400,
    )?;
    let mut file = tokio::fs::File::from_std(fs::File::from(fd));
    file.write_all(render_usb_audit_serial_hmac_key(key).as_bytes()).await?;
    crate::sys::path_safe::fchmod(file.as_fd(), 0o400)?;
    file.sync_all().await?;
    rustix::fs::fsync(dir_fd).map_err(|err| io::Error::from_raw_os_error(err.raw_os_error()))?;
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn generate_usb_audit_serial_hmac_key() -> Result<UsbAuditSerialHmacKey, BrokerError> {
    let random = read_high_entropy_bytes(USB_AUDIT_SERIAL_HMAC_RANDOM_BYTES).await?;
    let (key, id_bytes) = random.split_at(USB_AUDIT_SERIAL_HMAC_KEY_BYTES);
    Ok(UsbAuditSerialHmacKey {
        slot: UsbAuditSerialHmacKeySlot::Current,
        key_id: format!("usb-audit-{}", lower_hex(id_bytes)),
        key: key.to_vec(),
    })
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn read_high_entropy_bytes(len: usize) -> Result<Vec<u8>, BrokerError> {
    use tokio::io::AsyncReadExt as _;
    let mut file = tokio::fs::File::open("/dev/urandom").await.map_err(|err| {
        BrokerError::LiveHandler(format!(
            "open kernel CSPRNG for USB audit key failed: {err}"
        ))
    })?;
    let mut bytes = vec![0u8; len];
    file.read_exact(&mut bytes).await.map_err(|err| {
        BrokerError::LiveHandler(format!(
            "read kernel CSPRNG for USB audit key failed: {err}"
        ))
    })?;
    Ok(bytes)
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn render_usb_audit_serial_hmac_key(key: &UsbAuditSerialHmacKey) -> String {
    format!(
        "{USB_AUDIT_SERIAL_HMAC_KEY_MAGIC}\nkey_id={}\nkey_hex={}\n",
        key.key_id,
        lower_hex(&key.key)
    )
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn parse_usb_audit_serial_hmac_key(
    contents: &str,
    slot: UsbAuditSerialHmacKeySlot,
) -> Result<UsbAuditSerialHmacKey, BrokerError> {
    let mut lines = contents.lines();
    if lines.next() != Some(USB_AUDIT_SERIAL_HMAC_KEY_MAGIC) {
        return Err(BrokerError::LiveHandler(
            "USB audit serial HMAC key has invalid magic".to_owned(),
        ));
    }
    let mut key_id = None;
    let mut key_hex = None;
    for line in lines {
        if let Some(value) = line.strip_prefix("key_id=") {
            key_id = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("key_hex=") {
            key_hex = Some(value.to_owned());
        }
    }
    let key_id = key_id
        .filter(|value| usb_audit_key_id_is_safe(value))
        .ok_or_else(|| {
            BrokerError::LiveHandler("USB audit serial HMAC key id is invalid".to_owned())
        })?;
    let key = decode_fixed_hex_key(
        &key_hex.ok_or_else(|| {
            BrokerError::LiveHandler("USB audit serial HMAC key material is missing".to_owned())
        })?,
        USB_AUDIT_SERIAL_HMAC_KEY_BYTES,
    )?;
    Ok(UsbAuditSerialHmacKey { slot, key_id, key })
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn usb_audit_key_id_is_safe(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn decode_fixed_hex_key(value: &str, len: usize) -> Result<Vec<u8>, BrokerError> {
    if value.len() != len * 2 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BrokerError::LiveHandler(
            "USB audit serial HMAC key material is invalid".to_owned(),
        ));
    }
    (0..value.len())
        .step_by(2)
        .map(|idx| {
            u8::from_str_radix(&value[idx..idx + 2], 16).map_err(|err| {
                BrokerError::LiveHandler(format!("parse USB audit serial HMAC key failed: {err}"))
            })
        })
        .collect()
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let byte = *byte;
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn read_usb_serial_for_audit(sysfs_root: &Path, bus_id: &str) -> Option<String> {
    if d2b_contracts::usbip::validate_bus_id(bus_id).is_err() {
        return None;
    }
    let path = sysfs_root.join(bus_id).join("serial");
    match tokio::fs::read_to_string(path).await {
        Ok(raw) => {
            let trimmed = raw.trim().to_owned();
            (!trimmed.is_empty()).then_some(trimmed)
        }
        Err(_) => None,
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn usb_device_node_for_busid(sysfs_root: &Path, bus_id: &str) -> Result<PathBuf, BrokerError> {
    d2b_contracts::usbip::validate_bus_id(bus_id)
        .map_err(|err| BrokerError::LiveHandler(format!("invalid usbip bus_id: {err:?}")))?;
    let device_dir = sysfs_root.join(bus_id);
    let busnum = read_usb_decimal_attr(&device_dir, "busnum", bus_id).await?;
    let devnum = read_usb_decimal_attr(&device_dir, "devnum", bus_id).await?;
    Ok(PathBuf::from(format!(
        "/dev/bus/usb/{busnum:03}/{devnum:03}"
    )))
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn read_usb_decimal_attr(device_dir: &Path, attr: &str, bus_id: &str) -> Result<u16, BrokerError> {
    let path = device_dir.join(attr);
    let raw = tokio::fs::read_to_string(&path).await.map_err(|err| {
        BrokerError::LiveHandler(format!(
            "read USB {attr} for bus_id={bus_id} at {} failed: {err}",
            path.display()
        ))
    })?;
    raw.trim().parse::<u16>().map_err(|err| {
        BrokerError::LiveHandler(format!(
            "parse USB {attr} for bus_id={bus_id} at {} failed: {err}",
            path.display()
        ))
    })
}

#[cfg(all(test, not(feature = "layer1-bootstrap")))]
#[derive(Debug, Clone, PartialEq, Eq)]
enum TestUsbipBackendAclEvent {
    Grant { uid: u32 },
    Revoke { uid: u32 },
}

#[cfg(all(test, not(feature = "layer1-bootstrap")))]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn test_usbip_backend_acl_events() -> &'static Mutex<Vec<TestUsbipBackendAclEvent>> {
    static EVENTS: OnceLock<Mutex<Vec<TestUsbipBackendAclEvent>>> = OnceLock::new();
    EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

#[cfg(all(test, not(feature = "layer1-bootstrap")))]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn take_test_usbip_backend_acl_events() -> Vec<TestUsbipBackendAclEvent> {
    let mut events = test_usbip_backend_acl_events()
        .lock()
        .expect("test USBIP ACL event lock");
    std::mem::take(&mut *events)
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn rollback_usbip_bind_after_audit_failure<B: DispatchBackend>(
    backend: &B,
    resolver: &Arc<BundleResolver>,
    intent: &d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
    same_vm_replay: bool,
) {
    if same_vm_replay {
        return;
    }
    if let Err(revoke_error) = revoke_usbip_backend_device_acl(resolver, intent).await {
        warn!(
            bus_id = %intent.bus_id,
            vm = %intent.vm_name,
            error = ?revoke_error,
            "UsbipBind audit write failed and backend ACL rollback failed"
        );
    }
    match backend.usbip_unbind(intent).await {
        Ok(()) => {
            if let Err(lock_error) =
                crate::ops::usbip_lock::release_lock(&intent.lock_path, &intent.vm_name)
            {
                warn!(
                    bus_id = %intent.bus_id,
                    vm = %intent.vm_name,
                    error = %lock_error,
                    "UsbipBind audit write failed and lock rollback failed"
                );
            }
        }
        Err(unbind_error) => {
            warn!(
                bus_id = %intent.bus_id,
                vm = %intent.vm_name,
                error = ?unbind_error,
                "UsbipBind audit write failed and backend unbind rollback failed"
            );
        }
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn rollback_usbip_bind_after_acl_grant_failure<B: DispatchBackend>(
    backend: &B,
    intent: &d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
    same_vm_replay: bool,
    grant_error: BrokerError,
) -> BrokerError {
    if same_vm_replay {
        return grant_error;
    }
    match backend.usbip_unbind(intent).await {
        Ok(()) => {
            if let Err(lock_error) =
                crate::ops::usbip_lock::release_lock(&intent.lock_path, &intent.vm_name)
            {
                warn!(
                    bus_id = %intent.bus_id,
                    vm = %intent.vm_name,
                    grant_error = ?grant_error,
                    error = %lock_error,
                    "UsbipBind ACL grant failed, rollback unbind succeeded, but lock rollback failed"
                );
            }
        }
        Err(rollback_error) => {
            warn!(
                bus_id = %intent.bus_id,
                vm = %intent.vm_name,
                grant_error = ?grant_error,
                rollback_error = ?rollback_error,
                "UsbipBind ACL grant failed and rollback unbind also failed"
            );
        }
    }
    grant_error
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn handle_usbip_acl_revoke_failure_after_unbind(
    intent: &d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
    preserve_durable_claim: bool,
    revoke_error: BrokerError,
) -> BrokerError {
    if preserve_durable_claim {
        return revoke_error;
    }

    // `backend.usbip_unbind` succeeded before the ACL revoke was attempted.
    // Release the host-session claim on revoke failure unless a best-effort
    // live recheck proves the device is still attached to usbip-host. If the
    // recheck itself fails, trust the successful unbind result and release to
    // avoid converting an ACL cleanup error into a stale busid lock.
    match crate::ops::usbip_host::inspect_usbip_driver_binding(
        usb_device_sysfs_root(),
        &intent.bus_id,
    )
    .await
    {
        Ok(crate::ops::usbip_host::UsbipDriverBinding::BoundToUsbipHost) => {
            warn!(
                bus_id = %intent.bus_id,
                vm = %intent.vm_name,
                revoke_error = ?revoke_error,
                "UsbipUnbind ACL revoke failed after unbind, but device still appears bound to usbip-host; preserving lock for manual recovery"
            );
            return revoke_error;
        }
        Ok(observed) => {
            warn!(
                bus_id = %intent.bus_id,
                vm = %intent.vm_name,
                observed = ?observed,
                revoke_error = ?revoke_error,
                "UsbipUnbind ACL revoke failed after unbind; releasing lock because device is no longer bound to usbip-host"
            );
        }
        Err(inspect_error) => {
            warn!(
                bus_id = %intent.bus_id,
                vm = %intent.vm_name,
                inspect_error = %inspect_error,
                revoke_error = ?revoke_error,
                "UsbipUnbind ACL revoke failed after unbind and post-unbind inspection failed; releasing lock based on successful unbind"
            );
        }
    }

    if let Err(lock_error) =
        crate::ops::usbip_lock::release_lock(&intent.lock_path, &intent.vm_name)
    {
        return BrokerError::LiveHandler(format!(
            "USBIP ACL revoke failed after successful unbind and lock release failed: revoke_error={revoke_error:?}; lock_error={lock_error}"
        ));
    }
    revoke_error
}

#[cfg(not(feature = "layer1-bootstrap"))]
const USBIP_BACKEND_ACL_GRANT_ATTEMPTS: usize = 20;

#[cfg(not(feature = "layer1-bootstrap"))]
const USBIP_BACKEND_ACL_GRANT_RETRY_SLEEP: std::time::Duration =
    std::time::Duration::from_millis(100);

#[cfg(not(feature = "layer1-bootstrap"))]
async fn retry_usbip_backend_acl_grant<'a, V, G, R, S>(
    uid: u32,
    mut verify_device_node: V,
    mut grant_acl: G,
    mut revoke_acl: R,
    mut sleep: S,
) -> Result<(), BrokerError>
where
    V: FnMut() -> Pin<Box<dyn Future<Output = Result<PathBuf, BrokerError>> + Send + 'a>>,
    G: for<'b> FnMut(
            &'b Path,
            u32,
        ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'b>>,
    R: for<'b> FnMut(
            &'b Path,
            u32,
        ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'b>>,
    S: FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>,
{
    let mut last_error = None;

    for _ in 0..USBIP_BACKEND_ACL_GRANT_ATTEMPTS {
        let device_node = match verify_device_node().await {
            Ok(device_node) => device_node,
            Err(error) => {
                last_error = Some(error);
                sleep().await;
                continue;
            }
        };

        if let Err(error) = grant_acl(&device_node, uid).await {
            last_error = Some(error);
            sleep().await;
            continue;
        }

        match verify_device_node().await {
            Ok(current_device_node) if current_device_node == device_node => return Ok(()),
            Ok(current_device_node) => {
                let _ = revoke_acl(&device_node, uid).await;
                last_error = Some(BrokerError::LiveHandler(format!(
                    "USBIP device node changed while granting backend ACL: granted {}, observed {}; retrying",
                    device_node.display(),
                    current_device_node.display(),
                )));
            }
            Err(error) => {
                let _ = revoke_acl(&device_node, uid).await;
                last_error = Some(error);
            }
        }
        sleep().await;
    }

    Err(last_error.unwrap_or_else(|| {
        BrokerError::LiveHandler("grant USBIP backend device ACL failed".to_owned())
    }))
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn grant_usbip_backend_device_acl(
    resolver: &Arc<BundleResolver>,
    intent: &d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
    expected_identity: (u16, u16),
    _expected_device_node: PathBuf,
) -> Result<(), BrokerError> {
    let runner = usbip_backend_runner_intent(resolver, intent)?;
    #[cfg(test)]
    {
        let _ = expected_identity;
        test_usbip_backend_acl_events()
            .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            .map_err(|_| BrokerError::Protocol("test USBIP ACL event mutex poisoned".to_owned()))?
            .push(TestUsbipBackendAclEvent::Grant { uid: runner.uid });
        Ok(())
    }
    #[cfg(not(test))]
    {
        async fn verify_usbip_device_node(
            intent: &d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
            expected_identity: (u16, u16),
        ) -> Result<PathBuf, BrokerError> {
            let inspection = crate::ops::usbip_host::enforce_usbip_physical_policy(
                intent,
                usb_device_sysfs_root(),
            )
            .await
            .map_err(|err| map_usbip_host_inspection_error_for_intent(intent, err))?;
            let current_identity = (inspection.vendor, inspection.product);
            let current_device_node = inspection.device_node;
            if current_identity != expected_identity {
                return Err(BrokerError::LiveHandler(format!(
                    "USBIP device identity changed while granting backend ACL for bus_id={}: expected {:?}, observed {:?} at {}",
                    intent.bus_id,
                    expected_identity,
                    current_identity,
                    current_device_node.display(),
                )));
            }
            Ok(current_device_node)
        }
        retry_usbip_backend_acl_grant(
            runner.uid,
            || Box::pin(verify_usbip_device_node(intent, expected_identity)),
            |device_node, uid| {
                Box::pin(async move {
                    crate::live_handlers::live_grant_verified_device_acl(device_node, uid)
                        .await
                        .map_err(|err| BrokerError::LiveHandler(err.to_string()))
                })
            },
            |device_node, uid| {
                Box::pin(async move {
                    crate::live_handlers::live_revoke_verified_device_acl(device_node, uid)
                        .await
                        .map_err(|err| BrokerError::LiveHandler(err.to_string()))
                })
            },
            || {
                Box::pin(async {
                    tokio::time::sleep(USBIP_BACKEND_ACL_GRANT_RETRY_SLEEP).await
                })
            },
        )
        .await
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn revoke_usbip_backend_device_acl(
    resolver: &Arc<BundleResolver>,
    intent: &d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
) -> Result<(), BrokerError> {
    let runner = usbip_backend_runner_intent(resolver, intent)?;
    #[cfg(test)]
    {
        test_usbip_backend_acl_events()
            .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            .map_err(|_| BrokerError::Protocol("test USBIP ACL event mutex poisoned".to_owned()))?
            .push(TestUsbipBackendAclEvent::Revoke { uid: runner.uid });
        Ok(())
    }
    #[cfg(not(test))]
    {
        let inspection = crate::ops::usbip_host::enforce_usbip_physical_policy(
            intent,
            usb_device_sysfs_root(),
        )
        .await
        .map_err(|err| map_usbip_host_inspection_error_for_intent(intent, err))?;
        let device_node = inspection.device_node;
        crate::live_handlers::live_revoke_verified_device_acl(&device_node, runner.uid)
            .await
            .map_err(|err| BrokerError::LiveHandler(err.to_string()))
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn reconcile_active_usbip_backend_acls(
    resolver: &Arc<BundleResolver>,
) -> Result<(), BrokerError> {
    for intent in active_locked_usbip_bind_intents(resolver).await? {
        let inspection = match crate::ops::usbip_host::enforce_usbip_physical_policy(
            &intent,
            usb_device_sysfs_root(),
        )
        .await
        {
            Ok(inspection) => inspection,
            Err(crate::ops::usbip_host::UsbipHostInspectionError::DeviceMissing { bus_id }) => {
                tracing::debug!(
                    bus_id = %bus_id,
                    vm = %intent.vm_name,
                    "USBIP proxy reconcile skipped backend ACL refresh for absent device"
                );
                continue;
            }
            Err(
                crate::ops::usbip_host::UsbipHostInspectionError::DeviceDepartedDuringInspection {
                    bus_id,
                },
            ) => {
                tracing::debug!(
                    bus_id = %bus_id,
                    vm = %intent.vm_name,
                    "USBIP proxy reconcile skipped backend ACL refresh for device that departed during inspection"
                );
                continue;
            }
            Err(err) => return Err(map_usbip_host_inspection_error_for_intent(&intent, err)),
        };
        grant_usbip_backend_device_acl(
            resolver,
            &intent,
            (inspection.vendor, inspection.product),
            inspection.device_node,
        )
        .await?;
    }
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn usbip_backend_runner_intent<'a>(
    resolver: &'a Arc<BundleResolver>,
    intent: &d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
) -> Result<&'a d2b_core::bundle_resolver::ResolvedRunnerIntent, BrokerError> {
    let runner_id = d2b_core::bundle_resolver::intent_id_legacy_runner(
        &format!("sys-{}-usbipd", intent.env),
        "backend",
    );
    resolver
        .find_runner_intent(&runner_id)
        .ok_or(BrokerError::BundleIntentMissing {
            kind: "runner",
            intent_id: runner_id,
        })
}

/// Resolve the env USBIP backend runner UID for the explicit attach path.
/// Returns the UID of the `sys-<env>-usbipd/backend` runner so the broker
/// can grant the per-device ACL without needing a full `ResolvedUsbipBindIntent`.
#[cfg(not(feature = "layer1-bootstrap"))]
fn explicit_usbip_backend_uid(
    resolver: &Arc<BundleResolver>,
    env: &str,
) -> Result<u32, BrokerError> {
    let runner_id =
        d2b_core::bundle_resolver::intent_id_legacy_runner(&format!("sys-{env}-usbipd"), "backend");
    resolver
        .find_runner_intent(&runner_id)
        .map(|r| r.uid)
        .ok_or(BrokerError::BundleIntentMissing {
            kind: "runner",
            intent_id: runner_id,
        })
}

/// Validate and build the scoped nftables rule body for an explicit USBIP
/// attach. Cross-checks the request IPs against the env's declared host config
/// values to prevent the daemon from installing rules for a different env's
/// bridge. Returns the rule body string on success.
#[cfg(not(feature = "layer1-bootstrap"))]
fn build_explicit_usbip_rule_body(
    resolver: &BundleResolver,
    env: &str,
    req_host_uplink_ip: &str,
    req_net_uplink_ip: &str,
) -> Result<(String, String), BrokerError> {
    use std::net::IpAddr;
    let env_config = resolver.find_host_env(env).ok_or_else(|| {
        BrokerError::LiveHandler(format!(
            "explicit USBIP firewall: env {env:?} not found in host config"
        ))
    })?;

    // Validate anti-spoof bridge port flags (uplink must be isolated+neigh_suppress,
    // no MAC learning, no unknown-unicast flood).
    let uplink_flags = env_config
        .bridge_port_flags
        .iter()
        .find(|f| f.role == d2b_core::host::TapRole::Uplink)
        .ok_or_else(|| {
            BrokerError::LiveHandler(format!(
                "explicit USBIP firewall: env {env:?} has no uplink bridge port flags"
            ))
        })?;
    if !uplink_flags.isolated
        || !uplink_flags.neigh_suppress
        || uplink_flags.resolved_learning()
        || uplink_flags.resolved_unicast_flood()
    {
        return Err(BrokerError::LiveHandler(format!(
            "explicit USBIP firewall: env {env:?} uplink bridge port flags fail anti-spoof validation (isolated={} neigh_suppress={} learning={} unicast_flood={})",
            uplink_flags.isolated,
            uplink_flags.neigh_suppress,
            uplink_flags.resolved_learning(),
            uplink_flags.resolved_unicast_flood(),
        )));
    }

    // Cross-check request IPs against the env's declared values. This prevents
    // a compromised daemon from installing a carve-out scoped to the wrong env.
    let env_host_ip = env_config.host_uplink_ip.as_deref().ok_or_else(|| {
        BrokerError::LiveHandler(format!(
            "explicit USBIP firewall: env {env:?} has no host_uplink_ip in host config"
        ))
    })?;
    let env_net_ip = env_config.net_uplink_ip.as_deref().ok_or_else(|| {
        BrokerError::LiveHandler(format!(
            "explicit USBIP firewall: env {env:?} has no net_uplink_ip in host config"
        ))
    })?;
    if req_host_uplink_ip != env_host_ip || req_net_uplink_ip != env_net_ip {
        return Err(BrokerError::LiveHandler(format!(
            "explicit USBIP firewall: request IPs do not match env {env:?} declared IPs (host_uplink_ip: req={req_host_uplink_ip:?} env={env_host_ip:?}; net_uplink_ip: req={req_net_uplink_ip:?} env={env_net_ip:?})"
        )));
    }

    // Validate the IPs are non-loopback, non-unspecified IPv4.
    fn safe_ipv4(value: &str) -> Option<String> {
        match value.parse::<IpAddr>().ok()? {
            IpAddr::V4(addr)
                if !addr.is_unspecified() && !addr.is_loopback() && !addr.is_multicast() =>
            {
                Some(addr.to_string())
            }
            _ => None,
        }
    }
    let host_ip = safe_ipv4(req_host_uplink_ip).ok_or_else(|| {
        BrokerError::LiveHandler(format!(
            "explicit USBIP firewall: host_uplink_ip {req_host_uplink_ip:?} is not a valid non-loopback IPv4"
        ))
    })?;
    let net_ip = safe_ipv4(req_net_uplink_ip).ok_or_else(|| {
        BrokerError::LiveHandler(format!(
            "explicit USBIP firewall: net_uplink_ip {req_net_uplink_ip:?} is not a valid non-loopback IPv4"
        ))
    })?;

    let bridge_ifname = env_config.bridge.as_str();
    // Validate the bridge ifname is safe for nft literals.
    if bridge_ifname.contains('"') || bridge_ifname.contains('\\') || bridge_ifname.contains('\0') {
        return Err(BrokerError::LiveHandler(format!(
            "explicit USBIP firewall: env {env:?} bridge ifname contains unsafe characters"
        )));
    }

    let rule_body = format!(
        "iifname \"{bridge_ifname}\" ip saddr {net_ip} ip daddr {host_ip} ip protocol tcp tcp dport 3240 accept"
    );
    Ok((bridge_ifname.to_owned(), rule_body))
}

/// Grant the per-device ACL to the env's USBIP backend runner for the explicit
/// attach path. Unlike `grant_usbip_backend_device_acl` this does NOT check a
/// vendor/product allowlist; the explicit path carries no bundle allowlist.
/// Retries up to 20 times with 100ms sleep (same policy as the declared path).
#[cfg(not(feature = "layer1-bootstrap"))]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn grant_explicit_usbip_backend_acl(
    resolver: &Arc<BundleResolver>,
    env: &str,
    bus_id: &str,
    expected_identity: (u16, u16),
    _expected_device_node: PathBuf,
) -> Result<(), BrokerError> {
    let backend_uid = explicit_usbip_backend_uid(resolver, env)?;
    #[cfg(test)]
    {
        let _ = bus_id;
        let _ = expected_identity;
        test_usbip_backend_acl_events()
            .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            .map_err(|_| BrokerError::Protocol("test USBIP ACL event mutex poisoned".to_owned()))?
            .push(TestUsbipBackendAclEvent::Grant { uid: backend_uid });
        Ok(())
    }
    #[cfg(not(test))]
    {
        retry_usbip_backend_acl_grant(
            backend_uid,
            || Box::pin(verify_explicit_usbip_device_stable(bus_id, expected_identity)),
            |device_node, uid| {
                Box::pin(async move {
                    crate::live_handlers::live_grant_verified_device_acl(device_node, uid)
                        .await
                        .map_err(|err| BrokerError::LiveHandler(err.to_string()))
                })
            },
            |device_node, uid| {
                Box::pin(async move {
                    crate::live_handlers::live_revoke_verified_device_acl(device_node, uid)
                        .await
                        .map_err(|err| BrokerError::LiveHandler(err.to_string()))
                })
            },
            || {
                Box::pin(async {
                    tokio::time::sleep(USBIP_BACKEND_ACL_GRANT_RETRY_SLEEP).await
                })
            },
        )
        .await
    }
}

/// Stability check for explicit USBIP device ACL grant. Uses
/// `inspect_usbip_host_device` (no allowlist check) to verify the device at
/// `bus_id` still matches `expected_identity` and `expected_device_node`.
#[cfg(all(not(feature = "layer1-bootstrap"), not(test)))]
async fn verify_explicit_usbip_device_stable(
    bus_id: &str,
    expected_identity: (u16, u16),
) -> Result<PathBuf, BrokerError> {
    let inspection =
        crate::ops::usbip_host::inspect_usbip_host_device(usb_device_sysfs_root(), bus_id)
            .await
            .map_err(|err| {
                BrokerError::LiveHandler(format!(
                    "explicit USBIP device stability check failed for bus_id={bus_id}: {err}"
                ))
            })?;
    let current_identity = (inspection.vendor, inspection.product);
    let current_device_node = inspection.device_node;
    if current_identity != expected_identity {
        return Err(BrokerError::LiveHandler(format!(
            "USBIP device identity changed while granting backend ACL for bus_id={bus_id}: expected {:?}, observed {:?} at {}",
            expected_identity,
            current_identity,
            current_device_node.display(),
        )));
    }
    Ok(current_device_node)
}

/// Revoke the per-device ACL from the env's USBIP backend runner for the
/// explicit attach path rollback. Best-effort; failures are logged only.
#[cfg(not(feature = "layer1-bootstrap"))]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn revoke_explicit_usbip_backend_acl(
    resolver: &Arc<BundleResolver>,
    env: &str,
    bus_id: &str,
) -> Result<(), BrokerError> {
    let backend_uid = explicit_usbip_backend_uid(resolver, env)?;
    #[cfg(test)]
    {
        let _ = bus_id;
        test_usbip_backend_acl_events()
            .lock() // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            .map_err(|_| BrokerError::Protocol("test USBIP ACL event mutex poisoned".to_owned()))?
            .push(TestUsbipBackendAclEvent::Revoke { uid: backend_uid });
        Ok(())
    }
    #[cfg(not(test))]
    {
        let inspection = crate::ops::usbip_host::inspect_usbip_host_device(
            usb_device_sysfs_root(),
            bus_id,
        )
        .await
        .map_err(|err| {
                    BrokerError::LiveHandler(format!(
                        "explicit USBIP revoke inspection failed for bus_id={bus_id}: {err}"
                    ))
                })?;
        crate::live_handlers::live_revoke_verified_device_acl(&inspection.device_node, backend_uid)
            .await
            .map_err(|err| BrokerError::LiveHandler(err.to_string()))
    }
}

/// Collect rule bodies for all currently active explicit USBIP carve-outs
/// (busids locked in `/run/d2b/locks/usbip/` that are neither declared
/// in the bundle as static intents nor resolvable via the wildcard
/// `pending` fallback). Used by `build_usbip_explicit_firewall_decision`
/// to preserve existing explicit carve-outs when atomically replacing the
/// nftables state.
///
/// Entries for `excluding_bus_id` are skipped (caller adds its own carveout).
#[cfg(not(feature = "layer1-bootstrap"))]
async fn collect_active_explicit_usbip_carveouts(
    resolver: &BundleResolver,
    excluding_bus_id: &str,
) -> Vec<(String, String)> {
    let Ok(mut entries) = tokio::fs::read_dir("/run/d2b/locks/usbip").await else {
        return Vec::new();
    };
    let mut collected = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        collected.push(entry);
    }
    collected
        .into_iter()
        .filter_map(|entry| {
            let bus_id = entry.file_name().into_string().ok()?;
            d2b_contracts::usbip::validate_bus_id(&bus_id).ok()?;
            if bus_id == excluding_bus_id {
                return None;
            }
            // Skip busids that are handled by the declared (static or wildcard) path.
            if static_usbip_busid_owner(resolver, &bus_id).is_some() {
                return None;
            }
            let vm = crate::ops::usbip_lock::peek_owner(&entry.path())?;
            if find_wildcard_usbip_bind_intent_for(resolver, &vm, &bus_id).is_some() {
                return None;
            }
            // Pure explicit busid - reconstruct rule body from the zone
            // Guest resource + host config. The v3 Guest's environment is
            // its Zone name (host.json environments are empty in v3, so a
            // missing host config fails closed by skipping the carveout).
            let env = resolver
                .guest_vm_resources()
                .find(|(_, resource)| resource.metadata().name().as_str() == vm)
                .map(|(zone, _)| zone.as_str().to_owned())?;
            let env_config = resolver.find_host_env(env.as_str())?;
            let host_ip = env_config.host_uplink_ip.as_deref()?;
            let net_ip = env_config.net_uplink_ip.as_deref()?;
            // Validate bridge port flags fail-closed: skip carveout if anti-spoof
            // invariants are no longer satisfied (operator may have changed bridge config).
            let uplink_flags = env_config
                .bridge_port_flags
                .iter()
                .find(|f| f.role == d2b_core::host::TapRole::Uplink)?;
            if !uplink_flags.isolated
                || !uplink_flags.neigh_suppress
                || uplink_flags.resolved_learning()
                || uplink_flags.resolved_unicast_flood()
            {
                return None;
            }
            let bridge_ifname = env_config.bridge.as_str();
            if bridge_ifname.contains('"')
                || bridge_ifname.contains('\\')
                || bridge_ifname.contains('\0')
            {
                return None;
            }
            let rule_body = format!(
                "iifname \"{bridge_ifname}\" ip saddr {net_ip} ip daddr {host_ip} ip protocol tcp tcp dport 3240 accept"
            );
            Some((bus_id, rule_body))
        })
        .collect()
}

/// Build the nft firewall decision for an explicit USBIP attach. Starts from
/// the host nft base, preserves all currently-active declared and explicit
/// carve-outs, and inserts the new carve-out for `bus_id`.
#[cfg(not(feature = "layer1-bootstrap"))]
async fn build_usbip_explicit_firewall_decision(
    resolver: &BundleResolver,
    host_nft_intent: &d2b_core::bundle_resolver::ResolvedNftIntent,
    bus_id: &str,
    rule_body: &str,
) -> Result<crate::ops::usbip_firewall::UsbipBindFirewallRuleDecision, BrokerError> {
    let mut batch = d2b_host::nftables::NftBatch::parse(host_nft_intent.script_body.as_str())
        .map_err(|err| BrokerError::NftScriptParseFailed(err.to_string()))?;
    let mut inserted = std::collections::BTreeSet::<String>::new();

    // Add carveouts for all declared (statically locked) USBIP busids.
    for id in resolver.usbip_bind_intent_ids() {
        let Some(bind_intent) = resolver.find_usbip_bind_intent(id) else {
            continue;
        };
        let Some(owner) = crate::ops::usbip_lock::peek_owner(&bind_intent.lock_path) else {
            continue;
        };
        if owner != bind_intent.vm_name {
            continue;
        }
        let firewall_id = d2b_core::bundle_resolver::intent_id_usbip_firewall(
            &bind_intent.env,
            &bind_intent.bus_id,
        );
        if !inserted.insert(firewall_id.clone()) {
            continue;
        }
        let Some(active_firewall) = resolver.find_usbip_firewall_intent(&firewall_id) else {
            continue;
        };
        batch
            .add_usbip_carveout_expr(
                d2b_host::nftables::ChainHook::Input,
                &d2b_host::nftables::BusId::new(active_firewall.bus_id.as_str()),
                active_firewall.nft_rule_body.as_str(),
            )
            .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
    }

    // Add carveouts for active dynamic (declared wildcard) USBIP busids.
    for bind_intent in active_dynamic_usbip_bind_intents(resolver).await {
        let firewall_id = d2b_core::bundle_resolver::intent_id_usbip_firewall(
            &bind_intent.env,
            &bind_intent.bus_id,
        );
        if !inserted.insert(firewall_id.clone()) {
            continue;
        }
        let Some(active_firewall) = find_usbip_firewall_intent_or_wildcard(resolver, &firewall_id)
        else {
            continue;
        };
        batch
            .add_usbip_carveout_expr(
                d2b_host::nftables::ChainHook::Input,
                &d2b_host::nftables::BusId::new(active_firewall.bus_id.as_str()),
                active_firewall.nft_rule_body.as_str(),
            )
            .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
    }

    // Add carveouts for currently active explicit busids (not in bundle, but locked).
    // Reconstructed on-the-fly from the lock owner's manifest entry + env host config.
    for (explicit_bus_id, explicit_rule_body) in
        collect_active_explicit_usbip_carveouts(resolver, bus_id).await
    {
        let carveout_id = format!("explicit:{explicit_bus_id}");
        if !inserted.insert(carveout_id) {
            continue;
        }
        batch
            .add_usbip_carveout_expr(
                d2b_host::nftables::ChainHook::Input,
                &d2b_host::nftables::BusId::new(explicit_bus_id.as_str()),
                explicit_rule_body.as_str(),
            )
            .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
    }

    // Insert the new explicit carveout last.
    crate::ops::usbip_firewall::bind_firewall_rule(
        batch,
        &d2b_host::nftables::BusId::new(bus_id),
        rule_body,
    )
    .map_err(|err| BrokerError::LiveHandler(err.to_string()))
}

fn runner_role_for_process_role(
    role: &d2b_core::processes::ProcessRole,
) -> Option<d2b_contracts_broker::broker_wire::RunnerRole> {
    use d2b_contracts_broker::broker_wire::RunnerRole;
    use d2b_core::processes::ProcessRole;

    match role {
        ProcessRole::ProviderController => Some(RunnerRole::ProviderController),
        ProcessRole::SwtpmPreStartFlush => Some(RunnerRole::SwtpmFlush),
        ProcessRole::Swtpm => Some(RunnerRole::Swtpm),
        ProcessRole::Virtiofsd => Some(RunnerRole::Virtiofsd),
        ProcessRole::Video => Some(RunnerRole::Video),
        ProcessRole::Gpu | ProcessRole::GpuRenderNode => Some(RunnerRole::Gpu),
        ProcessRole::Audio => Some(RunnerRole::Audio),
        ProcessRole::CloudHypervisorRunner => Some(RunnerRole::CloudHypervisor),
        ProcessRole::QemuMediaRunner => Some(RunnerRole::QemuMedia),
        ProcessRole::ActivationNixosRunner => Some(RunnerRole::ActivationNixos),
        ProcessRole::VsockRelay => Some(RunnerRole::VsockRelay),
        ProcessRole::OtelHostBridge => Some(RunnerRole::OtelHostBridge),
        ProcessRole::Usbip => Some(RunnerRole::Usbip),
        ProcessRole::WaylandProxy => Some(RunnerRole::WaylandProxy),
        ProcessRole::HostReconcile
        | ProcessRole::StoreVirtiofsPreflight
        | ProcessRole::ComponentSessionHealth
        | ProcessRole::SecurityKeyFrontend => None,
    }
}

/// The wire `role_id` one trusted runner intent fences against. The
/// cloud-hypervisor runner keeps its daemon-side `ch-runner` alias; every
/// other intent uses its own `role_id`.
///
/// SINGLE EVALUATION POINT for the alias: spawn validation and observation
/// both read it instead of re-listing the match.
#[cfg(not(feature = "layer1-bootstrap"))]
fn wire_role_id_for_intent(intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent) -> &str {
    match intent.role {
        d2b_core::processes::ProcessRole::CloudHypervisorRunner => "ch-runner",
        _ => intent.role_id.as_str(),
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn validate_sandbox_launch_plan(
    req: &d2b_contracts_broker::broker_wire::SpawnRunnerRequest,
    intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent,
    plan: &d2b_contracts_broker::broker_wire::SandboxLaunchPlan,
) -> Result<(), BrokerError> {
    if plan.domain
        != req
            .execution_domain
            .unwrap_or(d2b_contracts_resource::v3::execution_policy::ExecutionDomain::System)
    {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "sandbox_plan.domain",
            requested: format!("{:?}", plan.domain),
            resolved: "request-domain".to_owned(),
        });
    }
    if !plan.no_new_privileges {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "sandbox_plan.no_new_privileges",
            requested: "false".to_owned(),
            resolved: "true".to_owned(),
        });
    }
    if plan.start_root != intent.root_carve_out {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "sandbox_plan.start_root",
            requested: plan.start_root.to_string(),
            resolved: intent.root_carve_out.to_string(),
        });
    }
    if !plan.read_only_root {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "sandbox_plan.read_only_root",
            requested: "false".to_owned(),
            resolved: "unsupported".to_owned(),
        });
    }
    if plan.oom_score_adj != 0 {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "sandbox_plan.oom_score_adj",
            requested: plan.oom_score_adj.to_string(),
            resolved: "unsupported".to_owned(),
        });
    }
    if plan.environment_class != d2b_contracts_resource::v3::process::EnvironmentClass::Minimal {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "sandbox_plan.environment_class",
            requested: format!("{:?}", plan.environment_class),
            resolved: "minimal-only".to_owned(),
        });
    }
    let expected_namespaces = &intent.namespaces;
    let requested_namespaces = |class| match class {
        d2b_contracts_resource::v3::process::NamespaceClass::User => expected_namespaces.user,
        d2b_contracts_resource::v3::process::NamespaceClass::Pid => expected_namespaces.pid,
        d2b_contracts_resource::v3::process::NamespaceClass::Mount => expected_namespaces.mount,
        d2b_contracts_resource::v3::process::NamespaceClass::Ipc => expected_namespaces.ipc,
        d2b_contracts_resource::v3::process::NamespaceClass::Uts => expected_namespaces.uts,
        d2b_contracts_resource::v3::process::NamespaceClass::Network => expected_namespaces.net,
        d2b_contracts_resource::v3::process::NamespaceClass::Cgroup
        | d2b_contracts_resource::v3::process::NamespaceClass::Time => false,
    };
    for class in &plan.namespace_classes {
        if !requested_namespaces(*class) {
            tracing::warn!(
                requested = ?plan.namespace_classes,
                trusted = ?intent.namespaces,
                rejected = ?class,
                "SpawnRunner namespace contract mismatch",
            );
            return Err(BrokerError::SpawnRunnerIntentMismatch {
                field: "sandbox_plan.namespace_classes",
                requested: format!("{class:?}"),
                resolved: "bundle-profile".to_owned(),
            });
        }
    }
    if plan.namespace_classes.iter().any(|class| {
        matches!(
            class,
            d2b_contracts_resource::v3::process::NamespaceClass::User
        )
    }) != expected_namespaces.user
    {
        tracing::warn!(
            requested = ?plan.namespace_classes,
            trusted = ?intent.namespaces,
            "SpawnRunner user namespace contract mismatch",
        );
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "sandbox_plan.namespace_classes",
            requested: "user".to_owned(),
            resolved: "bundle-profile".to_owned(),
        });
    }
    if plan.user_namespace.is_some() != intent.user_namespace.is_some()
        || plan.user_namespace.is_some_and(|spec| {
            spec.mapping_class
                != d2b_contracts_resource::v3::process::MappingClass::ProcessPrincipalRoot
        })
    {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "sandbox_plan.user_namespace",
            requested: format!("{:?}", plan.user_namespace),
            resolved: "bundle-profile".to_owned(),
        });
    }
    if let Some(umask) = &plan.umask {
        let parsed =
            u32::from_str_radix(umask, 8).map_err(|_| BrokerError::SpawnRunnerIntentMismatch {
                field: "sandbox_plan.umask",
                requested: umask.clone(),
                resolved: "valid-octal".to_owned(),
            })?;
        if intent.umask != Some(parsed) {
            return Err(BrokerError::SpawnRunnerIntentMismatch {
                field: "sandbox_plan.umask",
                requested: umask.clone(),
                resolved: intent
                    .umask
                    .map_or_else(|| "inherit".to_owned(), |value| format!("{value:o}")),
            });
        }
    } else if intent.umask.is_some() {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "sandbox_plan.umask",
            requested: "inherit".to_owned(),
            resolved: "bundle-profile".to_owned(),
        });
    }
    if plan.capability_classes.iter().any(|class| {
        !intent
            .capabilities
            .iter()
            .any(|capability| capability_matches(*class, capability))
    }) {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "sandbox_plan.capability_classes",
            requested: "mismatch".to_owned(),
            resolved: "bundle-profile".to_owned(),
        });
    }
    if plan.seccomp_class.as_str() != "strict" || intent.seccomp_policy_ref.is_none() {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "sandbox_plan.seccomp_class",
            requested: plan.seccomp_class.as_str().to_owned(),
            resolved: "strict".to_owned(),
        });
    }
    let expected_namespaces = [
        (
            d2b_contracts_resource::v3::process::NamespaceClass::User,
            expected_namespaces.user,
        ),
        (
            d2b_contracts_resource::v3::process::NamespaceClass::Pid,
            expected_namespaces.pid,
        ),
        (
            d2b_contracts_resource::v3::process::NamespaceClass::Mount,
            expected_namespaces.mount,
        ),
        (
            d2b_contracts_resource::v3::process::NamespaceClass::Ipc,
            expected_namespaces.ipc,
        ),
        (
            d2b_contracts_resource::v3::process::NamespaceClass::Uts,
            expected_namespaces.uts,
        ),
        (
            d2b_contracts_resource::v3::process::NamespaceClass::Network,
            expected_namespaces.net,
        ),
    ]
    .into_iter()
    .filter_map(|(class, enabled)| enabled.then_some(class))
    .collect::<BTreeSet<_>>();
    let actual_namespaces = plan
        .namespace_classes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if actual_namespaces != expected_namespaces
        || actual_namespaces.len() != plan.namespace_classes.len()
    {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "sandbox_plan.namespace_classes",
            requested: "mismatch".to_owned(),
            resolved: "bundle-profile".to_owned(),
        });
    }
    let expected_capabilities = intent
        .capabilities
        .iter()
        .filter_map(|capability| match capability.as_str() {
            "CAP_NET_BIND_SERVICE" => {
                Some(d2b_contracts_resource::v3::process::CapabilityClass::NetworkBind)
            }
            "CAP_NET_RAW" => Some(d2b_contracts_resource::v3::process::CapabilityClass::NetworkRaw),
            "CAP_NET_ADMIN" => {
                Some(d2b_contracts_resource::v3::process::CapabilityClass::NetworkAdmin)
            }
            "CAP_SYS_TIME" => Some(d2b_contracts_resource::v3::process::CapabilityClass::SysTime),
            "CAP_SYS_PTRACE" => {
                Some(d2b_contracts_resource::v3::process::CapabilityClass::SysPtrace)
            }
            "CAP_SYS_ADMIN" => Some(d2b_contracts_resource::v3::process::CapabilityClass::SysAdmin),
            "CAP_DAC_OVERRIDE" => {
                Some(d2b_contracts_resource::v3::process::CapabilityClass::DacOverride)
            }
            "CAP_FOWNER" => Some(d2b_contracts_resource::v3::process::CapabilityClass::Fowner),
            "CAP_CHOWN" => Some(d2b_contracts_resource::v3::process::CapabilityClass::Chown),
            "CAP_SETUID" => Some(d2b_contracts_resource::v3::process::CapabilityClass::Setuid),
            "CAP_SETGID" => Some(d2b_contracts_resource::v3::process::CapabilityClass::Setgid),
            "CAP_AUDIT_WRITE" => {
                Some(d2b_contracts_resource::v3::process::CapabilityClass::AuditWrite)
            }
            "CAP_KILL" => Some(d2b_contracts_resource::v3::process::CapabilityClass::Kill),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let actual_capabilities = plan
        .capability_classes
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    if actual_capabilities != expected_capabilities
        || actual_capabilities.len() != plan.capability_classes.len()
    {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "sandbox_plan.capability_classes",
            requested: "mismatch".to_owned(),
            resolved: "bundle-profile".to_owned(),
        });
    }
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn capability_matches(
    class: d2b_contracts_resource::v3::process::CapabilityClass,
    capability: &str,
) -> bool {
    let expected = match class {
        d2b_contracts_resource::v3::process::CapabilityClass::NetworkBind => "CAP_NET_BIND_SERVICE",
        d2b_contracts_resource::v3::process::CapabilityClass::NetworkRaw => "CAP_NET_RAW",
        d2b_contracts_resource::v3::process::CapabilityClass::NetworkAdmin => "CAP_NET_ADMIN",
        d2b_contracts_resource::v3::process::CapabilityClass::SysTime => "CAP_SYS_TIME",
        d2b_contracts_resource::v3::process::CapabilityClass::SysPtrace => "CAP_SYS_PTRACE",
        d2b_contracts_resource::v3::process::CapabilityClass::SysAdmin => "CAP_SYS_ADMIN",
        d2b_contracts_resource::v3::process::CapabilityClass::DacOverride => "CAP_DAC_OVERRIDE",
        d2b_contracts_resource::v3::process::CapabilityClass::Fowner => "CAP_FOWNER",
        d2b_contracts_resource::v3::process::CapabilityClass::Chown => "CAP_CHOWN",
        d2b_contracts_resource::v3::process::CapabilityClass::Setuid => "CAP_SETUID",
        d2b_contracts_resource::v3::process::CapabilityClass::Setgid => "CAP_SETGID",
        d2b_contracts_resource::v3::process::CapabilityClass::AuditWrite => "CAP_AUDIT_WRITE",
        d2b_contracts_resource::v3::process::CapabilityClass::Kill => "CAP_KILL",
    };
    capability == expected
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn typed_process_identity(
    resource_ref: Option<&d2b_contracts_resource::v3::ResourceRef>,
    resource_uid: Option<&d2b_contracts_resource::v3::ResourceUid>,
    zone_uid: Option<&d2b_contracts_resource::v3::ResourceUid>,
    generation: Option<u64>,
    runtime_scope: Option<[u8; 32]>,
    intent_role_id: &str,
    guest_execution: Option<&d2b_contracts_broker::broker_wire::GuestExecutionBinding>,
) -> Result<bool, BrokerError> {
    let typed = resource_ref.is_some()
        || resource_uid.is_some()
        || zone_uid.is_some()
        || runtime_scope.is_some();
    if !typed {
        return Ok(false);
    }
    let (Some(resource_ref), Some(resource_uid), Some(zone_uid), Some(generation), Some(scope)) = (
        resource_ref,
        resource_uid,
        zone_uid,
        generation,
        runtime_scope,
    ) else {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "process_identity",
            requested: "incomplete".to_owned(),
            resolved: "resource/zone/uid/generation/scope-required".to_owned(),
        });
    };
    if generation == 0 || scope == [0; 32] {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "process_identity",
            requested: "invalid".to_owned(),
            resolved: "nonzero-resource/zone/uid/generation/scope".to_owned(),
        });
    }
    if !matches!(
        resource_ref.resource_type().as_str(),
        "Process" | "EphemeralProcess"
    ) {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "resource_ref",
            requested: resource_ref.to_canonical_string(),
            resolved: "Process-or-EphemeralProcess".to_owned(),
        });
    }
    let expected = private_runtime_scope(
        zone_uid,
        guest_execution.map(|binding| &binding.target_uid),
        resource_ref,
        resource_uid,
        intent_role_id,
        generation,
    );
    if scope != expected {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "runtime_scope",
            requested: "mismatch".to_owned(),
            resolved: "zone-resource-generation-role-commitment".to_owned(),
        });
    }
    Ok(true)
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn typed_control_identity_complete(
    resource_ref: Option<&d2b_contracts_resource::v3::ResourceRef>,
    resource_uid: Option<&d2b_contracts_resource::v3::ResourceUid>,
    zone_uid: Option<&d2b_contracts_resource::v3::ResourceUid>,
    generation: Option<u64>,
    runtime_scope: Option<[u8; 32]>,
    provider_ref: Option<&d2b_contracts_resource::v3::ResourceRef>,
    provider_identity: Option<[u8; 32]>,
    template_identity: Option<[u8; 32]>,
) -> bool {
    let typed = resource_ref.is_some()
        || resource_uid.is_some()
        || zone_uid.is_some()
        || generation.is_some()
        || runtime_scope.is_some()
        || provider_ref.is_some()
        || provider_identity.is_some()
        || template_identity.is_some();
    if !typed {
        return true;
    }
    resource_ref.is_some()
        && resource_uid.is_some()
        && zone_uid.is_some()
        && generation.is_some_and(|generation| generation > 0)
        && runtime_scope.is_some_and(|scope| scope != [0; 32])
        && provider_ref.is_some()
        && provider_identity.is_some_and(|identity| identity != [0; 32])
        && template_identity.is_some_and(|identity| identity != [0; 32])
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn private_runtime_scope(
    zone_uid: &d2b_contracts_resource::v3::ResourceUid,
    guest_uid: Option<&d2b_contracts_resource::v3::ResourceUid>,
    resource_ref: &d2b_contracts_resource::v3::ResourceRef,
    resource_uid: &d2b_contracts_resource::v3::ResourceUid,
    role_id: &str,
    generation: u64,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b-process-runtime-scope-v1");
    let resource_ref = resource_ref.to_canonical_string();
    for value in [
        zone_uid.as_str(),
        guest_uid.map(|uid| uid.as_str()).unwrap_or(""),
        resource_ref.as_str(),
        resource_uid.as_str(),
        role_id,
    ] {
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    digest.update(generation.to_le_bytes());
    digest.finalize().into()
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn validate_typed_process_metadata(
    typed: bool,
    owner_ref: Option<&d2b_contracts_resource::v3::ResourceRef>,
    provider_ref: Option<&d2b_contracts_resource::v3::ResourceRef>,
    provider_identity: Option<[u8; 32]>,
    template_identity: Option<[u8; 32]>,
    guest_execution: Option<&d2b_contracts_broker::broker_wire::GuestExecutionBinding>,
    // Serving-worker posture already resolved from the trusted intent (see
    // `LaunchPosture`); this fence never re-derives it.
    serving_worker: bool,
    intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent,
    // Owning-Device scope already pinned from the verified bundle (the
    // launched row's `metadata.ownerRef`, `device_worker::resolve_launch_scope`).
    // `Some` on the launch path, where that Device anchors every runtime path
    // the launch derives; `None` for the observe/adopt requests, which carry
    // no owner uid to pin and derive no Device paths.
    device_worker_scope: Option<&crate::ops::device_worker::DeviceWorkerScope>,
) -> Result<(), BrokerError> {
    if !typed {
        return Ok(());
    }
    let execution_is_guest = intent.execution_ref.starts_with("Guest/");
    if execution_is_guest != guest_execution.is_some()
        || guest_execution.is_some_and(|binding| !binding.is_valid())
    {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "guest_execution",
            requested: "mismatch".to_owned(),
            resolved: "bundle-execution-target".to_owned(),
        });
    }
    let Some(provider_identity) = provider_identity else {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "provider_identity",
            requested: "missing".to_owned(),
            resolved: "signed-process-provider".to_owned(),
        });
    };
    if provider_identity == [0; 32] {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "provider_identity",
            requested: "zero".to_owned(),
            resolved: "signed-process-provider".to_owned(),
        });
    }
    let Some(provider_ref) = provider_ref else {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "provider_ref",
            requested: "missing".to_owned(),
            resolved: "signed-process-provider".to_owned(),
        });
    };
    if provider_ref.resource_type().as_str() != "Provider"
        || !matches!(
            provider_ref.name().as_str(),
            "system-minijail" | "system-systemd"
        )
    {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "provider_ref",
            requested: "invalid".to_owned(),
            resolved: "fixed-process-provider".to_owned(),
        });
    }
    let mut provider_digest = Sha256::new();
    provider_digest.update(b"d2b-process-provider-v1");
    provider_digest.update(provider_ref.name().as_str().as_bytes());
    let expected_provider_identity: [u8; 32] = provider_digest.finalize().into();
    if provider_identity != expected_provider_identity {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "provider_identity",
            requested: "mismatch".to_owned(),
            resolved: "signed-process-provider".to_owned(),
        });
    }
    let expected_template = if intent.role == d2b_core::processes::ProcessRole::ProviderController
        || d2b_core::bundle_resolver::is_device_worker_role(&intent.role)
    {
        // A Provider controller's template is the signed profile id;
        // a Device worker's is its declared template. Both are named by the
        // template the launch resolved through, never by the per-row role id.
        intent.profile_id.as_str()
    } else {
        intent.role_id.as_str()
    };
    let mut digest = Sha256::new();
    digest.update(b"d2b-process-template-v1");
    digest.update(expected_template.as_bytes());
    let expected_template: [u8; 32] = digest.finalize().into();
    if template_identity != Some(expected_template) {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "template_identity",
            requested: "mismatch".to_owned(),
            resolved: "bundle-process-template".to_owned(),
        });
    }
    if let Some(owner) = owner_ref {
        // The owning Device of a Device worker row is pinned from the verified
        // bundle by the launch arm: a request that names another Device is
        // refused BY NAME here, before any Device-derived identity is trusted.
        if let Some(scope) = device_worker_scope
            && owner != &scope.device_ref
        {
            return Err(BrokerError::SpawnRunnerIntentMismatch {
                field: "owner_ref",
                requested: owner.to_canonical_string(),
                resolved: scope.device_ref.to_canonical_string(),
            });
        }
        let owner_type = owner.resource_type().as_str();
        let admitted = if d2b_core::bundle_resolver::is_device_worker_role(&intent.role) {
            // A Device worker row is declared and owned by the Device it
            // serves; its provider is the Provider that signed the template
            // the intent was resolved through. WHICH Device owns the launched
            // row is pinned from the verified bundle by the launch arm
            // (`device_worker::resolve_launch_scope`: the row's
            // `metadata.ownerRef`, plus the Device row's durable uid), and the
            // request must name exactly that Device - a launch aimed at
            // another Device would derive another Guest's runtime socket
            // directory and state Volume. The scope is `None` for the
            // observe/adopt requests, which carry no owner uid to pin and
            // derive no Device paths.
            owner_type == "Device"
                && device_worker_scope.is_none_or(|scope| owner == &scope.device_ref)
        } else {
            matches!(owner_type, "Guest" | "Host" | "Provider" | "VolumeBinding")
        };
        if !admitted {
            return Err(BrokerError::SpawnRunnerIntentMismatch {
                field: "owner_ref",
                requested: "invalid".to_owned(),
                resolved: "Guest-or-Host-or-Provider-or-VolumeBinding-or-Device-worker".to_owned(),
            });
        }
    } else if d2b_core::bundle_resolver::is_device_worker_role(&intent.role) {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "owner_ref",
            requested: "missing".to_owned(),
            resolved: "Device".to_owned(),
        });
    }
    if intent.role == d2b_core::processes::ProcessRole::ProviderController {
        if serving_worker {
            // A binding-owned serving worker's semantic owner is the
            // runtime-minted VolumeBinding, not the Provider that signed the
            // serving template. The bundle cannot pin the binding identity
            // (it is derived from the attachment tuple at reconcile time),
            // so the fence pins the owner type; the template, execution
            // target, and runtime-scope fences pin the rest.
            if !owner_ref.is_some_and(|owner| owner.resource_type().as_str() == "VolumeBinding") {
                return Err(BrokerError::SpawnRunnerIntentMismatch {
                    field: "owner_ref",
                    requested: "invalid".to_owned(),
                    resolved: "VolumeBinding".to_owned(),
                });
            }
        } else {
            let expected_owner = intent
                .owner_ref
                .as_deref()
                .map(d2b_contracts_resource::v3::ResourceRef::parse)
                .transpose()
                .map_err(|_| BrokerError::SpawnRunnerIntentMismatch {
                    field: "owner_ref",
                    requested: "invalid".to_owned(),
                    resolved: "bundle-owner".to_owned(),
                })?;
            if owner_ref != expected_owner.as_ref() {
                return Err(BrokerError::SpawnRunnerIntentMismatch {
                    field: "owner_ref",
                    requested: "mismatch".to_owned(),
                    resolved: "bundle-owner".to_owned(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn runtime_scope_segment(scope: [u8; 32]) -> String {
    let mut rendered = String::with_capacity(64);
    for byte in scope {
        rendered.push_str(&format!("{byte:02x}"));
    }
    format!("process-{rendered}")
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn private_cgroup_placement(
    placement: &d2b_core::sandbox_profile::CgroupPlacement,
    vm_name: &str,
    runtime_scope: Option<[u8; 32]>,
    typed: bool,
) -> Result<d2b_core::sandbox_profile::CgroupPlacement, BrokerError> {
    if !typed {
        return Ok(placement.clone());
    }
    let Some(runtime_scope) = runtime_scope else {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "cgroup_commitment",
            requested: "missing".to_owned(),
            resolved: "runtime-scope-required".to_owned(),
        });
    };
    let components = Path::new(&placement.subtree)
        .components()
        .collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
        || !matches!(
            components.first(),
            Some(std::path::Component::Normal(value)) if *value == "d2b.slice"
        )
    {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "cgroup_commitment",
            requested: placement.subtree.clone(),
            resolved: "d2b.slice/<zone>/<guest>/<role>".to_owned(),
        });
    }
    let segment = |index: usize| {
        components.get(index).and_then(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
    };
    let role_start = if components.len() >= 4 && segment(2) == Some(vm_name) {
        3
    } else if components.len() >= 3 && segment(1) == Some(vm_name) {
        2
    } else {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "cgroup_commitment",
            requested: placement.subtree.clone(),
            resolved: "d2b.slice/<zone>/<guest>/<role>".to_owned(),
        });
    };
    let role_path = components[role_start..]
        .iter()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    if role_path.is_empty() {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "cgroup_commitment",
            requested: placement.subtree.clone(),
            resolved: "d2b.slice/<zone>/<guest>/<role>".to_owned(),
        });
    }
    let mut private = placement.clone();
    private.subtree = format!(
        "d2b.slice/{}/{role_path}",
        runtime_scope_segment(runtime_scope)
    );
    Ok(private)
}

/// The trusted identity of one resource-backed `w1-swtpm` launch.
///
/// A resource-backed (typed) Process launch is placed under a private cgroup
/// scope (`private_cgroup_placement`) that deliberately carries no VM name,
/// and the Device-worker template intent ships no writable paths, so the
/// spawn-time swtpm-dir fence cannot read the VM identity from the plan.
/// Resolve it from the owning Device the launch arm already pinned against the
/// verified bundle ([`crate::ops::device_worker::DeviceWorkerScope`]): the
/// Device's declared Guest owner names the VM, and the trusted
/// `path:swtpm-state:<guest>` storage row names the state root (which must
/// equal the Provider's `path:tpm-state` policy root). The Device ref and uid
/// read here are the pinned ones, never the request's claim. `None` leaves the
/// launch to the hardening's fail-closed refusal.
#[cfg(not(feature = "layer1-bootstrap"))]
fn resource_backed_swtpm_identity(
    resolver: &BundleResolver,
    req: &d2b_contracts_broker::broker_wire::SpawnRunnerRequest,
    device_worker: &crate::ops::device_worker::DeviceWorkerLaunch,
) -> Option<crate::ops::swtpm_dir::ResourceBackedSwtpm> {
    if !matches!(req.role, RunnerRole::Swtpm | RunnerRole::SwtpmFlush) {
        return None;
    }
    let scope = device_worker.scope.as_ref()?;
    crate::ops::swtpm_dir::resource_backed_identity(
        resolver,
        &scope.zone_uid,
        &scope.device_ref,
        Some(&scope.device_uid),
    )
}

/// What one launch arm resolved for a Device-owned worker row.
///
/// A launch whose intent is not a Device-owned worker role resolves
/// [`DeviceWorkerLaunch::default`]: it has no Device-derived runtime path and
/// no per-Guest socket directory to open.
#[cfg(not(feature = "layer1-bootstrap"))]
fn resolve_device_worker_launch(
    resolver: &BundleResolver,
    req: &d2b_contracts_broker::broker_wire::SpawnRunnerRequest,
    intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent,
) -> Result<crate::ops::device_worker::DeviceWorkerLaunch, BrokerError> {
    use crate::ops::device_worker::{
        DeviceWorkerLaunch, binds_runtime_socket, resolve_launch_scope,
    };
    if !d2b_core::bundle_resolver::is_device_worker_role(&intent.role) {
        return Ok(DeviceWorkerLaunch::default());
    }
    let binds_runtime_socket = binds_runtime_socket(&intent.role);
    // A legacy VM-scoped Device worker (the `swtpm` role of a manifest VM)
    // launches without a typed Process identity: it carries no resource row,
    // so there is no owning Device to pin. Its runtime socket directory is
    // derived from the launch's own trusted placement instead (the `w1-swtpm`
    // fence's `derive_paths`, which cross-checks the placement's VM against
    // the plan's writable paths) - never from a launch argument.
    let (Some(resource_ref), Some(zone_uid)) = (req.resource_ref.as_ref(), req.zone_uid.as_ref())
    else {
        return Ok(DeviceWorkerLaunch {
            scope: None,
            binds_runtime_socket,
        });
    };
    let scope = resolve_launch_scope(
        resolver,
        resource_ref,
        zone_uid,
        req.owner_ref.as_ref(),
        req.owner_uid.as_ref(),
    )
    .map_err(|error| BrokerError::SpawnRunnerIntentMismatch {
        field: error.field(),
        requested: error.requested(),
        resolved: error.resolved(),
    })?;
    Ok(DeviceWorkerLaunch {
        scope: Some(scope),
        binds_runtime_socket,
    })
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn validate_spawn_runner_request_matches_intent(
    req: &d2b_contracts_broker::broker_wire::SpawnRunnerRequest,
    intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent,
    // Posture already resolved from the trusted intent (see `LaunchPosture`);
    // this fence never re-derives it.
    posture: LaunchPosture,
    // Owning-Device scope already pinned from the verified bundle by
    // `resolve_device_worker_launch` (see `DeviceWorkerScope`); `None` for
    // every launch that is not a typed Device-owned worker.
    device_worker_scope: Option<&crate::ops::device_worker::DeviceWorkerScope>,
) -> Result<(), BrokerError> {
    if req.vm_id.as_str() != intent.vm_name {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "vm_id",
            requested: req.vm_id.as_str().to_owned(),
            resolved: intent.vm_name.clone(),
        });
    }
    if let Some(execution_ref) = &req.execution_ref {
        let expected = d2b_contracts_resource::v3::ResourceRef::parse(&intent.execution_ref)
            .map_err(|_| BrokerError::SpawnRunnerIntentMismatch {
                field: "execution_ref",
                requested: execution_ref.to_canonical_string(),
                resolved: "invalid-bundle-reference".to_owned(),
            })?;
        if execution_ref != &expected {
            return Err(BrokerError::SpawnRunnerIntentMismatch {
                field: "execution_ref",
                requested: execution_ref.to_canonical_string(),
                resolved: expected.to_canonical_string(),
            });
        }
    }
    let expected_domain = match intent.execution_domain {
        d2b_core::processes::ProcessExecutionDomain::System => {
            d2b_contracts_resource::v3::execution_policy::ExecutionDomain::System
        }
        d2b_core::processes::ProcessExecutionDomain::User => {
            d2b_contracts_resource::v3::execution_policy::ExecutionDomain::User
        }
    };
    if req
        .execution_domain
        .is_some_and(|domain| domain != expected_domain)
    {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "execution_domain",
            requested: format!("{:?}", req.execution_domain),
            resolved: format!("{expected_domain:?}"),
        });
    }
    let expected_user = intent
        .user_ref
        .as_deref()
        .map(d2b_contracts_resource::v3::ResourceRef::parse)
        .transpose()
        .map_err(|_| BrokerError::SpawnRunnerIntentMismatch {
            field: "user_ref",
            requested: "invalid-bundle-reference".to_owned(),
            resolved: "invalid-bundle-reference".to_owned(),
        })?;
    if req.user_ref != expected_user {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "user_ref",
            requested: format!("{:?}", req.user_ref),
            resolved: format!("{expected_user:?}"),
        });
    }

    let expected_role_id = wire_role_id_for_intent(intent);
    if req.role_id.as_str() != expected_role_id {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "role_id",
            requested: req.role_id.as_str().to_owned(),
            resolved: expected_role_id.to_owned(),
        });
    }
    let Some(expected_role) = runner_role_for_process_role(&intent.role) else {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "role",
            requested: req.role.as_str().to_owned(),
            resolved: format!("{:?}", intent.role),
        });
    };
    if req.role != expected_role {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "role",
            requested: req.role.as_str().to_owned(),
            resolved: expected_role.as_str().to_owned(),
        });
    }
    // Controller-supplied arguments are admitted only by the resolved
    // template's own declaration: the executable and the sandbox posture
    // stay broker-resolved, so a launch may only append arguments after
    // the pinned binary, never replace it.
    if req.launch_args.is_some() && !intent.accepts_launch_args {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "launch_args",
            requested: "present".to_owned(),
            resolved: "template-does-not-admit-controller-arguments".to_owned(),
        });
    }
    let typed = typed_process_identity(
        req.resource_ref.as_ref(),
        req.resource_uid.as_ref(),
        req.zone_uid.as_ref(),
        req.generation,
        req.runtime_scope,
        &intent.role_id,
        req.guest_execution.as_ref(),
    )?;
    if !typed && posture.is_provider_controller() {
        // Every ProviderController launch runs with the runner sandbox the
        // typed Process identity selects (uid/gid, runtime scope, cgroup
        // leaf, escrow). An untyped request could reference a ProviderController
        // intent while skipping the typed metadata fences, including the
        // intent-derived serving-worker posture - refuse it outright.
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "process_identity",
            requested: "untyped".to_owned(),
            resolved: "typed-provider-controller-required".to_owned(),
        });
    }
    validate_typed_process_metadata(
        typed,
        req.owner_ref.as_ref(),
        req.provider_ref.as_ref(),
        req.provider_identity,
        req.template_identity,
        req.guest_execution.as_ref(),
        posture.is_serving_worker(),
        intent,
        device_worker_scope,
    )?;
    if typed
        && (!req.runtime_allocations.is_empty()
            || req.workload_identity.is_some()
            || req.network_tap_context.is_some())
    {
        return Err(BrokerError::SpawnRunnerIntentMismatch {
            field: "runtime_allocations",
            requested: "caller-supplied-runtime-state".to_owned(),
            resolved: "bundle-authoritative-runtime".to_owned(),
        });
    }
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn read_hex_u16(path: PathBuf, bus_id: &str) -> Result<u16, BrokerError> {
    let raw = tokio::fs::read_to_string(&path).await.map_err(|err| {
        BrokerError::LiveHandler(format!(
            "read USB identity for bus_id={bus_id} at {} failed: {err}",
            path.display()
        ))
    })?;
    u16::from_str_radix(raw.trim().trim_start_matches("0x"), 16).map_err(|err| {
        BrokerError::LiveHandler(format!(
            "parse USB identity for bus_id={bus_id} at {} failed: {err}",
            path.display()
        ))
    })
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn enforce_usbip_allowlist(
    intent: &d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
    sysfs_root: &Path,
) -> Result<(u16, u16), BrokerError> {
    let inspection = crate::ops::usbip_host::enforce_usbip_physical_policy(intent, sysfs_root)
        .await
        .map_err(|err| map_usbip_host_inspection_error_for_intent(intent, err))?;
    Ok((inspection.vendor, inspection.product))
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn map_usbip_host_inspection_error_for_intent(
    intent: &d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
    err: crate::ops::usbip_host::UsbipHostInspectionError,
) -> BrokerError {
    match err {
        crate::ops::usbip_host::UsbipHostInspectionError::AllowlistMismatch {
            bus_id,
            vendor,
            product,
        } => BrokerError::UsbipDeviceNotAllowed {
            busid: bus_id,
            vendor,
            product,
        },
        crate::ops::usbip_host::UsbipHostInspectionError::AllowlistMissing { .. } => {
            BrokerError::UsbipPolicyMismatch {
                busid: intent.bus_id.clone(),
                reason: "vendor/product allowlist is missing",
            }
        }
        crate::ops::usbip_host::UsbipHostInspectionError::TopologyIncomplete { bus_id, .. } => {
            BrokerError::UsbipPolicyMismatch {
                busid: bus_id,
                reason: "declared physical topology is incomplete",
            }
        }
        crate::ops::usbip_host::UsbipHostInspectionError::TopologyMismatch { bus_id, .. } => {
            BrokerError::UsbipPolicyMismatch {
                busid: bus_id,
                reason: "observed physical topology does not match the declaration",
            }
        }
        crate::ops::usbip_host::UsbipHostInspectionError::DeviceMissing { bus_id }
        | crate::ops::usbip_host::UsbipHostInspectionError::DeviceDepartedDuringInspection {
            bus_id,
            ..
        } => BrokerError::UsbipDeviceAbsent { busid: bus_id },
        other => BrokerError::LiveHandler(other.to_string()),
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) async fn extend_usbip_backend_device_binds(
    resolver: &BundleResolver,
    vm_id: &str,
    role_id: &str,
    role: &d2b_contracts_broker::broker_wire::RunnerRole,
    mount_policy: &mut d2b_core::sandbox_profile::MountPolicy,
) -> Result<(), BrokerError> {
    if !matches!(role, d2b_contracts_broker::broker_wire::RunnerRole::Usbip)
        || role_id != "backend"
        || !vm_id.starts_with("sys-")
        || !vm_id.ends_with("-usbipd")
    {
        return Ok(());
    }
    let env = vm_id
        .strip_prefix("sys-")
        .and_then(|value| value.strip_suffix("-usbipd"))
        .ok_or_else(|| BrokerError::LiveHandler(format!("invalid USBIP backend vm_id {vm_id}")))?;
    let mut binds = std::collections::BTreeSet::new();
    for id in resolver.usbip_bind_intent_ids() {
        let Some(intent) = resolver.find_usbip_bind_intent(id) else {
            continue;
        };
        if intent.env != env {
            continue;
        }
        let Some(owner) = crate::ops::usbip_lock::peek_owner(&usbip_lock_path_for_intent(&intent))
        else {
            continue;
        };
        if owner != intent.vm_name {
            continue;
        }
        let inspection = crate::ops::usbip_host::enforce_usbip_physical_policy(
            intent,
            usb_device_sysfs_root(),
        )
        .await
        .map_err(|err| map_usbip_host_inspection_error_for_intent(intent, err))?;
        let device_node = inspection.device_node;
        binds.insert(device_node.display().to_string());
    }
    for intent in active_dynamic_usbip_bind_intents(resolver).await {
        if intent.env != env {
            continue;
        }
        let inspection = crate::ops::usbip_host::enforce_usbip_physical_policy(
            &intent,
            usb_device_sysfs_root(),
        )
        .await
        .map_err(|err| map_usbip_host_inspection_error_for_intent(&intent, err))?;
        let device_node = inspection.device_node;
        binds.insert(device_node.display().to_string());
    }
    if binds.is_empty() {
        return Err(BrokerError::LiveHandler(format!(
            "USBIP backend {vm_id}:{role_id} has no active locked busid device node to bind"
        )));
    }
    mount_policy.device_binds = binds.into_iter().collect();
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn extend_audio_runner_pipewire_props(
    vm_id: &str,
    role_id: &str,
    role: &d2b_contracts_broker::broker_wire::RunnerRole,
    env: &mut Vec<String>,
) -> Result<(), BrokerError> {
    if !matches!(role, d2b_contracts_broker::broker_wire::RunnerRole::Audio) || role_id != "audio" {
        return Ok(());
    }
    let state_path = PathBuf::from(format!("/var/lib/d2b/vms/{vm_id}/state/audio-state.json"));
    let bytes = tokio::fs::read(&state_path).await.map_err(|err| {
        BrokerError::LiveHandler(format!(
            "audio runner {vm_id}:{role_id} could not read {}: {err}",
            state_path.display()
        ))
    })?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|err| {
        BrokerError::LiveHandler(format!(
            "audio runner {vm_id}:{role_id} could not parse {}: {err}",
            state_path.display()
        ))
    })?;
    let mic = audio_state_value(&value, "mic", vm_id, role_id)?;
    let speaker = audio_state_value(&value, "speaker", vm_id, role_id)?;
    let input_target = audio_input_target_node(env, vm_id, role_id)?;
    let target_prop = if mic == "on" {
        input_target
            .as_deref()
            .map(|target| format!(" target.object = \"{target}\""))
            .unwrap_or_default()
    } else {
        String::new()
    };
    env.retain(|entry| !entry.starts_with("PIPEWIRE_PROPS="));
    env.push(format!(
        "PIPEWIRE_PROPS={{ application.name = \"d2b-{vm_id}\" node.name = \"d2b-{vm_id}\" node.description = \"d2b {vm_id}\" d2b.vm = \"{vm_id}\" d2b.mic = \"{mic}\" d2b.speaker = \"{speaker}\"{target_prop} }}"
    ));
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn audio_input_target_node(
    env: &[String],
    vm_id: &str,
    role_id: &str,
) -> Result<Option<String>, BrokerError> {
    let Some(raw) = env
        .iter()
        .find_map(|entry| entry.strip_prefix("D2B_AUDIO_INPUT_TARGET_NODE="))
    else {
        return Ok(None);
    };
    if raw.is_empty() || raw.contains('"') || raw.contains('\n') {
        return Err(BrokerError::LiveHandler(format!(
            "audio runner {vm_id}:{role_id} has invalid D2B_AUDIO_INPUT_TARGET_NODE"
        )));
    }
    Ok(Some(raw.to_owned()))
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn audio_state_value<'a>(
    value: &'a Value,
    key: &str,
    vm_id: &str,
    role_id: &str,
) -> Result<&'a str, BrokerError> {
    let state = value.get(key).and_then(Value::as_str).ok_or_else(|| {
        BrokerError::LiveHandler(format!(
            "audio runner {vm_id}:{role_id} state missing string key {key:?}"
        ))
    })?;
    match state {
        "on" | "off" => Ok(state),
        other => Err(BrokerError::LiveHandler(format!(
            "audio runner {vm_id}:{role_id} state key {key:?} has invalid value {other:?}"
        ))),
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) async fn cleanup_stale_sockets(paths: &[PathBuf]) -> Result<(), BrokerError> {
    for path in paths {
        cleanup_stale_unix_socket(path).await?;
    }
    Ok(())
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) async fn cleanup_video_stale_socket(
    role: &d2b_contracts_broker::broker_wire::RunnerRole,
    argv: &[String],
) -> Result<(), BrokerError> {
    if !matches!(role, d2b_contracts_broker::broker_wire::RunnerRole::Video) {
        return Ok(());
    }
    let path = video_socket_path(argv).await?;
    if !path.starts_with("/run/d2b-video/") {
        return Err(BrokerError::LiveHandler(format!(
            "video socket preflight refusing non-d2b socket path {}",
            path.display()
        )));
    }
    cleanup_stale_unix_socket_without_probe(&path, "video socket preflight").await
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn video_socket_path(argv: &[String]) -> Result<PathBuf, BrokerError> {
    let mut iter = argv.iter();
    while let Some(arg) = iter.next() {
        if arg == "--socket-path" {
            let Some(path) = iter.next() else {
                break;
            };
            return Ok(PathBuf::from(path));
        }
        if let Some(path) = arg.strip_prefix("--socket-path=") {
            return Ok(PathBuf::from(path));
        }
    }
    Err(BrokerError::LiveHandler(
        "video socket preflight could not find --socket-path in runner argv".to_owned(),
    ))
}

// The OtelHostBridge runner is `socat UNIX-LISTEN:<host-egress.sock>,...`.
// socat does not unlink a pre-existing socket path before binding, so a
// stale `host-egress.sock` left behind by a prior bridge instance (e.g.
// after the obs VM is restarted, draining and respawning the bridge)
// makes the fresh socat exit immediately with "address in use". The
// readiness probe only checks the socket *file* exists, so the stale
// socket masks the failure and host telemetry silently stops flowing.
// Mirror the cloud-hypervisor / video preflight: drop a provably-stale
// (non-listening) socket before spawn so obs-VM restarts self-heal.
#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) async fn cleanup_otel_host_bridge_stale_socket(
    role: &d2b_contracts_broker::broker_wire::RunnerRole,
    argv: &[String],
) -> Result<(), BrokerError> {
    if !matches!(
        role,
        d2b_contracts_broker::broker_wire::RunnerRole::OtelHostBridge
    ) {
        return Ok(());
    }
    let path = otel_host_bridge_socket_path(argv)?;
    if !path.starts_with("/run/d2b/otel/") {
        return Err(BrokerError::LiveHandler(format!(
            "otel-host-bridge socket preflight refusing non-d2b socket path {}",
            path.display()
        )));
    }
    cleanup_stale_unix_socket_without_probe(&path, "otel-host-bridge socket preflight").await
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn otel_host_bridge_socket_path(argv: &[String]) -> Result<PathBuf, BrokerError> {
    for arg in argv {
        if let Some(rest) = arg.strip_prefix("UNIX-LISTEN:") {
            let path = rest.split(',').next().unwrap_or(rest);
            if !path.is_empty() {
                return Ok(PathBuf::from(path));
            }
        }
    }
    Err(BrokerError::LiveHandler(
        "otel-host-bridge socket preflight could not find UNIX-LISTEN socket in runner argv"
            .to_owned(),
    ))
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn cleanup_stale_unix_socket(path: &Path) -> Result<(), BrokerError> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(BrokerError::LiveHandler(format!(
                "runner socket preflight could not stat {}: {err}",
                path.display()
            )));
        }
    };
    if !metadata.file_type().is_socket() {
        return Err(BrokerError::LiveHandler(format!(
            "runner socket preflight refusing to remove non-socket path {}",
            path.display()
        )));
    }
    match tokio::net::UnixStream::connect(path).await {
        Ok(_) => Err(BrokerError::LiveHandler(format!(
            "runner socket preflight found active listener at {}",
            path.display()
        ))),
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
            ) =>
        {
            tokio::fs::remove_file(path).await.map_err(|remove_err| {
                BrokerError::LiveHandler(format!(
                    "runner socket preflight could not remove stale {}: {remove_err}",
                    path.display()
                ))
            })
        }
        Err(err) => Err(BrokerError::LiveHandler(format!(
            "runner socket preflight could not prove {} stale: {err}",
            path.display()
        ))),
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn cleanup_stale_unix_socket_without_probe(path: &Path, context: &str) -> Result<(), BrokerError> {
    let metadata = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => {
            return Err(BrokerError::LiveHandler(format!(
                "{context} could not stat {}: {err}",
                path.display()
            )));
        }
    };
    if !metadata.file_type().is_socket() {
        return Err(BrokerError::LiveHandler(format!(
            "{context} refusing to remove non-socket path {}",
            path.display()
        )));
    }
    if unix_socket_listening_path(path).await {
        return Err(BrokerError::LiveHandler(format!(
            "{context} found active listener at {}",
            path.display()
        )));
    }
    tokio::fs::remove_file(path).await.map_err(|remove_err| {
        BrokerError::LiveHandler(format!(
            "{context} could not remove stale {}: {remove_err}",
            path.display()
        ))
    })
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn unix_socket_listening_path(path: &Path) -> bool {
    const SO_ACCEPTCON: u64 = 0x0001_0000;
    let expected = path.to_string_lossy();
    let Ok(contents) = tokio::fs::read_to_string("/proc/net/unix").await else {
        return false;
    };
    contents.lines().skip(1).any(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 8 {
            return false;
        }
        let flags = u64::from_str_radix(fields[3], 16).unwrap_or(0);
        let socket_type = fields[4];
        let socket_path = fields[7];
        socket_path == expected && socket_type == "0001" && (flags & SO_ACCEPTCON) != 0
    })
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn build_usbip_firewall_decision(
    resolver: &BundleResolver,
    host_nft_intent: &d2b_core::bundle_resolver::ResolvedNftIntent,
    current: &d2b_core::bundle_resolver::ResolvedUsbipFirewallIntent,
) -> Result<crate::ops::usbip_firewall::UsbipBindFirewallRuleDecision, BrokerError> {
    let mut batch = d2b_host::nftables::NftBatch::parse(host_nft_intent.script_body.as_str())
        .map_err(|err| BrokerError::NftScriptParseFailed(err.to_string()))?;
    let mut inserted = std::collections::BTreeSet::<String>::new();

    for id in resolver.usbip_bind_intent_ids() {
        let Some(bind_intent) = resolver.find_usbip_bind_intent(id) else {
            continue;
        };
        let Some(owner) = crate::ops::usbip_lock::peek_owner(&bind_intent.lock_path) else {
            continue;
        };
        if owner != bind_intent.vm_name {
            continue;
        }
        let firewall_id = d2b_core::bundle_resolver::intent_id_usbip_firewall(
            &bind_intent.env,
            &bind_intent.bus_id,
        );
        if !inserted.insert(firewall_id.clone()) || firewall_id == current.intent_id {
            continue;
        }
        let Some(active_firewall) = resolver.find_usbip_firewall_intent(&firewall_id) else {
            return Err(BrokerError::BundleIntentMissing {
                kind: "usbip-firewall",
                intent_id: firewall_id,
            });
        };
        batch
            .add_usbip_carveout_expr(
                d2b_host::nftables::ChainHook::Input,
                &d2b_host::nftables::BusId::new(active_firewall.bus_id.as_str()),
                active_firewall.nft_rule_body.as_str(),
            )
            .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
    }
    for bind_intent in active_dynamic_usbip_bind_intents(resolver).await {
        let firewall_id = d2b_core::bundle_resolver::intent_id_usbip_firewall(
            &bind_intent.env,
            &bind_intent.bus_id,
        );
        if !inserted.insert(firewall_id.clone()) || firewall_id == current.intent_id {
            continue;
        }
        let Some(active_firewall) = find_usbip_firewall_intent_or_wildcard(resolver, &firewall_id)
        else {
            return Err(BrokerError::BundleIntentMissing {
                kind: "usbip-firewall",
                intent_id: firewall_id,
            });
        };
        batch
            .add_usbip_carveout_expr(
                d2b_host::nftables::ChainHook::Input,
                &d2b_host::nftables::BusId::new(active_firewall.bus_id.as_str()),
                active_firewall.nft_rule_body.as_str(),
            )
            .map_err(|err| BrokerError::LiveHandler(err.to_string()))?;
    }

    crate::ops::usbip_firewall::bind_firewall_rule(
        batch,
        &d2b_host::nftables::BusId::new(current.bus_id.as_str()),
        current.nft_rule_body.as_str(),
    )
    .map_err(|err| BrokerError::LiveHandler(err.to_string()))
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn find_usbip_firewall_intent_or_wildcard(
    resolver: &BundleResolver,
    intent_id: &str,
) -> Option<d2b_core::bundle_resolver::ResolvedUsbipFirewallIntent> {
    if let Some(intent) = resolver.find_usbip_firewall_intent(intent_id) {
        return Some(intent.clone());
    }
    let (env, bus_id) = parse_usbip_firewall_intent_id(intent_id)?;
    d2b_contracts::usbip::validate_bus_id(&bus_id).ok()?;
    let pending_id = d2b_core::bundle_resolver::intent_id_usbip_firewall(&env, "pending");
    let source = resolver.find_usbip_firewall_intent(&pending_id)?;
    Some(d2b_core::bundle_resolver::ResolvedUsbipFirewallIntent {
        intent_id: intent_id.to_owned(),
        bus_id,
        env,
        nft_rule_body: source.nft_rule_body.clone(),
        desired_hash: source.desired_hash.clone(),
    })
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn parse_usbip_firewall_intent_id(intent_id: &str) -> Option<(String, String)> {
    let rest = intent_id.strip_prefix("usbip-fw:env:")?;
    let (env, bus_id) = rest.split_once(":bus:")?;
    if env.is_empty() || bus_id.is_empty() {
        None
    } else {
        Some((env.to_owned(), bus_id.to_owned()))
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn active_dynamic_usbip_bind_intents(
    resolver: &BundleResolver,
) -> Vec<d2b_core::bundle_resolver::ResolvedUsbipBindIntent> {
    let Ok(mut entries) = tokio::fs::read_dir("/run/d2b/locks/usbip").await else {
        return Vec::new();
    };
    let mut collected = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        collected.push(entry);
    }
    collected
        .into_iter()
        .filter_map(|entry| {
            let bus_id = entry.file_name().into_string().ok()?;
            d2b_contracts::usbip::validate_bus_id(&bus_id).ok()?;
            let owner = crate::ops::usbip_lock::peek_owner(&entry.path())?;
            find_wildcard_usbip_bind_intent_for(resolver, &owner, &bus_id)
        })
        .collect()
}

#[cfg(not(feature = "layer1-bootstrap"))]
async fn active_locked_usbip_bind_intents(
    resolver: &BundleResolver,
) -> Result<Vec<d2b_core::bundle_resolver::ResolvedUsbipBindIntent>, BrokerError> {
    let mut out = Vec::new();
    for (intent, owner) in resolver
        .usbip_bind_intent_ids()
        .filter_map(|id| resolver.find_usbip_bind_intent(id))
        .filter_map(|intent| {
            let owner = crate::ops::usbip_lock::peek_owner(&intent.lock_path)?;
            Some((intent, owner))
        })
    {
        if owner != intent.vm_name {
            return Err(BrokerError::LiveHandler(format!(
                "usbip proxy reconcile refused foreign lock for opaque intent {}",
                intent.intent_id
            )));
        }
        out.push(intent.clone());
    }
    out.extend(active_dynamic_usbip_bind_intents(resolver).await);
    Ok(out)
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn find_usbip_bind_intent_or_wildcard(
    resolver: &BundleResolver,
    intent_id: &str,
) -> Option<d2b_core::bundle_resolver::ResolvedUsbipBindIntent> {
    if let Some(intent) = resolver.find_usbip_bind_intent(intent_id) {
        return Some(intent.clone());
    }
    let (env, vm, bus_id) = parse_usbip_bind_intent_id(intent_id)?;
    d2b_contracts::usbip::validate_bus_id(&bus_id).ok()?;
    if static_usbip_busid_owner(resolver, &bus_id).is_some() {
        return None;
    }
    let pending_id = d2b_core::bundle_resolver::intent_id_usbip_bind(&env, &vm, "pending");
    let source = resolver.find_usbip_bind_intent(&pending_id)?;
    Some(dynamic_usbip_bind_intent(source, &bus_id))
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn parse_usbip_bind_intent_id(intent_id: &str) -> Option<(String, String, String)> {
    let rest = intent_id.strip_prefix("usbip-bind:env:")?;
    let (env, rest) = rest.split_once(":vm:")?;
    let (vm, bus_id) = rest.split_once(":bus:")?;
    if env.is_empty() || vm.is_empty() || bus_id.is_empty() {
        None
    } else {
        Some((env.to_owned(), vm.to_owned(), bus_id.to_owned()))
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn find_usbip_bind_intent_for(
    resolver: &BundleResolver,
    vm_name: &str,
    bus_id: &str,
) -> Option<d2b_core::bundle_resolver::ResolvedUsbipBindIntent> {
    let exact = resolver.usbip_bind_intent_ids().find_map(|id| {
        let intent = resolver.find_usbip_bind_intent(id)?;
        if intent.vm_name == vm_name && intent.bus_id == bus_id {
            Some(intent.clone())
        } else {
            None
        }
    });
    exact.or_else(|| {
        if static_usbip_busid_owner(resolver, bus_id).is_some() {
            None
        } else {
            find_wildcard_usbip_bind_intent_for(resolver, vm_name, bus_id)
        }
    })
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn find_usbip_bind_intent_by_busid(
    resolver: &BundleResolver,
    bus_id: &str,
) -> Option<d2b_core::bundle_resolver::ResolvedUsbipBindIntent> {
    let exact = resolver.usbip_bind_intent_ids().find_map(|id| {
        let intent = resolver.find_usbip_bind_intent(id)?;
        if intent.bus_id == bus_id {
            Some(intent.clone())
        } else {
            None
        }
    });
    exact.or_else(|| {
        let lock_path = usbip_lock_path_for_busid(bus_id);
        let owner = crate::ops::usbip_lock::peek_owner(&lock_path)?;
        find_wildcard_usbip_bind_intent_for(resolver, &owner, bus_id)
    })
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn find_wildcard_usbip_bind_intent_for(
    resolver: &BundleResolver,
    vm_name: &str,
    bus_id: &str,
) -> Option<d2b_core::bundle_resolver::ResolvedUsbipBindIntent> {
    d2b_contracts::usbip::validate_bus_id(bus_id).ok()?;
    if static_usbip_busid_owner(resolver, bus_id).is_some() {
        return None;
    }
    resolver.usbip_bind_intent_ids().find_map(|id| {
        let source = resolver.find_usbip_bind_intent(id)?;
        if source.vm_name == vm_name && source.bus_id == "pending" {
            Some(dynamic_usbip_bind_intent(source, bus_id))
        } else {
            None
        }
    })
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn static_usbip_busid_owner(resolver: &BundleResolver, bus_id: &str) -> Option<String> {
    resolver.usbip_bind_intent_ids().find_map(|id| {
        let intent = resolver.find_usbip_bind_intent(id)?;
        if intent.bus_id == bus_id && intent.bus_id != "pending" {
            Some(intent.vm_name.clone())
        } else {
            None
        }
    })
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn dynamic_usbip_bind_intent(
    source: &d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
    bus_id: &str,
) -> d2b_core::bundle_resolver::ResolvedUsbipBindIntent {
    d2b_core::bundle_resolver::ResolvedUsbipBindIntent {
        intent_id: d2b_core::bundle_resolver::intent_id_usbip_bind(
            &source.env,
            &source.vm_name,
            bus_id,
        ),
        bus_id: bus_id.to_owned(),
        vm_name: source.vm_name.clone(),
        env: source.env.clone(),
        lock_path: usbip_lock_path_for_busid(bus_id),
        vendor_product_allowlist: source.vendor_product_allowlist.clone(),
        dynamic_bus_id: true,
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn usbip_lock_path_for_busid(bus_id: &str) -> PathBuf {
    PathBuf::from(format!("/run/d2b/locks/usbip/{bus_id}"))
}

#[cfg(feature = "layer1-bootstrap")]
fn handle_export_broker_audit(
    since: Option<&str>,
    filter: Option<&str>,
    caller_uid: u32,
    caller_gid: u32,
    caller_role: CallerRole,
    audit_log: &AuditLog,
) -> Result<BrokerResponse, BrokerError> {
    if !caller_role_is_admin(&caller_role) {
        audit_log
            .write_entry_with_caller_ids(
                "ExportBrokerAudit",
                caller_uid,
                caller_gid,
                "callable-read-only",
                "audit-log",
                "denied",
            )
            .map_err(|err| BrokerError::Protocol(err.to_string()))?;
        return Err(BrokerError::AuditRequiresAdmin);
    }
    let lines = audit_log
        .export_lines(since, filter)
        .map_err(|err| BrokerError::Protocol(err.to_string()))?;
    audit_log
        .write_entry_with_caller_ids(
            "ExportBrokerAudit",
            caller_uid,
            caller_gid,
            "callable-read-only",
            "audit-log",
            "ok",
        )
        .map_err(|err| BrokerError::Protocol(err.to_string()))?;
    Ok(export_broker_audit_ok_response(lines))
}

#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn validate_socket_parent(path: &Path, test_mode: bool) -> Result<(), RunError> {
    let parent = path.parent().ok_or_else(|| {
        RunError::Usage(format!(
            "socket path must have a parent directory: {}",
            path.display()
        ))
    })?;
    let metadata = fs::symlink_metadata(parent)?;
    if metadata.file_type().is_symlink() {
        return Err(RunError::Usage(format!(
            "socket parent must not be a symlink: {}",
            parent.display()
        )));
    }
    if !metadata.is_dir() {
        return Err(RunError::Usage(format!(
            "socket parent must be a directory: {}",
            parent.display()
        )));
    }
    let expected_uid = if test_mode {
        nix::unistd::Uid::current().as_raw()
    } else {
        0
    };
    if metadata.uid() != expected_uid {
        return Err(RunError::Usage(format!(
            "socket parent owner mismatch for {}: expected uid {} but saw {}",
            parent.display(),
            expected_uid,
            metadata.uid()
        )));
    }
    Ok(())
}

#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn prepare_socket_path(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path).is_ok() {
        path_safe::remove_nofollow(path)?;
    }
    Ok(())
}

#[cfg(feature = "layer1-bootstrap")]
fn run_probe(
    socket_path: PathBuf,
    request: RequestEnvelope,
    expect_response: bool,
) -> Result<(), RunError> {
    let socket = connect_seqpacket(&socket_path)?;
    send_json_frame(socket.as_raw_fd(), &request)?;
    let response = recv_json_frame::<BrokerResponse>(socket.as_raw_fd())?;
    if let Some(response) = response {
        println!(
            "{}",
            serde_json::to_string(&response).map_err(|err| RunError::Protocol(err.to_string()))?
        );
        Ok(())
    } else if expect_response {
        Err(RunError::Protocol(
            "connection closed before response".to_owned(),
        ))
    } else {
        Err(RunError::Protocol(
            "connection closed before export response".to_owned(),
        ))
    }
}

#[cfg(feature = "layer1-bootstrap")]
/// One flag arm of the shared bootstrap parser: it sees the flag name, the
/// remaining arguments, and the cursor, and advances the cursor past the
/// arguments it consumed.
type FlagArm<'a> = dyn FnMut(&str, &[String], &mut usize) -> Result<(), RunError> + 'a;

#[cfg(feature = "layer1-bootstrap")]
fn parse_common_flags(
    rest: &[String],
    extra: &mut FlagArm<'_>,
) -> Result<(PathBuf, Option<u32>), RunError> {
    let mut socket_path = PathBuf::from(DEFAULT_SOCKET_PATH);
    let mut test_uid = None;
    let mut index = 0;
    while index < rest.len() {
        match rest[index].as_str() {
            "--socket-path" => {
                index += 1;
                socket_path = PathBuf::from(expect_arg(rest, index, "--socket-path")?);
            }
            "--test-uid" => {
                index += 1;
                test_uid = Some(
                    expect_arg(rest, index, "--test-uid")?
                        .parse()
                        .map_err(|_| RunError::Usage("invalid --test-uid".to_owned()))?,
                );
            }
            other => extra(other, rest, &mut index)?,
        }
        index += 1;
    }
    Ok((socket_path, test_uid))
}

#[cfg(feature = "layer1-bootstrap")]
fn parse_probe_flags(rest: Vec<String>) -> Result<(PathBuf, Option<u32>), RunError> {
    parse_common_flags(&rest, &mut |flag, _, _| {
        Err(RunError::Usage(format!("unknown probe flag: {flag}")))
    })
}

#[cfg(feature = "layer1-bootstrap")]
fn parse_stub_flags(rest: &[String]) -> Result<(PathBuf, Option<u32>, String), RunError> {
    let mut operation = None;
    let (socket_path, test_uid) = parse_common_flags(rest, &mut |flag, rest, index| {
        if flag != "--operation" {
            return Err(RunError::Usage(format!("unknown probe-stub flag: {flag}")));
        }
        *index += 1;
        operation = Some(expect_arg(rest, *index, "--operation")?.to_owned());
        Ok(())
    })?;
    Ok((
        socket_path,
        test_uid,
        operation.ok_or_else(|| RunError::Usage("missing --operation".to_owned()))?,
    ))
}

#[cfg(feature = "layer1-bootstrap")]
fn parse_export_flags(rest: &[String]) -> Result<(PathBuf, Option<u32>, CallerRole), RunError> {
    let mut caller_role = None;
    let (socket_path, test_uid) = parse_common_flags(rest, &mut |flag, rest, index| {
        if flag != "--caller-role" {
            return Err(RunError::Usage(format!(
                "unknown probe-export-audit flag: {flag}"
            )));
        }
        *index += 1;
        caller_role = crate::bootstrap::wire::caller_role_from_cli(expect_arg(
            rest,
            *index,
            "--caller-role",
        )?);
        Ok(())
    })?;
    Ok((
        socket_path,
        test_uid,
        caller_role.ok_or_else(|| RunError::Usage("missing --caller-role".to_owned()))?,
    ))
}

fn expect_arg<'a>(args: &'a [String], index: usize, flag: &str) -> Result<&'a str, RunError> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| RunError::Usage(format!("missing value for {flag}")))
}

fn caller_role_is_admin(caller_role: &CallerRole) -> bool {
    #[cfg(feature = "layer1-bootstrap")]
    {
        caller_role.is_admin_uid()
    }
    #[cfg(not(feature = "layer1-bootstrap"))]
    {
        matches!(
            caller_role,
            CallerRole::AdminUid { .. } | CallerRole::RootUid { .. }
        )
    }
}

impl BrokerError {
    #[allow(clippy::too_many_arguments)]
    fn audit(
        &self,
        audit_log: &AuditLog,
        caller_uid: u32,
        caller_gid: u32,
        caller_role: &CallerRole,
        audit_context: &DispatchAuditContext,
        #[cfg(not(feature = "layer1-bootstrap"))] resolver: Option<&BundleResolver>,
        operation: &str,
        opaque_target_id: &str,
    ) -> io::Result<()> {
        #[cfg(not(feature = "layer1-bootstrap"))]
        let bundle_metadata = audit_bundle_metadata(resolver);
        #[cfg(not(feature = "layer1-bootstrap"))]
        let authz_result = caller_role_authz_result(caller_role);
        #[cfg(feature = "layer1-bootstrap")]
        let bundle_metadata = AuditBundleMetadata {
            bundle_version: "unknown",
            bundle_hash: "",
        };
        #[cfg(feature = "layer1-bootstrap")]
        let authz_result = "launcher";
        #[cfg(feature = "layer1-bootstrap")]
        let _ = caller_role;
        #[cfg(not(feature = "layer1-bootstrap"))]
        let supplied_join = audit_context
            .audit_join
            .as_ref()
            .map(|join| (join.zone_id.as_str(), join.operation_identity.as_str()));
        #[cfg(feature = "layer1-bootstrap")]
        let supplied_join = None;
        let has_typed_terminal = matches!(
            self,
            Self::Unimplemented { .. }
                | Self::UnknownOperation { .. }
                | Self::UsbipDeviceNotAllowed { .. }
                | Self::UsbipPolicyMismatch { .. }
                | Self::CoexistenceRefused { .. }
                | Self::NftScriptParseFailed(_)
                | Self::CarveoutOrderingViolation(_)
                | Self::NftablesDriftDetected { .. }
                | Self::StoreSyncFailed { .. }
                | Self::SwtpmDirHardening { .. }
                | Self::RequestValidation { .. }
                | Self::ProfileOperationRefused { .. }
        );
        let has_request_payload = !audit_context
            .request_fields
            .as_object()
            .is_some_and(|object| object.is_empty());
        if !has_typed_terminal && has_request_payload {
            let (typed_public_operation_id, typed_scope_id) = supplied_join
                .map_or((operation, opaque_target_id), |(_, operation_identity)| {
                    (operation_identity, operation_identity)
                });
            audit_log.record_with_join(
                operation,
                typed_public_operation_id,
                caller_uid,
                caller_gid,
                audit_context.peer_pid,
                audit_context.peer_role.as_str(),
                authz_result,
                "",
                typed_scope_id,
                audit_context.verb.as_str(),
                audit_context.request_fields.clone(),
                "error",
                Some("broker-error"),
                None,
                bundle_metadata.bundle_version,
                bundle_metadata.bundle_hash,
                audit_context.duration_us(),
                Some(serde_json::json!({ "error_class": "broker-error" })),
                supplied_join,
            )?;
        }
        match self {
            Self::Unimplemented {
                operation: op,
                target_wave,
            } => {
                // Legacy short-record (preserved for the export-audit /
                // socket-acl gates).
                audit_log.write_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "stubbed-unimplemented",
                    opaque_target_id,
                    "denied",
                )?;
                // Typed [`OpAuditRecord`] record for every decision.
                audit_log.record_with_join(
                    op,
                    opaque_target_id,
                    caller_uid,
                    caller_gid,
                    audit_context.peer_pid,
                    audit_context.peer_role.as_str(),
                    authz_result,
                    "",
                    opaque_target_id,
                    audit_context.verb.as_str(),
                    audit_context.request_fields.clone(),
                    "errored",
                    Some("w3-pending-typed-wire"),
                    None,
                    bundle_metadata.bundle_version,
                    bundle_metadata.bundle_hash,
                    audit_context.duration_us(),
                    Some(serde_json::json!({ "target_wave": target_wave })),
                    supplied_join,
                )?;
            }
            Self::UnknownOperation { operation: op } => {
                audit_log.write_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "unknown-operation",
                    opaque_target_id,
                    "denied",
                )?;
                audit_log.record_with_join(
                    op,
                    opaque_target_id,
                    caller_uid,
                    caller_gid,
                    audit_context.peer_pid,
                    audit_context.peer_role.as_str(),
                    authz_result,
                    "",
                    opaque_target_id,
                    audit_context.verb.as_str(),
                    audit_context.request_fields.clone(),
                    "denied-unknown",
                    Some("unknown-operation"),
                    None,
                    bundle_metadata.bundle_version,
                    bundle_metadata.bundle_hash,
                    audit_context.duration_us(),
                    Some(serde_json::json!({
                        "reason": "broker does not yet implement USBIP live-device-routing ops",
                        "target_wave": "W6"
                    })),
                    supplied_join,
                )?;
            }
            Self::MinijailValidation { reason } => {
                audit_log.write_error_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "minijail-validation-failed",
                    opaque_target_id,
                    "Broker.MinijailValidation",
                    reason,
                )?;
            }
            Self::NoPidfd { runner_id } => {
                audit_log.write_error_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "runner-pidfd-missing",
                    runner_id,
                    "Broker.NoPidfd",
                    &format!("no pidfd registered for runner `{runner_id}`"),
                )?;
            }
            Self::BundleResolverUnavailable => {
                audit_log.write_error_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "bundle-resolver-unavailable",
                    opaque_target_id,
                    "Broker.BundleResolverUnavailable",
                    "Broker started without a loadable bundle at ServerConfig.bundle_path. Bundle-dependent real-wire ops cannot resolve their BundleOpId refs.",
                )?;
            }
            Self::BundleTampered { path, reason } => {
                audit_log.write_error_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "bundle-tampered",
                    opaque_target_id,
                    "Broker.BundleTampered",
                    &format!("bundle artifact {path} failed tamper-resistance check: {reason}"),
                )?;
            }
            Self::BundleIntentMissing { kind, intent_id } => {
                audit_log.write_error_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "bundle-intent-missing",
                    intent_id,
                    "Broker.BundleIntentMissing",
                    &format!("no {kind} intent in the trusted bundle for opaque id `{intent_id}`"),
                )?;
            }
            Self::UsbipDeviceNotAllowed {
                busid,
                vendor,
                product,
            } => {
                audit_log.write_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "usbip-device-not-allowed",
                    busid,
                    "denied",
                )?;
                audit_log.record_with_join(
                    operation,
                    busid,
                    caller_uid,
                    caller_gid,
                    audit_context.peer_pid,
                    audit_context.peer_role.as_str(),
                    authz_result,
                    "",
                    busid,
                    audit_context.verb.as_str(),
                    audit_context.request_fields.clone(),
                    "denied-refused",
                    Some("usbip-device-not-allowed"),
                    None,
                    bundle_metadata.bundle_version,
                    bundle_metadata.bundle_hash,
                    audit_context.duration_us(),
                    Some(serde_json::json!({
                        "vendor": format!("{vendor:04x}"),
                        "product": format!("{product:04x}"),
                    })),
                    supplied_join,
                )?;
            }
            Self::UsbipPolicyMismatch { busid, reason } => {
                audit_log.write_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "usbip-policy-mismatch",
                    busid,
                    "denied",
                )?;
                audit_log.record_with_join(
                    operation,
                    busid,
                    caller_uid,
                    caller_gid,
                    audit_context.peer_pid,
                    audit_context.peer_role.as_str(),
                    authz_result,
                    "",
                    busid,
                    audit_context.verb.as_str(),
                    audit_context.request_fields.clone(),
                    "denied-policy",
                    Some("usbip-policy-mismatch"),
                    None,
                    bundle_metadata.bundle_version,
                    bundle_metadata.bundle_hash,
                    audit_context.duration_us(),
                    Some(serde_json::json!({
                        "reason": reason,
                    })),
                    supplied_join,
                )?;
            }
            Self::UsbipLockConflict { busid, owner } => {
                audit_log.write_error_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "usbip-lock-conflict",
                    busid,
                    "Broker.UsbipLockConflict",
                    &format!("UsbipBind refused: busid {busid} is already claimed by {owner}"),
                )?;
            }
            Self::UsbipDeviceAbsent { busid } => {
                audit_log.write_error_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "usbip-device-absent",
                    busid,
                    "Broker.UsbipDeviceAbsent",
                    &format!("UsbipBind refused: USB device {busid} is not present in sysfs"),
                )?;
            }
            Self::LiveHandler(message) => {
                // Surface the operator-facing root cause (errno / path /
                // stderr) in the broker journal, not just the
                // `Broker.LiveHandlerFailed` wrapper kind. The same
                // detail is recorded in the audit log; an operator
                // reading `journalctl -u d2b-broker` should not
                // have to cross-reference the audit jsonl to learn why a
                // runner spawn failed.
                tracing::warn!(
                    operation = operation,
                    error_kind = "Broker.LiveHandlerFailed",
                    detail = %message,
                    "broker live-handler op failed"
                );
                audit_log.write_error_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "live-handler-error",
                    opaque_target_id,
                    "Broker.LiveHandlerFailed",
                    message,
                )?;
            }
            Self::Protocol(message) => {
                audit_log.write_error_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "protocol-error",
                    opaque_target_id,
                    "Broker.Protocol",
                    message,
                )?;
            }
            Self::ProfileOperationRefused { profile, operation } => {
                audit_log.write_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "profile-operation-denied",
                    opaque_target_id,
                    "denied",
                )?;
                audit_log.record_with_join(
                    operation,
                    opaque_target_id,
                    caller_uid,
                    caller_gid,
                    audit_context.peer_pid,
                    audit_context.peer_role.as_str(),
                    authz_result,
                    "",
                    opaque_target_id,
                    audit_context.verb.as_str(),
                    audit_context.request_fields.clone(),
                    "denied-refused",
                    Some("profile-operation-denied"),
                    None,
                    bundle_metadata.bundle_version,
                    bundle_metadata.bundle_hash,
                    audit_context.duration_us(),
                    Some(serde_json::json!({ "profile": profile.as_str() })),
                    supplied_join,
                )?;
            }
            Self::CoexistenceRefused { manager, rationale } => {
                audit_log.write_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "coexistence-refused",
                    opaque_target_id,
                    "denied",
                )?;
                audit_log.record_with_join(
                    operation,
                    opaque_target_id,
                    caller_uid,
                    caller_gid,
                    audit_context.peer_pid,
                    audit_context.peer_role.as_str(),
                    authz_result,
                    "",
                    opaque_target_id,
                    audit_context.verb.as_str(),
                    audit_context.request_fields.clone(),
                    "denied-refused",
                    Some("coexistence-refused"),
                    None,
                    bundle_metadata.bundle_version,
                    bundle_metadata.bundle_hash,
                    audit_context.duration_us(),
                    Some(serde_json::json!({
                        "manager": format!("{manager:?}"),
                        "rationale": rationale,
                    })),
                    supplied_join,
                )?;
            }
            Self::NftScriptParseFailed(detail) => {
                audit_log.write_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "nft-script-parse-failed",
                    opaque_target_id,
                    "errored",
                )?;
                audit_log.record_with_join(
                    operation,
                    opaque_target_id,
                    caller_uid,
                    caller_gid,
                    audit_context.peer_pid,
                    audit_context.peer_role.as_str(),
                    authz_result,
                    "",
                    opaque_target_id,
                    audit_context.verb.as_str(),
                    audit_context.request_fields.clone(),
                    "errored",
                    Some("nft-script-parse-failed"),
                    None,
                    bundle_metadata.bundle_version,
                    bundle_metadata.bundle_hash,
                    audit_context.duration_us(),
                    Some(serde_json::json!({ "detail": detail })),
                    supplied_join,
                )?;
            }
            Self::CarveoutOrderingViolation(detail) => {
                audit_log.write_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "nft-carveout-ordering-violation",
                    opaque_target_id,
                    "denied",
                )?;
                audit_log.record_with_join(
                    operation,
                    opaque_target_id,
                    caller_uid,
                    caller_gid,
                    audit_context.peer_pid,
                    audit_context.peer_role.as_str(),
                    authz_result,
                    "",
                    opaque_target_id,
                    audit_context.verb.as_str(),
                    audit_context.request_fields.clone(),
                    "denied-refused",
                    Some("nft-carveout-ordering-violation"),
                    None,
                    bundle_metadata.bundle_version,
                    bundle_metadata.bundle_hash,
                    audit_context.duration_us(),
                    Some(serde_json::json!({ "detail": detail })),
                    supplied_join,
                )?;
            }
            Self::NftablesDriftDetected { expected, observed } => {
                audit_log.write_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "nftables-drift-detected",
                    opaque_target_id,
                    "denied",
                )?;
                audit_log.record_with_join(
                    operation,
                    opaque_target_id,
                    caller_uid,
                    caller_gid,
                    audit_context.peer_pid,
                    audit_context.peer_role.as_str(),
                    authz_result,
                    "",
                    opaque_target_id,
                    audit_context.verb.as_str(),
                    audit_context.request_fields.clone(),
                    "denied-refused",
                    Some("nftables-drift-detected"),
                    None,
                    bundle_metadata.bundle_version,
                    bundle_metadata.bundle_hash,
                    audit_context.duration_us(),
                    Some(serde_json::json!({
                        "expected": expected,
                        "observed": observed,
                    })),
                    supplied_join,
                )?;
            }
            Self::OtelHostBridgeIntentInvalid {
                intent_vm,
                expected_obs_vm,
            } => {
                audit_log.write_error_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "otel-host-bridge-intent-invalid",
                    opaque_target_id,
                    "Broker.OtelHostBridgeIntentInvalid",
                    &format!(
                        "OtelHostBridge runner intent points at VM `{intent_vm}` but the trusted bundle declares the obs VM as `{expected_obs_vm}` (closed-set)"
                    ),
                )?;
            }
            Self::SpawnRunnerIntentMismatch {
                field,
                requested,
                resolved,
            } => {
                audit_log.write_error_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "spawn-runner-intent-mismatch",
                    opaque_target_id,
                    "Broker.SpawnRunnerIntentMismatch",
                    &format!(
                        "SpawnRunner {field} mismatch: request `{requested}` does not match trusted bundle intent `{resolved}`"
                    ),
                )?;
            }
            // The StoreSync dispatch arm already wrote the signed terminal
            // `OperationFields::StoreSync` record (ADR 0027: exactly one
            // terminal record per attempt). Writing the generic error entry
            // here would emit a duplicate, so this is a deliberate no-op.
            Self::StoreSyncFailed { .. } => {}
            // The SpawnRunner dispatch arm already wrote the terminal
            // path-free `PrepareSwtpmDir` record for the fail-closed
            // hardening step; writing the generic error entry here would
            // duplicate it, so this is a deliberate no-op (mirrors
            // `StoreSyncFailed`).
            Self::SwtpmDirHardening { .. } => {}
            Self::RequestValidation {
                operation: op,
                reason,
            } => {
                audit_log.write_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "request-validation-failed",
                    opaque_target_id,
                    "denied",
                )?;
                audit_log.record_with_join(
                    op,
                    opaque_target_id,
                    caller_uid,
                    caller_gid,
                    audit_context.peer_pid,
                    audit_context.peer_role.as_str(),
                    authz_result,
                    "",
                    opaque_target_id,
                    audit_context.verb.as_str(),
                    audit_context.request_fields.clone(),
                    "denied-refused",
                    Some("request-validation-failed"),
                    None,
                    bundle_metadata.bundle_version,
                    bundle_metadata.bundle_hash,
                    audit_context.duration_us(),
                    Some(serde_json::json!({ "reason": reason })),
                    supplied_join,
                )?;
            }
            Self::IpcRateLimited => {
                audit_log.write_entry_with_caller_ids(
                    operation,
                    caller_uid,
                    caller_gid,
                    "ipc-rate-limited",
                    opaque_target_id,
                    "denied",
                )?;
            }
            _ => {}
        }
        Ok(())
    }

    fn into_response(self) -> BrokerResponse {
        match self {
            Self::Unimplemented {
                operation,
                target_wave,
            } => unimplemented_response(operation, target_wave),
            Self::UnknownOperation { operation } => error_response(
                "unknown-operation",
                operation,
                Some("W6"),
                &format!("{operation} is USBIP live-device-routing and is not yet implemented."),
                "USBIP live-device-routing is not yet implemented; only `UsbipBindFirewallRule` skeleton support ships today.",
            ),
            Self::AuditRequiresAdmin => authz_audit_requires_admin_response(),
            Self::HostShutdownRestricted => error_response(
                "host-shutdown-restricted",
                "HostShutdown",
                None,
                "host shutdown capability permits only stop",
                "retry only the stop operation during host shutdown",
            ),
            Self::MinijailValidation { reason } => error_response(
                "Broker.MinijailValidation",
                "SpawnRunner",
                Some("W17"),
                &reason,
                "Fix the bundle's minijail profile invariants and retry SpawnRunner.",
            ),
            Self::NoPidfd { runner_id } => error_response(
                "Broker.NoPidfd",
                "SignalRunner",
                Some("W4"),
                &format!("no pidfd registered for runner `{runner_id}`"),
                "Open or spawn the runner first so the broker can retain a pidfd for signaling.",
            ),
            Self::BundleResolverUnavailable => error_response(
                "Broker.BundleResolverUnavailable",
                "BundleResolver",
                Some("W12"),
                "broker operation failed; details are available only in the redacted audit channel",
                "Restore the trusted bundle at the broker-configured bundle path and retry; the broker reloads the bundle on the next request.",
            ),
            Self::BundleTampered { .. } => error_response(
                "bundle-tampered",
                "BundleResolver",
                None,
                "Trusted bundle failed integrity checks; privileged operations are refused.",
                "rebuild the bundle from a trusted source (nixos-rebuild switch) and verify ownership root:d2bd 0640; refuse to run mutating verbs until the bundle is restored",
            ),
            Self::BundleIntentMissing { kind, .. } => error_response(
                "Broker.BundleIntentMissing",
                "BundleResolver",
                Some("W12"),
                &format!("trusted bundle does not contain the requested {kind} intent"),
                "broker operation failed; details are available only in the redacted audit channel",
            ),
            Self::UsbipDeviceNotAllowed { .. } => error_response(
                "Broker.UsbipDeviceNotAllowed",
                "UsbipBind",
                Some("W6"),
                "UsbipBind refused because the selected device is outside the trusted bundle allowlist",
                "Allow the device's vendor:product in host.json or bind an approved USB device before retrying.",
            ),
            Self::UsbipPolicyMismatch { reason, .. } => error_response(
                "Broker.UsbipPolicyMismatch",
                "UsbipBind",
                None,
                &format!(
                    "UsbipBind refused before device exposure because the required USB policy check failed: {reason}"
                ),
                "Fix the USBIP declaration, physical port/topology, and vendor:product allowlist, rebuild the trusted bundle, then retry.",
            ),
            Self::UsbipLockConflict { busid, .. } => error_response(
                "Broker.UsbipLockConflict",
                "UsbipBind",
                None,
                &format!("UsbipBind refused: busid {busid} is already claimed by another VM"),
                "Stop the VM currently holding this busid or use `d2b usb detach` to release the claim, then retry.",
            ),
            Self::UsbipDeviceAbsent { busid } => error_response(
                "Broker.UsbipDeviceAbsent",
                "UsbipBind",
                None,
                &format!("UsbipBind refused: USB device {busid} is not present in sysfs"),
                "Confirm the physical device is connected and recognized by the host kernel, then retry.",
            ),
            Self::LiveHandler(message) => error_response(
                "Broker.LiveHandlerFailed",
                "LiveHandler",
                Some("W12"),
                &public_live_handler_message(&message),
                "Inspect the broker audit log for the failing live executor's underlying syscall.",
            ),
            Self::CoexistenceRefused { manager, rationale } => error_response(
                "Broker.CoexistenceRefused",
                "ApplyNftables",
                Some("W12"),
                &format!(
                    "ApplyNftables refused by host.json firewall coexistence policy for {manager:?}: {rationale}"
                ),
                "Adjust host.json firewallCoexistencePolicy or remove the conflicting managed firewall before retrying.",
            ),
            Self::NftScriptParseFailed(detail) => error_response(
                "Broker.NftScriptParseFailed",
                "ApplyNftables",
                Some("W12"),
                &format!(
                    "ApplyNftables refused because the resolver-emitted `inet d2b` script could not be parsed: {detail}"
                ),
                "Inspect the emitted nftables script and regenerate the trusted bundle before retrying.",
            ),
            Self::CarveoutOrderingViolation(detail) => error_response(
                "Broker.CarveoutOrderingViolation",
                "ApplyNftables",
                Some("W12"),
                &format!(
                    "ApplyNftables refused because a specific USBIP carve-out would be shadowed by a broader forward-chain rule: {detail}"
                ),
                "Reorder the emitted forward-chain rules so per-busid carve-outs sit before any broad allow/drop rules.",
            ),
            Self::NftablesDriftDetected { expected, observed } => error_response(
                "Broker.NftablesDriftDetected",
                "ApplyNftables",
                Some("W12"),
                &format!(
                    "ApplyNftables refused because the canonical `inet d2b` hash no longer matches host.json (expected={expected}, observed={observed})"
                ),
                "Investigate out-of-band nftables changes or refresh host.json with the last applied table hash before retrying.",
            ),
            Self::Protocol(message) => error_response(
                "Broker.Protocol",
                "Broker",
                None,
                &public_protocol_message(&message),
                "Inspect the private broker socket framing and retry.",
            ),
            Self::PeerCredentialRefused { operation } => error_response(
                "Broker.PeerCredentialRefused",
                operation,
                None,
                "broker peer credential check refused the private request",
                "Ensure only d2bd connects to d2b-broker.socket; restart d2bd after host credential changes.",
            ),
            Self::ProfileOperationRefused { profile, operation } => error_response(
                "Broker.ProfileOperationDenied",
                operation,
                None,
                &format!(
                    "broker profile `{}` does not admit this effect class",
                    profile.as_str()
                ),
                "Use the fixed effect adapter for the authority that owns this operation; the active broker profile cannot be changed by a request.",
            ),
            Self::OtelHostBridgeIntentInvalid {
                intent_vm,
                expected_obs_vm,
            } => error_response(
                "Broker.OtelHostBridgeIntentInvalid",
                "SpawnRunner",
                Some("P1"),
                &format!(
                    "OtelHostBridge runner intent points at VM `{intent_vm}` but the trusted bundle declares the obs VM as `{expected_obs_vm}` (closed-set)"
                ),
                "Rebuild the bundle so the OtelHostBridge runner intent's vm_name matches manifest._observability.vmName, then retry SpawnRunner.",
            ),
            Self::SpawnRunnerIntentMismatch {
                field,
                requested,
                resolved,
            } => {
                tracing::warn!(field, "SpawnRunner trusted intent mismatch");
                error_response(
                    "Broker.SpawnRunnerIntentMismatch",
                    "SpawnRunner",
                    Some("P1"),
                    &format!(
                        "SpawnRunner {field} mismatch: request `{requested}` does not match trusted bundle intent `{resolved}`"
                    ),
                    "Use the BundleOpId that matches the requested VM/role; daemon and broker versions may be out of sync.",
                )
            }
            Self::StoreSyncFailed {
                error_stage,
                message,
            } => error_response(
                "Broker.StoreSyncFailed",
                "StoreSync",
                None,
                &format!("StoreSync failed ({error_stage}): {message}"),
                "Inspect the signed StoreSync audit record (operation_fields.error_stage) for the failing phase; retry after resolving the underlying condition.",
            ),
            Self::SwtpmDirHardening { reason, .. } => error_response(
                "Broker.SwtpmDirHardening",
                "PrepareSwtpmDir",
                None,
                // PATH-FREE: only the closed-set reason slug reaches the
                // wire envelope.
                &format!("swtpm-dir hardening refused: {reason}"),
                "Inspect the signed PrepareSwtpmDir audit record (operation_fields.fail_reason) for the refusal cause; do NOT delete or recreate the per-VM swtpm state dir - that destroys the TPM2 NVRAM and forces IdP re-enrollment.",
            ),
            Self::RequestValidation { operation, reason } => error_response(
                "Broker.RequestValidation",
                operation,
                None,
                &format!("broker request validation failed: {reason}"),
                "Regenerate the daemon request from the trusted d2b bundle and retry.",
            ),
            Self::IpcRateLimited => error_response(
                "Broker.IpcRateLimited",
                "Broker",
                None,
                "broker IPC request rate limit exceeded",
                "Retry after the current rate-limit window; persistent failures indicate a daemon bug or local DoS.",
            ),
        }
    }
}

fn public_live_handler_message(message: &str) -> String {
    let lower = message.to_ascii_lowercase();
    if lower.contains("usbip") || lower.contains("/sys/bus/usb") {
        "privileged USB host operation failed; details are available only in the broker audit log"
            .to_owned()
    } else {
        "privileged host operation failed; details are available only in the broker audit log"
            .to_owned()
    }
}

fn public_protocol_message(message: &str) -> String {
    if message.contains("usb")
        || message.contains("USB")
        || message.contains('/')
        || message.contains("..")
    {
        "broker rejected a malformed private request".to_owned()
    } else {
        message.to_owned()
    }
}

fn profile_capabilities(profile: BrokerProfile) -> Vec<String> {
    match profile {
        BrokerProfile::Host => CAPABILITIES.iter().map(|item| (*item).to_owned()).collect(),
        BrokerProfile::Guest => profile
            .operations()
            .iter()
            .map(|item| item.as_str().to_owned())
            .collect(),
    }
}

/// Render one broker-side helper failure for a kernel refusal detail:
/// the closed-set wire kind plus the operator-facing message, by the same
/// path-free contract the wire error envelope uses. `BrokerError`'s
/// `Debug` is redacted, so the kernel surface cannot format it directly.
#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn broker_error_kernel_detail(error: BrokerError) -> String {
    match error.into_response() {
        BrokerResponse::Error(response) => {
            format!("{}: {}", response.kind, response.message)
        }
        _ => "spawn-process helper failed".to_owned(),
    }
}

fn hello_ok_response(profile: BrokerProfile) -> BrokerResponse {
    #[cfg(feature = "layer1-bootstrap")]
    {
        BrokerResponse::HelloOk {
            server_version: "0.0.0-w2-bootstrap".to_owned(),
            selected_version: "0.0.0-test".to_owned(),
            capabilities: profile_capabilities(profile),
        }
    }
    #[cfg(not(feature = "layer1-bootstrap"))]
    {
        BrokerResponse::Hello(d2b_contracts_broker::broker_wire::HelloResponse {
            server_version: "0.0.0-w2".to_owned(),
            selected_version: "0.0.0-w2".to_owned(),
            capabilities: profile_capabilities(profile),
        })
    }
}

#[cfg(feature = "layer1-bootstrap")]
fn export_broker_audit_ok_response(lines: Vec<String>) -> BrokerResponse {
    BrokerResponse::ExportBrokerAuditOk { lines }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn export_broker_audit_ok_response(
    page: d2b_contracts_broker::broker_wire::ExportBrokerAuditResponse,
) -> BrokerResponse {
    BrokerResponse::ExportBrokerAudit(page)
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn ack_response(operation: &str) -> BrokerResponse {
    BrokerResponse::Ack(d2b_contracts_broker::broker_wire::AckResponse {
        accepted: true,
        operation: operation.to_owned(),
    })
}

fn unimplemented_response(operation: &str, target_wave: &str) -> BrokerResponse {
    error_response(
        "Broker.Unimplemented",
        operation,
        Some(target_wave),
        &format!("{operation} is intentionally stubbed and performs no host mutation."),
        &format!(
            "The privileged host implementation for {operation} is not yet available; retry once it lands."
        ),
    )
}

fn authz_audit_requires_admin_response() -> BrokerResponse {
    error_response(
        "authz-audit-requires-admin",
        "ExportBrokerAudit",
        None,
        "ExportBrokerAudit requires caller_role: AdminUid { uid } from d2bd.",
        "Have d2bd verify d2b.site.adminUsers before forwarding the audit export request.",
    )
}

fn error_response(
    kind: &str,
    operation: &str,
    target_wave: Option<&str>,
    message: &str,
    remediation: &str,
) -> BrokerResponse {
    let message = redact_public_detail(message);
    let remediation = redact_public_detail(remediation);
    #[cfg(feature = "layer1-bootstrap")]
    {
        BrokerResponse::Error {
            kind: kind.to_owned(),
            operation: operation.to_owned(),
            target_wave: target_wave.map(str::to_owned),
            message,
            remediation,
        }
    }
    #[cfg(not(feature = "layer1-bootstrap"))]
    {
        BrokerResponse::Error(d2b_contracts_broker::broker_wire::BrokerErrorResponse {
            kind: kind.to_owned(),
            operation: operation.to_owned(),
            target_wave: target_wave.map(str::to_owned),
            message,
            action: remediation,
        })
    }
}

fn redact_public_detail(detail: &str) -> String {
    if d2b_contracts_resource::v3::is_canonical_digest(detail) {
        return detail.to_owned();
    }
    if matches!(
        detail,
        "broker operation failed; details are available only in the redacted audit channel"
            | "Trusted bundle failed integrity checks; privileged operations are refused."
            | "privileged host operation failed; details are available only in the broker audit log"
            | "privileged USB host operation failed; details are available only in the broker audit log"
            | "Inspect the broker audit log for the failing live executor's underlying syscall."
            | "Restore the trusted bundle at the broker-configured bundle path and retry; the broker reloads the bundle on the next request."
            | "rebuild the bundle from a trusted source (nixos-rebuild switch) and verify ownership root:d2bd 0640; refuse to run mutating verbs until the bundle is restored"
    ) || detail.starts_with("trusted bundle does not contain the requested ")
    {
        return detail.to_owned();
    }
    d2b_contracts_resource::v3::canonical_digest("d2b:broker-public-error:v1", detail.as_bytes())
}

/// Start the background SIGCHLD reaper on the broker's own reactor.
///
/// The loop waits on async signals, so it shares the reactor the accept loop
/// runs on rather than owning a runtime of its own. The pidfd registry Mutex
/// is safe to lock from a tokio task: it is never held across an await, and
/// the reaper's `waitid` calls are non-blocking (`WNOHANG`).
#[cfg(not(feature = "layer1-bootstrap"))]
fn start_sigchld_reaper(runtime: &tokio::runtime::Runtime, audit_log: Arc<AuditLog>) {
    // Publish the audit handle so the targeted post-spawn reap can
    // write the same forensic ChildReaped record the SIGCHLD loop does.
    let _ = broker_audit_log_handle().set(Arc::clone(&audit_log));

    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    let sigchld = runtime.block_on(async {
        use tokio::signal::unix::{SignalKind, signal};

        signal(SignalKind::child())
    });

    let mut sigchld = match sigchld {
        Ok(s) => s,
        Err(err) => {
            tracing::error!(
                error = %err,
                "broker: failed to install SIGCHLD handler; pidfd reap loop disabled"
            );
            return;
        }
    };

    runtime.spawn(async move {
        loop {
            if sigchld.recv().await.is_none() {
                break;
            }
            reap_all_pidfds(audit_log.as_ref()).await;
        }
    });
}

/// Iterate the pidfd registry and call
/// `waitid(P_PIDFD, WEXITED|WNOHANG)` on each entry. Entries whose child
/// has exited are removed from the registry, a `ChildReaped` notification
/// is pushed to the ring buffer, and a forensics record is appended to the
/// audit log.
///
/// Async because the reap notification push waits on the buffer lock (plan
/// U8): the primary reaper's notification is never dropped under a
/// concurrent reader. The `waitid` probes themselves stay non-blocking
/// (`WNOHANG`), so no await is held across a syscall.
#[cfg(not(feature = "layer1-bootstrap"))]
async fn reap_all_pidfds(audit_log: &AuditLog) {
    use d2b_contracts_broker::broker_wire::{
        ChildExitKind, ChildExitStatus, ChildReapedNotification,
    };
    use nix::errno::Errno;
    use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};

    let runner_ids: Vec<String> = runner_pidfds().keys();

    for runner_id in runner_ids {
        let Some(pidfd_dup) = runner_pidfds().duplicate(&runner_id) else {
            // Absent (concurrent deregistration) or un-duplicable; the cell
            // accessor warns on dup failure, absence is a silent skip.
            continue;
        };

        let wait_flags = WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG;
        match waitid(Id::PIDFd(pidfd_dup.as_fd()), wait_flags) {
            Ok(WaitStatus::Exited(pid, code)) => {
                let notif = ChildReapedNotification {
                    runner_id: runner_id.clone(),
                    pid: pid.as_raw(),
                    exit_status: ChildExitStatus {
                        kind: ChildExitKind::Exited,
                        code: Some(code),
                        signal: None,
                    },
                    reaped_at_ms: reaped_at_ms_now(),
                };
                remove_and_notify_async(&runner_id, notif, audit_log).await;
            }
            Ok(WaitStatus::Signaled(pid, sig, _)) => {
                let sig_num = sig as libc::c_int;
                let notif = ChildReapedNotification {
                    runner_id: runner_id.clone(),
                    pid: pid.as_raw(),
                    exit_status: ChildExitStatus {
                        kind: if sig_num == libc::SIGKILL {
                            ChildExitKind::Killed
                        } else {
                            ChildExitKind::Signaled
                        },
                        code: None,
                        signal: Some(sig_num),
                    },
                    reaped_at_ms: reaped_at_ms_now(),
                };
                remove_and_notify_async(&runner_id, notif, audit_log).await;
            }
            Ok(WaitStatus::StillAlive) | Ok(_) => {}
            Err(Errno::ECHILD) => {
                tracing::debug!(
                    runner_id = %runner_id,
                    "reap_all_pidfds: ECHILD (already reaped); removing stale registry entry"
                );
                let _ = remove_runner_registries(&runner_id);
            }
            Err(err) => {
                tracing::warn!(runner_id = %runner_id, error = %err, "reap_all_pidfds: waitid failed");
            }
        }
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn reaped_at_ms_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Process-global handle to the broker's audit log, set when the
/// SIGCHLD reaper starts. Lets the targeted post-spawn reap
/// ([`targeted_reap_runner`]) write the same forensic `ChildReaped`
/// audit record the SIGCHLD loop writes, without threading an
/// `AuditLog` reference through the `DispatchBackend::spawn_runner`
/// trait boundary.
#[cfg(not(feature = "layer1-bootstrap"))]
fn broker_audit_log_handle() -> &'static OnceLock<Arc<AuditLog>> {
    static HANDLE: OnceLock<Arc<AuditLog>> = OnceLock::new();
    &HANDLE
}

/// Targeted, non-blocking reap of a SINGLE broker-spawned child,
/// keyed by its pidfd. Closes two zombie-leak windows around pidfd
/// registration that the SIGCHLD loop alone cannot guarantee to cover:
///
/// 1. the child exits in the window between `clone3` and the registry
///    insertion (its SIGCHLD may have already been coalesced/consumed
///    by a reap pass that ran before the entry existed); and
/// 2. registration itself fails and the broker is about to drop the
///    pidfd - without an explicit reap the child would zombie.
///
/// `waitid(P_PIDFD, WEXITED|WNOHANG)` is inherently generation-exact:
/// a pidfd can never refer to a reused PID, so this is the
/// strongest possible start-time/generation key (no separate
/// start_time_ticks comparison is required). On a real exit the child
/// is reaped, removed from the registry, a `ChildReaped` notification
/// is pushed for the daemon's rollback to confirm, and a forensic
/// audit record is appended. `ECHILD` means the SIGCHLD loop already
/// reaped it (also a clean terminal state); `StillAlive` leaves the
/// child for the SIGCHLD loop.
#[cfg(not(feature = "layer1-bootstrap"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TargetedReapOutcome {
    Reaped,
    AlreadyReaped,
    StillAlive,
    Failed,
}

#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) fn targeted_reap_runner(
    runner_id: &str,
    pidfd: std::os::fd::BorrowedFd<'_>,
) -> TargetedReapOutcome {
    use d2b_contracts_broker::broker_wire::{
        ChildExitKind, ChildExitStatus, ChildReapedNotification,
    };
    use nix::errno::Errno;
    use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};

    let wait_flags = WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG;
    match waitid(Id::PIDFd(pidfd), wait_flags) {
        Ok(WaitStatus::Exited(pid, code)) => {
            let notif = ChildReapedNotification {
                runner_id: runner_id.to_owned(),
                pid: pid.as_raw(),
                exit_status: ChildExitStatus {
                    kind: ChildExitKind::Exited,
                    code: Some(code),
                    signal: None,
                },
                reaped_at_ms: reaped_at_ms_now(),
            };
            if deliver_targeted_reap(runner_id, notif) {
                TargetedReapOutcome::Reaped
            } else {
                TargetedReapOutcome::Failed
            }
        }
        Ok(WaitStatus::Signaled(pid, sig, _)) => {
            let sig_num = sig as libc::c_int;
            let notif = ChildReapedNotification {
                runner_id: runner_id.to_owned(),
                pid: pid.as_raw(),
                exit_status: ChildExitStatus {
                    kind: if sig_num == libc::SIGKILL {
                        ChildExitKind::Killed
                    } else {
                        ChildExitKind::Signaled
                    },
                    code: None,
                    signal: Some(sig_num),
                },
                reaped_at_ms: reaped_at_ms_now(),
            };
            if deliver_targeted_reap(runner_id, notif) {
                TargetedReapOutcome::Reaped
            } else {
                TargetedReapOutcome::Failed
            }
        }
        Ok(WaitStatus::StillAlive) | Ok(_) => {
            // Still running: the SIGCHLD loop will reap it on exit.
            TargetedReapOutcome::StillAlive
        }
        Err(Errno::ECHILD) => {
            // Already reaped by the SIGCHLD loop; drop any stale entry.
            if remove_runner_registries(runner_id) {
                TargetedReapOutcome::AlreadyReaped
            } else {
                TargetedReapOutcome::Failed
            }
        }
        Err(err) => {
            tracing::warn!(runner_id = %runner_id, error = %err, "targeted_reap_runner: waitid failed");
            TargetedReapOutcome::Failed
        }
    }
}

/// Remove the registry entry, push the `ChildReaped` notification, and
/// write the forensic audit record when the process-global audit
/// handle is available. Mirrors [`remove_and_notify`] but resolves the
/// audit log from the global handle instead of a passed reference.
#[cfg(not(feature = "layer1-bootstrap"))]
fn deliver_targeted_reap(
    runner_id: &str,
    notif: d2b_contracts_broker::broker_wire::ChildReapedNotification,
) -> bool {
    match broker_audit_log_handle().get() {
        Some(audit_log) => remove_and_notify(runner_id, notif, audit_log.as_ref()),
        None => {
            // No audit handle (e.g. a unit test that didn't start the
            // reaper): still reap + notify so the child can't zombie.
            let removed = remove_runner_registries(runner_id);
            push_child_reap_notification(notif);
            tracing::info!(
                runner_id = %runner_id,
                "broker: child reaped via targeted post-spawn reap (no audit handle)"
            );
            removed
        }
    }
}

/// The registry-removal + audit half shared by the sync and async reap
/// notification paths; the notification push differs only in how it takes
/// the buffer lock (plan U8).
#[cfg(not(feature = "layer1-bootstrap"))]
fn remove_and_notify_common(
    runner_id: &str,
    notif: &d2b_contracts_broker::broker_wire::ChildReapedNotification,
    audit_log: &AuditLog,
) -> bool {
    let removed = remove_runner_registries(runner_id);
    if !removed {
        tracing::warn!(
            runner_id = %runner_id,
            "reap: failed to clear runner registration"
        );
    }
    if let Err(err) = audit_log.write_child_reaped(notif) {
        tracing::warn!(runner_id = %runner_id, error = %err, "reap: audit write_child_reaped failed");
    }
    tracing::info!(
        runner_id = %runner_id,
        pid = notif.pid,
        exit_status = ?notif.exit_status,
        "broker: child reaped via SIGCHLD handler",
    );
    removed
}

/// Remove the registry entry, push the `ChildReaped` notification, and
/// write the forensic audit record, from a synchronous context (the
/// targeted post-spawn reap). The push takes the non-blocking `try_lock`.
#[cfg(not(feature = "layer1-bootstrap"))]
fn remove_and_notify(
    runner_id: &str,
    notif: d2b_contracts_broker::broker_wire::ChildReapedNotification,
    audit_log: &AuditLog,
) -> bool {
    let removed = remove_and_notify_common(runner_id, &notif, audit_log);
    push_child_reap_notification(notif);
    removed
}

/// The async variant of [`remove_and_notify`]: the push waits on the
/// buffer lock so the SIGCHLD reaper's notification is never dropped.
#[cfg(not(feature = "layer1-bootstrap"))]
async fn remove_and_notify_async(
    runner_id: &str,
    notif: d2b_contracts_broker::broker_wire::ChildReapedNotification,
    audit_log: &AuditLog,
) -> bool {
    let removed = remove_and_notify_common(runner_id, &notif, audit_log);
    push_child_reap_notification_async(notif).await;
    removed
}

/// Bound on the spawn-rollback reap: `SIGKILL` is asynchronous (a child can
/// sit in uninterruptible sleep), so [`cleanup_spawned_runner_after_failure`]
/// polls `waitid` with `WNOHANG` under this deadline instead of parking the
/// executor worker on a blocking wait for as long as the child takes to die.
#[cfg(not(feature = "layer1-bootstrap"))]
const SPAWN_ROLLBACK_REAP_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);
/// Poll interval for the spawn-rollback reap.
#[cfg(not(feature = "layer1-bootstrap"))]
const SPAWN_ROLLBACK_REAP_POLL: std::time::Duration = std::time::Duration::from_millis(10);

/// Kill and asynchronously reap a child when a post-spawn commit step fails.
/// The broker must not return an error while leaving a live process or stale
/// runner identity behind: the caller will retry the lifecycle operation and
/// the next attempt must be able to reserve the same runner id.
///
/// The reap is a bounded `WNOHANG` poll (the same non-blocking probe every
/// sibling reap path in this file uses) rather than a blocking `waitid`. On
/// deadline exhaustion the pidfd entry is left in place: the SIGCHLD reaper
/// owns the zombie (it is signal-driven and reaps on death), the next spawn
/// reservation evicts the stale entry once the process is gone, and while
/// the child is still alive the entry keeps refusing a duplicate spawn.
#[cfg(not(feature = "layer1-bootstrap"))]
pub(crate) async fn cleanup_spawned_runner_after_failure(
    runner_id: &str,
    pidfd: std::os::fd::BorrowedFd<'_>,
) {
    remove_runner_metadata(runner_id);
    if let Err(err) = crate::sys::pidfd_sys::pidfd_send_signal(pidfd, libc::SIGKILL) {
        tracing::debug!(
            runner_id = %runner_id,
            error = %err,
            "spawn rollback: pidfd SIGKILL failed; child may already have exited"
        );
    }

    use nix::errno::Errno;
    use nix::sys::wait::{Id, WaitPidFlag, WaitStatus, waitid};
    let deadline = tokio::time::Instant::now() + SPAWN_ROLLBACK_REAP_DEADLINE;
    loop {
        match waitid(Id::PIDFd(pidfd), WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG) {
            Ok(WaitStatus::Exited(..)) | Ok(WaitStatus::Signaled(..)) | Err(Errno::ECHILD) => {
                runner_pidfds().remove(runner_id);
                return;
            }
            Ok(WaitStatus::StillAlive) | Ok(_) => {
                if tokio::time::Instant::now() >= deadline {
                    tracing::warn!(
                        runner_id = %runner_id,
                        "spawn rollback: reap deadline exceeded; the SIGCHLD reaper owns the child"
                    );
                    return;
                }
                tokio::time::sleep(SPAWN_ROLLBACK_REAP_POLL).await;
            }
            Err(err) => {
                tracing::warn!(
                    runner_id = %runner_id,
                    error = %err,
                    "spawn rollback: pidfd reap failed"
                );
                runner_pidfds().remove(runner_id);
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(feature = "layer1-bootstrap"))]
    use d2b_contracts::types::BundleOpId;
    #[cfg(not(feature = "layer1-bootstrap"))]
    use d2b_contracts_broker::broker_wire::GuestExecutionBinding;
    use nix::unistd::Gid;
    #[cfg(not(feature = "layer1-bootstrap"))]
    use serde::Serialize;
    use serde_json::Value;
    #[cfg(not(feature = "layer1-bootstrap"))]
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    #[cfg(not(feature = "layer1-bootstrap"))]
    use std::os::fd::OwnedFd;
    #[cfg(not(feature = "layer1-bootstrap"))]
    use std::path::Path;
    use std::path::PathBuf;
    #[cfg(not(feature = "layer1-bootstrap"))]
    use std::sync::Arc;
    #[cfg(not(feature = "layer1-bootstrap"))]
    use std::sync::MutexGuard;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(not(feature = "layer1-bootstrap"))]
    static TEST_USB_SYSFS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    #[test]
    fn public_error_details_fail_closed_for_paths_and_attacker_text() {
        assert_eq!(
            redact_public_detail("failed at /private/host/path"),
            d2b_contracts_resource::v3::canonical_digest(
                "d2b:broker-public-error:v1",
                b"failed at /private/host/path"
            )
        );
        assert_eq!(
            redact_public_detail("attacker error text"),
            d2b_contracts_resource::v3::canonical_digest(
                "d2b:broker-public-error:v1",
                b"attacker error text"
            )
        );
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn network_request_validation_rejects_legacy_and_mixed_scope_refs() {
        let zone_uid =
            d2b_contracts_resource::v3::ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000")
                .unwrap();
        let network_uid =
            d2b_contracts_resource::v3::ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001")
                .unwrap();
        let network_generation = d2b_contracts_resource::v3::ResourceGeneration::new(4).unwrap();
        let attachment_generation = d2b_contracts_resource::v3::ResourceGeneration::new(7).unwrap();
        let bundle_generation = d2b_contracts_resource::v3::ResourceBundleGenerationId::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        assert_eq!(
            validate_uid_network_authority(
                "network:123e4567-e89b-42d3-a456-426614174000:223e4567-e89b-42d3-a456-426614174001",
                &format!(
                    "network-route:123e4567-e89b-42d3-a456-426614174000:223e4567-e89b-42d3-a456-426614174001:{}:0",
                    d2b_core::bundle_resolver::network_name_token("work")
                ),
                &zone_uid,
                &network_uid,
                network_generation,
                attachment_generation,
                &bundle_generation,
            ),
            Ok(())
        );
        assert_eq!(
            validate_uid_network_authority(
                "network:123e4567-e89b-42d3-a456-426614174000:223e4567-e89b-42d3-a456-426614174001",
                "route:zone:work:network:net:0",
                &zone_uid,
                &network_uid,
                network_generation,
                attachment_generation,
                &bundle_generation,
            ),
            Err("network-admission-mismatch")
        );
        assert_eq!(
            validate_uid_network_authority(
                "network:123e4567-e89b-42d3-a456-426614174000:223e4567-e89b-42d3-a456-426614174001",
                "network-route:323e4567-e89b-42d3-a456-426614174002:423e4567-e89b-42d3-a456-426614174003:work:0",
                &zone_uid,
                &network_uid,
                network_generation,
                attachment_generation,
                &bundle_generation,
            ),
            Err("network-admission-mismatch")
        );
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn network_request_validation_rejects_swapped_projection_sysctl_and_hosts_refs() {
        // U12 retired the typed network-family frames: the shape
        // validators the retired arms ran (`validate_uid_network_authority`
        // and friends) are exercised directly here, exactly as the family
        // handler's payload validation runs them daemon-side after the cut.
        use d2b_contracts_resource::v3::{
            ResourceBundleGenerationId, ResourceGeneration, ResourceUid,
        };

        let zone_uid = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
        let network_uid = ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap();
        let other_network_uid = ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap();
        let network_generation = ResourceGeneration::new(4).unwrap();
        let attachment_generation = ResourceGeneration::new(7).unwrap();
        let bundle_generation = ResourceBundleGenerationId::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        let scope_id = format!("network:{}:{}", zone_uid.as_str(), network_uid.as_str());
        let other_token = d2b_core::bundle_resolver::network_name_token("other");

        // A projection intent naming a DIFFERENT network than the scope's
        // network is refused (the swapped-ref shape the retired arm's
        // validation caught).
        assert_eq!(
            validate_uid_network_authority(
                &scope_id,
                &format!(
                    "network-firewall:{}:{}:{}",
                    zone_uid.as_str(),
                    other_network_uid.as_str(),
                    other_token
                ),
                &zone_uid,
                &network_uid,
                network_generation,
                attachment_generation,
                &bundle_generation,
            ),
            Err("network-admission-mismatch")
        );

        // The same swapped-ref shape for the sysctl intent.
        assert_eq!(
            validate_uid_network_authority(
                &scope_id,
                &format!(
                    "network-sysctl:{}:{}:{}:lan:disable-ipv6",
                    zone_uid.as_str(),
                    other_network_uid.as_str(),
                    other_token
                ),
                &zone_uid,
                &network_uid,
                network_generation,
                attachment_generation,
                &bundle_generation,
            ),
            Err("network-admission-mismatch")
        );

        // The same swapped-ref shape for the hosts intent.
        assert_eq!(
            validate_uid_network_authority(
                &scope_id,
                &format!(
                    "network-hosts:{}:{}:{}",
                    zone_uid.as_str(),
                    other_network_uid.as_str(),
                    other_token
                ),
                &zone_uid,
                &network_uid,
                network_generation,
                attachment_generation,
                &bundle_generation,
            ),
            Err("network-admission-mismatch")
        );

        // A matching intent passes the shape gate (the family handler's
        // resolver still fences the installed generation).
        assert_eq!(
            validate_uid_network_authority(
                &scope_id,
                &format!(
                    "network-firewall:{}:{}:{}",
                    zone_uid.as_str(),
                    network_uid.as_str(),
                    other_token
                ),
                &zone_uid,
                &network_uid,
                network_generation,
                attachment_generation,
                &bundle_generation,
            ),
            Ok(())
        );
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn tap_create_rejects_the_all_none_legacy_shape() {
        // U12 retired the typed CreatePersistentTap frame: the envelope
        // carrier admits the generic frame and the typed payload parse is
        // the fail-closed gate for a legacy all-none shape (no provenance
        // tuple, no admitted interface set).
        let request = serde_json::from_value::<BrokerRequest>(serde_json::json!({
            "kind": "EnvelopeInvoke",
            "payload": {
                "operation": "CreatePersistentTap",
                "zone": "guest",
                "payload": {
                    "roleId": "network-attachment",
                    "vmId": "guest"
                },
                "chainRootInvocationId": null,
                "chainIdentities": null,
                "fdIndexes": [],
                "fdKinds": []
            }
        }))
        .expect("the envelope carrier decodes");
        let BrokerRequest::EnvelopeInvoke(invoke) = request else {
            panic!("expected EnvelopeInvoke");
        };
        assert!(
            serde_json::from_value::<d2b_contracts_broker::broker_wire::CreatePersistentTapRequest>(
                invoke.payload
            )
            .is_err(),
            "the legacy all-none tap shape must fail the typed payload parse"
        );
    }

    #[test]
    fn swtpm_hardening_failure_uses_the_typed_path_free_operation() {
        let error = BrokerError::SwtpmDirHardening {
            audit: crate::ops::audit_op::SwtpmDirAudit {
                vm_id: "work-vm".to_owned(),
                base_dir_hash: "fnv1a64:0000000000000000".to_owned(),
                result: crate::ops::audit_op::SwtpmDirResult::FailedClosed,
                mode: 0o700,
                owner_uid: 1000,
                owner_gid: 1000,
                marker_result: crate::ops::audit_op::SwtpmMarkerResult::FailedClosed,
                fail_reason: Some("swtpm-dir-marker-mismatch".to_owned()),
            },
            reason: "swtpm-dir-marker-mismatch",
        };

        #[cfg(feature = "layer1-bootstrap")]
        assert!(matches!(
            error.into_response(),
            BrokerResponse::Error {
                kind,
                operation,
                message,
                ..
            } if kind == "Broker.SwtpmDirHardening"
                && operation == "PrepareSwtpmDir"
                && !message.contains('/')
        ));
        #[cfg(not(feature = "layer1-bootstrap"))]
        assert!(matches!(
            error.into_response(),
            BrokerResponse::Error(response)
                if response.kind == "Broker.SwtpmDirHardening"
                    && response.operation == "PrepareSwtpmDir"
                    && !response.message.contains('/')
        ));
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn usb_sysfs_test_lock() -> MutexGuard<'static, ()> {
        TEST_USB_SYSFS_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn runner_role_mapping_covers_video_and_spawnable_roles() {
        use d2b_contracts_broker::broker_wire::RunnerRole;
        use d2b_core::processes::ProcessRole;

        let cases = [
            (
                ProcessRole::SwtpmPreStartFlush,
                Some(RunnerRole::SwtpmFlush),
            ),
            (ProcessRole::Swtpm, Some(RunnerRole::Swtpm)),
            (ProcessRole::Virtiofsd, Some(RunnerRole::Virtiofsd)),
            (ProcessRole::Video, Some(RunnerRole::Video)),
            (ProcessRole::Gpu, Some(RunnerRole::Gpu)),
            (ProcessRole::GpuRenderNode, Some(RunnerRole::Gpu)),
            (ProcessRole::Audio, Some(RunnerRole::Audio)),
            (
                ProcessRole::CloudHypervisorRunner,
                Some(RunnerRole::CloudHypervisor),
            ),
            (ProcessRole::QemuMediaRunner, Some(RunnerRole::QemuMedia)),
            (ProcessRole::VsockRelay, Some(RunnerRole::VsockRelay)),
            (ProcessRole::Usbip, Some(RunnerRole::Usbip)),
            (ProcessRole::HostReconcile, None),
            (ProcessRole::StoreVirtiofsPreflight, None),
            (ProcessRole::ComponentSessionHealth, None),
        ];

        for (role, expected) in cases {
            assert_eq!(runner_role_for_process_role(&role), expected);
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    fn guest_execution_binding(session_generation: u64) -> GuestExecutionBinding {
        GuestExecutionBinding {
            target_uid: d2b_contracts_resource::v3::ResourceUid::parse(
                "123e4567-e89b-42d3-a456-426614174000",
            )
            .expect("Guest UID"),
            boot_identity_digest: [7; 32],
            session_generation,
            assignment_epoch: 3,
            provider_generation: 4,
            controller_generation: 5,
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    fn guest_runner_registration(binding: GuestExecutionBinding) -> RunnerRegistration {
        RunnerRegistration {
            vm_id: "guest-vm".to_owned(),
            role_id: "guest-process".to_owned(),
            resource_ref: None,
            resource_uid: None,
            zone_uid: None,
            owner_ref: None,
            provider_ref: None,
            provider_identity: None,
            template_identity: None,
            generation: None,
            runtime_scope: None,
            role: d2b_contracts_broker::broker_wire::RunnerRole::CloudHypervisor,
            bundle_runner_intent_ref: "runner:guest-vm:guest-process".to_owned(),
            pid: 1,
            start_time_ticks: 1,
            binary_path: PathBuf::from("/bin/true"),
            cgroup_subtree: "d2b.slice/guest-vm/guest-process".to_owned(),
            guest_execution: Some(binding),
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn guest_runner_registration_rebinds_to_a_newer_session() {
        let mut registration = guest_runner_registration(guest_execution_binding(1));
        let next = guest_execution_binding(2);

        assert!(
            rebind_guest_execution_registration(&mut registration, Some(&next))
                .expect("newer session generation rebinds")
        );
        assert_eq!(registration.guest_execution, Some(next.clone()));
        assert!(
            !rebind_guest_execution_registration(&mut registration, Some(&next))
                .expect("same binding remains an exact match")
        );
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn guest_runner_registration_rejects_stale_or_changed_bindings() {
        for generation in [1, 0] {
            let mut registration = guest_runner_registration(guest_execution_binding(2));
            let stale = guest_execution_binding(generation);
            assert!(
                rebind_guest_execution_registration(&mut registration, Some(&stale)).is_err(),
                "session generation {generation} must not replace generation 2"
            );
        }

        let mut registration = guest_runner_registration(guest_execution_binding(2));
        for changed in [
            {
                let mut binding = guest_execution_binding(3);
                binding.target_uid = d2b_contracts_resource::v3::ResourceUid::parse(
                    "223e4567-e89b-42d3-a456-426614174000",
                )
                .expect("Guest UID");
                binding
            },
            {
                let mut binding = guest_execution_binding(3);
                binding.boot_identity_digest = [8; 32];
                binding
            },
            {
                let mut binding = guest_execution_binding(3);
                binding.assignment_epoch = 4;
                binding
            },
            {
                let mut binding = guest_execution_binding(3);
                binding.provider_generation = 5;
                binding
            },
            {
                let mut binding = guest_execution_binding(3);
                binding.controller_generation = 6;
                binding
            },
        ] {
            assert!(
                rebind_guest_execution_registration(&mut registration, Some(&changed)).is_err(),
                "non-session binding changes must not be accepted as a reconnect"
            );
            assert_eq!(
                registration.guest_execution,
                Some(guest_execution_binding(2))
            );
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn qemu_media_enroll_request_fields_redact_raw_busid() {
        let request = BrokerRequest::QemuMediaEnroll(
            d2b_contracts_broker::broker_wire::QemuMediaEnrollRequest {
                vm_id: d2b_contracts::types::VmId::new("media"),
                media_ref: d2b_contracts::types::MediaRef::new("installer-usb"),
                bus_id: "1-2.3".to_owned(),
                tracing_span_id: Some(d2b_contracts::types::TracingSpanId::new(
                    "usb-start-0000000000000001",
                )),
            },
        );

        let fields = request_fields_value(&request).expect("redacted fields");
        assert_eq!(fields["vmId"], "media");
        assert_eq!(fields["mediaRef"], "installer-usb");
        assert_eq!(fields["busIdProvided"], true);
        let rendered = fields.to_string();
        assert!(!rendered.contains("1-2.3"));
        assert!(!rendered.contains("/dev/"));
        assert!(!rendered.contains("usb-Vendor_SecretSerial"));
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn qemu_media_hotplug_request_fields_redact_runtime_busid() {
        let request = BrokerRequest::QemuMediaAttach(
            d2b_contracts_broker::broker_wire::QemuMediaHotplugRequest {
                vm_id: d2b_contracts::types::VmId::new("media"),
                bus_id: "1-2.3".to_owned(),
                tracing_span_id: None,
            },
        );

        let fields = request_fields_value(&request).expect("redacted fields");
        assert_eq!(fields["vmId"], "media");
        assert_eq!(fields["busIdProvided"], true);
        let rendered = fields.to_string();
        assert!(!rendered.contains("1-2.3"));
        assert!(!rendered.contains("/dev/"));
        assert!(!rendered.contains("usb-Vendor_SecretSerial"));
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn usbip_request_fields_project_trace_presence_without_trace_value() {
        let trace = d2b_contracts::types::TracingSpanId::new("usb-start-0000000000000001");
        let requests = [
            BrokerRequest::UsbipBind(d2b_contracts_broker::broker_wire::UsbipBindRequest {
                bundle_usbip_bind_intent_ref: d2b_contracts::types::BundleOpId::new(
                    "usbip-bind:env:work:vm:corp-vm:bus:1-2.3",
                ),
                tracing_span_id: Some(trace.clone()),
            }),
            BrokerRequest::UsbipUnbind(d2b_contracts_broker::broker_wire::UsbipUnbindRequest {
                bundle_usbip_bind_intent_ref: d2b_contracts::types::BundleOpId::new(
                    "usbip-bind:env:work:vm:corp-vm:bus:1-2.3",
                ),
                preserve_durable_claim: true,
                tracing_span_id: Some(trace.clone()),
            }),
            BrokerRequest::UsbipBindFirewallRule(
                d2b_contracts_broker::broker_wire::UsbipBindFirewallRuleRequest {
                    bundle_usbip_firewall_intent_ref: d2b_contracts::types::BundleOpId::new(
                        "usbip-fw:env:work:bus:1-2.3",
                    ),
                    tracing_span_id: Some(trace.clone()),
                },
            ),
            BrokerRequest::UsbipProxyReconcile(
                d2b_contracts_broker::broker_wire::UsbipProxyReconcileRequest {
                    scope_id: d2b_contracts::types::ScopeId::new("vm:corp-vm"),
                    tracing_span_id: Some(trace.clone()),
                },
            ),
        ];

        for request in requests {
            let fields = request_fields_value(&request).expect("bounded USBIP fields");
            assert_eq!(fields["tracingSpanIdPresent"], true);
            assert!(
                !fields.to_string().contains(trace.as_str()),
                "request_fields must carry only trace presence"
            );
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn qemu_media_boot_request_fields_are_vm_only() {
        let request =
            BrokerRequest::QemuMediaBoot(d2b_contracts_broker::broker_wire::QemuMediaBootRequest {
                vm_id: d2b_contracts::types::VmId::new("media"),
                tracing_span_id: None,
            });

        let fields = request_fields_value(&request).expect("redacted fields");
        assert_eq!(fields["vmId"], "media");
        let rendered = fields.to_string();
        assert!(!rendered.contains("bus"));
        assert!(!rendered.contains("/dev/"));
        assert!(!rendered.contains("usb-Vendor_SecretSerial"));
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn qemu_media_lifecycle_request_fields_are_bounded() {
        let request = BrokerRequest::QemuMediaQueryStatus(
            d2b_contracts_broker::broker_wire::QemuMediaQueryStatusRequest {
                vm_id: d2b_contracts::types::VmId::new("media"),
                shutdown_context: true,
                tracing_span_id: None,
            },
        );

        let fields = request_fields_value(&request).expect("bounded fields");
        assert_eq!(fields["vmId"], "media");
        assert_eq!(fields["shutdownContext"], true);
        let rendered = fields.to_string();
        assert!(!rendered.contains("return"));
        assert!(!rendered.contains("status\":\""));
        assert!(!rendered.contains("/dev/"));
    }

    struct AuditCase {
        error: BrokerError,
        operation: &'static str,
        target_id: String,
        decision: &'static str,
        error_kind: &'static str,
        error_message: String,
    }

    fn test_audit_dir(test_name: &str) -> PathBuf {
        let root = crate::test_scratch_root().join("runtime-audit-tests");
        crate::sys::path_safe::ensure_dir(&root, 0o750, None, None)
            .expect("create audit test root");
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();

        root.join(format!("{test_name}-{}-{unique}", std::process::id()))
    }

    #[test]
    fn every_wire_variant_has_a_dispatch_arm_or_a_recorded_deferral() {
        // The real-wire dispatch match serves one explicit arm per operation it
        // implements and defers the committed reserved stubs to the fallback
        // arm. Retiring a family arm is the point of the per-variant shrink, so
        // an arm that disappears must show up as a deliberate state rather
        // than as a variant that silently falls through to a refusal: a
        // committed deferral marker.
        //
        // The sets are pinned because the gate has to fail on a *new* state, and
        // a source-scanning test would fail on a comment. An arm moved out of the
        // match therefore has to move one of these names with it - which is
        // exactly the moment the retirement has to be decided.
        #[cfg(not(feature = "layer1-bootstrap"))]
        const DISPATCHED: &[&str] = &[
            "ApplyHostGenerationHandoff",
            "DelegateCgroupV2",
            "DiskInit",
            "EnvelopeInvoke",
            "ExportBrokerAudit",
            "Hello",
            "ModprobeIfAllowed",
            "OpenCgroupDir",
            "OpenDevice",
            "OpenFuse",
            "OpenHidrawSecurityKey",
            "OpenKvm",
            "OpenVhostNet",
            "OwnershipMatrixCheck",
            "PipeWireAudio",
            "PublishTrustedContext",
            "QemuMediaAttach",
            "QemuMediaBoot",
            "QemuMediaDetach",
            "QemuMediaEnroll",
            "QemuMediaQueryStatus",
            "QemuMediaQuit",
            "QemuMediaRefreshRegistry",
            "QemuMediaSystemPowerdown",
            "ReconcileStorageScope",
            // U10 retired the typed process-family arms (SignalRunner,
            // SpawnRunner among them) at wire v6: their privileged cores are
            // the broker-generic kernels served through the EnvelopeInvoke
            // arm, and a straggler wire frame is refused by the wire gate.
            // U12 retired the network-fds family arms (ApplyNftables,
            // CreateTapFd, SeedDnsmasqLease among them) the same way: the
            // thirteen network kernels serve through the EnvelopeInvoke arm.
            "StoreSync",
            "UsbipBind",
            "UsbipBindFirewallRule",
            "UsbipExplicitBind",
            "UsbipExplicitFirewallRule",
            "UsbipProxyReconcile",
            "UsbipUnbind",
            "ValidateLockSpec",
        ];
        #[cfg(not(feature = "layer1-bootstrap"))]
        {
            let dispatched: BTreeSet<&str> = DISPATCHED.iter().copied().collect();
            assert_eq!(
                dispatched.len(),
                DISPATCHED.len(),
                "the arm set names one operation once"
            );
            let wire: BTreeSet<&str> = crate::catalog::WIRE_VARIANTS.iter().copied().collect();
            for name in &dispatched {
                assert!(
                    wire.contains(name),
                    "{name}: an arm serves a variant the committed wire enum does not declare"
                );
            }
            let mut undecided: Vec<&str> = Vec::new();
            for name in crate::catalog::WIRE_VARIANTS {
                if dispatched.contains(name) {
                    assert_eq!(
                        crate::catalog::stub_target(name),
                        None,
                        "{name}: an arm serves it, so it is not a reserved stub"
                    );
                    continue;
                }
                if crate::catalog::stub_target(name).is_some() {
                    continue;
                }
                undecided.push(name);
            }
            assert!(
                undecided.is_empty(),
                "variants with no dispatch arm and no committed deferral: {undecided:?}"
            );
        }
    }

    #[test]
    fn a_served_operation_names_the_audit_shape_its_records_use() {
        // A variant dispatched by hand carries its own typed audit fields, and
        // the fallback arm refuses the reserved stubs. A served operation whose
        // row declares no audit shape therefore has no record shape on either
        // path - a row that can be dispatched but cannot be audited. The
        // reserved stubs never serve a request, so no record shape is expected
        // of them.
        #[cfg(not(feature = "layer1-bootstrap"))]
        {
            // Served, but their records carry no typed field shape yet. Pinned
            // so the gap cannot grow while the audit shapes are brought up to
            // the served set. `OwnershipMatrixCheck` is the first envelope
            // caller (U5): its wire arm records an allowed entry and the
            // envelope's own invocation is audited by the declaring peer;
            // its row therefore declares no typed audit shape yet.
            const UNAUDITED: &[&str] = &[
                "QemuMediaQueryStatus",
                "OwnershipMatrixCheck",
                // The generic invocation surface (U10): its wire arm runs the
                // committed operation's envelope, whose per-invocation audit
                // record is the named operation's record - the transport
                // variant itself carries no typed audit shape of its own.
                "EnvelopeInvoke",
            ];
            for name in crate::catalog::WIRE_VARIANTS {
                if crate::catalog::stub_target(name).is_some() || UNAUDITED.contains(name) {
                    continue;
                }
                let row = crate::catalog::BrokerOperationRow::find(name)
                    .unwrap_or_else(|| panic!("{name}: a wire variant with no committed row"));
                assert!(
                    !row.audit_fields.is_empty(),
                    "{name}: dispatched without an audit shape"
                );
            }
        }
    }

    #[test]
    fn a_committed_stub_carries_its_deferral_marker_rather_than_a_dispatch() {
        // A row the committed catalog still marks as a reserved stub is refused
        // by name through the fallback arm; the mirror image is a row that is
        // neither stubbed nor dispatchable, which would be a row the broker
        // commits and then cannot reach at all.
        #[cfg(not(feature = "layer1-bootstrap"))]
        {
            let mut both_stubbed_and_dispatchable: Vec<&str> = Vec::new();
            for name in crate::catalog::WIRE_VARIANTS {
                let Some(_marker) = crate::catalog::stub_target(name) else {
                    continue;
                };
                if crate::catalog::BrokerOperationRow::find(name).is_some_and(|row| {
                    row.disposition == crate::catalog::Disposition::PromotedLive
                })
                {
                    both_stubbed_and_dispatchable.push(name);
                }
            }
            assert!(
                both_stubbed_and_dispatchable.is_empty(),
                "stubbed rows the catalog also promotes: {both_stubbed_and_dispatchable:?}"
            );
        }
    }

    #[test]
    fn parse_command_rejects_removed_realm_metadata_flags() {
        for flag in ["--realm-controllers-path", "--realm-identity-path"] {
            let error = parse_command([
                "host".to_owned(),
                flag.to_owned(),
                "/etc/d2b/retired-realm-metadata.json".to_owned(),
                "--test-mode".to_owned(),
            ])
            .expect_err("retired realm metadata flags must be rejected");

            assert!(
                matches!(
                    error,
                    RunError::Usage(ref message) if message == &format!("unknown host flag: {flag}")
                ),
                "unexpected parser error for {flag}: {error:?}"
            );
        }
    }

    #[test]
    fn parse_command_binds_the_forward_socket_flag() {
        // The flag is the one deterministic input: the environment is only a
        // fallback, and a broker that names no peer must stay fail-closed
        // rather than picking a path of its own.
        let mode = parse_command([
            "host".to_owned(),
            "--test-mode".to_owned(),
            "--forward-socket".to_owned(),
            "/run/d2b/d2bd-forward.sock".to_owned(),
        ])
        .expect("a host command with a forward socket");
        let BrokerMode::Host(config) = mode else {
            panic!("host subcommand must build a host config");
        };
        assert_eq!(
            config.forward_socket_path.as_deref(),
            Some(Path::new("/run/d2b/d2bd-forward.sock"))
        );
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn write_json_file<T: Serialize>(path: &Path, value: &T) {
        use std::os::unix::fs::PermissionsExt;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directories for test json");
        }
        let mut body = serde_json::to_vec_pretty(value).expect("serialize test json");
        body.push(b'\n');
        fs::write(path, body).expect("write test json");
        // Bundle artifacts must be mode 0640 per BundleVerifyPolicy.
        let mut perms = fs::metadata(path).expect("stat test json").permissions();
        perms.set_mode(0o640);
        fs::set_permissions(path, perms).expect("chmod test json to 0640");
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    struct TestBundle {
        bundle_path: PathBuf,
        manifest_path: PathBuf,
        host_path: PathBuf,
        processes_path: PathBuf,
        resolver: Arc<BundleResolver>,
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    fn build_test_bundle(root: &Path) -> TestBundle {
        use d2b_contracts_resource::v3::IfName;
        use d2b_core::bundle::{Bundle, BundleGeneration};
        use d2b_core::host::{
            BridgePortFlags, ChNetHandoffMode, CloudHypervisorCapability, FdOwnershipEntry,
            HostChConfig, HostJson, HostsFileOwnership, IfNameMapping, Ipv6SysctlEntry,
            KernelModulesEntry, LanPolicy, NetEnv, NetworkManagerUnmanaged, NftChain,
            NftablesModel, OwnershipRule, SitePolicy, TapRole, UsbipBusidLock, UsbipLockOwner,
            UsbipLockScope, VendorProductPair,
        };
        use d2b_core::manifest_v04::{
            ManifestMeta, ManifestV04, ObservabilityMeta, VmEntry, VmLanPolicy, VmObservability,
        };
        use d2b_core::sandbox_profile::CgroupPlacement;
        use d2b_core::processes::{
            NodeId, ProcessNode, ProcessRole, ProcessesJson, VmProcessDag, VmProcessInvariants,
        };
        use d2b_core::runtime::RuntimeMetadata;
        use d2b_core::test_support::RoleProfileBuilder;

        let bundle_dir = root.join("bundle");
        let bundle_path = bundle_dir.join("bundle.json");
        let manifest_path = bundle_dir.join("vms.json");
        let host_path = bundle_dir.join("host.json");
        let processes_path = bundle_dir.join("processes.json");

        let host = HostJson {
            schema_version: "v2".to_owned(),
            security_key_selectors: Vec::new(),
            site: SitePolicy {
                allow_unsafe_east_west: false,
            },
            environments: vec![NetEnv {
                env: "work".to_owned(),
                bridge: IfName::new("nlworkbr0").expect("bridge ifname"),
                host_uplink_ip: Some("192.0.2.1".to_owned()),
                net_uplink_ip: Some("192.0.2.2".to_owned()),
                mtu: 1500,
                mss_clamp: Some(1460),
                lan: LanPolicy {
                    allow_east_west: false,
                    effective_east_west: false,
                },
                net_vm_forward_blocklist: vec!["0.0.0.0/0".to_owned()],
                external_network: None,
                bridge_port_flags: vec![
                    BridgePortFlags {
                        role: TapRole::WorkloadLan,
                        isolated: true,
                        neigh_suppress: true,
                        learning: None,
                        unicast_flood: None,
                        rule: "isolated workload bridge port".to_owned(),
                    },
                    BridgePortFlags {
                        role: TapRole::Uplink,
                        isolated: true,
                        neigh_suppress: true,
                        learning: Some(false),
                        unicast_flood: Some(false),
                        rule: "uplink point-to-point anti-spoofing".to_owned(),
                    },
                ],
                ipv6_sysctls: vec![Ipv6SysctlEntry {
                    if_name: IfName::new("nlworktap0").expect("sysctl ifname"),
                    disable_ipv6: 1,
                    accept_ra: 0,
                    autoconf: 0,
                    addr_gen_mode: 1,
                    arp_ignore: 1,
                }],
                usbip_busid_locks: vec![UsbipBusidLock {
                    vm: "corp-vm".to_owned(),
                    lock_owner: UsbipLockOwner::Daemon,
                    scope: UsbipLockScope::PerBusid,
                    bus_ids: vec!["1-2.3".to_owned()],
                    vendor_product_allowlist: vec![VendorProductPair {
                        vendor: 0x1050,
                        product: 0x0407,
                    }],
                }],
                usbip_backend_port: Some(3241),
            }],
            nftables: NftablesModel {
                family: "inet".to_owned(),
                table: "d2b".to_owned(),
                chains: vec![NftChain {
                    name: "input".to_owned(),
                    hook: Some("input".to_owned()),
                    priority: Some(0),
                    policy: Some("accept".to_owned()),
                    purpose: "test input chain".to_owned(),
                }],
                table_hash_after_apply: Some("fnv1a64:beadbeadbeadbead".to_owned()),
                ownership_id: "ownership-1".to_owned(),
            },
            network_manager: NetworkManagerUnmanaged {
                file_path: "/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf".to_owned(),
                match_criteria: vec!["interface-name:d2b-*".to_owned()],
                reload_behavior: "atomic-reload".to_owned(),
                ownership: OwnershipRule {
                    owner: "root".to_owned(),
                    group: "root".to_owned(),
                    mode: "0644".to_owned(),
                    drift_policy: "replace".to_owned(),
                },
            },
            hosts_file: HostsFileOwnership {
                start_marker: "# d2b-managed begin".to_owned(),
                end_marker: "# d2b-managed end".to_owned(),
                rule: "replace-managed-block".to_owned(),
            },
            kernel_modules: Vec::<KernelModulesEntry>::new(),
            fd_ownership: Vec::<FdOwnershipEntry>::new(),
            cloud_hypervisor_capabilities: Vec::<CloudHypervisorCapability>::new(),
            if_name_mappings: Vec::<IfNameMapping>::new(),
            qemu_media: None,
            ch: Some(HostChConfig {
                net_handoff_mode: ChNetHandoffMode::TapFd,
            }),
            firewall_coexistence_policy: None,
        };

        let processes = ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: vec![
                VmProcessDag {
                    workload_identity: None,
                    vm: "corp-vm".to_owned(),
                    nodes: vec![ProcessNode {
                        execution_ref: None,
                        execution_domain: None,
                        user_ref: None,
                        id: NodeId("ch-runner".to_owned()),
                        role: ProcessRole::CloudHypervisorRunner,
                        unit: Some("d2b@corp-vm.service".to_owned()),
                        binary_path: None,
                        argv: Vec::new(),
                        env: Vec::new(),
                        profile: RoleProfileBuilder::new()
                            .with_profile_id("profile-ch")
                            .with_uid(1001)
                            .with_gid(1001)
                            .with_namespaces(d2b_core::sandbox_profile::NamespaceSet {
                                mount: true,
                                pid: false,
                                net: false,
                                ipc: false,
                                uts: false,
                                user: false,
                            })
                            .with_seccomp_policy_ref(Some("profile-ch.seccomp"))
                            .with_read_only_paths(vec!["/nix/store".to_owned()])
                            .with_cgroup_placement(CgroupPlacement {
                                subtree: "d2b.slice/corp-vm/ch-runner".to_owned(),
                                controllers: vec!["cpu".to_owned(), "memory".to_owned()],
                                delegated: true,
                            })
                            .build(),
                        readiness: Vec::new(),
                        plan_ops: Vec::new(),
                        network_interfaces: Vec::new(),
                    }],
                    edges: Vec::new(),
                    invariants: VmProcessInvariants {
                        swtpm_pre_start_flush: true,
                        per_vm_audit_pipeline: true,
                        usbip_gating: true,
                        tpm_ownership_migration_without_running_vm_mutation: true,
                    },
                },
                VmProcessDag {
                    workload_identity: None,
                    vm: "sys-work-usbipd".to_owned(),
                    nodes: vec![ProcessNode {
                        execution_ref: None,
                        execution_domain: None,
                        user_ref: None,
                        id: NodeId("backend".to_owned()),
                        role: ProcessRole::Usbip,
                        unit: None,
                        binary_path: Some("/run/current-system/sw/bin/usbipd".to_owned()),
                        argv: vec!["usbipd".to_owned(), "-D".to_owned()],
                        env: Vec::new(),
                        profile: RoleProfileBuilder::new()
                            .with_profile_id("profile-usbip")
                            .with_uid(1002)
                            .with_gid(1002)
                            .with_cgroup_placement(CgroupPlacement {
                                subtree: "d2b.slice/sys-work-usbipd/backend".to_owned(),
                                controllers: vec!["cpu".to_owned(), "memory".to_owned()],
                                delegated: false,
                            })
                            .build(),
                        readiness: Vec::new(),
                        plan_ops: Vec::new(),
                        network_interfaces: Vec::new(),
                    }],
                    edges: Vec::new(),
                    invariants: VmProcessInvariants {
                        swtpm_pre_start_flush: true,
                        per_vm_audit_pipeline: true,
                        usbip_gating: true,
                        tpm_ownership_migration_without_running_vm_mutation: true,
                    },
                },
            ],
        };

        let manifest = ManifestV04 {
            manifest: ManifestMeta {
                manifest_version: 6,
            },
            observability: ObservabilityMeta {
                enabled: false,
                obs_vsock_cid: 3,
                obs_vsock_host_socket: "/run/d2b/obs.sock".to_owned(),
                signoz_otlp_grpc_port: 4317,
                signoz_otlp_http_port: 4318,
                signoz_url: "http://127.0.0.1:8080".to_owned(),
                vm_name: "obs".to_owned(),
            },
            vms: BTreeMap::from([(
                "corp-vm".to_owned(),
                VmEntry {
                    api_socket: Some("/run/d2b/vms/corp-vm/api.sock".to_owned()),
                    audio: false,
                    audio_service: Some(String::new()),
                    audio_state_file: Some(String::new()),
                    bridge: Some("br-work".to_owned()),
                    env: Some("work".to_owned()),
                    mtu: Some(1500),
                    mss_clamp: Some(1460),
                    lan: Some(VmLanPolicy {
                        allow_east_west: false,
                        effective_east_west: false,
                    }),
                    gpu_socket: Some(String::new()),
                    graphics: false,
                    is_net_vm: false,
                    name: "corp-vm".to_owned(),
                    net_vm: Some("sys-work-net".to_owned()),
                    observability: VmObservability {
                        agent_socket: Some("/run/d2b/vms/corp-vm/agent.sock".to_owned()),
                        enabled: false,
                        vsock_cid: Some(17),
                        vsock_host_socket: Some("/run/d2b/vms/corp-vm/agent-host.sock".to_owned()),
                    },
                    runtime: RuntimeMetadata::local_nixos(),
                    autostart: true,
                    security_key: false,
                    lifecycle: Default::default(),
                    shell: None,
                    ssh_user: Some("alice".to_owned()),
                    state_dir: "/var/lib/d2b/vms/corp-vm".to_owned(),
                    static_ip: Some("192.0.2.10".to_owned()),
                    tap: "tap-corp-vm".to_owned(),
                    tpm: false,
                    tpm_socket: Some(String::new()),
                    usbip_yubikey: true,
                    usbipd_host_ip: Some("192.0.2.1".to_owned()),
                },
            )]),
        };

        let bundle = Bundle {
            bundle_version: 1,
            schema_version: "v3".to_owned(),
            privileges_path: "privileges.json".to_owned(),
            storage_path: None,
            realm_workloads_launcher_v2_path: None,
            generation: BundleGeneration {
                generator: "unit-test".to_owned(),
                source_revision: Some("deadbeef".to_owned()),
                generated_at: Some("2026-01-01T00:00:00Z".to_owned()),
            },
            bundle_hash: None,
            artifact_hashes: None,
        };

        write_json_file(&manifest_path, &manifest);
        write_json_file(&host_path, &host);
        write_json_file(&processes_path, &processes);

        let resolver = Arc::new(BundleResolver::from_artifacts_with_zone_resource_bundles(
            bundle,
            host,
            processes,
            manifest,
            BTreeMap::new(),
        ));
        TestBundle {
            bundle_path,
            manifest_path,
            host_path,
            processes_path,
            resolver,
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    fn set_usbip_allowlist(
        bundle: &mut TestBundle,
        allowlist: Vec<d2b_core::host::VendorProductPair>,
    ) {
        let resolver = Arc::get_mut(&mut bundle.resolver).expect("resolver uniquely owned");
        resolver.test_set_usbip_allowlist("corp-vm", allowlist);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn prepare_test_usb_sysfs_device(vendor: &str, product: &str, devpath: &str) -> PathBuf {
        let root = crate::test_scratch_root().join("runtime-usb-sysfs-root");
        TEST_USB_SYSFS_ROOT
            .set(root.clone())
            .unwrap_or_else(|_| assert_eq!(TEST_USB_SYSFS_ROOT.get(), Some(&root)));
        let device_dir = root.join("1-2.3");
        fs::create_dir_all(&device_dir).expect("create fake USB sysfs device");
        fs::write(device_dir.join("idVendor"), format!("{vendor}\n")).expect("write vendor");
        fs::write(device_dir.join("idProduct"), format!("{product}\n")).expect("write product");
        fs::write(device_dir.join("busnum"), b"1\n").expect("write busnum");
        fs::write(device_dir.join("devnum"), b"7\n").expect("write devnum");
        fs::write(device_dir.join("devpath"), format!("{devpath}\n")).expect("write devpath");
        let _ = fs::remove_file(device_dir.join("driver"));
        let _ = fs::remove_file(device_dir.join("serial"));
        root
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    fn test_server_config(root: &Path, manifest_path: &Path) -> ServerConfig {
        ServerConfig {
            profile: BrokerProfile::Host,
            authority_id: "test-host".to_owned(),
            socket_path: root.join("broker.sock"),
            audit_dir: root.join("audit"),
            audit_retention_days: 14,
            bundle_path: manifest_path.to_path_buf(),
            state_dir: root.join("state"),
            activation_helper_path: root.join("activation-helper"),
            // The peer-credential gate compares the kernel-reported peer uid
            // against this: a fixture pinned to a literal uid only passes when
            // the test process happens to run as it (a CI runner runs as
            // 1001), so the fixture names the test process's own uid.
            // The peer-credential gate compares the kernel-reported peer uid
            // against this: a fixture pinned to a literal uid only passes when
            // the test process happens to run as it (a CI runner runs as
            // 1001), so the fixture names the test process's own uid.
            d2bd_uid: nix::unistd::Uid::current().as_raw(),
            d2bd_gid: Gid::current().as_raw(),
            store_sync_export_dir: root.join("observability").join("store-sync"),
            // No forwarding peer in a broker unit test: the envelope refuses
            // every forwarded operation, which is the fail-closed default.
            forward_socket_path: None,
            test_mode: true,
            #[cfg(not(feature = "layer1-bootstrap"))]
            retired_wire_variants: RETIRED_WIRE_VARIANTS,
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn test_usbip_intent_with_lock(
        root: &Path,
        bundle: &TestBundle,
    ) -> d2b_core::bundle_resolver::ResolvedUsbipBindIntent {
        let mut intent = find_usbip_bind_intent_for(&bundle.resolver, "corp-vm", "1-2.3")
            .expect("bundle usbip bind intent");
        let lock_dir = root.join("locks");
        fs::create_dir_all(&lock_dir).expect("create USBIP lock dir");
        intent.lock_path = lock_dir.join("1-2.3");
        intent
    }

    /// Read the StoreSync observability export lines the dispatch arm
    /// appended for `config` (today's rotated file). Returns the parsed
    /// records plus the raw JSON objects so tests can assert both the
    /// typed shape and the exact serialized key-set.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn read_store_sync_export(
        config: &ServerConfig,
    ) -> Vec<(
        crate::ops::store_sync_export::StoreSyncObservabilityRecord,
        serde_json::Map<String, serde_json::Value>,
    )> {
        let date = crate::audit::utc_date_string();
        let path = config
            .store_sync_export_dir
            .join(format!("store-sync-{date}.jsonl"));
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
            Err(err) => panic!("read store-sync export {}: {err}", path.display()),
        };
        contents
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let record: crate::ops::store_sync_export::StoreSyncObservabilityRecord =
                    serde_json::from_str(line).expect("parse export record");
                let obj = serde_json::from_str::<serde_json::Value>(line)
                    .expect("parse export json")
                    .as_object()
                    .expect("export record is a json object")
                    .clone();
                (record, obj)
            })
            .collect()
    }

    /// Assert the exported JSON object's key-set equals the signed
    /// allow-list and that no redaction field leaked.
    #[cfg(not(feature = "layer1-bootstrap"))]
    fn assert_export_allow_list(obj: &serde_json::Map<String, serde_json::Value>) {
        use crate::ops::store_sync_export::{EXPORTED_KEYS, REDACTED_KEYS};
        let mut actual: Vec<&str> = obj.keys().map(String::as_str).collect();
        actual.sort_unstable();
        let mut expected: Vec<&str> = EXPORTED_KEYS.to_vec();
        expected.sort_unstable();
        assert_eq!(
            actual, expected,
            "export key-set must equal the signed allow-list"
        );
        for redacted in REDACTED_KEYS {
            assert!(
                !obj.contains_key(*redacted),
                "redacted key {redacted:?} leaked into export surface"
            );
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn executable_identity_accepts_trusted_symlink_paths() {
        let expected = Path::new("/proc/self/exe");
        let actual = tokio::fs::read_link(expected).await.expect("read current executable");
        assert!(executable_paths_match(&actual, expected).await);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn executable_identity_accepts_static_wrapper_exec_target() {
        let root = tempfile::tempdir().expect("tempdir");
        let wrapper = root.path().join("cloud-hypervisor");
        let real = root.path().join(".cloud-hypervisor-real");
        tokio::fs::write(
            &wrapper,
            "#!/bin/sh\nexec \"$here/.cloud-hypervisor-real\" \"$@\"\n",
        )
        .await
        .expect("write wrapper");
        tokio::fs::write(&real, b"trusted executable")
            .await
            .expect("write executable");

        assert!(executable_paths_match(&real, &wrapper).await);
        assert!(!executable_paths_match(
            &root.path().join("other"),
            &wrapper
        )
        .await);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn executable_observation_keeps_permission_denied_distinct_from_a_mismatch() {
        let observed = observe_runner_executable(
            Err(io::Error::from(io::ErrorKind::PermissionDenied)),
            Path::new("/nix/store/provider-controller/bin/controller"),
        )
        .await
        .expect("permission-denied observation");
        assert_eq!(observed, RunnerExecutableObservation::PermissionDenied);
        assert!(!observed.is_verified_for_discovery());
        assert!(observed.is_verified_for_registered(true, true));
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn typed_process_cgroup_scope_is_private_and_collision_safe() {
        let placement = d2b_core::sandbox_profile::CgroupPlacement {
            subtree: "d2b.slice/corp-vm/cloud-hypervisor".to_owned(),
            controllers: vec!["cpu".to_owned()],
            delegated: false,
        };
        let first = private_cgroup_placement(&placement, "corp-vm", Some([1; 32]), true)
            .expect("private cgroup placement");
        let second = private_cgroup_placement(&placement, "corp-vm", Some([2; 32]), true)
            .expect("private cgroup placement");
        assert_ne!(first.subtree, second.subtree);
        assert!(!first.subtree.contains("corp-vm"));
        assert!(first.subtree.starts_with("d2b.slice/process-"));
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn typed_zone_guest_cgroup_scope_is_private_and_collision_safe() {
        let placement = d2b_core::sandbox_profile::CgroupPlacement {
            subtree: "d2b.slice/work/desktop/cloud-hypervisor".to_owned(),
            controllers: vec!["cpu".to_owned()],
            delegated: false,
        };
        let first = private_cgroup_placement(&placement, "desktop", Some([1; 32]), true)
            .expect("private Zone/Guest cgroup placement");
        let second = private_cgroup_placement(&placement, "desktop", Some([2; 32]), true)
            .expect("private Zone/Guest cgroup placement");
        assert_ne!(first.subtree, second.subtree);
        assert!(!first.subtree.contains("work"));
        assert!(!first.subtree.contains("desktop"));
        assert!(first.subtree.ends_with("/cloud-hypervisor"));
        assert!(first.subtree.starts_with("d2b.slice/process-"));
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn typed_runner_registry_keys_use_opaque_runtime_scope() {
        let resource_ref =
            d2b_contracts_resource::v3::ResourceRef::parse("Process/worker").expect("resource ref");
        let uid =
            d2b_contracts_resource::v3::ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000")
                .expect("resource uid");
        let zone_uid =
            d2b_contracts_resource::v3::ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001")
                .expect("zone uid");
        let first = runner_registry_key(
            "corp-vm",
            "worker",
            Some(&resource_ref),
            Some(&uid),
            Some(&zone_uid),
            Some([1; 32]),
        );
        let second = runner_registry_key(
            "corp-vm",
            "worker",
            Some(&resource_ref),
            Some(&uid),
            Some(&zone_uid),
            Some([2; 32]),
        );
        assert_ne!(first, second);
        assert!(first.starts_with("scope:"));
        assert!(!first.contains("corp-vm"));
        assert!(!first.contains("Process/worker"));
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn typed_process_metadata_rejects_substituted_provider_and_template() {
        let intent = d2b_core::test_support::ResolvedRunnerIntentBuilder::new()
            .with_role_id("cloud-hypervisor")
            .with_role(d2b_core::processes::ProcessRole::CloudHypervisorRunner)
            .with_execution_ref("Host/host-system")
            .build();
        let provider_ref =
            d2b_contracts_resource::v3::ResourceRef::parse("Provider/system-minijail")
                .expect("provider ref");
        let mut provider_digest = Sha256::new();
        provider_digest.update(b"d2b-process-provider-v1");
        provider_digest.update(b"system-minijail");
        let provider_identity: [u8; 32] = provider_digest.finalize().into();
        let mut template_digest = Sha256::new();
        template_digest.update(b"d2b-process-template-v1");
        template_digest.update(b"cloud-hypervisor");
        let template_identity: [u8; 32] = template_digest.finalize().into();

        assert!(
            validate_typed_process_metadata(
                true,
                None,
                Some(&provider_ref),
                Some(provider_identity),
                Some(template_identity),
                None,
                false,
                &intent,
                None,
            )
            .is_ok()
        );
        assert!(
            validate_typed_process_metadata(
                true,
                None,
                Some(&provider_ref),
                Some([9; 32]),
                Some(template_identity),
                None,
                false,
                &intent,
                None,
            )
            .is_err()
        );
        assert!(
            validate_typed_process_metadata(
                true,
                None,
                Some(&provider_ref),
                Some(provider_identity),
                Some([9; 32]),
                None,
                false,
                &intent,
                None,
            )
            .is_err()
        );
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn typed_spawn_allowlist_rejects_identity_and_runtime_substitutions() {
        use d2b_contracts::types::{BundleOpId, RoleId, VmId};
        use d2b_contracts_broker::broker_wire::{
            RunnerAllocation, RunnerAllocationKind, SpawnRunnerRequest,
        };
        use d2b_contracts_resource::v3::{ResourceRef, execution_policy::ExecutionDomain};

        let intent = d2b_core::test_support::ResolvedRunnerIntentBuilder::new()
            .with_intent_id("runner:corp-vm:cloud-hypervisor")
            .with_vm_name("corp-vm")
            .with_role_id("cloud-hypervisor")
            .with_role(d2b_core::processes::ProcessRole::CloudHypervisorRunner)
            .with_execution_ref("Host/host-system")
            .with_cgroup_placement(d2b_core::sandbox_profile::CgroupPlacement {
                subtree: "d2b.slice/corp-vm/cloud-hypervisor".to_owned(),
                controllers: vec!["cpu".to_owned()],
                delegated: false,
            })
            .build();
        let zone_uid =
            d2b_contracts_resource::v3::ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000")
                .expect("zone uid");
        let resource_uid =
            d2b_contracts_resource::v3::ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001")
                .expect("resource uid");
        let resource_ref =
            d2b_contracts_resource::v3::ResourceRef::parse("Process/cloud-hypervisor")
                .expect("resource ref");
        let runtime_scope = private_runtime_scope(
            &zone_uid,
            None,
            &resource_ref,
            &resource_uid,
            &intent.role_id,
            1,
        );
        let mut provider_digest = Sha256::new();
        provider_digest.update(b"d2b-process-provider-v1");
        provider_digest.update(b"system-minijail");
        let provider_identity: [u8; 32] = provider_digest.finalize().into();
        let mut template_digest = Sha256::new();
        template_digest.update(b"d2b-process-template-v1");
        template_digest.update(b"cloud-hypervisor");
        let template_identity: [u8; 32] = template_digest.finalize().into();
        let request = || SpawnRunnerRequest {
            vm_id: VmId::new("corp-vm"),
            role_id: RoleId::new("ch-runner"),
            resource_ref: Some(resource_ref.clone()),
            resource_uid: Some(resource_uid.clone()),
            zone_uid: Some(zone_uid.clone()),
            owner_ref: None,
            owner_uid: None,
            provider_ref: Some(
                d2b_contracts_resource::v3::ResourceRef::parse("Provider/system-minijail")
                    .expect("provider ref"),
            ),
            bundle_content_identity: Some("sha256:bundle".to_owned()),
            provider_identity: Some(provider_identity),
            template_identity: Some(template_identity),
            generation: Some(1),
            runtime_scope: Some(runtime_scope),
            activation_input: None,
            launch_args: None,
            sandbox_plan: None,
            role: RunnerRole::CloudHypervisor,
            bundle_runner_intent_ref: BundleOpId::new(intent.intent_id.clone()),
            execution_ref: Some(ResourceRef::parse("Host/host-system").expect("execution ref")),
            execution_domain: Some(ExecutionDomain::System),
            user_ref: None,
            guest_execution: None,
            runtime_allocations: Vec::new(),
            tracing_span_id: None,
            workload_identity: None,
            inherited_fd_count: 0,
            network_tap_context: None,
        };

        let valid = request();
        let posture = LaunchPosture::resolve(valid.role, &intent);
        assert_eq!(posture, LaunchPosture::Standard);
        assert!(
            validate_spawn_runner_request_matches_intent(&valid, &intent, posture, None).is_ok()
        );

        let mut mutations = Vec::new();
        let mut provider = valid.clone();
        provider.provider_identity = Some([9; 32]);
        mutations.push(provider);
        let mut template = valid.clone();
        template.template_identity = Some([9; 32]);
        mutations.push(template);
        let mut scope = valid.clone();
        scope.runtime_scope = Some([9; 32]);
        mutations.push(scope);
        let mut generation = valid.clone();
        generation.generation = Some(2);
        mutations.push(generation);
        let mut allocations = valid;
        allocations.runtime_allocations = vec![RunnerAllocation {
            kind: RunnerAllocationKind::VsockCid,
            opaque_ref: "cid:attacker".to_owned(),
        }];
        mutations.push(allocations);

        for mutation in mutations {
            assert!(
                validate_spawn_runner_request_matches_intent(&mutation, &intent, posture, None)
                    .is_err(),
                "mutated SpawnRunner request must fail before clone"
            );
        }
    }

    /// The launch-admission decision the `SpawnRunner` dispatch arm runs, in
    /// the arm's own order: posture resolution from the trusted intent ->
    /// inherited-descriptor contract -> trusted-intent fences (which refuse
    /// an untyped ProviderController). The returned posture is the carried
    /// value the rest of the arm reads (uid/gid, response indices, backend
    /// escrow custody).
    #[cfg(not(feature = "layer1-bootstrap"))]
    fn admission_posture(
        req: &d2b_contracts_broker::broker_wire::SpawnRunnerRequest,
        intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent,
        attached_fd_count: usize,
    ) -> Result<LaunchPosture, BrokerError> {
        let posture = LaunchPosture::resolve(req.role, intent);
        posture.validate_request_fds(req.inherited_fd_count, attached_fd_count)?;
        validate_spawn_runner_request_matches_intent(req, intent, posture, None)?;
        Ok(posture)
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    fn provider_controller_template_identity(profile_id: &str) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"d2b-process-template-v1");
        digest.update(profile_id.as_bytes());
        digest.finalize().into()
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    fn provider_identity_digest(provider_name: &str) -> [u8; 32] {
        let mut digest = Sha256::new();
        digest.update(b"d2b-process-provider-v1");
        digest.update(provider_name.as_bytes());
        digest.finalize().into()
    }

    /// The signed binding-owned serving-worker intent: `virtiofsd-worker`
    /// under `Provider/volume-virtiofs`, with the per-binding principal and
    /// ADR-0021 user namespace the resolver mints for that template
    /// (`uid = gid = principal >= 50_000`, in-NS root mapped to it). The
    /// single raw condition lives in `intent_is_serving_worker_template`;
    /// every fixture below reaches it through the production code paths.
    #[cfg(not(feature = "layer1-bootstrap"))]
    fn serving_worker_runner_intent() -> d2b_core::bundle_resolver::ResolvedRunnerIntent {
        let mut intent = d2b_core::test_support::ResolvedRunnerIntentBuilder::new()
            .with_intent_id("runner:vm:host-system:role:volume-worker")
            .with_vm_name("host-system")
            .with_role(d2b_core::processes::ProcessRole::ProviderController)
            .with_role_id("volume-worker")
            .with_profile_id("virtiofsd-worker")
            .with_execution_ref("Host/host-system")
            .with_uid(4242)
            .with_gid(4242)
            .with_user_namespace(Some(d2b_core::bundle_resolver::UserNamespaceSpec {
                host_uid_for_zero: 4242,
                host_gid_for_zero: 4242,
            }))
            .build();
        intent.owner_ref = Some("Provider/volume-virtiofs".to_owned());
        intent
    }

    /// A fully typed Process launch request for `intent`, in the shape the
    /// dispatch arm sees after daemon-side classification.
    #[cfg(not(feature = "layer1-bootstrap"))]
    fn typed_runner_request(
        intent: &d2b_core::bundle_resolver::ResolvedRunnerIntent,
        role: RunnerRole,
        resource_name: &str,
        owner_ref: Option<&str>,
        inherited_fd_count: u16,
    ) -> d2b_contracts_broker::broker_wire::SpawnRunnerRequest {
        use d2b_contracts::types::{BundleOpId, RoleId, VmId};
        use d2b_contracts_broker::broker_wire::SpawnRunnerRequest;
        use d2b_contracts_resource::v3::{
            ResourceRef, ResourceUid, execution_policy::ExecutionDomain,
        };
        use d2b_core::processes::ProcessRole;

        let resource_ref =
            ResourceRef::parse(&format!("Process/{resource_name}")).expect("resource ref");
        let resource_uid =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("resource uid");
        let zone_uid =
            ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").expect("zone uid");
        let runtime_scope = private_runtime_scope(
            &zone_uid,
            None,
            &resource_ref,
            &resource_uid,
            &intent.role_id,
            1,
        );
        let template_identity = if intent.role == ProcessRole::ProviderController
            || d2b_core::bundle_resolver::is_device_worker_role(&intent.role)
        {
            provider_controller_template_identity(&intent.profile_id)
        } else {
            provider_controller_template_identity(&intent.role_id)
        };
        let execution_domain = match intent.execution_domain {
            d2b_core::processes::ProcessExecutionDomain::System => ExecutionDomain::System,
            d2b_core::processes::ProcessExecutionDomain::User => ExecutionDomain::User,
        };
        let wire_role_id = wire_role_id_for_intent(intent).to_owned();
        SpawnRunnerRequest {
            vm_id: VmId::new(intent.vm_name.clone()),
            role_id: RoleId::new(wire_role_id),
            resource_ref: Some(resource_ref),
            resource_uid: Some(resource_uid),
            zone_uid: Some(zone_uid),
            owner_ref: owner_ref.map(|owner| ResourceRef::parse(owner).expect("owner ref")),
            owner_uid: None,
            provider_ref: Some(
                ResourceRef::parse("Provider/system-minijail").expect("provider ref"),
            ),
            bundle_content_identity: Some("sha256:bundle".to_owned()),
            provider_identity: Some(provider_identity_digest("system-minijail")),
            template_identity: Some(template_identity),
            generation: Some(1),
            runtime_scope: Some(runtime_scope),
            activation_input: None,
            launch_args: None,
            sandbox_plan: None,
            role,
            bundle_runner_intent_ref: BundleOpId::new(intent.intent_id.clone()),
            execution_ref: Some(
                d2b_contracts_resource::v3::ResourceRef::parse(&intent.execution_ref)
                    .expect("execution ref"),
            ),
            execution_domain: Some(execution_domain),
            user_ref: None,
            guest_execution: None,
            runtime_allocations: Vec::new(),
            tracing_span_id: None,
            workload_identity: None,
            inherited_fd_count,
            network_tap_context: None,
        }
    }

    /// The posture's descriptor-contract decision set, evaluated through the
    /// same production methods the dispatch arm and backend read. One test
    /// per posture pins this whole set, so dropping or diverging any single
    /// decision site (escrow contract, escrow custody, response indices, fd
    /// shape) fails that posture's test.
    ///
    /// Executor identity is deliberately NOT part of it: no posture decides
    /// it (`prepare_runner_launch_identity` takes it from the trusted intent,
    /// and `serving_worker_launch_runs_as_the_intent_principal` pins that).
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[derive(Debug, PartialEq, Eq)]
    struct PostureDecisions {
        bootstrap_response_index: Option<u32>,
        console_response_index_with_extras: Option<u32>,
        carries_controller_escrow: bool,
        accepts_zero_fds: bool,
        accepts_escrow_fd_shape: bool,
        accepts_two_escrow_fds: bool,
        rejects_unattached_inherited_fd: bool,
        rejects_over_max_fds: bool,
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    fn posture_decisions(posture: LaunchPosture) -> PostureDecisions {
        PostureDecisions {
            bootstrap_response_index: posture.bootstrap_response_index(),
            console_response_index_with_extras: posture.console_response_index(1),
            carries_controller_escrow: posture.carries_controller_escrow(),
            accepts_zero_fds: posture.validate_request_fds(0, 0).is_ok(),
            accepts_escrow_fd_shape: posture.validate_request_fds(1, 2).is_ok(),
            accepts_two_escrow_fds: posture.validate_request_fds(2, 3).is_ok(),
            rejects_unattached_inherited_fd: posture.validate_request_fds(1, 1).is_err(),
            rejects_over_max_fds: posture.validate_request_fds(257, 0).is_err(),
        }
    }

    /// R35 blocker: the serving-worker posture is a property of the resolved
    /// trusted intent. An untyped ProviderController request that carries a
    /// `VolumeBinding` owner ref (the exploit the review found) is refused by
    /// the trusted-intent fence, and the posture predicate itself ignores
    /// every request field, so no request can select the daemon uid/gid,
    /// the in-namespace root mapping, or the zero-fd escrow exemption.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn untyped_provider_controller_cannot_claim_serving_worker_posture() {
        use d2b_contracts::types::{BundleOpId, RoleId, VmId};
        use d2b_contracts_broker::broker_wire::SpawnRunnerRequest;
        use d2b_contracts_resource::v3::{ResourceRef, execution_policy::ExecutionDomain};

        let intent = serving_worker_runner_intent();
        // The reference intent is the legitimate serving template, so even
        // referencing a real serving intent must not hand an untyped
        // request the posture.
        assert_eq!(
            LaunchPosture::resolve(RunnerRole::ProviderController, &intent),
            LaunchPosture::ServingWorker
        );

        let attack = SpawnRunnerRequest {
            vm_id: VmId::new("host-system"),
            role_id: RoleId::new("volume-worker"),
            resource_ref: None,
            resource_uid: None,
            zone_uid: None,
            owner_ref: Some(ResourceRef::parse("VolumeBinding/forged").expect("owner ref")),
            owner_uid: None,
            provider_ref: None,
            bundle_content_identity: None,
            provider_identity: None,
            template_identity: None,
            generation: None,
            runtime_scope: None,
            activation_input: None,
            launch_args: None,
            sandbox_plan: None,
            role: RunnerRole::ProviderController,
            bundle_runner_intent_ref: BundleOpId::new(intent.intent_id.clone()),
            execution_ref: Some(ResourceRef::parse("Host/host-system").expect("execution ref")),
            execution_domain: Some(ExecutionDomain::System),
            user_ref: None,
            guest_execution: None,
            runtime_allocations: Vec::new(),
            tracing_span_id: None,
            workload_identity: None,
            inherited_fd_count: 0,
            network_tap_context: None,
        };

        let error = validate_spawn_runner_request_matches_intent(
            &attack,
            &intent,
            LaunchPosture::resolve(attack.role, &intent),
            None,
        )
        .expect_err("untyped ProviderController launch must be refused");
        match error {
            BrokerError::SpawnRunnerIntentMismatch {
                field, requested, ..
            } => {
                assert_eq!(field, "process_identity");
                assert_eq!(requested, "untyped");
            }
            other => panic!("expected SpawnRunnerIntentMismatch, got {other:?}"),
        }
        assert!(
            admission_posture(&attack, &intent, 0).is_err(),
            "the attack request must never reach the serving-worker posture"
        );

        // The posture resolves from the trusted intent alone: a different
        // semantic owner or a different profile keeps the ordinary
        // ProviderController posture regardless of any request claim, and a
        // non-controller role never reaches the serving-worker posture.
        let mut foreign_owner = intent.clone();
        foreign_owner.owner_ref = Some("Provider/volume-other".to_owned());
        assert_eq!(
            LaunchPosture::resolve(RunnerRole::ProviderController, &foreign_owner),
            LaunchPosture::ControllerEscrow
        );
        let mut other_profile = intent.clone();
        other_profile.profile_id = "controller-worker".to_owned();
        assert_eq!(
            LaunchPosture::resolve(RunnerRole::ProviderController, &other_profile),
            LaunchPosture::ControllerEscrow
        );
        assert_eq!(
            LaunchPosture::resolve(RunnerRole::CloudHypervisor, &intent),
            LaunchPosture::Standard,
            "only a controller-role request can carry the serving-worker posture"
        );
    }

    /// Serving-worker posture: the descriptor decision set at once. It stays
    /// admitted only as a typed launch whose owner is the runtime-minted
    /// VolumeBinding, keeps the zero-fd escrow exemption (enforced: an
    /// escrow-shaped descriptor set is refused), runs as the trusted intent's
    /// principal, and advertises no bootstrap index.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn serving_worker_posture_decides_the_whole_launch() {
        use d2b_contracts_resource::v3::ResourceRef;

        let intent = serving_worker_runner_intent();
        let posture = LaunchPosture::resolve(RunnerRole::ProviderController, &intent);
        assert_eq!(posture, LaunchPosture::ServingWorker);

        assert_eq!(
            posture_decisions(posture),
            PostureDecisions {
                bootstrap_response_index: None,
                console_response_index_with_extras: None,
                carries_controller_escrow: false,
                accepts_zero_fds: true,
                // The zero-fd contract the enum documents is enforced here:
                // an escrow-shaped descriptor set is NOT admitted, because no
                // custody is retained for it and it would be installed at the
                // Provider bootstrap fd slot.
                accepts_escrow_fd_shape: false,
                accepts_two_escrow_fds: false,
                rejects_unattached_inherited_fd: true,
                rejects_over_max_fds: true,
            }
        );

        let request = typed_runner_request(
            &intent,
            RunnerRole::ProviderController,
            "vol-worker",
            Some("VolumeBinding/vol-binding"),
            0,
        );
        let admitted = admission_posture(&request, &intent, 0).expect("typed serving worker");
        assert_eq!(admitted, LaunchPosture::ServingWorker);
        // Pre-PR security-review regression (low): a serving worker that
        // presents the controller-escrow descriptor shape is refused at the
        // single evaluation point. It is not custodied
        // (`carries_controller_escrow` is false) and no duplicate is returned,
        // so admitting it would install a caller-chosen descriptor at the
        // well-known Provider bootstrap fd slot (10) inside the worker.
        let mut with_escrow_shape = request.clone();
        with_escrow_shape.inherited_fd_count = 1;
        let error = admission_posture(&with_escrow_shape, &intent, 2)
            .expect_err("serving worker must carry no inherited fds");
        assert!(
            matches!(
                error,
                BrokerError::Protocol(ref message)
                    if message == "binding-owned serving worker must carry no inherited fds"
            ),
            "unexpected refusal: {error:?}"
        );
        // The owner fence demands the runtime-minted VolumeBinding; the
        // Provider that signed the serving template is refused.
        let mut provider_owner = request.clone();
        provider_owner.owner_ref =
            Some(ResourceRef::parse("Provider/volume-virtiofs").expect("owner ref"));
        assert!(
            validate_spawn_runner_request_matches_intent(&provider_owner, &intent, posture, None)
                .is_err()
        );
        // The serving template does not admit controller-supplied arguments,
        // so the launch argv stays the trusted intent's.
        let mut with_launch_args = request;
        with_launch_args.launch_args = Some(
            d2b_contracts_broker::broker_wire::RunnerLaunchArgs::new(vec!["--forge".to_owned()])
                .expect("launch args"),
        );
        assert!(
            validate_spawn_runner_request_matches_intent(&with_launch_args, &intent, posture, None)
                .is_err()
        );
    }

    /// Pre-PR security-review regression: the binding-owned serving worker was
    /// launched with the DAEMON's uid/gid and its in-namespace root mapped to
    /// the daemon identity. The broker's only authentication factor is peer
    /// identity - `peer_matches_instance` admits exactly the daemon uid/gid on
    /// the privileged socket - so a guest -> worker compromise yielded a
    /// process that could connect the broker socket, pass the pre-decode peer
    /// check, and use the whole daemon API (admin-class gates, bundle
    /// activation, SpawnRunner for any role/VM, read/write on every
    /// daemon-owned path).
    ///
    /// The launch identity is the trusted intent's principal for every
    /// posture, and the serving worker's two ticket-named trees are opened to
    /// that principal with per-runner ACLs instead: this drives the same
    /// entry point the dispatch arm calls, with the daemon's own ticket argv.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn serving_worker_launch_runs_as_the_intent_principal() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = test_audit_dir("serving-worker-principal");
        let config = test_server_config(&root, &root.join("broker.sock"));
        let intent = serving_worker_runner_intent();
        let posture = LaunchPosture::resolve(RunnerRole::ProviderController, &intent);
        assert_eq!(posture, LaunchPosture::ServingWorker);

        // The daemon's serving-worker ticket paths, realized the way it
        // provisions them: a private socket directory below the broker's
        // runtime directory (0700, daemon-owned) and a served view root.
        let socket_dir = root.join("vms").join("host-system");
        std::fs::create_dir_all(&socket_dir).expect("create socket dir");
        std::fs::set_permissions(&socket_dir, std::fs::Permissions::from_mode(0o700))
            .expect("chmod socket dir");
        let served = root.join("store-view").join("live");
        std::fs::create_dir_all(&served).expect("create served root");
        std::fs::set_permissions(&served, std::fs::Permissions::from_mode(0o750))
            .expect("chmod served root");
        let argv = vec![
            "virtiofsd".to_owned(),
            format!(
                "--socket-path={}",
                socket_dir.join("vol-abcd.vfd.sock").display()
            ),
            format!("--shared-dir={}", served.display()),
        ];

        let (uid, gid, namespace) =
            prepare_runner_launch_identity(posture, &config, &intent, &argv)
                .expect("serving-worker launch identity");
        assert_eq!(
            (uid, gid),
            (intent.uid, intent.gid),
            "a serving worker runs as the trusted intent's principal"
        );
        assert_ne!(
            (uid, gid),
            (config.d2bd_uid, config.d2bd_gid),
            "a serving worker must never carry the daemon identity: the \
             privileged socket admits exactly that uid/gid"
        );
        assert_eq!(
            namespace.map(|spec| (spec.host_uid_for_zero, spec.host_gid_for_zero)),
            Some((intent.uid, intent.gid)),
            "in-namespace root maps to the intent's principal (ADR 0021), \
             not to the daemon identity"
        );
        // The two trees the ticket names are opened to that principal
        // (asserted through the setfacl xattr, skipped when the host has no
        // setfacl binary).
        if [
            "/run/current-system/sw/bin/setfacl",
            "/usr/bin/setfacl",
            "/bin/setfacl",
        ]
        .iter()
        .any(|candidate| Path::new(candidate).exists())
        {
            for path in [&socket_dir, &served] {
                let fd = crate::sys::path_safe::open_dir_path_safe(path).expect("open dir");
                assert_eq!(
                    crate::sys::path_safe::fd_extended_acl_present(fd.as_fd())
                        .expect("inspect ACL"),
                    (true, false),
                    "{} must carry the per-runner access ACL",
                    path.display()
                );
            }
        }

        let _ = fs::remove_dir_all(&root);
    }

    /// Non-serving ProviderController posture: the whole decision set at once.
    /// It must carry the bootstrap escrow descriptor (the broker retains it
    /// as registry custody and returns no duplicate), keeps the intent
    /// uid/gid, and its owner fence is the bundle owner.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn controller_escrow_posture_decides_the_whole_launch() {
        use d2b_contracts_resource::v3::ResourceRef;
        use d2b_core::processes::ProcessRole;

        let intent = d2b_core::test_support::ResolvedRunnerIntentBuilder::new()
            .with_intent_id("runner:vm:host-system:role:volume-worker")
            .with_vm_name("host-system")
            .with_role(ProcessRole::ProviderController)
            .with_role_id("volume-worker")
            .with_profile_id("controller-worker")
            .with_execution_ref("Host/host-system")
            .with_uid(4242)
            .with_gid(4242)
            .build();
        let posture = LaunchPosture::resolve(RunnerRole::ProviderController, &intent);
        assert_eq!(posture, LaunchPosture::ControllerEscrow);

        assert_eq!(
            posture_decisions(posture),
            PostureDecisions {
                bootstrap_response_index: None,
                console_response_index_with_extras: None,
                carries_controller_escrow: true,
                accepts_zero_fds: false,
                accepts_escrow_fd_shape: true,
                accepts_two_escrow_fds: true,
                rejects_unattached_inherited_fd: true,
                rejects_over_max_fds: true,
            }
        );

        let request = typed_runner_request(
            &intent,
            RunnerRole::ProviderController,
            "vol-worker",
            None,
            1,
        );
        let admitted = admission_posture(&request, &intent, 2).expect("escrow controller");
        assert_eq!(admitted, LaunchPosture::ControllerEscrow);
        // A controller-role launch that presents no escrow descriptor, or an
        // owner other than the bundle owner, is refused before any spawn.
        let mut escrowless = request.clone();
        escrowless.inherited_fd_count = 0;
        assert!(admission_posture(&escrowless, &intent, 0).is_err());
        let mut foreign_owner = request;
        foreign_owner.owner_ref =
            Some(ResourceRef::parse("VolumeBinding/forged").expect("owner ref"));
        assert!(
            validate_spawn_runner_request_matches_intent(&foreign_owner, &intent, posture, None)
                .is_err()
        );
    }

    /// Ordinary runner posture: the descriptor decision set at once. No
    /// escrow descriptor is admitted or advertised, and the console-socket
    /// index is used when the launch carries extra response fds.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn standard_posture_decides_the_whole_launch() {
        use d2b_core::processes::ProcessRole;

        let intent = d2b_core::test_support::ResolvedRunnerIntentBuilder::new()
            .with_intent_id("runner:vm:corp-vm:role:virtiofsd")
            .with_vm_name("corp-vm")
            .with_role(ProcessRole::Virtiofsd)
            .with_role_id("virtiofsd")
            .with_execution_ref("Host/host-system")
            .with_uid(2000)
            .with_gid(2000)
            .build();
        let posture = LaunchPosture::resolve(RunnerRole::Virtiofsd, &intent);
        assert_eq!(posture, LaunchPosture::Standard);

        assert_eq!(
            posture_decisions(posture),
            PostureDecisions {
                bootstrap_response_index: None,
                console_response_index_with_extras: Some(1),
                carries_controller_escrow: false,
                accepts_zero_fds: true,
                accepts_escrow_fd_shape: false,
                accepts_two_escrow_fds: false,
                rejects_unattached_inherited_fd: true,
                rejects_over_max_fds: true,
            }
        );

        let request = typed_runner_request(&intent, RunnerRole::Virtiofsd, "fs", None, 0);
        let admitted = admission_posture(&request, &intent, 0).expect("ordinary runner");
        assert_eq!(admitted, LaunchPosture::Standard);
        // An ordinary launch may never carry a controller escrow descriptor.
        let mut with_inherited_fd = request;
        with_inherited_fd.inherited_fd_count = 1;
        assert!(admission_posture(&with_inherited_fd, &intent, 2).is_err());
    }

    /// Device-worker posture: a Device-owned worker row (`Process/swtpm-<device>`,
    /// `Process/gpu-<device>`, `EphemeralProcess/swtpm-flush-<device>`) launches
    /// through its own closed broker runner role, never as a Provider
    /// controller: zero escrow descriptors, the intent's principal uid/gid,
    /// and a Device owner (the Device the row is declared under). Its template
    /// identity is the declared template, not the per-row role id.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn device_worker_posture_decides_the_whole_launch() {
        use d2b_contracts_resource::v3::ResourceRef;
        use d2b_core::processes::ProcessRole;

        let intent = d2b_core::test_support::ResolvedRunnerIntentBuilder::new()
            .with_intent_id("runner:vm:host-system:role:swtpm-tpm")
            .with_vm_name("host-system")
            .with_role(ProcessRole::Swtpm)
            .with_role_id("swtpm-tpm")
            .with_profile_id("swtpm-socket")
            .with_execution_ref("Host/host-system")
            .with_uid(60_100)
            .with_gid(60_100)
            .build();
        let posture = LaunchPosture::resolve(RunnerRole::Swtpm, &intent);
        assert_eq!(posture, LaunchPosture::Standard);
        assert!(!posture.is_provider_controller());

        let request = typed_runner_request(
            &intent,
            RunnerRole::Swtpm,
            "swtpm-tpm",
            Some("Device/tpm"),
            0,
        );
        let admitted = admission_posture(&request, &intent, 0).expect("device worker");
        assert_eq!(admitted, LaunchPosture::Standard);

        // The row's semantic owner is the Device it is declared under: another
        // owner type, or none at all, is refused before any spawn.
        for owner in [Some("Provider/device-tpm"), Some("Guest/dev"), None] {
            let mut forged = request.clone();
            forged.owner_ref = owner.map(|owner| ResourceRef::parse(owner).expect("owner ref"));
            assert!(
                validate_spawn_runner_request_matches_intent(&forged, &intent, posture, None)
                    .is_err(),
                "a Device worker must carry its Device owner, got {owner:?}"
            );
        }
        // The Device the request names must be the Device that owns the
        // launched row under the verified bundle: the arm pins that scope and
        // this fence refuses any request that disagrees with it (a launch
        // aimed at another Device would derive another Guest's runtime paths).
        let pinned = crate::ops::device_worker::DeviceWorkerScope {
            zone_uid: d2b_contracts_resource::v3::ResourceUid::parse(
                "123e4567-e89b-42d3-a456-426614174000",
            )
            .expect("zone uid"),
            device_ref: ResourceRef::parse("Device/gpu0").expect("device ref"),
            device_uid: d2b_contracts_resource::v3::ResourceUid::parse(
                "4f37d6a6-759a-4c81-9a55-eaf2e00665e8",
            )
            .expect("device uid"),
            guest: "other-guest".to_owned(),
        };
        let error =
            validate_spawn_runner_request_matches_intent(&request, &intent, posture, Some(&pinned))
                .expect_err("a request claiming another Device must be refused by name");
        match error {
            BrokerError::SpawnRunnerIntentMismatch {
                field,
                requested,
                resolved,
            } => {
                assert_eq!(field, "owner_ref");
                assert_eq!(requested, "Device/tpm");
                assert_eq!(resolved, "Device/gpu0");
            }
            other => panic!("expected an owner_ref refusal, got {other:?}"),
        }
        // The template identity is the declared template; the per-row role id
        // is not admitted.
        let mut wrong_template = request;
        wrong_template.template_identity = Some(provider_controller_template_identity("swtpm-tpm"));
        assert!(
            validate_spawn_runner_request_matches_intent(&wrong_template, &intent, posture, None)
                .is_err()
        );
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn open_pidfd_maps_cloud_hypervisor_compat_role_to_bundle_intent() {
        assert_eq!(
            runner_intent_id_for_open_pidfd("acceptance-guest", "ch-runner"),
            "runner:vm:acceptance-guest:role:cloud-hypervisor"
        );
        assert_eq!(
            runner_intent_id_for_open_pidfd("acceptance-guest", "swtpm"),
            "runner:vm:acceptance-guest:role:swtpm"
        );
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn dummy_fd() -> OwnedFd {
        std::fs::File::open("/dev/null")
            .expect("open /dev/null for dummy fd")
            .into()
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    fn is_uuid_v4_like(value: &str) -> bool {
        let chars: Vec<char> = value.chars().collect();
        chars.len() == 36
            && matches!(chars.get(8), Some('-'))
            && matches!(chars.get(13), Some('-'))
            && matches!(chars.get(18), Some('-'))
            && matches!(chars.get(23), Some('-'))
            && matches!(chars.get(14), Some('4'))
            && matches!(chars.get(19), Some('8' | '9' | 'a' | 'b'))
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[derive(Default)]
    struct FakeDispatchBackend {
        registered_runners: Mutex<std::collections::BTreeSet<String>>,
        usbip_events: Mutex<Vec<FakeUsbipEvent>>,
        envelope: crate::envelope::BrokerEnvelope,
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[derive(Debug, Clone, PartialEq, Eq)]
    enum FakeUsbipEvent {
        Bind { intent_id: String },
        Unbind { intent_id: String },
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    impl FakeDispatchBackend {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn remember_runner(&self, runner_id: &str) -> Result<(), BrokerError> {
            self.registered_runners
                .lock()
                .map_err(|_| {
                    BrokerError::Protocol("fake runner registry mutex poisoned".to_owned())
                })?
                .insert(runner_id.to_owned());
            Ok(())
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn has_runner(&self, runner_id: &str) -> Result<bool, BrokerError> {
            Ok(self
                .registered_runners
                .lock()
                .map_err(|_| {
                    BrokerError::Protocol("fake runner registry mutex poisoned".to_owned())
                })?
                .contains(runner_id))
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn push_usbip_event(&self, event: FakeUsbipEvent) -> Result<(), BrokerError> {
            self.usbip_events
                .lock()
                .map_err(|_| BrokerError::Protocol("fake USBIP event mutex poisoned".to_owned()))?
                .push(event);
            Ok(())
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn take_usbip_events(&self) -> Vec<FakeUsbipEvent> {
            let mut events = self.usbip_events.lock().expect("fake USBIP event lock");
            std::mem::take(&mut *events)
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    impl DispatchBackend for FakeDispatchBackend {
        fn operation_envelope(&self) -> &crate::envelope::BrokerEnvelope {
            &self.envelope
        }

        fn apply_nftables<'a>(
            &'a self,
            _resolver: &'a BundleResolver,
            _intent: &'a d2b_core::bundle_resolver::ResolvedNftIntent,
            _desired_hash: Option<&'a str>,
            _destroy: bool,
        ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            Ok(())
        
            })
        }

        fn apply_route<'a>(
            &'a self,
            _state_dir: &'a Path,
            _intent: &'a d2b_core::bundle_resolver::ResolvedRouteIntent,
            _provenance: &'a d2b_contracts_resource::v3::NetworkProvenance,
            _destroy: bool,
        ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            Ok(())
        
            })
        }

        fn apply_sysctl<'a>(
            &'a self,
            _intent: &'a d2b_core::bundle_resolver::ResolvedSysctlIntent,
            _destroy: bool,
        ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            Ok(())
        
            })
        }

        fn update_hosts_file<'a>(
            &'a self,
            _intent: &'a d2b_core::bundle_resolver::ResolvedHostsIntent,
            _destroy: bool,
        ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            Ok(())
        
            })
        }

        fn apply_nm_unmanaged<'a>(
            &'a self,
            _intent: &'a d2b_core::bundle_resolver::ResolvedNmUnmanagedIntent,
            _destroy: bool,
        ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            Ok(())
        
            })
        }

        fn set_bridge_port_flags<'a>(
            &'a self,
            req: &'a d2b_contracts_broker::broker_wire::SetBridgePortFlagsRequest,
            _resolver: &'a BundleResolver,
        ) -> Pin<Box<dyn Future<Output = Result<d2b_contracts_broker::broker_wire::BridgePortFlagsResponse, BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            Ok(d2b_contracts_broker::broker_wire::BridgePortFlagsResponse {
                bridge: d2b_contracts_resource::v3::IfName::new("nlworkbr0")
                    .expect("fake bridge ifname"),
                isolated: true,
                neigh_suppress: true,
                port: d2b_contracts_resource::v3::IfName::new(&format!(
                    "tap-{}",
                    req.vm_id.as_str()
                ))
                .expect("fake tap ifname"),
            })
        
            })
        }

        fn open_pidfd<'a>(
            &'a self,
            runner_id: &'a str,
            pid: i32,
            expected_start_time_ticks: u64,
        ) -> Pin<Box<dyn Future<Output = Result<crate::live_handlers::OpenPidfdResult, BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            self.remember_runner(runner_id)?;
            Ok(crate::live_handlers::OpenPidfdResult {
                pidfd: dummy_fd(),
                pid,
                verified_start_time_ticks: expected_start_time_ticks,
            })
        
            })
        }

        fn signal_runner<'a>(
            &'a self,
            runner_id: &'a str,
            _signal: d2b_contracts_broker::broker_wire::RunnerSignal,
        ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            if self.has_runner(runner_id)? {
                Ok(())
            } else {
                Err(BrokerError::NoPidfd {
                    runner_id: runner_id.to_owned(),
                })
            }
        
            })
        }

        fn spawn_runner<'a>(
            &'a self,
            runner_id: &'a str,
            _plan_input: &'a crate::ops::spawn_runner::SpawnRunnerPlanInput,
            _resolver: &'a BundleResolver,
            _req: &'a d2b_contracts_broker::broker_wire::SpawnRunnerRequest,
            _posture: LaunchPosture,
            _device_worker: &'a crate::ops::device_worker::DeviceWorkerLaunch,
            request_fds: Vec<OwnedFd>,
            _audit_log: &'a crate::audit::AuditLog,
        ) -> Pin<Box<dyn Future<Output = Result<crate::live_handlers::SpawnRunnerResult, BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            drop(request_fds);
            self.remember_runner(runner_id)?;
            Ok(crate::live_handlers::SpawnRunnerResult {
                pidfd: dummy_fd(),
                extra_response_fds: Vec::new(),
                pid: 4242,
                start_time_ticks: 123456,
                used_fork_fallback: false,
                swtpm_dir_audit: None,
            })
        
            })
        }

        fn apply_host_generation_handoff<'a>(
            &'a self,
            state_dir: &'a std::path::Path,
            helper_path: &'a std::path::Path,
            request: &'a d2b_contracts_broker::host_generation::ApplyHostGenerationHandoff,
        ) -> Pin<Box<dyn Future<Output = Result<
            d2b_contracts_broker::broker_wire::ApplyHostGenerationHandoffResponse,
            BrokerError,
        >> + Send + 'a>> {
            Box::pin(async move {
            crate::ops::host_generation_handoff::apply_with_helper(state_dir, helper_path, request)
                .await
                .map_err(|error| BrokerError::LiveHandler(error.to_string()))
            })
        }

        fn usbip_bind<'a>(
            &'a self,
            intent: &'a d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
        ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            self.push_usbip_event(FakeUsbipEvent::Bind {
                intent_id: intent.intent_id.clone(),
            })?;
            Ok(())
        
            })
        }

        fn usbip_unbind<'a>(
            &'a self,
            intent: &'a d2b_core::bundle_resolver::ResolvedUsbipBindIntent,
        ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            self.push_usbip_event(FakeUsbipEvent::Unbind {
                intent_id: intent.intent_id.clone(),
            })?;
            Ok(())
        
            })
        }

        fn usbip_bind_firewall_rule<'a>(
            &'a self,
            _resolver: &'a BundleResolver,
            _intent: &'a d2b_core::bundle_resolver::ResolvedUsbipFirewallIntent,
        ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            Ok(())
        
            })
        }

        fn usbip_proxy_reconcile<'a>(
            &'a self,
            _expectations: &'a [(String, String, PathBuf)],
        ) -> Pin<Box<dyn Future<Output = Result<(), BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            Ok(())
        
            })
        }

        fn qemu_media_system_powerdown<'a>(
            &'a self,
            req: &'a d2b_contracts_broker::broker_wire::QemuMediaLifecycleRequest,
        ) -> Pin<Box<dyn Future<Output = Result<d2b_contracts_broker::broker_wire::QemuMediaLifecycleResponse, BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            Ok(
                d2b_contracts_broker::broker_wire::QemuMediaLifecycleResponse {
                    vm_id: req.vm_id.clone(),
                    command:
                        d2b_contracts_broker::broker_wire::QemuMediaLifecycleAction::SystemPowerdown,
                },
            )
        
            })
        }

        fn qemu_media_query_status<'a>(
            &'a self,
            req: &'a d2b_contracts_broker::broker_wire::QemuMediaQueryStatusRequest,
        ) -> Pin<Box<dyn Future<Output = Result<d2b_contracts_broker::broker_wire::QemuMediaQueryStatusResponse, BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            let status = if req.shutdown_context {
                d2b_contracts_broker::broker_wire::QemuMediaVmStatus::ConnectionLostDuringShutdown
            } else {
                d2b_contracts_broker::broker_wire::QemuMediaVmStatus::Running
            };
            Ok(
                d2b_contracts_broker::broker_wire::QemuMediaQueryStatusResponse {
                    vm_id: req.vm_id.clone(),
                    status,
                },
            )
        
            })
        }

        fn qemu_media_quit<'a>(
            &'a self,
            req: &'a d2b_contracts_broker::broker_wire::QemuMediaLifecycleRequest,
        ) -> Pin<Box<dyn Future<Output = Result<d2b_contracts_broker::broker_wire::QemuMediaLifecycleResponse, BrokerError>> + Send + 'a>> {
            Box::pin(async move {
            Ok(
                d2b_contracts_broker::broker_wire::QemuMediaLifecycleResponse {
                    vm_id: req.vm_id.clone(),
                    command: d2b_contracts_broker::broker_wire::QemuMediaLifecycleAction::Quit,
                },
            )
        
            })
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn qemu_media_lifecycle_dispatch_audits_mutations_but_not_status_poll() {
        use d2b_contracts::types::{TracingSpanId, VmId};
        use d2b_contracts_broker::broker_wire::{
            BrokerCallerRole, BrokerRequest, QemuMediaLifecycleAction, QemuMediaLifecycleRequest,
            QemuMediaQueryStatusRequest, QemuMediaVmStatus,
        };

        let root = test_audit_dir("qemu-lifecycle-dispatch-audit");
        let bundle = build_test_bundle(&root);
        let config = test_server_config(&root, &bundle.manifest_path);
        let (log, capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");
        let backend = FakeDispatchBackend::default();
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let caller_gid = Gid::current().as_raw();

        let dispatch = |request: BrokerRequest| {
            let audit_context = DispatchAuditContext::from_request(&request, 4242, &caller_role)
                .expect("audit context");
            envelope_call_runtime().block_on(dispatch_request_with_backend(
                request,
                1000,
                caller_gid,
                caller_role.clone(),
                &audit_context,
                &config,
                &log,
                Some(&bundle.resolver),
                &backend,
            ))
            .expect("dispatch succeeds")
        };

        let powerdown = dispatch(BrokerRequest::QemuMediaSystemPowerdown(
            QemuMediaLifecycleRequest {
                vm_id: VmId::new("media"),
                tracing_span_id: Some(TracingSpanId::new("span-powerdown")),
            },
        ));
        match powerdown.response {
            BrokerResponse::QemuMediaSystemPowerdown(response) => {
                assert_eq!(response.command, QemuMediaLifecycleAction::SystemPowerdown);
            }

            other => panic!("expected QemuMediaSystemPowerdown, got {other:?}"),
        }

        let before_query = capture.lock().expect("capture before query").len();
        let query = dispatch(BrokerRequest::QemuMediaQueryStatus(
            QemuMediaQueryStatusRequest {
                vm_id: VmId::new("media"),
                shutdown_context: true,
                tracing_span_id: Some(TracingSpanId::new("span-query")),
            },
        ));
        match query.response {
            BrokerResponse::QemuMediaQueryStatus(response) => {
                assert_eq!(
                    response.status,
                    QemuMediaVmStatus::ConnectionLostDuringShutdown
                );
            }
            other => panic!("expected QemuMediaQueryStatus, got {other:?}"),
        }
        assert_eq!(
            capture.lock().expect("capture after query").len(),
            before_query,
            "query-status polling must not emit success audit records"
        );

        let quit = dispatch(BrokerRequest::QemuMediaQuit(QemuMediaLifecycleRequest {
            vm_id: VmId::new("media"),
            tracing_span_id: Some(TracingSpanId::new("span-quit")),
        }));
        match quit.response {
            BrokerResponse::QemuMediaQuit(response) => {
                assert_eq!(response.command, QemuMediaLifecycleAction::Quit);
            }
            other => panic!("expected QemuMediaQuit, got {other:?}"),
        }

        let records = capture.lock().expect("capture final");
        let qmp_records: Vec<_> = records
            .iter()
            .filter(|record| record.operation.starts_with("QemuMedia"))
            .collect();
        assert_eq!(qmp_records.len(), 2);
        assert_eq!(qmp_records[0].operation, "QemuMediaSystemPowerdown");
        assert_eq!(qmp_records[1].operation, "QemuMediaQuit");

        let _ = fs::remove_dir_all(&root);
    }

    /// The U10 sandwich kernels the retired process-family wire arms left
    /// behind (KTD10): each committed broker-generic kernel row dispatches
    /// through the envelope under the row's own payload contract and fd
    /// facet. What was family knowledge in the retired arms - resolving a
    /// zone-native subject from the trusted bundle, deriving an identity
    /// from an accepted socket, matching a runner intent - stays on the
    /// family side after the cut, so the broker-side surface is exactly
    /// this kernel seam.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn retired_process_family_kernels_dispatch_through_the_envelope() {
        use crate::envelope::{BrokerEnvelope, ForwardingDispatcher, KernelDispatcher};
        use crate::kernel_ops::{KernelConfig, kernel_table};
        use d2b_contracts_broker::broker_wire::{
            BrokerCallerRole, BrokerRequest, EnvelopeInvokeRequest, FdKind,
        };
        use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
        use std::os::unix::fs::PermissionsExt;

        let root = test_audit_dir("process-kernels-envelope");
        fs::create_dir_all(&root).expect("create test root");
        let config = test_server_config(&root, &root.join("unused-bundle.json"));
        let (log, _capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");
        let kernels = kernel_table(KernelConfig {
            state_dir: config.state_dir.clone(),
            runtime_root: root.join("runtime"),
            daemon_uid: config.d2bd_uid,
            daemon_gid: config.d2bd_gid,
            bundle_path: config.bundle_path.clone(),
        });
        let envelope = BrokerEnvelope::over(
            crate::catalog::BrokerProfileId::Host,
            Box::new(KernelDispatcher::new(
                kernels,
                ForwardingDispatcher::default(),
            )),
        )
        .commit_forwarded()
        .build();
        let backend = FakeDispatchBackend {
            envelope,
            ..FakeDispatchBackend::default()
        };
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let caller_gid = Gid::current().as_raw();

        let invoke = |operation: &str,
                      payload: serde_json::Value,
                      request_fds: Vec<OwnedFd>,
                      chain: (Option<String>, Option<Vec<String>>)|
         -> Result<DispatchResult, BrokerError> {
            let request = BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
                operation: operation.to_owned(),
                zone: "work".to_owned(),
                payload,
                chain_root_invocation_id: chain.0,
                chain_identities: chain.1,
                fd_indexes: (0..request_fds.len() as u32).collect(),
                fd_kinds: vec![FdKind::Any; request_fds.len()],
            });
            let audit_context = DispatchAuditContext::from_request(&request, 4242, &caller_role)
                .expect("audit context");
            envelope_call_runtime().block_on(dispatch_request_with_backend_and_request_fds(
                request,
                1000,
                caller_gid,
                caller_role.clone(),
                &audit_context,
                &config,
                &log,
                None,
                &backend,
                request_fds,
            ))
        };
        let envelope_response = |result: DispatchResult| match result.response {
            BrokerResponse::EnvelopeInvoke(response) => response,
            other => panic!("expected an EnvelopeInvoke response, got {other:?}"),
        };
        let result_of = |response: &d2b_contracts_broker::broker_wire::EnvelopeInvokeResponse| {
            response.result.clone().expect("a dispatched kernel result")
        };

        // open-pidfd: the pidfd_open + start-time verification kernel; the
        // pidfd travels back over the fd leg.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep child");
        let pid = child.id() as i32;
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).expect("child stat");
        let start_time =
            crate::ops::pidfd::parse_proc_stat_start_time(&stat).expect("child start time");
        let response = envelope_response(
            invoke(
                "open-pidfd",
                serde_json::json!({ "pid": pid, "expectedStartTimeTicks": start_time }),
                Vec::new(),
                (None, None),
            )
            .expect("open-pidfd dispatches"),
        );
        assert_eq!(response.refusal, None);
        assert_eq!(
            result_of(&response)
                .get("pid")
                .and_then(serde_json::Value::as_i64),
            Some(pid as i64)
        );
        assert_eq!(
            result_of(&response)
                .get("verifiedStartTimeTicks")
                .and_then(serde_json::Value::as_u64),
            Some(start_time)
        );

        // signal-pidfd: signal 0 is the existence probe and SIGKILL the
        // terminal signal, each on a pidfd attached over the fd leg.
        let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
        let response = envelope_response(
            invoke(
                "signal-pidfd",
                serde_json::json!({ "signal": 0 }),
                vec![pidfd],
                (None, None),
            )
            .expect("signal-pidfd dispatches"),
        );
        assert_eq!(response.refusal, None);
        assert_eq!(
            result_of(&response)
                .get("signaled")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );
        let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
        let response = envelope_response(
            invoke(
                "signal-pidfd",
                serde_json::json!({ "signal": libc::SIGKILL }),
                vec![pidfd],
                (None, None),
            )
            .expect("signal-pidfd dispatches"),
        );
        assert_eq!(response.refusal, None);
        child.wait().expect("child reaped");

        // prepare-directory: the mkdir/chmod primitive over the already
        // resolved path. The owner is the caller's own principal so the
        // test needs no root; the audit result names the kind and the
        // replace-or-create outcome.
        let prepared = root.join("prepared");
        let response = envelope_response(
            invoke(
                "prepare-directory",
                serde_json::json!({
                    "kind": "state",
                    "baseDir": prepared.display().to_string(),
                    "vmIdOrScope": "acceptance-guest",
                    "mode": 0o700,
                    "ownerUid": nix::unistd::Uid::current().as_raw(),
                    "ownerGid": Gid::current().as_raw(),
                    "createdPaths": [],
                }),
                Vec::new(),
                (None, None),
            )
            .expect("prepare-directory dispatches"),
        );
        assert_eq!(response.refusal, None);
        assert_eq!(
            result_of(&response)
                .get("kind")
                .and_then(serde_json::Value::as_str),
            Some("state-dir")
        );
        assert_eq!(
            result_of(&response)
                .get("vm_id_or_scope")
                .and_then(serde_json::Value::as_str),
            Some("acceptance-guest")
        );
        assert_eq!(
            result_of(&response)
                .get("replace_or_create_result")
                .and_then(serde_json::Value::as_str),
            Some("created")
        );
        assert_eq!(
            fs::metadata(&prepared)
                .expect("prepared dir")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        // An unknown directory kind is refused by the kernel inside the
        // envelope response.
        let response = envelope_response(
            invoke(
                "prepare-directory",
                serde_json::json!({
                    "kind": "bogus",
                    "baseDir": root.join("x").display().to_string(),
                    "vmIdOrScope": "g",
                    "mode": 0o700,
                    "ownerUid": 0,
                    "ownerGid": 0,
                }),
                Vec::new(),
                (None, None),
            )
            .expect("unknown-kind prepare-directory dispatches"),
        );
        assert_eq!(
            response.refusal.as_deref(),
            Some(crate::envelope::HANDLER_REFUSED)
        );

        // open-peer-pidfd-from-accepted-socket: one accepted socket
        // descriptor in, the peer pidfd out over the fd leg.
        let (left, right) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("socketpair");
        let response = envelope_response(
            invoke(
                "open-peer-pidfd-from-accepted-socket",
                serde_json::json!({}),
                vec![left],
                (None, None),
            )
            .expect("peer-pidfd kernel dispatches"),
        );
        assert_eq!(response.refusal, None);
        assert_eq!(
            result_of(&response).as_object().map(|object| object.len()),
            Some(0)
        );
        assert_eq!(
            response.fd_indexes.len(),
            1,
            "the peer pidfd returns over the fd leg"
        );
        drop(right);

        // The accepted-socket fd facet stays exact after the cut: a missing
        // descriptor is refused by the kernel with the fd-leg code and an
        // oversized set is refused by the envelope's fd gate before
        // dispatch.
        let response = envelope_response(
            invoke(
                "open-peer-pidfd-from-accepted-socket",
                serde_json::json!({}),
                Vec::new(),
                (None, None),
            )
            .expect("missing-fd peer-pidfd dispatches"),
        );
        assert_eq!(response.refusal.as_deref(), Some(crate::envelope::FD_LEG));
        let (extra_a, extra_b) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("socketpair");
        let response = envelope_response(
            invoke(
                "open-peer-pidfd-from-accepted-socket",
                serde_json::json!({}),
                vec![extra_a, extra_b],
                (None, None),
            )
            .expect("oversized peer-pidfd dispatches"),
        );
        assert_eq!(response.refusal.as_deref(), Some(crate::envelope::FD_LEG));

        // The generic surface has no caller-supplied audit-join field: the
        // evidence chain is the join, and a partial chain is refused at the
        // EnvelopeInvoke arm before any dispatch.
        let error = invoke(
            "open-peer-pidfd-from-accepted-socket",
            serde_json::json!({}),
            Vec::new(),
            (Some("invocation-nested".to_owned()), None),
        )
        .expect_err("a partial chain must not dispatch");
        assert!(
            matches!(&error, BrokerError::RequestValidation { operation, reason }
                if *operation == "EnvelopeInvoke"
                    && *reason == "the evidence chain is partial"),
            "unexpected refusal: {error:?}"
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// The U12 network kernels the retired network-fds wire arms left
    /// behind (KTD10): each committed broker-generic network kernel row
    /// dispatches through the envelope under the row's own payload contract
    /// and fd facet. What was family knowledge in the retired arms -
    /// resolving a zone-native subject from the trusted bundle, deriving
    /// the per-VM dnsmasq lease identity, matching a bridge intent - stays
    /// on the family side after the cut, so the broker-side surface is
    /// exactly this kernel seam. The harness carries no bundle (the
    /// unused-bundle slot), so the resolver-dependent kernels are pinned
    /// on their fail-closed missing-intent refusal before any host effect,
    /// the pure-admission seed-dnsmasq-lease kernel is pinned on its full
    /// admit/refuse surface, and the field-contract kernels are pinned on
    /// their payload refusals.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn retired_network_family_kernels_dispatch_through_the_envelope() {
        use crate::envelope::{BrokerEnvelope, ForwardingDispatcher, KernelDispatcher};
        use crate::kernel_ops::{KernelConfig, kernel_table};
        use d2b_contracts_broker::broker_wire::{
            BrokerCallerRole, BrokerRequest, EnvelopeInvokeRequest, FdKind,
        };

        let root = test_audit_dir("network-kernels-envelope");
        fs::create_dir_all(&root).expect("create test root");
        let config = test_server_config(&root, &root.join("unused-bundle.json"));
        let (log, _capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");
        let kernels = kernel_table(KernelConfig {
            state_dir: config.state_dir.clone(),
            runtime_root: root.join("runtime"),
            daemon_uid: config.d2bd_uid,
            daemon_gid: config.d2bd_gid,
            bundle_path: config.bundle_path.clone(),
        });
        let envelope = BrokerEnvelope::over(
            crate::catalog::BrokerProfileId::Host,
            Box::new(KernelDispatcher::new(
                kernels,
                ForwardingDispatcher::default(),
            )),
        )
        .commit_forwarded()
        .build();
        let backend = FakeDispatchBackend {
            envelope,
            ..FakeDispatchBackend::default()
        };
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let caller_gid = Gid::current().as_raw();

        let invoke = |operation: &str,
                      payload: serde_json::Value,
                      request_fds: Vec<OwnedFd>,
                      chain: (Option<String>, Option<Vec<String>>)|
         -> Result<DispatchResult, BrokerError> {
            let request = BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
                operation: operation.to_owned(),
                zone: "work".to_owned(),
                payload,
                chain_root_invocation_id: chain.0,
                chain_identities: chain.1,
                fd_indexes: (0..request_fds.len() as u32).collect(),
                fd_kinds: vec![FdKind::Any; request_fds.len()],
            });
            let audit_context = DispatchAuditContext::from_request(&request, 4242, &caller_role)
                .expect("audit context");
            envelope_call_runtime().block_on(dispatch_request_with_backend_and_request_fds(
                request,
                1000,
                caller_gid,
                caller_role.clone(),
                &audit_context,
                &config,
                &log,
                None,
                &backend,
                request_fds,
            ))
        };
        let envelope_response = |result: DispatchResult| match result.response {
            BrokerResponse::EnvelopeInvoke(response) => response,
            other => panic!("expected an EnvelopeInvoke response, got {other:?}"),
        };
        let result_of = |response: &d2b_contracts_broker::broker_wire::EnvelopeInvokeResponse| {
            response.result.clone().expect("a dispatched kernel result")
        };

        let zone_uid =
            d2b_contracts_resource::v3::ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001")
                .expect("valid zone uid");
        let network_uid =
            d2b_contracts_resource::v3::ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002")
                .expect("valid network uid");
        let bundle_generation =
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

        // seed-dnsmasq-lease: the pure admission kernel. The per-VM lease
        // row is derived, never caller-supplied: the kernel re-derives the
        // expected child VM name from the admitted Network identity and
        // refuses a mismatch, so the full admit/refuse surface runs with no
        // host effect.
        let expected_vm = d2b_contracts_resource::v3::derive_network_child_name(&network_uid, "vm");
        let response = envelope_response(
            invoke(
                "seed-dnsmasq-lease",
                serde_json::json!({
                    "vmId": expected_vm.as_str(),
                    "scopeId": format!("network:{}:{}", zone_uid.as_str(), network_uid.as_str()),
                    "zoneUid": zone_uid.as_str(),
                    "networkUid": network_uid.as_str(),
                    "networkGeneration": 7,
                    "attachmentGeneration": 11,
                    "bundleGeneration": bundle_generation,
                }),
                Vec::new(),
                (None, None),
            )
            .expect("seed-dnsmasq-lease dispatches"),
        );
        assert_eq!(response.refusal, None);
        assert_eq!(
            result_of(&response)
                .get("seeded")
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        // A caller-supplied VM name that does not match the derived child
        // name is refused fail-closed.
        let response = envelope_response(
            invoke(
                "seed-dnsmasq-lease",
                serde_json::json!({
                    "vmId": "wrong-vm",
                    "scopeId": format!("network:{}:{}", zone_uid.as_str(), network_uid.as_str()),
                    "zoneUid": zone_uid.as_str(),
                    "networkUid": network_uid.as_str(),
                    "networkGeneration": 7,
                    "attachmentGeneration": 11,
                    "bundleGeneration": bundle_generation,
                }),
                Vec::new(),
                (None, None),
            )
            .expect("mismatched seed-dnsmasq-lease dispatches"),
        );
        assert_eq!(
            response.refusal.as_deref(),
            Some(crate::envelope::HANDLER_REFUSED)
        );
        assert!(
            response
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("network-admission-mismatch")),
            "the refusal names the admission mismatch: {:?}",
            response.detail
        );

        // apply-nftables-projection: an action outside the closed
        // apply/remove set is refused by the kernel before any exec.
        let response = envelope_response(
            invoke(
                "apply-nftables-projection",
                serde_json::json!({
                    "scriptBody": "table inet d2b {}",
                    "marker": "d2b managed: test",
                    "trustedHash": "fnv1a64:test",
                    "callerHash": serde_json::Value::Null,
                    "expectedGenerationId": bundle_generation,
                    "installedGenerationId": bundle_generation,
                    "action": "bogus",
                }),
                Vec::new(),
                (None, None),
            )
            .expect("bogus-action projection dispatches"),
        );
        assert_eq!(
            response.refusal.as_deref(),
            Some(crate::envelope::HANDLER_REFUSED)
        );
        assert!(
            response
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("unknown action")),
            "the refusal names the unknown action: {:?}",
            response.detail
        );

        // create-bridge: an ifname the IfName contract rejects is refused
        // before any netlink mutation.
        let response = envelope_response(
            invoke(
                "create-bridge",
                serde_json::json!({
                    "intentId": "bridge:test",
                    "scopeLabel": "work",
                    "bridgeIfname": "bad ifname",
                    "mtu": 1500,
                }),
                Vec::new(),
                (None, None),
            )
            .expect("invalid-ifname create-bridge dispatches"),
        );
        assert_eq!(
            response.refusal.as_deref(),
            Some(crate::envelope::HANDLER_REFUSED)
        );
        assert!(
            response
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("bridgeIfname")),
            "the refusal names the invalid ifname: {:?}",
            response.detail
        );

        // create-bridge: an ipv4Address the Ipv4Cidr contract rejects is
        // refused before any netlink mutation.
        let response = envelope_response(
            invoke(
                "create-bridge",
                serde_json::json!({
                    "intentId": "bridge:test",
                    "scopeLabel": "work",
                    "bridgeIfname": "br-test",
                    "mtu": 1500,
                    "ipv4Address": "192.0.2.1",
                }),
                Vec::new(),
                (None, None),
            )
            .expect("invalid-ipv4Address create-bridge dispatches"),
        );
        assert_eq!(
            response.refusal.as_deref(),
            Some(crate::envelope::HANDLER_REFUSED)
        );
        assert!(
            response
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("ipv4Address")),
            "the refusal names the invalid ipv4Address: {:?}",
            response.detail
        );

        // The field-contract kernels refuse a payload missing a required
        // field before any host effect: the envelope's payload gate answers
        // a payload missing a row-declared required field with the
        // invalid-payload code, and the kernel's own field gate answers a
        // payload that passes the row contract but misses a kernel field
        // with a handler refusal.
        for (operation, payload) in [
            ("apply-nftables", serde_json::json!({})),
            ("apply-nm-unmanaged", serde_json::json!({})),
            ("apply-route", serde_json::json!({})),
            ("apply-sysctl", serde_json::json!({})),
            ("delete-bridge", serde_json::json!({})),
            ("update-hosts-file", serde_json::json!({})),
        ] {
            let response = envelope_response(
                invoke(operation, payload, Vec::new(), (None, None))
                    .expect("missing-field kernel dispatches"),
            );
            assert!(
                response.refusal.is_some(),
                "{operation} must refuse a payload missing a required field: {response:?}"
            );
            if response.refusal.as_deref() == Some(crate::envelope::HANDLER_REFUSED) {
                assert!(
                    response
                        .detail
                        .as_deref()
                        .is_some_and(|detail| detail.contains("missing")),
                    "{operation} kernel refusal names the missing field: {:?}",
                    response.detail
                );
            }
        }

        // The typed-payload kernels: the whole payload is the typed wire
        // request, so a malformed payload is refused - by the envelope's
        // payload gate when it misses a row-declared required field, and by
        // the kernel's typed parse when it passes the row contract but is
        // not the typed request.
        for operation in [
            "create-persistent-tap",
            "delete-persistent-tap",
            "create-tap-fd",
            "set-bridge-port-flags",
        ] {
            let response = envelope_response(
                invoke(operation, serde_json::json!({}), Vec::new(), (None, None))
                    .expect("malformed typed kernel dispatches"),
            );
            assert!(
                response.refusal.is_some(),
                "{operation} must refuse a malformed typed payload: {response:?}"
            );
        }

        // create-tap-fd: the ONLY fd-bearing network kernel. The harness
        // carries no bundle, so the kernel refuses on the missing-intent
        // path (the resolver is unavailable) before it ever opens
        // /dev/net/tun; the typed request is otherwise fully validated.
        let response = envelope_response(
            invoke(
                "create-tap-fd",
                serde_json::json!({
                    "roleId": "network-attachment",
                    "vmId": "work-vm",
                    "bundleTapIntentRef": "tap:test",
                    "attachmentId": "123e4567-e89b-42d3-a456-426614174000",
                    "networkGeneration": 7,
                    "attachmentGeneration": 11,
                    "zoneUid": zone_uid.as_str(),
                    "networkUid": network_uid.as_str(),
                    "bundleGeneration": bundle_generation,
                    "admittedInterfaceNames": ["enp0s1"],
                }),
                Vec::new(),
                (None, None),
            )
            .expect("create-tap-fd dispatches"),
        );
        assert_eq!(
            response.refusal.as_deref(),
            Some(crate::envelope::HANDLER_REFUSED)
        );
        assert!(
            response
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("bundle resolver unavailable")),
            "create-tap-fd refuses on the missing-intent path: {:?}",
            response.detail
        );

        // The remaining resolver-dependent kernels refuse the same way:
        // the trusted intent is resolved from the broker's own bundle
        // copy, and with no bundle installed every one of them fails
        // closed before any host effect.
        for (operation, payload) in [
            (
                "create-persistent-tap",
                serde_json::json!({
                    "roleId": "network-attachment",
                    "vmId": "work-vm",
                    "bundleTapIntentRef": "tap:test",
                    "attachmentId": "123e4567-e89b-42d3-a456-426614174000",
                    "networkGeneration": 7,
                    "attachmentGeneration": 11,
                    "zoneUid": zone_uid.as_str(),
                    "networkUid": network_uid.as_str(),
                    "bundleGeneration": bundle_generation,
                    "admittedInterfaceNames": ["enp0s1"],
                }),
            ),
            (
                "delete-persistent-tap",
                serde_json::json!({
                    "attachmentId": "123e4567-e89b-42d3-a456-426614174000",
                    "expectedZoneUid": zone_uid.as_str(),
                    "expectedNetworkUid": network_uid.as_str(),
                    "expectedNetworkGeneration": 7,
                    "expectedAttachmentGeneration": 11,
                    "expectedBundleGeneration": bundle_generation,
                }),
            ),
            (
                "set-bridge-port-flags",
                serde_json::json!({
                    "vmId": "work-vm",
                    "roleId": "workload-lan",
                    "networkTapContext": serde_json::Value::Null,
                }),
            ),
        ] {
            let response = envelope_response(
                invoke(operation, payload, Vec::new(), (None, None))
                    .expect("resolver-dependent kernel dispatches"),
            );
            assert_eq!(
                response.refusal.as_deref(),
                Some(crate::envelope::HANDLER_REFUSED),
                "{operation} must refuse without a bundle: {response:?}"
            );
            assert!(
                response
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("bundle resolver unavailable")),
                "{operation} refusal names the missing bundle: {:?}",
                response.detail
            );
        }

        let _ = fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // consume-cell / complete-cell kernel tests (U11): the retired
    // ConsumeLifecycleLease arm's one-time cell semantics - AE2's
    // consume-once/replay-refusal, the concurrent exactly-one-winner
    // property, and the durable restart-replay resistance - restored on
    // the generic cell kernels through the envelope.
    // ------------------------------------------------------------------

    /// The lease-shaped payload one cell-kernel test invokes with. The
    /// full lease identity is the one-time key (KTD3): every field the
    /// retired arm keyed on, in the wire vocabulary's camelCase spelling.
    fn cell_kernel_payload(operation_id: &str) -> serde_json::Value {
        serde_json::json!({
            "zoneUid": "11111111-1111-4111-8111-111111111111",
            "guestUid": "22222222-2222-4222-8222-222222222222",
            "guestGeneration": 4,
            "providerAssignmentGeneration": 9,
            "policyRevision": 7,
            "operationId": operation_id,
            "operation": "start",
            "stopOnly": false,
        })
    }

    /// The envelope + dispatch harness one cell-kernel test drives: a real
    /// kernel table over the broker's cell store, exactly as the retired
    /// arm's helpers used it, with the invocation dispatched through the
    /// EnvelopeInvoke arm.
    struct CellKernelHarness {
        config: ServerConfig,
        log: AuditLog,
        backend: FakeDispatchBackend,
        caller_role: d2b_contracts_broker::broker_wire::BrokerCallerRole,
        caller_gid: u32,
    }

    impl CellKernelHarness {
        fn new(
            root: &Path,
            caller_role: d2b_contracts_broker::broker_wire::BrokerCallerRole,
        ) -> Self {
            use crate::envelope::{BrokerEnvelope, ForwardingDispatcher, KernelDispatcher};
            use crate::kernel_ops::{KernelConfig, kernel_table};

            let config = test_server_config(root, &root.join("unused-bundle.json"));
            let (log, _capture) = AuditLog::open_capturing(
                &config.audit_dir,
                Gid::current().as_raw(),
                true,
                config.audit_retention_days,
            )
            .expect("open capturing audit log");
            let kernels = kernel_table(KernelConfig {
                state_dir: config.state_dir.clone(),
                runtime_root: root.join("runtime"),
                daemon_uid: config.d2bd_uid,
                daemon_gid: config.d2bd_gid,
                bundle_path: config.bundle_path.clone(),
            });
            let envelope = BrokerEnvelope::over(
                crate::catalog::BrokerProfileId::Host,
                Box::new(KernelDispatcher::new(
                    kernels,
                    ForwardingDispatcher::default(),
                )),
            )
            .commit_forwarded()
            .build();
            let backend = FakeDispatchBackend {
                envelope,
                ..FakeDispatchBackend::default()
            };
            Self {
                config,
                log,
                backend,
                caller_role,
                caller_gid: Gid::current().as_raw(),
            }
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn invoke(
            &self,
            operation: &str,
            payload: serde_json::Value,
        ) -> Result<DispatchResult, BrokerError> {
            use d2b_contracts_broker::broker_wire::{BrokerRequest, EnvelopeInvokeRequest};
            let request = BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
                operation: operation.to_owned(),
                zone: "work".to_owned(),
                payload,
                chain_root_invocation_id: None,
                chain_identities: None,
                fd_indexes: Vec::new(),
                fd_kinds: Vec::new(),
            });
            let audit_context =
                DispatchAuditContext::from_request(&request, 4242, &self.caller_role)
                    .expect("audit context");
            envelope_call_runtime().block_on(dispatch_request_with_backend_and_request_fds(
                request,
                1000,
                self.caller_gid,
                self.caller_role.clone(),
                &audit_context,
                &self.config,
                &self.log,
                None,
                &self.backend,
                Vec::new(),
            ))
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn consume_cell_and_complete_cell_dispatch_through_the_envelope() {
        let root = test_audit_dir("cell-kernels-envelope");
        fs::create_dir_all(&root).expect("create test root");
        let harness = CellKernelHarness::new(
            &root,
            d2b_contracts_broker::broker_wire::BrokerCallerRole::AdminUid { uid: 1000 },
        );
        let envelope_response = |result: DispatchResult| match result.response {
            BrokerResponse::EnvelopeInvoke(response) => response,
            other => panic!("expected an EnvelopeInvoke response, got {other:?}"),
        };
        let operation_id = format!("cell-kernel-lease-{}", std::process::id());

        // consume-cell: the one-time claim wins exactly once (AE2).
        let response = envelope_response(
            harness
                .invoke("consume-cell", cell_kernel_payload(&operation_id))
                .expect("consume-cell dispatches"),
        );
        assert_eq!(response.refusal, None);
        assert_eq!(
            response
                .result
                .as_ref()
                .and_then(|result| result.get("consumed"))
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        // The same identity while the claim is live: in-progress refusal.
        let response = envelope_response(
            harness
                .invoke("consume-cell", cell_kernel_payload(&operation_id))
                .expect("in-progress consume-cell dispatches"),
        );
        assert_eq!(
            response.refusal.as_deref(),
            Some(crate::envelope::HANDLER_REFUSED)
        );
        assert_eq!(
            response.detail.as_deref(),
            Some("cell-in-progress"),
            "the documented cell code for a live claim"
        );

        // complete-cell: the completion phase records the durable marker.
        let response = envelope_response(
            harness
                .invoke("complete-cell", cell_kernel_payload(&operation_id))
                .expect("complete-cell dispatches"),
        );
        assert_eq!(response.refusal, None);
        assert_eq!(
            response
                .result
                .as_ref()
                .and_then(|result| result.get("completed"))
                .and_then(serde_json::Value::as_bool),
            Some(true)
        );

        // Completed one-time cell: the same identity replays the recorded
        // outcome - a refusal (AE2).
        let response = envelope_response(
            harness
                .invoke("consume-cell", cell_kernel_payload(&operation_id))
                .expect("replayed consume-cell dispatches"),
        );
        assert_eq!(
            response.refusal.as_deref(),
            Some(crate::envelope::HANDLER_REFUSED)
        );
        assert_eq!(
            response.detail.as_deref(),
            Some("cell-replayed"),
            "the documented cell code for a completed replay"
        );

        // A genuinely new invocation (fresh operation id) is a fresh grant:
        // the lease stays consumable per operation.
        let next = format!("cell-kernel-lease-next-{}", std::process::id());
        let response = envelope_response(
            harness
                .invoke("consume-cell", cell_kernel_payload(&next))
                .expect("fresh consume-cell dispatches"),
        );
        assert_eq!(response.refusal, None);
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn consume_cell_concurrent_callers_have_exactly_one_winner() {
        // AE2's concurrency property through the kernel: N callers race the
        // same one-time identity; the cell store's single-process lock
        // serializes the compare-and-consume, so exactly one caller wins
        // and every other is refused with a documented cell code.
        let root = test_audit_dir("cell-kernels-concurrent");
        fs::create_dir_all(&root).expect("create test root");
        let harness = Arc::new(CellKernelHarness::new(
            &root,
            d2b_contracts_broker::broker_wire::BrokerCallerRole::AdminUid { uid: 1000 },
        ));
        let operation_id = format!("cell-kernel-race-{}", std::process::id());
        let callers: Vec<_> = (0..8)
            .map(|_| {
                let harness = Arc::clone(&harness);
                let operation_id = operation_id.clone();
                std::thread::spawn(move || {
                    harness
                        .invoke("consume-cell", cell_kernel_payload(&operation_id))
                        .expect("consume-cell dispatches")
                })
            })
            .collect();
        let mut winners = 0_usize;
        let mut refusals = 0_usize;
        for caller in callers {
            let result = caller.join().expect("caller thread");
            match result.response {
                BrokerResponse::EnvelopeInvoke(response) => {
                    if response.refusal.is_none() {
                        assert_eq!(
                            response
                                .result
                                .as_ref()
                                .and_then(|result| result.get("consumed"))
                                .and_then(serde_json::Value::as_bool),
                            Some(true)
                        );
                        winners += 1;
                    } else {
                        assert_eq!(
                            response.refusal.as_deref(),
                            Some(crate::envelope::HANDLER_REFUSED)
                        );
                        assert!(
                            matches!(
                                response.detail.as_deref(),
                                Some("cell-in-progress" | "cell-replayed")
                            ),
                            "a loser is refused with a documented cell code: {:?}",
                            response.detail
                        );
                        refusals += 1;
                    }
                }
                other => panic!("expected an EnvelopeInvoke response, got {other:?}"),
            }
        }
        assert_eq!(
            winners, 1,
            "exactly one concurrent caller consumes the one-time cell"
        );
        assert_eq!(refusals, 7, "every other caller is refused");
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn consume_cell_durable_records_refuse_replay_after_store_reopen() {
        // Restart-replay resistance (AE2) over the kernel's durable leg:
        // `init_broker_store` opens the store under the daemon state root,
        // so a broker restart reopens that root. The completed one-time
        // marker must survive the reopen and still refuse the replayed
        // consume, and a different principal's replay must refuse too
        // (KTD3).
        let root = tempfile::tempdir().expect("cell store root");
        let store = crate::state_cells::CellStore::open(root.path()).expect("open store");
        let payload = cell_kernel_payload(&format!("cell-kernel-restart-{}", std::process::id()));
        let canonical: d2b_contracts_resource::v3::CanonicalJsonObject =
            serde_json::from_value(payload).expect("canonical payload");
        let identity =
            crate::kernel_ops::cell_identity(&canonical).expect("canonical cell identity");
        let principal = "admin";
        assert_eq!(
            store
                .consume(
                    "lifecycle-leases-v2",
                    &identity,
                    principal,
                    crate::catalog::CellDurability::OneTime
                )
                .expect("consume"),
            crate::state_cells::ConsumeDecision::Granted
        );
        store
            .complete("lifecycle-leases-v2", &identity, principal)
            .expect("complete");
        drop(store);

        // The broker restarts: the store reopens the same root.
        let reopened = crate::state_cells::CellStore::open(root.path()).expect("reopen store");
        assert_eq!(
            reopened
                .consume(
                    "lifecycle-leases-v2",
                    &identity,
                    principal,
                    crate::catalog::CellDurability::OneTime
                )
                .expect("replayed consume"),
            crate::state_cells::ConsumeDecision::Replayed,
            "the completed marker survives the restart and refuses the replay"
        );
        // A different principal's replay of the same identity refuses too
        // (KTD3): invocation ids alone never gate a one-time grant.
        assert_eq!(
            reopened
                .consume(
                    "lifecycle-leases-v2",
                    &identity,
                    "daemon",
                    crate::catalog::CellDurability::OneTime
                )
                .expect("foreign replay"),
            crate::state_cells::ConsumeDecision::ForeignPrincipal
        );
    }

    // ------------------------------------------------------------------
    // spawn-process kernel tests (U10 P1): the retired SpawnRunner arm's
    // in-broker behaviors - the USBIP backend device-bind extension, the
    // serving-worker ACL grant, the stale-socket preflight cleanups and
    // the duplicate-runner guard with the runner-id-keyed registration -
    // restored on the spawn-process kernel path.
    // ------------------------------------------------------------------

    /// The envelope + dispatch harness one spawn-process kernel test
    /// drives.
    struct SpawnKernelHarness {
        config: ServerConfig,
        log: AuditLog,
        backend: FakeDispatchBackend,
        caller_role: d2b_contracts_broker::broker_wire::BrokerCallerRole,
        caller_gid: u32,
    }

    impl SpawnKernelHarness {
        fn new(root: &Path, bundle_path: &Path, runtime_root: &Path) -> Self {
            use crate::envelope::{BrokerEnvelope, ForwardingDispatcher, KernelDispatcher};
            use crate::kernel_ops::{KernelConfig, kernel_table};

            let config = test_server_config(root, &root.join("unused-bundle.json"));
            let (log, _capture) = AuditLog::open_capturing(
                &config.audit_dir,
                Gid::current().as_raw(),
                true,
                config.audit_retention_days,
            )
            .expect("open capturing audit log");
            let kernels = kernel_table(KernelConfig {
                state_dir: config.state_dir.clone(),
                runtime_root: runtime_root.to_path_buf(),
                daemon_uid: config.d2bd_uid,
                daemon_gid: config.d2bd_gid,
                bundle_path: bundle_path.to_path_buf(),
            });
            let envelope = BrokerEnvelope::over(
                crate::catalog::BrokerProfileId::Host,
                Box::new(KernelDispatcher::new(
                    kernels,
                    ForwardingDispatcher::default(),
                )),
            )
            .commit_forwarded()
            .build();
            let backend = FakeDispatchBackend {
                envelope,
                ..FakeDispatchBackend::default()
            };
            Self {
                config,
                log,
                backend,
                caller_role: d2b_contracts_broker::broker_wire::BrokerCallerRole::AdminUid {
                    uid: 1000,
                },
                caller_gid: Gid::current().as_raw(),
            }
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn invoke(
            &self,
            operation: &str,
            payload: serde_json::Value,
            request_fds: Vec<OwnedFd>,
        ) -> Result<DispatchResult, BrokerError> {
            use d2b_contracts_broker::broker_wire::{BrokerRequest, EnvelopeInvokeRequest, FdKind};

            let request = BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
                operation: operation.to_owned(),
                zone: "work".to_owned(),
                payload,
                chain_root_invocation_id: None,
                chain_identities: None,
                fd_indexes: (0..request_fds.len() as u32).collect(),
                fd_kinds: vec![FdKind::Any; request_fds.len()],
            });
            let audit_context =
                DispatchAuditContext::from_request(&request, 4242, &self.caller_role)
                    .expect("audit context");
            envelope_call_runtime().block_on(dispatch_request_with_backend_and_request_fds(
                request,
                1000,
                self.caller_gid,
                self.caller_role.clone(),
                &audit_context,
                &self.config,
                &self.log,
                None,
                &self.backend,
                request_fds,
            ))
        }
    }

    fn envelope_response(
        result: DispatchResult,
    ) -> d2b_contracts_broker::broker_wire::EnvelopeInvokeResponse {
        match result.response {
            BrokerResponse::EnvelopeInvoke(response) => response,
            other => panic!("expected an EnvelopeInvoke response, got {other:?}"),
        }
    }

    /// The absolute path of one test binary (NixOS keeps no `/bin`).
    fn spawn_test_binary(name: &str) -> String {
        for candidate in [
            format!("/run/current-system/sw/bin/{name}"),
            format!("/usr/bin/{name}"),
            format!("/bin/{name}"),
        ] {
            if Path::new(&candidate).exists() {
                return candidate;
            }
        }
        panic!("no {name} binary found for the spawn kernel test");
    }

    /// The minimal unprivileged spawn-process payload one kernel test
    /// drives: no namespaces, no cgroup leaf, no device binds, and the
    /// current principal (a real clone3 spawn succeeds without broker
    /// credentials only in that shape).
    fn spawn_payload(
        argv: Vec<String>,
        role: &str,
        serving_worker: bool,
        vm_id: &str,
        role_id: &str,
        bundle_runner_intent_ref: &str,
    ) -> serde_json::Value {
        spawn_payload_with_preflight(argv, role, serving_worker, vm_id, role_id, bundle_runner_intent_ref, Vec::new())
    }

    /// The same minimal spawn payload plus a user namespace mapping the
    /// child's in-ns root to the current principal. A plain (no-namespace)
    /// child that runs as an unprivileged principal calls `setgroups` and
    /// fails with EPERM before `execve`; inside a user namespace the
    /// broker skips that step, so the kernel-spawned child is genuinely
    /// alive and the test can exercise liveness against a running runner.
    fn spawn_payload_with_user_namespace(
        argv: Vec<String>,
        role: &str,
        serving_worker: bool,
        vm_id: &str,
        role_id: &str,
        bundle_runner_intent_ref: &str,
    ) -> serde_json::Value {
        let mut payload = spawn_payload_with_preflight(
            argv,
            role,
            serving_worker,
            vm_id,
            role_id,
            bundle_runner_intent_ref,
            Vec::new(),
        );
        payload["namespaces"]["user"] = serde_json::json!(true);
        payload["userNamespace"] = serde_json::json!({
            "hostUidForZero": nix::unistd::Uid::current().as_raw(),
            "hostGidForZero": Gid::current().as_raw(),
        });
        payload
    }

    fn spawn_payload_with_preflight(
        argv: Vec<String>,
        role: &str,
        serving_worker: bool,
        vm_id: &str,
        role_id: &str,
        bundle_runner_intent_ref: &str,
        preflight_socket_paths: Vec<String>,
    ) -> serde_json::Value {
        serde_json::json!({
            "binaryPath": argv[0],
            "argv": argv,
            "preflightSocketPaths": preflight_socket_paths,
            "uid": nix::unistd::Uid::current().as_raw(),
            "gid": Gid::current().as_raw(),
            "supplementaryGroups": [],
            "env": [],
            "capabilities": [],
            "namespaces": {
                "mount": false, "pid": false, "net": false, "ipc": false, "uts": false, "user": false,
            },
            "mountPolicy": {
                "readOnlyPaths": [],
                "writablePaths": [],
                "nixStoreReadOnly": false,
                "hideDeviceNodesByDefault": false,
                "deviceBinds": [],
            },
            "cgroupPlacement": { "subtree": "", "controllers": [], "delegated": false },
            "rootCarveOut": false,
            "skipBinaryExistsCheck": false,
            "role": role,
            "servingWorker": serving_worker,
            "runnerIdentity": {
                "vmId": vm_id,
                "roleId": role_id,
                "resourceRef": null,
                "resourceUid": null,
                "zoneUid": null,
                "generation": null,
                "runtimeScope": null,
                "ownerRef": null,
                "providerRef": null,
                "providerIdentity": null,
                "templateIdentity": null,
                "bundleRunnerIntentRef": bundle_runner_intent_ref,
                "guestExecution": null,
            },
        })
    }

    /// Drop one kernel-spawned runner's runner-id-keyed registrations and
    /// drain the reap buffer so no state leaks into a sibling test.
    fn cleanup_spawn_test_runner(runner_id: &str) {
        runner_pidfds().remove(runner_id);
        remove_runner_metadata(runner_id);
        let _ = drain_child_reap_buffer();
    }

    /// Serializes every test that mutates the process-global broker
    /// registries (runner metadata, pidfds, reap buffer, controller
    /// bootstrap). These are production statics shared across the binary,
    /// so tests that write them must never run concurrently: one test's
    /// registration would be clobbered by another test's cleanup. The
    /// guard also clears the registries on entry and exit, so each test
    /// sees a clean, isolated snapshot and leaves none behind.
    struct RegistryTestGuard {
        _lock: MutexGuard<'static, ()>,
    }

    impl RegistryTestGuard {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn new() -> Self {
            static LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
            let lock = LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            clear_registry_state();
            Self { _lock: lock }
        }
    }

    impl Drop for RegistryTestGuard {
        fn drop(&mut self) {
            clear_registry_state();
        }
    }

    /// Clear every process-global broker registry so no state leaks
    /// between tests. Non-blocking try-lock (plan U8) - a Busy collision
    /// is retried by the next guard acquisition.
    fn clear_registry_state() {
        let _ = drain_child_reap_buffer();
        runner_pidfds().clear();
        if let Ok(mut registry) = runner_metadata_registry().try_lock() {
            registry.clear();
        }
        if let Ok(mut registry) = controller_bootstrap_registry().try_lock() {
            registry.clear();
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn spawn_process_usbip_backend_extends_mount_policy_with_device_binds() {
        let _usb_sysfs_guard = usb_sysfs_test_lock();
        let _registry_guard = RegistryTestGuard::new();
        let root = test_audit_dir("spawn-kernel-usbip-binds");
        fs::create_dir_all(&root).expect("create test root");
        let bundle = build_test_bundle(&root);
        TEST_KERNEL_BUNDLE_RESOLVER
            .set(bundle.resolver.clone())
            .unwrap_or_else(|_| {
                assert!(Arc::ptr_eq(
                    TEST_KERNEL_BUNDLE_RESOLVER.get().expect("set once"),
                    &bundle.resolver
                ))
            });
        // The fake USB sysfs device the bundle's locked busid names
        // (vendor 1050, product 0407, bus 1, dev 7): the derived device
        // node is /dev/bus/usb/001/007.
        let sysfs_root = prepare_test_usb_sysfs_device("1050", "0407", "2.3");
        // Redirect the busid lock probe to a scratch root and seed the
        // durable claim for the bundle's locked VM.
        let lock_root = root.join("locks");
        fs::create_dir_all(&lock_root).expect("create lock root");
        TEST_USBIP_LOCK_ROOT
            .set(lock_root.clone())
            .unwrap_or_else(|_| assert_eq!(TEST_USBIP_LOCK_ROOT.get(), Some(&lock_root)));
        crate::ops::usbip_lock::acquire_lock(
            &lock_root.join("1-2.3"),
            "corp-vm",
            nix::unistd::Uid::current().as_raw(),
            Gid::current().as_raw(),
        )
        .expect("seed the busid lock");

        let harness = SpawnKernelHarness::new(&root, &bundle.bundle_path, &root.join("runtime"));
        let response = envelope_response(
            harness
                .invoke(
                    "spawn-process",
                    spawn_payload(
                        vec![spawn_test_binary("true")],
                        "usbip",
                        false,
                        "sys-work-usbipd",
                        "backend",
                        "runner:sys-work-usbipd:backend",
                    ),
                    Vec::new(),
                )
                .expect("usbip backend spawn dispatches"),
        );
        assert_eq!(
            response.refusal, None,
            "spawn refused: {:?}",
            response.detail
        );
        let result = response.result.clone().expect("a dispatched kernel result");
        assert_eq!(
            result
                .get("deviceBinds")
                .and_then(serde_json::Value::as_array)
                .map(|binds| binds
                    .iter()
                    .filter_map(|value| value.as_str())
                    .collect::<Vec<_>>()),
            Some(vec!["/dev/bus/usb/001/007"]),
            "the kernel must extend the mount policy with the locked device node"
        );
        // The runner-id-keyed registration the broker's removal paths key on.
        let runner_id = "sys-work-usbipd:backend";
        assert!(
            runner_pidfds().contains_key(runner_id),
            "the kernel must register the pidfd under the runner id"
        );
        // The registry is a `tokio::sync::Mutex` (plan U8); concurrent
        // spawn-kernel tests hold it for sub-microsecond critical sections,
        // so the assertion retries the non-blocking try-lock instead of
        // panicking on a Busy collision.
        let registered = loop {
            if let Ok(registry) = runner_metadata_registry().try_lock() {
                break registry.contains_key(runner_id);
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        assert!(
            registered,
            "the kernel must register the runner metadata under the runner id"
        );
        cleanup_spawn_test_runner(runner_id);
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&sysfs_root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn spawn_process_serving_worker_grants_ticket_tree_acls() {
        let _registry_guard = RegistryTestGuard::new();
        use std::os::unix::fs::PermissionsExt;

        if ![
            "/run/current-system/sw/bin/setfacl",
            "/usr/bin/setfacl",
            "/bin/setfacl",
        ]
        .iter()
        .any(|candidate| Path::new(candidate).exists())
        {
            eprintln!("skipping serving-worker ACL kernel test: no setfacl binary");
            return;
        }
        let root = test_audit_dir("spawn-kernel-serving-acl");
        fs::create_dir_all(&root).expect("create test root");
        let runtime_root = root.join("run");
        let socket_dir = runtime_root.join("vms").join("guest");
        fs::create_dir_all(&socket_dir).expect("create socket dir");
        fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700))
            .expect("chmod runtime root");
        fs::set_permissions(&socket_dir, fs::Permissions::from_mode(0o700))
            .expect("chmod socket dir");
        let shared = root.join("view");
        fs::create_dir_all(&shared).expect("create view root");
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o750)).expect("chmod view root");

        let harness =
            SpawnKernelHarness::new(&root, &root.join("unused-bundle.json"), &runtime_root);
        let response = envelope_response(
            harness
                .invoke(
                    "spawn-process",
                    spawn_payload(
                        vec![
                            spawn_test_binary("true"),
                            format!(
                                "--socket-path={}",
                                socket_dir.join("vol-abcd.vfd.sock").display()
                            ),
                            format!("--shared-dir={}", shared.display()),
                        ],
                        "provider-controller",
                        true,
                        "vm-a",
                        "virtiofsd-worker",
                        "runner:vm-a:virtiofsd-worker",
                    ),
                    Vec::new(),
                )
                .expect("serving worker spawn dispatches"),
        );
        assert_eq!(
            response.refusal, None,
            "spawn refused: {:?}",
            response.detail
        );
        let socket_fd =
            crate::sys::path_safe::open_dir_path_safe(&socket_dir).expect("open socket dir");
        assert_eq!(
            crate::sys::path_safe::fd_extended_acl_present(socket_fd.as_fd())
                .expect("inspect socket dir ACL"),
            (true, false),
            "the serving worker's private socket directory must carry the runner ACL"
        );
        let view_fd = crate::sys::path_safe::open_dir_path_safe(&shared).expect("open view root");
        assert_eq!(
            crate::sys::path_safe::fd_extended_acl_present(view_fd.as_fd())
                .expect("inspect view root ACL"),
            (true, false),
            "the served view root must carry the runner ACL"
        );
        cleanup_spawn_test_runner("vm-a:virtiofsd-worker");
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn spawn_process_cloud_hypervisor_unlinks_stale_socket_before_spawn() {
        let _registry_guard = RegistryTestGuard::new();
        let root = test_audit_dir("spawn-kernel-stale-socket");
        fs::create_dir_all(&root).expect("create test root");
        // The sandbox execroot can push even a relative bind path past
        // SUN_LEN (108 bytes); the stale socket lives on the short temp
        // base so the bind itself succeeds before the unlink is proven.
        let mut stale = std::env::temp_dir();
        stale.push(format!("d2b-stale-{}.sock", std::process::id()));
        let _ = fs::remove_file(&stale);
        let listener = std::os::unix::net::UnixListener::bind(&stale).expect("bind stale socket");
        drop(listener);
        assert!(
            stale.exists(),
            "the dropped listener leaves a stale socket file"
        );

        let harness = SpawnKernelHarness::new(
            &root,
            &root.join("unused-bundle.json"),
            &root.join("runtime"),
        );
        let response = envelope_response(
            harness
                .invoke(
                    "spawn-process",
                    spawn_payload_with_preflight(
                        vec![
                            spawn_test_binary("true"),
                            "--api-socket".to_owned(),
                            stale.display().to_string(),
                        ],
                        "cloud-hypervisor",
                        false,
                        "vm-stale",
                        "ch-runner",
                        "runner:vm-stale:ch-runner",
                        vec![stale.display().to_string()],
                    ),
                    Vec::new(),
                )
                .expect("cloud-hypervisor spawn dispatches"),
        );
        assert_eq!(
            response.refusal, None,
            "spawn refused: {:?}",
            response.detail
        );
        assert!(
            !stale.exists(),
            "the stale socket must be unlinked before the spawn proceeds"
        );
        let result = response.result.clone().expect("a dispatched kernel result");
        assert!(
            result
                .get("pid")
                .and_then(serde_json::Value::as_i64)
                .is_some(),
            "the spawn must proceed after the stale socket cleanup"
        );
        cleanup_spawn_test_runner("vm-stale:ch-runner");
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn spawn_process_refuses_a_second_live_spawn_for_the_same_runner() {
        let _registry_guard = RegistryTestGuard::new();
        let root = test_audit_dir("spawn-kernel-duplicate-guard");
        fs::create_dir_all(&root).expect("create test root");
        let harness = SpawnKernelHarness::new(
            &root,
            &root.join("unused-bundle.json"),
            &root.join("runtime"),
        );
        // The first child must be GENUINELY alive when the second spawn is
        // checked, because the contract under test is that a duplicate of a
        // live registration is refused. A plain (no-namespace) kernel spawn
        // running as an unprivileged principal dies in `setgroups` before
        // exec (CHILD_EXIT_SETGROUPS) and would funnel into the stale-
        // registration reclaim path instead, so the fixture spawns it in a
        // user namespace where the broker skips that step.
        let first = envelope_response(
            harness
                .invoke(
                    "spawn-process",
                    spawn_payload_with_user_namespace(
                        vec![spawn_test_binary("sleep"), "30".to_owned()],
                        "cloud-hypervisor",
                        false,
                        "vm-a",
                        "ch-runner",
                        "runner:vm-a:ch-runner",
                    ),
                    Vec::new(),
                )
                .expect("first spawn dispatches"),
        );
        assert_eq!(
            first.refusal, None,
            "first spawn refused: {:?}",
            first.detail
        );
        let pid = first
            .result
            .clone()
            .expect("a dispatched kernel result")
            .get("pid")
            .and_then(serde_json::Value::as_i64)
            .expect("pid") as i32;

        // A second live spawn for the same runner id is refused BEFORE a
        // child is created, with the retired arm's documented message.
        let second = envelope_response(
            harness
                .invoke(
                    "spawn-process",
                    spawn_payload(
                        vec![spawn_test_binary("true")],
                        "cloud-hypervisor",
                        false,
                        "vm-a",
                        "ch-runner",
                        "runner:vm-a:ch-runner",
                    ),
                    Vec::new(),
                )
                .expect("second spawn dispatches"),
        );
        assert_eq!(
            second.refusal.as_deref(),
            Some(crate::envelope::HANDLER_REFUSED),
            "a duplicate live spawn must be refused: {:?}",
            second.detail
        );
        assert!(
            second.detail.as_deref().is_some_and(|detail| {
                detail.contains(
                    "runner vm-a:ch-runner already has an active registration; \
                     refusing duplicate spawn",
                )
            }),
            "the refusal must carry the retired arm's documented message: {:?}",
            second.detail
        );

        // Cleanup: kill the first child and drop its registrations. A
        // sibling test's `waitpid(-1)` may have already reaped the child
        // (ESRCH), which is fine - the guard is registration-keyed, not
        // liveness-keyed.
        let _ = nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        );
        let _ = nix::sys::wait::waitpid(nix::unistd::Pid::from_raw(pid), None);
        cleanup_spawn_test_runner("vm-a:ch-runner");
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn publish_trusted_context_updates_the_store_and_acks_the_epoch() {
        use d2b_contracts_broker::broker_wire::{
            BrokerCallerRole, BrokerRequest, PublishTrustedContextResponse,
            PublishTrustedContextValues,
        };

        let root = test_audit_dir("publish-trusted-context");
        crate::envelope::init_trusted_context_store(&root).expect("init the trusted context store");
        let bundle = build_test_bundle(&root);
        let config = test_server_config(&root, &bundle.manifest_path);
        let (log, capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");
        let backend = FakeDispatchBackend::default();
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let values = PublishTrustedContextValues {
            zone: "test".to_owned(),
            provider_set_revision: 3,
            controller_generation: 4,
            guest_generation: 2,
        };
        let audit_context = DispatchAuditContext::from_request(
            &BrokerRequest::PublishTrustedContext(values.clone()),
            4242,
            &caller_role,
        )
        .expect("audit context");

        let result = envelope_call_runtime().block_on(dispatch_request_with_backend(
            BrokerRequest::PublishTrustedContext(values.clone()),
            1000,
            Gid::current().as_raw(),
            caller_role.clone(),
            &audit_context,
            &config,
            &log,
            Some(&bundle.resolver),
            &backend,
        ))
        .expect("the publication dispatches");
        assert!(
            result.fds.is_empty(),
            "the publication carries no descriptors"
        );
        match result.response {
            BrokerResponse::PublishTrustedContext(PublishTrustedContextResponse {
                broker_epoch,
            }) => {
                assert_eq!(
                    broker_epoch, 1,
                    "a fresh store acknowledges its first epoch nonce"
                );
            }
            other => panic!("expected a PublishTrustedContext acknowledgement, got {other:?}"),
        }

        // The cache is durable and monotonic: a publication that would move a
        // cached value backwards is refused with the stale-context code, and
        // the acknowledgement the daemon advances on only ever carries an
        // epoch that store state accepted.
        let mut backward = values.clone();
        backward.provider_set_revision = 1;
        let audit_context = DispatchAuditContext::from_request(
            &BrokerRequest::PublishTrustedContext(backward.clone()),
            4242,
            &caller_role,
        )
        .expect("audit context");
        let error = envelope_call_runtime().block_on(dispatch_request_with_backend(
            BrokerRequest::PublishTrustedContext(backward),
            1000,
            Gid::current().as_raw(),
            caller_role.clone(),
            &audit_context,
            &config,
            &log,
            Some(&bundle.resolver),
            &backend,
        ))
        .expect_err("a backwards publication is refused");
        assert!(
            matches!(error, BrokerError::LiveHandler(ref detail) if detail.contains("stale-context")),
            "the refusal names the stale-context code, got {error:?}"
        );

        // The acknowledged publication is audited with the published values.
        let records = capture.lock().expect("capture lock after dispatch");
        let record = records.last().expect("one publication audit record");
        assert_eq!(record.operation, "PublishTrustedContext");
        let fields = OperationFields::from_operation_value(
            "PublishTrustedContext",
            record
                .operation_fields
                .clone()
                .expect("operation fields present"),
        )
        .expect("deserialize operation fields");
        assert_eq!(
            fields,
            OperationFields::PublishTrustedContext {
                zone: "test".to_owned(),
                provider_set_revision: 3,
                controller_generation: 4,
                guest_generation: 2,
            },
            "the audit record carries the published zone and values"
        );
        drop(records);

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn the_ownership_matrix_preflight_caller_answers_through_the_envelope_end_to_end() {
        use crate::envelope::{BrokerEnvelope, ForwardingDispatcher};
        use crate::forwarding::SocketForwarder;
        use crate::protocol::{bind_seqpacket, recv_json_frame, send_json_frame};
        use d2b_contracts::types::VmId;
        use d2b_contracts_broker::broker_wire::{
            BrokerCallerRole, BrokerRequest, ForwardOperationOutcome, ForwardOperationRequest,
            ForwardOperationResponse, OwnershipMatrixCheckRequest,
        };
        use nix::sys::socket::{SockFlag, accept4};
        use std::os::fd::AsRawFd;

        // U5 happy path: the migrated caller dispatches the typed wire
        // request and the serving arm routes it through the envelope under
        // `CallerAuthority::Daemon`; the dispatch step crosses the forward
        // carrier to the declaring process's leg, whose answer travels back
        // as the wire Ack. The peer is the daemon's rendezvous-shaped half
        // (a bound forward socket accepted before the dispatch starts) and
        // answers only from bytes it read off the accepted connection, so
        // the operations/zone/payload assertions are grounded in what
        // actually crossed the socket.
        let root = test_audit_dir("ownership-prep-envelope");
        fs::create_dir_all(&root).expect("create audit test dir");
        let config = test_server_config(&root, &root.join("unused-bundle.json"));
        let (log, _capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");

        let socket_root = tempfile::tempdir().expect("forward socket dir");
        let forward_path = socket_root.path().join("d2bd-forward.sock");
        let listener = bind_seqpacket(&forward_path).expect("bind the test forwarding peer");
        let peer = std::thread::spawn(move || {
            let fd = accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC)
                .expect("the broker's forwarder dials the peer");
            let request = recv_json_frame::<ForwardOperationRequest>(fd.as_raw_fd())
                .expect("read the forwarded invocation")
                .expect("the broker's forwarder sent a frame");
            assert_eq!(request.operation, "OwnershipMatrixCheck");
            assert_eq!(
                request.zone, "vm-1",
                "the wire carries no zone; the call's zone is the vm id (the wire's audit-join axis)"
            );
            assert_eq!(
                request.payload,
                serde_json::json!({ "vm": "vm-1" }),
                "the forwarded payload is the canonical preflight payload"
            );
            send_json_frame(
                fd.as_raw_fd(),
                &ForwardOperationResponse {
                    outcome: ForwardOperationOutcome::Result {
                        result: serde_json::json!({ "clean": true }),
                        fd_indexes: vec![],
                        fd_kinds: vec![],
                    },
                },
            )
            .expect("write the forwarded reply");
        });

        // The same envelope shape the production broker builds
        // (`live_operation_envelope`): every committed forwarded row served
        // through the carrier, with the row's committed grants deciding the
        // caller. Built in the test rather than through the process static
        // so the two scenarios below stay deterministic regardless of test
        // order.
        let envelope = BrokerEnvelope::over(
            crate::catalog::BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(SocketForwarder::new(
                forward_path,
            ))),
        )
        .commit_forwarded()
        .build();
        let backend = FakeDispatchBackend {
            envelope,
            ..FakeDispatchBackend::default()
        };
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let request = BrokerRequest::OwnershipMatrixCheck(OwnershipMatrixCheckRequest {
            vm_id: VmId::new("vm-1"),
            tracing_span_id: None,
        });
        let audit_context = DispatchAuditContext::from_request(&request, 4242, &caller_role)
            .expect("audit context");
        let result = envelope_call_runtime().block_on(dispatch_request_with_backend(
            request,
            1000,
            Gid::current().as_raw(),
            caller_role,
            &audit_context,
            &config,
            &log,
            None,
            &backend,
        ))
        .expect("the preflight dispatches through the envelope");
        match result.response {
            BrokerResponse::Ack(ack) => {
                assert!(ack.accepted);
                assert_eq!(ack.operation, "OwnershipMatrixCheck");
            }
            other => panic!("expected an OwnershipMatrixCheck ack, got {other:?}"),
        }
        peer.join().expect("the forwarding peer completes");
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn an_absent_forwarding_peer_refuses_the_preflight_with_the_unregistered_handler_code() {
        use crate::envelope::{BrokerEnvelope, ForwardingDispatcher};
        use d2b_contracts::types::VmId;
        use d2b_contracts_broker::broker_wire::{
            BrokerCallerRole, BrokerRequest, OwnershipMatrixCheckRequest,
        };

        // U5 error path (the prebind gap): before the daemon binds its
        // forwarding leg, the envelope holds no serving peer and refuses the
        // forwarded operation with `unregistered-handler` - the documented
        // fail-closed code, never a silent queue or a synthesized success.
        let root = test_audit_dir("ownership-prep-prebind");
        fs::create_dir_all(&root).expect("create audit test dir");
        let config = test_server_config(&root, &root.join("unused-bundle.json"));
        let (log, _capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");
        let envelope = BrokerEnvelope::over(
            crate::catalog::BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::default()),
        )
        .commit_forwarded()
        .build();
        let backend = FakeDispatchBackend {
            envelope,
            ..FakeDispatchBackend::default()
        };
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let request = BrokerRequest::OwnershipMatrixCheck(OwnershipMatrixCheckRequest {
            vm_id: VmId::new("vm-1"),
            tracing_span_id: None,
        });
        let audit_context = DispatchAuditContext::from_request(&request, 4242, &caller_role)
            .expect("audit context");
        let error = envelope_call_runtime().block_on(dispatch_request_with_backend(
            request,
            1000,
            Gid::current().as_raw(),
            caller_role,
            &audit_context,
            &config,
            &log,
            None,
            &backend,
        ))
        .expect_err("a call before the daemon binds must refuse");
        match &error {
            BrokerError::LiveHandler(detail) => assert!(
                detail.contains("unregistered-handler"),
                "the refusal names the documented prebind code, got {error:?}"
            ),
            other => panic!("expected the prebind refusal, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&root);
    }

    /// The generic envelope invocation surface (U10, KTD10) drives the
    /// committed-operation envelope over the origination leg: the arm
    /// classifies the attested caller, the envelope resolves/authorizes/
    /// validates/audits the committed row, and the dispatch crosses the
    /// forward carrier to the declaring process, whose answer travels back
    /// as the generic `EnvelopeInvoke` response. The peer is the daemon's
    /// rendezvous-shaped half and answers only from bytes it read off the
    /// accepted connection, so the operation/zone/payload assertions are
    /// grounded in what actually crossed the socket.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn an_envelope_invoke_root_call_crosses_the_carrier_and_returns_the_result() {
        use crate::envelope::{BrokerEnvelope, ForwardingDispatcher};
        use crate::forwarding::SocketForwarder;
        use crate::protocol::{bind_seqpacket, recv_json_frame, send_json_frame};
        use d2b_contracts_broker::broker_wire::{
            BrokerCallerRole, BrokerRequest, EnvelopeInvokeRequest, ForwardOperationOutcome,
            ForwardOperationRequest, ForwardOperationResponse,
        };
        use nix::sys::socket::{SockFlag, accept4};

        let root = test_audit_dir("envelope-invoke-happy");
        fs::create_dir_all(&root).expect("create audit test dir");
        let config = test_server_config(&root, &root.join("unused-bundle.json"));
        let (log, _capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");

        let socket_root = tempfile::tempdir().expect("forward socket dir");
        let forward_path = socket_root.path().join("d2bd-forward.sock");
        let listener = bind_seqpacket(&forward_path).expect("bind the test forwarding peer");
        let peer = std::thread::spawn(move || {
            let fd = accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC)
                .expect("the broker's forwarder dials the peer");
            let request = recv_json_frame::<ForwardOperationRequest>(fd.as_raw_fd())
                .expect("read the forwarded invocation")
                .expect("the broker's forwarder sent a frame");
            assert_eq!(request.operation, "inspect-process-family");
            assert_eq!(request.zone, "test");
            assert_eq!(
                request.payload,
                serde_json::json!({ "resourceType": "Process" })
            );
            send_json_frame(
                fd.as_raw_fd(),
                &ForwardOperationResponse {
                    outcome: ForwardOperationOutcome::Result {
                        result: serde_json::json!({
                            "family": "process",
                            "resourceType": "Process",
                            "zone": "test",
                        }),
                        fd_indexes: vec![],
                        fd_kinds: vec![],
                    },
                },
            )
            .expect("write the forwarded reply");
        });

        let envelope = BrokerEnvelope::over(
            crate::catalog::BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(SocketForwarder::new(
                forward_path,
            ))),
        )
        .commit_forwarded()
        .build();
        let backend = FakeDispatchBackend {
            envelope,
            ..FakeDispatchBackend::default()
        };
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let request = BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
            operation: "inspect-process-family".to_owned(),
            zone: "test".to_owned(),
            payload: serde_json::json!({ "resourceType": "Process" }),
            chain_root_invocation_id: None,
            chain_identities: None,
            fd_indexes: vec![],
            fd_kinds: vec![],
        });
        let audit_context = DispatchAuditContext::from_request(&request, 4242, &caller_role)
            .expect("audit context");
        let result = envelope_call_runtime().block_on(dispatch_request_with_backend(
            request,
            1000,
            Gid::current().as_raw(),
            caller_role,
            &audit_context,
            &config,
            &log,
            None,
            &backend,
        ))
        .expect("the generic invocation dispatches through the envelope");
        match result.response {
            BrokerResponse::EnvelopeInvoke(response) => {
                assert_eq!(response.operation, "inspect-process-family");
                assert!(!response.invocation_id.is_empty());
                assert_eq!(response.refusal, None);
                let result = response
                    .result
                    .expect("the happy path carries the canonical result");
                assert_eq!(
                    result.get("family").and_then(serde_json::Value::as_str),
                    Some("process")
                );
                assert_eq!(
                    result.get("zone").and_then(serde_json::Value::as_str),
                    Some("test")
                );
            }
            other => panic!("expected an EnvelopeInvoke response, got {other:?}"),
        }
        peer.join().expect("the forwarding peer completes");
        let _ = fs::remove_dir_all(&root);
    }

    /// The generic invocation's error path: an operation no serving peer
    /// holds is refused with the envelope's closed `unregistered-handler`
    /// code inside the `EnvelopeInvoke` response, so the caller sees the
    /// refusal code and the invocation id instead of a transport error.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn an_envelope_invoke_call_without_a_serving_peer_refuses_with_the_closed_code() {
        use crate::envelope::{BrokerEnvelope, ForwardingDispatcher};
        use d2b_contracts_broker::broker_wire::{
            BrokerCallerRole, BrokerRequest, EnvelopeInvokeRequest,
        };

        let root = test_audit_dir("envelope-invoke-prebind");
        fs::create_dir_all(&root).expect("create audit test dir");
        let config = test_server_config(&root, &root.join("unused-bundle.json"));
        let (log, _capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");
        let envelope = BrokerEnvelope::over(
            crate::catalog::BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::default()),
        )
        .commit_forwarded()
        .build();
        let backend = FakeDispatchBackend {
            envelope,
            ..FakeDispatchBackend::default()
        };
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let request = BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
            operation: "inspect-process-family".to_owned(),
            zone: "test".to_owned(),
            payload: serde_json::json!({ "resourceType": "Process" }),
            chain_root_invocation_id: None,
            chain_identities: None,
            fd_indexes: vec![],
            fd_kinds: vec![],
        });
        let audit_context = DispatchAuditContext::from_request(&request, 4242, &caller_role)
            .expect("audit context");
        let result = envelope_call_runtime().block_on(dispatch_request_with_backend(
            request,
            1000,
            Gid::current().as_raw(),
            caller_role,
            &audit_context,
            &config,
            &log,
            None,
            &backend,
        ))
        .expect("the refusal travels inside the EnvelopeInvoke response");
        match result.response {
            BrokerResponse::EnvelopeInvoke(response) => {
                assert_eq!(response.operation, "inspect-process-family");
                assert_eq!(
                    response.refusal.as_deref(),
                    Some(crate::envelope::UNREGISTERED_HANDLER)
                );
                assert!(!response.invocation_id.is_empty());
            }
            other => panic!("expected an EnvelopeInvoke refusal, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn an_unwritable_state_dir_refuses_publications_fail_closed_without_taking_the_broker_down() {
        use d2b_contracts_broker::broker_wire::{
            BrokerCallerRole, BrokerRequest, PublishTrustedContextValues,
        };

        // Lazy-store regression (U2 startup fix): opening the
        // trusted-context store must not happen at broker startup - a state
        // root that cannot host `trusted-context/` refuses publications
        // fail-closed while the broker keeps serving everything else.
        // `state_dir` is a regular file, so `state_dir/trusted-context` can
        // never be created on any host under any uid (a chmod-based
        // read-only dir would not bite a root test runner).
        let root = test_audit_dir("trusted-store-unwritable");
        fs::create_dir_all(&root).expect("create test root");
        let blocker = root.join("state");
        fs::write(&blocker, b"a file blocks the trusted-context store")
            .expect("write the blocking file");
        let config = test_server_config(&root, &root.join("unused-bundle.json"));
        let config = ServerConfig {
            state_dir: blocker.clone(),
            ..config
        };

        // The open itself fails closed with the documented store-unavailable
        // failure, deterministically, regardless of whether an earlier test
        // already opened the process store.
        let open_error = crate::envelope::init_trusted_context_store(&blocker)
            .expect_err("a state dir that cannot host the store must fail its open");
        assert!(
            format!("{open_error}").contains("trusted-context"),
            "the open failure names the store: {open_error}"
        );

        let (log, _capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");
        let backend = FakeDispatchBackend::default();
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let values = PublishTrustedContextValues {
            zone: "test".to_owned(),
            provider_set_revision: 1,
            controller_generation: 1,
            guest_generation: 1,
        };
        let audit_context = DispatchAuditContext::from_request(
            &BrokerRequest::PublishTrustedContext(values.clone()),
            4242,
            &caller_role,
        )
        .expect("audit context");
        let outcome = envelope_call_runtime().block_on(dispatch_request_with_backend(
            BrokerRequest::PublishTrustedContext(values),
            1000,
            Gid::current().as_raw(),
            caller_role,
            &audit_context,
            &config,
            &log,
            None,
            &backend,
        ));
        match outcome {
            // No store exists yet (this process never published before):
            // the arm's lazy open refuses the publication by name, and the
            // refusal is a dispatch error - the broker process itself keeps
            // serving.
            Err(BrokerError::LiveHandler(detail)) => {
                assert!(
                    detail.contains("trusted-context store unavailable"),
                    "the refusal names the documented store failure: {detail}"
                );
            }
            // An earlier test already opened the process store at its own
            // scratch root: the broker serves publications from the
            // already-open store (the documented reuse path) - equally not
            // a startup failure.
            Ok(result) => {
                assert!(
                    matches!(result.response, BrokerResponse::PublishTrustedContext(_)),
                    "an already-open process store keeps serving publications, got {:?}",
                    result.response
                );
            }
            Err(other) => panic!("unexpected publication failure: {other:?}"),
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[cfg_attr(
        not(test_root),
        ignore = "v1.1.1fu11: requires write access to /var/lib/d2b/runtime/ which only root can do; run with --cfg test_root in a privileged test environment"
    )]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn dispatch_request_writes_typed_op_audit_records_for_all_live_arms() {
        use d2b_contracts::types::{BundleOpId, ScopeId, TracingSpanId, VmId};
        use d2b_contracts_broker::broker_wire::{
            BrokerAuditFilter, BrokerCallerRole, BrokerRequest,
        };
        use d2b_core::bundle_resolver::intent_id_usbip_firewall;

        let root = test_audit_dir("dispatch-typed-op-audit");
        let bundle = build_test_bundle(&root);
        let config = test_server_config(&root, &bundle.manifest_path);
        let (log, capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");
        let backend = FakeDispatchBackend::default();
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let caller_gid = Gid::current().as_raw();
        let peer_pid = 4242;
        let _usb_sysfs_guard = usb_sysfs_test_lock();
        prepare_test_usb_sysfs_device("1050", "0407", "2.3");

        let assert_dispatch = |request: BrokerRequest,
                               operation: &str,
                               expected_fields: OperationFields,
                               expected_tracing: Option<&str>| {
            let expected_request_fields = request_fields_value(&request).expect("request fields");
            let audit_context =
                DispatchAuditContext::from_request(&request, peer_pid, &caller_role)
                    .expect("audit context");
            let before = capture.lock().expect("capture lock before dispatch").len();
            let result = envelope_call_runtime().block_on(dispatch_request_with_backend(
                request,
                1000,
                caller_gid,
                caller_role.clone(),
                &audit_context,
                &config,
                &log,
                Some(&bundle.resolver),
                &backend,
            ))
            .expect("dispatch succeeds");
            let records = capture.lock().expect("capture lock after dispatch");
            assert_eq!(
                records.len(),
                before + 1,
                "{operation} should add one audit record"
            );
            let record = records[before].clone();
            drop(records);
            assert_eq!(record.operation, operation);
            assert_eq!(
                record.bundle_version,
                bundle.resolver.audit_bundle_version()
            );
            assert_eq!(record.bundle_hash, bundle.resolver.audit_bundle_hash());
            assert_eq!(record.peer_uid, 1000);
            assert_eq!(record.peer_gid, caller_gid);
            assert_eq!(record.peer_pid, peer_pid);
            assert_eq!(record.peer_role, caller_role.for_display());
            assert_eq!(record.authz_result, "admin");
            assert_eq!(record.verb, operation);
            assert_eq!(record.request_fields, expected_request_fields);
            assert_eq!(record.decision, "allowed");
            assert_eq!(record.result, "success");
            assert_eq!(record.error_kind, None);
            assert_eq!(record.tracing_span_id.as_deref(), expected_tracing);
            assert!(is_uuid_v4_like(&record.event_id));
            let fields = OperationFields::from_operation_value(
                operation,
                record.operation_fields.expect("operation fields present"),
            )
            .expect("deserialize operation fields");
            assert_eq!(
                fields, expected_fields,
                "unexpected operation_fields for {operation}"
            );
            result
        };

        let assert_ack = |result: DispatchResult, operation: &str| {
            assert!(result.fds.is_empty(), "{operation} should not return fds");
            match result.response {
                BrokerResponse::Ack(response) => {
                    assert!(response.accepted);
                    assert_eq!(response.operation, operation);
                }
                other => panic!("expected Ack for {operation}, got {other:?}"),
            }
        };

        let hello = assert_dispatch(
            BrokerRequest::Hello(d2b_contracts_broker::broker_wire::HelloRequest {
                client_version: "1.2.3".to_owned(),
                supported_features: vec!["typed-audit".to_owned()],
            }),
            "Hello",
            OperationFields::Hello {
                client_version: "1.2.3".to_owned(),
            },
            Some("usb-start-0000000000000001"),
        );
        match hello.response {
            BrokerResponse::Hello(response) => {
                assert_eq!(response.selected_version, "0.0.0-w2");
                assert!(response.capabilities.contains(&"Hello".to_owned()));
            }
            other => panic!("expected Hello response, got {other:?}"),
        }

        let export_filter = BrokerAuditFilter {
            env: Some("work".to_owned()),
            operation: Some("Run".to_owned()),
            vm: Some("corp-vm".to_owned()),
            role: None,
            outcome: None,
            severity: None,
        };
        let export_filter_json = serde_json::to_string(&export_filter).expect("serialize filter");
        let export = assert_dispatch(
            BrokerRequest::ExportBrokerAudit(
                d2b_contracts_broker::broker_wire::ExportBrokerAuditRequest {
                    since: Some("2026-01-01T00:00:00Z".to_owned()),
                    filter: Some(export_filter),
                    cursor: None,
                    limit: 256,
                },
            ),
            "ExportBrokerAudit",
            OperationFields::ExportBrokerAudit {
                since: Some("2026-01-01T00:00:00Z".to_owned()),
                filter: Some(export_filter_json),
            },
            None,
        );
        match export.response {
            BrokerResponse::ExportBrokerAudit(response) => {
                assert_eq!(response.entries.len(), 0)
            }
            other => panic!("expected ExportBrokerAudit response, got {other:?}"),
        }

        // U10 retired the typed process-family wire arms (OpenPidfd,
        // SignalRunner, SpawnRunner) at wire v6. Their privileged cores
        // are served by the broker-generic kernels through the envelope
        // (see `retired_process_family_kernels_dispatch_through_the_envelope`
        // for the dispatch surface), and their typed audit shapes stay in
        // the vocabulary for the records the pre-cut binaries wrote. A
        // straggler frame for a retired variant is refused by the wire
        // gate with the stale-wire-version code plus an audit record
        // (KTD10) - see tests/broker_protocol_compatibility.rs.

        assert_ack(
            assert_dispatch(
                BrokerRequest::UsbipBind(d2b_contracts_broker::broker_wire::UsbipBindRequest {
                    bundle_usbip_bind_intent_ref: BundleOpId::new(
                        d2b_core::bundle_resolver::intent_id_usbip_bind("work", "corp-vm", "1-2.3"),
                    ),
                    tracing_span_id: Some(TracingSpanId::new("usb-start-0000000000000001")),
                }),
                "UsbipBind",
                OperationFields::UsbipBind {
                    bus_id: "1-2.3".to_owned(),
                    vm: "corp-vm".to_owned(),
                    device_identity: Some(UsbAuditDeviceIdentity {
                        vendor_id: Some("1050".to_owned()),
                        product_id: Some("0407".to_owned()),
                        serial_observed: false,
                        serial_correlation: None,
                        previous_serial_correlation: None,
                    }),
                },
                Some("usb-start-0000000000000001"),
            ),
            "UsbipBind",
        );

        assert_ack(
            assert_dispatch(
                BrokerRequest::UsbipUnbind(d2b_contracts_broker::broker_wire::UsbipUnbindRequest {
                    bundle_usbip_bind_intent_ref: BundleOpId::new(
                        d2b_core::bundle_resolver::intent_id_usbip_bind("work", "corp-vm", "1-2.3"),
                    ),
                    preserve_durable_claim: false,
                    tracing_span_id: Some(TracingSpanId::new("usb-stop-0000000000000002")),
                }),
                "UsbipUnbind",
                OperationFields::UsbipUnbind {
                    bus_id: "1-2.3".to_owned(),
                },
                Some("usb-stop-0000000000000002"),
            ),
            "UsbipUnbind",
        );

        assert_ack(
            assert_dispatch(
                BrokerRequest::UsbipProxyReconcile(
                    d2b_contracts_broker::broker_wire::UsbipProxyReconcileRequest {
                        scope_id: ScopeId::new("global"),
                        tracing_span_id: Some(TracingSpanId::new("usb-proxy-0000000000000003")),
                    },
                ),
                "UsbipProxyReconcile",
                OperationFields::UsbipProxyReconcile {},
                Some("usb-proxy-0000000000000003"),
            ),
            "UsbipProxyReconcile",
        );

        assert_ack(
            assert_dispatch(
                BrokerRequest::UsbipBindFirewallRule(
                    d2b_contracts_broker::broker_wire::UsbipBindFirewallRuleRequest {
                        bundle_usbip_firewall_intent_ref: BundleOpId::new(
                            intent_id_usbip_firewall("work", "1-2.3"),
                        ),
                        tracing_span_id: Some(TracingSpanId::new("span-usbip-fw")),
                    },
                ),
                "UsbipBindFirewallRule",
                OperationFields::UsbipBindFirewallRule {
                    bundle_usbip_firewall_intent_ref: intent_id_usbip_firewall("work", "1-2.3"),
                },
                Some("span-usbip-fw"),
            ),
            "UsbipBindFirewallRule",
        );

        let qemu_powerdown = assert_dispatch(
            BrokerRequest::QemuMediaSystemPowerdown(
                d2b_contracts_broker::broker_wire::QemuMediaLifecycleRequest {
                    vm_id: VmId::new("media"),
                    tracing_span_id: Some(TracingSpanId::new("span-qmp-powerdown")),
                },
            ),
            "QemuMediaSystemPowerdown",
            OperationFields::QemuMediaSystemPowerdown {
                vm_id: "media".to_owned(),
                qmp_command: "system_powerdown".to_owned(),
            },
            Some("span-qmp-powerdown"),
        );
        match qemu_powerdown.response {
            BrokerResponse::QemuMediaSystemPowerdown(response) => {
                assert_eq!(
                    response.command,
                    d2b_contracts_broker::broker_wire::QemuMediaLifecycleAction::SystemPowerdown
                );
            }
            other => panic!("expected QemuMediaSystemPowerdown response, got {other:?}"),
        }

        let before_query = capture.lock().expect("capture before query").len();
        let query_context = DispatchAuditContext::from_request(
            &BrokerRequest::QemuMediaQueryStatus(
                d2b_contracts_broker::broker_wire::QemuMediaQueryStatusRequest {
                    vm_id: VmId::new("media"),
                    shutdown_context: true,
                    tracing_span_id: Some(TracingSpanId::new("span-qmp-status")),
                },
            ),
            peer_pid,
            &caller_role,
        )
        .expect("query audit context");
        let query_result = envelope_call_runtime().block_on(dispatch_request_with_backend(
            BrokerRequest::QemuMediaQueryStatus(
                d2b_contracts_broker::broker_wire::QemuMediaQueryStatusRequest {
                    vm_id: VmId::new("media"),
                    shutdown_context: true,
                    tracing_span_id: Some(TracingSpanId::new("span-qmp-status")),
                },
            ),
            1000,
            caller_gid,
            caller_role.clone(),
            &query_context,
            &config,
            &log,
            Some(&bundle.resolver),
            &backend,
        ))
        .expect("query status succeeds without success audit");
        match query_result.response {
            BrokerResponse::QemuMediaQueryStatus(response) => {
                assert_eq!(
                    response.status,
                    d2b_contracts_broker::broker_wire::QemuMediaVmStatus::ConnectionLostDuringShutdown
                );
            }
            other => panic!("expected QemuMediaQueryStatus response, got {other:?}"),
        }
        assert_eq!(
            capture.lock().expect("capture after query").len(),
            before_query,
            "read-only query-status must suppress success audit"
        );

        let qemu_quit = assert_dispatch(
            BrokerRequest::QemuMediaQuit(
                d2b_contracts_broker::broker_wire::QemuMediaLifecycleRequest {
                    vm_id: VmId::new("media"),
                    tracing_span_id: Some(TracingSpanId::new("span-qmp-quit")),
                },
            ),
            "QemuMediaQuit",
            OperationFields::QemuMediaQuit {
                vm_id: "media".to_owned(),
                qmp_command: "quit".to_owned(),
            },
            Some("span-qmp-quit"),
        );
        match qemu_quit.response {
            BrokerResponse::QemuMediaQuit(response) => {
                assert_eq!(
                    response.command,
                    d2b_contracts_broker::broker_wire::QemuMediaLifecycleAction::Quit
                );
            }
            other => panic!("expected QemuMediaQuit response, got {other:?}"),
        }

        assert_eq!(
            capture.lock().expect("capture final lock").len(),
            8,
            "expected one typed audit record per live dispatch arm"
        );

        let _ = fs::remove_dir_all(&root);
    }
    /// Build a `corp-vm` bundle whose resolved store-view intent points at
    /// tempdir-backed closure + farm paths (rooted under `root`, single
    /// filesystem) so a full `StoreSync` dispatch round-trip runs
    /// unprivileged. `host_generation` controls the resolved generation so
    /// a mismatching wire token can deterministically force a pre-lock
    /// failure. Returns the bundle plus the per-VM hardlink-farm root.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn store_sync_dispatch_bundle(root: &Path, host_generation: u32) -> (TestBundle, PathBuf) {
        use d2b_contracts_resource::v3::ZoneId;
        use d2b_core::bundle_resolver::{ResolvedStoreViewIntent, intent_id_store_view};

        let mut bundle = build_test_bundle(root);

        let store_src = root.join("nix-store-mock");
        fs::create_dir_all(&store_src).expect("create fake nix store");
        let toplevel = store_src.join("aaaaaaaaaaaaaaaa-corp-vm-system");
        fs::create_dir_all(&toplevel).expect("create fake toplevel");
        fs::write(toplevel.join("hello"), "payload").expect("write fake toplevel payload");
        let db_dump = root.join("corp-vm-registration");
        fs::write(&db_dump, "db-dump").expect("write fake db dump");
        let state_dir = root.join("state").join("corp-vm");
        let farm_path = state_dir.join("store-view");
        fs::create_dir_all(&farm_path).expect("create per-vm farm root");

        // Zone-native store-view: inject the resolved intent directly (the
        // v3 catalog materialisation is not part of unit fixtures).
        let zone = ZoneId::parse("work").expect("zone");
        let intent = ResolvedStoreViewIntent {
            intent_id: intent_id_store_view(&zone, "corp-vm"),
            vm: "corp-vm".to_owned(),
            generation: u64::from(host_generation),
            hardlink_farm_path: farm_path.clone(),
            target_view_path: farm_path.join("live/aaaaaaaaaaaaaaaa-corp-vm-system"),
            closure_paths: vec![toplevel],
            db_dump_path: db_dump,
        };
        let resolver = Arc::get_mut(&mut bundle.resolver).expect("resolver uniquely owned");
        resolver.test_inject_store_view_intent(intent);
        resolver.test_inject_zone_resource_bundle(
            "zones/work/resource-bundle.json".to_owned(),
            zone_bundle_with_guest("work", "corp-vm"),
        );
        (bundle, farm_path)
    }

    /// Build a minimal verified v3 zone resource bundle carrying one Guest
    /// (`corp-vm`) so zone-native accessors resolve it in the `work` Zone.
    #[cfg(not(feature = "layer1-bootstrap"))]
    fn zone_bundle_with_guest(zone: &str, guest: &str) -> Vec<u8> {
        use std::collections::BTreeMap;

        use d2b_contracts_resource::v3::{
            CanonicalJsonObject, ResourceName, ResourceTypeName, Timestamp, ZoneId,
        };
        use d2b_contracts_zone_session::v3::resource_bundle::{
            BundleResource, BundleResourceMetadata, ResourceBundle,
        };

        let zone = ZoneId::parse(zone).expect("zone");
        let resource = BundleResource::new(
            ResourceTypeName::parse("Guest").expect("guest type"),
            BundleResourceMetadata::new(
                ResourceName::parse(guest).expect("guest name"),
                zone.clone(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            CanonicalJsonObject::parse(br#"{"providerRef":"Provider/runtime-cloud-hypervisor"}"#)
                .expect("guest spec"),
        )
        .expect("guest resource");
        let bundle = ResourceBundle::new(
            zone,
            vec![resource],
            format!("sha256:{}", "b".repeat(64)),
            BTreeMap::new(),
            BTreeMap::new(),
            Timestamp::parse("1970-01-01T00:00:00.000Z").expect("timestamp"),
        )
        .expect("zone resource bundle");
        serde_json::to_vec(&bundle).expect("serialize zone resource bundle")
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    fn store_sync_request(
        generation_token: u32,
    ) -> d2b_contracts_broker::broker_wire::BrokerRequest {
        use d2b_contracts::types::{BundleClosureRef, TracingSpanId, VmId};
        use d2b_contracts_broker::broker_wire::{BrokerRequest, StoreSyncRequest};
        use d2b_core::bundle_resolver::intent_id_store_view;

        BrokerRequest::StoreSync(StoreSyncRequest {
            vm_id: VmId::new("corp-vm"),
            bundle_closure_ref: BundleClosureRef::new(intent_id_store_view(
                &d2b_contracts_resource::v3::ZoneId::parse("work").expect("zone"),
                "corp-vm",
            )),
            generation_token,
            tracing_span_id: Some(TracingSpanId::new("span-store-sync")),
        })
    }

    /// W3 success emission must survive the W4 dispatch-arm refactor: the
    /// first (non-fast) sync emits EXACTLY ONE allowed terminal record with
    /// the deferred-cleanup `ok_non_fast_path` shape.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn store_sync_dispatch_emits_single_success_record() {
        use crate::ops::store_sync_audit::{
            AuthzOutcome, CleanupReason, CleanupStatus, ErrorStage, SyncStatus,
        };
        use d2b_contracts_broker::broker_wire::BrokerCallerRole;

        let root = test_audit_dir("store-sync-dispatch-success");
        let (bundle, _farm) = store_sync_dispatch_bundle(&root, 7);
        let config = test_server_config(&root, &bundle.manifest_path);
        let (log, capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");
        let backend = FakeDispatchBackend::default();
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let caller_gid = Gid::current().as_raw();

        let request = store_sync_request(7);
        let audit_context = DispatchAuditContext::from_request(&request, 4242, &caller_role)
            .expect("audit context");
        let before = capture.lock().expect("capture lock before").len();
        let result = envelope_call_runtime().block_on(dispatch_request_with_backend(
            request,
            1000,
            caller_gid,
            caller_role.clone(),
            &audit_context,
            &config,
            &log,
            Some(&bundle.resolver),
            &backend,
        ))
        .expect("store sync succeeds");
        match result.response {
            BrokerResponse::StoreSync(resp) => {
                assert_eq!(resp.vm, "corp-vm");
                assert_eq!(resp.generation_token, 7);
                assert!(!resp.cleanup_deferred);
            }
            other => panic!("expected StoreSync response, got {other:?}"),
        }

        let records = capture.lock().expect("capture lock after");
        assert_eq!(records.len(), before + 1, "exactly one terminal record");
        let record = records[before].clone();
        drop(records);
        assert_eq!(record.operation, "StoreSync");
        assert_eq!(record.decision, "allowed");
        assert_eq!(record.result, "success");
        assert_eq!(record.error_kind, None);
        let fields = match OperationFields::from_operation_value(
            "StoreSync",
            record.operation_fields.clone().expect("operation fields"),
        )
        .expect("deserialize store-sync fields")
        {
            OperationFields::StoreSync(fields) => fields,
            other => panic!("expected StoreSync fields, got {other:?}"),
        };
        fields.validate().expect("signed schema holds");
        assert_eq!(fields.sync_status, SyncStatus::Ok);
        assert_eq!(fields.env.as_deref(), Some("work"));
        assert_eq!(fields.error_stage, ErrorStage::None);
        assert_eq!(fields.cleanup_status, CleanupStatus::Completed);
        assert_eq!(fields.cleanup_reason, CleanupReason::None);
        assert_eq!(fields.authz_outcome, AuthzOutcome::Allow);
        assert!(!fields.fast_path);
        assert_eq!(
            fields.linked_count + fields.skipped_count,
            fields.closure_count
        );

        // ADR 0027 observability export: exactly one StoreSync-only
        // record, projected to the signed allow-list (redaction fields
        // absent), carrying the terminal success shape with the target
        // VM in JSON content (not a Loki label).
        let exported = read_store_sync_export(&config);
        assert_eq!(exported.len(), 1, "exactly one exported record on success");
        let (export_record, export_obj) = &exported[0];
        assert_export_allow_list(export_obj);
        assert_eq!(export_record.target_vm, "corp-vm");
        assert_eq!(export_record.target_env.as_deref(), Some("work"));
        assert_eq!(export_record.vm_id, "corp-vm");
        assert_eq!(export_record.generation_token, 7);
        assert_eq!(export_record.sync_status, SyncStatus::Ok);
        assert_eq!(export_record.error_stage, ErrorStage::None);
        assert_eq!(export_record.cleanup_status, CleanupStatus::Completed);
        assert_eq!(export_record.authz_outcome, AuthzOutcome::Allow);
        assert!(!export_record.fast_path);

        let _ = fs::remove_dir_all(&root);
    }

    /// A malformed authoritative audit join must yield the typed protocol
    /// refusal, never a panic. The parse-failure leg cannot be driven at
    /// HEAD: `authoritative_audit_join` computes canonical digests, so
    /// `CanonicalAuditDigest::parse` always succeeds on its output (the
    /// refusal was observed under a mutation that made the join return raw
    /// strings - see the wave-0 report). This test pins the typed-refusal
    /// surface of the converted call: a valid join builds the context
    /// without panicking, and a supplied join that mismatches the request's
    /// canonical join is refused with the typed Protocol error.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn from_request_refuses_a_malformed_audit_join_with_a_typed_protocol_error() {
        let request = store_sync_request(7);
        let caller_role = CallerRole::AdminUid { uid: 1000 };

        let context = DispatchAuditContext::from_request(&request, 4242, &caller_role)
            .expect("valid audit join builds the dispatch context");
        let (zone_id, operation_identity) = request
            .authoritative_audit_join()
            .expect("store sync carries an authoritative join");
        let join = context.audit_join.as_ref().expect("join recorded");
        assert_eq!(join.zone_id.as_str(), zone_id.as_str());
        assert_eq!(join.operation_identity.as_str(), operation_identity.as_str());

        let foreign_join = AuditJoinContext {
            zone_id: CanonicalAuditDigest::parse(
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            )
            .expect("fixed canonical digest"),
            operation_identity: CanonicalAuditDigest::parse(
                "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            )
            .expect("fixed canonical digest"),
        };
        let error = DispatchAuditContext::from_request_with_join(
            &request,
            4242,
            &caller_role,
            Some(&foreign_join),
        )
        .expect_err("a mismatched supplied join must be refused");
        assert!(matches!(error, BrokerError::Protocol(_)));
    }

    /// A second sync of the same closure must take the fast path and still
    /// emit EXACTLY ONE allowed record carrying `skipped_fast_path` +
    /// `fast_path`.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn store_sync_dispatch_fast_path_emits_single_skipped_record() {
        use crate::ops::store_sync_audit::{CleanupReason, CleanupStatus, SyncStatus};
        use d2b_contracts_broker::broker_wire::BrokerCallerRole;

        let root = test_audit_dir("store-sync-dispatch-fast-path");
        let (bundle, _farm) = store_sync_dispatch_bundle(&root, 7);
        let config = test_server_config(&root, &bundle.manifest_path);
        let (log, capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");
        let backend = FakeDispatchBackend::default();
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let caller_gid = Gid::current().as_raw();

        let dispatch_once = || {
            let request = store_sync_request(7);
            let audit_context = DispatchAuditContext::from_request(&request, 4242, &caller_role)
                .expect("audit ctx");
            envelope_call_runtime().block_on(dispatch_request_with_backend(
                request,
                1000,
                caller_gid,
                caller_role.clone(),
                &audit_context,
                &config,
                &log,
                Some(&bundle.resolver),
                &backend,
            ))
            .expect("store sync succeeds")
        };

        // Publish generation 7, then re-sync it (fast path).
        let _ = dispatch_once();
        let before_fast = capture.lock().expect("capture lock").len();
        let _ = dispatch_once();

        let records = capture.lock().expect("capture lock after fast path");
        assert_eq!(
            records.len(),
            before_fast + 1,
            "fast-path re-sync emits exactly one record"
        );
        let record = records[before_fast].clone();
        drop(records);
        assert_eq!(record.operation, "StoreSync");
        assert_eq!(record.decision, "allowed");
        assert_eq!(record.result, "success");
        let fields = match OperationFields::from_operation_value(
            "StoreSync",
            record.operation_fields.clone().expect("operation fields"),
        )
        .expect("deserialize store-sync fields")
        {
            OperationFields::StoreSync(fields) => fields,
            other => panic!("expected StoreSync fields, got {other:?}"),
        };
        fields.validate().expect("signed schema holds");
        assert_eq!(fields.sync_status, SyncStatus::Ok);
        assert!(fields.fast_path);
        assert_eq!(fields.cleanup_status, CleanupStatus::SkippedFastPath);
        assert_eq!(fields.cleanup_reason, CleanupReason::FastPath);
        assert_eq!(fields.linked_count, 0);
        assert_eq!(fields.skipped_count, fields.closure_count);
        assert_eq!(fields.swept_count, 0);

        // ADR 0027 observability export: each terminal attempt exports
        // exactly one record, so the publish + fast-path re-sync produce
        // two lines; the second carries the pure fast-path shape.
        let exported = read_store_sync_export(&config);
        assert_eq!(
            exported.len(),
            2,
            "publish + fast-path re-sync export one record each"
        );
        let (fast_record, fast_obj) = &exported[1];
        assert_export_allow_list(fast_obj);
        assert_eq!(fast_record.sync_status, SyncStatus::Ok);
        assert!(fast_record.fast_path, "second export is the fast path");
        assert_eq!(fast_record.cleanup_status, CleanupStatus::SkippedFastPath);
        assert_eq!(fast_record.cleanup_reason, CleanupReason::FastPath);
        assert_eq!(fast_record.linked_count, 0);
        assert_eq!(fast_record.skipped_count, fast_record.closure_count);

        let _ = fs::remove_dir_all(&root);
    }

    /// A deterministic pre-lock failure (wire generation token does not
    /// match the resolved closure generation) must emit EXACTLY ONE signed
    /// `failed` terminal record (decision = errored), leak no guest
    /// metadata, and NOT produce a duplicate record when the outer
    /// error-audit path runs.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn store_sync_dispatch_failure_emits_single_signed_failure_record() {
        use crate::ops::store_sync_audit::{
            AuthzOutcome, CleanupReason, CleanupStatus, ErrorStage, SyncStatus,
        };
        use d2b_contracts_broker::broker_wire::BrokerCallerRole;

        let root = test_audit_dir("store-sync-dispatch-failure");
        // Resolved generation is 7; the wire asks for 8 → GenerationMismatch
        // before lock/filesystem side effects (error_stage = probe).
        let (bundle, farm) = store_sync_dispatch_bundle(&root, 7);
        let config = test_server_config(&root, &bundle.manifest_path);
        let (log, capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");
        let backend = FakeDispatchBackend::default();
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let caller_gid = Gid::current().as_raw();

        let request = store_sync_request(8);
        let audit_context = DispatchAuditContext::from_request(&request, 4242, &caller_role)
            .expect("audit context");
        let before = capture.lock().expect("capture lock before").len();
        let error = envelope_call_runtime().block_on(dispatch_request_with_backend(
            request,
            1000,
            caller_gid,
            caller_role.clone(),
            &audit_context,
            &config,
            &log,
            Some(&bundle.resolver),
            &backend,
        ))
        .expect_err("generation mismatch must fail");
        match &error {
            BrokerError::StoreSyncFailed {
                error_stage,
                message,
            } => {
                assert_eq!(*error_stage, "store-sync-probe-failed");
                assert!(message.contains("generation"), "message: {message}");
            }
            other => panic!("expected StoreSyncFailed, got {other:?}"),
        }

        let after_dispatch = capture.lock().expect("capture lock after dispatch").len();
        assert_eq!(
            after_dispatch,
            before + 1,
            "failure emits exactly one terminal record"
        );

        let record = {
            let records = capture.lock().expect("capture lock for record");
            records[before].clone()
        };
        assert_eq!(record.operation, "StoreSync");
        assert_eq!(record.decision, "errored");
        assert_eq!(record.result, "error");
        assert_eq!(
            record.error_kind.as_deref(),
            Some("store-sync-probe-failed")
        );
        let fields = match OperationFields::from_operation_value(
            "StoreSync",
            record.operation_fields.clone().expect("operation fields"),
        )
        .expect("deserialize store-sync fields")
        {
            OperationFields::StoreSync(fields) => fields,
            other => panic!("expected StoreSync fields, got {other:?}"),
        };
        fields.validate().expect("signed schema holds for failure");
        assert_eq!(fields.sync_status, SyncStatus::Failed);
        assert_eq!(fields.error_stage, ErrorStage::Probe);
        assert_eq!(fields.cleanup_status, CleanupStatus::NotAttempted);
        assert_eq!(fields.cleanup_reason, CleanupReason::None);
        assert_eq!(fields.authz_outcome, AuthzOutcome::Allow);
        assert!(!fields.fast_path);

        // No guest-served metadata may be planted by a pre-lock failure.
        assert!(
            !farm.join("meta").exists(),
            "pre-lock failure must not write guest metadata"
        );
        assert!(!farm.join("live").exists());
        assert!(!farm.join("state").exists());

        // ADR 0027 observability export: a failed terminal attempt also
        // exports EXACTLY ONE allow-list record (failed shape, classified
        // error_stage, no host-only fields). The export sink is separate
        // from the broker audit log, so the duplicate-suppression on the
        // outer error path does not touch it.
        let exported = read_store_sync_export(&config);
        assert_eq!(exported.len(), 1, "exactly one exported record on failure");
        let (export_record, export_obj) = &exported[0];
        assert_export_allow_list(export_obj);
        assert_eq!(export_record.target_vm, "corp-vm");
        assert_eq!(export_record.sync_status, SyncStatus::Failed);
        assert_eq!(export_record.error_stage, ErrorStage::Probe);
        assert_eq!(export_record.cleanup_status, CleanupStatus::NotAttempted);
        assert_eq!(export_record.cleanup_reason, CleanupReason::None);
        assert_eq!(export_record.authz_outcome, AuthzOutcome::Allow);

        // The outer error-audit path must NOT write a second (duplicate)
        // record: BrokerError::StoreSyncFailed.audit() is a no-op because
        // the terminal record was already emitted in the dispatch arm.
        error
            .audit(
                &log,
                1000,
                caller_gid,
                &CallerRole::AdminUid { uid: 1000 },
                &audit_context,
                Some(&bundle.resolver),
                "StoreSync",
                "corp-vm",
            )
            .expect("outer error audit");
        let after_outer = capture.lock().expect("capture lock after outer").len();
        assert_eq!(
            after_outer,
            before + 1,
            "outer error-audit must not duplicate the terminal StoreSync record"
        );
        // The export is likewise emitted exactly once across the whole
        // failure path (the outer audit no-op cannot re-export).
        assert_eq!(
            read_store_sync_export(&config).len(),
            1,
            "outer error-audit must not duplicate the StoreSync export record"
        );

        let _ = fs::remove_dir_all(&root);
    }

    fn otel_host_bridge_socket_path_extracts_unix_listen_target() {
        let argv = vec![
            "/run/current-system/sw/bin/socat".to_owned(),
            "-d".to_owned(),
            "-d".to_owned(),
            "UNIX-LISTEN:/run/d2b/otel/host-egress.sock,fork,reuseaddr,mode=0660".to_owned(),
            "EXEC:\"/run/current-system/sw/bin/d2b-ch-vsock-connect \
             /var/lib/d2b/vms/sys-obs/vsock.sock 14317\""
                .to_owned(),
        ];
        let path = otel_host_bridge_socket_path(&argv).expect("extract UNIX-LISTEN target");
        assert_eq!(
            path,
            PathBuf::from("/run/d2b/otel/host-egress.sock"),
            "the socket path is the UNIX-LISTEN target stripped of socat options"
        );
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn otel_host_bridge_socket_path_errors_without_unix_listen() {
        let argv = vec![
            "/run/current-system/sw/bin/socat".to_owned(),
            "-d".to_owned(),
        ];
        assert!(
            otel_host_bridge_socket_path(&argv).is_err(),
            "argv without a UNIX-LISTEN address must be rejected"
        );
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn cleanup_otel_host_bridge_stale_socket_noop_for_other_role() {
        use d2b_contracts_broker::broker_wire::RunnerRole;
        // A non-bridge role must short-circuit Ok before touching argv or
        // the filesystem, even with an otherwise-dangerous argv.
        let argv = vec!["UNIX-LISTEN:/etc/shadow,fork".to_owned()];
        envelope_call_runtime().block_on(cleanup_otel_host_bridge_stale_socket(&RunnerRole::CloudHypervisor, &argv))
            .expect("non-bridge role is a no-op");
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn cleanup_otel_host_bridge_stale_socket_rejects_path_outside_otel_runtime_dir() {
        use d2b_contracts_broker::broker_wire::RunnerRole;
        // The prefix guard must refuse any socket outside the d2b OTel
        // runtime dir so a malformed bundle can never unlink an arbitrary
        // path before the guarded `cleanup_stale_unix_socket_without_probe`.
        let argv = vec!["UNIX-LISTEN:/tmp/evil.sock,fork".to_owned()];
        assert!(
            envelope_call_runtime().block_on(cleanup_otel_host_bridge_stale_socket(&RunnerRole::OtelHostBridge, &argv)).is_err(),
            "socket path outside /run/d2b/otel/ must be refused"
        );
    }

    // U10 retired the typed `SpawnRunner` wire arm: runner-intent
    // validation (role/bridge/vm closed-set matching, the
    // `OtelHostBridge` fences) moved to the daemon-side family handler,
    // which validates the typed request against its own resolver and
    // invokes the broker's `spawn-process` kernel with the resolved plan.
    // The broker kernel executes a fully resolved plan and carries no
    // bundle knowledge, so these two broker-side intent-fence tests are
    // retired with the arm; the daemon-side equivalent belongs to the
    // declaring provider's tests (U10d2). The
    // `spawn_runner_rejects_otel_host_bridge_intent_for_non_obs_vm`
    // broker-side test was retired with the typed `SpawnRunner` arm.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[cfg(not(feature = "layer1-bootstrap"))]
    // The retired `SignalRunner` arm refused a runner the broker's
    // metadata registry did not know (`NoPidfd`). The U10 kernel surface
    // is registry-free by design - the daemon passes the pidfd it
    // received from `spawn-process` over the fd leg - so the successor
    // property is kernel-level: a signal on a stale pidfd (the target
    // already reaped) is refused with the handler-errored code inside the
    // envelope response.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn signal_pidfd_refuses_a_stale_pidfd() {
        use crate::envelope::{BrokerEnvelope, ForwardingDispatcher, KernelDispatcher};
        use crate::kernel_ops::{KernelConfig, kernel_table};
        use d2b_contracts_broker::broker_wire::{
            BrokerCallerRole, BrokerRequest, EnvelopeInvokeRequest, FdKind,
        };

        let root = test_audit_dir("signal-pidfd-stale");
        fs::create_dir_all(&root).expect("create test root");
        let config = test_server_config(&root, &root.join("unused-bundle.json"));
        let (log, _capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");
        let kernels = kernel_table(KernelConfig {
            state_dir: config.state_dir.clone(),
            runtime_root: root.join("runtime"),
            daemon_uid: config.d2bd_uid,
            daemon_gid: config.d2bd_gid,
            bundle_path: config.bundle_path.clone(),
        });
        let envelope = BrokerEnvelope::over(
            crate::catalog::BrokerProfileId::Host,
            Box::new(KernelDispatcher::new(
                kernels,
                ForwardingDispatcher::default(),
            )),
        )
        .commit_forwarded()
        .build();
        let backend = FakeDispatchBackend {
            envelope,
            ..FakeDispatchBackend::default()
        };
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };

        // A pidfd that names a child which has already been reaped: the
        // kernel's pidfd_send_signal cannot prove the target, and the
        // call is refused rather than guessed.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep child");
        let pid = child.id() as i32;
        let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(pid),
            nix::sys::signal::Signal::SIGKILL,
        )
        .expect("kill child");
        child.wait().expect("child reaped");

        let request = BrokerRequest::EnvelopeInvoke(EnvelopeInvokeRequest {
            operation: "signal-pidfd".to_owned(),
            zone: "work".to_owned(),
            payload: serde_json::json!({ "signal": libc::SIGTERM }),
            chain_root_invocation_id: None,
            chain_identities: None,
            fd_indexes: vec![0],
            fd_kinds: vec![FdKind::Any],
        });
        let audit_context = DispatchAuditContext::from_request(&request, 5151, &caller_role)
            .expect("audit context");
        let result = envelope_call_runtime().block_on(dispatch_request_with_backend_and_request_fds(
            request,
            1000,
            Gid::current().as_raw(),
            caller_role,
            &audit_context,
            &config,
            &log,
            None,
            &backend,
            vec![pidfd],
        ))
        .expect("the refusal travels inside the EnvelopeInvoke response");
        let BrokerResponse::EnvelopeInvoke(response) = result.response else {
            panic!(
                "expected an EnvelopeInvoke response, got {:?}",
                result.response
            );
        };
        assert_eq!(
            response.refusal.as_deref(),
            Some(crate::envelope::HANDLER_ERRORED),
            "a stale pidfd must be refused, not guessed: {:?}",
            response.detail
        );
        assert_eq!(response.result, None);

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn usbip_bind_rejects_device_outside_allowlist() {
        let root = test_audit_dir("usbip-allowlist");
        let mut bundle = build_test_bundle(&root);
        set_usbip_allowlist(
            &mut bundle,
            vec![d2b_core::host::VendorProductPair {
                vendor: 0x1050,
                product: 0x0407,
            }],
        );
        let sysfs_root = root.join("usb-sysfs");
        let device_dir = sysfs_root.join("1-2.3");
        fs::create_dir_all(&device_dir).expect("create fake usb sysfs dir");
        fs::write(device_dir.join("idVendor"), b"abcd\n").expect("write fake vendor id");
        fs::write(device_dir.join("idProduct"), b"1234\n").expect("write fake product id");
        fs::write(device_dir.join("busnum"), b"1\n").expect("write fake bus number");
        fs::write(device_dir.join("devnum"), b"7\n").expect("write fake device number");
        fs::write(device_dir.join("devpath"), b"2.3\n").expect("write fake port path");

        let intent = find_usbip_bind_intent_for(&bundle.resolver, "corp-vm", "1-2.3")
            .expect("bundle usbip bind intent");
        let err = envelope_call_runtime().block_on(enforce_usbip_allowlist(&intent, &sysfs_root))
            .expect_err("device outside allowlist must be rejected");
        match err {
            BrokerError::UsbipDeviceNotAllowed {
                busid,
                vendor,
                product,
            } => {
                assert_eq!(busid, "1-2.3");
                assert_eq!(vendor, 0xabcd);
                assert_eq!(product, 0x1234);
            }
            other => panic!("expected UsbipDeviceNotAllowed, got {other:?}"),
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn usbip_bind_rejects_missing_allowlist_as_required_policy() {
        let root = test_audit_dir("usbip-missing-allowlist");
        let mut bundle = build_test_bundle(&root);
        set_usbip_allowlist(&mut bundle, Vec::new());
        let sysfs_root = root.join("usb-sysfs");
        let device_dir = sysfs_root.join("1-2.3");
        fs::create_dir_all(&device_dir).expect("create fake usb sysfs dir");
        fs::write(device_dir.join("idVendor"), b"1050\n").expect("write fake vendor id");
        fs::write(device_dir.join("idProduct"), b"0407\n").expect("write fake product id");
        fs::write(device_dir.join("busnum"), b"1\n").expect("write fake bus number");
        fs::write(device_dir.join("devnum"), b"7\n").expect("write fake device number");
        fs::write(device_dir.join("devpath"), b"2.3\n").expect("write fake port path");

        let intent = find_usbip_bind_intent_for(&bundle.resolver, "corp-vm", "1-2.3")
            .expect("bundle usbip bind intent");
        let err = envelope_call_runtime().block_on(enforce_usbip_allowlist(&intent, &sysfs_root))
            .expect_err("missing required allowlist must fail closed");
        match err {
            BrokerError::UsbipPolicyMismatch { busid, reason } => {
                assert_eq!(busid, "1-2.3");
                assert!(reason.contains("allowlist"));
            }
            other => panic!("expected UsbipPolicyMismatch, got {other:?}"),
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn usbip_bind_rejects_topology_mismatch_as_required_policy() {
        let root = test_audit_dir("usbip-topology-policy");
        let mut bundle = build_test_bundle(&root);
        set_usbip_allowlist(
            &mut bundle,
            vec![d2b_core::host::VendorProductPair {
                vendor: 0x1050,
                product: 0x0407,
            }],
        );
        let sysfs_root = root.join("usb-sysfs");
        let device_dir = sysfs_root.join("1-2.3");
        fs::create_dir_all(&device_dir).expect("create fake usb sysfs dir");
        fs::write(device_dir.join("idVendor"), b"1050\n").expect("write fake vendor id");
        fs::write(device_dir.join("idProduct"), b"0407\n").expect("write fake product id");
        fs::write(device_dir.join("busnum"), b"1\n").expect("write fake bus number");
        fs::write(device_dir.join("devnum"), b"7\n").expect("write fake device number");
        fs::write(device_dir.join("devpath"), b"2.4\n").expect("write fake mismatched port path");

        let intent = find_usbip_bind_intent_for(&bundle.resolver, "corp-vm", "1-2.3")
            .expect("bundle usbip bind intent");
        let err = envelope_call_runtime().block_on(enforce_usbip_allowlist(&intent, &sysfs_root))
            .expect_err("topology mismatch must fail closed as policy");
        match &err {
            BrokerError::UsbipPolicyMismatch { busid, reason } => {
                assert_eq!(busid, "1-2.3");
                assert!(reason.contains("topology"));
            }
            other => panic!("expected UsbipPolicyMismatch, got {other:?}"),
        }

        let BrokerResponse::Error(response) = err.into_response() else {
            panic!("expected broker error response");
        };
        let rendered = format!("{} {}", response.message, response.action);
        assert!(response.kind.contains("PolicyMismatch"));
        assert!(
            rendered
                .split_whitespace()
                .any(d2b_contracts_resource::v3::is_canonical_digest)
        );
        assert!(!rendered.contains("1-2.3"), "{rendered}");
        assert!(!rendered.contains("/sys/"), "{rendered}");

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn usb_broker_ipc_refuses_non_daemon_so_peercred_before_dispatch() {
        use d2b_contracts::types::{BundleOpId, ScopeId};
        use d2b_contracts_broker::broker_wire::{
            BrokerCallerRole, BrokerRequestEnvelope, UsbipBindFirewallRuleRequest,
            UsbipBindRequest, UsbipProxyReconcileRequest, UsbipUnbindRequest,
        };
        use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
        use nix::unistd::Uid;
        use std::os::fd::AsRawFd;

        let root = test_audit_dir("usb-peercred-refused");
        fs::create_dir_all(&root).expect("create audit test dir");
        let actual_uid = Uid::current().as_raw();
        let caller_gid = Gid::current().as_raw();
        let configured_daemon_uid = if actual_uid == 0 { 1 } else { 0 };
        let mut config = test_server_config(&root, &root.join("unused-bundle.json"));
        config.test_mode = false;
        config.d2bd_uid = configured_daemon_uid;
        let log = Arc::new(
            AuditLog::open(
                &config.audit_dir,
                Gid::current().as_raw(),
                true,
                config.audit_retention_days,
            )
            .expect("open audit log"),
        );
        let limiter = Arc::new(tokio::sync::Mutex::new(IpcRateLimiter::new(64)));
        // The connection handler reads and writes frames through the reactor
        // now, so the test drives it the way the server does.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let served = Server {
            config: Arc::new(config.clone()),
            audit_log: Arc::clone(&log),
            dispatches: DispatchPool::new(2),
            nested_dispatches: DispatchPool::new(2),
            ipc_rate_limiter: Arc::clone(&limiter),
        };

        let requests = vec![
            BrokerRequest::UsbipBind(UsbipBindRequest {
                bundle_usbip_bind_intent_ref: BundleOpId::new(
                    "usbip-bind:env:work:vm:corp-vm:bus:1-2.3",
                ),
                tracing_span_id: None,
            }),
            BrokerRequest::UsbipUnbind(UsbipUnbindRequest {
                bundle_usbip_bind_intent_ref: BundleOpId::new(
                    "usbip-bind:env:work:vm:corp-vm:bus:1-2.3",
                ),
                preserve_durable_claim: false,
                tracing_span_id: None,
            }),
            BrokerRequest::UsbipBindFirewallRule(UsbipBindFirewallRuleRequest {
                bundle_usbip_firewall_intent_ref: BundleOpId::new("usbip-fw:env:work:bus:1-2.3"),
                tracing_span_id: None,
            }),
            BrokerRequest::UsbipProxyReconcile(UsbipProxyReconcileRequest {
                scope_id: ScopeId::new("env:work"),
                tracing_span_id: None,
            }),
        ];
        let mut operations = Vec::new();

        for request in requests {
            let operation = request.op_name();
            operations.push(operation);
            let envelope = BrokerRequestEnvelope {
                request,
                caller_role: BrokerCallerRole::AdminUid {
                    uid: configured_daemon_uid,
                },
                // Ignored because config.test_mode=false: the broker must use the
                // kernel SO_PEERCRED uid, not a caller-supplied envelope field.
                test_peer_uid: Some(configured_daemon_uid),
                audit_join: None,
            };
            let (client, server) = socketpair(
                AddressFamily::Unix,
                SockType::SeqPacket,
                None,
                SockFlag::SOCK_CLOEXEC,
            )
            .expect("socketpair");
            crate::protocol::send_json_frame(client.as_raw_fd(), &envelope)
                .expect("send broker request");
            runtime
                .block_on(async {
                    let connection =
                        AsyncSeqpacket::from_owned(server).expect("register accepted socket");
                    handle_connection(connection, &served).await
                })
                .expect("handle refused peer");
            let response = crate::protocol::recv_json_frame::<BrokerResponse>(client.as_raw_fd());
            assert!(
                matches!(
                    response,
                    Err(ref error) if error.kind() == io::ErrorKind::ConnectionReset
                ) || matches!(response, Ok(None)),
                "unauthenticated peers must be closed before request decoding: {response:?}"
            );
        }

        let audit = fs::read_to_string(log.current_daily_path()).expect("read audit log");
        for _operation in operations {
            assert!(audit.contains(r#""op":"PeerAuthentication""#), "{audit}");
        }
        assert_eq!(
            audit
                .matches(r#""disposition":"peer-refused-before-decode""#)
                .count(),
            4,
            "{audit}"
        );
        assert!(
            audit.contains(&format!("\"caller_uid\":{actual_uid}")),
            "{audit}"
        );
        assert!(
            audit.contains(&format!("\"caller_gid\":{caller_gid}")),
            "{audit}"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn a_retired_variant_call_from_an_old_binary_is_refused_with_the_stale_wire_code_and_audited() {
        // Mixed-version matrix fixture (U4 item 4 / KTD10): `ValidateBundle`
        // is the previous protocol's request the current protocol retired
        // (see tests/broker_protocol_compatibility.rs), standing in for the
        // variants U10 retires. A straggler's frame is well-formed JSON but
        // not a current `RequestEnvelope`, so without the gate it would be a
        // pre-dispatch wire-malformed-json drop; the gate must instead
        // answer the typed stale-wire-version refusal and append an audit
        // record.
        const FIXTURE_RETIRED: &[RetiredWireVariant] = &[RetiredWireVariant {
            variant: "ValidateBundle",
            retired_in_version: 4,
        }];

        let root = test_audit_dir("stale-wire-version-gate");
        fs::create_dir_all(&root).expect("create audit test dir");
        let mut config = test_server_config(&root, &root.join("unused-bundle.json"));
        config.retired_wire_variants = FIXTURE_RETIRED;
        let log = Arc::new(
            AuditLog::open(
                &config.audit_dir,
                Gid::current().as_raw(),
                true,
                config.audit_retention_days,
            )
            .expect("open audit log"),
        );
        let limiter = Arc::new(tokio::sync::Mutex::new(IpcRateLimiter::new(64)));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let served = Server {
            config: Arc::new(config.clone()),
            audit_log: Arc::clone(&log),
            dispatches: DispatchPool::new(2),
            nested_dispatches: DispatchPool::new(2),
            ipc_rate_limiter: Arc::clone(&limiter),
        };

        // The old binary's frame: the previous-protocol envelope spells the
        // request as the internally tagged `{"kind": "ValidateBundle"}`, so
        // the frame still parses as JSON while never decoding as a current
        // `RequestEnvelope`.
        let (client, server) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("socketpair");
        crate::protocol::send_json_frame(
            client.as_raw_fd(),
            &serde_json::json!({ "request": { "kind": "ValidateBundle" } }),
        )
        .expect("send the straggler frame");
        runtime
            .block_on(async {
                let connection =
                    AsyncSeqpacket::from_owned(server).expect("register accepted socket");
                handle_connection(connection, &served).await
            })
            .expect("the gate answers instead of dropping the connection");
        let response = crate::protocol::recv_json_frame::<BrokerResponse>(client.as_raw_fd())
            .expect("the gate wrote a reply frame")
            .expect("the reply frame is present");
        let BrokerResponse::Error(refusal) = response else {
            panic!("expected a typed error response, got {response:?}");
        };
        assert_eq!(refusal.kind, crate::envelope::STALE_WIRE_VERSION);
        assert_eq!(refusal.operation, "ValidateBundle");
        assert!(
            refusal.message.contains("ValidateBundle") && refusal.message.contains("4"),
            "the refusal names the retired variant and the wire-version boundary it was retired at: {}",
            refusal.message
        );

        let audit = fs::read_to_string(log.current_daily_path()).expect("read audit log");
        assert!(audit.contains(r#""op":"ValidateBundle""#), "{audit}");
        assert!(
            audit.contains(r#""disposition":"stale-wire-version""#),
            "{audit}"
        );
        assert!(audit.contains(r#""outcome":"refused""#), "{audit}");

        let _ = fs::remove_dir_all(&root);
    }

    /// A current wire variant passes the gate: the Value-first decode with
    /// an empty production retirement table serves the request exactly as
    /// the typed decode did, so the gate is a tax only retired variants pay.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn a_current_wire_variant_passes_the_retired_wire_gate() {
        for retired in RETIRED_WIRE_VARIANTS {
            assert!(
                crate::catalog::WIRE_VARIANTS
                    .iter()
                    .all(|variant| *variant != retired.variant),
                "{}: the retirement table names a variant the wire enum still declares",
                retired.variant
            );
        }
        let root = test_audit_dir("stale-wire-gate-current");
        fs::create_dir_all(&root).expect("create audit test dir");
        let config = test_server_config(&root, &root.join("unused-bundle.json"));
        let log = Arc::new(
            AuditLog::open(
                &config.audit_dir,
                Gid::current().as_raw(),
                true,
                config.audit_retention_days,
            )
            .expect("open audit log"),
        );
        let limiter = Arc::new(tokio::sync::Mutex::new(IpcRateLimiter::new(64)));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let served = Server {
            config: Arc::new(config.clone()),
            audit_log: Arc::clone(&log),
            dispatches: DispatchPool::new(2),
            nested_dispatches: DispatchPool::new(2),
            ipc_rate_limiter: Arc::clone(&limiter),
        };
        use d2b_contracts_broker::broker_wire::{
            BrokerCallerRole, BrokerRequestEnvelope, HelloRequest,
        };
        let (client, server) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("socketpair");
        crate::protocol::send_json_frame(
            client.as_raw_fd(),
            &BrokerRequestEnvelope {
                request: BrokerRequest::Hello(HelloRequest {
                    client_version: d2b_contracts_broker::PROTOCOL_VERSION.to_string(),
                    supported_features: Vec::new(),
                }),
                caller_role: BrokerCallerRole::RootUid { uid: 0 },
                test_peer_uid: None,
                audit_join: None,
            },
        )
        .expect("send a current Hello");
        runtime
            .block_on(async {
                let connection =
                    AsyncSeqpacket::from_owned(server).expect("register accepted socket");
                handle_connection(connection, &served).await
            })
            .expect("the current request is served");
        let response = crate::protocol::recv_json_frame::<BrokerResponse>(client.as_raw_fd())
            .expect("read the reply frame")
            .expect("the reply frame is present");
        assert!(
            matches!(response, BrokerResponse::Hello(_)),
            "a current wire variant passes the gate: {response:?}"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn usb_broker_ipc_validation_rejects_traversal_and_oversized_inputs() {
        use d2b_contracts::types::{BundleOpId, ScopeId};
        use d2b_contracts_broker::broker_wire::{
            ModprobeIfAllowedRequest, UsbipBindFirewallRuleRequest, UsbipBindRequest,
            UsbipProxyReconcileRequest, UsbipUnbindRequest,
        };

        let cases = vec![
            (
                BrokerRequest::UsbipBind(UsbipBindRequest {
                    bundle_usbip_bind_intent_ref: BundleOpId::new(
                        "usbip-bind:env:work:vm:corp-vm:bus:../1-2",
                    ),
                    tracing_span_id: None,
                }),
                "UsbipBind",
                "invalid-bundle-op-id",
            ),
            (
                BrokerRequest::UsbipUnbind(UsbipUnbindRequest {
                    bundle_usbip_bind_intent_ref: BundleOpId::new(
                        "usbip-bind:env:work:vm:corp-vm:bus:1-2/serial",
                    ),
                    preserve_durable_claim: false,
                    tracing_span_id: None,
                }),
                "UsbipUnbind",
                "invalid-bundle-op-id",
            ),
            (
                BrokerRequest::ModprobeIfAllowed(ModprobeIfAllowedRequest {
                    module_name: "../usbip-host".to_owned(),
                    tracing_span_id: None,
                }),
                "ModprobeIfAllowed",
                "invalid-module-name",
            ),
            (
                BrokerRequest::ModprobeIfAllowed(ModprobeIfAllowedRequest {
                    module_name: "x".repeat(MAX_MODULE_NAME_LEN + 1),
                    tracing_span_id: None,
                }),
                "ModprobeIfAllowed",
                "module-name-too-long",
            ),
            (
                BrokerRequest::UsbipProxyReconcile(UsbipProxyReconcileRequest {
                    scope_id: ScopeId::new("../global"),
                    tracing_span_id: None,
                }),
                "UsbipProxyReconcile",
                "invalid-scope-id",
            ),
            (
                BrokerRequest::UsbipBindFirewallRule(UsbipBindFirewallRuleRequest {
                    bundle_usbip_firewall_intent_ref: BundleOpId::new(
                        "usbip-fw:env:work:bus:../1-2",
                    ),
                    tracing_span_id: None,
                }),
                "UsbipBindFirewallRule",
                "invalid-bundle-op-id",
            ),
        ];

        for (request, expected_operation, expected_reason) in cases {
            match validate_broker_request(&request) {
                Err(BrokerError::RequestValidation { operation, reason }) => {
                    assert_eq!(operation, expected_operation);
                    assert_eq!(reason, expected_reason);
                }
                other => {
                    panic!("expected RequestValidation for {expected_operation}, got {other:?}")
                }
            }
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn usb_broker_ipc_validation_is_shape_only_not_authorization() {
        use d2b_contracts::types::{BundleOpId, ScopeId};
        use d2b_contracts_broker::broker_wire::{
            UsbipBindFirewallRuleRequest, UsbipBindRequest, UsbipProxyReconcileRequest,
        };

        let requests = [
            BrokerRequest::UsbipBind(UsbipBindRequest {
                bundle_usbip_bind_intent_ref: BundleOpId::new(
                    "usbip-bind:env:not-in-this-bundle:vm:not-in-this-bundle:bus:1-2.3",
                ),
                tracing_span_id: None,
            }),
            BrokerRequest::UsbipProxyReconcile(UsbipProxyReconcileRequest {
                scope_id: ScopeId::new("env:not-in-this-bundle"),
                tracing_span_id: None,
            }),
            BrokerRequest::UsbipBindFirewallRule(UsbipBindFirewallRuleRequest {
                bundle_usbip_firewall_intent_ref: BundleOpId::new(
                    "usbip-fw:env:not-in-this-bundle:bus:1-2.3",
                ),
                tracing_span_id: None,
            }),
        ];

        for request in requests {
            validate_broker_request(&request).expect(
                "broker IPC validation must remain shape-only; bundle/lifecycle authorization is enforced by daemon classification plus resolver dispatch",
            );
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn tap_create_validation_binds_every_identity_component_before_dispatch() {
        use d2b_contracts::types::{BundleOpId, RoleId, VmId};
        use d2b_contracts_broker::broker_wire::CreatePersistentTapRequest;
        use d2b_contracts_resource::v3::{
            ResourceBundleGenerationId, ResourceGeneration, ResourceUid,
        };

        let zone_uid =
            ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").expect("zone uid");
        let network_uid =
            ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").expect("network uid");
        let attachment_id =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("attachment uid");
        let role_id = RoleId::new("runner-lan");
        let bundle_generation = ResourceBundleGenerationId::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("bundle generation");
        let admitted_interface_names = vec![
            d2b_contracts_resource::v3::derive_network_ifname(
                &zone_uid,
                &network_uid,
                d2b_contracts_resource::v3::NetworkIfRole::LanBridge,
                None,
            )
            .unwrap(),
            d2b_contracts_resource::v3::derive_network_ifname(
                &zone_uid,
                &network_uid,
                d2b_contracts_resource::v3::NetworkIfRole::WorkloadGuestTap,
                Some(&attachment_id),
            )
            .unwrap(),
        ];
        let request = CreatePersistentTapRequest {
            role_id: role_id.clone(),
            vm_id: VmId::new("corp-vm"),
            bundle_tap_intent_ref: BundleOpId::new(
                d2b_core::bundle_resolver::intent_id_network_tap(
                    &zone_uid,
                    &network_uid,
                    &attachment_id,
                    (
                        ResourceGeneration::new(4).unwrap(),
                        ResourceGeneration::new(7).unwrap(),
                    ),
                    &bundle_generation,
                    role_id.as_str(),
                    "corp-vm",
                ),
            ),
            attachment_id: attachment_id.clone(),
            network_generation: ResourceGeneration::new(4).unwrap(),
            attachment_generation: ResourceGeneration::new(7).unwrap(),
            zone_uid: zone_uid.clone(),
            network_uid: network_uid.clone(),
            bundle_generation: bundle_generation.clone(),
            admitted_interface_names: admitted_interface_names.clone(),
            tracing_span_id: None,
        };
        // U12 retired the typed CreatePersistentTap frame: the identity
        // binding the retired arm's validation enforced is exercised
        // directly on the shape validator the family handler's payload
        // validation runs daemon-side after the cut.
        let validate = |request: &CreatePersistentTapRequest| {
            validate_tap_create_provenance(
                &request.bundle_tap_intent_ref,
                &request.vm_id,
                &request.role_id,
                &request.attachment_id,
                &request.zone_uid,
                &request.network_uid,
                request.network_generation,
                request.attachment_generation,
                &request.bundle_generation,
                &request.admitted_interface_names,
            )
        };
        assert!(validate(&request).is_ok());

        let cases = [
            (
                "zone",
                CreatePersistentTapRequest {
                    zone_uid: ResourceUid::parse("423e4567-e89b-42d3-a456-426614174003").unwrap(),
                    ..request.clone()
                },
            ),
            (
                "network",
                CreatePersistentTapRequest {
                    network_uid: ResourceUid::parse("523e4567-e89b-42d3-a456-426614174004")
                        .unwrap(),
                    ..request.clone()
                },
            ),
            (
                "attachment",
                CreatePersistentTapRequest {
                    attachment_id: ResourceUid::parse("623e4567-e89b-42d3-a456-426614174005")
                        .unwrap(),
                    ..request.clone()
                },
            ),
            (
                "network-generation",
                CreatePersistentTapRequest {
                    network_generation: ResourceGeneration::new(5).unwrap(),
                    ..request.clone()
                },
            ),
            (
                "attachment-generation",
                CreatePersistentTapRequest {
                    attachment_generation: ResourceGeneration::new(8).unwrap(),
                    ..request.clone()
                },
            ),
            (
                "bundle-generation",
                CreatePersistentTapRequest {
                    bundle_generation: ResourceBundleGenerationId::parse(
                        "sha256:1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                    )
                    .unwrap(),
                    ..request.clone()
                },
            ),
            (
                "role",
                CreatePersistentTapRequest {
                    role_id: RoleId::new("runner-uplink"),
                    ..request.clone()
                },
            ),
            (
                "admitted-interfaces",
                CreatePersistentTapRequest {
                    admitted_interface_names: Vec::new(),
                    ..request.clone()
                },
            ),
        ];
        for (field, swapped) in cases {
            assert_eq!(
                validate(&swapped),
                Err("network-admission-mismatch"),
                "swapped {field} identity must be refused"
            );
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn tap_fd_validation_requires_the_same_complete_identity() {
        use d2b_contracts::types::{BundleOpId, RoleId, VmId};
        use d2b_contracts_broker::broker_wire::CreateTapFdRequest;
        use d2b_contracts_resource::v3::{
            ResourceBundleGenerationId, ResourceGeneration, ResourceUid,
        };

        let zone_uid =
            ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").expect("zone uid");
        let network_uid =
            ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").expect("network uid");
        let attachment_id =
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("attachment uid");
        let role_id = RoleId::new("runner-lan");
        let network_generation = ResourceGeneration::new(4).unwrap();
        let attachment_generation = ResourceGeneration::new(7).unwrap();
        let bundle_generation = ResourceBundleGenerationId::parse(
            "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        let admitted_interface_names = vec![
            d2b_contracts_resource::v3::derive_network_ifname(
                &zone_uid,
                &network_uid,
                d2b_contracts_resource::v3::NetworkIfRole::LanBridge,
                None,
            )
            .unwrap(),
            d2b_contracts_resource::v3::derive_network_ifname(
                &zone_uid,
                &network_uid,
                d2b_contracts_resource::v3::NetworkIfRole::WorkloadGuestTap,
                Some(&attachment_id),
            )
            .unwrap(),
        ];
        let request = CreateTapFdRequest {
            role_id: role_id.clone(),
            vm_id: VmId::new("corp-vm"),
            bundle_tap_intent_ref: BundleOpId::new(
                d2b_core::bundle_resolver::intent_id_network_tap(
                    &zone_uid,
                    &network_uid,
                    &attachment_id,
                    (network_generation, attachment_generation),
                    &bundle_generation,
                    role_id.as_str(),
                    "corp-vm",
                ),
            ),
            attachment_id,
            network_generation,
            attachment_generation,
            zone_uid,
            network_uid,
            bundle_generation,
            admitted_interface_names,
            tracing_span_id: None,
        };
        // U12 retired the typed CreateTapFd frame: the complete-identity
        // requirement is exercised directly on the shape validator the
        // family handler's payload validation runs daemon-side after the
        // cut.
        assert!(
            validate_tap_create_provenance(
                &request.bundle_tap_intent_ref,
                &request.vm_id,
                &request.role_id,
                &request.attachment_id,
                &request.zone_uid,
                &request.network_uid,
                request.network_generation,
                request.attachment_generation,
                &request.bundle_generation,
                &request.admitted_interface_names,
            )
            .is_ok()
        );
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn usb_broker_public_errors_are_fail_secure() {
        let sensitive = [
            BrokerError::LiveHandler(
                "read USB identity for bus_id=1-2.3 at /sys/bus/usb/devices/1-2.3/idVendor failed: serial ABC"
                    .to_owned(),
            ),
            BrokerError::BundleTampered {
                path: "/var/lib/d2b/current-bundle/manifest.json".to_owned(),
                reason: "mode 0666".to_owned(),
            },
            BrokerError::BundleResolverUnavailable,
            BrokerError::BundleIntentMissing {
                kind: "usbip-bind",
                intent_id: "vm=corp-vm bus=1-2.3".to_owned(),
            },
            BrokerError::UsbipDeviceNotAllowed {
                busid: "1-2.3".to_owned(),
                vendor: 0xabcd,
                product: 0x1234,
            },
            BrokerError::UsbipPolicyMismatch {
                busid: "1-2.3".to_owned(),
                reason: "observed physical topology does not match the declaration",
            },
            BrokerError::PeerCredentialRefused {
                operation: "UsbipBind",
            },
        ];

        for error in sensitive {
            let BrokerResponse::Error(response) = error.into_response() else {
                panic!("expected broker error response");
            };
            let rendered = format!("{} {}", response.message, response.action);
            assert!(!rendered.contains("/sys/"), "{rendered}");
            assert!(
                !rendered.contains("/var/lib/d2b/current-bundle"),
                "{rendered}"
            );
            assert!(!rendered.contains("1-2.3"), "{rendered}");
            assert!(!rendered.contains("abcd"), "{rendered}");
            assert!(!rendered.contains("1234"), "{rendered}");
            assert!(!rendered.contains("ABC"), "{rendered}");
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn usb_audit_identity_keeps_vid_pid_and_redacts_raw_serial() {
        let identity = usb_audit_device_identity(
            (0x1050, 0x0407),
            Some("serial-should-never-serialize"),
            None,
        );

        assert_eq!(identity.vendor_id.as_deref(), Some("1050"));
        assert_eq!(identity.product_id.as_deref(), Some("0407"));
        assert!(identity.serial_observed);
        assert_eq!(identity.serial_correlation, None);
        assert_eq!(identity.previous_serial_correlation, None);

        let encoded = serde_json::to_string(&identity).expect("audit identity serializes");
        assert!(encoded.contains("1050"));
        assert!(encoded.contains("0407"));
        assert!(encoded.contains("serialObserved"));
        assert!(!encoded.contains("serial-should-never-serialize"));
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn usb_audit_identity_uses_deterministic_hmac_serial_correlation() {
        let keyring = UsbAuditSerialHmacKeyring {
            current: UsbAuditSerialHmacKey {
                slot: UsbAuditSerialHmacKeySlot::Current,
                key_id: "audit-key-v1".to_owned(),
                key: b"0123456789abcdef0123456789abcdef".to_vec(),
            },
            previous: None,
        };
        let left = usb_audit_device_identity((0x1050, 0x0407), Some("serial-a"), Some(&keyring))
            .serial_correlation
            .expect("serial correlation present");
        let same = usb_audit_device_identity((0x1050, 0x0407), Some("serial-a"), Some(&keyring))
            .serial_correlation
            .expect("serial correlation present");
        let right = usb_audit_device_identity((0x1050, 0x0407), Some("serial-b"), Some(&keyring))
            .serial_correlation
            .expect("serial correlation present");

        assert_eq!(left.key_id, "audit-key-v1");
        assert_eq!(left, same);
        assert_ne!(left.hmac_sha256, right.hmac_sha256);
        assert_eq!(left.hmac_sha256.len(), 64);
        assert!(
            left.hmac_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn usb_audit_identity_emits_current_and_previous_key_correlations() {
        let keyring = UsbAuditSerialHmacKeyring {
            current: UsbAuditSerialHmacKey {
                slot: UsbAuditSerialHmacKeySlot::Current,
                key_id: "audit-key-current".to_owned(),
                key: b"current-current-current-current-32".to_vec(),
            },
            previous: Some(UsbAuditSerialHmacKey {
                slot: UsbAuditSerialHmacKeySlot::Previous,
                key_id: "audit-key-previous".to_owned(),
                key: b"fedcba9876543210fedcba9876543210".to_vec(),
            }),
        };

        let identity =
            usb_audit_device_identity((0x1050, 0x0407), Some("same-serial"), Some(&keyring));
        let current = identity
            .serial_correlation
            .expect("current correlation present");
        let previous = identity
            .previous_serial_correlation
            .expect("previous correlation present");

        assert_eq!(current.key_id, "audit-key-current");
        assert_eq!(previous.key_id, "audit-key-previous");
        assert_ne!(current.hmac_sha256, previous.hmac_sha256);
        assert_eq!(current.hmac_sha256.len(), 64);
        assert_eq!(previous.hmac_sha256.len(), 64);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn usb_audit_rotation_event_is_scrubbed_and_bounded() {
        let keyring = UsbAuditSerialHmacKeyring {
            current: UsbAuditSerialHmacKey {
                slot: UsbAuditSerialHmacKeySlot::Current,
                key_id: "audit-key-current".to_owned(),
                key: b"current-secret-material-never-log".to_vec(),
            },
            previous: Some(UsbAuditSerialHmacKey {
                slot: UsbAuditSerialHmacKeySlot::Previous,
                key_id: "audit-key-previous".to_owned(),
                key: b"previous-secret-material-never-log".to_vec(),
            }),
        };

        let audit = usb_serial_correlation_key_rotation_audit(&keyring)
            .expect("previous slot opens a rotation window");
        assert_eq!(audit.previous_key_id, "audit-key-previous");
        assert_eq!(audit.current_key_id, "audit-key-current");
        assert_eq!(audit.active_key_count, 2);
        assert_eq!(audit.grace_window_seconds, 30 * 24 * 60 * 60);
        assert_eq!(audit.correlation_version, "d2b-usb-audit-serial-v1");

        let encoded = serde_json::to_value(OperationFields::UsbSerialCorrelationKeyRotate(audit))
            .expect("rotation event serializes");
        let obj = encoded.as_object().expect("object fields");
        let observed: BTreeSet<&str> = obj.keys().map(String::as_str).collect();
        assert_eq!(
            observed,
            [
                "activeKeyCount",
                "correlationVersion",
                "currentKeyId",
                "graceWindowSeconds",
                "previousKeyId",
            ]
            .into_iter()
            .collect()
        );
        let rendered = encoded.to_string();
        assert!(rendered.contains("audit-key-current"));
        assert!(rendered.contains("audit-key-previous"));
        assert!(!rendered.contains("secret-material"));
        assert!(!rendered.contains("key_hex"));
        assert!(!rendered.contains("same-serial"));
        assert!(!rendered.contains("1-2.3"));
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn usbip_bind_audit_failure_rolls_back_backend_bind_and_acl() {
        use d2b_contracts_broker::broker_wire::{BrokerCallerRole, BrokerRequest};

        let root = test_audit_dir("usbip-bind-audit-failure-rollback");
        let bundle = build_test_bundle(&root);
        let config = test_server_config(&root, &bundle.manifest_path);
        let _usb_sysfs_guard = usb_sysfs_test_lock();
        let _ = take_test_usbip_backend_acl_events();
        prepare_test_usb_sysfs_device("1050", "0407", "2.3");

        let log = AuditLog::open_with_write_limit(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
            0,
            false,
        )
        .expect("open rate-limited audit log");
        let backend = FakeDispatchBackend::default();
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let caller_gid = Gid::current().as_raw();
        let intent_id = d2b_core::bundle_resolver::intent_id_usbip_bind("work", "corp-vm", "1-2.3");
        let request =
            BrokerRequest::UsbipBind(d2b_contracts_broker::broker_wire::UsbipBindRequest {
                bundle_usbip_bind_intent_ref: BundleOpId::new(intent_id.as_str()),
                tracing_span_id: None,
            });
        let audit_context = DispatchAuditContext::from_request(&request, 4242, &caller_role)
            .expect("audit context");

        let result = envelope_call_runtime().block_on(dispatch_request_with_backend(
            request,
            1000,
            caller_gid,
            caller_role,
            &audit_context,
            &config,
            &log,
            Some(&bundle.resolver),
            &backend,
        ))
        .expect("privileged audit is never rate limited");
        assert_eq!(
            backend.take_usbip_events(),
            vec![FakeUsbipEvent::Bind { intent_id }],
            "privileged success remains durable despite the unprivileged write limit"
        );
        assert_eq!(
            take_test_usbip_backend_acl_events(),
            vec![TestUsbipBackendAclEvent::Grant { uid: 1002 }],
            "successful bind retains its backend ACL"
        );

        let audit = fs::read_to_string(log.current_daily_path()).expect("read audit log");
        assert!(
            audit.contains(r#""operation":"UsbipBind""#),
            "privileged terminal record must be present: {audit}"
        );
        let _ = result;

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn usbip_bind_acl_grant_failure_releases_lock_after_successful_rollback_unbind() {
        let root = test_audit_dir("usbip-bind-acl-grant-failure-lock-release");
        let bundle = build_test_bundle(&root);
        let intent = test_usbip_intent_with_lock(&root, &bundle);
        crate::ops::usbip_lock::acquire_lock(
            &intent.lock_path,
            &intent.vm_name,
            nix::unistd::Uid::current().as_raw(),
            nix::unistd::Gid::current().as_raw(),
        )
        .expect("seed post-bind lock");
        let backend = FakeDispatchBackend::default();

        let error = envelope_call_runtime().block_on(rollback_usbip_bind_after_acl_grant_failure(
            &backend,
            &intent,
            false,
            BrokerError::LiveHandler("grant failed".to_owned()),
        ));

        assert!(matches!(
            error,
            BrokerError::LiveHandler(ref message) if message == "grant failed"
        ));
        assert_eq!(
            backend.take_usbip_events(),
            vec![FakeUsbipEvent::Unbind {
                intent_id: intent.intent_id.clone()
            }],
            "grant failure rollback must unbind a fresh backend bind"
        );
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&intent.lock_path),
            None,
            "grant failure with successful rollback unbind must release the busid lock"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn usbip_bind_acl_grant_failure_does_not_rollback_same_vm_replay() {
        let root = test_audit_dir("usbip-bind-acl-grant-failure-replay-preserve");
        let bundle = build_test_bundle(&root);
        let intent = test_usbip_intent_with_lock(&root, &bundle);
        crate::ops::usbip_lock::acquire_lock(
            &intent.lock_path,
            &intent.vm_name,
            nix::unistd::Uid::current().as_raw(),
            nix::unistd::Gid::current().as_raw(),
        )
        .expect("seed same-VM replay lock");
        let backend = FakeDispatchBackend::default();

        let error = envelope_call_runtime().block_on(rollback_usbip_bind_after_acl_grant_failure(
            &backend,
            &intent,
            true,
            BrokerError::LiveHandler("grant failed".to_owned()),
        ));

        assert!(matches!(
            error,
            BrokerError::LiveHandler(ref message) if message == "grant failed"
        ));
        assert_eq!(
            backend.take_usbip_events(),
            Vec::new(),
            "same-VM replay grant failure must not unbind an already-active claim"
        );
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&intent.lock_path),
            Some(intent.vm_name.clone()),
            "same-VM replay grant failure must preserve the durable claim"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn usbip_bind_audit_failure_does_not_rollback_same_vm_replay() {
        let _usb_sysfs_guard = usb_sysfs_test_lock();
        let root = test_audit_dir("usbip-bind-audit-failure-replay-preserve");
        let bundle = build_test_bundle(&root);
        let intent = test_usbip_intent_with_lock(&root, &bundle);
        let _ = take_test_usbip_backend_acl_events();
        crate::ops::usbip_lock::acquire_lock(
            &intent.lock_path,
            &intent.vm_name,
            nix::unistd::Uid::current().as_raw(),
            nix::unistd::Gid::current().as_raw(),
        )
        .expect("seed same-VM replay lock");
        envelope_call_runtime().block_on(grant_usbip_backend_device_acl(
            &bundle.resolver,
            &intent,
            (0x1050, 0x0407),
            PathBuf::from("/dev/bus/usb/001/002"),
        ))
        .expect("seed same-VM replay ACL grant");
        let backend = FakeDispatchBackend::default();

        envelope_call_runtime().block_on(rollback_usbip_bind_after_audit_failure(&backend, &bundle.resolver, &intent, true));

        assert_eq!(
            backend.take_usbip_events(),
            Vec::new(),
            "same-VM replay audit failure must not unbind an already-active claim"
        );
        assert_eq!(
            take_test_usbip_backend_acl_events(),
            vec![TestUsbipBackendAclEvent::Grant { uid: 1002 }],
            "same-VM replay audit failure must not revoke an existing backend ACL"
        );
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&intent.lock_path),
            Some(intent.vm_name.clone()),
            "same-VM replay audit failure must preserve the durable claim"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn usbip_proxy_reconcile_skips_absent_locked_device_acl_refresh() {
        let _usb_sysfs_guard = usb_sysfs_test_lock();
        let root = test_audit_dir("usbip-proxy-reconcile-absent-device");
        let bundle = build_test_bundle(&root);
        let intent = test_usbip_intent_with_lock(&root, &bundle);
        let _ = take_test_usbip_backend_acl_events();
        let sysfs_root = crate::test_scratch_root().join("runtime-usb-sysfs-root");
        TEST_USB_SYSFS_ROOT
            .set(sysfs_root.clone())
            .unwrap_or_else(|_| assert_eq!(TEST_USB_SYSFS_ROOT.get(), Some(&sysfs_root)));
        let _ = fs::remove_dir_all(&sysfs_root);
        crate::ops::usbip_lock::acquire_lock(
            &intent.lock_path,
            &intent.vm_name,
            nix::unistd::Uid::current().as_raw(),
            nix::unistd::Gid::current().as_raw(),
        )
        .expect("seed lock for absent device");

        envelope_call_runtime().block_on(reconcile_active_usbip_backend_acls(&bundle.resolver))
            .expect("absent locked hardware should not make proxy reconcile fail");

        assert_eq!(
            take_test_usbip_backend_acl_events(),
            Vec::new(),
            "reconcile must not grant an ACL when the locked USB hardware is absent"
        );
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&intent.lock_path),
            Some(intent.vm_name.clone()),
            "reconcile preserves the durable claim for later device return"
        );

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&sysfs_root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn usbip_unbind_acl_revoke_failure_releases_lock_when_device_is_unbound() {
        let _usb_sysfs_guard = usb_sysfs_test_lock();
        let root = test_audit_dir("usbip-unbind-acl-revoke-failure-release");
        let bundle = build_test_bundle(&root);
        let intent = test_usbip_intent_with_lock(&root, &bundle);
        prepare_test_usb_sysfs_device("1050", "0407", "2.3");
        crate::ops::usbip_lock::acquire_lock(
            &intent.lock_path,
            &intent.vm_name,
            nix::unistd::Uid::current().as_raw(),
            nix::unistd::Gid::current().as_raw(),
        )
        .expect("seed lock");

        let error = envelope_call_runtime().block_on(handle_usbip_acl_revoke_failure_after_unbind(
            &intent,
            false,
            BrokerError::LiveHandler("revoke failed".to_owned()),
        ));

        assert!(matches!(
            error,
            BrokerError::LiveHandler(ref message) if message == "revoke failed"
        ));
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&intent.lock_path),
            None,
            "after successful unbind, an ACL revoke error must not leak the busid lock when the device is no longer bound"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn usbip_unbind_acl_revoke_failure_preserves_lock_when_device_still_bound() {
        use std::os::unix::fs::symlink;

        let _usb_sysfs_guard = usb_sysfs_test_lock();
        let root = test_audit_dir("usbip-unbind-acl-revoke-failure-preserve-bound");
        let bundle = build_test_bundle(&root);
        let intent = test_usbip_intent_with_lock(&root, &bundle);
        let sysfs_root = prepare_test_usb_sysfs_device("1050", "0407", "2.3");
        let driver_root = sysfs_root
            .parent()
            .expect("USB sysfs root has bus parent")
            .join("drivers")
            .join("usbip-host");
        fs::create_dir_all(&driver_root).expect("create usbip-host driver root");
        symlink(&driver_root, sysfs_root.join("1-2.3").join("driver")).expect("driver symlink");
        crate::ops::usbip_lock::acquire_lock(
            &intent.lock_path,
            &intent.vm_name,
            nix::unistd::Uid::current().as_raw(),
            nix::unistd::Gid::current().as_raw(),
        )
        .expect("seed lock");

        let error = envelope_call_runtime().block_on(handle_usbip_acl_revoke_failure_after_unbind(
            &intent,
            false,
            BrokerError::LiveHandler("revoke failed".to_owned()),
        ));

        assert!(matches!(
            error,
            BrokerError::LiveHandler(ref message) if message == "revoke failed"
        ));
        assert_eq!(
            crate::ops::usbip_lock::peek_owner(&intent.lock_path),
            Some(intent.vm_name.clone()),
            "if post-unbind inspection still sees usbip-host, preserve the lock for manual recovery"
        );

        let _ = fs::remove_dir_all(&root);
    }

    // ------------------------------------------------------------------
    // retry_usbip_backend_acl_grant unit tests
    // ------------------------------------------------------------------

    /// Helper that tracks grant/revoke calls for retry function unit tests.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[derive(Default, Debug)]
    struct AclCallLog {
        grants: Vec<PathBuf>,
        revokes: Vec<PathBuf>,
        sleeps: usize,
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn retry_acl_grant_succeeds_immediately_when_node_is_stable() {
        use std::cell::RefCell;
        let log = RefCell::new(AclCallLog::default());
        let node = PathBuf::from("/dev/bus/usb/001/007");
        let result = envelope_call_runtime().block_on(retry_usbip_backend_acl_grant(
            1002,
            || Box::pin(async { Ok(node.clone()) }),
            |path, _uid| {
                log.borrow_mut().grants.push(path.to_owned());
                Box::pin(async { Ok(()) })
            },
            |path, _uid| {
                log.borrow_mut().revokes.push(path.to_owned());
                Box::pin(async { Ok(()) })
            },
            || {
                log.borrow_mut().sleeps += 1;
                Box::pin(async {})
            },
        ));
        assert!(result.is_ok(), "stable node must succeed immediately");
        let log = log.into_inner();
        assert_eq!(log.grants, vec![node], "exactly one grant");
        assert!(log.revokes.is_empty(), "no revoke on clean success");
        assert_eq!(log.sleeps, 0, "no sleep on first-attempt success");
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn retry_acl_grant_converges_after_transient_node_change() {
        // Simulate a device re-enumeration: first verify returns node A,
        // post-grant verify returns node B (different /dev node, same
        // VID/PID identity from the caller's perspective). The retry then
        // observes stable node B and succeeds.
        use std::cell::RefCell;
        let node_a = PathBuf::from("/dev/bus/usb/001/009");
        let node_b = PathBuf::from("/dev/bus/usb/001/010");
        let log = RefCell::new(AclCallLog::default());
        // verify_device_node: first call → A, second call (post-grant re-check) → B,
        // third call (retry pre-grant) → B, fourth call (post-grant re-check) → B.
        let call_count = RefCell::new(0usize);
        let result = envelope_call_runtime().block_on(retry_usbip_backend_acl_grant(
            1002,
            || {
                let n = *call_count.borrow();
                *call_count.borrow_mut() += 1;
                let outcome = match n {
                    0 => Ok(node_a.clone()), // first pre-grant verify → A
                    1 => Ok(node_b.clone()), // post-grant verify → B (changed!)
                    _ => Ok(node_b.clone()), // subsequent calls → B (stable)
                };
                Box::pin(async move { outcome })
            },
            |path, _uid| {
                log.borrow_mut().grants.push(path.to_owned());
                Box::pin(async { Ok(()) })
            },
            |path, _uid| {
                log.borrow_mut().revokes.push(path.to_owned());
                Box::pin(async { Ok(()) })
            },
            || {
                log.borrow_mut().sleeps += 1;
                Box::pin(async {})
            },
        ));
        assert!(result.is_ok(), "must converge after transient node change");
        let log = log.into_inner();
        // First attempt: grant A, post-grant verify sees B → revoke A, sleep.
        // Second attempt: grant B, post-grant verify sees B → success.
        assert!(
            log.grants.contains(&node_a),
            "first grant is for the initial node A"
        );
        assert!(
            log.grants.contains(&node_b),
            "retry grant is for the stable node B"
        );
        assert_eq!(
            log.revokes,
            vec![node_a.clone()],
            "must revoke old grant on A when re-verify sees B"
        );
        assert_eq!(log.sleeps, 1, "exactly one sleep between attempts");
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn retry_acl_grant_fails_when_verify_permanently_fails() {
        use std::cell::RefCell;
        let log = RefCell::new(AclCallLog::default());
        let err_msg = "identity changed: VID/PID mismatch";
        let result = envelope_call_runtime().block_on(retry_usbip_backend_acl_grant(
            1002,
            || {
                let error = BrokerError::LiveHandler(err_msg.to_owned());
                Box::pin(async move { Err(error) })
            },
            |path, _uid| {
                log.borrow_mut().grants.push(path.to_owned());
                Box::pin(async { Ok(()) })
            },
            |path, _uid| {
                log.borrow_mut().revokes.push(path.to_owned());
                Box::pin(async { Ok(()) })
            },
            || {
                log.borrow_mut().sleeps += 1;
                Box::pin(async {})
            },
        ));
        assert!(result.is_err(), "must fail when verify never succeeds");
        let log = log.into_inner();
        assert!(log.grants.is_empty(), "no grants when verify always fails");
        assert!(log.revokes.is_empty(), "no revokes when grant never ran");
        assert_eq!(
            log.sleeps, USBIP_BACKEND_ACL_GRANT_ATTEMPTS,
            "must sleep once per attempt"
        );
        match result.unwrap_err() {
            BrokerError::LiveHandler(msg) => {
                assert_eq!(msg, err_msg, "last error from verify must be propagated")
            }
            other => panic!("expected LiveHandler, got {other:?}"),
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn retry_acl_grant_revokes_before_every_retry_on_post_grant_verify_failure() {
        // Grant succeeds, but post-grant verify always returns a different
        // node. The function must revoke on every attempt before giving up.
        use std::cell::RefCell;
        let base = "/dev/bus/usb/001/";
        let log = RefCell::new(AclCallLog::default());
        let counter = RefCell::new(0u32);
        let result = envelope_call_runtime().block_on(retry_usbip_backend_acl_grant(
            1002,
            || {
                let n = *counter.borrow();
                *counter.borrow_mut() += 1;
                // Each call returns a unique node so post-grant verify
                // always sees a "changed" path.
                let outcome = Ok(PathBuf::from(format!("{base}{n:03}")));
                Box::pin(async move { outcome })
            },
            |path, _uid| {
                log.borrow_mut().grants.push(path.to_owned());
                Box::pin(async { Ok(()) })
            },
            |path, _uid| {
                log.borrow_mut().revokes.push(path.to_owned());
                Box::pin(async { Ok(()) })
            },
            || {
                log.borrow_mut().sleeps += 1;
                Box::pin(async {})
            },
        ));
        assert!(result.is_err(), "must fail when node never stabilizes");
        let log = log.into_inner();
        assert_eq!(
            log.grants.len(),
            USBIP_BACKEND_ACL_GRANT_ATTEMPTS,
            "one grant attempt per retry round"
        );
        assert_eq!(
            log.revokes.len(),
            USBIP_BACKEND_ACL_GRANT_ATTEMPTS,
            "must revoke once per grant when post-grant verify always disagrees"
        );
        // Every granted node must eventually be revoked.
        for granted in &log.grants {
            assert!(
                log.revokes.contains(granted),
                "granted node {granted:?} must be revoked"
            );
        }
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn retry_acl_grant_tolerates_benign_enoent_during_revoke() {
        // After a node change the old node may already be gone (kernel
        // removes /dev/bus/usb/B/D during re-enumeration). Revoke errors
        // must not prevent the retry from proceeding or propagating the
        // real error.
        use std::cell::RefCell;
        let node_a = PathBuf::from("/dev/bus/usb/001/021");
        let node_b = PathBuf::from("/dev/bus/usb/001/022");
        let call_count = RefCell::new(0usize);
        let log = RefCell::new(AclCallLog::default());
        // Revoke always fails with "not found" - benign during re-enum.
        let result = envelope_call_runtime().block_on(retry_usbip_backend_acl_grant(
            1002,
            || {
                let n = *call_count.borrow();
                *call_count.borrow_mut() += 1;
                let outcome = match n {
                    0 => Ok(node_a.clone()),
                    1 => Ok(node_b.clone()), // post-grant re-check sees B
                    _ => Ok(node_b.clone()), // stable from here on
                };
                Box::pin(async move { outcome })
            },
            |path, _uid| {
                log.borrow_mut().grants.push(path.to_owned());
                Box::pin(async { Ok(()) })
            },
            |path, _uid| {
                log.borrow_mut().revokes.push(path.to_owned());
                // Simulate ENOENT: old node already removed.
                Box::pin(async {
                    Err(BrokerError::LiveHandler(
                        "No such file or directory".to_owned(),
                    ))
                })
            },
            || {
                log.borrow_mut().sleeps += 1;
                Box::pin(async {})
            },
        ));
        // The revoke error must not surface as the final result; the
        // retry on node B must succeed.
        assert!(
            result.is_ok(),
            "benign revoke ENOENT must not abort the retry loop"
        );
        let log = log.into_inner();
        assert!(
            log.revokes.contains(&node_a),
            "attempted revoke on the stale node A"
        );
        assert!(log.grants.contains(&node_b), "retry grant on stable node B");
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    fn usb_audit_rotation_audit_dedupe_suppresses_repeats_and_allows_retry() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let audit = UsbSerialCorrelationKeyRotationAudit {
            previous_key_id: format!("audit-key-previous-dedupe-{unique}"),
            current_key_id: format!("audit-key-current-dedupe-{unique}"),
            active_key_count: 2,
            grace_window_seconds: USB_AUDIT_SERIAL_HMAC_PREVIOUS_KEY_GRACE_WINDOW_SECONDS,
            correlation_version: USB_AUDIT_SERIAL_CORRELATION_VERSION.to_owned(),
        };

        let dedupe_key = mark_usb_audit_serial_hmac_rotation_audit_logged(&audit)
            .expect("first rotation audit for key pair is allowed");
        assert!(mark_usb_audit_serial_hmac_rotation_audit_logged(&audit).is_none());

        unmark_usb_audit_serial_hmac_rotation_audit_logged(&dedupe_key);
        let retry_dedupe_key = mark_usb_audit_serial_hmac_rotation_audit_logged(&audit)
            .expect("failed audit write can clear the marker and retry");
        assert_eq!(retry_dedupe_key, dedupe_key);
        unmark_usb_audit_serial_hmac_rotation_audit_logged(&retry_dedupe_key);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn usbip_bind_with_previous_serial_hmac_key_emits_one_rotation_audit_record_per_key_pair() {
        use d2b_contracts::types::TracingSpanId;
        use d2b_contracts_broker::broker_wire::{BrokerCallerRole, BrokerRequest};
        use std::os::unix::fs::PermissionsExt;

        let root = test_audit_dir("usb-serial-hmac-rotation-audit");
        let bundle = build_test_bundle(&root);
        let config = test_server_config(&root, &bundle.manifest_path);
        let key_dir = usb_audit_serial_hmac_key_dir(&config.state_dir);
        fs::create_dir_all(&key_dir).expect("create key dir");

        let current_secret = b"current-secret-material-12345678";
        let previous_secret = b"previous-secret-material-1234567";
        assert_eq!(current_secret.len(), USB_AUDIT_SERIAL_HMAC_KEY_BYTES);
        assert_eq!(previous_secret.len(), USB_AUDIT_SERIAL_HMAC_KEY_BYTES);
        let _usb_sysfs_guard = usb_sysfs_test_lock();

        let write_key = |file_name: &str, key: &UsbAuditSerialHmacKey| {
            let path = key_dir.join(file_name);
            fs::write(&path, render_usb_audit_serial_hmac_key(key)).expect("write key");
            let mut perms = fs::metadata(&path).expect("stat key").permissions();
            perms.set_mode(0o400);
            fs::set_permissions(&path, perms).expect("chmod key");
        };
        write_key(
            USB_AUDIT_SERIAL_HMAC_CURRENT_KEY_FILE,
            &UsbAuditSerialHmacKey {
                slot: UsbAuditSerialHmacKeySlot::Current,
                key_id: "audit-key-current".to_owned(),
                key: current_secret.to_vec(),
            },
        );
        write_key(
            USB_AUDIT_SERIAL_HMAC_PREVIOUS_KEY_FILE,
            &UsbAuditSerialHmacKey {
                slot: UsbAuditSerialHmacKeySlot::Previous,
                key_id: "audit-key-previous".to_owned(),
                key: previous_secret.to_vec(),
            },
        );

        let sysfs_root = prepare_test_usb_sysfs_device("1050", "0407", "2.3");
        fs::write(
            sysfs_root.join("1-2.3").join("serial"),
            "raw-usb-serial-never-log\n",
        )
        .expect("write fake serial");

        let (log, capture) = AuditLog::open_capturing(
            &config.audit_dir,
            Gid::current().as_raw(),
            true,
            config.audit_retention_days,
        )
        .expect("open capturing audit log");
        let backend = FakeDispatchBackend::default();
        let caller_role = BrokerCallerRole::AdminUid { uid: 1000 };
        let caller_gid = Gid::current().as_raw();
        let make_request = |span_id: &str| {
            BrokerRequest::UsbipBind(d2b_contracts_broker::broker_wire::UsbipBindRequest {
                bundle_usbip_bind_intent_ref: BundleOpId::new(
                    d2b_core::bundle_resolver::intent_id_usbip_bind("work", "corp-vm", "1-2.3"),
                ),
                tracing_span_id: Some(TracingSpanId::new(span_id)),
            })
        };
        let request = make_request("span-usb-rotate");
        let audit_context = DispatchAuditContext::from_request(&request, 4242, &caller_role)
            .expect("audit context");

        let dispatch = envelope_call_runtime().block_on(dispatch_request_with_backend(
            request,
            1000,
            caller_gid,
            caller_role.clone(),
            &audit_context,
            &config,
            &log,
            Some(&bundle.resolver),
            &backend,
        ))
        .expect("dispatch succeeds");
        match dispatch.response {
            BrokerResponse::Ack(response) => {
                assert!(response.accepted);
                assert_eq!(response.operation, "UsbipBind");
            }
            other => panic!("expected UsbipBind ack, got {other:?}"),
        }

        let records = capture.lock().expect("capture lock").clone();
        assert_eq!(records.len(), 2);
        let rotation_record = records
            .iter()
            .find(|record| record.operation == "UsbSerialCorrelationKeyRotate")
            .expect("rotation audit record");
        assert!(d2b_contracts_resource::v3::is_canonical_digest(
            &rotation_record.public_operation_id
        ));
        assert_eq!(rotation_record.subject_id, "usb-audit-serial-hmac");
        assert_eq!(rotation_record.scope_id, "host");
        assert_eq!(rotation_record.verb, "UsbSerialCorrelationKeyRotate");
        assert_eq!(
            rotation_record.request_fields,
            serde_json::json!({
                "detectedDuring": "UsbipBind",
                "tracingSpanIdPresent": true,
            })
        );
        assert_eq!(
            rotation_record.tracing_span_id.as_deref(),
            Some("span-usb-rotate")
        );
        let rotation_fields = OperationFields::from_operation_value(
            "UsbSerialCorrelationKeyRotate",
            rotation_record
                .operation_fields
                .clone()
                .expect("rotation fields"),
        )
        .expect("parse rotation fields");
        assert_eq!(
            rotation_fields,
            OperationFields::UsbSerialCorrelationKeyRotate(UsbSerialCorrelationKeyRotationAudit {
                previous_key_id: "audit-key-previous".to_owned(),
                current_key_id: "audit-key-current".to_owned(),
                active_key_count: 2,
                grace_window_seconds: 30 * 24 * 60 * 60,
                correlation_version: "d2b-usb-audit-serial-v1".to_owned(),
            })
        );

        let bind_record = records
            .iter()
            .find(|record| record.operation == "UsbipBind")
            .expect("bind audit record");
        let bind_fields = OperationFields::from_operation_value(
            "UsbipBind",
            bind_record.operation_fields.clone().expect("bind fields"),
        )
        .expect("parse bind fields");
        let OperationFields::UsbipBind {
            device_identity: Some(device_identity),
            ..
        } = bind_fields
        else {
            panic!("expected UsbipBind device identity");
        };
        assert!(device_identity.serial_observed);
        assert!(device_identity.serial_correlation.is_some());
        assert!(device_identity.previous_serial_correlation.is_some());

        let rotation_json = serde_json::to_string(rotation_record).expect("serialize rotation");
        assert!(!rotation_json.contains("raw-usb-serial-never-log"));
        assert!(!rotation_json.contains("1-2.3"));
        assert!(!rotation_json.contains("key_hex"));

        let exported = log.export_lines(None, None).expect("export audit lines");
        let rendered = exported.join("\n");
        assert!(rendered.contains("UsbSerialCorrelationKeyRotate"));
        assert!(!rendered.contains("raw-usb-serial-never-log"));
        assert!(!rendered.contains("current-secret-material-12345678"));
        assert!(!rendered.contains("previous-secret-material-1234567"));
        assert!(!rendered.contains(&lower_hex(current_secret)));
        assert!(!rendered.contains(&lower_hex(previous_secret)));
        assert!(!rendered.contains("key_hex"));

        let repeat_request = make_request("span-usb-rotate-repeat");
        let repeat_audit_context =
            DispatchAuditContext::from_request(&repeat_request, 4242, &caller_role)
                .expect("repeat audit context");
        envelope_call_runtime().block_on(dispatch_request_with_backend(
            repeat_request,
            1000,
            caller_gid,
            caller_role,
            &repeat_audit_context,
            &config,
            &log,
            Some(&bundle.resolver),
            &backend,
        ))
        .expect("repeat dispatch succeeds");

        let records_after_repeat = capture.lock().expect("capture lock").clone();
        assert_eq!(records_after_repeat.len(), 3);
        assert_eq!(
            records_after_repeat
                .iter()
                .filter(|record| record.operation == "UsbSerialCorrelationKeyRotate")
                .count(),
            1
        );
        assert_eq!(
            records_after_repeat
                .iter()
                .filter(|record| record.operation == "UsbipBind")
                .count(),
            2
        );
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn usb_audit_serial_hmac_keyring_creates_root_only_current_key_and_reads_previous() {
        use std::os::unix::fs::PermissionsExt;

        let state_dir = test_audit_dir("usb-audit-hmac-keyring");
        tokio::fs::create_dir_all(&state_dir).await.expect("create state dir");
        let key_dir = usb_audit_serial_hmac_key_dir(&state_dir);
        tokio::fs::create_dir_all(&key_dir).await.expect("create key dir");
        let previous = UsbAuditSerialHmacKey {
            slot: UsbAuditSerialHmacKeySlot::Previous,
            key_id: "audit-key-previous".to_owned(),
            key: b"fedcba9876543210fedcba9876543210".to_vec(),
        };
        let previous_path = key_dir.join(USB_AUDIT_SERIAL_HMAC_PREVIOUS_KEY_FILE);
        tokio::fs::write(&previous_path, render_usb_audit_serial_hmac_key(&previous))
            .await
            .expect("write previous key");
        let mut perms = tokio::fs::metadata(&previous_path)
            .await
            .expect("stat previous key")
            .permissions();
        perms.set_mode(0o400);
        tokio::fs::set_permissions(&previous_path, perms).await.expect("chmod previous key");

        let keyring = usb_audit_serial_hmac_keyring(&state_dir, true).await.expect("keyring loads");
        assert_eq!(keyring.current.slot, UsbAuditSerialHmacKeySlot::Current);
        assert!(keyring.current.key_id.starts_with("usb-audit-"));
        assert_eq!(keyring.current.key.len(), USB_AUDIT_SERIAL_HMAC_KEY_BYTES);
        assert_eq!(keyring.previous, Some(previous));

        let current_path = key_dir.join(USB_AUDIT_SERIAL_HMAC_CURRENT_KEY_FILE);
        let mode = tokio::fs::metadata(&current_path)
            .await
            .expect("stat current key")
            .mode()
            & 0o777;
        assert_eq!(mode, 0o400);

        let first_current = keyring.current.clone();
        let loaded_again =
            usb_audit_serial_hmac_keyring(&state_dir, true).await.expect("keyring loads again");
        assert_eq!(loaded_again.current, first_current);
    }

    #[test]
    fn broker_ipc_rate_limiter_refuses_excess_uid_requests() {
        let mut limiter = IpcRateLimiter::new(2);
        assert!(limiter.check(IpcRatePool::Daemon, 1000, "d2b-admin", "UsbipBind"));
        assert!(limiter.check(IpcRatePool::Daemon, 1000, "d2b-admin", "UsbipBind"));
        assert!(!limiter.check(IpcRatePool::Daemon, 1000, "d2b-admin", "UsbipBind"));
        assert!(
            limiter.check(IpcRatePool::Daemon, 1001, "d2b-admin", "UsbipBind"),
            "other UIDs have independent buckets"
        );
    }

    #[test]
    fn broker_ipc_rate_limiter_keys_on_stable_role_and_operation() {
        let mut limiter = IpcRateLimiter::new(1);
        assert!(limiter.check(IpcRatePool::Daemon, 1000, "d2b-admin", "UsbipBind"));
        assert!(
            !limiter.check(IpcRatePool::Daemon, 1000, "d2b-admin", "UsbipBind"),
            "same stable uid/role/op bucket must be limited"
        );
        assert!(
            limiter.check(IpcRatePool::Daemon, 1000, "d2b-admin", "UsbipUnbind"),
            "distinct USB operations have independent stable buckets"
        );
        assert!(
            limiter.check(IpcRatePool::Daemon, 1000, "launcher-uid", "UsbipBind"),
            "forwarded caller roles have independent stable buckets"
        );
    }

    #[test]
    fn broker_ipc_rate_limiter_caps_bucket_growth_fail_closed() {
        let now = Instant::now();
        let mut limiter = IpcRateLimiter::with_limits(64, 2);

        assert!(limiter.check_at(IpcRatePool::Daemon, 1000, "d2b-admin", "UsbipBind", now));
        assert!(limiter.check_at(IpcRatePool::Daemon, 1001, "d2b-admin", "UsbipBind", now));
        assert_eq!(limiter.daemon_buckets.len(), 2);
        for uid in 1002..1100 {
            assert!(
                !limiter.check_at(IpcRatePool::Daemon, uid, "d2b-admin", "UsbipBind", now),
                "new UID buckets must fail closed once the cap is reached"
            );
        }
        assert_eq!(
            limiter.daemon_buckets.len(),
            2,
            "refused peers must not allocate unbounded buckets"
        );
        assert!(
            limiter.check_at(IpcRatePool::Daemon, 1000, "d2b-admin", "UsbipBind", now),
            "existing users keep their bucket while new buckets are refused"
        );
    }

    #[test]
    fn broker_ipc_rate_limiter_direct_peer_flood_preserves_daemon_capacity() {
        let now = Instant::now();
        let mut limiter = IpcRateLimiter::with_limits(64, 2);

        assert!(limiter.check_at(
            IpcRatePool::Direct,
            1000,
            "direct-broker-peer",
            "direct-broker-connect",
            now
        ));
        assert!(limiter.check_at(
            IpcRatePool::Direct,
            1001,
            "direct-broker-peer",
            "direct-broker-connect",
            now
        ));
        assert!(
            !limiter.check_at(
                IpcRatePool::Direct,
                1002,
                "direct-broker-peer",
                "direct-broker-connect",
                now
            ),
            "direct peers should fail closed once their own pool is full"
        );
        assert_eq!(limiter.direct_buckets.len(), 2);

        assert!(
            limiter.check_at(IpcRatePool::Daemon, 4242, "d2b-admin", "UsbipBind", now),
            "daemon-forwarded requests must retain reserved bucket capacity"
        );
        assert_eq!(limiter.daemon_buckets.len(), 1);
        assert_eq!(limiter.direct_buckets.len(), 2);
    }

    #[test]
    fn broker_ipc_rate_limiter_evicts_expired_buckets_before_allocating() {
        let now = Instant::now();
        let after_window = now + IPC_RATE_LIMIT_WINDOW + Duration::from_millis(1);
        let mut limiter = IpcRateLimiter::with_limits(64, 2);

        assert!(limiter.check_at(IpcRatePool::Daemon, 1000, "d2b-admin", "UsbipBind", now));
        assert!(limiter.check_at(IpcRatePool::Daemon, 1001, "d2b-admin", "UsbipBind", now));
        assert_eq!(limiter.daemon_buckets.len(), 2);

        assert!(
            limiter.check_at(
                IpcRatePool::Daemon,
                1002,
                "d2b-admin",
                "UsbipBind",
                after_window
            ),
            "expired buckets should be reclaimed for later callers"
        );
        assert_eq!(
            limiter.daemon_buckets.len(),
            1,
            "expired UID buckets must not accumulate after eviction"
        );
        assert!(
            limiter.daemon_buckets.contains_key(&IpcRateKey {
                uid: 1002,
                role: "d2b-admin",
                operation: "UsbipBind",
            }),
            "only the fresh caller should remain after eviction"
        );
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn usbip_static_busid_owner_is_global_authority() {
        let root = test_audit_dir("usbip-static-owner");
        let bundle = build_test_bundle(&root);

        assert_eq!(
            static_usbip_busid_owner(&bundle.resolver, "1-2.3").as_deref(),
            Some("corp-vm")
        );
        assert!(
            find_wildcard_usbip_bind_intent_for(&bundle.resolver, "other-vm", "1-2.3").is_none(),
            "wildcard fallback must not claim a statically assigned busid"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn broker_error_audit_records_error_kind_and_message_for_errored_variants() {
        let audit_dir = test_audit_dir("broker-error-audit");
        fs::create_dir_all(&audit_dir).expect("create audit dir");
        let cases = vec![
            AuditCase {
                error: BrokerError::BundleResolverUnavailable,
                operation: "RunHostInstall",
                target_id: "operation".to_owned(),
                decision: "bundle-resolver-unavailable",
                error_kind: "Broker.BundleResolverUnavailable",
                error_message: "Broker started without a loadable bundle at ServerConfig.bundle_path. Bundle-dependent real-wire ops cannot resolve their BundleOpId refs.".to_owned(),
            },
            AuditCase {
                error: BrokerError::BundleIntentMissing {
                    kind: "installer",
                    intent_id: "installer:missing".to_owned(),
                },
                operation: "RunHostInstall",
                target_id: "installer:missing".to_owned(),
                decision: "bundle-intent-missing",
                error_kind: "Broker.BundleIntentMissing",
                error_message:
                    "no installer intent in the trusted bundle for opaque id `installer:missing`"
                        .to_owned(),
            },
            AuditCase {
                error: BrokerError::LiveHandler(
                    "systemctl enable d2bd failed: Unit d2bd.service does not exist"
                        .to_owned(),
                ),
                operation: "RunHostInstall",
                target_id: "operation".to_owned(),
                decision: "live-handler-error",
                error_kind: "Broker.LiveHandlerFailed",
                error_message:
                    "systemctl enable d2bd failed: Unit d2bd.service does not exist"
                        .to_owned(),
            },
            AuditCase {
                error: BrokerError::UsbipLockConflict {
                    busid: "1-2.3".to_owned(),
                    owner: "other-vm".to_owned(),
                },
                operation: "UsbipBind",
                target_id: "1-2.3".to_owned(),
                decision: "usbip-lock-conflict",
                error_kind: "Broker.UsbipLockConflict",
                error_message: "UsbipBind refused: busid 1-2.3 is already claimed by other-vm"
                    .to_owned(),
            },
            AuditCase {
                error: BrokerError::UsbipDeviceAbsent {
                    busid: "1-2.4".to_owned(),
                },
                operation: "UsbipBind",
                target_id: "1-2.4".to_owned(),
                decision: "usbip-device-absent",
                error_kind: "Broker.UsbipDeviceAbsent",
                error_message: "UsbipBind refused: USB device 1-2.4 is not present in sysfs"
                    .to_owned(),
            },
            AuditCase {
                error: BrokerError::Protocol("read request frame failed: unexpected EOF".to_owned()),
                operation: "RunHostInstall",
                target_id: "operation".to_owned(),
                decision: "protocol-error",
                error_kind: "Broker.Protocol",
                error_message: "read request frame failed: unexpected EOF".to_owned(),
            },
        ];

        let caller_gid = Gid::current().as_raw();
        let exported = {
            fs::create_dir_all(&audit_dir).expect("create audit dir");
            let log = AuditLog::open(&audit_dir, caller_gid, true, 14).expect("open audit log");
            for case in &cases {
                let audit_context = DispatchAuditContext {
                    peer_pid: 4242,
                    peer_role: CallerRole::AdminUid { uid: 1000 }.for_display().to_owned(),
                    verb: case.operation.to_owned(),
                    request_fields: Value::Object(Default::default()),
                    started_at: Instant::now(),
                    audit_join: None,
                };
                #[cfg(not(feature = "layer1-bootstrap"))]
                case.error
                    .audit(
                        &log,
                        1000,
                        caller_gid,
                        &CallerRole::AdminUid { uid: 1000 },
                        &audit_context,
                        None,
                        case.operation,
                        &case.target_id,
                    )
                    .expect("audit error");
                #[cfg(feature = "layer1-bootstrap")]
                case.error
                    .audit(
                        &log,
                        1000,
                        caller_gid,
                        &CallerRole::AdminUid { uid: 1000 },
                        &audit_context,
                        case.operation,
                        &case.target_id,
                    )
                    .expect("audit error");
            }
            log.export_lines(None, None).expect("export audit lines")
        };

        assert_eq!(exported.len(), cases.len());
        for (case, line) in cases.iter().zip(exported.iter()) {
            let value: Value = serde_json::from_str(line).expect("parse audit line");
            assert_eq!(
                value.get("op").and_then(Value::as_str),
                Some(case.operation)
            );
            assert_eq!(value.get("caller_uid").and_then(Value::as_u64), Some(1000));
            assert_eq!(
                value.get("caller_gid").and_then(Value::as_u64),
                Some(u64::from(caller_gid))
            );
            assert_eq!(
                value.get("disposition").and_then(Value::as_str),
                Some(case.decision)
            );
            assert_eq!(
                value.get("opaque_target_id").and_then(Value::as_str),
                Some(case.target_id.as_str())
            );
            assert_eq!(
                value.get("outcome").and_then(Value::as_str),
                Some("errored")
            );
            assert_eq!(
                value.get("error_kind").and_then(Value::as_str),
                Some(case.error_kind)
            );
            let error_message = value
                .get("error_message")
                .and_then(Value::as_str)
                .expect("redacted error message");
            assert!(error_message.starts_with("sha256:"));
            assert_ne!(error_message, case.error_message);
        }

        let _ = fs::remove_dir_all(&audit_dir);
    }

    #[cfg(not(feature = "layer1-bootstrap"))]
    mod reap_tests {
        use super::*;
        use d2b_contracts_broker::broker_wire::{
            ChildExitKind, ChildExitStatus, ChildReapedNotification,
        };
        use nix::sys::signal::{Signal, kill};
        use nix::unistd::Pid;
        use std::process::Command;
        use std::time::{Duration, Instant};

        // One shared serialization guard for every test that mutates the
        // process-global broker registries: reap tests and spawn-process
        // tests use the same lock, so a reap test's cleanup can never
        // clobber a spawn test's mid-test registration (and vice versa).
        type ReapTestGuard = RegistryTestGuard;

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn start_test_reaper(test_name: &str) -> tokio::runtime::Runtime {
            let audit_dir = test_audit_dir(test_name);
            fs::create_dir_all(&audit_dir).expect("create reap audit dir");
            let audit_log = AuditLog::open(&audit_dir, Gid::current().as_raw(), true, 0)
                .expect("open reap audit log");
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("d2b-broker-test-reaper")
                .enable_all()
                .build()
                .expect("test reaper runtime");
            start_sigchld_reaper(&rt, Arc::new(audit_log));
            // No readiness sleep is needed: `start_sigchld_reaper` installs
            // the SIGCHLD handler synchronously, and tokio's signal channel
            // buffers any signal that arrives before the reaper task's
            // `recv` is polled. Every child below is spawned after this
            // returns, so no SIGCHLD can precede the handler.
            rt
        }

        /// Run one closure against the runner metadata registry with the
        /// non-blocking `try_lock` (plan U8: the registry is a tokio Mutex
        /// reached from sync test bodies), retrying a Busy collision.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn with_runner_metadata_mut<R>(
            f: impl FnOnce(&mut std::collections::HashMap<String, RunnerRegistration>) -> R,
        ) -> R {
            loop {
                if let Ok(mut registry) = runner_metadata_registry().try_lock() {
                    return f(&mut registry);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        /// Park on the reap-notification wake until `predicate` holds over the
        /// reap buffer, or `guard` elapses. The wake is registered before
        /// the buffer is checked, so a push racing the check cannot be
        /// missed. `guard` is a last-resort hang breaker only: a healthy
        /// reaper wakes the wait the moment the notification lands, so
        /// wall-clock load cannot trip it - the wait lasts however long the
        /// reaper actually takes.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn wait_for_reap_buffer(
            runtime: &tokio::runtime::Runtime,
            guard: Duration,
            mut predicate: impl FnMut(
                &std::collections::VecDeque<
                    d2b_contracts_broker::broker_wire::ChildReapedNotification,
                >,
            ) -> bool,
        ) -> bool {
            runtime.block_on(async {
                let deadline = tokio::time::Instant::now() + guard;
                loop {
                    let notified = CHILD_REAP_NOTIFY.notified();
                    // Non-blocking try-lock (plan U8): a Busy collision
                    // (the reap task's push) skips this check and parks on
                    // the wake the push fires.
                    if let Ok(buffer) = child_reap_buffer().try_lock()
                        && predicate(&buffer)
                    {
                        return true;
                    }
                    let remaining =
                        deadline.saturating_duration_since(tokio::time::Instant::now());
                    if remaining.is_zero() {
                        return false;
                    }
                    // The notify is the fast path; the short timeout only
                    // re-checks after a missed wake (a Busy collision or a
                    // drain by a parallel test), never as the primary wait.
                    tokio::time::timeout(remaining.min(Duration::from_millis(250)), notified)
                        .await
                        .ok();
                }
            })
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn wait_for_notification(
            runner_id: &str,
            runtime: &tokio::runtime::Runtime,
            guard: Duration,
        ) -> Option<ChildReapedNotification> {
            if !wait_for_reap_buffer(runtime, guard, |buffer| {
                buffer.iter().any(|n| n.runner_id == runner_id)
            }) {
                return None;
            }
            loop {
                if let Ok(buffer) = child_reap_buffer().try_lock() {
                    return buffer
                        .iter()
                        .find(|n| n.runner_id == runner_id)
                        .cloned();
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }

        fn observe_request() -> d2b_contracts_broker::broker_wire::ObserveRunnerRequest {
            d2b_contracts_broker::broker_wire::ObserveRunnerRequest {
                vm_id: d2b_contracts::types::VmId::new("reap-vm"),
                role_id: d2b_contracts::types::RoleId::new("ch-runner"),
                role: d2b_contracts_broker::broker_wire::RunnerRole::CloudHypervisor,
                bundle_runner_intent_ref: d2b_contracts::types::BundleOpId::new(
                    "runner:reap-vm:role:cloud-hypervisor",
                ),
                resource_ref: None,
                resource_uid: None,
                zone_uid: None,
                owner_ref: None,
                provider_ref: None,
                provider_identity: None,
                template_identity: None,
                generation: None,
                runtime_scope: None,
                guest_execution: None,
                tracing_span_id: None,
            }
        }

        fn test_runner_registration(pid: i32, start_time_ticks: u64) -> RunnerRegistration {
            RunnerRegistration {
                vm_id: "reap-vm".to_owned(),
                role_id: "ch-runner".to_owned(),
                resource_ref: None,
                resource_uid: None,
                zone_uid: None,
                generation: None,
                runtime_scope: None,
                owner_ref: None,
                provider_ref: None,
                provider_identity: None,
                template_identity: None,
                role: d2b_contracts_broker::broker_wire::RunnerRole::CloudHypervisor,
                bundle_runner_intent_ref: "runner:reap-vm:role:cloud-hypervisor".to_owned(),
                pid,
                start_time_ticks,
                binary_path: PathBuf::from("/nix/store/current-provider-controller/bin/controller"),
                cgroup_subtree: "d2b.slice/reap-vm/ch-runner".to_owned(),
                guest_execution: None,
            }
        }

        #[test]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn registered_observation_keeps_live_runner_registered_until_reaped() {
            let _guard = ReapTestGuard::new();

            let child = Command::new("sleep")
                .arg("600")
                .spawn()
                .expect("spawn sleep child");
            let pid = child.id() as i32;
            let runner_id = "reap-vm:ch-runner";
            let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
            let start_time_ticks = read_proc_start_time_ticks(pid)
                .expect("read start time")
                .expect("live child");
            runner_pidfds()
                .insert(runner_id, pidfd.try_clone().expect("clone pidfd"))
                .expect("register runner pidfd");
            with_runner_metadata_mut(|registry| {
                registry.insert(
                    runner_id.to_owned(),
                    test_runner_registration(pid, start_time_ticks),
                );
            });

            let registration = test_runner_registration(pid, start_time_ticks);
            let response = reap_registered_runner_after_observation(
                &observe_request(),
                runner_id,
                &registration,
                Some(pidfd),
            );
            assert!(response.present);
            assert!(!response.cgroup_verified);
            assert!(!response.executable_verified);
            assert!(
                runner_pidfds().contains_key(&runner_id),
                "StillAlive must preserve the exact pidfd registration"
            );
            assert!(
                with_runner_metadata_mut(|registry| registry.contains_key(runner_id)),
                "StillAlive must preserve runner metadata"
            );

            kill(Pid::from_raw(pid), Signal::SIGKILL).expect("kill child");
            let _ = nix::sys::wait::waitpid(Pid::from_raw(pid), None);
            std::mem::forget(child);
        }

        #[test]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn registered_observation_reports_absent_only_after_exact_reap() {
            let _guard = ReapTestGuard::new();

            let child = Command::new("true").spawn().expect("spawn true child");
            let pid = child.id() as i32;
            let runner_id = "reap-vm:ch-runner";
            let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
            runner_pidfds()
                .insert(runner_id, pidfd.try_clone().expect("clone pidfd"))
                .expect("register runner pidfd");
            with_runner_metadata_mut(|registry| {
                registry.insert(runner_id.to_owned(), test_runner_registration(pid, 1));
            });
            std::mem::forget(child);

            let registration = test_runner_registration(pid, 1);
            let request = observe_request();
            // Progress-based wait on the state transition the test is
            // about - the registered pidfd reporting the child's exit.
            // Each iteration is a real waitid probe on the pidfd, so the
            // loop converges whenever the test thread runs; the 30 s
            // deadline is a last-resort guard, not a load-bearing bound.
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut response = present_unverified_runner_response(&request, &registration);
            while response.present && Instant::now() < deadline {
                let retained_pidfd = runner_pidfds().duplicate(runner_id);
                response = reap_registered_runner_after_observation(
                    &request,
                    runner_id,
                    &registration,
                    retained_pidfd,
                );
                if response.present {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }

            assert!(!response.present, "reap must precede an absent response");
            assert!(
                !runner_pidfds().contains_key(&runner_id),
                "reaped runner must be removed from pidfd registry"
            );
            assert!(
                !with_runner_metadata_mut(|registry| registry.contains_key(runner_id)),
                "reaped runner must be removed from metadata registry"
            );
        }

        #[test]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn reap_loop_processes_exited_child() {
            let _guard = ReapTestGuard::new();
            let _rt = start_test_reaper("reap-exited-child");

            let child = Command::new("sleep")
                .arg("1")
                .spawn()
                .expect("spawn sleep child");
            let pid = child.id() as i32;
            let runner_id = format!("test-vm:test-role-{pid}");
            {
                let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
                runner_pidfds()
                    .insert(&runner_id, pidfd)
                    .expect("register runner pidfd");
            }
            std::mem::forget(child);

            let notif = wait_for_notification(&runner_id, &_rt, Duration::from_secs(30))
                .expect("ChildReaped notification should appear");
            assert_eq!(notif.exit_status.kind, ChildExitKind::Exited);
            assert_eq!(notif.exit_status.code, Some(0));
        }

        #[test]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn reserve_reclaims_stale_registration_of_a_dead_runner() {
            // A launch whose reply is lost leaves the broker holding the
            // runner-id-keyed registration of the spawned child. When that
            // child exits or is killed, the runner-keyed duplicate can
            // survive. The next spawn for the same runner id must not be
            // refused forever by the dead registration: the reserve guard
            // asks the registered pidfd (the authoritative handle) and
            // reclaims the stale entry, exactly as the watchdog relaunch in
            // the guest-preflight check needs after a lost spawn reply.
            let _guard = ReapTestGuard::new();

            let runner_id = "reap-vm:ch-runner";
            let child = Command::new("true")
                .spawn()
                .expect("spawn a short-lived child");
            let pid = child.id() as i32;
            let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
            // The start-time ticks are hardcoded, not read from /proc: the
            // `true` child exits immediately, so a /proc read races the
            // exit (a zombie reports no start time and the test would fail
            // spuriously). The value is never consulted here anyway -
            // `reserve_runner_id_for_spawn` decides liveness from the
            // registered pidfd alone.
            runner_pidfds()
                .insert(runner_id, pidfd)
                .expect("register runner pidfd");
            with_runner_metadata_mut(|registry| {
                registry.insert(
                    runner_id.to_owned(),
                    test_runner_registration(pid, 1),
                );
            });
            // Reap the child so the registered pidfd reports it exited.
            let _ = nix::sys::wait::waitpid(Pid::from_raw(pid), None);
            std::mem::forget(child);

            assert!(
                runner_pidfds().contains_key(runner_id),
                "precondition: registration exists"
            );
            assert!(reserve_runner_id_for_spawn(runner_id).is_ok());
            assert!(
                !runner_pidfds().contains_key(runner_id),
                "the stale pidfd registration must be reclaimed"
            );
            assert!(
                !with_runner_metadata_mut(|registry| registry.contains_key(runner_id)),
                "the stale metadata registration must be reclaimed"
            );
        }

        #[test]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn reserve_keeps_refusing_a_live_registration() {
            let _guard = ReapTestGuard::new();

            let child = Command::new("sleep")
                .arg("600")
                .spawn()
                .expect("spawn sleep child");
            let pid = child.id() as i32;
            let runner_id = "reap-vm:ch-runner-live";
            let start_time_ticks = read_proc_start_time_ticks(pid)
                .expect("read start time")
                .expect("live child");
            let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
            runner_pidfds()
                .insert(runner_id, pidfd)
                .expect("register runner pidfd");
            with_runner_metadata_mut(|registry| {
                registry.insert(
                    runner_id.to_owned(),
                    test_runner_registration(pid, start_time_ticks),
                );
            });

            let error = reserve_runner_id_for_spawn(runner_id)
                .expect_err("a live registration must still refuse a duplicate spawn");
            assert!(matches!(error, BrokerError::Protocol(_)));
            assert!(
                runner_pidfds().contains_key(runner_id),
                "the live registration must be kept intact"
            );

            kill(Pid::from_raw(pid), Signal::SIGKILL).expect("kill child");
            let _ = nix::sys::wait::waitpid(Pid::from_raw(pid), None);
            std::mem::forget(child);
            runner_pidfds().remove(runner_id);
            with_runner_metadata_mut(|registry| {
                registry.remove(runner_id);
            });
        }

        #[test]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn reap_loop_signaled_sigterm() {
            let _guard = ReapTestGuard::new();
            let _rt = start_test_reaper("reap-signaled-sigterm");

            let child = Command::new("sleep")
                .arg("600")
                .spawn()
                .expect("spawn sleep child");
            let pid = child.id() as i32;
            let runner_id = format!("test-vm:sigterm-{pid}");
            {
                let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
                runner_pidfds()
                    .insert(&runner_id, pidfd)
                    .expect("register runner pidfd");
            }
            kill(Pid::from_raw(pid), Signal::SIGTERM).expect("kill SIGTERM");
            std::mem::forget(child);

            let notif = wait_for_notification(&runner_id, &_rt, Duration::from_secs(30))
                .expect("ChildReaped notification for SIGTERM");
            assert_eq!(notif.exit_status.kind, ChildExitKind::Signaled);
            assert_eq!(notif.exit_status.signal, Some(libc::SIGTERM));
        }

        #[test]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn reap_loop_killed_sigkill() {
            let _guard = ReapTestGuard::new();
            let _rt = start_test_reaper("reap-killed-sigkill");

            let child = Command::new("sleep")
                .arg("600")
                .spawn()
                .expect("spawn sleep child");
            let pid = child.id() as i32;
            let runner_id = format!("test-vm:sigkill-{pid}");
            {
                let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
                runner_pidfds()
                    .insert(&runner_id, pidfd)
                    .expect("register runner pidfd");
            }
            kill(Pid::from_raw(pid), Signal::SIGKILL).expect("kill SIGKILL");
            std::mem::forget(child);

            let notif = wait_for_notification(&runner_id, &_rt, Duration::from_secs(30))
                .expect("ChildReaped notification for SIGKILL");
            assert_eq!(notif.exit_status.kind, ChildExitKind::Killed);
            assert_eq!(notif.exit_status.signal, Some(libc::SIGKILL));
        }

        #[test]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn reap_loop_concurrent_stress_8_children() {
            let _guard = ReapTestGuard::new();
            let _rt = start_test_reaper("reap-concurrent-stress");

            let mut runner_ids = Vec::new();
            for i in 0..8 {
                let child = Command::new("sleep")
                    .arg("1")
                    .spawn()
                    .expect("spawn stress child");
                let pid = child.id() as i32;
                let runner_id = format!("test-vm:stress-{i}-{pid}");
                let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
                runner_pidfds()
                    .insert(&runner_id, pidfd)
                    .expect("register runner pidfd");
                runner_ids.push(runner_id);
                std::mem::forget(child);
            }

            // Event-driven wait: the reap task wakes the wait the moment each
            // notification lands, so the wait lasts however long the reaper
            // actually takes; the 30 s guard is a last-resort hang breaker
            // for a broken reaper, not a load-bearing bound.
            let all_reaped = wait_for_reap_buffer(&_rt, Duration::from_secs(30), |buffer| {
                buffer
                    .iter()
                    .filter(|n| runner_ids.iter().any(|id| id == &n.runner_id))
                    .count()
                    == runner_ids.len()
            });
            assert!(
                all_reaped,
                "only some of {} children were reaped within 30 s",
                runner_ids.len()
            );
        }

        #[test]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn targeted_reap_reaps_already_exited_child() {
            // No SIGCHLD reaper is started: the targeted post-spawn
            // reap alone must reap a child that has already exited,
            // closing the registration-window zombie leak.
            let _guard = ReapTestGuard::new();

            let child = Command::new("true").spawn().expect("spawn true child");
            let pid = child.id() as i32;
            let runner_id = format!("test-vm:targeted-exited-{pid}");
            let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
            let registry_dup = pidfd.try_clone().expect("dup pidfd for registry");
            runner_pidfds()
                .insert(&runner_id, registry_dup)
                .expect("register runner pidfd");
            std::mem::forget(child);

            // The child exits ~immediately; loop the targeted reap until
            // it observes the exit (deterministic, no background loop).
            // Each iteration is a real waitid probe on the pidfd, so the
            // loop converges whenever the test thread runs; the 30 s
            // deadline is a last-resort guard, not a load-bearing bound.
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut reaped = None;
            let mut outcome = TargetedReapOutcome::StillAlive;
            while Instant::now() < deadline {
                outcome = targeted_reap_runner(&runner_id, pidfd.as_fd());
                // Non-blocking try-lock (plan U8): a Busy collision is
                // retried, never blocking the test.
                if let Ok(buffer) = child_reap_buffer().try_lock()
                    && let Some(n) = buffer
                        .iter()
                        .find(|n| n.runner_id == runner_id)
                        .cloned()
                {
                    reaped = Some(n);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }

            let notif = reaped.expect("targeted reap should reap the exited child");
            assert_eq!(outcome, TargetedReapOutcome::Reaped);
            assert_eq!(notif.exit_status.kind, ChildExitKind::Exited);
            assert_eq!(notif.exit_status.code, Some(0));
            // Registry entry must be gone so the SIGCHLD loop won't
            // double-reap a since-reused PID.
            assert!(
                !runner_pidfds().contains_key(&runner_id),
                "registry entry must be removed after targeted reap"
            );
        }

        #[test]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn spawn_rollback_reaps_and_deregisters_the_child() {
            // The post-spawn rollback reap must kill, reap, and
            // deregister the child so a retry can reserve the runner id
            // and no zombie is left behind.
            let _guard = ReapTestGuard::new();

            let child = Command::new("true").spawn().expect("spawn true child");
            let pid = child.id() as i32;
            let runner_id = format!("test-vm:rollback-{pid}");
            let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
            let registry_dup = pidfd.try_clone().expect("dup pidfd for registry");
            runner_pidfds()
                .insert(&runner_id, registry_dup)
                .expect("register runner pidfd");
            with_runner_metadata_mut(|registry| {
                registry.insert(runner_id.clone(), test_runner_registration(pid, 1));
            });
            std::mem::forget(child);

            envelope_call_runtime().block_on(cleanup_spawned_runner_after_failure(
                &runner_id,
                pidfd.as_fd(),
            ));

            assert!(
                !runner_pidfds().contains_key(&runner_id),
                "rollback reap must remove the pidfd registration"
            );
            assert!(
                !with_runner_metadata_mut(|registry| registry.contains_key(&runner_id)),
                "rollback reap must remove the runner metadata"
            );
            // The child must be reaped, not left a zombie: a fresh
            // WNOHANG probe on the pidfd reports ECHILD (already reaped).
            use nix::errno::Errno;
            use nix::sys::wait::{Id, WaitPidFlag, waitid};
            match waitid(Id::PIDFd(pidfd.as_fd()), WaitPidFlag::WEXITED | WaitPidFlag::WNOHANG) {
                Err(Errno::ECHILD) => {}
                other => panic!("rollback reap left the child unreaped: {other:?}"),
            }
        }

        #[test]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn targeted_reap_leaves_running_child_for_sigchld_loop() {
            // A still-running child must NOT be reaped by the targeted
            // pass: it stays registered for the SIGCHLD loop.
            let _guard = ReapTestGuard::new();

            let child = Command::new("sleep")
                .arg("600")
                .spawn()
                .expect("spawn sleep child");
            let pid = child.id() as i32;
            let runner_id = format!("test-vm:targeted-alive-{pid}");
            let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
            let registry_dup = pidfd.try_clone().expect("dup pidfd for registry");
            runner_pidfds()
                .insert(&runner_id, registry_dup)
                .expect("register runner pidfd");

            let outcome = targeted_reap_runner(&runner_id, pidfd.as_fd());
            assert_eq!(outcome, TargetedReapOutcome::StillAlive);

            let none_pending = loop {
                // Non-blocking try-lock (plan U8): retry a Busy collision.
                if let Ok(buffer) = child_reap_buffer().try_lock() {
                    break buffer.iter().all(|n| n.runner_id != runner_id);
                }
                std::thread::sleep(Duration::from_millis(1));
            };
            assert!(
                none_pending,
                "running child must not be reaped by targeted pass"
            );
            assert!(
                runner_pidfds().contains_key(&runner_id),
                "running child must remain registered"
            );

            // Clean up the still-running child.
            kill(Pid::from_raw(pid), Signal::SIGKILL).expect("kill SIGKILL");
            let _ = nix::sys::wait::waitpid(Pid::from_raw(pid), None);
            std::mem::forget(child);
        }

        #[test]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn targeted_reap_reports_signaled_child() {
            let _guard = ReapTestGuard::new();

            let child = Command::new("sleep")
                .arg("600")
                .spawn()
                .expect("spawn sleep child");
            let pid = child.id() as i32;
            let runner_id = format!("test-vm:targeted-signaled-{pid}");
            let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
            let registry_dup = pidfd.try_clone().expect("dup pidfd for registry");
            runner_pidfds()
                .insert(&runner_id, registry_dup)
                .expect("register runner pidfd");
            std::mem::forget(child);

            kill(Pid::from_raw(pid), Signal::SIGKILL).expect("kill SIGKILL");

            // Progress-based wait on the targeted reap observing the
            // signal: each iteration is a real waitid probe on the pidfd,
            // so the loop converges whenever the test thread runs; the
            // 30 s deadline is a last-resort guard, not a load-bearing
            // bound.
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut reaped = None;
            let mut outcome = TargetedReapOutcome::StillAlive;
            while Instant::now() < deadline {
                outcome = targeted_reap_runner(&runner_id, pidfd.as_fd());
                // Non-blocking try-lock (plan U8): a Busy collision is
                // retried, never blocking the test.
                if let Ok(buffer) = child_reap_buffer().try_lock()
                    && let Some(n) = buffer
                        .iter()
                        .find(|n| n.runner_id == runner_id)
                        .cloned()
                {
                    reaped = Some(n);
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }

            let notif = reaped.expect("targeted reap should observe the signaled child");
            assert_eq!(outcome, TargetedReapOutcome::Reaped);
            assert_eq!(notif.exit_status.kind, ChildExitKind::Killed);
            assert_eq!(notif.exit_status.signal, Some(libc::SIGKILL));
        }

        #[test]
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn targeted_reap_echild_clears_stale_registry_entry() {
            // If the SIGCHLD loop already reaped the child, a later
            // targeted reap sees ECHILD and must drop the stale entry
            // rather than leaving a dangling pidfd.
            let _guard = ReapTestGuard::new();

            let child = Command::new("true").spawn().expect("spawn true child");
            let pid = child.id() as i32;
            let runner_id = format!("test-vm:targeted-echild-{pid}");
            let pidfd = crate::sys::pidfd_sys::pidfd_open(pid, 0).expect("pidfd_open");
            let registry_dup = pidfd.try_clone().expect("dup pidfd for registry");
            runner_pidfds()
                .insert(&runner_id, registry_dup)
                .expect("register runner pidfd");
            with_runner_metadata_mut(|registry| {
                registry.insert(runner_id.clone(), test_runner_registration(pid, 1));
            });

            // Reap the child out-of-band so the pidfd waitid yields ECHILD.
            let _ = nix::sys::wait::waitpid(Pid::from_raw(pid), None);
            std::mem::forget(child);

            let outcome = targeted_reap_runner(&runner_id, pidfd.as_fd());
            assert_eq!(outcome, TargetedReapOutcome::AlreadyReaped);

            assert!(
                !runner_pidfds().contains_key(&runner_id),
                "ECHILD must clear the stale registry entry"
            );
            assert!(
                !with_runner_metadata_mut(|registry| registry.contains_key(&runner_id)),
                "ECHILD must clear stale runner metadata"
            );
        }

        #[test]
        fn reap_buffer_overflow_drops_oldest() {
            let _guard = ReapTestGuard::new();

            for i in 0..=CHILD_REAP_BUFFER_CAP {
                push_child_reap_notification(ChildReapedNotification {
                    runner_id: format!("overflow-{i}"),
                    pid: i as i32,
                    exit_status: ChildExitStatus {
                        kind: ChildExitKind::Exited,
                        code: Some(0),
                        signal: None,
                    },
                    reaped_at_ms: 0,
                });
            }

            let drained = drain_child_reap_buffer();
            assert_eq!(drained.len(), CHILD_REAP_BUFFER_CAP);
            assert!(!drained.iter().any(|n| n.runner_id == "overflow-0"));
            assert!(drained.iter().any(|n| n.runner_id == "overflow-256"));
        }
    }

    /// Two dispatch jobs run at the same time, not one after the other.
    ///
    /// Each job blocks until the other has started. A pool that ran one job
    /// at a time - the shape a single accept thread had, and the shape a lone
    /// blocking worker would keep - never releases the barrier and the join
    /// below times out.
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn two_dispatch_jobs_run_at_the_same_time() {
        let dispatches = DispatchPool::new(2);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("test runtime");
        let rendezvous = Arc::new(std::sync::Barrier::new(2));
        let both = runtime.block_on(async {
            let first = {
                let rendezvous = Arc::clone(&rendezvous);
                dispatches.run(move || rendezvous.wait())
            };
            let second = {
                let rendezvous = Arc::clone(&rendezvous);
                dispatches.run(move || rendezvous.wait())
            };
            tokio::time::timeout(Duration::from_secs(10), async move {
                tokio::join!(first, second)
            })
            .await
        });
        assert!(
            both.is_ok(),
            "two requests must be able to run at once: {both:?}"
        );
    }

    /// The broker serves connections concurrently: the accept path never runs
    /// a request, so a caller that connects and then sends nothing cannot hold
    /// the next caller's request behind it.
    ///
    /// This is the regression the async listener exists for. The synchronous
    /// server read the first connection's frame inline on its accept thread,
    /// so the second client's request stayed in the listen backlog, unaccepted,
    /// until the silent connection closed - and the read below timed out.
    #[cfg(not(feature = "layer1-bootstrap"))]
    #[test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn a_silent_connection_does_not_hold_the_next_request() {
        use d2b_contracts_broker::broker_wire::{
            BrokerCallerRole, BrokerRequestEnvelope, HelloRequest,
        };
        use nix::unistd::Uid;

        let root = test_audit_dir("broker-serves-connections-concurrently");
        fs::create_dir_all(&root).expect("create test dir");
        // The socket lives in its own short directory: a Unix socket path is
        // capped at 107 bytes, and the scratch root above is longer than that.
        let socket_dir = tempfile::tempdir().expect("socket dir");
        let socket_path = socket_dir.path().join("broker.sock");
        let mut config = test_server_config(&root, &root.join("absent-bundle.json"));
        let caller_uid = Uid::current().as_raw();
        config.d2bd_uid = caller_uid;
        config.d2bd_gid = Gid::current().as_raw();
        config.socket_path = socket_path.clone();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("test runtime");
        let listener =
            crate::protocol::bind_seqpacket(&socket_path).expect("bind the broker's test socket");
        let audit_log = Arc::new(
            AuditLog::open(&config.audit_dir, Gid::current().as_raw(), true, 0)
                .expect("open audit log"),
        );
        let server = Arc::new(Server {
            config: Arc::new(config),
            audit_log,
            dispatches: DispatchPool::new(2),
            nested_dispatches: DispatchPool::new(2),
            ipc_rate_limiter: Arc::new(tokio::sync::Mutex::new(IpcRateLimiter::new(64))),
        });
        let serving = runtime.spawn(serve(server, listener));

        let answer = runtime.block_on(async move {
            // The first client connects and stays silent for the whole test.
            let silent =
                crate::protocol::connect_seqpacket_bounded(&socket_path, Duration::from_secs(5))
                    .await
                    .expect("first client connects");
            let caller =
                crate::protocol::connect_seqpacket_bounded(&socket_path, Duration::from_secs(5))
                    .await
                    .expect("second client connects");
            let envelope = BrokerRequestEnvelope {
                request: BrokerRequest::Hello(HelloRequest {
                    client_version: "0.0.0-test".to_owned(),
                    supported_features: Vec::new(),
                }),
                caller_role: BrokerCallerRole::AdminUid { uid: caller_uid },
                test_peer_uid: Some(caller_uid),
                audit_join: None,
            };
            caller.send_json_frame(&envelope).await.expect("send Hello");
            let answer = tokio::time::timeout(
                Duration::from_secs(10),
                caller.recv_json_frame::<BrokerResponse>(),
            )
            .await
            .expect("the next request is answered while the first connection stays silent")
            .expect("frame read")
            .expect("the broker answered");
            drop(silent);
            answer
        });
        assert!(
            matches!(
                answer,
                BrokerResponse::Hello(ref response) if response.selected_version == "0.0.0-w2"
            ),
            "expected the daemon handshake, got {answer:?}"
        );
        serving.abort();
        let _ = fs::remove_dir_all(&root);
    }
}
