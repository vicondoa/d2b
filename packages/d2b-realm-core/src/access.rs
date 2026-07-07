//! Realm access bindings. Bindings describe how a client reaches a
//! realm controller without carrying credential material or provider tokens.

use crate::capability::{Capability, CapabilitySet, MAX_CAPABILITY_SET_LEN};
use crate::ids::{ControllerGenerationId, ProviderId, WorkloadId};
use crate::realm::{RealmControllerPlacement, RealmPath};
use crate::target::RealmTarget;
use crate::token::ProtocolToken;
use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};

/// Maximum bytes in a pathname Linux `sockaddr_un.sun_path`, reserving the
/// trailing NUL.
pub const MAX_UNIX_SOCKET_PATH_LEN: usize = 107;
/// Maximum bytes in a non-secret access-binding reference.
pub const MAX_ACCESS_REF_LEN: usize = 128;
/// Maximum bytes in a resolver input target string.
pub const MAX_ACCESS_TARGET_INPUT_LEN: usize = 388;
/// Maximum alias bindings accepted in one resolver input.
pub const MAX_ACCESS_ALIAS_BINDINGS: usize = 64;
/// Maximum binding kinds in one client contract.
pub const MAX_ACCESS_CLIENT_BINDINGS: usize = 8;
/// Maximum ambiguity/conflict candidates carried in one diagnostic.
pub const MAX_ACCESS_CONFLICT_CANDIDATES: usize = 16;
/// Maximum diagnostics carried in one resolver output.
pub const MAX_ACCESS_DIAGNOSTICS: usize = 16;

/// Absolute Unix socket path for a host-local realm.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct UnixSocketPath(String);

impl UnixSocketPath {
    /// Validate a bounded absolute Unix socket path.
    pub fn parse(raw: impl Into<String>) -> Option<Self> {
        let raw = raw.into();
        if raw.is_empty()
            || raw.len() > MAX_UNIX_SOCKET_PATH_LEN
            || !raw.starts_with('/')
            || raw.contains('\0')
            || raw.contains("..")
        {
            return None;
        }
        Some(Self(raw))
    }

    /// Borrow the socket path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for UnixSocketPath {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "UnixSocketPath(<{} bytes>)", self.0.len())
    }
}

impl<'de> Deserialize<'de> for UnixSocketPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("invalid Unix socket path"))
    }
}

impl JsonSchema for UnixSocketPath {
    fn schema_name() -> String {
        "UnixSocketPath".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                max_length: Some(MAX_UNIX_SOCKET_PATH_LEN as u32),
                min_length: Some(1),
                pattern: Some("^/[^\\0]*$".to_owned()),
            })),
            ..Default::default()
        })
    }
}

/// Opaque, non-secret reference to provider/remote binding metadata. The value
/// is intentionally not a credential and is redacted from `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct AccessBindingRef(String);

impl AccessBindingRef {
    /// Validate a bounded printable reference token.
    pub fn parse(raw: impl Into<String>) -> Option<Self> {
        let raw = raw.into();
        if raw.is_empty()
            || raw.len() > MAX_ACCESS_REF_LEN
            || !raw.as_bytes()[0].is_ascii_alphanumeric()
            || !raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return None;
        }
        let compact = raw
            .chars()
            .filter(|c| !matches!(c, '-' | '_' | '.'))
            .flat_map(char::to_lowercase)
            .collect::<String>();
        if ["secret", "password", "bearer", "token", "credential"]
            .iter()
            .any(|marker| compact.contains(marker))
        {
            return None;
        }
        Some(Self(raw))
    }

    /// Borrow the reference.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for AccessBindingRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "AccessBindingRef(<{} bytes>)", self.0.len())
    }
}

impl<'de> Deserialize<'de> for AccessBindingRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("invalid access binding reference"))
    }
}

impl JsonSchema for AccessBindingRef {
    fn schema_name() -> String {
        "AccessBindingRef".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                max_length: Some(MAX_ACCESS_REF_LEN as u32),
                min_length: Some(1),
                pattern: Some("^[A-Za-z0-9][A-Za-z0-9._-]*$".to_owned()),
            })),
            ..Default::default()
        })
    }
}

/// Operator-supplied target string accepted by a realm access resolver. The
/// value may be a fully-qualified target or a context-dependent alias, so it is
/// bounded and redacted here rather than parsed as a [`RealmTarget`] at decode.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct RealmAccessTargetInput(String);

impl RealmAccessTargetInput {
    /// Validate a bounded, non-NUL target input string.
    pub fn parse(raw: impl Into<String>) -> Option<Self> {
        let raw = raw.into();
        if raw.is_empty()
            || raw.len() > MAX_ACCESS_TARGET_INPUT_LEN
            || raw.contains('\0')
            || raw.chars().any(char::is_whitespace)
        {
            return None;
        }
        Some(Self(raw))
    }

    /// Borrow the input target string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for RealmAccessTargetInput {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "RealmAccessTargetInput(<{} bytes>)", self.0.len())
    }
}

impl<'de> Deserialize<'de> for RealmAccessTargetInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("invalid realm access target input"))
    }
}

impl JsonSchema for RealmAccessTargetInput {
    fn schema_name() -> String {
        "RealmAccessTargetInput".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                max_length: Some(MAX_ACCESS_TARGET_INPUT_LEN as u32),
                min_length: Some(1),
                pattern: Some("^[^\\0\\s]+$".to_owned()),
            })),
            ..Default::default()
        })
    }
}

fn deserialize_bounded_vec<'de, D, T, const MAX: usize>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let values = Vec::<T>::deserialize(deserializer)?;
    if values.len() > MAX {
        return Err(serde::de::Error::custom(format!(
            "array exceeds maximum length {MAX}"
        )));
    }
    Ok(values)
}

/// How a resolver learned about a target alias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "source", rename_all = "kebab-case", deny_unknown_fields)]
pub enum RealmAccessAliasSource {
    /// The operator supplied a fully-qualified canonical target.
    FullyQualified,
    /// The alias came from a local alias table.
    AliasTable {
        /// Bare alias that matched the target.
        alias: WorkloadId,
        /// Non-secret table/source identifier.
        source_ref: AccessBindingRef,
    },
    /// The resolver appended a selected default realm to a bare workload.
    DefaultRealm {
        /// Default realm selection metadata.
        selection: DefaultRealmSelectionMetadata,
    },
}

/// Why a default realm was selected for a bare workload target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DefaultRealmSelectionSource {
    /// The process or daemon configuration selected the default.
    Configuration,
    /// The caller selected the default explicitly.
    ExplicitRequest,
    /// The resolver selected the reserved local realm for compatibility.
    LocalCompatibility,
}

/// Metadata proving whether default-realm selection was involved in resolving
/// a target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DefaultRealmSelectionMetadata {
    pub realm: RealmPath,
    pub source: DefaultRealmSelectionSource,
    pub applied: bool,
}

/// One alias-table binding supplied as resolver input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmAccessAliasBinding {
    pub alias: WorkloadId,
    pub target: RealmTarget,
    pub source_ref: AccessBindingRef,
}

/// Client-supported access binding kinds. This is a contract declaration, not a
/// transport implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RealmAccessClientBindingKind {
    /// Client can connect directly to a host-local Unix socket and preserve
    /// kernel `SO_PEERCRED` semantics.
    DirectHostLocalUnixSocket,
    /// Client can use a remote realm transport reference.
    RemoteRealmTransportRef,
    /// Client can use a provider transport reference.
    ProviderRealmTransportRef,
}

/// Client-side access contract presented to the resolver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmAccessClientContract {
    #[serde(deserialize_with = "deserialize_bounded_vec::<_, _, MAX_ACCESS_CLIENT_BINDINGS>")]
    #[schemars(length(max = 8))]
    pub supported_bindings: Vec<RealmAccessClientBindingKind>,
    /// When true, a host-local result must be a direct socket connection; a
    /// byte proxy is not acceptable because it would hide the original peer
    /// credentials from `d2bd`.
    pub require_direct_local_so_peercred: bool,
}

/// Input DTO for realm access resolution. Runtime routing can continue using
/// existing paths while callers and tests exchange this contract shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmAccessResolverRequest {
    pub requested_target: RealmAccessTargetInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_realm: Option<DefaultRealmSelectionMetadata>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_bounded_vec::<_, _, MAX_ACCESS_ALIAS_BINDINGS>"
    )]
    #[schemars(length(max = 64))]
    pub aliases: Vec<RealmAccessAliasBinding>,
    pub required_capabilities: CapabilitySet,
    pub client: RealmAccessClientContract,
}

/// Source of peer credentials for a host-local direct socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HostLocalPeerCredentialSource {
    /// The connecting client process is the peer observed by `d2bd`.
    ConnectingClientProcess,
}

/// Component that verifies host-local peer credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HostLocalPeerCredentialChecker {
    /// The public daemon socket performs `SO_PEERCRED` admission.
    D2bdPublicSocket,
}

/// Whether a host-local result uses a byte proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HostLocalProxyStatus {
    /// No proxy process is inserted between the client and `d2bd`.
    NoByteProxy,
}

/// Evidence that a direct host-local binding preserves OS peer credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostLocalPeerCredentialSemantics {
    pub source: HostLocalPeerCredentialSource,
    pub checked_by: HostLocalPeerCredentialChecker,
    pub proxy: HostLocalProxyStatus,
}

impl HostLocalPeerCredentialSemantics {
    /// Direct client-to-`d2bd` Unix socket semantics.
    pub const fn direct_client_peercred() -> Self {
        Self {
            source: HostLocalPeerCredentialSource::ConnectingClientProcess,
            checked_by: HostLocalPeerCredentialChecker::D2bdPublicSocket,
            proxy: HostLocalProxyStatus::NoByteProxy,
        }
    }
}

/// Client-consumable binding selected by the access resolver. The host-local
/// variant is explicitly direct; a byte-proxy implementation would require a
/// different, incompatible variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "binding",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RealmAccessClientBinding {
    DirectHostLocalUnix {
        socket_path: UnixSocketPath,
        peer_credentials: HostLocalPeerCredentialSemantics,
    },
    RemoteRealmTransportRef {
        transport: ProtocolToken,
        binding_ref: AccessBindingRef,
    },
    ProviderRealmTransportRef {
        provider: ProviderId,
        transport: ProtocolToken,
        binding_ref: AccessBindingRef,
    },
}

impl RealmAccessClientBinding {
    /// Build a client binding from a transport binding, preserving direct
    /// host-local `SO_PEERCRED` semantics for Unix sockets.
    pub fn from_transport(transport: &RealmTransportBinding) -> Self {
        match transport {
            RealmTransportBinding::LocalUnixSocket { socket_path } => Self::DirectHostLocalUnix {
                socket_path: socket_path.clone(),
                peer_credentials: HostLocalPeerCredentialSemantics::direct_client_peercred(),
            },
            RealmTransportBinding::RemoteRealmTransport {
                transport,
                binding_ref,
            } => Self::RemoteRealmTransportRef {
                transport: transport.clone(),
                binding_ref: binding_ref.clone(),
            },
            RealmTransportBinding::ProviderRealmTransport {
                provider,
                transport,
                binding_ref,
            } => Self::ProviderRealmTransportRef {
                provider: provider.clone(),
                transport: transport.clone(),
                binding_ref: binding_ref.clone(),
            },
        }
    }
}

/// Capability preflight status for a resolved binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "status",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CapabilityPreflightStatus {
    Satisfied,
    Denied {
        reason: CapabilityPreflightDenialReason,
        #[serde(deserialize_with = "deserialize_bounded_vec::<_, _, MAX_CAPABILITY_SET_LEN>")]
        #[schemars(length(max = 64))]
        missing: Vec<Capability>,
    },
}

/// Why capability preflight denied a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CapabilityPreflightDenialReason {
    MissingCapability,
    UnsupportedCrossRealmCapability,
    MissingRealmController,
    StaleRealmController,
}

/// Capability preflight input/output snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmAccessCapabilityPreflight {
    pub required: CapabilitySet,
    pub advertised: CapabilitySet,
    pub status: CapabilityPreflightStatus,
}

/// Candidate involved in alias ambiguity or binding conflict diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmAccessConflictCandidate {
    pub target: RealmTarget,
    pub realm: RealmPath,
    pub alias_source: RealmAccessAliasSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement: Option<RealmControllerPlacement>,
}

/// Typed resolver diagnostics. These are safe to return to clients and audit:
/// they carry bounded ids, targets, capabilities, and generation ids only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RealmAccessResolverDiagnostic {
    AliasAmbiguous {
        alias: WorkloadId,
        #[serde(
            deserialize_with = "deserialize_bounded_vec::<_, _, MAX_ACCESS_CONFLICT_CANDIDATES>"
        )]
        #[schemars(length(max = 16))]
        candidates: Vec<RealmAccessConflictCandidate>,
    },
    OldNodeQualifiedTarget {
        legacy_target: RealmAccessTargetInput,
        suggested: RealmTarget,
    },
    MissingRealmBinding {
        target: RealmTarget,
        realm: RealmPath,
    },
    UnsupportedCrossRealmCapability {
        target: RealmTarget,
        capability: Capability,
        placement: RealmControllerPlacement,
    },
    StaleRealmController {
        realm: RealmPath,
        expected_generation: ControllerGenerationId,
        observed_generation: Option<ControllerGenerationId>,
    },
    MissingRealmController {
        realm: RealmPath,
    },
}

/// Fail-closed resolver error shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmAccessResolverError {
    pub diagnostic: RealmAccessResolverDiagnostic,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_bounded_vec::<_, _, MAX_ACCESS_DIAGNOSTICS>"
    )]
    #[schemars(length(max = 16))]
    pub related: Vec<RealmAccessResolverDiagnostic>,
}

/// Successful resolver output for a canonical target and binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmAccessResolverResponse {
    pub canonical_target: RealmTarget,
    pub resolved_realm: RealmPath,
    pub placement: RealmControllerPlacement,
    pub access_binding: RealmAccessBinding,
    pub client_binding: RealmAccessClientBinding,
    pub capability_preflight: RealmAccessCapabilityPreflight,
    pub alias_source: RealmAccessAliasSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_realm: Option<DefaultRealmSelectionMetadata>,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "deserialize_bounded_vec::<_, _, MAX_ACCESS_DIAGNOSTICS>"
    )]
    #[schemars(length(max = 16))]
    pub diagnostics: Vec<RealmAccessResolverDiagnostic>,
}

/// Transport binding for reaching a realm controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RealmTransportBinding {
    /// Direct host-local Unix socket. Authentication remains OS DAC +
    /// `SO_PEERCRED`; no local-root byte proxy is implied.
    LocalUnixSocket {
        /// Socket path for this realm controller.
        socket_path: UnixSocketPath,
    },
    /// Remote realm-tree transport selected by the resolver.
    RemoteRealmTransport {
        /// Bounded transport kind, e.g. `relay-v1` or `mtls-v1`.
        transport: ProtocolToken,
        /// Non-secret resolver reference for endpoint lookup.
        binding_ref: AccessBindingRef,
    },
    /// Provider-backed realm transport selected by the resolver.
    ProviderRealmTransport {
        /// Provider owning the transport binding.
        provider: ProviderId,
        /// Bounded provider transport kind.
        transport: ProtocolToken,
        /// Non-secret provider binding reference.
        binding_ref: AccessBindingRef,
    },
}

/// Resolved access binding for a realm controller generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmAccessBinding {
    /// Realm reached by this binding.
    pub realm: RealmPath,
    /// Controller generation that issued this binding.
    pub controller_generation: ControllerGenerationId,
    /// Controller placement metadata.
    pub placement: RealmControllerPlacement,
    /// Concrete transport binding.
    pub transport: RealmTransportBinding,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::RealmId;
    use schemars::schema_for;

    fn realm(label: &str) -> RealmPath {
        RealmPath::new(vec![RealmId::parse(label).unwrap()]).unwrap()
    }

    #[test]
    fn local_socket_binding_round_trips_and_redacts_debug() {
        let binding = RealmAccessBinding {
            realm: realm("work"),
            controller_generation: ControllerGenerationId::parse("gen-1").unwrap(),
            placement: RealmControllerPlacement::HostLocal,
            transport: RealmTransportBinding::LocalUnixSocket {
                socket_path: UnixSocketPath::parse("/run/d2b/realms/work/public.sock").unwrap(),
            },
        };
        let debug = format!("{binding:?}");
        assert!(debug.contains("UnixSocketPath(<"));
        assert!(!debug.contains("/run/d2b/realms/work/public.sock"));

        let json = serde_json::to_string(&binding).unwrap();
        let back: RealmAccessBinding = serde_json::from_str(&json).unwrap();
        assert_eq!(back.realm.target_form(), "work");
    }

    #[test]
    fn remote_binding_rejects_credential_shaped_refs_and_unknown_fields() {
        assert!(AccessBindingRef::parse("relay-ref-1").is_some());
        assert!(AccessBindingRef::parse("-relay-ref-1").is_none());
        assert!(AccessBindingRef::parse(".relay-ref-1").is_none());
        assert!(AccessBindingRef::parse("_relay-ref-1").is_none());
        assert!(AccessBindingRef::parse("bearer-token").is_none());
        let binding = RealmTransportBinding::RemoteRealmTransport {
            transport: ProtocolToken::parse("relay-v1").unwrap(),
            binding_ref: AccessBindingRef::parse("relay-ref-1").unwrap(),
        };
        let json = serde_json::to_string(&binding).unwrap();
        assert!(json.contains("bindingRef"));
        assert!(!json.contains("credential"));
        assert!(!format!("{binding:?}").contains("relay-ref-1"));

        let json = "{\"realm\":[\"work\"],\"controllerGeneration\":\"gen-1\",\
            \"placement\":{\"kind\":\"host-local\"},\
            \"transport\":{\"type\":\"remote-realm-transport\",\"transport\":\"relay-v1\",\
            \"bindingRef\":\"relay-ref-1\",\"credential\":\"nope\"}}";
        assert!(serde_json::from_str::<RealmAccessBinding>(json).is_err());
    }

    #[test]
    fn access_binding_schema_is_generated() {
        let schema = schema_for!(RealmAccessBinding);
        assert!(schema.schema.metadata.is_some());
        let schema = schema_for!(UnixSocketPath);
        assert_eq!(
            schema.schema.string.unwrap().max_length,
            Some(MAX_UNIX_SOCKET_PATH_LEN as u32)
        );
        let schema = schema_for!(RealmAccessResolverResponse);
        assert!(schema.schema.metadata.is_some());
    }

    #[test]
    fn unix_socket_path_rejects_kernel_incompatible_paths() {
        let max_valid = format!("/{}", "x".repeat(MAX_UNIX_SOCKET_PATH_LEN - 1));
        assert_eq!(max_valid.len(), 107);
        assert!(UnixSocketPath::parse(max_valid).is_some());

        let too_long = format!("/{}", "x".repeat(MAX_UNIX_SOCKET_PATH_LEN));
        assert_eq!(too_long.len(), 108);
        assert!(UnixSocketPath::parse(&too_long).is_none());
        assert!(UnixSocketPath::parse("\0abstract").is_none());
        assert!(UnixSocketPath::parse("@abstract").is_none());

        let overlong = format!("\"{too_long}\"");
        assert!(serde_json::from_str::<UnixSocketPath>(&overlong).is_err());
    }

    #[test]
    fn resolver_response_models_host_local_as_direct_not_proxy() {
        let transport = RealmTransportBinding::LocalUnixSocket {
            socket_path: UnixSocketPath::parse("/run/d2b/realms/work/public.sock").unwrap(),
        };
        let binding = RealmAccessBinding {
            realm: realm("work"),
            controller_generation: ControllerGenerationId::parse("gen-1").unwrap(),
            placement: RealmControllerPlacement::HostLocal,
            transport: transport.clone(),
        };
        let response = RealmAccessResolverResponse {
            canonical_target: RealmTarget::parse("builder.work.d2b").unwrap(),
            resolved_realm: realm("work"),
            placement: RealmControllerPlacement::HostLocal,
            access_binding: binding,
            client_binding: RealmAccessClientBinding::from_transport(&transport),
            capability_preflight: RealmAccessCapabilityPreflight {
                required: CapabilitySet::from_caps([Capability::Lifecycle]),
                advertised: CapabilitySet::from_caps([Capability::Lifecycle, Capability::Vsock]),
                status: CapabilityPreflightStatus::Satisfied,
            },
            alias_source: RealmAccessAliasSource::FullyQualified,
            default_realm: None,
            diagnostics: vec![],
        };

        let value = serde_json::to_value(&response).unwrap();
        assert_eq!(value["clientBinding"]["binding"], "direct-host-local-unix");
        assert_eq!(
            value["clientBinding"]["peerCredentials"]["source"],
            "connecting-client-process"
        );
        assert_eq!(
            value["clientBinding"]["peerCredentials"]["checkedBy"],
            "d2bd-public-socket"
        );
        assert_eq!(
            value["clientBinding"]["peerCredentials"]["proxy"],
            "no-byte-proxy"
        );
        assert!(!format!("{response:?}").contains("/run/d2b/realms/work/public.sock"));
        let round_trip: RealmAccessResolverResponse = serde_json::from_value(value).unwrap();
        assert_eq!(
            round_trip.canonical_target.to_canonical(),
            "builder.work.d2b"
        );
    }

    #[test]
    fn resolver_request_and_diagnostics_are_bounded_and_strict() {
        assert!(RealmAccessTargetInput::parse("builder.work.d2b").is_some());
        assert!(RealmAccessTargetInput::parse("builder work.d2b").is_none());
        assert!(
            RealmAccessTargetInput::parse("x".repeat(MAX_ACCESS_TARGET_INPUT_LEN + 1)).is_none()
        );

        let too_many_bindings = serde_json::json!({
            "requestedTarget": "builder",
            "aliases": (0..=MAX_ACCESS_ALIAS_BINDINGS).map(|i| serde_json::json!({
                "alias": format!("a{i}"),
                "target": "builder.work.d2b",
                "sourceRef": format!("alias-ref-{i}")
            })).collect::<Vec<_>>(),
            "requiredCapabilities": ["lifecycle"],
            "client": {
                "supportedBindings": ["direct-host-local-unix-socket"],
                "requireDirectLocalSoPeercred": true
            }
        });
        assert!(serde_json::from_value::<RealmAccessResolverRequest>(too_many_bindings).is_err());

        let unknown_field = serde_json::json!({
            "requestedTarget": "builder",
            "requiredCapabilities": ["lifecycle"],
            "client": {
                "supportedBindings": ["direct-host-local-unix-socket"],
                "requireDirectLocalSoPeercred": true,
                "proxyAllowed": true
            }
        });
        assert!(serde_json::from_value::<RealmAccessResolverRequest>(unknown_field).is_err());

        let candidate = RealmAccessConflictCandidate {
            target: RealmTarget::parse("builder.work.d2b").unwrap(),
            realm: realm("work"),
            alias_source: RealmAccessAliasSource::AliasTable {
                alias: WorkloadId::parse("builder").unwrap(),
                source_ref: AccessBindingRef::parse("aliases-v1").unwrap(),
            },
            placement: Some(RealmControllerPlacement::HostLocal),
        };
        let diagnostic = RealmAccessResolverDiagnostic::AliasAmbiguous {
            alias: WorkloadId::parse("builder").unwrap(),
            candidates: vec![candidate],
        };
        let json = serde_json::to_string(&diagnostic).unwrap();
        assert!(json.contains("alias-ambiguous"));
        let decoded: RealmAccessResolverDiagnostic = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, diagnostic);
    }

    #[test]
    fn capability_preflight_has_typed_cross_realm_denial() {
        let preflight = RealmAccessCapabilityPreflight {
            required: CapabilitySet::from_caps([Capability::GpuAccel]),
            advertised: CapabilitySet::from_caps([Capability::Lifecycle]),
            status: CapabilityPreflightStatus::Denied {
                reason: CapabilityPreflightDenialReason::UnsupportedCrossRealmCapability,
                missing: vec![Capability::GpuAccel],
            },
        };
        let json = serde_json::to_string(&preflight).unwrap();
        assert!(json.contains("unsupported-cross-realm-capability"));
        let decoded: RealmAccessCapabilityPreflight = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, preflight);
    }
}
