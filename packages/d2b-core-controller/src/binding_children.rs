//! Core-owned validation and materialization of semantic Binding children.
//!
//! Providers describe child intent and supply the signed desired resource
//! bodies. Core validates each body against its intent, materializes the
//! UID-free create payloads, and adapts observed Resource API envelopes into
//! owner-index rows.

use std::collections::BTreeMap;

use d2b_contracts_provider::v3::semantic_services::child_resources::{
    BindingChildIntent, BindingChildKind, BindingChildPlacement,
};
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, RESOURCE_ENVELOPE_DOMAIN_TAG, ResourceRef, ResourceTypeName, ResourceUid,
    ZoneRevision, canonical_digest,
};
use d2b_contracts_zone_session::v3::resource_bundle::BundleResource;

use crate::{ObservedChild, OwnerReconcileError, ResourceKey};

/// One provider-supplied desired child body paired with its semantic intent.
#[derive(Clone, PartialEq, Eq)]
pub struct BindingChildResource {
    intent: BindingChildIntent,
    canonical_resource: Vec<u8>,
}

impl core::fmt::Debug for BindingChildResource {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BindingChildResource")
            .field("resource_type", &self.intent.resource_ref().resource_type())
            .field("has_canonical_resource", &true)
            .finish()
    }
}

impl BindingChildResource {
    /// Validate and pair one signed desired child body with its intent.
    pub fn new(
        intent: BindingChildIntent,
        canonical_resource: Vec<u8>,
    ) -> Result<Self, BindingChildMaterializationError> {
        if canonical_resource.is_empty() {
            return Err(BindingChildMaterializationError::EmptyResource);
        }
        let resource: BundleResource = serde_json::from_slice(&canonical_resource)
            .map_err(|_| BindingChildMaterializationError::MalformedResource)?;
        let expected_type = ResourceTypeName::parse(intent.kind().resource_type())
            .expect("closed child ResourceType is valid");
        let actual_ref = ResourceRef::new(
            resource.resource_type().clone(),
            resource.metadata().name().clone(),
        );
        if resource.resource_type() != &expected_type || &actual_ref != intent.resource_ref() {
            return Err(BindingChildMaterializationError::IdentityMismatch);
        }
        if resource.metadata().owner_ref() != Some(intent.owner_ref()) {
            return Err(BindingChildMaterializationError::OwnerMismatch);
        }
        validate_spec(&intent, resource.spec())?;
        let canonical = CanonicalJsonValue::parse(&canonical_resource)
            .map_err(|_| BindingChildMaterializationError::NonCanonicalResource)?
            .to_canonical_bytes();
        if canonical != canonical_resource {
            return Err(BindingChildMaterializationError::NonCanonicalResource);
        }
        Ok(Self {
            intent,
            canonical_resource,
        })
    }

    /// Borrow the validated child intent.
    pub const fn intent(&self) -> &BindingChildIntent {
        &self.intent
    }

    /// Borrow the canonical desired body.
    pub fn canonical_resource(&self) -> &[u8] {
        &self.canonical_resource
    }

    /// Materialize the complete create payload Core submits to the Resource
    /// API for this child.
    ///
    /// Providers only declare UID-free intent. Core supplies the common
    /// metadata and status envelope while the store remains responsible for
    /// minting the authoritative UID and revision.
    pub fn create_payload(
        &self,
        zone: &d2b_contracts_resource::v3::ZoneId,
    ) -> Result<Vec<u8>, BindingChildMaterializationError> {
        let resource: BundleResource = serde_json::from_slice(&self.canonical_resource)
            .map_err(|_| BindingChildMaterializationError::MalformedResource)?;
        if resource.metadata().zone() != zone {
            return Err(BindingChildMaterializationError::OwnerMismatch);
        }
        materialize_child_create_payload(&self.intent, zone)
    }
}

/// Compute the stable desired-state digest for a child resource.
///
/// Resource API payloads gain UID, revision, timestamps, status, and
/// controller metadata when they enter the store. Those fields are runtime
/// state, not Provider intent, so owner repair compares only the identity,
/// ownership, presentation metadata, and spec layers.
pub fn semantic_child_digest(
    canonical_resource: &[u8],
) -> Result<String, BindingChildMaterializationError> {
    let value = CanonicalJsonValue::parse(canonical_resource)
        .map_err(|_| BindingChildMaterializationError::MalformedResource)?;
    let CanonicalJsonValue::Object(root) = value else {
        return Err(BindingChildMaterializationError::MalformedResource);
    };
    let Some(resource_type) = root.get("type") else {
        return Err(BindingChildMaterializationError::MalformedResource);
    };
    let Some(CanonicalJsonValue::Object(metadata)) = root.get("metadata") else {
        return Err(BindingChildMaterializationError::MalformedResource);
    };
    let Some(spec) = root.get("spec") else {
        return Err(BindingChildMaterializationError::MalformedResource);
    };
    let Some(name) = metadata.get("name") else {
        return Err(BindingChildMaterializationError::MalformedResource);
    };
    let Some(zone) = metadata.get("zone") else {
        return Err(BindingChildMaterializationError::MalformedResource);
    };
    let mut semantic_metadata = BTreeMap::new();
    semantic_metadata.insert("name".to_owned(), name.clone());
    semantic_metadata.insert("zone".to_owned(), zone.clone());
    semantic_metadata.insert(
        "ownerRef".to_owned(),
        metadata
            .get("ownerRef")
            .cloned()
            .unwrap_or(CanonicalJsonValue::Null),
    );
    semantic_metadata.insert(
        "labels".to_owned(),
        metadata
            .get("labels")
            .cloned()
            .unwrap_or_else(|| CanonicalJsonValue::Object(BTreeMap::new())),
    );
    semantic_metadata.insert(
        "annotations".to_owned(),
        metadata
            .get("annotations")
            .cloned()
            .unwrap_or_else(|| CanonicalJsonValue::Object(BTreeMap::new())),
    );
    let mut semantic = BTreeMap::new();
    semantic.insert("type".to_owned(), resource_type.clone());
    semantic.insert(
        "metadata".to_owned(),
        CanonicalJsonValue::Object(semantic_metadata),
    );
    semantic.insert("spec".to_owned(), spec.clone());
    let canonical = CanonicalJsonValue::Object(semantic).to_canonical_bytes();
    Ok(canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &canonical))
}

/// Build an observed child row from a complete Resource API envelope.
///
/// This is the Core-side adapter used after a relist. It deliberately derives
/// the digest from the stored body instead of trusting a Provider-supplied
/// payload digest, keeping UID/revision fencing separate from desired-state
/// convergence.
pub fn observed_child_from_resource(
    target: ResourceKey,
    owner: &ResourceKey,
    owner_generation: d2b_contracts_resource::v3::ResourceGeneration,
    revision: ZoneRevision,
    canonical_resource: &[u8],
    deletion_requested: bool,
    deletion_ready: bool,
) -> Result<ObservedChild, BindingChildMaterializationError> {
    let digest = semantic_child_digest(canonical_resource)?;
    let value = CanonicalJsonValue::parse(canonical_resource)
        .map_err(|_| BindingChildMaterializationError::MalformedResource)?;
    let CanonicalJsonValue::Object(root) = value else {
        return Err(BindingChildMaterializationError::MalformedResource);
    };
    let Some(CanonicalJsonValue::String(resource_type)) = root.get("type") else {
        return Err(BindingChildMaterializationError::MalformedResource);
    };
    if resource_type != target.resource_ref().resource_type().as_str() {
        return Err(BindingChildMaterializationError::IdentityMismatch);
    }
    let Some(CanonicalJsonValue::Object(metadata)) = root.get("metadata") else {
        return Err(BindingChildMaterializationError::MalformedResource);
    };
    let Some(CanonicalJsonValue::String(name)) = metadata.get("name") else {
        return Err(BindingChildMaterializationError::MalformedResource);
    };
    if name != target.resource_ref().name().as_str() {
        return Err(BindingChildMaterializationError::IdentityMismatch);
    }
    let Some(CanonicalJsonValue::String(zone)) = metadata.get("zone") else {
        return Err(BindingChildMaterializationError::MalformedResource);
    };
    if zone != target.zone().as_str() {
        return Err(BindingChildMaterializationError::OwnerMismatch);
    }
    let Some(CanonicalJsonValue::String(uid)) = metadata.get("uid") else {
        return Err(BindingChildMaterializationError::MalformedResource);
    };
    if ResourceUid::parse(uid).map_err(|_| BindingChildMaterializationError::IdentityMismatch)?
        != *target.uid()
    {
        return Err(BindingChildMaterializationError::IdentityMismatch);
    }
    let Some(CanonicalJsonValue::Integer(observed_revision)) = metadata.get("revision") else {
        return Err(BindingChildMaterializationError::MalformedResource);
    };
    let observed_revision = u64::try_from(*observed_revision)
        .ok()
        .map(ZoneRevision::new)
        .filter(|observed_revision| observed_revision.get() != 0)
        .ok_or(BindingChildMaterializationError::MalformedResource)?;
    if observed_revision != revision {
        return Err(BindingChildMaterializationError::OwnerReconcile(
            OwnerReconcileError::StaleRevision,
        ));
    }
    let Some(owner_ref) = metadata.get("ownerRef") else {
        return Err(BindingChildMaterializationError::OwnerMismatch);
    };
    let CanonicalJsonValue::String(owner_ref) = owner_ref else {
        return Err(BindingChildMaterializationError::OwnerMismatch);
    };
    let owner_ref = ResourceRef::parse(owner_ref)
        .map_err(|_| BindingChildMaterializationError::OwnerMismatch)?;
    let Some(CanonicalJsonValue::Integer(generation)) = metadata.get("generation") else {
        return Err(BindingChildMaterializationError::MalformedResource);
    };
    let generation = u64::try_from(*generation)
        .ok()
        .and_then(|generation| d2b_contracts_resource::v3::ResourceGeneration::new(generation).ok())
        .ok_or(BindingChildMaterializationError::MalformedResource)?;
    let observed = ObservedChild::with_owner_and_dependencies(
        target,
        owner,
        owner_generation,
        revision,
        digest,
        deletion_requested,
        deletion_ready,
        std::iter::empty(),
    )
    .map_err(BindingChildMaterializationError::OwnerReconcile)?
    .with_generation(generation);
    if observed.owner_ref() != Some(&owner_ref) {
        return Err(BindingChildMaterializationError::OwnerMismatch);
    }
    Ok(observed)
}

/// Build the canonical, UID-free Resource API create payload for one child.
///
/// This is intentionally owned by Core: Providers cannot smuggle arbitrary
/// resource envelopes or choose a different Process Provider. The semantic
/// Provider remains the Endpoint owner, while Process execution is delegated
/// to the fixed system Process Provider.
pub fn materialize_child_create_payload(
    intent: &BindingChildIntent,
    zone: &d2b_contracts_resource::v3::ZoneId,
) -> Result<Vec<u8>, BindingChildMaterializationError> {
    let owner_ref = intent.owner_ref().to_canonical_string();
    let provider_ref = match intent.kind() {
        BindingChildKind::Process | BindingChildKind::EphemeralProcess => {
            "Provider/system-systemd".to_owned()
        }
        BindingChildKind::Endpoint => intent.provider_ref().to_canonical_string(),
    };
    let mut spec = serde_json::Map::new();
    match intent.kind() {
        BindingChildKind::Process | BindingChildKind::EphemeralProcess => {
            let process_provider = intent
                .process_provider()
                .unwrap_or("Provider/system-systemd");
            let process_template = intent.process_template().unwrap_or_else(|| intent.role());
            let process_domain = intent.process_domain().map(|domain| match domain {
                d2b_contracts_resource::v3::ExecutionDomain::System => "system",
                d2b_contracts_resource::v3::ExecutionDomain::User => "user",
            });
            let process_class =
                intent
                    .process_class()
                    .unwrap_or(if intent.kind() == BindingChildKind::Process {
                        "service"
                    } else {
                        "worker"
                    });
            spec.insert(
                "executionRef".to_owned(),
                serde_json::Value::String(intent.execution_ref().to_canonical_string()),
            );
            spec.insert(
                "processClass".to_owned(),
                serde_json::Value::String(process_class.to_owned()),
            );
            spec.insert(
                "template".to_owned(),
                serde_json::Value::String(process_template.to_owned()),
            );
            spec.insert(
                "providerRef".to_owned(),
                serde_json::Value::String(process_provider.to_owned()),
            );
            if let Some(process_domain) = process_domain {
                spec.insert(
                    "domain".to_owned(),
                    serde_json::Value::String(process_domain.to_owned()),
                );
            }
            if let Some(user_ref) = intent.process_user() {
                spec.insert(
                    "userRef".to_owned(),
                    serde_json::Value::String(user_ref.to_canonical_string()),
                );
            }
        }
        BindingChildKind::Endpoint => {
            let producer = intent
                .producer_ref()
                .ok_or(BindingChildMaterializationError::ProducerMismatch)?;
            spec.insert(
                "providerRef".to_owned(),
                serde_json::Value::String(provider_ref),
            );
            spec.insert(
                "producerRef".to_owned(),
                serde_json::Value::String(producer.to_canonical_string()),
            );
            spec.insert(
                "endpointClass".to_owned(),
                serde_json::Value::String("service".to_owned()),
            );
            spec.insert(
                "transport".to_owned(),
                serde_json::Value::String("opaque-carriage".to_owned()),
            );
            spec.insert(
                "purpose".to_owned(),
                serde_json::Value::String(intent.role().to_owned()),
            );
            spec.insert(
                "locality".to_owned(),
                serde_json::Value::String(
                    match intent.placement() {
                        BindingChildPlacement::Host => "host-local",
                        BindingChildPlacement::Guest => "guest-local",
                    }
                    .to_owned(),
                ),
            );
            spec.insert(
                "visibility".to_owned(),
                serde_json::Value::String("provider".to_owned()),
            );
            spec.insert(
                "attachmentPolicy".to_owned(),
                serde_json::json!({
                    "supported": true,
                    "maxAttachments": 1
                }),
            );
            spec.insert(
                "consumerPolicy".to_owned(),
                serde_json::json!({
                    "allowedOperations": ["resolve", "attach", "observe"]
                }),
            );
            spec.insert(
                "lifecyclePolicy".to_owned(),
                serde_json::Value::String("recycle-with-producer".to_owned()),
            );
        }
    }
    let value = serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": intent.kind().resource_type(),
        "metadata": {
            "name": intent.resource_ref().name().as_str(),
            "zone": zone.as_str(),
            "ownerRef": owner_ref,
            "finalizers": [],
            "deletionRequestedAt": null,
            "createdAt": "1970-01-01T00:00:00.000Z",
            "updatedAt": "1970-01-01T00:00:00.000Z",
            "generation": 1,
            "revision": 1,
            "managedBy": "controller"
        },
        "spec": spec,
        "status": {
            "observedGeneration": 0,
            "phase": "Pending",
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
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| BindingChildMaterializationError::MalformedResource)?;
    let canonical = CanonicalJsonValue::parse(&bytes)
        .map_err(|_| BindingChildMaterializationError::MalformedResource)?
        .to_canonical_bytes();
    if canonical != bytes {
        return Err(BindingChildMaterializationError::NonCanonicalResource);
    }
    Ok(canonical)
}

fn validate_spec(
    intent: &BindingChildIntent,
    spec: &d2b_contracts_resource::v3::CanonicalJsonObject,
) -> Result<(), BindingChildMaterializationError> {
    match intent.kind() {
        BindingChildKind::Process | BindingChildKind::EphemeralProcess => {
            let Some(execution_ref) = spec.get("executionRef") else {
                return Err(BindingChildMaterializationError::ExecutionTargetMismatch);
            };
            if execution_ref
                != &CanonicalJsonValue::String(intent.execution_ref().to_canonical_string())
            {
                return Err(BindingChildMaterializationError::ExecutionTargetMismatch);
            }
            if let Some(provider) = intent.process_provider()
                && spec.get("providerRef") != Some(&CanonicalJsonValue::String(provider.to_owned()))
            {
                return Err(BindingChildMaterializationError::ProcessContractMismatch);
            }
            if let Some(template) = intent.process_template()
                && spec.get("template") != Some(&CanonicalJsonValue::String(template.to_owned()))
            {
                return Err(BindingChildMaterializationError::ProcessContractMismatch);
            }
            if let Some(class) = intent.process_class()
                && spec.get("processClass") != Some(&CanonicalJsonValue::String(class.to_owned()))
            {
                return Err(BindingChildMaterializationError::ProcessContractMismatch);
            }
            if let Some(domain) = intent.process_domain() {
                let expected = match domain {
                    d2b_contracts_resource::v3::ExecutionDomain::System => "system",
                    d2b_contracts_resource::v3::ExecutionDomain::User => "user",
                };
                if spec.get("domain") != Some(&CanonicalJsonValue::String(expected.to_owned())) {
                    return Err(BindingChildMaterializationError::ProcessContractMismatch);
                }
            }
            if let Some(user_ref) = intent.process_user()
                && spec.get("userRef")
                    != Some(&CanonicalJsonValue::String(user_ref.to_canonical_string()))
            {
                return Err(BindingChildMaterializationError::ProcessContractMismatch);
            }
        }
        BindingChildKind::Endpoint => {
            let Some(producer_ref) = intent.producer_ref() else {
                return Ok(());
            };
            let Some(actual) = spec.get("producerRef") else {
                return Err(BindingChildMaterializationError::ProducerMismatch);
            };
            if actual != &CanonicalJsonValue::String(producer_ref.to_canonical_string()) {
                return Err(BindingChildMaterializationError::ProducerMismatch);
            }
        }
    }
    Ok(())
}

/// Errors from Core child validation or owner planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingChildMaterializationError {
    /// The desired body was empty.
    EmptyResource,
    /// The desired body was not a valid bundle resource.
    MalformedResource,
    /// Resource type or name differed from the UID-free intent.
    IdentityMismatch,
    /// The child did not name its Binding owner.
    OwnerMismatch,
    /// The Process execution target differed from the intent.
    ExecutionTargetMismatch,
    /// The Process provider, template, class, domain, or user differed from
    /// the declared intent.
    ProcessContractMismatch,
    /// The Endpoint producer differed from the intent.
    ProducerMismatch,
    /// The body was valid JSON but not canonical.
    NonCanonicalResource,
    /// The generic owner planner rejected the desired/observed set.
    OwnerReconcile(OwnerReconcileError),
}

impl core::fmt::Display for BindingChildMaterializationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::EmptyResource => "binding-child-resource-empty",
            Self::MalformedResource => "binding-child-resource-malformed",
            Self::IdentityMismatch => "binding-child-resource-identity-mismatch",
            Self::OwnerMismatch => "binding-child-resource-owner-mismatch",
            Self::ExecutionTargetMismatch => "binding-child-resource-execution-mismatch",
            Self::ProcessContractMismatch => "binding-child-resource-process-contract-mismatch",
            Self::ProducerMismatch => "binding-child-resource-producer-mismatch",
            Self::NonCanonicalResource => "binding-child-resource-not-canonical",
            Self::OwnerReconcile(error) => return write!(formatter, "{error}"),
        })
    }
}

impl std::error::Error for BindingChildMaterializationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_provider::v3::semantic_services::{
        SemanticFamily,
        child_resources::{
            BindingChildKind, BindingChildPlacement, BindingChildRequest, BindingChildSet,
            explicit_binding_children,
        },
    };
    use d2b_contracts_resource::v3::{CanonicalJsonObject, ResourceEnvelope, ZoneId};
    use d2b_contracts_zone_session::v3::resource_bundle::BundleResourceMetadata;
    use std::collections::BTreeMap;

    fn child_set() -> BindingChildSet {
        explicit_binding_children(
            SemanticFamily::Audio,
            ResourceRef::parse("audio.d2bus.org.AudioBinding/microphone").unwrap(),
            ResourceRef::parse("audio.d2bus.org.AudioService/host").unwrap(),
            ResourceRef::parse("Guest/dev-vm").unwrap(),
            ResourceRef::parse("Provider/audio-pipewire").unwrap(),
            &[
                BindingChildRequest::new(
                    BindingChildKind::Process,
                    BindingChildPlacement::Guest,
                    "guest-agent",
                ),
                BindingChildRequest::endpoint(
                    BindingChildPlacement::Guest,
                    "guest-endpoint",
                    "guest-agent",
                ),
            ],
        )
        .unwrap()
    }

    fn child_resource(intent: &BindingChildIntent) -> BindingChildResource {
        let mut fields = BTreeMap::new();
        match intent.kind() {
            BindingChildKind::Process | BindingChildKind::EphemeralProcess => {
                fields.insert(
                    "executionRef".to_owned(),
                    CanonicalJsonValue::String(intent.execution_ref().to_canonical_string()),
                );
            }
            BindingChildKind::Endpoint => {
                fields.insert(
                    "producerRef".to_owned(),
                    CanonicalJsonValue::String(
                        intent.producer_ref().unwrap().to_canonical_string(),
                    ),
                );
            }
        }
        let spec = CanonicalJsonObject::parse(
            &serde_json::to_vec(&CanonicalJsonValue::Object(fields)).unwrap(),
        )
        .unwrap();
        let metadata = BundleResourceMetadata::new(
            intent.resource_ref().name().clone(),
            ZoneId::parse("dev").unwrap(),
            Some(intent.owner_ref().clone()),
            BTreeMap::new(),
            BTreeMap::new(),
        );
        let resource = BundleResource::new(
            ResourceTypeName::parse(intent.kind().resource_type()).unwrap(),
            metadata,
            spec,
        )
        .unwrap();
        let bytes = CanonicalJsonValue::parse(&serde_json::to_vec(&resource).unwrap())
            .unwrap()
            .to_canonical_bytes();
        BindingChildResource::new(intent.clone(), bytes).unwrap()
    }

    #[test]
    fn materializes_store_create_payloads_with_process_provider_and_endpoint_contract() {
        let set = child_set();
        let process_intent = set.child("guest-agent").unwrap();
        let endpoint_intent = set.child("guest-endpoint").unwrap();
        let process = BindingChildResource::new(
            process_intent.clone(),
            child_resource(process_intent).canonical_resource().to_vec(),
        )
        .unwrap();
        let process_payload = process
            .create_payload(&ZoneId::parse("dev").unwrap())
            .unwrap();
        let mut process_value: serde_json::Value =
            serde_json::from_slice(&process_payload).unwrap();
        process_value["metadata"]["uid"] =
            serde_json::Value::String("123e4567-e89b-42d3-a456-426614174000".to_owned());
        let process_bytes = serde_json::to_vec(&process_value).unwrap();
        let process_envelope = ResourceEnvelope::from_json(&process_bytes).unwrap();
        assert_eq!(
            process_envelope
                .spec()
                .provider_ref()
                .unwrap()
                .to_canonical_string(),
            "Provider/system-systemd"
        );
        assert_eq!(
            process_envelope.spec().base().get("executionRef"),
            Some(&CanonicalJsonValue::String("Guest/dev-vm".to_owned()))
        );

        let endpoint = BindingChildResource::new(
            endpoint_intent.clone(),
            child_resource(endpoint_intent)
                .canonical_resource()
                .to_vec(),
        )
        .unwrap();
        let endpoint_payload = endpoint
            .create_payload(&ZoneId::parse("dev").unwrap())
            .unwrap();
        let mut endpoint_value: serde_json::Value =
            serde_json::from_slice(&endpoint_payload).unwrap();
        endpoint_value["metadata"]["uid"] =
            serde_json::Value::String("223e4567-e89b-42d3-a456-426614174000".to_owned());
        let endpoint_envelope =
            ResourceEnvelope::from_json(&serde_json::to_vec(&endpoint_value).unwrap()).unwrap();
        assert_eq!(
            endpoint_envelope.spec().base().get("producerRef"),
            Some(&CanonicalJsonValue::String(
                endpoint_intent
                    .producer_ref()
                    .unwrap()
                    .to_canonical_string()
            ))
        );
    }

    #[test]
    fn validates_child_resource_identity_and_rejects_mutated_contracts() {
        let set = child_set();
        let intent = set.child("guest-agent").unwrap();
        let resource = child_resource(intent);
        assert_eq!(resource.intent().resource_ref(), intent.resource_ref());
        assert_eq!(
            resource.intent().execution_ref(),
            &ResourceRef::parse("Guest/dev-vm").unwrap()
        );

        let reject_spec = |mut spec: serde_json::Value| {
            let mut invalid =
                serde_json::from_slice::<serde_json::Value>(resource.canonical_resource()).unwrap();
            invalid["spec"] = spec.take();
            let bytes = CanonicalJsonValue::parse(&serde_json::to_vec(&invalid).unwrap())
                .unwrap()
                .to_canonical_bytes();
            BindingChildResource::new(intent.clone(), bytes)
        };

        let spec = |resource: &BindingChildResource| {
            serde_json::from_slice::<serde_json::Value>(resource.canonical_resource()).unwrap()
                ["spec"]
                .clone()
        };
        let mut execution = spec(&resource);
        execution["executionRef"] = serde_json::Value::String("Guest/other".to_owned());
        assert_eq!(
            reject_spec(execution),
            Err(BindingChildMaterializationError::ExecutionTargetMismatch)
        );
    }

    #[test]
    fn semantic_digest_ignores_store_runtime_fields() {
        let set = child_set();
        let child = child_resource(set.child("guest-agent").unwrap());
        let payload = child
            .create_payload(&ZoneId::parse("dev").unwrap())
            .unwrap();
        let mut stored: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        stored["metadata"]["uid"] =
            serde_json::Value::String("423e4567-e89b-42d3-a456-426614174000".to_owned());
        stored["metadata"]["revision"] = serde_json::Value::from(91_u64);
        stored["metadata"]["updatedAt"] =
            serde_json::Value::String("2026-08-22T02:00:00.000Z".to_owned());
        stored["status"]["phase"] = serde_json::Value::String("Ready".to_owned());
        let stored_bytes = serde_json::to_vec(&stored).unwrap();
        assert_eq!(
            semantic_child_digest(&payload).unwrap(),
            semantic_child_digest(&stored_bytes).unwrap()
        );
    }

    #[test]
    fn create_payload_rejects_a_cross_zone_submission() {
        let set = child_set();
        let child = child_resource(set.child("guest-agent").unwrap());
        assert_eq!(
            child.create_payload(&ZoneId::parse("other").unwrap()),
            Err(BindingChildMaterializationError::OwnerMismatch)
        );
    }
}
