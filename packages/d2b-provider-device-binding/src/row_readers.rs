//! Read-side helpers over a committed `DeviceBinding` row.
//!
//! Every reader of a binding row - the consumer deciding whether its device
//! capability is realized, a reader asking whether the row is still current,
//! and this family's own driver tests - reads it through the two pure
//! helpers here: the fenced readiness projection the binding actor publishes,
//! and the committed request the row carries. Neither touches a store, and
//! both fail closed on a row they cannot read.
//!
//! The projection is this crate's own wire contract: `DeviceBinding` has no
//! projection in the shared resource contracts, and a binding's readiness is
//! the one thing a consumer reads off the row, so the shape is declared here
//! and pinned by the round-trip test at the foot of this module. It carries
//! readiness, the fence the evidence was observed under, and a stable
//! provider reason - never the device node path, the physical authority key,
//! the mediation adapter's own state, or any host path.

use d2b_contracts_resource::v3::StoredResource;
use d2b_contracts_resource::v3::device_binding::DeviceBindingSpec;
use d2b_contracts_resource::v3::resource_status::StatusCode;
use d2b_contracts_resource::v3::{ResourceGeneration, ResourceUid, ZoneRevision};

use crate::facets::DeviceBindingFence;

/// The fenced readiness one `DeviceBinding` row publishes.
///
/// The wire shape is `{"ready": bool, "fence": {"uid", "generation",
/// "revision"}, "reason": <code or null>}`. Readiness is only current under a
/// fence that matches the row's own uid and generation, so a
/// deleted-and-recreated row, a superseded spec, or a report pinned ahead of
/// the stored revision can never report a capability as ready.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingReadiness {
    ready: bool,
    fence: DeviceBindingFence,
    reason: Option<StatusCode>,
}

impl BindingReadiness {
    /// Build one fenced readiness report.
    ///
    /// A reason that is not a canonical status code is dropped rather than
    /// reported verbatim: a projection that cannot name its own reason
    /// answers `false` with no reason, never a fabricated one.
    pub fn new(ready: bool, fence: DeviceBindingFence, reason: Option<&str>) -> Self {
        Self {
            ready,
            fence,
            reason: reason.and_then(|code| StatusCode::parse(code).ok()),
        }
    }

    /// Whether the row's realization was observed serving.
    pub const fn ready(&self) -> bool {
        self.ready
    }

    /// The stable provider reason, when the row is not ready.
    pub const fn reason(&self) -> Option<&StatusCode> {
        self.reason.as_ref()
    }

    /// The fence this report was observed under.
    pub const fn fence(&self) -> &DeviceBindingFence {
        &self.fence
    }

    /// Render the wire projection this actor publishes.
    pub fn to_projection(&self) -> serde_json::Value {
        serde_json::json!({
            "ready": self.ready,
            "fence": {
                "uid": self.fence.uid().to_canonical_string(),
                "generation": self.fence.generation().get(),
                "revision": self.fence.revision().get(),
            },
            "reason": self.reason.as_ref().map(StatusCode::as_str),
        })
    }

    /// Read one projection back, failing closed on anything that is not
    /// exactly this shape.
    pub fn from_projection(value: &serde_json::Value) -> Option<Self> {
        let ready = value.get("ready")?.as_bool()?;
        let fence = value.get("fence")?;
        let uid = ResourceUid::parse(fence.get("uid")?.as_str()?).ok()?;
        let generation = ResourceGeneration::new(fence.get("generation")?.as_u64()?).ok()?;
        let revision = ZoneRevision::new(fence.get("revision")?.as_u64()?);
        let reason = match value.get("reason") {
            None | Some(serde_json::Value::Null) => None,
            Some(code) => Some(StatusCode::parse(code.as_str()?).ok()?),
        };
        Some(Self {
            ready,
            fence: DeviceBindingFence::new(uid, generation, revision),
            reason,
        })
    }

    /// Whether this report is ready under the row's current identity.
    pub fn is_current(
        &self,
        uid: &ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
    ) -> bool {
        self.ready && self.fence.matches(uid, generation, revision)
    }
}

/// Whether one stored `DeviceBinding` carries a current fenced readiness
/// projection.  Unparseable, unfenced, or stale projections fail closed.
pub fn binding_readiness_current(binding: &StoredResource) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&binding.canonical_json) else {
        return false;
    };
    value
        .pointer("/status/resource")
        .and_then(BindingReadiness::from_projection)
        .is_some_and(|readiness| {
            readiness.is_current(&binding.uid, binding.generation, binding.revision)
        })
}

/// Parse a committed `DeviceBinding` row back as the canonical device
/// binding spec.
///
/// The row's base spec *is* the graph's own [`DeviceBindingSpec`], so this is
/// the same declaration read back out of committed bytes rather than a
/// translated copy of it. The reserved envelope fields the spec store carries
/// beside the five typed ones are stripped first; a row whose remaining shape
/// is not the canonical spec returns `None` rather than guessing a
/// relationship out of one.
pub fn parsed_binding_spec(binding: &StoredResource) -> Option<DeviceBindingSpec> {
    let mut spec = serde_json::from_slice::<serde_json::Value>(&binding.canonical_json)
        .ok()?
        .get("spec")?
        .clone();
    {
        let object = spec.as_object_mut()?;
        for field in ["providerRef", "updatePolicy", "provider"] {
            object.remove(field);
        }
    }
    serde_json::from_value::<DeviceBindingSpec>(spec).ok()
}

#[cfg(test)]
mod tests {
    use d2b_contracts_resource::v3::{
        ResourceName, ResourceRef, ResourceTypeName, StateDigest, ZoneId, device_binding::DeviceClaimRequest,
    };

    use super::*;

    fn fence() -> DeviceBindingFence {
        DeviceBindingFence::new(
            ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("uid"),
            ResourceGeneration::new(3).expect("generation"),
            ZoneRevision::new(3),
        )
    }

    fn row(canonical_json: &str) -> StoredResource {
        StoredResource {
            resource_ref: ResourceRef::new(
                ResourceTypeName::parse("DeviceBinding").expect("type"),
                ResourceName::parse("dev-binding-0001").expect("name"),
            ),
            zone: ZoneId::parse("work").expect("zone"),
            uid: ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("uid"),
            owner_uid: None,
            owner_generation: None,
            generation: ResourceGeneration::new(3).expect("generation"),
            revision: ZoneRevision::new(3),
            canonical_json: canonical_json.as_bytes().to_vec(),
            payload_digest: StateDigest::parse(format!("sha256:{}", "0".repeat(64)))
                .expect("digest"),
        }
    }

    /// The projection round-trips: what the actor publishes is exactly what a
    /// reader gets back, with the reason carried only when the row is not
    /// ready.
    #[test]
    fn the_projection_round_trips_through_its_wire_shape() {
        let pending = BindingReadiness::new(false, fence(), Some("device-attachment-not-ready"));
        let projection = pending.to_projection();
        assert_eq!(
            projection["fence"]["generation"].as_u64(),
            Some(3),
            "the fence names the row generation"
        );
        assert_eq!(
            projection["reason"].as_str(),
            Some("device-attachment-not-ready")
        );
        assert_eq!(
            BindingReadiness::from_projection(&projection).as_ref(),
            Some(&pending),
            "the published projection reads back as the report that produced it"
        );

        let ready = BindingReadiness::new(true, fence(), None);
        let projection = ready.to_projection();
        assert!(
            projection.get("reason").is_some_and(serde_json::Value::is_null),
            "a ready report carries no reason rather than inventing one"
        );
        assert_eq!(BindingReadiness::from_projection(&projection), Some(ready));
    }

    /// A projection that is not exactly this shape is never read as ready: a
    /// missing readiness bit, a missing fence, an unfenceable revision, and a
    /// reason that is not a canonical code all fail closed.
    #[test]
    fn an_unreadable_projection_fails_closed() {
        assert!(BindingReadiness::from_projection(&serde_json::json!({})).is_none());
        assert!(
            BindingReadiness::from_projection(&serde_json::json!({
                "ready": true,
                "fence": { "uid": "11111111-1111-4111-8111-111111111111", "generation": 3 },
            }))
            .is_none(),
            "a fence with no revision is not a fence"
        );
        assert!(
            BindingReadiness::from_projection(&serde_json::json!({
                "ready": true,
                "fence": {
                    "uid": "11111111-1111-4111-8111-111111111111",
                    "generation": 0,
                    "revision": 3,
                },
            }))
            .is_none(),
            "generation zero is not a committed row generation"
        );
        let dropped = BindingReadiness::new(
            true,
            fence(),
            Some("Not A Canonical Status Code"),
        );
        assert_eq!(
            dropped.reason(),
            None,
            "an unparseable reason is dropped, never reported verbatim"
        );
    }

    /// The fence is a currency bound, not a free field: a report under
    /// another identity, a superseded spec, or a revision ahead of the stored
    /// one never reports the row ready.
    #[test]
    fn the_fence_pins_reassignment_spec_change_and_revision() {
        let ready = BindingReadiness::new(true, fence(), None);
        let uid = ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("uid");
        let generation = ResourceGeneration::new(3).expect("generation");
        let revision = ZoneRevision::new(3);
        assert!(ready.is_current(&uid, generation, revision));
        assert!(
            !ready.is_current(
                &uid,
                ResourceGeneration::new(4).expect("generation"),
                revision
            ),
            "a superseded spec never reports ready"
        );
        assert!(
            !ready.is_current(
                &ResourceUid::parse("22222222-2222-4222-8222-222222222222").expect("uid"),
                generation,
                revision
            ),
            "a deleted-and-recreated row is a different relationship"
        );
        assert!(
            !ready.is_current(&uid, generation, ZoneRevision::new(2)),
            "a fence ahead of the stored revision is not current"
        );
        assert!(
            !BindingReadiness::new(false, fence(), None).is_current(&uid, generation, revision),
            "a not-ready report is never current"
        );
    }

    /// The stored-row reader: readiness is read through the fence, and the
    /// committed request is read back as the canonical declaration with the
    /// reserved envelope fields stripped.
    #[test]
    fn a_stored_row_reads_back_its_request_and_its_fenced_readiness() {
        let readiness = BindingReadiness::new(true, fence(), None);
        let canonical = serde_json::json!({
            "spec": {
                "providerRef": "Provider/device-gpu",
                "deviceRef": "Device/gpu0",
                "executionRef": "Process/render",
                "slot": "gpu0",
                "function": "render",
                "claim": "shared",
                "source": {
                    "admittedRights": ["share"],
                    "arbitration": "shared",
                    "realizedFacets": ["device-attachment"],
                },
            },
            "status": { "resource": readiness.to_projection() },
        })
        .to_string();
        let stored = row(&canonical);
        assert!(binding_readiness_current(&stored));
        let spec = parsed_binding_spec(&stored).expect("the committed spec reads back");
        assert_eq!(
            spec.device_ref(),
            &ResourceRef::parse("Device/gpu0").expect("ref")
        );
        assert_eq!(spec.slot().as_str(), "gpu0");
        assert_eq!(
            *spec.claim(),
            DeviceClaimRequest::Shared,
            "the committed claim mode reads back exactly"
        );

        // The same report under an earlier stored revision is stale, the same
        // report under a foreign uid is a different relationship, and a row
        // whose status layer is not this projection is not ready either.
        let mut ahead = stored.clone();
        ahead.revision = ZoneRevision::new(2);
        assert!(!binding_readiness_current(&ahead));
        let mut foreign = stored.clone();
        foreign.uid = ResourceUid::parse("33333333-3333-4333-8333-333333333333").expect("uid");
        assert!(!binding_readiness_current(&foreign));
        assert!(!binding_readiness_current(&row("{\"spec\":{}}")));
        assert!(
            parsed_binding_spec(&row("{\"spec\":{\"deviceRef\":\"Device/gpu0\"}}")).is_none(),
            "a partial spec is not a device binding row"
        );
    }
}