//! Realm access bindings (ADR 0043). Bindings describe how a client reaches a
//! realm controller without carrying credential material or provider tokens.

use crate::ids::{ControllerGenerationId, ProviderId};
use crate::realm::{RealmControllerPlacement, RealmPath};
use crate::token::ProtocolToken;
use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};

/// Maximum bytes in a local realm Unix socket path.
pub const MAX_UNIX_SOCKET_PATH_LEN: usize = 255;
/// Maximum bytes in a non-secret access-binding reference.
pub const MAX_ACCESS_REF_LEN: usize = 128;

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
    }
}
