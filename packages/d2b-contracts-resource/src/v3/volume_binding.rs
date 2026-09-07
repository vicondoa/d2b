//! Neutral `VolumeBinding` ResourceType contracts.
//!
//! One `VolumeBinding` is the durable neutral record of one Volume /
//! execution-target / named-view relationship.  It is a standard unqualified
//! core type: the Volume side mints it and owns the relationship, and
//! `volume-virtiofs` only observes it and writes its fenced status
//! projection.  The spec never carries a host path, socket path,
//! shared-directory path, argv, or numeric identity.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    ResourceRef, ResourceUid,
    execution_policy::{BoundedToken, PrimitiveSpecError},
    identity::{ResourceGeneration, ZoneRevision},
    resource_status::{ResourceCondition, ResourcePhase, StatusCode},
    volume::{AttachmentAccess, validate_mount_path},
};

/// Canonical standard VolumeBinding ResourceType.
pub const VOLUME_BINDING_RESOURCE_TYPE: &str = "VolumeBinding";

/// Maximum Guest-visible mount path length.
pub const MAX_BINDING_MOUNT_PATH_BYTES: usize = 255;

/// Strict base VolumeBinding specification.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VolumeBindingSpec {
    volume_ref: ResourceRef,
    execution_ref: ResourceRef,
    view: BoundedToken,
    access: AttachmentAccess,
    mount_path: String,
}

impl VolumeBindingSpec {
    /// Construct a strict binding specification from typed references.
    pub fn new(
        volume_ref: ResourceRef,
        execution_ref: ResourceRef,
        view: impl Into<String>,
        access: AttachmentAccess,
        mount_path: impl Into<String>,
    ) -> Result<Self, PrimitiveSpecError> {
        if volume_ref.resource_type().as_str() != "Volume"
            || execution_ref.resource_type().as_str() != "Guest"
        {
            return Err(PrimitiveSpecError::WrongResourceType);
        }
        let view = BoundedToken::parse(view.into())?;
        let mount_path = mount_path.into();
        if !validate_mount_path(&mount_path) {
            return Err(PrimitiveSpecError::InvalidPath);
        }
        Ok(Self {
            volume_ref,
            execution_ref,
            view,
            access,
            mount_path,
        })
    }

    /// Return the standard ResourceType name.
    pub const fn resource_type(&self) -> &'static str {
        VOLUME_BINDING_RESOURCE_TYPE
    }

    /// Borrow the bound Volume.
    pub const fn volume_ref(&self) -> &ResourceRef {
        &self.volume_ref
    }

    /// Borrow the Guest execution target.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the named Volume view.
    pub const fn view(&self) -> &BoundedToken {
        &self.view
    }

    /// Return the requested access mode.
    pub const fn access(&self) -> AttachmentAccess {
        self.access
    }

    /// Borrow the Guest-visible mount path.
    pub fn mount_path(&self) -> &str {
        &self.mount_path
    }

    /// Admit one binding create or update against its owner reference.
    ///
    /// Every binding mutation must be owned by an existing Volume: a create
    /// or update without a Volume owner reference is a direct external
    /// create and is rejected.
    pub fn admit_owner_ref(owner_ref: Option<&ResourceRef>) -> Result<(), PrimitiveSpecError> {
        match owner_ref {
            Some(owner) if owner.resource_type().as_str() == "Volume" => Ok(()),
            Some(_) => Err(PrimitiveSpecError::WrongResourceType),
            None => Err(PrimitiveSpecError::MissingRequiredField),
        }
    }
}

impl core::fmt::Debug for VolumeBindingSpec {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("VolumeBindingSpec(<redacted>)")
    }
}

impl<'de> Deserialize<'de> for VolumeBindingSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            volume_ref: ResourceRef,
            execution_ref: ResourceRef,
            view: String,
            access: AttachmentAccess,
            mount_path: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.volume_ref,
            wire.execution_ref,
            wire.view,
            wire.access,
            wire.mount_path,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// The UID / generation / revision fence on readiness evidence.
///
/// A readiness report is only current when the UID and generation match the
/// binding's own identity and the fence revision is at most the stored
/// revision.  Every store mutation (including the status write carrying the
/// report itself) advances the stored revision past the observed one, so an
/// exact-revision rule could never latch; the UID pins reassignment and the
/// generation pins spec changes, which together bound report staleness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolumeBindingReadinessFence {
    /// The binding UID the evidence was observed under.
    pub uid: ResourceUid,
    /// The binding spec generation the evidence was observed under.
    pub generation: ResourceGeneration,
    /// The Zone-store revision the evidence was observed under.
    pub revision: ZoneRevision,
}

impl VolumeBindingReadinessFence {
    /// Whether this fence still matches the binding's current identity.
    pub fn matches(
        &self,
        uid: &ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
    ) -> bool {
        self.uid == *uid && self.generation == generation && self.revision <= revision
    }
}

/// Public VolumeBinding status resource projection.
///
/// It exposes only fenced readiness and a stable safe failure code.  The
/// worker, endpoint, socket, resolved paths, argv, and numeric identities
/// stay provider-private and never appear here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolumeBindingStatusResource {
    /// Whether the serving side reports the relationship ready.
    pub ready: bool,
    /// The fence the readiness evidence was observed under.
    pub fence: VolumeBindingReadinessFence,
    /// The stable safe failure code, when not ready.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<StatusCode>,
}

impl VolumeBindingStatusResource {
    /// Whether this projection reports ready under the current fence.
    ///
    /// Readiness without a matching UID and generation is never current
    /// (fail-closed).  The fence revision only needs to precede the stored
    /// revision: the status write carrying the report advances the store
    /// past the observed commit.
    pub fn readiness_is_current(
        &self,
        uid: &ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
    ) -> bool {
        self.ready && self.fence.matches(uid, generation, revision)
    }
}

/// Public VolumeBinding status projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolumeBindingStatus {
    /// Universal lifecycle phase.
    pub phase: ResourcePhase,
    /// Universal conditions.
    #[serde(default)]
    pub conditions: Vec<ResourceCondition>,
    /// Binding-specific fenced readiness facts.
    pub resource: VolumeBindingStatusResource,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume_ref() -> ResourceRef {
        ResourceRef::parse("Volume/work-state").expect("valid fixture ref")
    }

    fn guest_ref() -> ResourceRef {
        ResourceRef::parse("Guest/work-vm").expect("valid fixture ref")
    }

    fn spec() -> VolumeBindingSpec {
        VolumeBindingSpec::new(
            volume_ref(),
            guest_ref(),
            "ro-store",
            AttachmentAccess::ReadOnly,
            "/nix/.ro-store",
        )
        .expect("valid fixture spec")
    }

    fn uid() -> ResourceUid {
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("valid fixture uid")
    }

    fn fence() -> VolumeBindingReadinessFence {
        VolumeBindingReadinessFence {
            uid: uid(),
            generation: ResourceGeneration::new(1).expect("nonzero generation"),
            revision: ZoneRevision::new(9),
        }
    }

    #[test]
    fn spec_serializes_and_round_trips_strictly() {
        let value = serde_json::to_value(spec()).expect("spec serializes");
        assert_eq!(
            value,
            serde_json::json!({
                "volumeRef": "Volume/work-state",
                "executionRef": "Guest/work-vm",
                "view": "ro-store",
                "access": "read-only",
                "mountPath": "/nix/.ro-store",
            })
        );
        let parsed: VolumeBindingSpec =
            serde_json::from_value(value).expect("spec round trips");
        assert_eq!(parsed, spec());
    }

    #[test]
    fn spec_rejects_unknown_fields_and_wrong_references() {
        let mut unknown = serde_json::to_value(spec()).expect("spec serializes");
        unknown["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<VolumeBindingSpec>(unknown).is_err());

        let wrong_volume = VolumeBindingSpec::new(
            ResourceRef::parse("Guest/work-vm").expect("valid fixture ref"),
            guest_ref(),
            "ro-store",
            AttachmentAccess::ReadOnly,
            "/nix/.ro-store",
        );
        assert_eq!(wrong_volume.unwrap_err(), PrimitiveSpecError::WrongResourceType);

        let invalid_path = VolumeBindingSpec::new(
            volume_ref(),
            guest_ref(),
            "ro-store",
            AttachmentAccess::ReadOnly,
            "relative/path",
        );
        assert_eq!(invalid_path.unwrap_err(), PrimitiveSpecError::InvalidPath);
    }

    #[test]
    fn admission_rejects_direct_external_creates_without_a_volume_owner() {
        assert!(VolumeBindingSpec::admit_owner_ref(Some(&volume_ref())).is_ok());
        assert_eq!(
            VolumeBindingSpec::admit_owner_ref(None).unwrap_err(),
            PrimitiveSpecError::MissingRequiredField,
            "owner-less binding create must be rejected"
        );
        assert_eq!(
            VolumeBindingSpec::admit_owner_ref(Some(&guest_ref())).unwrap_err(),
            PrimitiveSpecError::WrongResourceType,
            "a non-Volume owner reference must be rejected"
        );
    }

    #[test]
    fn fence_structural_validity_is_enforced_by_typed_fields() {
        let value = serde_json::to_value(fence()).expect("fence serializes");
        assert_eq!(
            value,
            serde_json::json!({
                "uid": "123e4567-e89b-42d3-a456-426614174000",
                "generation": 1,
                "revision": 9,
            })
        );
        assert!(
            serde_json::from_value::<VolumeBindingReadinessFence>(value).is_ok(),
            "a structurally valid fence parses"
        );

        let mut malformed = serde_json::to_value(fence()).expect("fence serializes");
        malformed["uid"] = serde_json::json!("not-a-uid");
        assert!(
            serde_json::from_value::<VolumeBindingReadinessFence>(malformed).is_err(),
            "a fence with a malformed UID must be rejected"
        );

        let mut zeroed = serde_json::to_value(fence()).expect("fence serializes");
        zeroed["generation"] = serde_json::json!(0);
        assert!(
            serde_json::from_value::<VolumeBindingReadinessFence>(zeroed).is_err(),
            "a fence with a zero generation must be rejected"
        );

        let mut unknown = serde_json::to_value(fence()).expect("fence serializes");
        unknown["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<VolumeBindingReadinessFence>(unknown).is_err());
    }

    #[test]
    fn stale_or_reassigned_readiness_never_reports_current() {
        let status = VolumeBindingStatusResource {
            ready: true,
            fence: fence(),
            reason: None,
        };
        assert!(status.readiness_is_current(
            &uid(),
            ResourceGeneration::new(1).expect("nonzero generation"),
            ZoneRevision::new(9),
        ));

        // Wrong UID: a reassigned binding name must not report ready.
        let reassigned = ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001")
            .expect("valid fixture uid");
        assert!(!status.readiness_is_current(
            &reassigned,
            ResourceGeneration::new(1).expect("nonzero generation"),
            ZoneRevision::new(9),
        ));

        // Older generation: a superseded spec must not report ready.
        assert!(!status.readiness_is_current(
            &uid(),
            ResourceGeneration::new(2).expect("nonzero generation"),
            ZoneRevision::new(9),
        ));

        // Newer stored revision: later store commits (including the status
        // write carrying this report) do not invalidate the evidence, so
        // the report stays current while UID and generation match.
        assert!(status.readiness_is_current(
            &uid(),
            ResourceGeneration::new(1).expect("nonzero generation"),
            ZoneRevision::new(10),
        ));

        // Future-dated fence: evidence claiming a commit newer than the
        // stored resource is forged or corrupt, never current.
        assert!(!status.readiness_is_current(
            &uid(),
            ResourceGeneration::new(1).expect("nonzero generation"),
            ZoneRevision::new(8),
        ));

        // Not ready stays not ready under any fence.
        let pending = VolumeBindingStatusResource {
            ready: false,
            fence: fence(),
            reason: Some(StatusCode::parse("view-not-found").expect("valid code")),
        };
        assert!(!pending.readiness_is_current(
            &uid(),
            ResourceGeneration::new(1).expect("nonzero generation"),
            ZoneRevision::new(9),
        ));
    }

    #[test]
    fn status_serialization_exposes_no_paths_sockets_argv_or_numeric_identities() {
        let status = VolumeBindingStatus {
            phase: ResourcePhase::Ready,
            conditions: Vec::new(),
            resource: VolumeBindingStatusResource {
                ready: true,
                fence: fence(),
                reason: None,
            },
        };
        let rendered = serde_json::to_string(&status).expect("status serializes");
        let banned = [
            "mountPath",
            "/nix",
            "socket",
            "Socket",
            "argv",
            "Argv",
            "path",
            "Path",
            "sharedDir",
            "endpoint",
            "worker",
            "Worker",
            // Numeric host identities never appear; the only UID is the
            // binding's own opaque fence evidence below.
            "uid=",
            "gid=",
            "processRef",
        ];
        for entry in banned {
            assert!(
                !rendered.contains(entry),
                "status must not expose {entry}: {rendered}"
            );
        }
        // The fence UID is opaque evidence, not a numeric identity, but it
        // is the only identity-shaped field and must appear exactly once as
        // the fence value.
        let value = serde_json::to_value(&status).expect("status serializes");
        assert_eq!(
            value["resource"]["fence"]["uid"],
            serde_json::json!("123e4567-e89b-42d3-a456-426614174000")
        );
        assert_eq!(value["resource"]["ready"], serde_json::json!(true));
        assert!(value["resource"].get("reason").is_none());

        let parsed: VolumeBindingStatus =
            serde_json::from_value(serde_json::to_value(&status).expect("serializes"))
                .expect("status round trips");
        assert_eq!(parsed, status);
    }

    #[test]
    fn status_rejects_unknown_fields() {
        let mut unknown = serde_json::to_value(VolumeBindingStatus {
            phase: ResourcePhase::Pending,
            conditions: Vec::new(),
            resource: VolumeBindingStatusResource {
                ready: false,
                fence: fence(),
                reason: None,
            },
        })
        .expect("status serializes");
        unknown["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<VolumeBindingStatus>(unknown).is_err());
    }
}
