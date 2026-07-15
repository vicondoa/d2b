//! Generated protobuf/ttrpc contracts for every d2b-owned ComponentSession v2 service.
//!
//! The generated DTOs contain only bounded opaque identifiers, digests, closed
//! enums, stream identifiers, and ComponentSession attachment indexes. Caller
//! identity and method capability are intentionally absent: authenticated
//! session state and [`SERVICE_INVENTORY`] are their sole authority.

use std::{collections::BTreeSet, error::Error, fmt};

use protobuf::{Enum, Message, MessageField};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{
    v2_component_session::{
        CorrelationId, IdempotencyKey, MAX_LOGICAL_MESSAGE_BYTES, MAX_REQUEST_ATTACHMENTS,
        MAX_REQUEST_LIFETIME_MS, RequestEnvelope, RequestId, TraceId,
    },
    v2_identity::{ProviderId, ProviderType as IdentityProviderType, RealmId, RoleId, WorkloadId},
    v2_provider::{
        AudioChannel as CanonicalAudioChannel, AudioDirection as CanonicalAudioDirection,
        ConfiguredItemId, DeviceSelectorId,
        InfrastructurePowerState as CanonicalInfrastructurePowerState, MAX_PROVIDER_CAPABILITIES,
        ObservabilityCursor, ObservabilityExportFormat as CanonicalObservabilityExportFormat,
        ObservabilityView as CanonicalObservabilityView, ProviderMethod,
        ProviderOperationInput as CanonicalProviderOperationInput, StorageSnapshotId,
        TransportBindingId,
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
        validate_outcome_error(
            self.outcome
                .enum_value()
                .map_err(|_| ServiceContractError::InvalidEnum)?,
            self.error.as_ref(),
        )
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
