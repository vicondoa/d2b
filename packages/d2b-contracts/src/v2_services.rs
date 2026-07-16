//! Generated protobuf/ttrpc contracts for every d2b-owned ComponentSession v2 service.
//!
//! The generated DTOs contain only bounded opaque identifiers, digests, closed
//! enums, stream identifiers, and ComponentSession attachment indexes. Caller
//! identity and method capability are intentionally absent: authenticated
//! session state and [`SERVICE_INVENTORY`] are their sole authority.

use std::{collections::BTreeSet, error::Error, fmt};

use protobuf::{Enum, EnumOrUnknown, Message, MessageField};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{
    v2_component_session::{
        BoundedVec, CorrelationId, IdempotencyKey, MAX_LOGICAL_MESSAGE_BYTES,
        MAX_REQUEST_ATTACHMENTS, MAX_REQUEST_LIFETIME_MS, RequestEnvelope, RequestId, TraceId,
    },
    v2_identity::{ProviderId, ProviderType as IdentityProviderType, RealmId, RoleId, WorkloadId},
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
#[path = "generated_v2_services/daemon.rs"]
pub mod daemon;
#[path = "generated_v2_services/daemon_ttrpc.rs"]
pub mod daemon_ttrpc;
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
            "InspectExec" => false, "OpenShell" => true, "FileTransfer" => true,
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
        schema_version: 2,
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
