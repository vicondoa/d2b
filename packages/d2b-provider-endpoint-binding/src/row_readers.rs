//! Read-side helpers over a stored `EndpointBinding` row.
//!
//! Every consumer of a binding row - the consumer provider deciding whether
//! a binding admits its request, the plane's admission check reading what a
//! binding declares, and this family's own driver tests - reads it through
//! the two pure readers here: the fenced readiness projection the binding
//! actor publishes, and the typed request the row's base spec carries.
//! Neither touches a store, and both fail closed on a row they cannot read.

use d2b_contracts_resource::v3::StoredResource;
use d2b_contracts_resource::v3::endpoint_binding::EndpointBindingSpec;

use crate::driver::{
    EndpointBindingStatusResource, committed_source_selector,
};

/// Whether one stored `EndpointBinding` carries a current fenced readiness
/// projection.  Unparseable or unfenced projections fail closed.
pub fn binding_readiness_current(child: &StoredResource) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&child.canonical_json) else {
        return false;
    };
    let Some(resource) = value
        .pointer("/status/resource")
        .cloned()
        .map(serde_json::from_value::<EndpointBindingStatusResource>)
        .transpose()
        .ok()
        .flatten()
    else {
        return false;
    };
    resource.readiness_is_current(&child.uid, child.generation, child.revision)
}

/// Parse a stored binding row to its typed row spec, stripping the reserved
/// envelope fields the store carries alongside the four typed ones. Returns
/// `None` for genuinely broken resources.
///
/// This is the committed ROW, read back out of its own bytes. The canonical
/// source-side request is the consumer's own declaration rather than
/// something a reader can reconstruct here: a binding row carries no
/// purpose, because the purpose the delivery rides is the endpoint row's.
pub fn parsed_binding_spec(binding: &StoredResource) -> Option<EndpointBindingSpec> {
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
    serde_json::from_value::<EndpointBindingSpec>(spec).ok()
}

/// Whether one stored binding names exactly one endpoint through the
/// exact-endpoint rule the driver applies at validate.
///
/// A read-side caller that wants to know whether a row can name a
/// neighbourhood - before the driver has seen it - asks here, and a row whose
/// selector is a wildcard, an alternation, a cross-Zone target, or the wrong
/// source type answers `false` rather than being resolved as best it can.
pub fn names_one_exact_endpoint(binding: &StoredResource) -> bool {
    let Some(base) = serde_json::from_slice::<serde_json::Value>(&binding.canonical_json)
        .ok()
        .and_then(|value| value.get("spec").cloned())
    else {
        return false;
    };
    committed_source_selector(&base).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    use d2b_contracts_resource::v3::{
        BindingArbitration, BindingRealizationFacet, BindingSourceDecision, CanonicalJsonValue,
        EndpointAttachmentKind, RequestedRights, ResourceGeneration, ResourceRef, ResourceUid,
        ZoneId, ZoneRevision, execution_policy::BoundedToken,
    };

    fn target(resource_type: &str, name: &str) -> ResourceRef {
        ResourceRef::parse(&format!("{resource_type}/{name}")).expect("resource reference")
    }

    fn stored_binding_with_spec(spec: serde_json::Value) -> StoredResource {
        let resource_ref = target("EndpointBinding", "binding");
        let zone = ZoneId::parse("dev").expect("zone");
        let value = serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": resource_ref.resource_type().as_str(),
            "metadata": {
                "name": resource_ref.name().as_str(),
                "zone": zone.as_str(),
                "ownerRef": null,
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
                "phase": "Ready",
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
        let canonical = CanonicalJsonValue::parse(
            &serde_json::to_vec(&value).expect("resource serialization"),
        )
        .expect("canonical resource")
        .to_canonical_bytes();
        StoredResource {
            resource_ref,
            zone,
            uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("uid"),
            owner_uid: None,
            owner_generation: None,
            generation: ResourceGeneration::new(1).expect("generation"),
            revision: ZoneRevision::new(1),
            canonical_json: canonical,
            payload_digest: d2b_contracts_resource::v3::StateDigest::parse(format!(
                "sha256:{}",
                "0".repeat(64)
            ))
            .expect("a zero digest is a valid state digest"),
        }
    }

    fn row_spec() -> serde_json::Value {
        serde_json::json!({
            "endpointRef": "Endpoint/display",
            "executionRef": "Process/consumer",
            "slot": "primary",
            "attachment": "connect",
            "source": {
                "admittedRights": ["consume"],
                "arbitration": "shared",
                "realizedFacets": ["endpoint-descriptor"],
            },
        })
    }

    fn binding_with_fence(fence: serde_json::Value, ready: bool) -> StoredResource {
        let mut binding = stored_binding_with_spec(row_spec());
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&binding.canonical_json).expect("row");
        value["status"]["resource"] = serde_json::json!({"ready": ready, "fence": fence});
        binding.canonical_json = CanonicalJsonValue::parse(
            &serde_json::to_vec(&value).expect("binding serialization"),
        )
        .expect("canonical binding")
        .to_canonical_bytes();
        binding
    }

    #[test]
    fn binding_readiness_requires_a_current_fenced_projection() {
        let uid = "123e4567-e89b-42d3-a456-426614174000";
        let current = binding_with_fence(
            serde_json::json!({"uid": uid, "generation": 1, "revision": 1}),
            true,
        );
        assert!(binding_readiness_current(&current));

        // A not-ready projection never reports the row ready, whatever its
        // fence says.
        let not_ready = binding_with_fence(
            serde_json::json!({"uid": uid, "generation": 1, "revision": 1}),
            false,
        );
        assert!(!binding_readiness_current(&not_ready));

        // A fence under another UID is stale.
        let foreign = binding_with_fence(
            serde_json::json!({
                "uid": "223e4567-e89b-42d3-a456-426614174000",
                "generation": 1,
                "revision": 1
            }),
            true,
        );
        assert!(!binding_readiness_current(&foreign));

        // A fence under an older generation is stale: the stored row is at
        // generation 1 and the projection reports generation 2.
        let stale_generation = binding_with_fence(
            serde_json::json!({"uid": uid, "generation": 2, "revision": 1}),
            true,
        );
        assert!(!binding_readiness_current(&stale_generation));

        // A fence ahead of the stored revision is not current: the revision
        // is a currency bound, not a free field.
        let ahead = binding_with_fence(
            serde_json::json!({"uid": uid, "generation": 1, "revision": 9}),
            true,
        );
        assert!(!binding_readiness_current(&ahead));

        // A row with no projection at all fails closed.
        let unfenced = stored_binding_with_spec(row_spec());
        assert!(!binding_readiness_current(&unfenced));
    }

    #[test]
    fn parsed_binding_spec_strips_reserved_fields_and_rejects_broken_specs() {
        let own = stored_binding_with_spec(row_spec());
        let parsed = parsed_binding_spec(&own).expect("the canonical row parses");
        assert_eq!(parsed.endpoint_ref(), &target("Endpoint", "display"));
        assert_eq!(parsed.execution_ref(), &target("Process", "consumer"));
        assert_eq!(parsed.slot().as_str(), "primary");
        assert_eq!(parsed.attachment(), &EndpointAttachmentKind::Connect);

        // A minted record carries the reserved envelope `providerRef`
        // alongside the five typed fields; the parse attributes it instead
        // of failing closed.
        let mut minted_spec = row_spec();
        minted_spec["providerRef"] = serde_json::json!("Provider/endpoint");
        let minted = stored_binding_with_spec(minted_spec);
        assert_eq!(
            parsed_binding_spec(&minted)
                .expect("the minted spec parses")
                .endpoint_ref(),
            &target("Endpoint", "display")
        );

        // A row missing the source is genuinely broken.
        let broken = stored_binding_with_spec(serde_json::json!({
            "executionRef": "Process/consumer",
            "slot": "primary",
            "attachment": "connect",
            "source": {
                "admittedRights": ["consume"],
                "arbitration": "shared",
                "realizedFacets": ["endpoint-descriptor"],
            },
        }));
        assert!(parsed_binding_spec(&broken).is_none());
    }

    #[test]
    fn names_one_exact_endpoint_refuses_a_selector_that_could_name_more() {
        assert!(names_one_exact_endpoint(&stored_binding_with_spec(row_spec())));

        for (endpoint_ref, why) in [
            ("Endpoint/*", "a wildcard"),
            ("Endpoint/display?", "a single-character wildcard"),
            ("Endpoint/display,Endpoint/other", "an alternation"),
            ("other-zone:Endpoint/display", "a cross-Zone target"),
            ("Endpoint/shared/display", "a second separator"),
            ("Device/display", "the wrong source type"),
        ] {
            let mut spec = row_spec();
            spec["endpointRef"] = serde_json::json!(endpoint_ref);
            let row = stored_binding_with_spec(spec);
            assert!(
                !names_one_exact_endpoint(&row),
                "{why} ({endpoint_ref}) must not read as one exact endpoint"
            );
        }
    }

    #[test]
    fn the_row_contract_admits_a_process_consumer_and_refuses_a_host() {
        // The row contract's own rule, exercised here so a change to it is
        // visible in this crate's suite: the consumer set is derived from the
        // binding kind, not asserted as a Guest bound.
        let row = |consumer: ResourceRef| {
            EndpointBindingSpec::new(
                target("Endpoint", "display"),
                consumer,
                EndpointAttachmentKind::Attach,
                BoundedToken::parse("primary".to_owned()).expect("slot"),
                BindingSourceDecision::new(
                    vec![RequestedRights::Consume],
                    BindingArbitration::Shared,
                    vec![
                        BindingRealizationFacet::EndpointDescriptor,
                        BindingRealizationFacet::EndpointPathname,
                    ],
                )
                .expect("source decision"),
            )
        };
        assert!(
            row(target("Process", "consumer")).is_ok(),
            "a helper Process is an admitted consumer of an endpoint"
        );
        assert!(
            row(target("EphemeralProcess", "worker")).is_ok(),
            "so is an ephemeral helper"
        );
        assert!(
            row(target("Guest", "work-vm")).is_ok(),
            "so is a Guest"
        );
        assert!(
            row(target("Host", "host-system")).is_err(),
            "a Host is never the consumer of an endpoint binding"
        );
        // A source that is not an Endpoint is refused by the same rule.
        assert!(
            EndpointBindingSpec::new(
                target("Device", "gpu"),
                target("Process", "consumer"),
                EndpointAttachmentKind::Connect,
                BoundedToken::parse("primary".to_owned()).expect("slot"),
                BindingSourceDecision::new(
                    vec![RequestedRights::Consume],
                    BindingArbitration::Shared,
                    vec![BindingRealizationFacet::EndpointDescriptor],
                )
                .expect("source decision"),
            )
            .is_err()
        );
    }
}
