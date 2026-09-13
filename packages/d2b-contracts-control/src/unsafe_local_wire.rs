//! Private unsafe-local helper protocol.
//!
//! The authenticated Unix peer credential is the execution identity. No frame
//! carries a uid, environment, cwd, compositor path, or arbitrary public argv.

use d2b_contracts::{
    configured_argv::ConfiguredArgv, ids::OperationId, token::ProtocolToken,
    workload_identity::WorkloadTarget,
};
pub use d2b_contracts_resource::v3::ZoneResourceIdentity;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

pub const UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION: u32 = 3;
pub const MAX_HELPER_FRAME_SIZE: usize = 256 * 1024;
/// Value requested through `SO_SNDBUF` and `SO_RCVBUF` on both control peers.
pub const HELPER_SOCKET_BUFFER_REQUEST_BYTES: usize = MAX_HELPER_FRAME_SIZE;
/// Minimum value that `getsockopt` must report after Linux doubles the request.
pub const MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES: usize = MAX_HELPER_FRAME_SIZE * 2;
pub const MAX_HELPER_QUEUE_DEPTH: usize = 128;
pub const MAX_HELPER_SNAPSHOT_SCOPES: usize = 1024;
pub const MAX_COMPLETED_OPERATIONS_PER_UID: usize = 1024;
pub const MAX_COMPLETED_OPERATION_AGE_SECS: u64 = 24 * 60 * 60;

pub const fn unsafe_local_helper_protocol_supported(version: u32) -> bool {
    version == UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperHello {
    pub protocol_version: u32,
    pub generation: u64,
    #[serde(default)]
    pub features: Vec<ProtocolToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperHelloAccepted {
    pub protocol_version: u32,
    pub generation: u64,
    pub heartbeat_interval_secs: u32,
    pub operation_timeout_secs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperHeartbeat {
    pub generation: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HelperScopeKind {
    LauncherApp,
    WaylandProxy,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScopeIdentity {
    pub invocation_id: String,
    pub kind: HelperScopeKind,
}

impl fmt::Debug for ScopeIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopeIdentity")
            .field("invocation_id", &"<redacted>")
            .field("kind", &self.kind)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HelperScopeState {
    Starting,
    Active,
    Stopping,
    Exited,
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperScopeSnapshot {
    pub operation_id: OperationId,
    pub workload: ZoneResourceIdentity,
    pub scope: ScopeIdentity,
    pub state: HelperScopeState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperSnapshot {
    pub generation: u64,
    pub scopes: Vec<HelperScopeSnapshot>,
}

impl HelperSnapshot {
    pub fn validate(&self) -> Result<(), HelperFailureCode> {
        if self.generation == 0 {
            return Err(HelperFailureCode::InvalidRequest);
        }
        if self.scopes.len() > MAX_HELPER_SNAPSHOT_SCOPES {
            return Err(HelperFailureCode::InvalidRequest);
        }
        self.scopes.iter().try_for_each(|scope| {
            validate_unsafe_local_resource_identity(&scope.workload)
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HelperSnapshotWire {
    generation: u64,
    scopes: Vec<HelperScopeSnapshot>,
}

impl<'de> Deserialize<'de> for HelperSnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = HelperSnapshotWire::deserialize(deserializer)?;
        let snapshot = Self {
            generation: wire.generation,
            scopes: wire.scopes,
        };
        snapshot
            .validate()
            .map_err(|_| serde::de::Error::custom("invalid helper snapshot"))?;
        Ok(snapshot)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperLaunchRequest {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub workload: ZoneResourceIdentity,
    pub target: WorkloadTarget,
    pub item_id: ProtocolToken,
    pub argv: ConfiguredArgv,
    pub graphical: bool,
    pub realm_accent_color: RealmAccentColor,
}

impl fmt::Debug for HelperLaunchRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelperLaunchRequest")
            .field("request_id", &self.request_id)
            .field("operation_id", &self.operation_id)
            .field("workload", &self.workload)
            .field("target", &"<redacted>")
            .field("item_id", &self.item_id)
            .field("argv_count", &self.argv.as_slice().len())
            .field("graphical", &self.graphical)
            .field("realm_accent_color", &self.realm_accent_color)
            .finish()
    }
}

impl HelperLaunchRequest {
    pub fn validate_bounds(&self) -> Result<(), HelperFailureCode> {
        validate_unsafe_local_resource_identity(&self.workload)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HelperLaunchRequestWire {
    request_id: u64,
    operation_id: OperationId,
    workload: ZoneResourceIdentity,
    target: WorkloadTarget,
    item_id: ProtocolToken,
    argv: ConfiguredArgv,
    graphical: bool,
    realm_accent_color: RealmAccentColor,
}

impl<'de> Deserialize<'de> for HelperLaunchRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = HelperLaunchRequestWire::deserialize(deserializer)?;
        let request = Self {
            request_id: wire.request_id,
            operation_id: wire.operation_id,
            workload: wire.workload,
            target: wire.target,
            item_id: wire.item_id,
            argv: wire.argv,
            graphical: wire.graphical,
            realm_accent_color: wire.realm_accent_color,
        };
        request
            .validate_bounds()
            .map_err(|_| serde::de::Error::custom("invalid helper launch request"))?;
        Ok(request)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
pub struct RealmAccentColor(#[schemars(regex(pattern = "^#[0-9a-f]{6}$"))] String);

impl RealmAccentColor {
    pub fn new(value: impl Into<String>) -> Result<Self, HelperFailureCode> {
        let value = value.into();
        let valid = value.len() == 7
            && value.starts_with('#')
            && value[1..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        valid
            .then_some(Self(value))
            .ok_or(HelperFailureCode::InvalidRequest)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RealmAccentColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RealmAccentColor(<validated>)")
    }
}

impl<'de> Deserialize<'de> for RealmAccentColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?)
            .map_err(|_| serde::de::Error::custom("realm accent color must match ^#[0-9a-f]{6}$"))
    }
}

pub fn validate_unsafe_local_resource_identity(
    identity: &ZoneResourceIdentity,
) -> Result<(), HelperFailureCode> {
    matches!(
        identity.resource_ref().resource_type().as_str(),
        "Host" | "Guest" | "Process" | "EphemeralProcess"
    )
    .then_some(())
    .ok_or(HelperFailureCode::InvalidRequest)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HelperFailureCode {
    InvalidRequest,
    OperationIdConflict,
    QueueFull,
    Timeout,
    UserManagerUnavailable,
    EnvironmentInvalid,
    ExecutableUnavailable,
    ScopeCreateFailed,
    ScopeIdentityMismatch,
    GraphicalSessionInactive,
    WaylandUnavailable,
    ProxyUnavailable,
    FirstClientTimeout,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HelperOperationDisposition {
    Committed,
    AlreadyCommitted,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperOperationResult {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub disposition: HelperOperationDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ScopeIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HelperOperationRejected {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub code: HelperFailureCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum DaemonToUnsafeLocalHelper {
    HelloAccepted(HelperHelloAccepted),
    Heartbeat(HelperHeartbeat),
    Launch(HelperLaunchRequest),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum UnsafeLocalHelperToDaemon {
    Hello(HelperHello),
    Snapshot(HelperSnapshot),
    Heartbeat(HelperHeartbeat),
    Operation(HelperOperationResult),
    Rejected(HelperOperationRejected),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UnsafeLocalHelperWireSchema {
    pub protocol_version: u32,
    pub daemon_to_helper: DaemonToUnsafeLocalHelper,
    pub helper_to_daemon: UnsafeLocalHelperToDaemon,
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::{configured_argv::ConfiguredArgv, workload_identity::WorkloadTarget};
    use d2b_contracts_resource::v3::{
        ResourceGeneration, ResourceRef, ResourceUid, ZoneId, ZoneResourceIdentity, ZoneRevision,
    };
    use serde::de::DeserializeOwned;

    fn workload() -> ZoneResourceIdentity {
        zone_identity(
            "work",
            "123e4567-e89b-42d3-a456-426614174000",
            "323e4567-e89b-42d3-a456-426614174002",
            1,
        )
    }

    fn zone_identity(
        zone: &str,
        zone_uid: &str,
        resource_uid: &str,
        generation: u64,
    ) -> ZoneResourceIdentity {
        ZoneResourceIdentity::new(
            ZoneId::parse(zone).unwrap(),
            ResourceUid::parse(zone_uid).unwrap(),
            ResourceRef::parse("Process/tools").unwrap(),
            ResourceUid::parse(resource_uid).unwrap(),
            ResourceGeneration::new(generation).unwrap(),
            ZoneRevision::new(1),
        )
    }

    fn operation(value: &str) -> OperationId {
        OperationId::parse(value).unwrap()
    }

    #[test]
    fn zone_identity_fences_same_name_requests_and_excludes_realm_fields() {
        let work = zone_identity(
            "work",
            "123e4567-e89b-42d3-a456-426614174000",
            "323e4567-e89b-42d3-a456-426614174002",
            3,
        );
        let personal = zone_identity(
            "personal",
            "223e4567-e89b-42d3-a456-426614174001",
            "423e4567-e89b-42d3-a456-426614174003",
            3,
        );
        assert_ne!(work, personal);
        let encoded = serde_json::to_value(&work).unwrap();
        assert_eq!(encoded["zone"], "work");
        assert_eq!(encoded["resourceRef"], "Process/tools");
        assert_eq!(encoded["generation"], 3);
        assert!(encoded.get("realmId").is_none());
        assert!(encoded.get("realmPath").is_none());
        assert!(encoded.get("canonicalTarget").is_none());
        assert_eq!(format!("{work:?}"), "ZoneResourceIdentity(<redacted>)");
        let mut legacy = encoded.clone();
        legacy["realmId"] = serde_json::json!("work");
        assert!(serde_json::from_value::<ZoneResourceIdentity>(legacy).is_err());

        let launch = HelperLaunchRequest {
            request_id: 2,
            operation_id: operation("op-zone-launch"),
            workload: work,
            target: WorkloadTarget::parse("tools.work.d2b").unwrap(),
            item_id: ProtocolToken::parse("browser").unwrap(),
            argv: ConfiguredArgv::new(vec!["private-argv-canary".to_owned()]).unwrap(),
            graphical: false,
            realm_accent_color: RealmAccentColor::new("#cc3344").unwrap(),
        };
        round_trip(&launch);
        assert!(!format!("{launch:?}").contains("tools.work.d2b"));
        assert!(!format!("{launch:?}").contains("private-argv-canary"));
    }

    #[test]
    fn zone_identity_changes_are_not_accepted_as_the_same_resource() {
        let current = zone_identity(
            "work",
            "123e4567-e89b-42d3-a456-426614174000",
            "323e4567-e89b-42d3-a456-426614174002",
            3,
        );
        let stale_uid = zone_identity(
            "work",
            "123e4567-e89b-42d3-a456-426614174000",
            "423e4567-e89b-42d3-a456-426614174003",
            3,
        );
        let stale_generation = zone_identity(
            "work",
            "123e4567-e89b-42d3-a456-426614174000",
            "323e4567-e89b-42d3-a456-426614174002",
            4,
        );
        assert_ne!(current, stale_uid);
        assert_ne!(current, stale_generation);
        assert_eq!(current.resource_ref(), stale_uid.resource_ref());
        assert_eq!(current.resource_ref(), stale_generation.resource_ref());
    }

    #[test]
    fn helper_requests_reject_non_execution_resource_identities() {
        let invalid = ZoneResourceIdentity::new(
            ZoneId::parse("work").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ResourceRef::parse("Volume/secret").unwrap(),
            ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap(),
            ResourceGeneration::new(1).unwrap(),
            ZoneRevision::new(1),
        );
        let mut launch = serde_json::to_value(HelperLaunchRequest {
            request_id: 2,
            operation_id: operation("op-invalid-launch"),
            workload: workload(),
            target: WorkloadTarget::parse("tools.work.d2b").unwrap(),
            item_id: ProtocolToken::parse("browser").unwrap(),
            argv: ConfiguredArgv::new(vec!["browser".to_owned()]).unwrap(),
            graphical: false,
            realm_accent_color: RealmAccentColor::new("#cc3344").unwrap(),
        })
        .unwrap();
        launch["workload"] = serde_json::to_value(&invalid).unwrap();
        assert!(serde_json::from_value::<HelperLaunchRequest>(launch).is_err());

        let snapshot = serde_json::json!({
            "generation": 1,
            "scopes": [{
                "operationId": "op-invalid-snapshot",
                "workload": serde_json::to_value(invalid).unwrap(),
                "scope": {
                    "invocationId": "00112233445566778899aabbccddeeff",
                    "kind": "launcher-app"
                },
                "state": "active"
            }]
        });
        assert!(serde_json::from_value::<HelperSnapshot>(snapshot).is_err());
    }

    fn round_trip<T>(value: &T)
    where
        T: Serialize + DeserializeOwned + PartialEq + fmt::Debug,
    {
        let encoded = serde_json::to_vec(value).unwrap();
        let decoded: T = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(&decoded, value);
    }

    #[test]
    fn launch_requests_round_trip_and_correlate() {
        let request = HelperLaunchRequest {
            request_id: 2,
            operation_id: operation("op-launch"),
            workload: workload(),
            target: WorkloadTarget::parse("tools.work.d2b").unwrap(),
            item_id: ProtocolToken::parse("browser").unwrap(),
            argv: ConfiguredArgv::new(vec!["browser".to_owned()]).unwrap(),
            graphical: true,
            realm_accent_color: RealmAccentColor::new("#cc3344").unwrap(),
        };
        round_trip(&request);
        let encoded = serde_json::to_string(&request).unwrap();
        assert!(!encoded.contains("request_id"));
        assert!(!encoded.contains("operation_id"));
    }

    #[test]
    fn helper_frames_reject_unknown_and_forbidden_fields() {
        let hello = r#"{
          "type":"hello",
          "payload":{"protocolVersion":3,"generation":1,"features":[],"uid":1000}
        }"#;
        assert!(serde_json::from_str::<UnsafeLocalHelperToDaemon>(hello).is_err());

        let launch = serde_json::json!({
            "type": "launch",
            "payload": {
                "requestId": 1,
                "operationId": "op-launch",
                "workload": serde_json::to_value(workload()).unwrap(),
                "target": "tools.work.d2b",
                "itemId": "browser",
                "argv": ["browser"],
                "graphical": false,
                "realmAccentColor": "#cc3344",
                "cwd": "/forbidden"
            }
        });
        assert!(serde_json::from_value::<DaemonToUnsafeLocalHelper>(launch).is_err());
    }

    #[test]
    fn older_helper_versions_are_rejected() {
        assert_eq!(UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION, 3);
        assert!(!unsafe_local_helper_protocol_supported(1));
        assert!(!unsafe_local_helper_protocol_supported(2));
        assert!(unsafe_local_helper_protocol_supported(3));
    }

    #[test]
    fn realm_accent_color_is_strict_and_canonical() {
        let color = RealmAccentColor::new("#cc3344").unwrap();
        assert_eq!(color.as_str(), "#cc3344");
        for invalid in [
            "cc3344",
            "#CC3344",
            "#123",
            "#1234567",
            "#12345g",
            "#123456\n",
        ] {
            assert!(RealmAccentColor::new(invalid).is_err(), "{invalid:?}");
            assert!(
                serde_json::from_value::<RealmAccentColor>(serde_json::json!(invalid)).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn helper_socket_buffer_floors_stay_closed() {
        assert_eq!(HELPER_SOCKET_BUFFER_REQUEST_BYTES, MAX_HELPER_FRAME_SIZE);
        assert_eq!(
            MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES,
            MAX_HELPER_FRAME_SIZE * 2
        );
    }
}
