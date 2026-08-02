//! Child-local ZoneLink ResourceType contract.
//!
//! A ZoneLink is stored only in the child Zone.  Its transport settings are
//! opaque canonical configuration, its credentials are same-Zone Credential
//! references, and its status contains only bounded connection/cursor
//! observations.  No parent resource reference, locator, descriptor, or
//! credential bytes can be represented here.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    CanonicalJsonObject, ResourceRef, ResourceUid, Timestamp, ZoneId,
    execution_policy::redacted_debug,
};

/// The canonical ZoneLink ResourceType name.
pub const ZONE_LINK_RESOURCE_TYPE: &str = "ZoneLink";
/// The Core drain finalizer for a ZoneLink.
pub const ZONE_LINK_DRAIN_FINALIZER: &str = "core.zone-link-drain";
/// Maximum Credential references in one ZoneLink.
pub const MAX_ZONE_LINK_CREDENTIALS: usize = 8;
/// Maximum persisted local intents.
pub const MAX_ZONE_LINK_INTENTS: u32 = 256;
/// Maximum active named streams admitted by the ZoneLink schema.
pub const MAX_ZONE_LINK_ACTIVE_STREAMS: u32 = 128;

/// ZoneLink schema and lifecycle failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneLinkContractError {
    InvalidChildZone,
    InvalidTransportProvider,
    WrongResourceType,
    TooManyCredentials,
    DuplicateCredential,
    InvalidLimits,
    InvalidIntentCount,
    InvalidReference,
    BoundExceeded,
}

impl core::fmt::Display for ZoneLinkContractError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidChildZone => "zonelink-child-zone-invalid",
            Self::InvalidTransportProvider => "zonelink-transport-provider-invalid",
            Self::WrongResourceType => "zonelink-reference-type-invalid",
            Self::TooManyCredentials => "zonelink-credential-bound-exceeded",
            Self::DuplicateCredential => "zonelink-duplicate-credential",
            Self::InvalidLimits => "zonelink-limits-invalid",
            Self::InvalidIntentCount => "zonelink-intent-queue-full",
            Self::InvalidReference => "zonelink-reference-invalid",
            Self::BoundExceeded => "zonelink-bound-exceeded",
        })
    }
}

impl std::error::Error for ZoneLinkContractError {}

/// ZoneLink connection and queue limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ZoneLinkLimits {
    max_pending_intents: u32,
    max_active_streams: u32,
    reconnect_max_attempts: u32,
    reconnect_window_secs: u32,
}

impl ZoneLinkLimits {
    /// Construct validated limits.
    pub const fn new(
        max_pending_intents: u32,
        max_active_streams: u32,
        reconnect_max_attempts: u32,
        reconnect_window_secs: u32,
    ) -> Result<Self, ZoneLinkContractError> {
        if max_pending_intents == 0
            || max_pending_intents > MAX_ZONE_LINK_INTENTS
            || max_active_streams == 0
            || max_active_streams > MAX_ZONE_LINK_ACTIVE_STREAMS
            || reconnect_max_attempts == 0
            || reconnect_window_secs == 0
        {
            return Err(ZoneLinkContractError::InvalidLimits);
        }
        Ok(Self {
            max_pending_intents,
            max_active_streams,
            reconnect_max_attempts,
            reconnect_window_secs,
        })
    }

    /// Default protocol limits.
    pub const fn default_values() -> Self {
        Self {
            max_pending_intents: 256,
            max_active_streams: 32,
            reconnect_max_attempts: 10,
            reconnect_window_secs: 300,
        }
    }

    /// Return the pending-intent bound.
    pub const fn max_pending_intents(self) -> u32 {
        self.max_pending_intents
    }

    /// Return the active-stream bound.
    pub const fn max_active_streams(self) -> u32 {
        self.max_active_streams
    }

    /// Return the reconnect-attempt bound.
    pub const fn reconnect_max_attempts(self) -> u32 {
        self.reconnect_max_attempts
    }

    /// Return the reconnect window in seconds.
    pub const fn reconnect_window_secs(self) -> u32 {
        self.reconnect_window_secs
    }
}

impl Default for ZoneLinkLimits {
    fn default() -> Self {
        Self::default_values()
    }
}

impl<'de> Deserialize<'de> for ZoneLinkLimits {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default = "default_pending")]
            max_pending_intents: u32,
            #[serde(default = "default_streams")]
            max_active_streams: u32,
            #[serde(default = "default_attempts")]
            reconnect_max_attempts: u32,
            #[serde(default = "default_window")]
            reconnect_window_secs: u32,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.max_pending_intents,
            wire.max_active_streams,
            wire.reconnect_max_attempts,
            wire.reconnect_window_secs,
        )
        .map_err(serde::de::Error::custom)
    }
}

const fn default_pending() -> u32 {
    256
}
const fn default_streams() -> u32 {
    32
}
const fn default_attempts() -> u32 {
    10
}
const fn default_window() -> u32 {
    300
}

/// Desired state for a child-local ZoneLink.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZoneLinkSpec {
    child_zone_name: ZoneId,
    transport_provider_ref: ResourceRef,
    transport_settings: CanonicalJsonObject,
    transport_credentials: Vec<ResourceRef>,
    disabled: bool,
    limits: ZoneLinkLimits,
}

impl ZoneLinkSpec {
    /// Construct the strict ZoneLink desired state.
    pub fn new(
        child_zone_name: ZoneId,
        transport_provider_ref: ResourceRef,
        transport_settings: CanonicalJsonObject,
        mut transport_credentials: Vec<ResourceRef>,
        disabled: bool,
        limits: ZoneLinkLimits,
    ) -> Result<Self, ZoneLinkContractError> {
        if !transport_provider_ref
            .resource_type()
            .as_str()
            .eq("Provider")
            || !transport_provider_ref
                .name()
                .as_str()
                .starts_with("transport-")
        {
            return Err(ZoneLinkContractError::InvalidTransportProvider);
        }
        if transport_credentials.len() > MAX_ZONE_LINK_CREDENTIALS {
            return Err(ZoneLinkContractError::TooManyCredentials);
        }
        if transport_credentials
            .iter()
            .any(|reference| reference.resource_type().as_str() != "Credential")
        {
            return Err(ZoneLinkContractError::WrongResourceType);
        }
        transport_credentials.sort();
        if transport_credentials
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(ZoneLinkContractError::DuplicateCredential);
        }
        Ok(Self {
            child_zone_name,
            transport_provider_ref,
            transport_settings,
            transport_credentials,
            disabled,
            limits,
        })
    }

    /// Borrow the child Zone name; the controller must compare it to the
    /// enclosing store's self-resource name.
    pub const fn child_zone_name(&self) -> &ZoneId {
        &self.child_zone_name
    }

    /// Borrow the same-Zone transport Provider.
    pub const fn transport_provider_ref(&self) -> &ResourceRef {
        &self.transport_provider_ref
    }

    /// Borrow provider-specific transport settings.
    pub const fn transport_settings(&self) -> &CanonicalJsonObject {
        &self.transport_settings
    }

    /// Borrow Credential references.
    pub fn transport_credentials(&self) -> &[ResourceRef] {
        &self.transport_credentials
    }

    /// Whether reconnect is disabled by the operator.
    pub const fn disabled(&self) -> bool {
        self.disabled
    }

    /// Return the bounded connection limits.
    pub const fn limits(&self) -> ZoneLinkLimits {
        self.limits
    }

    /// Validate the self-name invariant against the enclosing Zone.
    pub fn validate_child_zone(
        &self,
        enclosing_zone: &ZoneId,
    ) -> Result<(), ZoneLinkContractError> {
        if &self.child_zone_name == enclosing_zone {
            Ok(())
        } else {
            Err(ZoneLinkContractError::InvalidChildZone)
        }
    }
}

redacted_debug!(ZoneLinkSpec);

impl<'de> Deserialize<'de> for ZoneLinkSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            child_zone_name: ZoneId,
            transport_provider_ref: ResourceRef,
            #[serde(default)]
            transport_settings: CanonicalJsonObject,
            #[serde(default)]
            transport_credentials: Vec<ResourceRef>,
            #[serde(default)]
            disabled: bool,
            #[serde(default)]
            limits: ZoneLinkLimits,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.child_zone_name,
            wire.transport_provider_ref,
            wire.transport_settings,
            wire.transport_credentials,
            wire.disabled,
            wire.limits,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Closed ZoneLink condition names.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ZoneLinkConditionType {
    TransportReachable,
    SessionEstablished,
    ChildAuthorized,
    CursorSynchronized,
    LocalIntentsDrained,
    DisabledByOperator,
    IntentApplicationFailed,
}

/// ResourceType-common ZoneLink status layer.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ZoneLinkStatusResource {
    child_zone_uid: Option<ResourceUid>,
    connected: bool,
    last_connected_at: Option<Timestamp>,
    last_disconnected_at: Option<Timestamp>,
    last_sent_revision: Option<u64>,
    last_acked_revision: Option<u64>,
    last_received_revision: Option<u64>,
    last_applied_revision: Option<u64>,
    link_epoch: u64,
    pending_local_intents: u32,
    child_authorized: bool,
}

impl ZoneLinkStatusResource {
    /// Construct status and enforce cursor/queue bounds.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        child_zone_uid: Option<ResourceUid>,
        connected: bool,
        last_connected_at: Option<Timestamp>,
        last_disconnected_at: Option<Timestamp>,
        last_sent_revision: Option<u64>,
        last_acked_revision: Option<u64>,
        last_received_revision: Option<u64>,
        last_applied_revision: Option<u64>,
        link_epoch: u64,
        pending_local_intents: u32,
        child_authorized: bool,
    ) -> Result<Self, ZoneLinkContractError> {
        if pending_local_intents > MAX_ZONE_LINK_INTENTS
            || !ordered(last_sent_revision, last_acked_revision)
            || !ordered(last_received_revision, last_applied_revision)
        {
            return Err(ZoneLinkContractError::BoundExceeded);
        }
        Ok(Self {
            child_zone_uid,
            connected,
            last_connected_at,
            last_disconnected_at,
            last_sent_revision,
            last_acked_revision,
            last_received_revision,
            last_applied_revision,
            link_epoch,
            pending_local_intents,
            child_authorized,
        })
    }

    /// Return the acknowledged child Zone UID.
    pub const fn child_zone_uid(&self) -> Option<&ResourceUid> {
        self.child_zone_uid.as_ref()
    }

    /// Whether the allocator-bound session is connected.
    pub const fn connected(&self) -> bool {
        self.connected
    }

    /// Return the session epoch.
    pub const fn link_epoch(&self) -> u64 {
        self.link_epoch
    }

    /// Return the pending local intent count.
    pub const fn pending_local_intents(&self) -> u32 {
        self.pending_local_intents
    }

    /// Whether the parent authorized the child.
    pub const fn child_authorized(&self) -> bool {
        self.child_authorized
    }
}

fn ordered(left: Option<u64>, right: Option<u64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => right <= left,
        _ => true,
    }
}

redacted_debug!(ZoneLinkStatusResource);

impl<'de> Deserialize<'de> for ZoneLinkStatusResource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            child_zone_uid: Option<ResourceUid>,
            connected: bool,
            last_connected_at: Option<Timestamp>,
            last_disconnected_at: Option<Timestamp>,
            last_sent_revision: Option<u64>,
            last_acked_revision: Option<u64>,
            last_received_revision: Option<u64>,
            last_applied_revision: Option<u64>,
            link_epoch: u64,
            pending_local_intents: u32,
            child_authorized: bool,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.child_zone_uid,
            wire.connected,
            wire.last_connected_at,
            wire.last_disconnected_at,
            wire.last_sent_revision,
            wire.last_acked_revision,
            wire.last_received_revision,
            wire.last_applied_revision,
            wire.link_epoch,
            wire.pending_local_intents,
            wire.child_authorized,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Alias used by generic status adapters.
pub type ZoneLinkStatus = ZoneLinkStatusResource;

/// Record admission of one locally queued intent.
pub const fn admit_local_intent(
    pending: u32,
    limits: ZoneLinkLimits,
) -> Result<u32, ZoneLinkContractError> {
    if pending >= limits.max_pending_intents() {
        Err(ZoneLinkContractError::InvalidIntentCount)
    } else {
        Ok(pending + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(name: &str) -> ResourceRef {
        ResourceRef::parse(&format!("Provider/{name}")).unwrap()
    }

    #[test]
    fn child_name_and_transport_provider_are_closed() {
        let spec = ZoneLinkSpec::new(
            ZoneId::parse("guest").unwrap(),
            provider("transport-unix"),
            CanonicalJsonObject::empty(),
            Vec::new(),
            false,
            ZoneLinkLimits::default(),
        )
        .unwrap();
        assert!(
            spec.validate_child_zone(&ZoneId::parse("guest").unwrap())
                .is_ok()
        );
        assert!(
            ZoneLinkSpec::new(
                ZoneId::parse("guest").unwrap(),
                provider("runtime"),
                CanonicalJsonObject::empty(),
                Vec::new(),
                false,
                ZoneLinkLimits::default(),
            )
            .is_err()
        );
    }

    #[test]
    fn local_intent_queue_is_bounded() {
        let limits = ZoneLinkLimits::default();
        assert_eq!(admit_local_intent(255, limits), Ok(256));
        assert_eq!(
            admit_local_intent(256, limits),
            Err(ZoneLinkContractError::InvalidIntentCount)
        );
    }
}
