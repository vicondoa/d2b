//! Generated protobuf/ttrpc contracts for every d2b-owned ComponentSession v2 service.
//!
//! The generated DTOs contain only bounded opaque identifiers, digests, closed
//! enums, stream identifiers, and ComponentSession attachment indexes. Caller
//! identity and method capability are intentionally absent: authenticated
//! session state and [`SERVICE_INVENTORY`] are their sole authority.

use std::{collections::BTreeSet, error::Error, fmt, net::IpAddr};

use protobuf::{Enum, EnumOrUnknown, Message, MessageField};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{
    v2_component_session::{
        BoundedVec, CorrelationId, IdempotencyKey, MAX_LOGICAL_MESSAGE_BYTES,
        MAX_NAMED_STREAM_QUEUE_BYTES, MAX_REQUEST_ATTACHMENTS, MAX_REQUEST_LIFETIME_MS,
        RequestEnvelope, RequestId, TraceId,
    },
    v2_identity::{
        ProviderId, ProviderType as IdentityProviderType, RealmId, RealmLabel, RealmPath, RoleId,
        WorkloadId, WorkloadName,
    },
    v2_provider::{
        AdoptionState, AudioChannel as CanonicalAudioChannel,
        AudioDirection as CanonicalAudioDirection, ConfiguredItemId, DeviceSelectorId,
        Generation as ProviderGeneration,
        InfrastructurePowerState as CanonicalInfrastructurePowerState,
        MAX_OBSERVABILITY_QUERY_BYTES, MAX_OBSERVABILITY_QUERY_LIMIT, MAX_PROVIDER_CAPABILITIES,
        MAX_SAFE_JSON_INTEGER, OBSERVABILITY_RECORD_ENCODED_UPPER_BOUND_BYTES, ObservabilityCursor,
        ObservabilityExportFormat as CanonicalObservabilityExportFormat, ObservabilityLabels,
        ObservabilityMetricLabel, ObservabilityOperationLabel, ObservabilityOutcomeLabel,
        ObservabilityProjectionKind, ObservabilityQueryResult, ObservabilityRecord,
        ObservabilityView as CanonicalObservabilityView, ObservationReason, ObservedLifecycleState,
        ProviderHealth, ProviderHealthReason, ProviderHealthState, ProviderMethod,
        ProviderObservation, ProviderOperationInput as CanonicalProviderOperationInput,
        ProviderOperationRequest, ProviderRemediation, StorageSnapshotId, TransportBindingId,
    },
    v2_state::Generation,
};

#[path = "generated_v2_services/activation.rs"]
pub mod activation;
#[path = "generated_v2_services/activation_ttrpc.rs"]
pub mod activation_ttrpc;
#[path = "generated_v2_services/broker.rs"]
pub mod broker;
#[path = "generated_v2_services/broker_ttrpc.rs"]
pub mod broker_ttrpc;
#[path = "generated_v2_services/clipboard.rs"]
pub mod clipboard;
#[path = "generated_v2_services/clipboard_picker.rs"]
pub mod clipboard_picker;
#[path = "generated_v2_services/clipboard_picker_ttrpc.rs"]
pub mod clipboard_picker_ttrpc;
#[path = "generated_v2_services/clipboard_ttrpc.rs"]
pub mod clipboard_ttrpc;
#[allow(clippy::match_single_binding, clippy::needless_borrowed_reference)]
#[path = "generated_v2_services/common.rs"]
pub mod common;
#[allow(clippy::match_single_binding, clippy::needless_borrowed_reference)]
#[path = "generated_v2_services/daemon.rs"]
pub mod daemon;
#[path = "generated_v2_services/daemon_ttrpc.rs"]
pub mod daemon_ttrpc;
#[allow(clippy::match_single_binding, clippy::needless_borrowed_reference)]
#[path = "generated_v2_services/guest.rs"]
pub mod guest;
#[path = "generated_v2_services/guest_ttrpc.rs"]
pub mod guest_ttrpc;
#[path = "generated_v2_services/notify.rs"]
pub mod notify;
#[path = "generated_v2_services/notify_ttrpc.rs"]
pub mod notify_ttrpc;
macro_rules! provider_modules {
    ($($module:ident, $binding:ident, $file:literal, $binding_file:literal);+ $(;)?) => {
        $(
            #[path = $file]
            pub mod $module;
            #[path = $binding_file]
            pub mod $binding;
        )+
    };
}

provider_modules! {
    provider_audio, provider_audio_ttrpc,
        "generated_v2_services/provider_audio.rs",
        "generated_v2_services/provider_audio_ttrpc.rs";
    provider_credential, provider_credential_ttrpc,
        "generated_v2_services/provider_credential.rs",
        "generated_v2_services/provider_credential_ttrpc.rs";
    provider_device, provider_device_ttrpc,
        "generated_v2_services/provider_device.rs",
        "generated_v2_services/provider_device_ttrpc.rs";
    provider_display, provider_display_ttrpc,
        "generated_v2_services/provider_display.rs",
        "generated_v2_services/provider_display_ttrpc.rs";
    provider_infrastructure, provider_infrastructure_ttrpc,
        "generated_v2_services/provider_infrastructure.rs",
        "generated_v2_services/provider_infrastructure_ttrpc.rs";
    provider_network, provider_network_ttrpc,
        "generated_v2_services/provider_network.rs",
        "generated_v2_services/provider_network_ttrpc.rs";
    provider_observability, provider_observability_ttrpc,
        "generated_v2_services/provider_observability.rs",
        "generated_v2_services/provider_observability_ttrpc.rs";
    provider_runtime, provider_runtime_ttrpc,
        "generated_v2_services/provider_runtime.rs",
        "generated_v2_services/provider_runtime_ttrpc.rs";
    provider_storage, provider_storage_ttrpc,
        "generated_v2_services/provider_storage.rs",
        "generated_v2_services/provider_storage_ttrpc.rs";
    provider_substrate, provider_substrate_ttrpc,
        "generated_v2_services/provider_substrate.rs",
        "generated_v2_services/provider_substrate_ttrpc.rs";
    provider_transport, provider_transport_ttrpc,
        "generated_v2_services/provider_transport.rs",
        "generated_v2_services/provider_transport_ttrpc.rs";
}
#[path = "generated_v2_services/realm.rs"]
pub mod realm;
#[path = "generated_v2_services/realm_ttrpc.rs"]
pub mod realm_ttrpc;
#[path = "generated_v2_services/runtime_systemd_user.rs"]
pub mod runtime_systemd_user;
#[path = "generated_v2_services/runtime_systemd_user_ttrpc.rs"]
pub mod runtime_systemd_user_ttrpc;
#[path = "generated_v2_services/security_key.rs"]
pub mod security_key;
#[path = "generated_v2_services/security_key_ttrpc.rs"]
pub mod security_key_ttrpc;
#[path = "generated_v2_services/shell.rs"]
pub mod shell;
#[path = "generated_v2_services/shell_ttrpc.rs"]
pub mod shell_ttrpc;
#[allow(clippy::match_single_binding, clippy::needless_borrowed_reference)]
#[path = "generated_v2_services/terminal.rs"]
pub mod terminal;
#[path = "generated_v2_services/tty.rs"]
pub mod tty;
#[path = "generated_v2_services/tty_ttrpc.rs"]
pub mod tty_ttrpc;
#[path = "generated_v2_services/user.rs"]
pub mod user;
#[path = "generated_v2_services/user_ttrpc.rs"]
pub mod user_ttrpc;
#[path = "generated_v2_services/wayland.rs"]
pub mod wayland;
#[path = "generated_v2_services/wayland_ttrpc.rs"]
pub mod wayland_ttrpc;

#[path = "v2_guest_services.rs"]
pub mod guest_contract;

pub const MAX_PROTOBUF_MESSAGE_BYTES: usize = MAX_LOGICAL_MESSAGE_BYTES as usize;
pub const MAX_SERVICE_STRING_BYTES: usize = 64;
pub const MAX_PAGE_CURSOR_BYTES: usize = 128;
pub const MAX_PAGE_SIZE: u32 = 256;
pub const MAX_OBSERVATIONS: usize = 256;
pub const DIGEST_BYTES: usize = 32;
pub const MAX_ALLOCATOR_REQUEST_RESOURCES: usize = 32;
pub const MAX_ALLOCATOR_CONFLICTS: usize = 16;
pub const MAX_REALM_CHILD_FDS: usize = 64;
pub const CONTROLLER_PIDFD_ATTACHMENT_INDEX: u32 = 0;
pub const BROKER_PIDFD_ATTACHMENT_INDEX: u32 = 1;
pub const MAX_DAEMON_REALMS: usize = 64;
pub const MAX_DAEMON_WORKLOADS: usize = 256;
pub const MAX_DAEMON_DETAIL_BYTES: usize = 256;
pub const MAX_DAEMON_REFERENCE_BYTES: usize = 512;
pub const MAX_DAEMON_DEGRADED_REASONS: usize = 16;
pub const MAX_DAEMON_SERVICES: usize = 64;
pub const MAX_DAEMON_CAPABILITIES: usize = 32;
pub const MAX_DAEMON_MEDIA: usize = 32;
pub const MAX_DAEMON_USB_DEVICES: usize = 32;
pub const MAX_DAEMON_BRIDGES: usize = 32;
pub const MAX_DAEMON_READINESS: usize = 128;
pub const MAX_TERMINAL_ARGV: usize = 256;
pub const MAX_TERMINAL_ARG_BYTES: usize = 4096;
pub const MAX_TERMINAL_ARGV_BYTES: usize = 64 * 1024;
pub const MAX_TERMINAL_CHUNK_BYTES: usize = 64 * 1024;
pub const MAX_TERMINAL_FRAME_SEQUENCE: u64 = u32::MAX as u64;
pub const MIN_NAMED_STREAM_ID: u16 = 0x0100;

pub const SERVICE_PACKAGES: [&str; 15] = [
    "d2b.daemon.v2",
    "d2b.realm.v2",
    "d2b.guest.v2",
    "d2b.provider.v2",
    "d2b.broker.v2",
    "d2b.user.v2",
    "d2b.runtime.systemd-user.v2",
    "d2b.shell.v2",
    "d2b.clipboard.v2",
    "d2b.clipboard.picker.v2",
    "d2b.notify.v2",
    "d2b.security-key.v2",
    "d2b.wayland.v2",
    "d2b.activation.v2",
    "d2b.tty.v2",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MethodSpec {
    pub name: &'static str,
    pub mutating: bool,
    pub requires_idempotency: bool,
    pub max_request_bytes: u32,
    pub max_lifetime_ms: u32,
}

impl MethodSpec {
    const fn new(name: &'static str, mutating: bool) -> Self {
        Self {
            name,
            mutating,
            requires_idempotency: mutating,
            max_request_bytes: MAX_PROTOBUF_MESSAGE_BYTES as u32,
            max_lifetime_ms: MAX_REQUEST_LIFETIME_MS as u32,
        }
    }

    pub const fn method_id(self, package: &str, service: &str) -> u32 {
        stable_method_id(package, service, self.name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSpec {
    pub package: &'static str,
    pub service: &'static str,
    pub methods: &'static [MethodSpec],
}

macro_rules! methods {
    ($($name:literal => $mutating:literal),+ $(,)?) => {
        &[$(MethodSpec::new($name, $mutating)),+]
    };
}

pub const SERVICE_INVENTORY: &[ServiceSpec] = &[
    ServiceSpec {
        package: "d2b.daemon.v2",
        service: "DaemonService",
        methods: methods![
            "Resolve" => false, "ListRealms" => false, "ListWorkloads" => false,
            "Inspect" => false, "Apply" => true, "Start" => true, "Stop" => true,
            "Restart" => true, "Exec" => true, "Shell" => true, "OpenConsole" => true,
            "ExportAudit" => false, "Cancel" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.realm.v2",
        service: "RealmService",
        methods: methods![
            "Bootstrap" => true, "Enroll" => true, "ResolveRoute" => false,
            "AuthorizeShortcut" => true, "RevokeShortcut" => true,
            "ReportShortcutClose" => true, "Inspect" => false, "Cancel" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.guest.v2",
        service: "GuestService",
        methods: methods![
            "Bootstrap" => true, "Reconnect" => true, "Exec" => true, "CancelExec" => true,
            "InspectExec" => false, "OpenExecRetainedLog" => true, "OpenShell" => true, "FileTransfer" => true,
            "SecurityKey" => true, "Shutdown" => true, "Cancel" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.provider.v2",
        service: "RuntimeProviderService",
        methods: methods![
            "Health" => false, "Capabilities" => false, "Plan" => false, "Ensure" => true,
            "Start" => true, "Stop" => true, "Execute" => true, "Inspect" => false, "Adopt" => true,
            "Destroy" => true,
        ],
    },
    ServiceSpec {
        package: "d2b.provider.v2",
        service: "InfrastructureProviderService",
        methods: methods![
            "Health" => false, "Capabilities" => false, "Plan" => false, "Apply" => true,
            "SetPowerState" => true, "Inspect" => false, "Adopt" => true,
            "BootstrapBinding" => true, "Destroy" => true,
        ],
    },
    ServiceSpec {
        package: "d2b.provider.v2",
        service: "TransportProviderService",
        methods: methods![
            "Health" => false, "Capabilities" => false, "Connect" => true, "Listen" => true,
            "IssueBinding" => true, "RevokeBinding" => true, "Inspect" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.provider.v2",
        service: "SubstrateProviderService",
        methods: methods![
            "Health" => false, "Capabilities" => false, "Check" => false,
            "PlanRemediation" => false, "Apply" => true,
        ],
    },
    ServiceSpec {
        package: "d2b.provider.v2",
        service: "CredentialProviderService",
        methods: methods![
            "Health" => false, "Capabilities" => false, "Status" => false, "AcquireLease" => true,
            "RefreshLease" => true, "RevokeLease" => true,
        ],
    },
    ServiceSpec {
        package: "d2b.provider.v2",
        service: "DisplayProviderService",
        methods: methods![
            "Health" => false, "Capabilities" => false, "Open" => true, "Inspect" => false,
            "Adopt" => true, "Close" => true,
        ],
    },
    ServiceSpec {
        package: "d2b.provider.v2",
        service: "NetworkProviderService",
        methods: methods![
            "Health" => false, "Capabilities" => false, "Plan" => false, "Ensure" => true,
            "Inspect" => false, "Adopt" => true, "Release" => true,
        ],
    },
    ServiceSpec {
        package: "d2b.provider.v2",
        service: "StorageProviderService",
        methods: methods![
            "Health" => false, "Capabilities" => false, "Plan" => false, "Ensure" => true,
            "Inspect" => false, "Adopt" => true, "Snapshot" => true, "Destroy" => true,
        ],
    },
    ServiceSpec {
        package: "d2b.provider.v2",
        service: "DeviceProviderService",
        methods: methods![
            "Health" => false, "Capabilities" => false, "PlanAttach" => false, "Attach" => true,
            "Inspect" => false, "Adopt" => true, "Detach" => true,
        ],
    },
    ServiceSpec {
        package: "d2b.provider.v2",
        service: "AudioProviderService",
        methods: methods![
            "Health" => false, "Capabilities" => false, "Open" => true, "SetState" => true,
            "Inspect" => false, "Adopt" => true, "Close" => true,
        ],
    },
    ServiceSpec {
        package: "d2b.provider.v2",
        service: "ObservabilityProviderService",
        methods: methods![
            "Health" => false, "Capabilities" => false, "Status" => false, "Query" => false,
            "Subscribe" => true, "Export" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.broker.v2",
        service: "BrokerService",
        methods: methods![
            "ValidateLease" => false, "Allocate" => true, "Delegate" => true, "Spawn" => true,
            "OpenResource" => true, "Apply" => true, "Observe" => false, "RevokeLease" => true,
            "ExportAudit" => false, "Cancel" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.user.v2",
        service: "UserService",
        methods: methods![
            "Prompt" => true, "PollPrompt" => false, "CancelPrompt" => true,
            "DeleteCredential" => true, "RevokeExport" => true, "Inspect" => false,
            "Cancel" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.runtime.systemd-user.v2",
        service: "RuntimeSystemdUserService",
        methods: methods![
            "EnsureScope" => true, "StartProcess" => true, "InspectProcess" => false,
            "AdoptProcess" => true, "StopProcess" => true, "OpenTerminal" => true,
            "Cancel" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.shell.v2",
        service: "ShellService",
        methods: methods![
            "Create" => true, "Attach" => true, "Detach" => true, "List" => false,
            "Inspect" => false, "Kill" => true, "Cancel" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.clipboard.v2",
        service: "ClipboardService",
        methods: methods![
            "Offer" => true, "InspectOffer" => false, "AcceptTransfer" => true,
            "CompleteTransfer" => true, "CancelTransfer" => true, "BridgeReady" => false,
            "Cancel" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.clipboard.picker.v2",
        service: "ClipboardPickerService",
        methods: methods![
            "ListOffers" => false, "SelectOffer" => true, "CancelSelection" => true,
            "Cancel" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.notify.v2",
        service: "NotifyService",
        methods: methods![
            "Subscribe" => true, "Acknowledge" => true, "InvokeAction" => true,
            "Cancel" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.security-key.v2",
        service: "SecurityKeyService",
        methods: methods![
            "BeginCeremony" => true, "ExchangeReport" => true, "Approve" => true,
            "CancelCeremony" => true, "Inspect" => false, "Cancel" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.wayland.v2",
        service: "WaylandService",
        methods: methods![
            "OpenDisplay" => true, "InspectDisplay" => false, "CloseDisplay" => true,
            "BridgeReady" => false, "Cancel" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.activation.v2",
        service: "ActivationService",
        methods: methods![
            "Activate" => true, "Inspect" => false, "Cancel" => false,
        ],
    },
    ServiceSpec {
        package: "d2b.tty.v2",
        service: "TtyService",
        methods: methods![
            "EnterRawMode" => true, "RestoreMode" => true, "Inspect" => false,
            "Cancel" => false,
        ],
    },
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceInventoryDocument {
    pub schema_version: u32,
    pub services: Vec<ServiceDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDocument {
    pub package: String,
    pub service: String,
    pub schema_fingerprint: String,
    pub methods: Vec<MethodDocument>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MethodDocument {
    pub name: String,
    pub method_id: u32,
    pub mutating: bool,
    pub requires_idempotency: bool,
    pub max_request_bytes: u32,
    pub max_lifetime_ms: u32,
}

pub fn service_inventory_document() -> ServiceInventoryDocument {
    ServiceInventoryDocument {
        schema_version: 5,
        services: SERVICE_INVENTORY
            .iter()
            .map(|service| ServiceDocument {
                package: service.package.to_owned(),
                service: service.service.to_owned(),
                schema_fingerprint: hex_digest(service_schema_fingerprint(service)),
                methods: service
                    .methods
                    .iter()
                    .map(|method| MethodDocument {
                        name: method.name.to_owned(),
                        method_id: method.method_id(service.package, service.service),
                        mutating: method.mutating,
                        requires_idempotency: method.requires_idempotency,
                        max_request_bytes: method.max_request_bytes,
                        max_lifetime_ms: method.max_lifetime_ms,
                    })
                    .collect(),
            })
            .collect(),
    }
}

pub fn service_schema_fingerprint(service: &ServiceSpec) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b-service-schema-v2\0");
    digest.update(service.package.as_bytes());
    digest.update(b"\0");
    digest.update(service.service.as_bytes());
    if service.package == "d2b.provider.v2" {
        digest.update(b"\0provider-response-observability-query-result-v1");
    }
    if service.package == "d2b.broker.v2" {
        digest.update(b"\0typed-allocator-and-realm-child-spawn-v3");
    }
    if service.package == "d2b.daemon.v2" {
        digest.update(b"\0typed-results-and-terminal-stream-v2\0");
        digest.update(include_bytes!("../proto/v2/daemon.proto"));
        digest.update(b"\0");
        digest.update(include_bytes!("../proto/v2/terminal.proto"));
    }
    if service.package == "d2b.guest.v2" {
        digest.update(b"\0typed-guest-operations-v2\0");
        digest.update(include_bytes!("../proto/v2/guest.proto"));
        digest.update(b"\0");
        digest.update(include_bytes!("../proto/v2/terminal.proto"));
    }
    for method in service.methods {
        digest.update(b"\0");
        digest.update(method.name.as_bytes());
        digest.update(
            method
                .method_id(service.package, service.service)
                .to_be_bytes(),
        );
        digest.update([
            u8::from(method.mutating),
            u8::from(method.requires_idempotency),
        ]);
        digest.update(method.max_request_bytes.to_be_bytes());
        digest.update(method.max_lifetime_ms.to_be_bytes());
    }
    digest.finalize().into()
}

fn hex_digest(value: [u8; 32]) -> String {
    let mut encoded = String::with_capacity(64);
    for byte in value {
        use fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

const fn fnv1a(mut hash: u32, bytes: &[u8]) -> u32 {
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u32;
        hash = hash.wrapping_mul(16_777_619);
        index += 1;
    }
    hash
}

pub const fn stable_method_id(package: &str, service: &str, method: &str) -> u32 {
    let hash = fnv1a(2_166_136_261, package.as_bytes());
    let hash = fnv1a(fnv1a(hash, b"/"), service.as_bytes());
    fnv1a(fnv1a(hash, b"/"), method.as_bytes())
}

pub fn method_spec(package: &str, service: &str, method: &str) -> Option<&'static MethodSpec> {
    SERVICE_INVENTORY
        .iter()
        .find(|candidate| candidate.package == package && candidate.service == service)
        .and_then(|candidate| {
            candidate
                .methods
                .iter()
                .find(|candidate| candidate.name == method)
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceContractError {
    MessageTooLarge,
    UnknownField,
    MissingMetadata,
    InvalidIdentity,
    InvalidId,
    InvalidDigest,
    InvalidEnum,
    MissingOperationInput,
    InvalidOperationInput,
    InvalidDeadline,
    MissingIdempotency,
    BoundExceeded,
    DuplicateAttachment,
    InconsistentResponse,
    Encode,
    Decode,
}

impl fmt::Display for ServiceContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MessageTooLarge => "v2-service-message-too-large",
            Self::UnknownField => "v2-service-unknown-field",
            Self::MissingMetadata => "v2-service-missing-metadata",
            Self::InvalidIdentity => "v2-service-invalid-identity",
            Self::InvalidId => "v2-service-invalid-id",
            Self::InvalidDigest => "v2-service-invalid-digest",
            Self::InvalidEnum => "v2-service-invalid-enum",
            Self::MissingOperationInput => "v2-service-missing-operation-input",
            Self::InvalidOperationInput => "v2-service-invalid-operation-input",
            Self::InvalidDeadline => "v2-service-invalid-deadline",
            Self::MissingIdempotency => "v2-service-missing-idempotency",
            Self::BoundExceeded => "v2-service-bound-exceeded",
            Self::DuplicateAttachment => "v2-service-duplicate-attachment",
            Self::InconsistentResponse => "v2-service-inconsistent-response",
            Self::Encode => "v2-service-encode-failed",
            Self::Decode => "v2-service-decode-failed",
        })
    }
}

impl Error for ServiceContractError {}

pub trait StrictWireMessage: Message + Default {
    fn validate_wire(&self, requires_idempotency: bool) -> Result<(), ServiceContractError>;
}

pub fn decode_strict<T: StrictWireMessage>(
    bytes: &[u8],
    requires_idempotency: bool,
) -> Result<T, ServiceContractError> {
    if bytes.len() > MAX_PROTOBUF_MESSAGE_BYTES {
        return Err(ServiceContractError::MessageTooLarge);
    }
    let message = T::parse_from_bytes(bytes).map_err(|_| ServiceContractError::Decode)?;
    message.validate_wire(requires_idempotency)?;
    if message.compute_size() > MAX_PROTOBUF_MESSAGE_BYTES as u64 {
        return Err(ServiceContractError::MessageTooLarge);
    }
    Ok(message)
}

pub fn encode_strict<T: StrictWireMessage>(
    message: &T,
    requires_idempotency: bool,
) -> Result<Vec<u8>, ServiceContractError> {
    message.validate_wire(requires_idempotency)?;
    if message.compute_size() > MAX_PROTOBUF_MESSAGE_BYTES as u64 {
        return Err(ServiceContractError::MessageTooLarge);
    }
    message
        .write_to_bytes()
        .map_err(|_| ServiceContractError::Encode)
}

fn reject_unknown(message: &impl Message) -> Result<(), ServiceContractError> {
    if message.unknown_fields().iter().next().is_some() {
        Err(ServiceContractError::UnknownField)
    } else {
        Ok(())
    }
}

fn bounded_opaque(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.is_ascii()
        && value.as_bytes()[0].is_ascii_lowercase()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

fn optional_digest(value: &[u8]) -> bool {
    value.is_empty() || value.len() == DIGEST_BYTES
}

fn required_digest(value: &[u8]) -> bool {
    value.len() == DIGEST_BYTES && value.iter().any(|byte| *byte != 0)
}

fn required_message<T>(value: &MessageField<T>) -> Result<&T, ServiceContractError> {
    value.as_ref().ok_or(ServiceContractError::MissingMetadata)
}

fn validate_metadata(
    value: &common::RequestMetadata,
    requires_idempotency: bool,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    let envelope = RequestEnvelope {
        request_id: RequestId::new(value.request_id.clone())
            .map_err(|_| ServiceContractError::InvalidId)?,
        correlation_id: if value.correlation_id.is_empty() {
            None
        } else {
            Some(
                CorrelationId::new(value.correlation_id.as_bytes().to_vec())
                    .map_err(|_| ServiceContractError::InvalidId)?,
            )
        },
        trace_id: if value.trace_id.is_empty() {
            None
        } else {
            Some(
                TraceId::new(value.trace_id.clone())
                    .map_err(|_| ServiceContractError::InvalidId)?,
            )
        },
        idempotency_key: if value.idempotency_key.is_empty() {
            None
        } else {
            Some(
                IdempotencyKey::new(value.idempotency_key.clone())
                    .map_err(|_| ServiceContractError::InvalidId)?,
            )
        },
        issued_at_unix_ms: value.issued_at_unix_ms,
        expires_at_unix_ms: value.expires_at_unix_ms,
    };
    if requires_idempotency && envelope.idempotency_key.is_none() {
        return Err(ServiceContractError::MissingIdempotency);
    }
    if value.session_generation == 0 {
        return Err(ServiceContractError::InvalidId);
    }
    let lifetime = envelope
        .expires_at_unix_ms
        .checked_sub(envelope.issued_at_unix_ms)
        .ok_or(ServiceContractError::InvalidDeadline)?;
    if envelope.issued_at_unix_ms == 0
        || envelope.expires_at_unix_ms == 0
        || lifetime == 0
        || lifetime > MAX_REQUEST_LIFETIME_MS
    {
        return Err(ServiceContractError::InvalidDeadline);
    }
    Ok(())
}

pub fn admit_metadata(
    value: &common::RequestMetadata,
    requires_idempotency: bool,
    now_unix_ms: u64,
    service_max_lifetime_ms: u64,
    monotonic_remaining_nanos: Option<u64>,
    peer_ttrpc_timeout_nanos: Option<u64>,
) -> Result<u64, ServiceContractError> {
    validate_metadata(value, requires_idempotency)?;
    let envelope = RequestEnvelope {
        request_id: RequestId::new(value.request_id.clone())
            .map_err(|_| ServiceContractError::InvalidId)?,
        correlation_id: None,
        trace_id: None,
        idempotency_key: if value.idempotency_key.is_empty() {
            None
        } else {
            Some(
                IdempotencyKey::new(value.idempotency_key.clone())
                    .map_err(|_| ServiceContractError::InvalidId)?,
            )
        },
        issued_at_unix_ms: value.issued_at_unix_ms,
        expires_at_unix_ms: value.expires_at_unix_ms,
    };
    envelope
        .admit(
            now_unix_ms,
            service_max_lifetime_ms,
            monotonic_remaining_nanos,
            peer_ttrpc_timeout_nanos,
        )
        .map(|deadline| deadline.remaining_nanos)
        .map_err(|_| ServiceContractError::InvalidDeadline)
}

fn validate_scope(value: &common::IdentityScope) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    RealmId::parse(value.realm_id.clone()).map_err(|_| ServiceContractError::InvalidIdentity)?;
    if !value.workload_id.is_empty() {
        WorkloadId::parse(value.workload_id.clone())
            .map_err(|_| ServiceContractError::InvalidIdentity)?;
    }
    if !value.provider_id.is_empty() {
        ProviderId::parse(value.provider_id.clone())
            .map_err(|_| ServiceContractError::InvalidIdentity)?;
    }
    if !value.role_id.is_empty() {
        if value.workload_id.is_empty() || !value.provider_id.is_empty() {
            return Err(ServiceContractError::InvalidIdentity);
        }
        RoleId::parse(value.role_id.clone()).map_err(|_| ServiceContractError::InvalidIdentity)?;
    }
    if !value.provider_id.is_empty() && !value.workload_id.is_empty() {
        return Err(ServiceContractError::InvalidIdentity);
    }
    Ok(())
}

fn validate_attachments(values: &[u32]) -> Result<(), ServiceContractError> {
    if values.len() > MAX_REQUEST_ATTACHMENTS as usize
        || values
            .iter()
            .any(|index| *index >= u32::from(MAX_REQUEST_ATTACHMENTS))
    {
        return Err(ServiceContractError::BoundExceeded);
    }
    let unique: BTreeSet<_> = values.iter().copied().collect();
    if unique.len() != values.len() {
        return Err(ServiceContractError::DuplicateAttachment);
    }
    Ok(())
}

impl StrictWireMessage for common::ServiceRequest {
    fn validate_wire(&self, requires_idempotency: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        validate_metadata(required_message(&self.metadata)?, requires_idempotency)?;
        validate_scope(required_message(&self.scope)?)?;
        if (!self.resource_id.is_empty()
            && !bounded_opaque(&self.resource_id, MAX_SERVICE_STRING_BYTES))
            || (!self.operation_id.is_empty()
                && !bounded_opaque(&self.operation_id, MAX_SERVICE_STRING_BYTES))
            || (!self.stream_id.is_empty()
                && !bounded_opaque(&self.stream_id, MAX_SERVICE_STRING_BYTES))
            || (!self.page_cursor.is_empty()
                && !bounded_opaque(&self.page_cursor, MAX_PAGE_CURSOR_BYTES))
            || self.page_size > MAX_PAGE_SIZE
        {
            return Err(ServiceContractError::BoundExceeded);
        }
        if !optional_digest(&self.request_digest) {
            return Err(ServiceContractError::InvalidDigest);
        }
        if self.desired_state.enum_value().is_err() {
            return Err(ServiceContractError::InvalidEnum);
        }
        validate_attachments(&self.attachment_indexes)
    }
}

fn bounded_ascii(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| !byte.is_ascii_control() && byte != b'\\')
}

fn optional_bounded_ascii(value: &str, max: usize) -> bool {
    value.is_empty() || bounded_ascii(value, max)
}

fn valid_required_enum<E>(value: &EnumOrUnknown<E>, unspecified: E) -> bool
where
    E: Enum + Eq,
{
    value
        .enum_value()
        .ok()
        .is_some_and(|actual| actual != unspecified)
}

fn valid_optional_enum<E>(value: &EnumOrUnknown<E>) -> bool
where
    E: Enum,
{
    value.enum_value().is_ok()
}

fn validate_page(
    value: &daemon::PageInfo,
    returned: usize,
    maximum: usize,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if returned > maximum
        || value.returned_items as usize != returned
        || value.truncated == value.next_page_cursor.is_empty()
        || (!value.next_page_cursor.is_empty()
            && !bounded_ascii(&value.next_page_cursor, MAX_PAGE_CURSOR_BYTES))
        || (!value.total_items_known && value.total_items != 0)
        || (value.total_items_known && value.total_items < value.returned_items)
    {
        return Err(ServiceContractError::BoundExceeded);
    }
    Ok(())
}

fn validate_daemon_response_shape(
    outcome: &EnumOrUnknown<common::Outcome>,
    error: &MessageField<common::ErrorEnvelope>,
    page: &MessageField<daemon::PageInfo>,
    returned: usize,
    maximum: usize,
) -> Result<(), ServiceContractError> {
    let outcome = outcome
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?;
    match outcome {
        common::Outcome::OUTCOME_SUCCEEDED | common::Outcome::OUTCOME_DEGRADED => {
            if error.is_some() {
                return Err(ServiceContractError::InconsistentResponse);
            }
            validate_page(
                page.as_ref()
                    .ok_or(ServiceContractError::InconsistentResponse)?,
                returned,
                maximum,
            )
        }
        common::Outcome::OUTCOME_DENIED
        | common::Outcome::OUTCOME_CANCELLED
        | common::Outcome::OUTCOME_FAILED => {
            if returned != 0 || page.is_some() {
                return Err(ServiceContractError::InconsistentResponse);
            }
            validate_error(
                error
                    .as_ref()
                    .ok_or(ServiceContractError::InconsistentResponse)?,
            )
        }
        _ => Err(ServiceContractError::InconsistentResponse),
    }
}

fn validate_realm_projection(value: &daemon::RealmProjection) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    RealmId::parse(value.realm_id.clone()).map_err(|_| ServiceContractError::InvalidIdentity)?;
    RealmPath::parse(value.realm_path.clone())
        .map_err(|_| ServiceContractError::InvalidIdentity)?;
    if value.realm_label != "local-root" {
        RealmLabel::parse(value.realm_label.clone())
            .map_err(|_| ServiceContractError::InvalidIdentity)?;
    }
    if !valid_required_enum(&value.mode, daemon::RealmMode::REALM_MODE_UNSPECIFIED)
        || !valid_required_enum(&value.state, daemon::RealmState::REALM_STATE_UNSPECIFIED)
        || !valid_required_enum(
            &value.cross_realm_policy,
            daemon::CrossRealmPolicy::CROSS_REALM_POLICY_UNSPECIFIED,
        )
        || !valid_required_enum(
            &value.credential_boundary,
            daemon::CredentialBoundary::CREDENTIAL_BOUNDARY_UNSPECIFIED,
        )
        || value.generation == 0
    {
        return Err(ServiceContractError::InvalidEnum);
    }
    if !value.gateway_workload_id.is_empty() {
        WorkloadId::parse(value.gateway_workload_id.clone())
            .map_err(|_| ServiceContractError::InvalidIdentity)?;
    }
    if !value.gateway_target.is_empty() && !valid_canonical_target(&value.gateway_target) {
        return Err(ServiceContractError::InvalidIdentity);
    }
    let gateway_backed =
        value.mode.enum_value().ok() == Some(daemon::RealmMode::REALM_MODE_GATEWAY_BACKED);
    if gateway_backed != (!value.gateway_workload_id.is_empty() && !value.gateway_target.is_empty())
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

impl StrictWireMessage for daemon::ListRealmsResponse {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        validate_daemon_response_shape(
            &self.outcome,
            &self.error,
            &self.page,
            self.realms.len(),
            MAX_DAEMON_REALMS,
        )?;
        let mut identities = BTreeSet::new();
        for realm in &self.realms {
            validate_realm_projection(realm)?;
            if !identities.insert(realm.realm_id.as_str()) {
                return Err(ServiceContractError::InconsistentResponse);
            }
        }
        Ok(())
    }
}

fn valid_canonical_target(value: &str) -> bool {
    value.len() <= MAX_DAEMON_DETAIL_BYTES
        && value.ends_with(".d2b")
        && value.split('.').count() >= 3
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.is_ascii()
                && label.as_bytes()[0].is_ascii_lowercase()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
}

fn validate_workload_identity(
    value: &daemon::WorkloadIdentityProjection,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    RealmId::parse(value.realm_id.clone()).map_err(|_| ServiceContractError::InvalidIdentity)?;
    WorkloadId::parse(value.workload_id.clone())
        .map_err(|_| ServiceContractError::InvalidIdentity)?;
    RealmPath::parse(value.realm_path.clone())
        .map_err(|_| ServiceContractError::InvalidIdentity)?;
    WorkloadName::parse(value.workload_name.clone())
        .map_err(|_| ServiceContractError::InvalidIdentity)?;
    if !valid_canonical_target(&value.canonical_target) {
        return Err(ServiceContractError::InvalidIdentity);
    }
    Ok(())
}

fn validate_degraded_reason(value: &daemon::DegradedReason) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if !bounded_ascii(&value.reason, MAX_DAEMON_DETAIL_BYTES)
        || !bounded_ascii(&value.remediation, MAX_DAEMON_DETAIL_BYTES)
    {
        return Err(ServiceContractError::BoundExceeded);
    }
    Ok(())
}

fn validate_lifecycle(
    value: &daemon::WorkloadLifecycleProjection,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if !valid_required_enum(
        &value.state,
        daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_UNSPECIFIED,
    ) || value.generation == 0
        || value.degraded_reasons.len() > MAX_DAEMON_DEGRADED_REASONS
        || value.degraded == value.degraded_reasons.is_empty()
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    for reason in &value.degraded_reasons {
        validate_degraded_reason(reason)?;
    }
    Ok(())
}

fn validate_runtime(value: &daemon::RuntimeProjection) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if !valid_required_enum(&value.kind, daemon::RuntimeKind::RUNTIME_KIND_UNSPECIFIED)
        || !bounded_ascii(&value.detail, MAX_DAEMON_DETAIL_BYTES)
        || value.supported_capabilities.len() > MAX_DAEMON_CAPABILITIES
        || value.unsupported_capabilities.len() > MAX_DAEMON_CAPABILITIES
    {
        return Err(ServiceContractError::BoundExceeded);
    }
    let mut capabilities = BTreeSet::new();
    for capability in value
        .supported_capabilities
        .iter()
        .chain(&value.unsupported_capabilities)
    {
        if !valid_required_enum(
            capability,
            daemon::RuntimeCapability::RUNTIME_CAPABILITY_UNSPECIFIED,
        ) || !capabilities.insert(capability.value())
        {
            return Err(ServiceContractError::InvalidEnum);
        }
    }
    Ok(())
}

fn validate_service(value: &daemon::ServiceProjection) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if !valid_required_enum(&value.kind, daemon::ServiceKind::SERVICE_KIND_UNSPECIFIED)
        || !valid_required_enum(
            &value.state,
            daemon::ServiceState::SERVICE_STATE_UNSPECIFIED,
        )
        || !bounded_opaque(&value.role_id, MAX_SERVICE_STRING_BYTES)
    {
        return Err(ServiceContractError::InvalidEnum);
    }
    Ok(())
}

fn validate_autostart(value: &daemon::AutostartProjection) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if !valid_required_enum(
        &value.mode,
        daemon::AutostartMode::AUTOSTART_MODE_UNSPECIFIED,
    ) || !bounded_ascii(&value.reason, MAX_DAEMON_DETAIL_BYTES)
    {
        return Err(ServiceContractError::BoundExceeded);
    }
    Ok(())
}

fn validate_deployment(value: &daemon::DeploymentProjection) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if [
        &value.declared_guest_closure,
        &value.current_generation,
        &value.booted_generation,
    ]
    .iter()
    .any(|entry| !optional_bounded_ascii(entry, MAX_DAEMON_REFERENCE_BYTES))
        || (value.declared_guest_closure.is_empty()
            && value.current_generation.is_empty()
            && value.booted_generation.is_empty())
    {
        return Err(ServiceContractError::BoundExceeded);
    }
    Ok(())
}

fn validate_runner_parity(
    value: &daemon::RunnerParityProjection,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if !bounded_opaque(&value.declared_runner, MAX_SERVICE_STRING_BYTES)
        || !bounded_ascii(&value.parity_reference, MAX_DAEMON_REFERENCE_BYTES)
    {
        return Err(ServiceContractError::BoundExceeded);
    }
    Ok(())
}

fn validate_live_pool(
    value: &daemon::LivePoolIntegrityProjection,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if !valid_required_enum(
        &value.state,
        daemon::ServiceState::SERVICE_STATE_UNSPECIFIED,
    ) || !optional_bounded_ascii(&value.reason, MAX_DAEMON_DETAIL_BYTES)
        || !optional_bounded_ascii(&value.audit_reference, MAX_DAEMON_REFERENCE_BYTES)
        || !optional_bounded_ascii(&value.remediation, MAX_DAEMON_DETAIL_BYTES)
    {
        return Err(ServiceContractError::BoundExceeded);
    }
    Ok(())
}

fn validate_qemu_registry(
    value: &daemon::QemuMediaRegistryProjection,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if !valid_required_enum(
        &value.state,
        daemon::ServiceState::SERVICE_STATE_UNSPECIFIED,
    ) || !optional_bounded_ascii(&value.remediation, MAX_DAEMON_DETAIL_BYTES)
    {
        return Err(ServiceContractError::BoundExceeded);
    }
    Ok(())
}

fn validate_qemu_media(value: &daemon::QemuMediaProjection) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if !valid_required_enum(
        &value.firmware_mode,
        daemon::QemuMediaFirmwareMode::QEMU_MEDIA_FIRMWARE_MODE_UNSPECIFIED,
    ) || !valid_required_enum(
        &value.runner_state,
        daemon::ServiceState::SERVICE_STATE_UNSPECIFIED,
    ) || !valid_required_enum(
        &value.qmp_readiness,
        daemon::QemuMediaReadiness::QEMU_MEDIA_READINESS_UNSPECIFIED,
    ) || !valid_required_enum(
        &value.pre_cont_progress,
        daemon::QemuMediaProgress::QEMU_MEDIA_PROGRESS_UNSPECIFIED,
    ) || value.media.len() > MAX_DAEMON_MEDIA
    {
        return Err(ServiceContractError::InvalidEnum);
    }
    let mut media_refs = BTreeSet::new();
    for media in &value.media {
        reject_unknown(media)?;
        if !bounded_opaque(&media.media_ref, MAX_SERVICE_STRING_BYTES)
            || !bounded_opaque(&media.slot, MAX_SERVICE_STRING_BYTES)
            || !valid_required_enum(
                &media.source_kind,
                daemon::QemuMediaSourceKind::QEMU_MEDIA_SOURCE_KIND_UNSPECIFIED,
            )
            || !valid_required_enum(
                &media.format,
                daemon::QemuMediaFormat::QEMU_MEDIA_FORMAT_UNSPECIFIED,
            )
            || !media_refs.insert(media.media_ref.as_str())
        {
            return Err(ServiceContractError::BoundExceeded);
        }
        validate_qemu_registry(
            media
                .registry
                .as_ref()
                .ok_or(ServiceContractError::InconsistentResponse)?,
        )?;
    }
    Ok(())
}

fn validate_usb(value: &daemon::UsbProjection) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if value.devices.len() > MAX_DAEMON_USB_DEVICES {
        return Err(ServiceContractError::BoundExceeded);
    }
    let mut devices = BTreeSet::new();
    for device in &value.devices {
        reject_unknown(device)?;
        if !bounded_opaque(&device.device_id, MAX_SERVICE_STRING_BYTES)
            || !valid_required_enum(
                &device.state,
                daemon::UsbDeviceState::USB_DEVICE_STATE_UNSPECIFIED,
            )
            || !optional_bounded_ascii(&device.owner_workload_id, MAX_SERVICE_STRING_BYTES)
            || !optional_bounded_ascii(&device.slot, MAX_SERVICE_STRING_BYTES)
            || !optional_bounded_ascii(&device.media_ref, MAX_SERVICE_STRING_BYTES)
            || device.candidate_device_ids.len() > MAX_DAEMON_USB_DEVICES
            || device.degraded_reasons.len() > MAX_DAEMON_DEGRADED_REASONS
            || !devices.insert(device.device_id.as_str())
        {
            return Err(ServiceContractError::BoundExceeded);
        }
        if !device.owner_workload_id.is_empty() {
            WorkloadId::parse(device.owner_workload_id.clone())
                .map_err(|_| ServiceContractError::InvalidIdentity)?;
        }
        let mut candidates = BTreeSet::new();
        for candidate in &device.candidate_device_ids {
            if !bounded_opaque(candidate, MAX_SERVICE_STRING_BYTES)
                || !candidates.insert(candidate.as_str())
            {
                return Err(ServiceContractError::BoundExceeded);
            }
        }
        for reason in &device.degraded_reasons {
            validate_degraded_reason(reason)?;
        }
    }
    Ok(())
}

fn validate_bridge(value: &daemon::BridgeProjection) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    let valid_ifname = |name: &str| {
        !name.is_empty()
            && name.len() <= 15
            && name.is_ascii()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    };
    if !valid_ifname(&value.bridge) || (!value.tap.is_empty() && !valid_ifname(&value.tap)) {
        return Err(ServiceContractError::BoundExceeded);
    }
    Ok(())
}

fn validate_readiness(value: &daemon::ReadinessProjection) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if !bounded_opaque(&value.role_id, MAX_SERVICE_STRING_BYTES)
        || !bounded_ascii(&value.predicate_id, MAX_DAEMON_DETAIL_BYTES)
        || !valid_required_enum(
            &value.state,
            daemon::ServiceState::SERVICE_STATE_UNSPECIFIED,
        )
    {
        return Err(ServiceContractError::BoundExceeded);
    }
    Ok(())
}

fn validate_workload(value: &daemon::WorkloadProjection) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    validate_workload_identity(
        value
            .identity
            .as_ref()
            .ok_or(ServiceContractError::InvalidIdentity)?,
    )?;
    WorkloadName::parse(value.name.clone()).map_err(|_| ServiceContractError::InvalidIdentity)?;
    if !value.environment.is_empty() {
        WorkloadName::parse(value.environment.clone())
            .map_err(|_| ServiceContractError::InvalidIdentity)?;
    }
    if !value.static_ip.is_empty() && !matches!(value.static_ip.len(), 4 | 16) {
        return Err(ServiceContractError::BoundExceeded);
    }
    if value.static_ip.len() == 4 {
        let bytes: [u8; 4] = value
            .static_ip
            .as_slice()
            .try_into()
            .expect("length checked");
        let _ = IpAddr::from(bytes);
    } else if value.static_ip.len() == 16 {
        let bytes: [u8; 16] = value
            .static_ip
            .as_slice()
            .try_into()
            .expect("length checked");
        let _ = IpAddr::from(bytes);
    }
    validate_lifecycle(
        value
            .lifecycle
            .as_ref()
            .ok_or(ServiceContractError::InconsistentResponse)?,
    )?;
    validate_runtime(
        value
            .runtime
            .as_ref()
            .ok_or(ServiceContractError::InconsistentResponse)?,
    )?;
    if value.services.is_empty()
        || value.services.len() > MAX_DAEMON_SERVICES
        || value.bridge_checks.len() > MAX_DAEMON_BRIDGES
        || value.declared_roles.len() > MAX_DAEMON_SERVICES
        || value.readiness.len() > MAX_DAEMON_READINESS
    {
        return Err(ServiceContractError::BoundExceeded);
    }
    let mut services = BTreeSet::new();
    for service in &value.services {
        validate_service(service)?;
        if !services.insert((service.kind.value(), service.role_id.as_str())) {
            return Err(ServiceContractError::InconsistentResponse);
        }
    }
    if let Some(autostart) = value.autostart.as_ref() {
        validate_autostart(autostart)?;
    }
    if let Some(deployment) = value.deployment.as_ref() {
        validate_deployment(deployment)?;
    }
    if let Some(parity) = value.runner_parity.as_ref() {
        validate_runner_parity(parity)?;
    }
    if !valid_optional_enum(&value.api_ready) {
        return Err(ServiceContractError::InvalidEnum);
    }
    if let Some(integrity) = value.live_pool_integrity.as_ref() {
        validate_live_pool(integrity)?;
    }
    if let Some(qemu) = value.qemu_media.as_ref() {
        validate_qemu_media(qemu)?;
    }
    if let Some(usb) = value.usb.as_ref() {
        validate_usb(usb)?;
    }
    for bridge in &value.bridge_checks {
        validate_bridge(bridge)?;
    }
    let mut roles = BTreeSet::new();
    for role in &value.declared_roles {
        if !bounded_opaque(role, MAX_SERVICE_STRING_BYTES) || !roles.insert(role.as_str()) {
            return Err(ServiceContractError::BoundExceeded);
        }
    }
    for readiness in &value.readiness {
        validate_readiness(readiness)?;
    }
    Ok(())
}

fn validate_workload_response(
    message: &impl Message,
    outcome: &EnumOrUnknown<common::Outcome>,
    workloads: &[daemon::WorkloadProjection],
    page: &MessageField<daemon::PageInfo>,
    error: &MessageField<common::ErrorEnvelope>,
) -> Result<(), ServiceContractError> {
    reject_unknown(message)?;
    validate_daemon_response_shape(outcome, error, page, workloads.len(), MAX_DAEMON_WORKLOADS)?;
    let mut identities = BTreeSet::new();
    for workload in workloads {
        validate_workload(workload)?;
        let id = &workload
            .identity
            .as_ref()
            .ok_or(ServiceContractError::InvalidIdentity)?
            .workload_id;
        if !identities.insert(id.as_str()) {
            return Err(ServiceContractError::InconsistentResponse);
        }
    }
    Ok(())
}

impl StrictWireMessage for daemon::ListWorkloadsResponse {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        validate_workload_response(
            self,
            &self.outcome,
            &self.workloads,
            &self.page,
            &self.error,
        )
    }
}

impl StrictWireMessage for daemon::InspectResponse {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        validate_workload_response(
            self,
            &self.outcome,
            &self.workloads,
            &self.page,
            &self.error,
        )?;
        if !optional_bounded_ascii(&self.read_model, MAX_DAEMON_DETAIL_BYTES) {
            return Err(ServiceContractError::BoundExceeded);
        }
        Ok(())
    }
}

pub fn server_stream_name(stream_id: u16) -> Result<String, ServiceContractError> {
    if stream_id < MIN_NAMED_STREAM_ID {
        return Err(ServiceContractError::InvalidId);
    }
    Ok(format!("stream-{stream_id}"))
}

pub fn parse_server_stream_name(value: &str) -> Result<u16, ServiceContractError> {
    let channel = value
        .strip_prefix("stream-")
        .ok_or(ServiceContractError::InvalidId)?
        .parse::<u16>()
        .map_err(|_| ServiceContractError::InvalidId)?;
    if channel < MIN_NAMED_STREAM_ID || server_stream_name(channel)?.as_str() != value {
        return Err(ServiceContractError::InvalidId);
    }
    Ok(channel)
}

impl StrictWireMessage for terminal::TerminalOpenRequest {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        validate_metadata(required_message(&self.metadata)?, true)?;
        validate_scope(required_message(&self.scope)?)?;
        if !bounded_opaque(&self.resource_id, MAX_SERVICE_STRING_BYTES)
            || !bounded_opaque(&self.operation_id, MAX_SERVICE_STRING_BYTES)
            || !required_digest(&self.request_digest)
        {
            return Err(ServiceContractError::InvalidOperationInput);
        }
        Ok(())
    }
}

impl StrictWireMessage for terminal::TerminalOpenResponse {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        if self.session_generation == 0
            || RequestId::new(self.request_id.clone()).is_err()
            || !bounded_opaque(&self.operation_id, MAX_SERVICE_STRING_BYTES)
        {
            return Err(ServiceContractError::InvalidId);
        }
        let outcome = self
            .outcome
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?;
        match outcome {
            common::Outcome::OUTCOME_ACCEPTED => {
                if self.error.is_some()
                    || !bounded_opaque(&self.resource_handle, MAX_SERVICE_STRING_BYTES)
                {
                    return Err(ServiceContractError::InconsistentResponse);
                }
                parse_server_stream_name(&self.stream_id)?;
                if let Some(range) = self.retained_log.as_ref() {
                    validate_terminal_retained_log_range(range)?;
                }
            }
            common::Outcome::OUTCOME_DENIED
            | common::Outcome::OUTCOME_CANCELLED
            | common::Outcome::OUTCOME_FAILED => {
                if !self.stream_id.is_empty()
                    || !self.resource_handle.is_empty()
                    || self.retained_log.is_some()
                {
                    return Err(ServiceContractError::InconsistentResponse);
                }
                validate_error(
                    self.error
                        .as_ref()
                        .ok_or(ServiceContractError::InconsistentResponse)?,
                )?;
            }
            _ => return Err(ServiceContractError::InconsistentResponse),
        }
        Ok(())
    }
}

pub fn validate_terminal_open_response_for_request(
    request: &terminal::TerminalOpenRequest,
    response: &terminal::TerminalOpenResponse,
) -> Result<(), ServiceContractError> {
    request.validate_wire(true)?;
    response.validate_wire(false)?;
    let metadata = request
        .metadata
        .as_ref()
        .ok_or(ServiceContractError::MissingMetadata)?;
    if response.operation_id != request.operation_id
        || response.request_id != metadata.request_id
        || response.session_generation != metadata.session_generation
        || response.retained_log.is_some()
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

fn validate_terminal_retained_log_range(
    value: &terminal::TerminalRetainedLogRange,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    let requested_end = value
        .requested_offset
        .checked_add(u64::from(value.max_bytes))
        .ok_or(ServiceContractError::BoundExceeded)?;
    if !valid_required_enum(
        &value.output,
        terminal::OutputStream::OUTPUT_STREAM_UNSPECIFIED,
    ) || value.max_bytes == 0
        || value.max_bytes as usize > MAX_TERMINAL_CHUNK_BYTES
        || value.start_offset < value.requested_offset
        || value.start_offset > value.end_offset
        || value.end_offset > requested_end
        || value.end_offset.saturating_sub(value.start_offset) > u64::from(value.max_bytes)
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

fn validate_terminal_size(value: &terminal::TerminalSize) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if value.rows == 0
        || value.columns == 0
        || value.rows > u16::MAX.into()
        || value.columns > u16::MAX.into()
    {
        return Err(ServiceContractError::BoundExceeded);
    }
    Ok(())
}

fn validate_exec_selection(value: &terminal::ExecSelection) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    use terminal::exec_selection::Selection;
    let authority = value
        .authority
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?;
    match (&value.selection, authority) {
        (
            Some(Selection::Arbitrary(arbitrary)),
            terminal::ExecAuthority::EXEC_AUTHORITY_ADMIN_ARBITRARY,
        ) => {
            reject_unknown(arbitrary)?;
            if arbitrary.argv.is_empty() || arbitrary.argv.len() > MAX_TERMINAL_ARGV {
                return Err(ServiceContractError::BoundExceeded);
            }
            let mut total = 0_usize;
            for argument in &arbitrary.argv {
                total = total
                    .checked_add(argument.len())
                    .ok_or(ServiceContractError::BoundExceeded)?;
                if argument.is_empty()
                    || argument.len() > MAX_TERMINAL_ARG_BYTES
                    || argument.contains(&0)
                    || std::str::from_utf8(argument).is_err()
                {
                    return Err(ServiceContractError::BoundExceeded);
                }
            }
            if total > MAX_TERMINAL_ARGV_BYTES {
                return Err(ServiceContractError::BoundExceeded);
            }
        }
        (
            Some(Selection::ConfiguredLaunch(configured)),
            terminal::ExecAuthority::EXEC_AUTHORITY_CONFIGURED_LAUNCH,
        ) => {
            reject_unknown(configured)?;
            if !bounded_opaque(&configured.configured_item_id, MAX_SERVICE_STRING_BYTES) {
                return Err(ServiceContractError::BoundExceeded);
            }
        }
        _ => return Err(ServiceContractError::InvalidOperationInput),
    }
    match (value.tty, value.initial_size.as_ref()) {
        (true, Some(size)) => validate_terminal_size(size)?,
        (false, None) => {}
        _ => return Err(ServiceContractError::InvalidOperationInput),
    }
    if value.detached && value.tty {
        return Err(ServiceContractError::InvalidOperationInput);
    }
    Ok(())
}

fn validate_terminal_selection(
    value: &terminal::TerminalSelection,
) -> Result<terminal::TerminalKind, ServiceContractError> {
    reject_unknown(value)?;
    use terminal::terminal_selection::Selection;
    match value
        .selection
        .as_ref()
        .ok_or(ServiceContractError::MissingOperationInput)?
    {
        Selection::Exec(exec) => {
            validate_exec_selection(exec)?;
            Ok(terminal::TerminalKind::TERMINAL_KIND_EXEC)
        }
        Selection::Shell(shell) => {
            reject_unknown(shell)?;
            let action = shell
                .action
                .enum_value()
                .map_err(|_| ServiceContractError::InvalidEnum)?;
            match action {
                terminal::ShellAction::SHELL_ACTION_ATTACH_DEFAULT
                    if shell.shell_handle.is_empty()
                        && shell.configured_shell_id.is_empty()
                        && shell.initial_size.is_some() =>
                {
                    validate_terminal_size(shell.initial_size.as_ref().expect("checked"))?;
                }
                terminal::ShellAction::SHELL_ACTION_ATTACH_CONFIGURED
                    if shell.shell_handle.is_empty()
                        && bounded_opaque(&shell.configured_shell_id, MAX_SERVICE_STRING_BYTES)
                        && shell.initial_size.is_some() =>
                {
                    validate_terminal_size(shell.initial_size.as_ref().expect("checked"))?;
                }
                terminal::ShellAction::SHELL_ACTION_LIST
                    if shell.shell_handle.is_empty()
                        && shell.configured_shell_id.is_empty()
                        && !shell.force
                        && shell.initial_size.is_none() => {}
                terminal::ShellAction::SHELL_ACTION_DETACH
                | terminal::ShellAction::SHELL_ACTION_KILL
                    if bounded_opaque(&shell.shell_handle, MAX_SERVICE_STRING_BYTES)
                        && shell.configured_shell_id.is_empty()
                        && !shell.force
                        && shell.initial_size.is_none() => {}
                _ => return Err(ServiceContractError::InvalidOperationInput),
            }
            Ok(terminal::TerminalKind::TERMINAL_KIND_SHELL)
        }
        Selection::Console(console) => {
            reject_unknown(console)?;
            validate_terminal_size(
                console
                    .initial_size
                    .as_ref()
                    .ok_or(ServiceContractError::MissingOperationInput)?,
            )?;
            Ok(terminal::TerminalKind::TERMINAL_KIND_CONSOLE)
        }
        Selection::RetainedLog(retained) => {
            reject_unknown(retained)?;
            if !bounded_opaque(&retained.exec_handle, MAX_SERVICE_STRING_BYTES)
                || !valid_required_enum(
                    &retained.output,
                    terminal::OutputStream::OUTPUT_STREAM_UNSPECIFIED,
                )
                || retained.max_bytes == 0
                || retained.max_bytes as usize > MAX_TERMINAL_CHUNK_BYTES
            {
                return Err(ServiceContractError::InvalidOperationInput);
            }
            Ok(terminal::TerminalKind::TERMINAL_KIND_RETAINED_LOG)
        }
    }
}

fn validate_terminal_started(
    value: &terminal::TerminalStarted,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    let kind = value
        .kind
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?;
    if kind == terminal::TerminalKind::TERMINAL_KIND_UNSPECIFIED {
        return Err(ServiceContractError::InvalidEnum);
    }
    let provider = value
        .console_provider
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?;
    if (kind == terminal::TerminalKind::TERMINAL_KIND_CONSOLE)
        != (provider != terminal::ConsoleProviderKind::CONSOLE_PROVIDER_KIND_UNSPECIFIED)
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

fn validate_terminal_stdin(value: &terminal::TerminalStdin) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if value.data.len() > MAX_TERMINAL_CHUNK_BYTES || (value.data.is_empty() && !value.eof) {
        return Err(ServiceContractError::BoundExceeded);
    }
    Ok(())
}

fn validate_terminal_output(value: &terminal::TerminalOutput) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if value.data.len() > MAX_TERMINAL_CHUNK_BYTES || (value.data.is_empty() && !value.eof) {
        return Err(ServiceContractError::BoundExceeded);
    }
    Ok(())
}

fn validate_terminal_resize(value: &terminal::TerminalResize) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if value.operation_sequence == 0 {
        return Err(ServiceContractError::InvalidId);
    }
    validate_terminal_size(
        value
            .size
            .as_ref()
            .ok_or(ServiceContractError::MissingOperationInput)?,
    )
}

fn validate_terminal_signal(value: &terminal::TerminalSignal) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if value.operation_sequence == 0
        || !valid_required_enum(
            &value.signal,
            terminal::TerminalSignalKind::TERMINAL_SIGNAL_KIND_UNSPECIFIED,
        )
    {
        return Err(ServiceContractError::InvalidId);
    }
    Ok(())
}

fn validate_empty_terminal_message(value: &impl Message) -> Result<(), ServiceContractError> {
    reject_unknown(value)
}

fn validate_terminal_status(value: &terminal::TerminalStatus) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if !valid_required_enum(
        &value.status,
        terminal::TerminalStatusKind::TERMINAL_STATUS_KIND_UNSPECIFIED,
    ) {
        return Err(ServiceContractError::InvalidEnum);
    }
    Ok(())
}

fn validate_terminal_outcome(
    value: &terminal::TerminalOutcome,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    use terminal::terminal_outcome::Outcome;
    match value
        .outcome
        .as_ref()
        .ok_or(ServiceContractError::MissingOperationInput)?
    {
        Outcome::Exited(exited) => {
            reject_unknown(exited)?;
            if !(0..=255).contains(&exited.exit_code) {
                return Err(ServiceContractError::BoundExceeded);
            }
        }
        Outcome::Signaled(signaled) => {
            reject_unknown(signaled)?;
            if !(1..=64).contains(&signaled.signal) {
                return Err(ServiceContractError::BoundExceeded);
            }
        }
        Outcome::Cancelled(cancelled) => validate_empty_terminal_message(cancelled)?,
        Outcome::Detached(detached) => validate_empty_terminal_message(detached)?,
        Outcome::Closed(closed) => validate_empty_terminal_message(closed)?,
        Outcome::Failed(failed) => {
            reject_unknown(failed)?;
            if !valid_required_enum(
                &failed.error,
                terminal::TerminalErrorKind::TERMINAL_ERROR_KIND_UNSPECIFIED,
            ) || !valid_required_enum(&failed.retry, common::RetryClass::RETRY_CLASS_UNSPECIFIED)
            {
                return Err(ServiceContractError::InvalidEnum);
            }
        }
    }
    Ok(())
}

fn validate_shell_management_result(
    value: &terminal::ShellManagementResult,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    let action = value
        .action
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?;
    if action == terminal::ShellAction::SHELL_ACTION_UNSPECIFIED
        || value.sessions.len() > MAX_PAGE_SIZE as usize
        || !optional_bounded_ascii(&value.affected_shell_handle, MAX_SERVICE_STRING_BYTES)
        || (value.truncated && action != terminal::ShellAction::SHELL_ACTION_LIST)
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    let mut handles = BTreeSet::new();
    for session in &value.sessions {
        reject_unknown(session)?;
        if !bounded_opaque(&session.shell_handle, MAX_SERVICE_STRING_BYTES)
            || !valid_required_enum(
                &session.state,
                terminal::ShellSessionState::SHELL_SESSION_STATE_UNSPECIFIED,
            )
            || !handles.insert(session.shell_handle.as_str())
        {
            return Err(ServiceContractError::InvalidOperationInput);
        }
    }
    match action {
        terminal::ShellAction::SHELL_ACTION_LIST => {
            if !value.affected_shell_handle.is_empty() || value.applied {
                return Err(ServiceContractError::InconsistentResponse);
            }
        }
        terminal::ShellAction::SHELL_ACTION_ATTACH_DEFAULT
        | terminal::ShellAction::SHELL_ACTION_ATTACH_CONFIGURED
        | terminal::ShellAction::SHELL_ACTION_DETACH
        | terminal::ShellAction::SHELL_ACTION_KILL => {
            if !bounded_opaque(&value.affected_shell_handle, MAX_SERVICE_STRING_BYTES)
                || !value.applied
            {
                return Err(ServiceContractError::InconsistentResponse);
            }
        }
        terminal::ShellAction::SHELL_ACTION_UNSPECIFIED => unreachable!("validated above"),
    }
    Ok(())
}

impl StrictWireMessage for terminal::TerminalStreamFrame {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        if self.session_generation == 0
            || RequestId::new(self.request_id.clone()).is_err()
            || self.sequence > MAX_TERMINAL_FRAME_SEQUENCE
            || !bounded_opaque(&self.operation_id, MAX_SERVICE_STRING_BYTES)
            || !bounded_opaque(&self.resource_handle, MAX_SERVICE_STRING_BYTES)
        {
            return Err(ServiceContractError::InvalidId);
        }
        use terminal::terminal_stream_frame::Frame;
        match self
            .frame
            .as_ref()
            .ok_or(ServiceContractError::MissingOperationInput)?
        {
            Frame::Select(selection) => {
                validate_terminal_selection(selection)?;
            }
            Frame::Started(started) => validate_terminal_started(started)?,
            Frame::Stdin(stdin) => validate_terminal_stdin(stdin)?,
            Frame::Stdout(stdout) | Frame::Stderr(stdout) => validate_terminal_output(stdout)?,
            Frame::Resize(resize) => validate_terminal_resize(resize)?,
            Frame::Signal(signal) => validate_terminal_signal(signal)?,
            Frame::CloseStdin(close) => validate_empty_terminal_message(close)?,
            Frame::Detach(detach) => validate_empty_terminal_message(detach)?,
            Frame::Close(close) => validate_empty_terminal_message(close)?,
            Frame::Cancel(cancel) => validate_empty_terminal_message(cancel)?,
            Frame::Status(status) => validate_terminal_status(status)?,
            Frame::Outcome(outcome) => validate_terminal_outcome(outcome)?,
            Frame::ShellResult(result) => validate_shell_management_result(result)?,
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalFrameDirection {
    ClientToServer,
    ServerToClient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalProtocolState {
    AwaitSelection,
    AwaitStarted,
    Active,
    Closing,
    Terminal,
}

pub struct TerminalStreamValidator {
    kind: terminal::TerminalKind,
    session_generation: u64,
    request_id: [u8; 16],
    operation_id: String,
    resource_handle: String,
    next_client_sequence: u64,
    next_server_sequence: u64,
    state: TerminalProtocolState,
    tty: bool,
    detached_exec: bool,
    shell_action: Option<terminal::ShellAction>,
    retained_log: Option<RetainedLogStreamState>,
}

struct RetainedLogStreamState {
    output: terminal::OutputStream,
    requested_offset: u64,
    next_offset: u64,
    end_offset: u64,
    max_bytes: u32,
    eof: bool,
    saw_eof: bool,
}

impl fmt::Debug for TerminalStreamValidator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TerminalStreamValidator")
            .field("kind", &self.kind)
            .field("session_generation", &"<redacted>")
            .field("request_id", &"<redacted>")
            .field("operation_id", &"<redacted>")
            .field("resource_handle", &"<redacted>")
            .field("state", &self.state)
            .field("tty", &self.tty)
            .finish()
    }
}

impl TerminalStreamValidator {
    pub fn new(
        kind: terminal::TerminalKind,
        session_generation: u64,
        request_id: [u8; 16],
        operation_id: impl Into<String>,
        resource_handle: impl Into<String>,
    ) -> Result<Self, ServiceContractError> {
        let operation_id = operation_id.into();
        let resource_handle = resource_handle.into();
        if kind == terminal::TerminalKind::TERMINAL_KIND_UNSPECIFIED
            || session_generation == 0
            || request_id == [0; 16]
            || !bounded_opaque(&operation_id, MAX_SERVICE_STRING_BYTES)
            || !bounded_opaque(&resource_handle, MAX_SERVICE_STRING_BYTES)
        {
            return Err(ServiceContractError::InvalidId);
        }
        Ok(Self {
            kind,
            session_generation,
            request_id,
            operation_id,
            resource_handle,
            next_client_sequence: 0,
            next_server_sequence: 0,
            state: TerminalProtocolState::AwaitSelection,
            tty: false,
            detached_exec: false,
            shell_action: None,
            retained_log: None,
        })
    }

    pub fn bind_retained_log_range(
        &mut self,
        range: &terminal::TerminalRetainedLogRange,
    ) -> Result<(), ServiceContractError> {
        if self.kind != terminal::TerminalKind::TERMINAL_KIND_RETAINED_LOG
            || self.state != TerminalProtocolState::AwaitSelection
            || self.retained_log.is_some()
        {
            return Err(ServiceContractError::InconsistentResponse);
        }
        validate_terminal_retained_log_range(range)?;
        self.retained_log = Some(RetainedLogStreamState {
            output: range.output.enum_value_or_default(),
            requested_offset: range.requested_offset,
            next_offset: range.start_offset,
            end_offset: range.end_offset,
            max_bytes: range.max_bytes,
            eof: range.eof,
            saw_eof: false,
        });
        Ok(())
    }

    pub fn accept(
        &mut self,
        direction: TerminalFrameDirection,
        frame: &terminal::TerminalStreamFrame,
    ) -> Result<(), ServiceContractError> {
        frame.validate_wire(false)?;
        if frame.session_generation != self.session_generation
            || frame.request_id.as_slice() != self.request_id
            || frame.operation_id != self.operation_id
            || frame.resource_handle != self.resource_handle
        {
            return Err(ServiceContractError::InconsistentResponse);
        }
        let expected_sequence = match direction {
            TerminalFrameDirection::ClientToServer => self.next_client_sequence,
            TerminalFrameDirection::ServerToClient => self.next_server_sequence,
        };
        if frame.sequence != expected_sequence {
            return Err(ServiceContractError::InconsistentResponse);
        }
        self.accept_frame(direction, frame)?;
        let next = expected_sequence
            .checked_add(1)
            .ok_or(ServiceContractError::BoundExceeded)?;
        match direction {
            TerminalFrameDirection::ClientToServer => self.next_client_sequence = next,
            TerminalFrameDirection::ServerToClient => self.next_server_sequence = next,
        }
        Ok(())
    }

    fn accept_frame(
        &mut self,
        direction: TerminalFrameDirection,
        frame: &terminal::TerminalStreamFrame,
    ) -> Result<(), ServiceContractError> {
        use terminal::terminal_selection::Selection;
        use terminal::terminal_stream_frame::Frame;
        let payload = frame
            .frame
            .as_ref()
            .ok_or(ServiceContractError::MissingOperationInput)?;
        match (self.state, direction, payload) {
            (
                TerminalProtocolState::AwaitSelection,
                TerminalFrameDirection::ClientToServer,
                Frame::Select(selection),
            ) => {
                let selected = validate_terminal_selection(selection)?;
                if selected != self.kind {
                    return Err(ServiceContractError::InvalidOperationInput);
                }
                if matches!(
                    selection.selection.as_ref(),
                    Some(Selection::RetainedLog(retained))
                        if retained.exec_handle != self.resource_handle
                            || self.retained_log.as_ref().is_none_or(|binding| {
                                retained.output.enum_value().ok() != Some(binding.output)
                                    || retained.offset != binding.requested_offset
                                    || retained.max_bytes != binding.max_bytes
                            })
                ) {
                    return Err(ServiceContractError::InconsistentResponse);
                }
                if self.kind == terminal::TerminalKind::TERMINAL_KIND_RETAINED_LOG
                    && !matches!(selection.selection, Some(Selection::RetainedLog(_)))
                {
                    return Err(ServiceContractError::InconsistentResponse);
                }
                self.tty = match selection.selection.as_ref() {
                    Some(Selection::Exec(exec)) => {
                        self.detached_exec = exec.detached;
                        exec.tty
                    }
                    Some(Selection::Shell(shell)) => {
                        let action = shell
                            .action
                            .enum_value()
                            .map_err(|_| ServiceContractError::InvalidEnum)?;
                        self.shell_action = Some(action);
                        matches!(
                            action,
                            terminal::ShellAction::SHELL_ACTION_ATTACH_DEFAULT
                                | terminal::ShellAction::SHELL_ACTION_ATTACH_CONFIGURED
                        )
                    }
                    Some(Selection::Console(_)) => true,
                    Some(Selection::RetainedLog(_)) => false,
                    None => return Err(ServiceContractError::MissingOperationInput),
                };
                self.state = TerminalProtocolState::AwaitStarted;
                Ok(())
            }
            (
                TerminalProtocolState::AwaitStarted,
                TerminalFrameDirection::ServerToClient,
                Frame::Started(started),
            ) if started.kind.enum_value().ok() == Some(self.kind) && started.tty == self.tty => {
                if let Some(binding) = self.retained_log.as_ref() {
                    let (selected, other) = match binding.output {
                        terminal::OutputStream::OUTPUT_STREAM_STDOUT => {
                            (started.stdout_offset, started.stderr_offset)
                        }
                        terminal::OutputStream::OUTPUT_STREAM_STDERR => {
                            (started.stderr_offset, started.stdout_offset)
                        }
                        terminal::OutputStream::OUTPUT_STREAM_UNSPECIFIED => {
                            return Err(ServiceContractError::InvalidEnum);
                        }
                    };
                    if selected != binding.next_offset || other != 0 {
                        return Err(ServiceContractError::InconsistentResponse);
                    }
                }
                self.state = TerminalProtocolState::Active;
                Ok(())
            }
            (
                TerminalProtocolState::AwaitStarted,
                TerminalFrameDirection::ServerToClient,
                Frame::ShellResult(result),
            ) if self
                .shell_action
                .is_some_and(|action| action == result.action.enum_value_or_default())
                && !matches!(
                    self.shell_action,
                    Some(
                        terminal::ShellAction::SHELL_ACTION_ATTACH_DEFAULT
                            | terminal::ShellAction::SHELL_ACTION_ATTACH_CONFIGURED
                    )
                ) =>
            {
                self.state = TerminalProtocolState::Closing;
                Ok(())
            }
            (
                TerminalProtocolState::AwaitStarted,
                TerminalFrameDirection::ClientToServer,
                Frame::Detach(_) | Frame::Close(_) | Frame::Cancel(_),
            ) => {
                self.state = TerminalProtocolState::Closing;
                Ok(())
            }
            (
                TerminalProtocolState::Closing,
                TerminalFrameDirection::ServerToClient,
                Frame::Started(started),
            ) if started.kind.enum_value().ok() == Some(self.kind) && started.tty == self.tty => {
                Ok(())
            }
            (
                TerminalProtocolState::AwaitStarted
                | TerminalProtocolState::Active
                | TerminalProtocolState::Closing,
                TerminalFrameDirection::ServerToClient,
                Frame::Outcome(_),
            ) => {
                if self.detached_exec
                    && !matches!(
                        payload,
                        Frame::Outcome(outcome)
                            if matches!(
                                outcome.outcome,
                                Some(terminal::terminal_outcome::Outcome::Detached(_))
                            )
                    )
                {
                    return Err(ServiceContractError::InconsistentResponse);
                }
                if self.retained_log.as_ref().is_some_and(|binding| {
                    binding.next_offset != binding.end_offset || (binding.eof && !binding.saw_eof)
                }) {
                    return Err(ServiceContractError::InconsistentResponse);
                }
                self.state = TerminalProtocolState::Terminal;
                Ok(())
            }
            (
                TerminalProtocolState::Active,
                TerminalFrameDirection::ServerToClient,
                Frame::Stdout(output),
            ) if self.retained_log.as_ref().is_some_and(|binding| {
                binding.output == terminal::OutputStream::OUTPUT_STREAM_STDOUT
            }) =>
            {
                self.accept_retained_log_output(output)
            }
            (
                TerminalProtocolState::Active,
                TerminalFrameDirection::ServerToClient,
                Frame::Stderr(output),
            ) if self.retained_log.as_ref().is_some_and(|binding| {
                binding.output == terminal::OutputStream::OUTPUT_STREAM_STDERR
            }) =>
            {
                self.accept_retained_log_output(output)
            }
            (
                TerminalProtocolState::Active | TerminalProtocolState::Closing,
                TerminalFrameDirection::ServerToClient,
                Frame::Stdout(_) | Frame::Stderr(_) | Frame::Status(_),
            ) if !self.detached_exec && self.retained_log.is_none() => Ok(()),
            (
                TerminalProtocolState::Active,
                TerminalFrameDirection::ClientToServer,
                Frame::Stdin(_) | Frame::CloseStdin(_) | Frame::Signal(_),
            ) if !self.detached_exec
                && self.kind != terminal::TerminalKind::TERMINAL_KIND_RETAINED_LOG =>
            {
                Ok(())
            }
            (
                TerminalProtocolState::Active,
                TerminalFrameDirection::ClientToServer,
                Frame::Resize(_),
            ) if self.tty => Ok(()),
            (
                TerminalProtocolState::Active,
                TerminalFrameDirection::ClientToServer,
                Frame::Detach(_) | Frame::Close(_) | Frame::Cancel(_),
            ) => {
                self.state = TerminalProtocolState::Closing;
                Ok(())
            }
            _ => Err(ServiceContractError::InconsistentResponse),
        }
    }

    fn accept_retained_log_output(
        &mut self,
        output: &terminal::TerminalOutput,
    ) -> Result<(), ServiceContractError> {
        let binding = self
            .retained_log
            .as_mut()
            .ok_or(ServiceContractError::InconsistentResponse)?;
        if output.offset != binding.next_offset || binding.saw_eof {
            return Err(ServiceContractError::InconsistentResponse);
        }
        let len =
            u64::try_from(output.data.len()).map_err(|_| ServiceContractError::BoundExceeded)?;
        let next_offset = binding
            .next_offset
            .checked_add(len)
            .ok_or(ServiceContractError::BoundExceeded)?;
        if next_offset > binding.end_offset
            || next_offset.saturating_sub(binding.requested_offset) > u64::from(binding.max_bytes)
            || (output.eof && (!binding.eof || next_offset != binding.end_offset))
        {
            return Err(ServiceContractError::InconsistentResponse);
        }
        binding.next_offset = next_offset;
        binding.saw_eof = output.eof;
        Ok(())
    }

    pub fn is_terminal(&self) -> bool {
        self.state == TerminalProtocolState::Terminal
    }

    pub fn accept_transport_credit(&self, bytes: u32) -> Result<(), ServiceContractError> {
        if bytes == 0
            || bytes > MAX_NAMED_STREAM_QUEUE_BYTES
            || self.state == TerminalProtocolState::Terminal
        {
            return Err(ServiceContractError::InconsistentResponse);
        }
        Ok(())
    }

    pub fn accept_transport_close(&self) -> Result<(), ServiceContractError> {
        if self.state != TerminalProtocolState::Terminal {
            return Err(ServiceContractError::InconsistentResponse);
        }
        Ok(())
    }

    pub fn accept_transport_reset(&self) -> Result<(), ServiceContractError> {
        self.accept_transport_close()
    }
}

pub struct ServerStreamLease {
    stream_id: u16,
    client_opened: bool,
}

impl ServerStreamLease {
    pub fn reserve(stream_id: u16) -> Result<Self, ServiceContractError> {
        server_stream_name(stream_id)?;
        Ok(Self {
            stream_id,
            client_opened: false,
        })
    }

    pub fn name(&self) -> String {
        server_stream_name(self.stream_id).expect("validated reservation")
    }

    pub fn open_by_client(&mut self, name: &str) -> Result<u16, ServiceContractError> {
        let stream_id = parse_server_stream_name(name)?;
        if self.client_opened || stream_id != self.stream_id {
            return Err(ServiceContractError::InconsistentResponse);
        }
        self.client_opened = true;
        Ok(stream_id)
    }
}

impl fmt::Debug for ServerStreamLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerStreamLease")
            .field("stream_id", &"<redacted>")
            .field("client_opened", &self.client_opened)
            .finish()
    }
}

pub struct RedactedTerminalFrame<'a>(pub &'a terminal::TerminalStreamFrame);

impl fmt::Debug for RedactedTerminalFrame<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        use terminal::terminal_stream_frame::Frame;
        let kind = self.0.frame.as_ref().map(|frame| match frame {
            Frame::Select(_) => "select",
            Frame::Started(_) => "started",
            Frame::Stdin(_) => "stdin",
            Frame::Stdout(_) => "stdout",
            Frame::Stderr(_) => "stderr",
            Frame::Resize(_) => "resize",
            Frame::Signal(_) => "signal",
            Frame::CloseStdin(_) => "close-stdin",
            Frame::Detach(_) => "detach",
            Frame::Close(_) => "close",
            Frame::Cancel(_) => "cancel",
            Frame::Status(_) => "status",
            Frame::Outcome(_) => "outcome",
            Frame::ShellResult(_) => "shell-result",
        });
        formatter
            .debug_struct("TerminalStreamFrame")
            .field("session_generation", &"<redacted>")
            .field("request_id", &"<redacted>")
            .field("operation_id", &"<redacted>")
            .field("resource_handle", &"<redacted>")
            .field("sequence", &self.0.sequence)
            .field("kind", &kind)
            .finish()
    }
}

fn validate_realm_path(value: &str) -> Result<(), ServiceContractError> {
    let labels = value.split('.').collect::<Vec<_>>();
    if value.is_empty()
        || value.len() > 255
        || labels.is_empty()
        || labels.len() > 16
        || labels.iter().any(|label| {
            let mut bytes = label.bytes();
            !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
                || !bytes
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
    {
        return Err(ServiceContractError::InvalidIdentity);
    }
    Ok(())
}

fn validate_lease_owner(value: &broker::LeaseOwner) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    validate_realm_path(&value.realm_path)?;
    if !bounded_opaque(&value.controller_generation_id, MAX_SERVICE_STRING_BYTES)
        || value
            .node_id
            .as_deref()
            .is_some_and(|node| !bounded_opaque(node, MAX_SERVICE_STRING_BYTES))
    {
        return Err(ServiceContractError::InvalidId);
    }
    Ok(())
}

fn validate_acquisition_order(
    value: &broker::ResourceAcquisitionOrder,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if value.phase > u32::from(u16::MAX) || value.ordinal > u32::from(u16::MAX) {
        return Err(ServiceContractError::BoundExceeded);
    }
    Ok(())
}

fn validate_requested_resource(
    value: &broker::LeaseResourceRequest,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if !bounded_opaque(&value.resource_id, MAX_SERVICE_STRING_BYTES) {
        return Err(ServiceContractError::InvalidId);
    }
    if value.kind.enum_value().is_err()
        || value.kind.value() == broker::HostResourceKind::HOST_RESOURCE_KIND_UNSPECIFIED.value()
        || value.share.enum_value().is_err()
        || value.share.value() == broker::ResourceShareMode::RESOURCE_SHARE_MODE_UNSPECIFIED.value()
    {
        return Err(ServiceContractError::InvalidEnum);
    }
    validate_acquisition_order(required_message(&value.acquisition_order)?)
}

impl StrictWireMessage for broker::AllocateRequest {
    fn validate_wire(&self, requires_idempotency: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        validate_metadata(required_message(&self.metadata)?, requires_idempotency)?;
        validate_scope(required_message(&self.scope)?)?;
        validate_lease_owner(required_message(&self.owner)?)?;
        if !bounded_opaque(&self.operation_id, MAX_SERVICE_STRING_BYTES) {
            return Err(ServiceContractError::InvalidId);
        }
        if !required_digest(&self.request_digest) {
            return Err(ServiceContractError::InvalidDigest);
        }
        if self.resources.is_empty() || self.resources.len() > MAX_ALLOCATOR_REQUEST_RESOURCES {
            return Err(ServiceContractError::BoundExceeded);
        }
        let mut resource_ids = BTreeSet::new();
        for resource in &self.resources {
            validate_requested_resource(resource)?;
            if !resource_ids.insert(resource.resource_id.as_str()) {
                return Err(ServiceContractError::InvalidId);
            }
        }
        Ok(())
    }
}

fn validate_granted_resource(
    value: &broker::GrantedHostResource,
) -> Result<Option<u32>, ServiceContractError> {
    reject_unknown(value)?;
    if !bounded_opaque(&value.resource_id, MAX_SERVICE_STRING_BYTES)
        || !bounded_opaque(&value.delegation_id, MAX_SERVICE_STRING_BYTES)
    {
        return Err(ServiceContractError::InvalidId);
    }
    let kind = value
        .kind
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?;
    let share = value
        .share
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?;
    let delegation = value
        .delegation
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?;
    if kind == broker::HostResourceKind::HOST_RESOURCE_KIND_UNSPECIFIED
        || share == broker::ResourceShareMode::RESOURCE_SHARE_MODE_UNSPECIFIED
        || delegation == broker::ResourceDelegationKind::RESOURCE_DELEGATION_KIND_UNSPECIFIED
    {
        return Err(ServiceContractError::InvalidEnum);
    }
    if (delegation == broker::ResourceDelegationKind::RESOURCE_DELEGATION_KIND_FILE_DESCRIPTOR)
        != value.attachment_index.is_some()
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    validate_acquisition_order(required_message(&value.acquisition_order)?)?;
    Ok(value.attachment_index)
}

fn validate_allocator_conflict(
    value: &broker::AllocatorConflict,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if !bounded_opaque(&value.resource_id, MAX_SERVICE_STRING_BYTES)
        || value
            .existing_lease_id
            .as_deref()
            .is_some_and(|lease| !bounded_opaque(lease, MAX_SERVICE_STRING_BYTES))
    {
        return Err(ServiceContractError::InvalidId);
    }
    if value.kind.enum_value().is_err()
        || value.kind.value() == broker::HostResourceKind::HOST_RESOURCE_KIND_UNSPECIFIED.value()
        || value.reason.enum_value().is_err()
        || value.reason.value() == broker::AllocatorReason::ALLOCATOR_REASON_UNSPECIFIED.value()
    {
        return Err(ServiceContractError::InvalidEnum);
    }
    Ok(())
}

impl StrictWireMessage for broker::AllocateResponse {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        let outcome = self
            .outcome
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?;
        let status = self
            .status
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?;
        let reason = self
            .reason
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?;
        if outcome == common::Outcome::OUTCOME_UNSPECIFIED
            || status == broker::AllocationStatus::ALLOCATION_STATUS_UNSPECIFIED
            || !bounded_opaque(&self.operation_id, MAX_SERVICE_STRING_BYTES)
            || self.resources.len() > MAX_ALLOCATOR_REQUEST_RESOURCES
            || self.conflicts.len() > MAX_ALLOCATOR_CONFLICTS
        {
            return Err(ServiceContractError::InvalidEnum);
        }
        let mut resource_ids = BTreeSet::new();
        let mut attachments = Vec::new();
        for resource in &self.resources {
            if !resource_ids.insert(resource.resource_id.as_str()) {
                return Err(ServiceContractError::InvalidId);
            }
            if let Some(index) = validate_granted_resource(resource)? {
                attachments.push(index);
            }
        }
        validate_attachments(&attachments)?;
        for conflict in &self.conflicts {
            validate_allocator_conflict(conflict)?;
        }
        validate_outcome_error(outcome, self.error.as_ref())?;
        match status {
            broker::AllocationStatus::ALLOCATION_STATUS_GRANTED
                if outcome == common::Outcome::OUTCOME_SUCCEEDED
                    && bounded_opaque(&self.lease_id, MAX_SERVICE_STRING_BYTES)
                    && !self.resources.is_empty()
                    && reason == broker::AllocatorReason::ALLOCATOR_REASON_UNSPECIFIED
                    && self.conflicts.is_empty() =>
            {
                Ok(())
            }
            broker::AllocationStatus::ALLOCATION_STATUS_DENIED
                if matches!(
                    outcome,
                    common::Outcome::OUTCOME_DENIED
                        | common::Outcome::OUTCOME_CANCELLED
                        | common::Outcome::OUTCOME_FAILED
                ) && self.lease_id.is_empty()
                    && self.resources.is_empty()
                    && reason != broker::AllocatorReason::ALLOCATOR_REASON_UNSPECIFIED =>
            {
                Ok(())
            }
            _ => Err(ServiceContractError::InconsistentResponse),
        }
    }
}

fn validate_realm_child_fd(
    value: &broker::RealmChildFd,
) -> Result<
    (
        broker::RealmChildRole,
        broker::RealmChildFdKind,
        Option<&str>,
    ),
    ServiceContractError,
> {
    reject_unknown(value)?;
    let role = value
        .role
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?;
    let kind = value
        .kind
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?;
    if role == broker::RealmChildRole::REALM_CHILD_ROLE_UNSPECIFIED
        || kind == broker::RealmChildFdKind::REALM_CHILD_FD_KIND_UNSPECIFIED
    {
        return Err(ServiceContractError::InvalidEnum);
    }
    let resource_scoped = matches!(
        kind,
        broker::RealmChildFdKind::REALM_CHILD_FD_KIND_RESOURCE
            | broker::RealmChildFdKind::REALM_CHILD_FD_KIND_LEASE
    );
    match (resource_scoped, value.resource_id.as_deref()) {
        (true, Some(resource)) if bounded_opaque(resource, MAX_SERVICE_STRING_BYTES) => {}
        (true, Some(_)) => return Err(ServiceContractError::InvalidId),
        (true, None) => return Err(ServiceContractError::MissingOperationInput),
        (false, None) => {}
        (false, Some(_)) => return Err(ServiceContractError::InvalidOperationInput),
    }
    if (kind == broker::RealmChildFdKind::REALM_CHILD_FD_KIND_PUBLIC_LISTENER
        && role != broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER)
        || (kind == broker::RealmChildFdKind::REALM_CHILD_FD_KIND_BROKER_LISTENER
            && role != broker::RealmChildRole::REALM_CHILD_ROLE_BROKER)
    {
        return Err(ServiceContractError::InvalidOperationInput);
    }
    Ok((role, kind, value.resource_id.as_deref()))
}

impl StrictWireMessage for broker::SpawnRealmChildrenRequest {
    fn validate_wire(&self, requires_idempotency: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        validate_metadata(required_message(&self.metadata)?, requires_idempotency)?;
        let scope = required_message(&self.scope)?;
        validate_scope(scope)?;
        RealmId::parse(self.realm_id.clone()).map_err(|_| ServiceContractError::InvalidIdentity)?;
        if scope.realm_id != self.realm_id
            || !scope.workload_id.is_empty()
            || !scope.provider_id.is_empty()
            || !scope.role_id.is_empty()
        {
            return Err(ServiceContractError::InvalidIdentity);
        }
        if !bounded_opaque(&self.operation_id, MAX_SERVICE_STRING_BYTES)
            || !bounded_opaque(&self.controller_generation_id, MAX_SERVICE_STRING_BYTES)
            || !bounded_opaque(&self.controller_process_id, MAX_SERVICE_STRING_BYTES)
            || !bounded_opaque(&self.broker_process_id, MAX_SERVICE_STRING_BYTES)
            || self.controller_process_id == self.broker_process_id
        {
            return Err(ServiceContractError::InvalidId);
        }
        if !required_digest(&self.launch_record_digest) {
            return Err(ServiceContractError::InvalidDigest);
        }
        if self.fds.len() < 4 || self.fds.len() > MAX_REALM_CHILD_FDS {
            return Err(ServiceContractError::BoundExceeded);
        }
        let mut singleton_bindings = BTreeSet::new();
        let mut resource_bindings = BTreeSet::new();
        let mut attachments = Vec::with_capacity(self.fds.len());
        for fd in &self.fds {
            let (role, kind, resource_id) = validate_realm_child_fd(fd)?;
            let unique = match resource_id {
                Some(resource_id) => {
                    resource_bindings.insert((role.value(), kind.value(), resource_id))
                }
                None => singleton_bindings.insert((role.value(), kind.value())),
            };
            if !unique {
                return Err(ServiceContractError::InvalidOperationInput);
            }
            attachments.push(fd.attachment_index);
        }
        validate_attachments(&attachments)?;
        for required in [
            (
                broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER,
                broker::RealmChildFdKind::REALM_CHILD_FD_KIND_PUBLIC_LISTENER,
            ),
            (
                broker::RealmChildRole::REALM_CHILD_ROLE_BROKER,
                broker::RealmChildFdKind::REALM_CHILD_FD_KIND_BROKER_LISTENER,
            ),
            (
                broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER,
                broker::RealmChildFdKind::REALM_CHILD_FD_KIND_BOOTSTRAP_SESSION,
            ),
            (
                broker::RealmChildRole::REALM_CHILD_ROLE_BROKER,
                broker::RealmChildFdKind::REALM_CHILD_FD_KIND_BOOTSTRAP_SESSION,
            ),
        ] {
            if !singleton_bindings.contains(&(required.0.value(), required.1.value())) {
                return Err(ServiceContractError::MissingOperationInput);
            }
        }
        Ok(())
    }
}

fn validate_spawned_child(
    value: &broker::SpawnedRealmChild,
) -> Result<(broker::RealmChildRole, u32, u32), ServiceContractError> {
    reject_unknown(value)?;
    let role = value
        .role
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?;
    if role == broker::RealmChildRole::REALM_CHILD_ROLE_UNSPECIFIED {
        return Err(ServiceContractError::InvalidEnum);
    }
    if !bounded_opaque(&value.process_id, MAX_SERVICE_STRING_BYTES) || value.pid == 0 {
        return Err(ServiceContractError::InvalidId);
    }
    if !required_digest(&value.executable_digest) {
        return Err(ServiceContractError::InvalidDigest);
    }
    Ok((role, value.pidfd_attachment_index, value.pid))
}

impl StrictWireMessage for broker::SpawnRealmChildrenResponse {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        let outcome = self
            .outcome
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?;
        if outcome == common::Outcome::OUTCOME_UNSPECIFIED
            || !bounded_opaque(&self.operation_id, MAX_SERVICE_STRING_BYTES)
        {
            return Err(ServiceContractError::InvalidEnum);
        }
        validate_outcome_error(outcome, self.error.as_ref())?;
        if outcome == common::Outcome::OUTCOME_SUCCEEDED {
            if !required_digest(&self.launch_record_digest) || self.children.len() != 2 {
                return Err(ServiceContractError::InconsistentResponse);
            }
            let mut roles = BTreeSet::new();
            let mut process_ids = BTreeSet::new();
            let mut pids = BTreeSet::new();
            let mut attachments = Vec::with_capacity(2);
            for child in &self.children {
                let (role, attachment, pid) = validate_spawned_child(child)?;
                if !roles.insert(role.value())
                    || !process_ids.insert(child.process_id.as_str())
                    || !pids.insert(pid)
                {
                    return Err(ServiceContractError::InconsistentResponse);
                }
                attachments.push(attachment);
            }
            validate_attachments(&attachments)?;
            for child in &self.children {
                let expected_attachment = match child
                    .role
                    .enum_value()
                    .map_err(|_| ServiceContractError::InvalidEnum)?
                {
                    broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER => {
                        CONTROLLER_PIDFD_ATTACHMENT_INDEX
                    }
                    broker::RealmChildRole::REALM_CHILD_ROLE_BROKER => {
                        BROKER_PIDFD_ATTACHMENT_INDEX
                    }
                    broker::RealmChildRole::REALM_CHILD_ROLE_UNSPECIFIED => {
                        return Err(ServiceContractError::InvalidEnum);
                    }
                };
                if child.pidfd_attachment_index != expected_attachment {
                    return Err(ServiceContractError::InconsistentResponse);
                }
            }
            if !roles.contains(&broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER.value())
                || !roles.contains(&broker::RealmChildRole::REALM_CHILD_ROLE_BROKER.value())
            {
                return Err(ServiceContractError::InconsistentResponse);
            }
            Ok(())
        } else if matches!(
            outcome,
            common::Outcome::OUTCOME_DENIED
                | common::Outcome::OUTCOME_CANCELLED
                | common::Outcome::OUTCOME_FAILED
        ) && self.launch_record_digest.is_empty()
            && self.children.is_empty()
        {
            Ok(())
        } else {
            Err(ServiceContractError::InconsistentResponse)
        }
    }
}

pub fn validate_spawn_response_for_request(
    request: &broker::SpawnRealmChildrenRequest,
    response: &broker::SpawnRealmChildrenResponse,
) -> Result<(), ServiceContractError> {
    request.validate_wire(true)?;
    response.validate_wire(false)?;
    if response.operation_id != request.operation_id {
        return Err(ServiceContractError::InconsistentResponse);
    }
    if response.outcome.enum_value().ok() != Some(common::Outcome::OUTCOME_SUCCEEDED) {
        return Ok(());
    }
    if response.launch_record_digest != request.launch_record_digest {
        return Err(ServiceContractError::InconsistentResponse);
    }
    for child in &response.children {
        let expected_process_id = match child
            .role
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?
        {
            broker::RealmChildRole::REALM_CHILD_ROLE_CONTROLLER => {
                request.controller_process_id.as_str()
            }
            broker::RealmChildRole::REALM_CHILD_ROLE_BROKER => request.broker_process_id.as_str(),
            broker::RealmChildRole::REALM_CHILD_ROLE_UNSPECIFIED => {
                return Err(ServiceContractError::InvalidEnum);
            }
        };
        if child.process_id != expected_process_id {
            return Err(ServiceContractError::InconsistentResponse);
        }
    }
    Ok(())
}

pub fn decode_spawn_response_for_request(
    request: &broker::SpawnRealmChildrenRequest,
    bytes: &[u8],
) -> Result<broker::SpawnRealmChildrenResponse, ServiceContractError> {
    let response = decode_strict(bytes, false)?;
    validate_spawn_response_for_request(request, &response)?;
    Ok(response)
}

fn validate_provider_context(
    value: &common::ProviderOperationContext,
    requires_idempotency: bool,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    validate_metadata(required_message(&value.metadata)?, requires_idempotency)?;
    validate_scope(required_message(&value.scope)?)?;
    ProviderId::parse(value.provider_id.clone())
        .map_err(|_| ServiceContractError::InvalidIdentity)?;
    if !bounded_opaque(&value.operation_id, MAX_SERVICE_STRING_BYTES)
        || Generation::new(value.provider_generation).is_err()
        || value.policy_epoch == 0
    {
        return Err(ServiceContractError::InvalidId);
    }
    if value.provider_type.enum_value().is_err()
        || value.provider_type.value() == common::ProviderType::PROVIDER_TYPE_UNSPECIFIED.value()
    {
        return Err(ServiceContractError::InvalidEnum);
    }
    if !required_digest(&value.authorization_digest) || !required_digest(&value.request_digest) {
        return Err(ServiceContractError::InvalidDigest);
    }
    Ok(())
}

pub fn provider_type(
    value: &common::ProviderOperationContext,
) -> Result<IdentityProviderType, ServiceContractError> {
    value
        .provider_type
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)
        .and_then(|provider_type| match provider_type {
            common::ProviderType::PROVIDER_TYPE_RUNTIME => Ok(IdentityProviderType::Runtime),
            common::ProviderType::PROVIDER_TYPE_INFRASTRUCTURE => {
                Ok(IdentityProviderType::Infrastructure)
            }
            common::ProviderType::PROVIDER_TYPE_TRANSPORT => Ok(IdentityProviderType::Transport),
            common::ProviderType::PROVIDER_TYPE_SUBSTRATE => Ok(IdentityProviderType::Substrate),
            common::ProviderType::PROVIDER_TYPE_CREDENTIAL => Ok(IdentityProviderType::Credential),
            common::ProviderType::PROVIDER_TYPE_DISPLAY => Ok(IdentityProviderType::Display),
            common::ProviderType::PROVIDER_TYPE_NETWORK => Ok(IdentityProviderType::Network),
            common::ProviderType::PROVIDER_TYPE_STORAGE => Ok(IdentityProviderType::Storage),
            common::ProviderType::PROVIDER_TYPE_DEVICE => Ok(IdentityProviderType::Device),
            common::ProviderType::PROVIDER_TYPE_AUDIO => Ok(IdentityProviderType::Audio),
            common::ProviderType::PROVIDER_TYPE_OBSERVABILITY => {
                Ok(IdentityProviderType::Observability)
            }
            common::ProviderType::PROVIDER_TYPE_UNSPECIFIED => {
                Err(ServiceContractError::InvalidEnum)
            }
        })
}

pub fn provider_operation_input(
    value: &common::ProviderOperationInput,
) -> Result<CanonicalProviderOperationInput, ServiceContractError> {
    use common::provider_operation_input::Input;

    reject_unknown(value)?;
    let input = match value
        .input
        .as_ref()
        .ok_or(ServiceContractError::MissingOperationInput)?
    {
        Input::NoInput(value) => {
            reject_unknown(value)?;
            CanonicalProviderOperationInput::NoInput
        }
        Input::ConfiguredRuntimeExecution(value) => {
            reject_unknown(value)?;
            CanonicalProviderOperationInput::ConfiguredRuntimeExecution {
                configured_item_id: ConfiguredItemId::parse(value.configured_item_id.clone())
                    .map_err(|_| ServiceContractError::InvalidId)?,
            }
        }
        Input::InfrastructurePowerState(value) => {
            reject_unknown(value)?;
            let state = match value
                .state
                .enum_value()
                .map_err(|_| ServiceContractError::InvalidEnum)?
            {
                common::InfrastructurePowerState::INFRASTRUCTURE_POWER_STATE_RUNNING => {
                    CanonicalInfrastructurePowerState::Running
                }
                common::InfrastructurePowerState::INFRASTRUCTURE_POWER_STATE_STOPPED => {
                    CanonicalInfrastructurePowerState::Stopped
                }
                common::InfrastructurePowerState::INFRASTRUCTURE_POWER_STATE_UNSPECIFIED => {
                    return Err(ServiceContractError::InvalidEnum);
                }
            };
            CanonicalProviderOperationInput::InfrastructurePowerState { state }
        }
        Input::TransportBinding(value) => {
            reject_unknown(value)?;
            CanonicalProviderOperationInput::TransportBinding {
                transport_binding_id: TransportBindingId::parse(value.transport_binding_id.clone())
                    .map_err(|_| ServiceContractError::InvalidId)?,
            }
        }
        Input::StorageSnapshot(value) => {
            reject_unknown(value)?;
            CanonicalProviderOperationInput::StorageSnapshot {
                snapshot_id: StorageSnapshotId::parse(value.snapshot_id.clone())
                    .map_err(|_| ServiceContractError::InvalidId)?,
            }
        }
        Input::DeviceSelector(value) => {
            reject_unknown(value)?;
            CanonicalProviderOperationInput::DeviceSelector {
                device_selector_id: DeviceSelectorId::parse(value.device_selector_id.clone())
                    .map_err(|_| ServiceContractError::InvalidId)?,
            }
        }
        Input::AudioState(value) => {
            reject_unknown(value)?;
            let channel = match value
                .channel
                .enum_value()
                .map_err(|_| ServiceContractError::InvalidEnum)?
            {
                common::AudioChannel::AUDIO_CHANNEL_SPEAKER => CanonicalAudioChannel::Speaker,
                common::AudioChannel::AUDIO_CHANNEL_MICROPHONE => CanonicalAudioChannel::Microphone,
                common::AudioChannel::AUDIO_CHANNEL_UNSPECIFIED => {
                    return Err(ServiceContractError::InvalidEnum);
                }
            };
            let direction = match value
                .direction
                .enum_value()
                .map_err(|_| ServiceContractError::InvalidEnum)?
            {
                common::AudioDirection::AUDIO_DIRECTION_OUTPUT => CanonicalAudioDirection::Output,
                common::AudioDirection::AUDIO_DIRECTION_INPUT => CanonicalAudioDirection::Input,
                common::AudioDirection::AUDIO_DIRECTION_UNSPECIFIED => {
                    return Err(ServiceContractError::InvalidEnum);
                }
            };
            CanonicalProviderOperationInput::AudioState {
                channel,
                direction,
                mute: value.mute,
                volume: value
                    .volume
                    .map(u8::try_from)
                    .transpose()
                    .map_err(|_| ServiceContractError::BoundExceeded)?,
            }
        }
        Input::ObservabilityQuery(value) => {
            reject_unknown(value)?;
            let view = match value
                .view
                .enum_value()
                .map_err(|_| ServiceContractError::InvalidEnum)?
            {
                common::ObservabilityView::OBSERVABILITY_VIEW_HEALTH => {
                    CanonicalObservabilityView::Health
                }
                common::ObservabilityView::OBSERVABILITY_VIEW_LIFECYCLE => {
                    CanonicalObservabilityView::Lifecycle
                }
                common::ObservabilityView::OBSERVABILITY_VIEW_OPERATIONS => {
                    CanonicalObservabilityView::Operations
                }
                common::ObservabilityView::OBSERVABILITY_VIEW_UNSPECIFIED => {
                    return Err(ServiceContractError::InvalidEnum);
                }
            };
            CanonicalProviderOperationInput::ObservabilityQuery {
                view,
                cursor: value
                    .cursor
                    .as_ref()
                    .map(|cursor| ObservabilityCursor::parse(cursor.clone()))
                    .transpose()
                    .map_err(|_| ServiceContractError::InvalidId)?,
                limit: u16::try_from(value.limit)
                    .map_err(|_| ServiceContractError::BoundExceeded)?,
            }
        }
        Input::ObservabilityExport(value) => {
            reject_unknown(value)?;
            let format = match value
                .format
                .enum_value()
                .map_err(|_| ServiceContractError::InvalidEnum)?
            {
                common::ObservabilityExportFormat::OBSERVABILITY_EXPORT_FORMAT_JSON_LINES => {
                    CanonicalObservabilityExportFormat::JsonLines
                }
                common::ObservabilityExportFormat::OBSERVABILITY_EXPORT_FORMAT_OTLP_PROTOBUF => {
                    CanonicalObservabilityExportFormat::OtlpProtobuf
                }
                common::ObservabilityExportFormat::OBSERVABILITY_EXPORT_FORMAT_UNSPECIFIED => {
                    return Err(ServiceContractError::InvalidEnum);
                }
            };
            CanonicalProviderOperationInput::ObservabilityExport {
                format,
                start_at_unix_ms: value.start_at_unix_ms,
                end_at_unix_ms: value.end_at_unix_ms,
            }
        }
    };
    input.validate().map_err(|error| match error {
        crate::v2_provider::ProviderContractError::BoundExceeded => {
            ServiceContractError::BoundExceeded
        }
        crate::v2_provider::ProviderContractError::InvalidTimeRange => {
            ServiceContractError::InvalidDeadline
        }
        _ => ServiceContractError::InvalidOperationInput,
    })?;
    Ok(input)
}

fn provider_type_from_wire(
    value: EnumOrUnknown<common::ProviderType>,
) -> Result<IdentityProviderType, ServiceContractError> {
    match value
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?
    {
        common::ProviderType::PROVIDER_TYPE_RUNTIME => Ok(IdentityProviderType::Runtime),
        common::ProviderType::PROVIDER_TYPE_INFRASTRUCTURE => {
            Ok(IdentityProviderType::Infrastructure)
        }
        common::ProviderType::PROVIDER_TYPE_TRANSPORT => Ok(IdentityProviderType::Transport),
        common::ProviderType::PROVIDER_TYPE_SUBSTRATE => Ok(IdentityProviderType::Substrate),
        common::ProviderType::PROVIDER_TYPE_CREDENTIAL => Ok(IdentityProviderType::Credential),
        common::ProviderType::PROVIDER_TYPE_DISPLAY => Ok(IdentityProviderType::Display),
        common::ProviderType::PROVIDER_TYPE_NETWORK => Ok(IdentityProviderType::Network),
        common::ProviderType::PROVIDER_TYPE_STORAGE => Ok(IdentityProviderType::Storage),
        common::ProviderType::PROVIDER_TYPE_DEVICE => Ok(IdentityProviderType::Device),
        common::ProviderType::PROVIDER_TYPE_AUDIO => Ok(IdentityProviderType::Audio),
        common::ProviderType::PROVIDER_TYPE_OBSERVABILITY => {
            Ok(IdentityProviderType::Observability)
        }
        common::ProviderType::PROVIDER_TYPE_UNSPECIFIED => Err(ServiceContractError::InvalidEnum),
    }
}

fn provider_type_to_wire(value: IdentityProviderType) -> common::ProviderType {
    match value {
        IdentityProviderType::Runtime => common::ProviderType::PROVIDER_TYPE_RUNTIME,
        IdentityProviderType::Infrastructure => common::ProviderType::PROVIDER_TYPE_INFRASTRUCTURE,
        IdentityProviderType::Transport => common::ProviderType::PROVIDER_TYPE_TRANSPORT,
        IdentityProviderType::Substrate => common::ProviderType::PROVIDER_TYPE_SUBSTRATE,
        IdentityProviderType::Credential => common::ProviderType::PROVIDER_TYPE_CREDENTIAL,
        IdentityProviderType::Display => common::ProviderType::PROVIDER_TYPE_DISPLAY,
        IdentityProviderType::Network => common::ProviderType::PROVIDER_TYPE_NETWORK,
        IdentityProviderType::Storage => common::ProviderType::PROVIDER_TYPE_STORAGE,
        IdentityProviderType::Device => common::ProviderType::PROVIDER_TYPE_DEVICE,
        IdentityProviderType::Audio => common::ProviderType::PROVIDER_TYPE_AUDIO,
        IdentityProviderType::Observability => common::ProviderType::PROVIDER_TYPE_OBSERVABILITY,
    }
}

fn projection_from_wire(
    value: EnumOrUnknown<common::ObservabilityProjectionKind>,
) -> Result<ObservabilityProjectionKind, ServiceContractError> {
    match value
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?
    {
        common::ObservabilityProjectionKind::OBSERVABILITY_PROJECTION_KIND_METRICS => {
            Ok(ObservabilityProjectionKind::Metrics)
        }
        common::ObservabilityProjectionKind::OBSERVABILITY_PROJECTION_KIND_TRACE_SUMMARY => {
            Ok(ObservabilityProjectionKind::TraceSummary)
        }
        common::ObservabilityProjectionKind::OBSERVABILITY_PROJECTION_KIND_AUDIT_SUMMARY => {
            Ok(ObservabilityProjectionKind::AuditSummary)
        }
        common::ObservabilityProjectionKind::OBSERVABILITY_PROJECTION_KIND_UNSPECIFIED => {
            Err(ServiceContractError::InvalidEnum)
        }
    }
}

fn projection_to_wire(value: ObservabilityProjectionKind) -> common::ObservabilityProjectionKind {
    match value {
        ObservabilityProjectionKind::Metrics => {
            common::ObservabilityProjectionKind::OBSERVABILITY_PROJECTION_KIND_METRICS
        }
        ObservabilityProjectionKind::TraceSummary => {
            common::ObservabilityProjectionKind::OBSERVABILITY_PROJECTION_KIND_TRACE_SUMMARY
        }
        ObservabilityProjectionKind::AuditSummary => {
            common::ObservabilityProjectionKind::OBSERVABILITY_PROJECTION_KIND_AUDIT_SUMMARY
        }
    }
}

fn metric_from_wire(
    value: EnumOrUnknown<common::ObservabilityMetricLabel>,
) -> Result<ObservabilityMetricLabel, ServiceContractError> {
    match value
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?
    {
        common::ObservabilityMetricLabel::OBSERVABILITY_METRIC_LABEL_PROVIDER_HEALTH => {
            Ok(ObservabilityMetricLabel::ProviderHealth)
        }
        common::ObservabilityMetricLabel::OBSERVABILITY_METRIC_LABEL_LIFECYCLE_TRANSITION => {
            Ok(ObservabilityMetricLabel::LifecycleTransition)
        }
        common::ObservabilityMetricLabel::OBSERVABILITY_METRIC_LABEL_OPERATION_TOTAL => {
            Ok(ObservabilityMetricLabel::OperationTotal)
        }
        common::ObservabilityMetricLabel::OBSERVABILITY_METRIC_LABEL_OPERATION_DURATION => {
            Ok(ObservabilityMetricLabel::OperationDuration)
        }
        common::ObservabilityMetricLabel::OBSERVABILITY_METRIC_LABEL_QUEUE_DEPTH => {
            Ok(ObservabilityMetricLabel::QueueDepth)
        }
        common::ObservabilityMetricLabel::OBSERVABILITY_METRIC_LABEL_EXPORT_TRUNCATED => {
            Ok(ObservabilityMetricLabel::ExportTruncated)
        }
        common::ObservabilityMetricLabel::OBSERVABILITY_METRIC_LABEL_UNSPECIFIED => {
            Err(ServiceContractError::InvalidEnum)
        }
    }
}

fn metric_to_wire(value: ObservabilityMetricLabel) -> common::ObservabilityMetricLabel {
    match value {
        ObservabilityMetricLabel::ProviderHealth => {
            common::ObservabilityMetricLabel::OBSERVABILITY_METRIC_LABEL_PROVIDER_HEALTH
        }
        ObservabilityMetricLabel::LifecycleTransition => {
            common::ObservabilityMetricLabel::OBSERVABILITY_METRIC_LABEL_LIFECYCLE_TRANSITION
        }
        ObservabilityMetricLabel::OperationTotal => {
            common::ObservabilityMetricLabel::OBSERVABILITY_METRIC_LABEL_OPERATION_TOTAL
        }
        ObservabilityMetricLabel::OperationDuration => {
            common::ObservabilityMetricLabel::OBSERVABILITY_METRIC_LABEL_OPERATION_DURATION
        }
        ObservabilityMetricLabel::QueueDepth => {
            common::ObservabilityMetricLabel::OBSERVABILITY_METRIC_LABEL_QUEUE_DEPTH
        }
        ObservabilityMetricLabel::ExportTruncated => {
            common::ObservabilityMetricLabel::OBSERVABILITY_METRIC_LABEL_EXPORT_TRUNCATED
        }
    }
}

fn operation_label_from_wire(
    value: EnumOrUnknown<common::ObservabilityOperationLabel>,
) -> Result<ObservabilityOperationLabel, ServiceContractError> {
    match value
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?
    {
        common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_HEALTH => {
            Ok(ObservabilityOperationLabel::Health)
        }
        common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_PLAN => {
            Ok(ObservabilityOperationLabel::Plan)
        }
        common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_ENSURE => {
            Ok(ObservabilityOperationLabel::Ensure)
        }
        common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_START => {
            Ok(ObservabilityOperationLabel::Start)
        }
        common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_STOP => {
            Ok(ObservabilityOperationLabel::Stop)
        }
        common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_ATTACH => {
            Ok(ObservabilityOperationLabel::Attach)
        }
        common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_DETACH => {
            Ok(ObservabilityOperationLabel::Detach)
        }
        common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_ADOPT => {
            Ok(ObservabilityOperationLabel::Adopt)
        }
        common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_INSPECT => {
            Ok(ObservabilityOperationLabel::Inspect)
        }
        common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_SET_STATE => {
            Ok(ObservabilityOperationLabel::SetState)
        }
        common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_QUERY => {
            Ok(ObservabilityOperationLabel::Query)
        }
        common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_EXPORT => {
            Ok(ObservabilityOperationLabel::Export)
        }
        common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_UNSPECIFIED => {
            Err(ServiceContractError::InvalidEnum)
        }
    }
}

fn operation_label_to_wire(
    value: ObservabilityOperationLabel,
) -> common::ObservabilityOperationLabel {
    match value {
        ObservabilityOperationLabel::Health => {
            common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_HEALTH
        }
        ObservabilityOperationLabel::Plan => {
            common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_PLAN
        }
        ObservabilityOperationLabel::Ensure => {
            common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_ENSURE
        }
        ObservabilityOperationLabel::Start => {
            common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_START
        }
        ObservabilityOperationLabel::Stop => {
            common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_STOP
        }
        ObservabilityOperationLabel::Attach => {
            common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_ATTACH
        }
        ObservabilityOperationLabel::Detach => {
            common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_DETACH
        }
        ObservabilityOperationLabel::Adopt => {
            common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_ADOPT
        }
        ObservabilityOperationLabel::Inspect => {
            common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_INSPECT
        }
        ObservabilityOperationLabel::SetState => {
            common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_SET_STATE
        }
        ObservabilityOperationLabel::Query => {
            common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_QUERY
        }
        ObservabilityOperationLabel::Export => {
            common::ObservabilityOperationLabel::OBSERVABILITY_OPERATION_LABEL_EXPORT
        }
    }
}

fn outcome_label_from_wire(
    value: EnumOrUnknown<common::ObservabilityOutcomeLabel>,
) -> Result<ObservabilityOutcomeLabel, ServiceContractError> {
    match value
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?
    {
        common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_SUCCESS => {
            Ok(ObservabilityOutcomeLabel::Success)
        }
        common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_ALREADY_APPLIED => {
            Ok(ObservabilityOutcomeLabel::AlreadyApplied)
        }
        common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_DENIED => {
            Ok(ObservabilityOutcomeLabel::Denied)
        }
        common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_CANCELLED => {
            Ok(ObservabilityOutcomeLabel::Cancelled)
        }
        common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_DEADLINE_EXPIRED => {
            Ok(ObservabilityOutcomeLabel::DeadlineExpired)
        }
        common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_UNAVAILABLE => {
            Ok(ObservabilityOutcomeLabel::Unavailable)
        }
        common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_TRUNCATED => {
            Ok(ObservabilityOutcomeLabel::Truncated)
        }
        common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_UNSPECIFIED => {
            Err(ServiceContractError::InvalidEnum)
        }
    }
}

fn outcome_label_to_wire(value: ObservabilityOutcomeLabel) -> common::ObservabilityOutcomeLabel {
    match value {
        ObservabilityOutcomeLabel::Success => {
            common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_SUCCESS
        }
        ObservabilityOutcomeLabel::AlreadyApplied => {
            common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_ALREADY_APPLIED
        }
        ObservabilityOutcomeLabel::Denied => {
            common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_DENIED
        }
        ObservabilityOutcomeLabel::Cancelled => {
            common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_CANCELLED
        }
        ObservabilityOutcomeLabel::DeadlineExpired => {
            common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_DEADLINE_EXPIRED
        }
        ObservabilityOutcomeLabel::Unavailable => {
            common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_UNAVAILABLE
        }
        ObservabilityOutcomeLabel::Truncated => {
            common::ObservabilityOutcomeLabel::OBSERVABILITY_OUTCOME_LABEL_TRUNCATED
        }
    }
}

fn health_state_from_wire(
    value: EnumOrUnknown<common::ObservabilityHealthState>,
) -> Result<ProviderHealthState, ServiceContractError> {
    match value
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?
    {
        common::ObservabilityHealthState::OBSERVABILITY_HEALTH_STATE_HEALTHY => {
            Ok(ProviderHealthState::Healthy)
        }
        common::ObservabilityHealthState::OBSERVABILITY_HEALTH_STATE_DEGRADED => {
            Ok(ProviderHealthState::Degraded)
        }
        common::ObservabilityHealthState::OBSERVABILITY_HEALTH_STATE_UNAVAILABLE => {
            Ok(ProviderHealthState::Unavailable)
        }
        common::ObservabilityHealthState::OBSERVABILITY_HEALTH_STATE_FAILED => {
            Ok(ProviderHealthState::Failed)
        }
        common::ObservabilityHealthState::OBSERVABILITY_HEALTH_STATE_UNSPECIFIED => {
            Err(ServiceContractError::InvalidEnum)
        }
    }
}

fn health_state_to_wire(value: ProviderHealthState) -> common::ObservabilityHealthState {
    match value {
        ProviderHealthState::Healthy => {
            common::ObservabilityHealthState::OBSERVABILITY_HEALTH_STATE_HEALTHY
        }
        ProviderHealthState::Degraded => {
            common::ObservabilityHealthState::OBSERVABILITY_HEALTH_STATE_DEGRADED
        }
        ProviderHealthState::Unavailable => {
            common::ObservabilityHealthState::OBSERVABILITY_HEALTH_STATE_UNAVAILABLE
        }
        ProviderHealthState::Failed => {
            common::ObservabilityHealthState::OBSERVABILITY_HEALTH_STATE_FAILED
        }
    }
}

fn lifecycle_from_wire(
    value: EnumOrUnknown<common::ObservabilityLifecycleState>,
) -> Result<ObservedLifecycleState, ServiceContractError> {
    match value
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?
    {
        common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_PLANNED => {
            Ok(ObservedLifecycleState::Planned)
        }
        common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_READY => {
            Ok(ObservedLifecycleState::Ready)
        }
        common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_RUNNING => {
            Ok(ObservedLifecycleState::Running)
        }
        common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_STOPPED => {
            Ok(ObservedLifecycleState::Stopped)
        }
        common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_RELEASED => {
            Ok(ObservedLifecycleState::Released)
        }
        common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_DESTROYED => {
            Ok(ObservedLifecycleState::Destroyed)
        }
        common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_UNKNOWN => {
            Ok(ObservedLifecycleState::Unknown)
        }
        common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_QUARANTINED => {
            Ok(ObservedLifecycleState::Quarantined)
        }
        common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_UNSPECIFIED => {
            Err(ServiceContractError::InvalidEnum)
        }
    }
}

fn lifecycle_to_wire(value: ObservedLifecycleState) -> common::ObservabilityLifecycleState {
    match value {
        ObservedLifecycleState::Planned => {
            common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_PLANNED
        }
        ObservedLifecycleState::Ready => {
            common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_READY
        }
        ObservedLifecycleState::Running => {
            common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_RUNNING
        }
        ObservedLifecycleState::Stopped => {
            common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_STOPPED
        }
        ObservedLifecycleState::Released => {
            common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_RELEASED
        }
        ObservedLifecycleState::Destroyed => {
            common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_DESTROYED
        }
        ObservedLifecycleState::Unknown => {
            common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_UNKNOWN
        }
        ObservedLifecycleState::Quarantined => {
            common::ObservabilityLifecycleState::OBSERVABILITY_LIFECYCLE_STATE_QUARANTINED
        }
    }
}

fn adoption_from_wire(
    value: EnumOrUnknown<common::ObservabilityAdoptionState>,
) -> Result<AdoptionState, ServiceContractError> {
    match value
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?
    {
        common::ObservabilityAdoptionState::OBSERVABILITY_ADOPTION_STATE_NOT_ATTEMPTED => {
            Ok(AdoptionState::NotAttempted)
        }
        common::ObservabilityAdoptionState::OBSERVABILITY_ADOPTION_STATE_ADOPTED => {
            Ok(AdoptionState::Adopted)
        }
        common::ObservabilityAdoptionState::OBSERVABILITY_ADOPTION_STATE_REJECTED => {
            Ok(AdoptionState::Rejected)
        }
        common::ObservabilityAdoptionState::OBSERVABILITY_ADOPTION_STATE_AMBIGUOUS => {
            Ok(AdoptionState::Ambiguous)
        }
        common::ObservabilityAdoptionState::OBSERVABILITY_ADOPTION_STATE_UNSPECIFIED => {
            Err(ServiceContractError::InvalidEnum)
        }
    }
}

fn adoption_to_wire(value: AdoptionState) -> common::ObservabilityAdoptionState {
    match value {
        AdoptionState::NotAttempted => {
            common::ObservabilityAdoptionState::OBSERVABILITY_ADOPTION_STATE_NOT_ATTEMPTED
        }
        AdoptionState::Adopted => {
            common::ObservabilityAdoptionState::OBSERVABILITY_ADOPTION_STATE_ADOPTED
        }
        AdoptionState::Rejected => {
            common::ObservabilityAdoptionState::OBSERVABILITY_ADOPTION_STATE_REJECTED
        }
        AdoptionState::Ambiguous => {
            common::ObservabilityAdoptionState::OBSERVABILITY_ADOPTION_STATE_AMBIGUOUS
        }
    }
}

fn observation_reason_from_wire(
    value: EnumOrUnknown<common::ObservabilityObservationReason>,
) -> Result<ObservationReason, ServiceContractError> {
    match value
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?
    {
        common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_NONE => {
            Ok(ObservationReason::None)
        }
        common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_IDENTITY_MISMATCH => {
            Ok(ObservationReason::IdentityMismatch)
        }
        common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_CONFIGURATION_MISMATCH => {
            Ok(ObservationReason::ConfigurationMismatch)
        }
        common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_GENERATION_MISMATCH => {
            Ok(ObservationReason::GenerationMismatch)
        }
        common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_OWNER_MISMATCH => {
            Ok(ObservationReason::OwnerMismatch)
        }
        common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_MULTIPLE_CANDIDATES => {
            Ok(ObservationReason::MultipleCandidates)
        }
        common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_MISSING_EVIDENCE => {
            Ok(ObservationReason::MissingEvidence)
        }
        common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_CANCELLED => {
            Ok(ObservationReason::Cancelled)
        }
        common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_DEADLINE_EXPIRED => {
            Ok(ObservationReason::DeadlineExpired)
        }
        common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_UNSPECIFIED => {
            Err(ServiceContractError::InvalidEnum)
        }
    }
}

fn observation_reason_to_wire(value: ObservationReason) -> common::ObservabilityObservationReason {
    match value {
        ObservationReason::None => {
            common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_NONE
        }
        ObservationReason::IdentityMismatch => {
            common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_IDENTITY_MISMATCH
        }
        ObservationReason::ConfigurationMismatch => {
            common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_CONFIGURATION_MISMATCH
        }
        ObservationReason::GenerationMismatch => {
            common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_GENERATION_MISMATCH
        }
        ObservationReason::OwnerMismatch => {
            common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_OWNER_MISMATCH
        }
        ObservationReason::MultipleCandidates => {
            common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_MULTIPLE_CANDIDATES
        }
        ObservationReason::MissingEvidence => {
            common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_MISSING_EVIDENCE
        }
        ObservationReason::Cancelled => {
            common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_CANCELLED
        }
        ObservationReason::DeadlineExpired => {
            common::ObservabilityObservationReason::OBSERVABILITY_OBSERVATION_REASON_DEADLINE_EXPIRED
        }
    }
}

fn health_reason_from_wire(
    value: EnumOrUnknown<common::ObservabilityHealthReason>,
) -> Result<ProviderHealthReason, ServiceContractError> {
    match value
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?
    {
        common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_NONE => {
            Ok(ProviderHealthReason::None)
        }
        common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_PROVIDER_DEGRADED => {
            Ok(ProviderHealthReason::ProviderDegraded)
        }
        common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_HEALTH_TIMEOUT => {
            Ok(ProviderHealthReason::HealthTimeout)
        }
        common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_HEALTH_STALE => {
            Ok(ProviderHealthReason::HealthStale)
        }
        common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_SESSION_DISCONNECTED => {
            Ok(ProviderHealthReason::SessionDisconnected)
        }
        common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_QUEUE_PRESSURE => {
            Ok(ProviderHealthReason::QueuePressure)
        }
        common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_HANDSHAKE_TIMEOUT => {
            Ok(ProviderHealthReason::HandshakeTimeout)
        }
        common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_AUTHENTICATION_FAILED => {
            Ok(ProviderHealthReason::AuthenticationFailed)
        }
        common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_IDENTITY_MISMATCH => {
            Ok(ProviderHealthReason::IdentityMismatch)
        }
        common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_CONFIGURATION_MISMATCH => {
            Ok(ProviderHealthReason::ConfigurationMismatch)
        }
        common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_GENERATION_MISMATCH => {
            Ok(ProviderHealthReason::GenerationMismatch)
        }
        common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_CAPABILITY_MISMATCH => {
            Ok(ProviderHealthReason::CapabilityMismatch)
        }
        common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_ADOPTION_AMBIGUOUS => {
            Ok(ProviderHealthReason::AdoptionAmbiguous)
        }
        common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_UNSPECIFIED => {
            Err(ServiceContractError::InvalidEnum)
        }
    }
}

fn health_reason_to_wire(value: ProviderHealthReason) -> common::ObservabilityHealthReason {
    match value {
        ProviderHealthReason::None => {
            common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_NONE
        }
        ProviderHealthReason::ProviderDegraded => {
            common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_PROVIDER_DEGRADED
        }
        ProviderHealthReason::HealthTimeout => {
            common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_HEALTH_TIMEOUT
        }
        ProviderHealthReason::HealthStale => {
            common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_HEALTH_STALE
        }
        ProviderHealthReason::SessionDisconnected => {
            common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_SESSION_DISCONNECTED
        }
        ProviderHealthReason::QueuePressure => {
            common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_QUEUE_PRESSURE
        }
        ProviderHealthReason::HandshakeTimeout => {
            common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_HANDSHAKE_TIMEOUT
        }
        ProviderHealthReason::AuthenticationFailed => {
            common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_AUTHENTICATION_FAILED
        }
        ProviderHealthReason::IdentityMismatch => {
            common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_IDENTITY_MISMATCH
        }
        ProviderHealthReason::ConfigurationMismatch => {
            common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_CONFIGURATION_MISMATCH
        }
        ProviderHealthReason::GenerationMismatch => {
            common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_GENERATION_MISMATCH
        }
        ProviderHealthReason::CapabilityMismatch => {
            common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_CAPABILITY_MISMATCH
        }
        ProviderHealthReason::AdoptionAmbiguous => {
            common::ObservabilityHealthReason::OBSERVABILITY_HEALTH_REASON_ADOPTION_AMBIGUOUS
        }
    }
}

fn remediation_from_wire(
    value: EnumOrUnknown<common::ObservabilityRemediation>,
) -> Result<ProviderRemediation, ServiceContractError> {
    match value
        .enum_value()
        .map_err(|_| ServiceContractError::InvalidEnum)?
    {
        common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_NONE => {
            Ok(ProviderRemediation::None)
        }
        common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_RETRY_BOUNDED => {
            Ok(ProviderRemediation::RetryBounded)
        }
        common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_INSPECT_PROVIDER => {
            Ok(ProviderRemediation::InspectProvider)
        }
        common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_RESTART_AGENT => {
            Ok(ProviderRemediation::RestartAgent)
        }
        common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_RE_ENROLL_PEER => {
            Ok(ProviderRemediation::ReEnrollPeer)
        }
        common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_REPAIR_CONFIGURATION => {
            Ok(ProviderRemediation::RepairConfiguration)
        }
        common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_REPLACE_GENERATION => {
            Ok(ProviderRemediation::ReplaceGeneration)
        }
        common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_OPERATOR_INTERACTION => {
            Ok(ProviderRemediation::OperatorInteraction)
        }
        common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_UNSPECIFIED => {
            Err(ServiceContractError::InvalidEnum)
        }
    }
}

fn remediation_to_wire(value: ProviderRemediation) -> common::ObservabilityRemediation {
    match value {
        ProviderRemediation::None => {
            common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_NONE
        }
        ProviderRemediation::RetryBounded => {
            common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_RETRY_BOUNDED
        }
        ProviderRemediation::InspectProvider => {
            common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_INSPECT_PROVIDER
        }
        ProviderRemediation::RestartAgent => {
            common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_RESTART_AGENT
        }
        ProviderRemediation::ReEnrollPeer => {
            common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_RE_ENROLL_PEER
        }
        ProviderRemediation::RepairConfiguration => {
            common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_REPAIR_CONFIGURATION
        }
        ProviderRemediation::ReplaceGeneration => {
            common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_REPLACE_GENERATION
        }
        ProviderRemediation::OperatorInteraction => {
            common::ObservabilityRemediation::OBSERVABILITY_REMEDIATION_OPERATOR_INTERACTION
        }
    }
}

fn validate_observability_query_result_wire(
    value: &common::ObservabilityQueryResult,
) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    let observation = required_message(&value.observation)?;
    reject_unknown(observation)?;
    lifecycle_from_wire(observation.lifecycle)?;
    adoption_from_wire(observation.adoption)?;
    observation_reason_from_wire(observation.reason)?;
    let health_state = health_state_from_wire(observation.health_state)?;
    let health_reason = health_reason_from_wire(observation.health_reason)?;
    let remediation = remediation_from_wire(observation.health_remediation)?;
    let health = ProviderHealth {
        provider_id: ProviderId::parse("aaaaaaaaaaaaaaaaaaaa")
            .map_err(|_| ServiceContractError::InvalidId)?,
        registry_generation: ProviderGeneration::new(1)
            .map_err(|_| ServiceContractError::InvalidId)?,
        observed_at_unix_ms: observation.observed_at_unix_ms,
        state: health_state,
        reason: health_reason,
        remediation,
    };
    health
        .validate()
        .map_err(|_| ServiceContractError::InconsistentResponse)?;
    if observation.observed_at_unix_ms > MAX_SAFE_JSON_INTEGER
        || value.records.len() > usize::from(MAX_OBSERVABILITY_QUERY_LIMIT)
        || value.encoded_bytes_upper_bound > MAX_OBSERVABILITY_QUERY_BYTES
        || value.encoded_bytes_upper_bound
            < u32::try_from(value.records.len())
                .map_err(|_| ServiceContractError::BoundExceeded)?
                .saturating_mul(OBSERVABILITY_RECORD_ENCODED_UPPER_BOUND_BYTES)
        || value.truncated != value.next_cursor.is_some()
    {
        return Err(ServiceContractError::BoundExceeded);
    }
    if let Some(cursor) = &value.next_cursor {
        ObservabilityCursor::parse(cursor.clone()).map_err(|_| ServiceContractError::InvalidId)?;
    }
    let mut previous = None;
    for record in &value.records {
        reject_unknown(record)?;
        if record.observed_at_unix_ms > MAX_SAFE_JSON_INTEGER
            || record.observed_at_unix_ms > observation.observed_at_unix_ms
            || record.value > MAX_SAFE_JSON_INTEGER
        {
            return Err(ServiceContractError::BoundExceeded);
        }
        let projection = projection_from_wire(record.projection)?;
        let labels = required_message(&record.labels)?;
        reject_unknown(labels)?;
        let canonical = ObservabilityRecord {
            observed_at_unix_ms: record.observed_at_unix_ms,
            projection,
            labels: ObservabilityLabels {
                provider_type: provider_type_from_wire(labels.provider_type)?,
                health_state: health_state_from_wire(labels.health_state)?,
                metric: metric_from_wire(labels.metric)?,
                operation: operation_label_from_wire(labels.operation)?,
                outcome: outcome_label_from_wire(labels.outcome)?,
            },
            value: record.value,
        };
        if previous.is_some_and(|candidate| candidate >= canonical) {
            return Err(ServiceContractError::InconsistentResponse);
        }
        previous = Some(canonical);
    }
    Ok(())
}

pub fn observability_query_result_to_wire(
    value: &ObservabilityQueryResult,
    request: &ProviderOperationRequest,
) -> Result<common::ObservabilityQueryResult, ServiceContractError> {
    value
        .validate(request)
        .map_err(|_| ServiceContractError::InconsistentResponse)?;
    let observation = &value.observation;
    let records = value
        .records
        .iter()
        .map(|record| common::ObservabilityRecord {
            observed_at_unix_ms: record.observed_at_unix_ms,
            projection: EnumOrUnknown::new(projection_to_wire(record.projection)),
            labels: MessageField::some(common::ObservabilityLabels {
                provider_type: EnumOrUnknown::new(provider_type_to_wire(
                    record.labels.provider_type,
                )),
                health_state: EnumOrUnknown::new(health_state_to_wire(record.labels.health_state)),
                metric: EnumOrUnknown::new(metric_to_wire(record.labels.metric)),
                operation: EnumOrUnknown::new(operation_label_to_wire(record.labels.operation)),
                outcome: EnumOrUnknown::new(outcome_label_to_wire(record.labels.outcome)),
                ..Default::default()
            }),
            value: record.value,
            ..Default::default()
        })
        .collect();
    let wire = common::ObservabilityQueryResult {
        observation: MessageField::some(common::ObservabilityBoundObservation {
            observed_at_unix_ms: observation.observed_at_unix_ms,
            lifecycle: EnumOrUnknown::new(lifecycle_to_wire(observation.lifecycle)),
            adoption: EnumOrUnknown::new(adoption_to_wire(observation.adoption)),
            reason: EnumOrUnknown::new(observation_reason_to_wire(observation.reason)),
            health_state: EnumOrUnknown::new(health_state_to_wire(observation.health.state)),
            health_reason: EnumOrUnknown::new(health_reason_to_wire(observation.health.reason)),
            health_remediation: EnumOrUnknown::new(remediation_to_wire(
                observation.health.remediation,
            )),
            ..Default::default()
        }),
        records,
        next_cursor: value
            .next_cursor
            .as_ref()
            .map(|cursor| cursor.as_str().to_owned()),
        encoded_bytes_upper_bound: value.encoded_bytes_upper_bound,
        truncated: value.truncated,
        ..Default::default()
    };
    validate_observability_query_result_wire(&wire)?;
    Ok(wire)
}

pub fn observability_query_result_from_wire(
    value: &common::ObservabilityQueryResult,
    request: &ProviderOperationRequest,
) -> Result<ObservabilityQueryResult, ServiceContractError> {
    validate_observability_query_result_wire(value)?;
    let observation = required_message(&value.observation)?;
    let records = value
        .records
        .iter()
        .map(|record| {
            let labels = required_message(&record.labels)?;
            Ok(ObservabilityRecord {
                observed_at_unix_ms: record.observed_at_unix_ms,
                projection: projection_from_wire(record.projection)?,
                labels: ObservabilityLabels {
                    provider_type: provider_type_from_wire(labels.provider_type)?,
                    health_state: health_state_from_wire(labels.health_state)?,
                    metric: metric_from_wire(labels.metric)?,
                    operation: operation_label_from_wire(labels.operation)?,
                    outcome: outcome_label_from_wire(labels.outcome)?,
                },
                value: record.value,
            })
        })
        .collect::<Result<Vec<_>, ServiceContractError>>()?;
    let canonical = ObservabilityQueryResult {
        observation: ProviderObservation {
            provider_id: request.context.provider_id.clone(),
            provider_generation: request.context.provider_generation,
            realm_id: request.target.realm_id().clone(),
            workload_id: request.target.workload_id().cloned(),
            handle_id: None,
            resource_generation: None,
            observed_at_unix_ms: observation.observed_at_unix_ms,
            lifecycle: lifecycle_from_wire(observation.lifecycle)?,
            adoption: adoption_from_wire(observation.adoption)?,
            reason: observation_reason_from_wire(observation.reason)?,
            health: ProviderHealth {
                provider_id: request.context.provider_id.clone(),
                registry_generation: request.context.provider_generation,
                observed_at_unix_ms: observation.observed_at_unix_ms,
                state: health_state_from_wire(observation.health_state)?,
                reason: health_reason_from_wire(observation.health_reason)?,
                remediation: remediation_from_wire(observation.health_remediation)?,
            },
        },
        records: BoundedVec::new(records).map_err(|_| ServiceContractError::BoundExceeded)?,
        next_cursor: value
            .next_cursor
            .as_ref()
            .map(|cursor| ObservabilityCursor::parse(cursor.clone()))
            .transpose()
            .map_err(|_| ServiceContractError::InvalidId)?,
        encoded_bytes_upper_bound: value.encoded_bytes_upper_bound,
        truncated: value.truncated,
    };
    canonical
        .validate(request)
        .map_err(|_| ServiceContractError::InconsistentResponse)?;
    Ok(canonical)
}

pub fn validate_provider_response_for_method(
    response: &common::ProviderResponse,
    method: ProviderMethod,
) -> Result<(), ServiceContractError> {
    response.validate_wire(false)?;
    if method == ProviderMethod::ObservabilityQuery {
        if response.error.is_none() && response.observability_query_result.is_none() {
            return Err(ServiceContractError::InconsistentResponse);
        }
    } else if response.observability_query_result.is_some() {
        return Err(ServiceContractError::InconsistentResponse);
    }
    Ok(())
}

pub fn observability_query_response_from_wire(
    response: &common::ProviderResponse,
    request: &ProviderOperationRequest,
) -> Result<ObservabilityQueryResult, ServiceContractError> {
    validate_provider_response_for_method(response, ProviderMethod::ObservabilityQuery)?;
    if response.operation_id != request.context.operation_id.as_str()
        || response.error.is_some()
        || response
            .outcome
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?
            != common::Outcome::OUTCOME_SUCCEEDED
    {
        return Err(ServiceContractError::InconsistentResponse);
    }
    observability_query_result_from_wire(
        response
            .observability_query_result
            .as_ref()
            .ok_or(ServiceContractError::InconsistentResponse)?,
        request,
    )
}

impl StrictWireMessage for common::ProviderRequest {
    fn validate_wire(&self, requires_idempotency: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        validate_provider_context(required_message(&self.context)?, requires_idempotency)?;
        if !self.resource_id.is_empty()
            && !bounded_opaque(&self.resource_id, MAX_SERVICE_STRING_BYTES)
        {
            return Err(ServiceContractError::BoundExceeded);
        }
        if !optional_digest(&self.plan_digest) {
            return Err(ServiceContractError::InvalidDigest);
        }
        provider_operation_input(
            self.input
                .as_ref()
                .ok_or(ServiceContractError::MissingOperationInput)?,
        )?;
        validate_attachments(&self.attachment_indexes)
    }
}

impl StrictWireMessage for common::CapabilityRequest {
    fn validate_wire(&self, requires_idempotency: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        validate_provider_context(required_message(&self.context)?, requires_idempotency)
    }
}

impl StrictWireMessage for common::CancelRequest {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        if self.session_generation == 0 || RequestId::new(self.request_id.clone()).is_err() {
            return Err(ServiceContractError::InvalidId);
        }
        Ok(())
    }
}

fn validate_error(value: &common::ErrorEnvelope) -> Result<(), ServiceContractError> {
    reject_unknown(value)?;
    if value.kind.enum_value().is_err()
        || value.kind.value() == common::ErrorKind::ERROR_KIND_UNSPECIFIED.value()
        || value.retry.enum_value().is_err()
        || value.retry.value() == common::RetryClass::RETRY_CLASS_UNSPECIFIED.value()
    {
        return Err(ServiceContractError::InvalidEnum);
    }
    if !value.correlation_id.is_empty()
        && CorrelationId::new(value.correlation_id.as_bytes().to_vec()).is_err()
    {
        return Err(ServiceContractError::InvalidId);
    }
    Ok(())
}

fn validate_outcome_error(
    outcome: common::Outcome,
    error: Option<&common::ErrorEnvelope>,
) -> Result<(), ServiceContractError> {
    let required = matches!(
        outcome,
        common::Outcome::OUTCOME_DENIED
            | common::Outcome::OUTCOME_CANCELLED
            | common::Outcome::OUTCOME_FAILED
    );
    let forbidden = matches!(
        outcome,
        common::Outcome::OUTCOME_ACCEPTED
            | common::Outcome::OUTCOME_SUCCEEDED
            | common::Outcome::OUTCOME_NOT_APPLICABLE
    );
    if (required && error.is_none()) || (forbidden && error.is_some()) {
        return Err(ServiceContractError::InconsistentResponse);
    }
    if let Some(error) = error {
        validate_error(error)?;
    }
    Ok(())
}

pub fn provider_method_for_capability(
    capability: common::ProviderCapability,
) -> Result<ProviderMethod, ServiceContractError> {
    match capability {
        common::ProviderCapability::PROVIDER_CAPABILITY_UNSPECIFIED => {
            Err(ServiceContractError::InvalidEnum)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_RUNTIME_PLAN => {
            Ok(ProviderMethod::RuntimePlan)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_RUNTIME_ENSURE => {
            Ok(ProviderMethod::RuntimeEnsure)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_RUNTIME_START => {
            Ok(ProviderMethod::RuntimeStart)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_RUNTIME_STOP => {
            Ok(ProviderMethod::RuntimeStop)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_RUNTIME_EXECUTE => {
            Ok(ProviderMethod::RuntimeExecute)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_RUNTIME_INSPECT => {
            Ok(ProviderMethod::RuntimeInspect)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_RUNTIME_ADOPT => {
            Ok(ProviderMethod::RuntimeAdopt)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_RUNTIME_DESTROY => {
            Ok(ProviderMethod::RuntimeDestroy)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_INFRASTRUCTURE_PLAN => {
            Ok(ProviderMethod::InfrastructurePlan)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_INFRASTRUCTURE_APPLY => {
            Ok(ProviderMethod::InfrastructureApply)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_INFRASTRUCTURE_SET_POWER_STATE => {
            Ok(ProviderMethod::InfrastructureSetPowerState)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_INFRASTRUCTURE_INSPECT => {
            Ok(ProviderMethod::InfrastructureInspect)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_INFRASTRUCTURE_ADOPT => {
            Ok(ProviderMethod::InfrastructureAdopt)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_INFRASTRUCTURE_BOOTSTRAP_BINDING => {
            Ok(ProviderMethod::InfrastructureBootstrapBinding)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_INFRASTRUCTURE_DESTROY => {
            Ok(ProviderMethod::InfrastructureDestroy)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_TRANSPORT_CONNECT => {
            Ok(ProviderMethod::TransportConnect)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_TRANSPORT_LISTEN => {
            Ok(ProviderMethod::TransportListen)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_TRANSPORT_ISSUE_BINDING => {
            Ok(ProviderMethod::TransportIssueBinding)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_TRANSPORT_REVOKE_BINDING => {
            Ok(ProviderMethod::TransportRevokeBinding)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_TRANSPORT_INSPECT => {
            Ok(ProviderMethod::TransportInspect)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_SUBSTRATE_CHECK => {
            Ok(ProviderMethod::SubstrateCheck)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_SUBSTRATE_PLAN_REMEDIATION => {
            Ok(ProviderMethod::SubstratePlanRemediation)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_SUBSTRATE_APPLY => {
            Ok(ProviderMethod::SubstrateApply)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_CREDENTIAL_STATUS => {
            Ok(ProviderMethod::CredentialStatus)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_CREDENTIAL_ACQUIRE_LEASE => {
            Ok(ProviderMethod::CredentialAcquireLease)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_CREDENTIAL_REFRESH_LEASE => {
            Ok(ProviderMethod::CredentialRefreshLease)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_CREDENTIAL_REVOKE_LEASE => {
            Ok(ProviderMethod::CredentialRevokeLease)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_DISPLAY_OPEN => {
            Ok(ProviderMethod::DisplayOpen)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_DISPLAY_INSPECT => {
            Ok(ProviderMethod::DisplayInspect)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_DISPLAY_ADOPT => {
            Ok(ProviderMethod::DisplayAdopt)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_DISPLAY_CLOSE => {
            Ok(ProviderMethod::DisplayClose)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_NETWORK_PLAN => {
            Ok(ProviderMethod::NetworkPlan)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_NETWORK_ENSURE => {
            Ok(ProviderMethod::NetworkEnsure)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_NETWORK_INSPECT => {
            Ok(ProviderMethod::NetworkInspect)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_NETWORK_ADOPT => {
            Ok(ProviderMethod::NetworkAdopt)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_NETWORK_RELEASE => {
            Ok(ProviderMethod::NetworkRelease)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_STORAGE_PLAN => {
            Ok(ProviderMethod::StoragePlan)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_STORAGE_ENSURE => {
            Ok(ProviderMethod::StorageEnsure)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_STORAGE_INSPECT => {
            Ok(ProviderMethod::StorageInspect)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_STORAGE_ADOPT => {
            Ok(ProviderMethod::StorageAdopt)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_STORAGE_SNAPSHOT => {
            Ok(ProviderMethod::StorageSnapshot)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_STORAGE_DESTROY => {
            Ok(ProviderMethod::StorageDestroy)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_DEVICE_PLAN_ATTACH => {
            Ok(ProviderMethod::DevicePlanAttach)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_DEVICE_ATTACH => {
            Ok(ProviderMethod::DeviceAttach)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_DEVICE_INSPECT => {
            Ok(ProviderMethod::DeviceInspect)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_DEVICE_ADOPT => {
            Ok(ProviderMethod::DeviceAdopt)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_DEVICE_DETACH => {
            Ok(ProviderMethod::DeviceDetach)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_AUDIO_OPEN => Ok(ProviderMethod::AudioOpen),
        common::ProviderCapability::PROVIDER_CAPABILITY_AUDIO_SET_STATE => {
            Ok(ProviderMethod::AudioSetState)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_AUDIO_INSPECT => {
            Ok(ProviderMethod::AudioInspect)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_AUDIO_ADOPT => {
            Ok(ProviderMethod::AudioAdopt)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_AUDIO_CLOSE => {
            Ok(ProviderMethod::AudioClose)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_OBSERVABILITY_STATUS => {
            Ok(ProviderMethod::ObservabilityStatus)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_OBSERVABILITY_QUERY => {
            Ok(ProviderMethod::ObservabilityQuery)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_OBSERVABILITY_SUBSCRIBE => {
            Ok(ProviderMethod::ObservabilitySubscribe)
        }
        common::ProviderCapability::PROVIDER_CAPABILITY_OBSERVABILITY_EXPORT => {
            Ok(ProviderMethod::ObservabilityExport)
        }
    }
}

fn validate_observations(values: &[common::Observation]) -> Result<(), ServiceContractError> {
    if values.len() > MAX_OBSERVATIONS {
        return Err(ServiceContractError::BoundExceeded);
    }
    for value in values {
        reject_unknown(value)?;
        if !bounded_opaque(&value.resource_id, MAX_SERVICE_STRING_BYTES)
            || Generation::new(value.generation).is_err()
        {
            return Err(ServiceContractError::InvalidId);
        }
        if value.state.enum_value().is_err()
            || value.state.value()
                == common::ObservationState::OBSERVATION_STATE_UNSPECIFIED.value()
        {
            return Err(ServiceContractError::InvalidEnum);
        }
        if !required_digest(&value.digest) {
            return Err(ServiceContractError::InvalidDigest);
        }
    }
    Ok(())
}

impl StrictWireMessage for common::ServiceResponse {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        if self.outcome.enum_value().is_err()
            || self.outcome.value() == common::Outcome::OUTCOME_UNSPECIFIED.value()
        {
            return Err(ServiceContractError::InvalidEnum);
        }
        if (!self.operation_id.is_empty()
            && !bounded_opaque(&self.operation_id, MAX_SERVICE_STRING_BYTES))
            || (!self.resource_handle.is_empty()
                && !bounded_opaque(&self.resource_handle, MAX_SERVICE_STRING_BYTES))
            || (!self.stream_id.is_empty()
                && !bounded_opaque(&self.stream_id, MAX_SERVICE_STRING_BYTES))
            || (!self.next_page_cursor.is_empty()
                && !bounded_opaque(&self.next_page_cursor, MAX_PAGE_CURSOR_BYTES))
        {
            return Err(ServiceContractError::BoundExceeded);
        }
        if !optional_digest(&self.result_digest) {
            return Err(ServiceContractError::InvalidDigest);
        }
        validate_observations(&self.observations)?;
        validate_attachments(&self.attachment_indexes)?;
        validate_outcome_error(
            self.outcome
                .enum_value()
                .map_err(|_| ServiceContractError::InvalidEnum)?,
            self.error.as_ref(),
        )
    }
}

impl StrictWireMessage for common::ProviderResponse {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        if self.outcome.enum_value().is_err()
            || self.outcome.value() == common::Outcome::OUTCOME_UNSPECIFIED.value()
        {
            return Err(ServiceContractError::InvalidEnum);
        }
        if !bounded_opaque(&self.operation_id, MAX_SERVICE_STRING_BYTES)
            || (!self.resource_handle.is_empty()
                && !bounded_opaque(&self.resource_handle, MAX_SERVICE_STRING_BYTES))
            || (!self.stream_id.is_empty()
                && !bounded_opaque(&self.stream_id, MAX_SERVICE_STRING_BYTES))
        {
            return Err(ServiceContractError::BoundExceeded);
        }
        if !optional_digest(&self.result_digest) {
            return Err(ServiceContractError::InvalidDigest);
        }
        validate_observations(&self.observations)?;
        validate_attachments(&self.attachment_indexes)?;
        let outcome = self
            .outcome
            .enum_value()
            .map_err(|_| ServiceContractError::InvalidEnum)?;
        validate_outcome_error(outcome, self.error.as_ref())?;
        if let Some(result) = self.observability_query_result.as_ref() {
            if outcome != common::Outcome::OUTCOME_SUCCEEDED
                || self.error.is_some()
                || !self.resource_handle.is_empty()
                || !self.result_digest.is_empty()
                || !self.observations.is_empty()
                || !self.stream_id.is_empty()
                || !self.attachment_indexes.is_empty()
            {
                return Err(ServiceContractError::InconsistentResponse);
            }
            validate_observability_query_result_wire(result)?;
        }
        if self.error.is_some() && self.observability_query_result.is_some() {
            return Err(ServiceContractError::InconsistentResponse);
        }
        Ok(())
    }
}

impl StrictWireMessage for common::CapabilityResponse {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        if let Some(error) = self.error.as_ref() {
            validate_error(error)?;
            return if self.capabilities.is_empty()
                && self.provider_generation == 0
                && self.descriptor_digest.is_empty()
            {
                Ok(())
            } else {
                Err(ServiceContractError::InconsistentResponse)
            };
        }
        if self.capabilities.is_empty()
            || self.capabilities.len() > MAX_PROVIDER_CAPABILITIES
            || self.provider_generation == 0
        {
            return Err(ServiceContractError::BoundExceeded);
        }
        if !required_digest(&self.descriptor_digest) {
            return Err(ServiceContractError::InvalidDigest);
        }
        let mut unique = BTreeSet::new();
        for capability in &self.capabilities {
            let value = capability
                .enum_value()
                .map_err(|_| ServiceContractError::InvalidEnum)?;
            provider_method_for_capability(value)?;
            if !unique.insert(value.value()) {
                return Err(ServiceContractError::InvalidEnum);
            }
        }
        Ok(())
    }
}

impl StrictWireMessage for common::CancelResponse {
    fn validate_wire(&self, _: bool) -> Result<(), ServiceContractError> {
        reject_unknown(self)?;
        if self.outcome.enum_value().is_err()
            || self.outcome.value() == common::CancelOutcome::CANCEL_OUTCOME_UNSPECIFIED.value()
        {
            return Err(ServiceContractError::InvalidEnum);
        }
        Ok(())
    }
}

pub struct RedactedRequest<'a> {
    pub package: &'a str,
    pub service: &'a str,
    pub method: &'a str,
    pub request: &'a common::ServiceRequest,
}

impl fmt::Debug for RedactedRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedactedRequest")
            .field("package", &self.package)
            .field("service", &self.service)
            .field("method", &self.method)
            .field(
                "has_correlation",
                &self
                    .request
                    .metadata
                    .as_ref()
                    .is_some_and(|metadata| !metadata.correlation_id.is_empty()),
            )
            .field(
                "has_attachments",
                &!self.request.attachment_indexes.is_empty(),
            )
            .finish()
    }
}
