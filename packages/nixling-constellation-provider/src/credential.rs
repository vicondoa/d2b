//! The P0 three-plane Azure-auth / credential model (ADR 0032 + operator
//! directive).
//!
//! nixling stores **no Azure secret of its own**. Authentication to and
//! through Azure is split into three planes, and only the third is a nixling
//! credential:
//!
//! 1. [`CredentialPlane::AzureControlPlane`] — allocating/managing ACA, ACR,
//!    and the Relay namespace authenticates with the **operator's ambient
//!    Entra ID identity via the `az` CLI** (`DefaultAzureCredential` /
//!    `AzureCliCredential`), invoked locally. The Entra token cache is owned
//!    by `az` (`~/.azure`); it is never placed in the nixling store, bundle,
//!    manifest, daemon state, argv, env, or journal. Azure RBAC on the
//!    operator's identity governs what may be provisioned. This module only
//!    ever holds an [`AzureControlPlaneRef`] of **opaque, non-secret**
//!    tenant/subscription/region references.
//! 2. [`CredentialPlane::ContainerManagedIdentity`] — the ACA sandbox's
//!    **Managed Identity** mints its own short-lived tokens from IMDS for
//!    Relay/ACR. nixling never mints or hands a Relay SAS token to the
//!    container. This module holds only a [`ManagedIdentityRef`] (the MI
//!    client id), never a token.
//! 3. [`CredentialPlane::NixlingInternal`] — the **gateway-minted
//!    per-session credential** that authenticates the constellation peer /
//!    display session. It is independent of Azure: Relay + MI grant
//!    *reachability only* and never authenticate a constellation principal.
//!    This module carries the [`SessionCredentialBinding`] — the claims a
//!    minted credential is bound to — never the secret material itself.
//!
//! A NixOS eval-time assertion (outside this crate) rejects secret-shaped
//! host config so only the opaque references modelled here can be declared.

use nixling_constellation_core::{
    Capability, GatewayId, OperationId, RealmPath, StreamId, WorkloadId,
};
use schemars::{
    JsonSchema,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};

/// Which of the three credential planes a reference belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum CredentialPlane {
    /// Operator's ambient Entra identity via the `az` CLI (control-plane).
    AzureControlPlane,
    /// ACA sandbox Managed Identity (container → Azure).
    ContainerManagedIdentity,
    /// Gateway-minted per-session credential (nixling-internal peer/display).
    NixlingInternal,
}

/// Maximum length of an [`OpaqueAzureRef`].
pub const MAX_AZURE_REF_LEN: usize = 128;

/// A bounded, **non-secret** Azure resource reference: a tenant id,
/// subscription id, region, or Managed-Identity client id. The charset is
/// restricted to the shape real Azure identifiers take
/// (alphanumeric + `-` `_` `.`), which deliberately excludes the `=`, `/`,
/// `+`, and whitespace that base64/PEM/connection-string **secrets** carry —
/// so a secret-shaped value fails to parse (fail-closed). It is never a SAS
/// token, key, or connection string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct OpaqueAzureRef(String);

/// Why an [`OpaqueAzureRef`] failed validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureRefError {
    /// Empty reference.
    Empty,
    /// Reference exceeded [`MAX_AZURE_REF_LEN`].
    TooLong,
    /// Reference contained a character outside the safe identifier charset
    /// (it looks secret-shaped or carries an endpoint/path).
    BadShape,
}

impl core::fmt::Display for AzureRefError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AzureRefError::Empty => write!(f, "azure reference is empty"),
            AzureRefError::TooLong => {
                write!(f, "azure reference exceeds {MAX_AZURE_REF_LEN} bytes")
            }
            AzureRefError::BadShape => {
                write!(f, "azure reference is not a bare, non-secret identifier")
            }
        }
    }
}

impl std::error::Error for AzureRefError {}

impl OpaqueAzureRef {
    /// Validate and construct a non-secret Azure reference (fail-closed).
    pub fn parse(raw: impl Into<String>) -> Result<Self, AzureRefError> {
        let s = raw.into();
        if s.is_empty() {
            return Err(AzureRefError::Empty);
        }
        if s.len() > MAX_AZURE_REF_LEN {
            return Err(AzureRefError::TooLong);
        }
        if !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(AzureRefError::BadShape);
        }
        Ok(Self(s))
    }

    /// Borrow the reference.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for OpaqueAzureRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

/// ECMA-regex for a bare, non-secret Azure identifier (no `=`, `/`, `+`,
/// or whitespace that base64/PEM/connection-string secrets carry).
const AZURE_REF_PATTERN: &str = "^[A-Za-z0-9._-]+$";

impl JsonSchema for OpaqueAzureRef {
    fn schema_name() -> String {
        "OpaqueAzureRef".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                max_length: Some(MAX_AZURE_REF_LEN as u32),
                min_length: Some(1),
                pattern: Some(AZURE_REF_PATTERN.to_owned()),
            })),
            ..Default::default()
        })
    }
}

/// An opaque, non-secret reference to the Azure control-plane scope the
/// operator's Entra session is allowed to provision into. Carries **no**
/// credential: the actual auth is the operator's ambient `az` login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AzureControlPlaneRef {
    /// Entra tenant id (opaque GUID-shaped reference).
    pub tenant_id: OpaqueAzureRef,
    /// Subscription id (opaque GUID-shaped reference).
    pub subscription_id: OpaqueAzureRef,
    /// Pinned region (e.g. `eastus2`).
    pub region: OpaqueAzureRef,
}

/// An opaque reference to the Managed Identity assigned to an ACA sandbox.
/// The container fetches its own tokens from IMDS; this is never a token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManagedIdentityRef {
    /// Managed-Identity client id assigned to the sandbox.
    pub client_id: OpaqueAzureRef,
}

/// The claims a gateway-minted per-session credential is bound to. This is
/// the bearer authority for the display session; the secret material itself
/// is never modelled here, only the binding the gateway and the peer both
/// validate. A credential presented for a stream that does not match every
/// field is rejected (fail-closed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionCredentialBinding {
    /// Realm the session belongs to.
    pub realm: RealmPath,
    /// Gateway that minted the credential.
    pub gateway: GatewayId,
    /// Gateway generation — bumping it (rotation/revocation) invalidates
    /// every credential minted by an older generation.
    pub gateway_generation: u64,
    /// ACA session / workload the display session drives.
    pub workload: WorkloadId,
    /// Operation that authorized the display session.
    pub operation_id: OperationId,
    /// The single display stream this credential authorizes.
    pub display_stream: StreamId,
    /// Capability the credential grants (always `WindowForwarding` for a
    /// display session; a separate `clipboard` is a distinct binding).
    pub capability: Capability,
    /// Absolute expiry, seconds since the Unix epoch. A credential at or
    /// past this instant is invalid (fail-closed).
    pub expires_at_epoch_s: u64,
    /// Per-mint replay nonce / jti so a captured binding cannot be replayed.
    pub nonce: OpaqueAzureRef,
}

impl SessionCredentialBinding {
    /// Whether the credential is still within its validity window at
    /// `now_epoch_s`.
    pub fn is_unexpired(&self, now_epoch_s: u64) -> bool {
        now_epoch_s < self.expires_at_epoch_s
    }

    /// Whether this credential authorizes `stream` under `operation` for
    /// `workload` — the exact binding the mux MUST confirm before any
    /// Waypipe byte flows. Generation/expiry are checked separately by the
    /// gateway against its current generation + clock.
    pub fn authorizes(
        &self,
        operation: &OperationId,
        workload: &WorkloadId,
        stream: &StreamId,
    ) -> bool {
        &self.operation_id == operation
            && &self.workload == workload
            && &self.display_stream == stream
            && self.capability == Capability::WindowForwarding
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_ref_accepts_identifiers_and_rejects_secret_shapes() {
        // GUID-shaped + region identifiers parse.
        assert!(OpaqueAzureRef::parse("2f8e1c3a-1234-5678-9abc-def012345678").is_ok());
        assert!(OpaqueAzureRef::parse("eastus2").is_ok());
        assert!(OpaqueAzureRef::parse("my-mi_client.id").is_ok());
        // Secret/endpoint shapes (base64 padding, slashes, '+', spaces,
        // connection-string '=') are refused.
        assert_eq!(
            OpaqueAzureRef::parse("SharedAccessKey=abc/def+ghi=="),
            Err(AzureRefError::BadShape)
        );
        assert_eq!(
            OpaqueAzureRef::parse("sb://ns.servicebus.windows.net/"),
            Err(AzureRefError::BadShape)
        );
        assert_eq!(OpaqueAzureRef::parse(""), Err(AzureRefError::Empty));
        assert_eq!(
            OpaqueAzureRef::parse("x".repeat(MAX_AZURE_REF_LEN + 1)),
            Err(AzureRefError::TooLong)
        );
    }

    #[test]
    fn opaque_ref_deserialize_is_fail_closed() {
        assert!(serde_json::from_str::<OpaqueAzureRef>("\"eastus2\"").is_ok());
        // A secret-shaped value fails to decode.
        assert!(serde_json::from_str::<OpaqueAzureRef>("\"key=AAAA/BBBB+CCCC==\"").is_err());
    }

    fn binding() -> SessionCredentialBinding {
        SessionCredentialBinding {
            realm: RealmPath::local(),
            gateway: GatewayId::parse("gw-abc123").unwrap(),
            gateway_generation: 7,
            workload: WorkloadId::parse("demo").unwrap(),
            operation_id: OperationId::parse("op-1").unwrap(),
            display_stream: StreamId::parse("disp-1").unwrap(),
            capability: Capability::WindowForwarding,
            expires_at_epoch_s: 1_000,
            nonce: OpaqueAzureRef::parse("nonce-abc").unwrap(),
        }
    }

    #[test]
    fn binding_expiry_is_fail_closed() {
        let b = binding();
        assert!(b.is_unexpired(999));
        assert!(!b.is_unexpired(1_000));
        assert!(!b.is_unexpired(1_001));
    }

    #[test]
    fn binding_authorizes_only_the_exact_stream() {
        let b = binding();
        let op = OperationId::parse("op-1").unwrap();
        let wl = WorkloadId::parse("demo").unwrap();
        let stream = StreamId::parse("disp-1").unwrap();
        assert!(b.authorizes(&op, &wl, &stream));
        // A different stream/op/workload is refused.
        let other = StreamId::parse("disp-2").unwrap();
        assert!(!b.authorizes(&op, &wl, &other));
        let other_op = OperationId::parse("op-2").unwrap();
        assert!(!b.authorizes(&other_op, &wl, &stream));
    }

    #[test]
    fn non_window_forwarding_capability_never_authorizes_display() {
        let mut b = binding();
        b.capability = Capability::Clipboard;
        let op = OperationId::parse("op-1").unwrap();
        let wl = WorkloadId::parse("demo").unwrap();
        let stream = StreamId::parse("disp-1").unwrap();
        assert!(!b.authorizes(&op, &wl, &stream));
    }

    #[test]
    fn refs_round_trip_through_serde() {
        let cp = AzureControlPlaneRef {
            tenant_id: OpaqueAzureRef::parse("tenant-1").unwrap(),
            subscription_id: OpaqueAzureRef::parse("sub-1").unwrap(),
            region: OpaqueAzureRef::parse("eastus2").unwrap(),
        };
        let json = serde_json::to_string(&cp).unwrap();
        assert_eq!(
            serde_json::from_str::<AzureControlPlaneRef>(&json).unwrap(),
            cp
        );

        let mi = ManagedIdentityRef {
            client_id: OpaqueAzureRef::parse("mi-client-1").unwrap(),
        };
        let json = serde_json::to_string(&mi).unwrap();
        assert_eq!(
            serde_json::from_str::<ManagedIdentityRef>(&json).unwrap(),
            mi
        );
    }
}
