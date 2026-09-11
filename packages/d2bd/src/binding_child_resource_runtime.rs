//! Core-owned reconciliation for Provider-declared Binding children.
//!
//! Provider controllers return only closed, UID-free child intents. This
//! adapter relists the Resource API, asks Core to materialize and reconcile
//! those intents, then submits exact resource mutations. It never starts or
//! stops a feature process itself.


use d2b_contracts_resource::resource_proto as wire;
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ResourceEnvelope, ResourceRef, ResourceTypeName, ZoneId, canonical_digest,
    volume_binding::{VolumeBindingSpec, VolumeBindingStatusResource},
};
use d2b_core_controller::{
    BindingChildMaterializationError, BindingChildReconciler, HintTarget, OwnedChildIntent,
    OwnerLimits, OwnerMutation, observed_child_from_resource,
};
use d2b_resource_api::{ResourceApiClient, service::UnavailableUpgradeDispatcher};
use d2bd_runtime::resource_runtime_support::ZoneStoreBackend;
use d2b_resource_store::{
    StoreListRequest, StoreOperationContext, StoreProjection, StoredResource,
};
use d2b_resource_store_redb::RedbResourceStore;

const GUEST_CHILD_TYPES: [&str; 5] = [
    "Process",
    "EphemeralProcess",
    "Endpoint",
    "Volume",
    "VolumeBinding",
];
const OWNER_INDEX_MAX_DEPTH: usize = 8;
const OWNER_INDEX_MAX_WORK_ITEMS: usize = 64;

/// One non-semantic Provider owner and its Core-owned child set.
#[derive(Clone)]
pub(crate) struct OwnedChildOwner {
    /// The authoritative parent resource row.
    pub resource: StoredResource,
    /// `None` means the parent is deleting and must have no children.
    pub desired: Option<Vec<OwnedChildIntent>>,
    /// The parent is malformed or has a dangling relationship.
    pub fenced: bool,
}

/// Result of one bounded owner-child reconciliation step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OneOwnedChildProgress {
    /// The complete desired child set is present and current.
    Converged,
    /// One child mutation was submitted; the owner must be re-entered.
    Mutated,
    /// Progress remains blocked on a fresh child observation.
    Pending,
}

/// Reconcile one Guest-owned child mutation, including provider-created
/// Volumes.
pub(crate) async fn reconcile_one_guest_child(
    store: &RedbResourceStore,
    client: &ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>,
    zone: &ZoneId,
    owner: &OwnedChildOwner,
) -> Result<OneOwnedChildProgress, BindingChildRuntimeError> {
    reconcile_one_owned_child_for_types(store, client, zone, owner, &GUEST_CHILD_TYPES).await
}

async fn reconcile_one_owned_child_for_types(
    store: &RedbResourceStore,
    client: &ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>,
    zone: &ZoneId,
    owner: &OwnedChildOwner,
    resource_types: &[&str],
) -> Result<OneOwnedChildProgress, BindingChildRuntimeError> {
    if owner.fenced {
        return Ok(OneOwnedChildProgress::Pending);
    }
    let children = list_children_of_types(store, zone, resource_types).await?;
    validate_child_relist(&children)?;
    let limits = OwnerLimits::new(OWNER_INDEX_MAX_DEPTH, OWNER_INDEX_MAX_WORK_ITEMS)
        .expect("closed owner limits are valid");
    let mut reconciler = BindingChildReconciler::new(limits);
    let owner_target = HintTarget::new(
        owner.resource.zone.clone(),
        owner.resource.resource_ref.clone(),
        owner.resource.uid.clone(),
    );
    let observed = children
        .iter()
        .filter(|child| child_owner_ref(child) == Some(owner.resource.resource_ref.clone()))
        .map(|child| {
            observed_child_from_resource(
                HintTarget::new(
                    child.zone.clone(),
                    child.resource_ref.clone(),
                    child.uid.clone(),
                ),
                &owner_target,
                owner.resource.generation,
                child.revision,
                &child.canonical_json,
                deletion_requested(child),
                deletion_ready(child),
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(BindingChildRuntimeError::Core)?;
    reconciler
        .relist_with_owner_generation(owner_target.clone(), owner.resource.generation, observed)
        .map_err(|error| {
            BindingChildRuntimeError::Core(BindingChildMaterializationError::OwnerReconcile(error))
        })?;
    let plan = match &owner.desired {
        Some(desired) => reconciler
            .plan_owned(&owner_target, desired.iter().cloned())
            .map_err(BindingChildRuntimeError::Core)?,
        None => reconciler
            .plan_owned(&owner_target, std::iter::empty())
            .map_err(BindingChildRuntimeError::Core)?,
    };
    let mutation = first_guest_child_mutation(plan.mutations());
    if let Some(mutation) = mutation {
        apply_mutation(client, &owner.resource, &children, mutation).await?;
        return Ok(OneOwnedChildProgress::Mutated);
    }
    if plan.is_converged() {
        Ok(OneOwnedChildProgress::Converged)
    } else {
        Ok(OneOwnedChildProgress::Pending)
    }
}

fn first_guest_child_mutation(mutations: &[OwnerMutation]) -> Option<&OwnerMutation> {
    mutations
        .iter()
        .find(|mutation| !matches!(mutation, OwnerMutation::Delete { .. }))
}

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
/// Stable failures from the Core-to-Resource-API child adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BindingChildRuntimeError {
    /// The child or owner envelope could not be decoded.
    InvalidResource,
    /// The Core owner planner rejected the child set.
    Core(BindingChildMaterializationError),
    /// A Resource API mutation was refused or malformed.
    Api,
    /// The child relist failed.
    Store,
}

impl core::fmt::Display for BindingChildRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidResource => "binding-child-runtime-resource-invalid",
            Self::Core(error) => return write!(formatter, "binding-child-runtime-core-{error}"),
            Self::Api => "binding-child-runtime-api-failed",
            Self::Store => "binding-child-runtime-store-failed",
        })
    }
}

impl std::error::Error for BindingChildRuntimeError {}

fn validate_child_relist(children: &[StoredResource]) -> Result<(), BindingChildRuntimeError> {
    for child in children {
        let envelope = ResourceEnvelope::from_json(&child.canonical_json)
            .map_err(|_| BindingChildRuntimeError::InvalidResource)?;
        let envelope_ref = ResourceRef::new(
            envelope.resource_type().clone(),
            envelope.metadata().name().clone(),
        );
        if envelope_ref != child.resource_ref
            || envelope.metadata().zone() != &child.zone
            || envelope.metadata().uid() != &child.uid
            || envelope.metadata().generation() != child.generation
            || envelope.metadata().revision() != child.revision
        {
            return Err(BindingChildRuntimeError::InvalidResource);
        }
    }
    Ok(())
}



async fn apply_mutation(
    client: &ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>,
    owner: &StoredResource,
    children: &[StoredResource],
    mutation: &OwnerMutation,
) -> Result<(), BindingChildRuntimeError> {
    match mutation {
        OwnerMutation::Create {
            target,
            canonical_resource,
        } => create_child(client, owner, target, canonical_resource).await,
        OwnerMutation::Repair {
            target,
            expected_uid,
            expected_revision,
            canonical_resource,
        } => {
            let current = children
                .iter()
                .find(|child| {
                    &child.resource_ref == target
                        && &child.uid == expected_uid
                        && child.revision == *expected_revision
                })
                .ok_or(BindingChildRuntimeError::Api)?;
            update_child_spec(
                client,
                current,
                target,
                expected_uid,
                *expected_revision,
                canonical_resource,
            )
            .await
        }

        OwnerMutation::RequestDeletion {
            target,
            expected_uid,
            expected_revision,
        } => delete_child(client, owner, target, expected_uid, *expected_revision).await,
        OwnerMutation::Delete {
            target,
            expected_uid,
            expected_revision,
        } => delete_child(client, owner, target, expected_uid, *expected_revision).await,
    }
}

async fn create_child(
    client: &ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>,
    owner: &StoredResource,
    target: &ResourceRef,
    canonical_resource: &[u8],
) -> Result<(), BindingChildRuntimeError> {
    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_CREATE);
    mutation.target = protobuf::MessageField::some(identity(&owner.zone, target, None, None, None));
    mutation.precondition = protobuf::MessageField::some(create_precondition());
    mutation.resource = protobuf::MessageField::some(resource_body(
        &owner.zone,
        target,
        None,
        canonical_resource,
    )?);
    mutation.owner =
        protobuf::MessageField::some(identity(&owner.zone, &owner.resource_ref, None, None, None));

    let operation = crate::resource_runtime::bounded_operation_id(&format!(
        "binding-child-create-{}-{}",
        owner.resource_ref.to_canonical_string(),
        target.to_canonical_string()
    ));
    let mut request = wire::CreateRequest::new();
    request.meta = protobuf::MessageField::some(request_meta(&operation));
    request.mutation = protobuf::MessageField::some(mutation);
    let response = client.create(request).await;
    if response.error.is_some() {
        return Err(BindingChildRuntimeError::Api);
    }
    Ok(())
}

async fn update_child_spec(
    client: &ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>,
    current: &StoredResource,
    target: &ResourceRef,
    expected_uid: &d2b_contracts_resource::v3::ResourceUid,
    expected_revision: d2b_contracts_resource::v3::ZoneRevision,
    desired_create_resource: &[u8],
) -> Result<(), BindingChildRuntimeError> {
    let canonical = merge_desired_spec(&current.canonical_json, desired_create_resource)?;
    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_UPDATE_SPEC);
    mutation.target = protobuf::MessageField::some(identity(
        &current.zone,
        target,
        Some(expected_uid),
        Some(current.generation.get()),
        Some(expected_revision.get()),
    ));
    mutation.precondition =
        protobuf::MessageField::some(exact_precondition(expected_uid, expected_revision));
    mutation.resource = protobuf::MessageField::some(resource_body(
        &current.zone,
        target,
        Some(expected_uid),
        &canonical,
    )?);

    let operation = crate::resource_runtime::bounded_operation_id(&format!(
        "binding-child-repair-{}-{}-{}",
        target.to_canonical_string(),
        expected_uid.as_str(),
        expected_revision.get()
    ));
    let mut request = wire::UpdateSpecRequest::new();
    request.meta = protobuf::MessageField::some(request_meta(&operation));
    request.mutation = protobuf::MessageField::some(mutation);
    let response = client.update_spec(request).await;
    if response.error.is_some() {
        return Err(BindingChildRuntimeError::Api);
    }
    Ok(())
}

async fn delete_child(
    client: &ResourceApiClient<ZoneStoreBackend, UnavailableUpgradeDispatcher>,
    owner: &StoredResource,
    target: &ResourceRef,
    expected_uid: &d2b_contracts_resource::v3::ResourceUid,
    expected_revision: d2b_contracts_resource::v3::ZoneRevision,
) -> Result<(), BindingChildRuntimeError> {
    let mut mutation = wire::Mutation::new();
    mutation.kind = protobuf::EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_DELETE);
    mutation.target = protobuf::MessageField::some(identity(
        &owner.zone,
        target,
        Some(expected_uid),
        None,
        Some(expected_revision.get()),
    ));
    mutation.precondition =
        protobuf::MessageField::some(exact_precondition(expected_uid, expected_revision));

    let operation = crate::resource_runtime::bounded_operation_id(&format!(
        "binding-child-delete-{}-{}-{}",
        target.to_canonical_string(),
        expected_uid.as_str(),
        expected_revision.get()
    ));
    let mut request = wire::DeleteRequest::new();
    request.meta = protobuf::MessageField::some(request_meta(&operation));
    request.mutation = protobuf::MessageField::some(mutation);
    let response = client.delete(request).await;
    if response.error.is_some() {
        return Err(BindingChildRuntimeError::Api);
    }
    Ok(())
}

fn resource_body(
    zone: &ZoneId,
    target: &ResourceRef,
    uid: Option<&d2b_contracts_resource::v3::ResourceUid>,
    canonical_resource: &[u8],
) -> Result<wire::ResourceEnvelopeBytes, BindingChildRuntimeError> {
    let canonical = CanonicalJsonValue::parse(canonical_resource)
        .map_err(|_| BindingChildRuntimeError::InvalidResource)?
        .to_canonical_bytes();
    let mut body = wire::ResourceEnvelopeBytes::new();
    body.identity = protobuf::MessageField::some(identity(zone, target, uid, None, None));
    body.payload_digest = canonical_digest(
        d2b_contracts_resource::v3::RESOURCE_ENVELOPE_DOMAIN_TAG,
        &canonical,
    );
    body.canonical_json = canonical;
    Ok(body)
}

fn merge_desired_spec(
    current: &[u8],
    desired_create_resource: &[u8],
) -> Result<Vec<u8>, BindingChildRuntimeError> {
    let mut current = CanonicalJsonValue::parse(current)
        .map_err(|_| BindingChildRuntimeError::InvalidResource)?;
    let desired = CanonicalJsonValue::parse(desired_create_resource)
        .map_err(|_| BindingChildRuntimeError::InvalidResource)?;
    let (CanonicalJsonValue::Object(current), CanonicalJsonValue::Object(desired)) =
        (&mut current, desired)
    else {
        return Err(BindingChildRuntimeError::InvalidResource);
    };
    let Some(spec) = desired.get("spec").cloned() else {
        return Err(BindingChildRuntimeError::InvalidResource);
    };
    current.insert("spec".to_owned(), spec);
    let canonical = CanonicalJsonValue::Object(current.clone()).to_canonical_bytes();
    ResourceEnvelope::from_json(&canonical)
        .map_err(|_| BindingChildRuntimeError::InvalidResource)?;
    Ok(canonical)
}

fn identity(
    zone: &ZoneId,
    resource_ref: &ResourceRef,
    uid: Option<&d2b_contracts_resource::v3::ResourceUid>,
    generation: Option<u64>,
    revision: Option<u64>,
) -> wire::ResourceIdentity {
    let mut identity = wire::ResourceIdentity::new();
    identity.zone = zone.to_canonical_string();
    identity.resource_type = resource_ref.resource_type().as_str().to_owned();
    identity.name = resource_ref.name().as_str().to_owned();
    identity.uid = uid.map(|uid| uid.as_str().to_owned());
    identity.generation = generation;
    identity.revision = revision;
    identity
}

fn create_precondition() -> wire::Precondition {
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        protobuf::EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_CREATE_ABSENT);
    precondition
}

fn exact_precondition(
    uid: &d2b_contracts_resource::v3::ResourceUid,
    revision: d2b_contracts_resource::v3::ZoneRevision,
) -> wire::Precondition {
    let mut precondition = wire::Precondition::new();
    precondition.kind =
        protobuf::EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION);
    precondition.expected_uid = Some(uid.as_str().to_owned());
    precondition.expected_revision = Some(revision.get());
    precondition
}

fn request_meta(operation: &str) -> wire::RequestMeta {
    let mut meta = wire::RequestMeta::new();
    meta.operation_id = operation.to_owned();
    meta.idempotency_key = operation.to_owned();
    meta.correlation_id = operation.to_owned();
    meta.trace_id = operation.to_owned();
    meta
}

fn deletion_requested(resource: &StoredResource) -> bool {
    serde_json::from_slice::<serde_json::Value>(&resource.canonical_json)
        .ok()
        .and_then(|value| value.get("metadata").cloned())
        .and_then(|metadata| metadata.get("deletionRequestedAt").cloned())
        .is_some_and(|value| !value.is_null())
}

fn deletion_ready(resource: &StoredResource) -> bool {
    if !deletion_requested(resource) {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&resource.canonical_json) else {
        return false;
    };
    let Some(metadata) = value.get("metadata").and_then(serde_json::Value::as_object) else {
        return false;
    };
    metadata
        .get("finalizers")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|finalizers| finalizers.is_empty())
}

fn child_owner_ref(resource: &StoredResource) -> Option<ResourceRef> {
    ResourceEnvelope::from_json(&resource.canonical_json)
        .ok()
        .and_then(|envelope| envelope.metadata().owner_ref().cloned())
}

async fn list_children_of_types(
    store: &RedbResourceStore,
    zone: &ZoneId,
    resource_types: &[&str],
) -> Result<Vec<StoredResource>, BindingChildRuntimeError> {
    let mut request = StoreListRequest {
        operation: StoreOperationContext {
            operation_id: "binding-child-relist".to_owned(),
            idempotency_key: None,
            correlation_id: "binding-child-relist".to_owned(),
            trace_id: None,
            deadline_ms: 10_000,
        },
        zone: zone.clone(),
        resource_types: resource_types
            .iter()
            .map(|resource_type| {
                ResourceTypeName::parse(*resource_type).expect("closed child type")
            })
            .collect(),
        resource_names: Vec::new(),
        filters: Vec::new(),
        page_size: 256,
        cursor: None,
        projection: StoreProjection::Full,
    };
    let mut resources = Vec::new();
    loop {
        let page = store
            .list(request.clone())
            .await
            .map_err(|_| BindingChildRuntimeError::Store)?;
        resources.extend(page.resources);
        let Some(cursor) = page.next_cursor else {
            break;
        };
        request.cursor = Some(cursor);
    }
    Ok(resources)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(resource_type: &str, name: &str) -> ResourceRef {
        ResourceRef::parse(&format!("{resource_type}/{name}")).expect("resource reference")
    }

    fn stored_resource(
        resource_ref: &ResourceRef,
        owner_ref: Option<&ResourceRef>,
        phase: &str,
    ) -> StoredResource {
        stored_resource_with_spec(resource_ref, owner_ref, phase, serde_json::json!({}))
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

    fn set_deletion_state(
        resource: &mut StoredResource,
        deletion_requested: bool,
        finalizers: &[&str],
    ) {
        let mut value = serde_json::from_slice::<serde_json::Value>(&resource.canonical_json)
            .expect("resource JSON");
        let metadata = value
            .get_mut("metadata")
            .and_then(serde_json::Value::as_object_mut)
            .expect("metadata object");
        metadata.insert(
            "deletionRequestedAt".to_owned(),
            if deletion_requested {
                serde_json::json!("2026-08-19T00:00:00.000Z")
            } else {
                serde_json::Value::Null
            },
        );
        metadata.insert("finalizers".to_owned(), serde_json::json!(finalizers));
        resource.canonical_json =
            CanonicalJsonValue::parse(&serde_json::to_vec(&value).expect("resource serialization"))
                .expect("canonical resource")
                .to_canonical_bytes();
    }

    #[test]
    fn relist_deletion_ready_requires_deletion_request() {
        let child_ref = target("Process", "child");
        let owner = HintTarget::new(
            ZoneId::parse("dev").unwrap(),
            ResourceRef::parse("audio.d2bus.org.AudioBinding/owner").unwrap(),
            d2b_contracts_resource::v3::ResourceUid::parse("223e4567-e89b-42d3-a456-426614174000")
                .unwrap(),
        );
        let observe = |resource: &StoredResource| {
            observed_child_from_resource(
                HintTarget::new(
                    resource.zone.clone(),
                    resource.resource_ref.clone(),
                    resource.uid.clone(),
                ),
                &owner,
                d2b_contracts_resource::v3::ResourceGeneration::new(1).unwrap(),
                resource.revision,
                &resource.canonical_json,
                deletion_requested(resource),
                deletion_ready(resource),
            )
        };

        let live = stored_resource(&child_ref, Some(owner.resource_ref()), "Ready");
        let observed = observe(&live).expect("live child relists");
        assert!(!observed.deletion_requested());
        assert!(!observed.deletion_ready());

        let mut requested = stored_resource(&child_ref, Some(owner.resource_ref()), "Ready");
        set_deletion_state(&mut requested, true, &[]);
        let observed = observe(&requested).expect("requested child relists");
        assert!(observed.deletion_requested());
        assert!(observed.deletion_ready());

        let mut finalizing = stored_resource(&child_ref, Some(owner.resource_ref()), "Ready");
        set_deletion_state(&mut finalizing, true, &["child-finalizer"]);
        let observed = observe(&finalizing).expect("finalizing child relists");
        assert!(observed.deletion_requested());
        assert!(!observed.deletion_ready());
    }

    #[test]
    fn guest_child_progress_never_issues_a_second_delete() {
        let child_uid = d2b_contracts_resource::v3::ResourceUid::parse(
            "123e4567-e89b-42d3-a456-426614174000",
        )
        .unwrap();
        let child_revision = d2b_contracts_resource::v3::ZoneRevision::new(2);
        let requested = OwnerMutation::Delete {
            target: target("Process", "child"),
            expected_uid: child_uid.clone(),
            expected_revision: child_revision,
        };
        assert!(first_guest_child_mutation(&[requested]).is_none());

        let request = OwnerMutation::RequestDeletion {
            target: target("Process", "child"),
            expected_uid: child_uid,
            expected_revision: child_revision,
        };
        assert!(matches!(
            first_guest_child_mutation(&[request]),
            Some(OwnerMutation::RequestDeletion { .. })
        ));
    }

    #[test]
    fn malformed_child_relist_fails_closed() {
        let child_ref = target("Process", "child");
        let mut child = stored_resource(&child_ref, None, "Ready");
        child.canonical_json = b"{}".to_vec();
        assert_eq!(
            validate_child_relist(&[child]),
            Err(BindingChildRuntimeError::InvalidResource)
        );
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
