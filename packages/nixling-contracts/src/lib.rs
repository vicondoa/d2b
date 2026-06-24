#![doc = "Canonical public and private IPC wire types for nixling."]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::BTreeSet;

pub mod broker_wire;
pub mod generated;
pub mod guest_auth;
pub mod guest_proto {
    pub use crate::generated::guest_control::*;
}
pub mod guest_wire;
pub mod public_wire;
pub mod terminal_wire;
pub mod types;
pub mod usbip;

pub use nixling_core::error::{Error, SemverRange, Version};
pub use nixling_core::privileges_w3::W3BrokerOperation;

pub const MAX_FRAME_SIZE: usize = 1024 * 1024;
pub const PUBLIC_SOCKET_PATH: &str = "/run/nixling/public.sock";
pub const BROKER_SOCKET_PATH: &str = "/run/nixling/priv.sock";

/// Wire-protocol version. Earlier builds negotiated via [`SemverRange`];
/// the current broker handshake layers an explicit `PROTOCOL_VERSION`
/// constant on top so the `nixling-priv-broker`/`nixlingd` skew gate can
/// refuse strangers without a full version negotiation round-trip.
///
/// This constant was bumped from 2 → 3 when mutating broker variants
/// became **opaque ID** only: the daemon no longer passes inline nft text,
/// route specs, sysctl values, hosts entries, NM ifname sets, paths,
/// uids/gids, argv, env, caps, or seccomp profiles across the wire. Legacy
/// callers that send the old payload shape are refused with
/// `wire-version-mismatch`.
pub const PROTOCOL_VERSION: u32 = 3;

/// Broker operation capability set advertised at handshake time. The
/// daemon and broker compare their respective sets and refuse operations
/// with `wire-version-mismatch` when either side is older.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrokerCapabilities {
    /// Wire protocol version (matches [`PROTOCOL_VERSION`] when both
    /// sides support the same protocol).
    pub protocol_version: u32,
    /// Stable broker operation tags this side supports. Wire-equivalent
    /// to [`broker_wire::BrokerRequest`] variant discriminants; encoded
    /// as a sorted list for deterministic gold-files.
    pub broker_operations: Vec<String>,
}

impl BrokerCapabilities {
    /// Capabilities advertised by an up-to-date broker. The list
    /// includes both legacy variants that survive the current contract and
    /// every variant listed in [`W3BrokerOperation::all`].
    pub fn w3() -> Self {
        let mut operations: Vec<String> = W3BrokerOperation::all()
            .iter()
            .map(|op| op.wire_tag().to_owned())
            .collect();
        // Legacy variants that remain wire-stable but are not in the
        // closed operation enum (so they're not auto-included above).
        operations.extend(
            [
                "Hello",
                "ValidateBundle",
                "ExportBrokerAudit",
                "CreateOrReconcileUsersGroups",
                "SetupMountNamespace",
                "PrepareStoreView",
                "LaunchMinijailChild",
                "ReadSecretById",
                "InjectSecretById",
                "RotateSecretById",
                "UsbipBind",
                "UsbipUnbind",
                "UsbipProxyReconcile",
                "PauseBroker",
                "ResumeBroker",
            ]
            .into_iter()
            .map(str::to_owned),
        );
        operations.sort();
        operations.dedup();
        Self {
            protocol_version: PROTOCOL_VERSION,
            broker_operations: operations,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct FeatureFlag(String);

impl FeatureFlag {
    pub fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        let valid = !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !valid {
            return Err("feature flags must match [a-z0-9-]+".to_owned());
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn known(&self) -> Option<KnownFeatureFlag> {
        match self.as_str() {
            "typed-errors" => Some(KnownFeatureFlag::TypedErrors),
            "manifest-v04" => Some(KnownFeatureFlag::ManifestV04),
            "status-check-bridges" => Some(KnownFeatureFlag::StatusCheckBridges),
            "export-broker-audit" => Some(KnownFeatureFlag::ExportBrokerAudit),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for FeatureFlag {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for FeatureFlag {
    fn schema_name() -> String {
        "FeatureFlag".to_owned()
    }

    fn json_schema(r#gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        <String as JsonSchema>::json_schema(r#gen)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KnownFeatureFlag {
    TypedErrors,
    ManifestV04,
    StatusCheckBridges,
    ExportBrokerAudit,
}

impl KnownFeatureFlag {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TypedErrors => "typed-errors",
            Self::ManifestV04 => "manifest-v04",
            Self::StatusCheckBridges => "status-check-bridges",
            Self::ExportBrokerAudit => "export-broker-audit",
        }
    }

    pub fn wire_value(self) -> FeatureFlag {
        FeatureFlag(self.as_str().to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Hello {
    pub client_version: SemverRange,
    #[serde(default)]
    pub supported_features: Vec<FeatureFlag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelloOk {
    pub server_version: Version,
    pub selected_version: Version,
    pub capabilities: Vec<FeatureFlag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelloRejected {
    pub reason: HelloRejectedReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum HelloRejectedReason {
    VersionMismatch,
    CapabilityNegotiationFailed,
    InternalError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FramingSpec {
    pub transport: String,
    pub length_prefix_bytes: u8,
    pub length_prefix_encoding: String,
    pub body_encoding: String,
    pub max_frame_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SocketSpec {
    pub path: String,
    pub mode: String,
    pub owner: String,
    pub group: String,
    pub abstract_namespace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WireProtocolSchema {
    pub schema_version: String,
    pub framing: FramingSpec,
    pub public_socket: SocketSpec,
    pub broker_socket: SocketSpec,
    pub hello: Hello,
    pub hello_ok: HelloOk,
    pub hello_rejected: HelloRejected,
    pub public_request: public_wire::PublicRequest,
    pub public_response: public_wire::PublicResponse,
    pub broker_request: broker_wire::BrokerRequest,
    pub broker_response: broker_wire::BrokerResponse,
}

pub fn negotiate_hello(
    hello: &Hello,
    server_version: &Version,
    server_capabilities: &[FeatureFlag],
) -> Result<HelloOk, HelloRejected> {
    if !hello.client_version.allows(server_version) {
        return Err(HelloRejected {
            reason: HelloRejectedReason::VersionMismatch,
        });
    }

    let known_client_features: BTreeSet<_> = hello
        .supported_features
        .iter()
        .filter_map(|feature| feature.known())
        .collect();
    let mut capabilities: Vec<_> = server_capabilities
        .iter()
        .filter(|feature| {
            feature
                .known()
                .is_some_and(|known| known_client_features.contains(&known))
        })
        .cloned()
        .collect();
    capabilities.sort();
    capabilities.dedup();

    Ok(HelloOk {
        server_version: server_version.clone(),
        selected_version: server_version.clone(),
        capabilities,
    })
}

pub fn encode_frame<T>(message: &T) -> Result<Vec<u8>, Error>
where
    T: Serialize,
{
    let body = serde_json::to_vec(message)
        .map_err(|_| Error::malformed_json("frame", "serialize-failed"))?;
    if body.len() > MAX_FRAME_SIZE {
        return Err(Error::frame_too_large(
            body.len() as u64,
            MAX_FRAME_SIZE as u64,
        ));
    }

    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

pub fn decode_frame<T>(type_name: &'static str, frame: &[u8]) -> Result<T, Error>
where
    T: DeserializeOwned,
{
    if frame.len() < 4 {
        return Err(Error::malformed_json(type_name, "frame-too-short"));
    }

    let declared_length =
        u32::from_le_bytes(frame[..4].try_into().expect("prefix length")) as usize;
    if declared_length > MAX_FRAME_SIZE {
        return Err(Error::frame_too_large(
            declared_length as u64,
            MAX_FRAME_SIZE as u64,
        ));
    }

    let body = &frame[4..];
    if body.len() != declared_length {
        return Err(Error::malformed_json(type_name, "frame-length-mismatch"));
    }

    decode_json_body(type_name, body)
}

pub fn decode_json_body<T>(type_name: &'static str, body: &[u8]) -> Result<T, Error>
where
    T: DeserializeOwned,
{
    serde_json::from_slice(body).map_err(|error| classify_deserialize_error(type_name, &error))
}

pub mod cli_output;

fn classify_deserialize_error(type_name: &'static str, error: &serde_json::Error) -> Error {
    let message = error.to_string();
    if let Some(field) = extract_unknown_field(&message) {
        return Error::unknown_field(type_name, field);
    }
    if let Some(reason) = extract_ifname_error(&message) {
        return Error::if_name_invalid(reason);
    }

    let opaque_reason =
        if message.contains("EOF while parsing") || message.contains("eof while parsing") {
            "unexpected-eof"
        } else if message.contains("expected value") || message.contains("expected ident") {
            "invalid-json"
        } else if message.contains("invalid type") {
            "invalid-type"
        } else if message.contains("missing field") {
            "missing-field"
        } else {
            "decode-failed"
        };
    Error::malformed_json(type_name, opaque_reason)
}

fn extract_unknown_field(message: &str) -> Option<String> {
    let needle = "unknown field `";
    let start = message.find(needle)? + needle.len();
    let rest = &message[start..];
    let end = rest.find('`')?;
    Some(rest[..end].to_owned())
}

fn extract_ifname_error(message: &str) -> Option<nixling_core::host::IfNameError> {
    if message.contains("interface name must not be empty") {
        Some(nixling_core::host::IfNameError::Empty)
    } else if message.contains("interface name must be at most 15 bytes") {
        Some(nixling_core::host::IfNameError::TooLong)
    } else if message.contains("interface name contains characters outside [A-Za-z0-9_-]") {
        Some(nixling_core::host::IfNameError::InvalidCharacter)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BrokerCapabilities, FeatureFlag, Hello, KnownFeatureFlag, MAX_FRAME_SIZE, PROTOCOL_VERSION,
        SemverRange, Version, W3BrokerOperation, broker_wire::BrokerRequest, decode_frame,
        encode_frame, negotiate_hello,
    };

    #[test]
    fn hello_unknown_fields_fail_closed() {
        let frame = br#"\x24\0\0\0{"#;
        let _ = frame;
        let json = serde_json::json!({
            "clientVersion": ">=0.4.0, <0.5.0",
            "supportedFeatures": ["typed-errors"],
            "unexpected": true
        });
        let frame = encode_frame(&json).expect("encodes");
        let error = decode_frame::<Hello>("Hello", &frame).expect_err("unknown field fails");
        assert_eq!(error.kind().as_str(), "wire-unknown-field");
        assert!(error.message().contains("unexpected"));
    }

    #[test]
    fn handshake_ignores_unknown_feature_flags() {
        let hello = Hello {
            client_version: SemverRange::new(">=0.4.0, <0.5.0").expect("valid client range"),
            supported_features: vec![
                KnownFeatureFlag::TypedErrors.wire_value(),
                FeatureFlag::new("future-thing").expect("valid unknown feature flag"),
            ],
        };
        let server_version = Version::new("0.4.0").expect("valid version");
        let reply = negotiate_hello(
            &hello,
            &server_version,
            &[
                KnownFeatureFlag::TypedErrors.wire_value(),
                KnownFeatureFlag::ManifestV04.wire_value(),
            ],
        )
        .expect("compatible version");

        assert_eq!(reply.selected_version.as_str(), "0.4.0");
        assert_eq!(
            reply.capabilities,
            vec![KnownFeatureFlag::TypedErrors.wire_value()]
        );
    }

    #[test]
    fn frame_too_large_is_rejected() {
        let oversized = "x".repeat(MAX_FRAME_SIZE + 1);
        let error = encode_frame(&oversized).expect_err("oversized frame fails");
        assert_eq!(error.kind().as_str(), "wire-frame-too-large");
    }

    #[test]
    fn encode_frame_public_sock_cap_boundary_is_exact() {
        // A JSON string of N chars serializes to N+2 bytes (two quotes), so
        // drive the encoded body length to exactly cap-1, cap, and cap+1 to
        // pin the public.sock frame boundary. Removing the `> MAX_FRAME_SIZE`
        // check would let the cap+1 case through and fail this test.
        let body_len = |n: usize| serde_json::to_vec(&"x".repeat(n)).expect("serialize").len();
        // cap - 1 and cap fit.
        let below = "x".repeat(MAX_FRAME_SIZE - 3);
        assert_eq!(body_len(MAX_FRAME_SIZE - 3), MAX_FRAME_SIZE - 1);
        let frame = encode_frame(&below).expect("cap-1 body encodes");
        assert_eq!(frame.len(), 4 + (MAX_FRAME_SIZE - 1));

        let at = "x".repeat(MAX_FRAME_SIZE - 2);
        assert_eq!(body_len(MAX_FRAME_SIZE - 2), MAX_FRAME_SIZE);
        let frame = encode_frame(&at).expect("cap body encodes");
        assert_eq!(frame.len(), 4 + MAX_FRAME_SIZE);

        // cap + 1 fails closed.
        let over = "x".repeat(MAX_FRAME_SIZE - 1);
        assert_eq!(body_len(MAX_FRAME_SIZE - 1), MAX_FRAME_SIZE + 1);
        let error = encode_frame(&over).expect_err("cap+1 body fails");
        assert_eq!(error.kind().as_str(), "wire-frame-too-large");
    }

    #[test]
    fn decode_frame_public_sock_cap_boundary_is_exact() {
        // The declared length prefix is bounded against MAX_FRAME_SIZE before
        // the body is read. A prefix at cap must NOT be rejected as
        // frame-too-large (it fails later for the length mismatch / json),
        // while cap+1 is rejected as frame-too-large. Removing the
        // `declared_length > MAX_FRAME_SIZE` check would change the cap+1
        // error kind and fail this test.
        let mut at_cap = Vec::new();
        at_cap.extend_from_slice(&(MAX_FRAME_SIZE as u32).to_le_bytes());
        at_cap.extend_from_slice(b"{}"); // body shorter than declared on purpose
        let error =
            decode_frame::<crate::HelloOk>("HelloOk", &at_cap).expect_err("cap prefix still fails");
        assert_ne!(
            error.kind().as_str(),
            "wire-frame-too-large",
            "a cap-sized declared length must not be rejected as too large"
        );

        let mut over_cap = Vec::new();
        over_cap.extend_from_slice(&((MAX_FRAME_SIZE + 1) as u32).to_le_bytes());
        over_cap.extend_from_slice(b"{}");
        let error =
            decode_frame::<crate::HelloOk>("HelloOk", &over_cap).expect_err("cap+1 prefix fails");
        assert_eq!(error.kind().as_str(), "wire-frame-too-large");
    }

    #[test]
    fn invalid_ifname_bubbles_up_as_typed_error() {
        // CreateTapFd no longer carries `ifnameDerived` on the wire;
        // the broker derives the ifname server-side from the trusted
        // bundle row keyed by `role_id` + `vm_id`. The legacy "invalid
        // ifname bubbles up" case was reframed to assert that the dropped
        // field is fail-closed-rejected at the wire layer with
        // `wire-unknown-field`, preventing a future caller from supplying
        // it.
        let frame = encode_frame(&serde_json::json!({
            "kind": "CreateTapFd",
            "payload": {
                "ifnameDerived": "abcdefghijklmnop",
                "roleId": "runner",
                "vmId": "corp-vm"
            }
        }))
        .expect("encodes");
        let error =
            decode_frame::<BrokerRequest>("BrokerRequest", &frame).expect_err("ifname fails");
        assert_eq!(error.kind().as_str(), "wire-unknown-field");
    }

    #[test]
    fn hello_round_trip_preserves_length_prefix() {
        let hello = Hello {
            client_version: SemverRange::new(">=0.4.0, <0.5.0").expect("valid range"),
            supported_features: vec![KnownFeatureFlag::TypedErrors.wire_value()],
        };
        let frame = encode_frame(&hello).expect("encodes");
        let decoded = decode_frame::<Hello>("Hello", &frame).expect("decodes");
        assert_eq!(decoded, hello);
    }

    #[test]
    fn protocol_version_is_w3_opaque_id_baseline() {
        // This was bumped from 2 → 3 when the mutating broker variants
        // became opaque-ID-only.
        assert_eq!(PROTOCOL_VERSION, 3);
    }

    #[test]
    fn w3_broker_capabilities_advertise_every_w3_operation() {
        let caps = BrokerCapabilities::w3();
        assert_eq!(caps.protocol_version, PROTOCOL_VERSION);
        for op in W3BrokerOperation::all() {
            assert!(
                caps.broker_operations
                    .iter()
                    .any(|tag| tag == op.wire_tag()),
                "capability set missing {}",
                op.wire_tag()
            );
        }
    }

    #[test]
    fn w3_capabilities_round_trip_via_serde() {
        let caps = BrokerCapabilities::w3();
        let json = serde_json::to_string(&caps).expect("serialize");
        let decoded: BrokerCapabilities = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, caps);
    }
}
