//! Read-side helpers for Core-owned Binding children.
//!
//! The Guest family's child reconciliation lives in the per-type drivers
//! (`guest_driver` / `guest_effects`), which read the manager view directly;
//! this module keeps only the two pure readers over a stored VolumeBinding
//! row: the fenced readiness projection and the typed spec parse.

use d2b_contracts_resource::v3::volume_binding::{VolumeBindingSpec, VolumeBindingStatusResource};
use d2b_resource_store::StoredResource;

/// Whether one stored VolumeBinding carries a current fenced readiness
/// projection.  Unparseable or unfenced projections fail closed.
pub(crate) fn binding_readiness_current(child: &StoredResource) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&child.canonical_json) else {
        return false;
    };
    let Some(resource) = value
        .pointer("/status/resource")
        .cloned()
        .map(|resource| serde_json::from_value::<VolumeBindingStatusResource>(resource))
        .transpose()
        .ok()
        .flatten()
    else {
        return false;
    };
    resource.readiness_is_current(&child.uid, child.generation, child.revision)
}

/// Parse a stored binding envelope to its typed spec, stripping the
/// reserved envelope fields the minter stores alongside the five typed
/// ones.  Returns None for genuinely broken resources.
pub(crate) fn parsed_binding_spec(binding: &StoredResource) -> Option<VolumeBindingSpec> {
    let mut spec = serde_json::from_slice::<serde_json::Value>(&binding.canonical_json)
        .ok()?
        .get("spec")?
        .clone();
    let object = spec.as_object_mut()?;
    for field in ["providerRef", "updatePolicy", "provider"] {
        object.remove(field);
    }
    serde_json::from_value::<VolumeBindingSpec>(serde_json::Value::Object(object.clone())).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::{CanonicalJsonValue, ResourceRef, ZoneId};

    fn target(resource_type: &str, name: &str) -> ResourceRef {
        ResourceRef::parse(&format!("{resource_type}/{name}")).expect("resource reference")
    }

    fn stored_resource_with_spec(
        resource_ref: &ResourceRef,
        owner_ref: Option<&ResourceRef>,
        phase: &str,
        spec: serde_json::Value,
    ) -> StoredResource {
        let zone = ZoneId::parse("dev").expect("zone");
        let value = serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": resource_ref.resource_type().as_str(),
            "metadata": {
                "name": resource_ref.name().as_str(),
                "zone": zone.as_str(),
                "ownerRef": owner_ref.map(ResourceRef::to_canonical_string),
                "labels": {},
                "annotations": {},
                "finalizers": [],
                "managedBy": "controller",
                "deletionRequestedAt": null,
                "createdAt": "2026-08-19T00:00:00.000Z",
                "updatedAt": "2026-08-19T00:00:00.000Z",
                "generation": 1,
                "revision": 1,
                "uid": "123e4567-e89b-42d3-a456-426614174000"
            },
            "spec": spec,
            "status": {
                "observedGeneration": 0,
                "phase": phase,
                "conditions": [],
                "lastReconciledAt": null,
                "startedAt": null,
                "completedAt": null,
                "outcome": null,
                "update": {
                    "dependencies": {"count": 0, "refs": []},
                    "disruption": "None",
                    "lastAssessedAt": null,
                    "observedGeneration": 0,
                    "operationId": null,
                    "owned": {"count": 0, "refs": []},
                    "preserveState": true,
                    "reasons": [],
                    "state": "Unknown",
                    "targetGeneration": 1
                },
                "resource": {}
            }
        });
        let canonical =
            CanonicalJsonValue::parse(&serde_json::to_vec(&value).expect("resource serialization"))
                .expect("canonical resource")
                .to_canonical_bytes();
        StoredResource {
            resource_ref: resource_ref.clone(),
            zone,
            uid: d2b_contracts_resource::v3::ResourceUid::parse(
                "123e4567-e89b-42d3-a456-426614174000",
            )
            .expect("uid"),
            owner_uid: None,
            owner_generation: None,
            generation: d2b_contracts_resource::v3::ResourceGeneration::new(1).expect("generation"),
            revision: d2b_contracts_resource::v3::ZoneRevision::new(1),
            canonical_json: canonical,
            payload_digest: "sha256:test".to_owned(),
        }
    }

    fn binding_child_with_fence(
        binding_ref: &ResourceRef,
        owner_ref: &ResourceRef,
        fence: serde_json::Value,
    ) -> StoredResource {
        let mut binding = stored_resource_with_spec(
            binding_ref,
            Some(owner_ref),
            "Ready",
            serde_json::json!({"volumeRef": "Volume/volume"}),
        );
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&binding.canonical_json).unwrap();
        value["status"]["resource"] = serde_json::json!({
            "ready": true,
            "fence": fence,
        });
        binding.canonical_json = CanonicalJsonValue::parse(
            &serde_json::to_vec(&value).expect("binding serialization"),
        )
        .expect("canonical binding")
        .to_canonical_bytes();
        binding
    }

    #[test]
    fn binding_child_readiness_requires_current_fenced_projection() {
        let owner_ref = target("Volume", "volume");
        let binding_ref = target("VolumeBinding", "binding");
        let current_uid = "123e4567-e89b-42d3-a456-426614174000";

        // Phase Ready alone never satisfies a binding child: the typed
        // fenced projection is required (KTD3).
        let unfenced = stored_resource_with_spec(
            &binding_ref,
            Some(&owner_ref),
            "Ready",
            serde_json::json!({"volumeRef": "Volume/volume"}),
        );
        assert!(!binding_readiness_current(&unfenced));

        // A fence under another binding UID is stale.
        let foreign_uid = binding_child_with_fence(
            &binding_ref,
            &owner_ref,
            serde_json::json!({
                "uid": "223e4567-e89b-42d3-a456-426614174000",
                "generation": 1,
                "revision": 1
            }),
        );
        assert!(!binding_readiness_current(&foreign_uid));

        // A fence under an older generation is stale: the stored binding
        // is at generation 1, the projection reports generation 2.
        let stale_generation = binding_child_with_fence(
            &binding_ref,
            &owner_ref,
            serde_json::json!({
                "uid": current_uid,
                "generation": 2,
                "revision": 1
            }),
        );
        assert!(!binding_readiness_current(&stale_generation));

        // A fence matching the binding's UID, generation, and revision is
        // current.
        let current = binding_child_with_fence(
            &binding_ref,
            &owner_ref,
            serde_json::json!({
                "uid": current_uid,
                "generation": 1,
                "revision": 1
            }),
        );
        assert!(binding_readiness_current(&current));
    }

    #[test]
    fn parsed_binding_spec_strips_reserved_fields_and_rejects_broken_specs() {
        let guest_ref = target("Guest", "work-vm");
        let other_guest_ref = target("Guest", "other-vm");
        let binding_spec = |execution_ref: &str| {
            serde_json::json!({
                "volumeRef": "Volume/work-state",
                "executionRef": execution_ref,
                "view": "controller",
                "access": "read-only",
                "mountPath": "/state",
            })
        };
        let own = stored_resource_with_spec(
            &target("VolumeBinding", "own"),
            Some(&target("Volume", "work-state")),
            "Pending",
            binding_spec(guest_ref.to_canonical_string().as_str()),
        );
        assert_eq!(
            parsed_binding_spec(&own)
                .expect("clean spec parses")
                .execution_ref(),
            &guest_ref
        );
        // Minted records carry the reserved envelope providerRef alongside
        // the five typed fields; the parse must attribute them instead of
        // failing closed.
        let minted_spec = |execution_ref: &str| {
            serde_json::json!({
                "volumeRef": "Volume/work-state",
                "executionRef": execution_ref,
                "view": "controller",
                "access": "read-only",
                "mountPath": "/state",
                "providerRef": "Provider/volume-virtiofs",
            })
        };
        let own_minted = stored_resource_with_spec(
            &target("VolumeBinding", "own-minted"),
            Some(&target("Volume", "work-state")),
            "Pending",
            minted_spec(guest_ref.to_canonical_string().as_str()),
        );
        assert_eq!(
            parsed_binding_spec(&own_minted)
                .expect("minted spec parses")
                .execution_ref(),
            &guest_ref
        );
        let foreign_minted = stored_resource_with_spec(
            &target("VolumeBinding", "foreign-minted"),
            Some(&target("Volume", "work-state")),
            "Pending",
            minted_spec(other_guest_ref.to_canonical_string().as_str()),
        );
        assert_eq!(
            parsed_binding_spec(&foreign_minted)
                .expect("foreign spec parses")
                .execution_ref(),
            &other_guest_ref
        );
        // An unparseable spec is a broken resource.
        let broken = stored_resource_with_spec(
            &target("VolumeBinding", "broken"),
            Some(&target("Volume", "work-state")),
            "Pending",
            serde_json::json!({"volumeRef": "Volume/work-state"}),
        );
        assert!(parsed_binding_spec(&broken).is_none());
    }
}
