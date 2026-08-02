//! Diff and application adapter for integrity-verified Zone bundles.
//!
//! This layer translates the public bundle input contract into the existing
//! commit-before-effects configuration service. It also attaches the
//! store-owned management metadata and exact Credential lifecycle authority to
//! post-commit effects. Neither is accepted from the bundle.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::{
    BundleMetadata, BundleResource as InputBundleResource, ZoneBundle,
    v3::{ConfigurationGeneration, Timestamp},
};

use super::{
    ActivationOutcome, ActivationPlan, CanonicalSpec, ConfigurationAuditKind, ConfigurationEffect,
    ConfigurationError, ConfigurationIntent, ConfigurationService, GenerationCommitProof,
    ManagementAgent, ResourceBundle, ResourceKey, StoredResource,
};

/// Closed failure from adapting or applying a verified bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BundleApplyError {
    /// The bundle's canonical desired state could not be represented.
    CanonicalSpec,
    /// The underlying configuration service refused the transition.
    Configuration(ConfigurationError),
    /// An internally planned resource has no matching bundle payload.
    DesiredResourceMissing,
}

impl BundleApplyError {
    /// Return the stable failure label without identity-bearing data.
    pub const fn label(self) -> &'static str {
        match self {
            Self::CanonicalSpec => "bundle-apply-canonical-spec-invalid",
            Self::Configuration(error) => error.label(),
            Self::DesiredResourceMissing => "bundle-apply-desired-resource-missing",
        }
    }
}

impl core::fmt::Display for BundleApplyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.label())
    }
}

impl std::error::Error for BundleApplyError {}

impl From<ConfigurationError> for BundleApplyError {
    fn from(error: ConfigurationError) -> Self {
        Self::Configuration(error)
    }
}

/// One exact ordinary resource mutation required by an apply effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrdinaryMutationVerb {
    Create,
    UpdateSpec,
    Delete,
}

impl OrdinaryMutationVerb {
    /// Return the canonical resource verb.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::UpdateSpec => "update-spec",
            Self::Delete => "delete",
        }
    }
}

/// Supplemental Credential administration required for one lifecycle action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MutationAuthority {
    ordinary: OrdinaryMutationVerb,
    admin_credential_subresource: Option<OrdinaryMutationVerb>,
}

impl MutationAuthority {
    fn for_resource(key: &ResourceKey, ordinary: OrdinaryMutationVerb) -> Self {
        Self {
            ordinary,
            admin_credential_subresource: (key.type_name().as_str() == "Credential")
                .then_some(ordinary),
        }
    }

    /// Return the independently required ordinary CRUD verb.
    pub const fn ordinary(self) -> OrdinaryMutationVerb {
        self.ordinary
    }

    /// Return the exact `admin-credential` subresource, when required.
    pub const fn admin_credential_subresource(self) -> Option<OrdinaryMutationVerb> {
        self.admin_credential_subresource
    }
}

/// How one desired bundle resource is persisted after activation commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceApplyOperation {
    Create,
    UpdateSpec,
    RefreshConfigurationGeneration,
}

/// Runtime facts that gate one finalizer-safe cleanup attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemovedResourceObservation {
    incident_hold: bool,
    pending_finalizers: u32,
    live_controller_children: u32,
    elapsed_seconds: u64,
    max_finalizer_duration_seconds: u64,
}

impl RemovedResourceObservation {
    /// Construct a bounded cleanup observation supplied by the resource store.
    pub const fn new(
        incident_hold: bool,
        pending_finalizers: u32,
        live_controller_children: u32,
        elapsed_seconds: u64,
        max_finalizer_duration_seconds: u64,
    ) -> Self {
        Self {
            incident_hold,
            pending_finalizers,
            live_controller_children,
            elapsed_seconds,
            max_finalizer_duration_seconds,
        }
    }
}

/// Fail-closed disposition for one removed configuration resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovedResourceDisposition {
    /// An incident hold pins the resource and its source bundle.
    DeferredByIncidentHold,
    /// Finalizer holders must complete before deletion.
    AwaitingFinalizers,
    /// Finalizers exceeded the descriptor bound; no force-clear is permitted.
    FinalizerTimedOut,
    /// The owner controller must finish child cleanup.
    AwaitingControllerChildren,
    /// Store deletion may commit atomically.
    CommitDeletion,
}

impl RemovedResourceDisposition {
    /// Return the stable status condition or cleanup step.
    pub const fn label(self) -> &'static str {
        match self {
            Self::DeferredByIncidentHold => "Degraded/incident-hold",
            Self::AwaitingFinalizers => "pending-cleanup",
            Self::FinalizerTimedOut => "Degraded/finalizer-timeout",
            Self::AwaitingControllerChildren => "pending-cleanup",
            Self::CommitDeletion => "commit-deletion",
        }
    }

    /// Whether the store row may be deleted now.
    pub const fn permits_delete(self) -> bool {
        matches!(self, Self::CommitDeletion)
    }
}

/// Apply incident-hold, finalizer, stall, and owner-child ordering.
pub const fn removed_resource_disposition(
    observed: RemovedResourceObservation,
) -> RemovedResourceDisposition {
    if observed.incident_hold {
        RemovedResourceDisposition::DeferredByIncidentHold
    } else if observed.pending_finalizers > 0 {
        if observed.max_finalizer_duration_seconds > 0
            && observed.elapsed_seconds > observed.max_finalizer_duration_seconds
        {
            RemovedResourceDisposition::FinalizerTimedOut
        } else {
            RemovedResourceDisposition::AwaitingFinalizers
        }
    } else if observed.live_controller_children > 0 {
        RemovedResourceDisposition::AwaitingControllerChildren
    } else {
        RemovedResourceDisposition::CommitDeletion
    }
}

/// The desired payload retained for a post-commit resource-store mutation.
#[derive(Clone, PartialEq, Eq)]
pub struct DesiredBundleResource {
    key: ResourceKey,
    metadata: BundleMetadata,
    spec: d2b_contracts::v3::CanonicalJsonObject,
}

impl DesiredBundleResource {
    /// Borrow the Zone-local identity.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Borrow the Nix-authorable metadata projection.
    pub const fn metadata(&self) -> &BundleMetadata {
        &self.metadata
    }

    /// Borrow the exact desired-state object.
    pub const fn spec(&self) -> &d2b_contracts::v3::CanonicalJsonObject {
        &self.spec
    }
}

impl core::fmt::Debug for DesiredBundleResource {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DesiredBundleResource")
            .field("spec_fields", &self.spec.len())
            .finish_non_exhaustive()
    }
}

/// Per-item activation status for a foreign-owned name collision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameConflict {
    key: ResourceKey,
}

impl NameConflict {
    /// Borrow the conflicting resource identity.
    pub const fn key(&self) -> &ResourceKey {
        &self.key
    }

    /// Return the stable item condition.
    pub const fn condition(&self) -> &'static str {
        "Degraded/name-conflict"
    }
}

/// One post-commit effect from applying a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BundleApplyEffect {
    /// Persist desired state and core-owned management metadata.
    PersistResource {
        resource: DesiredBundleResource,
        operation: ResourceApplyOperation,
        managed_by: ManagementAgent,
        configuration_generation: ConfigurationGeneration,
        authority: Option<MutationAuthority>,
        triggers_reconcile: bool,
    },
    /// Request finalizer-safe asynchronous deletion.
    DeleteResource {
        key: ResourceKey,
        authority: MutationAuthority,
    },
    /// Cancel an outstanding Delete during rollback.
    CancelDelete(ResourceKey),
    /// Prune one prior bundle selected by the retention policy.
    PrunePriorBundle(d2b_contracts::v3::ResourceBundleGenerationId),
    /// Append one closed configuration audit event.
    AppendAudit(ConfigurationAuditKind),
    /// Append one name-conflict audit without seizing the existing resource.
    AppendNameConflictAudit(NameConflict),
    /// Notify reconcile loops after every mutation intent is queued.
    NotifyReconcile,
}

/// An open bundle apply pass. It releases no effect before durable commit.
#[derive(Debug)]
pub struct BundleApplyPlan {
    activation: ActivationPlan,
    desired: BTreeMap<ResourceKey, DesiredBundleResource>,
    conflicts: Vec<NameConflict>,
    configuration_generation: ConfigurationGeneration,
}

impl BundleApplyPlan {
    /// Borrow item-level foreign-name conflicts.
    pub fn name_conflicts(&self) -> &[NameConflict] {
        &self.conflicts
    }

    /// Borrow the underlying durable generation record to commit.
    pub const fn activation(&self) -> &ActivationPlan {
        &self.activation
    }
}

/// Result of diffing one verified input bundle.
#[derive(Debug)]
// Boxing the plan would change this public outcome's ownership API.
#[allow(clippy::large_enum_variant)]
pub enum BundleApplyOutcome {
    /// The content-derived generation identity is already serving.
    Unchanged { name_conflicts: Vec<NameConflict> },
    /// A new generation is ready for durable commit.
    Planned(BundleApplyPlan),
}

/// Durable-commit evidence for one bundle application.
#[derive(Debug)]
pub struct BundleApplyCommitProof {
    generation: GenerationCommitProof,
    desired: BTreeMap<ResourceKey, DesiredBundleResource>,
    conflicts: Vec<NameConflict>,
    configuration_generation: ConfigurationGeneration,
    unchanged: Vec<ResourceKey>,
}

impl BundleApplyCommitProof {
    /// Borrow the item-level conflicts retained for status projection.
    pub fn name_conflicts(&self) -> &[NameConflict] {
        &self.conflicts
    }
}

/// Diff an integrity-verified bundle against persisted resource-store rows.
///
/// A foreign-owned `(type, name)` is isolated as a degraded activation item.
/// It is omitted from the apply set, so the existing resource is untouched and
/// non-conflicting resources in the same generation continue to activate.
pub fn begin_bundle_apply(
    service: &mut ConfigurationService,
    bundle: &ZoneBundle,
    store: &[StoredResource],
    now: &Timestamp,
) -> Result<BundleApplyOutcome, BundleApplyError> {
    begin_bundle_apply_mode(service, bundle, store, now, false)
}

/// Diff a retained bundle for an explicit rollback activation.
pub fn begin_bundle_rollback(
    service: &mut ConfigurationService,
    bundle: &ZoneBundle,
    store: &[StoredResource],
    now: &Timestamp,
) -> Result<BundleApplyOutcome, BundleApplyError> {
    begin_bundle_apply_mode(service, bundle, store, now, true)
}

fn begin_bundle_apply_mode(
    service: &mut ConfigurationService,
    bundle: &ZoneBundle,
    store: &[StoredResource],
    now: &Timestamp,
    rollback: bool,
) -> Result<BundleApplyOutcome, BundleApplyError> {
    let declared: Vec<(ResourceKey, DesiredBundleResource, super::BundleResource)> = bundle
        .resources()
        .iter()
        .map(adapt_resource)
        .collect::<Result<_, _>>()?;
    let foreign: BTreeSet<&ResourceKey> = store
        .iter()
        .filter(|row| row.managed_by() != ManagementAgent::Configuration)
        .map(StoredResource::key)
        .collect();

    let conflicts: Vec<NameConflict> = declared
        .iter()
        .filter(|(key, _, _)| foreign.contains(key))
        .map(|(key, _, _)| NameConflict { key: key.clone() })
        .collect();
    let desired: BTreeMap<ResourceKey, DesiredBundleResource> = declared
        .iter()
        .filter(|(key, _, _)| !foreign.contains(key))
        .map(|(key, desired, _)| (key.clone(), desired.clone()))
        .collect();
    let resources: Vec<super::BundleResource> = declared
        .into_iter()
        .filter(|(key, _, _)| !foreign.contains(key))
        .map(|(_, _, resource)| resource)
        .collect();

    let adapted = ResourceBundle::new(
        bundle.zone().clone(),
        bundle.content_hash().clone(),
        resources,
    )?;
    let outcome = if rollback {
        service.begin_rollback(&adapted, store, now)?
    } else {
        service.begin_activation(&adapted, store, now)?
    };
    match outcome {
        ActivationOutcome::Unchanged => Ok(BundleApplyOutcome::Unchanged {
            name_conflicts: conflicts,
        }),
        ActivationOutcome::Planned(activation) => {
            let configuration_generation = activation.next_record().active_ordinal();
            Ok(BundleApplyOutcome::Planned(BundleApplyPlan {
                activation,
                desired,
                conflicts,
                configuration_generation,
            }))
        }
    }
}

/// Discard an open pass without mutation or effect.
pub fn abort_bundle_apply(service: &mut ConfigurationService, plan: BundleApplyPlan) {
    service.abort_activation(plan.activation);
}

/// Record the durable generation commit and issue apply proof.
pub fn commit_bundle_apply(
    service: &mut ConfigurationService,
    plan: BundleApplyPlan,
    now: &Timestamp,
) -> Result<BundleApplyCommitProof, BundleApplyError> {
    let unchanged = plan.activation.unchanged().to_vec();
    let generation = service.commit_activation(plan.activation, now)?;
    Ok(BundleApplyCommitProof {
        generation,
        desired: plan.desired,
        conflicts: plan.conflicts,
        configuration_generation: plan.configuration_generation,
        unchanged,
    })
}

/// Consume commit proof and release store mutations exactly once.
pub fn release_bundle_apply_effects(
    service: &mut ConfigurationService,
    proof: BundleApplyCommitProof,
) -> Result<Vec<BundleApplyEffect>, BundleApplyError> {
    let BundleApplyCommitProof {
        generation,
        desired,
        conflicts,
        configuration_generation,
        unchanged,
    } = proof;
    let effects = service.release_activation_effects(generation)?;
    let mut rendered = Vec::with_capacity(effects.len() + unchanged.len() + conflicts.len());
    for effect in effects {
        match effect {
            ConfigurationEffect::QueueIntent(intent) => match intent {
                ConfigurationIntent::Create(key) => rendered.push(persist_effect(
                    &desired,
                    configuration_generation,
                    key,
                    ResourceApplyOperation::Create,
                    Some(OrdinaryMutationVerb::Create),
                    true,
                )?),
                ConfigurationIntent::UpdateSpec(key) => rendered.push(persist_effect(
                    &desired,
                    configuration_generation,
                    key,
                    ResourceApplyOperation::UpdateSpec,
                    Some(OrdinaryMutationVerb::UpdateSpec),
                    true,
                )?),
                ConfigurationIntent::Delete(key) => {
                    rendered.push(BundleApplyEffect::DeleteResource {
                        authority: MutationAuthority::for_resource(
                            &key,
                            OrdinaryMutationVerb::Delete,
                        ),
                        key,
                    });
                }
                ConfigurationIntent::CancelDelete(key) => {
                    rendered.push(BundleApplyEffect::CancelDelete(key));
                }
            },
            ConfigurationEffect::PrunePriorBundle(generation) => {
                rendered.push(BundleApplyEffect::PrunePriorBundle(generation));
            }
            ConfigurationEffect::AppendAudit(kind) => {
                rendered.push(BundleApplyEffect::AppendAudit(kind));
                rendered.extend(
                    conflicts
                        .iter()
                        .cloned()
                        .map(BundleApplyEffect::AppendNameConflictAudit),
                );
            }
            ConfigurationEffect::NotifyReconcile => {
                for key in &unchanged {
                    rendered.push(persist_effect(
                        &desired,
                        configuration_generation,
                        key.clone(),
                        ResourceApplyOperation::RefreshConfigurationGeneration,
                        None,
                        false,
                    )?);
                }
                rendered.push(BundleApplyEffect::NotifyReconcile);
            }
        }
    }
    Ok(rendered)
}

fn adapt_resource(
    resource: &InputBundleResource,
) -> Result<(ResourceKey, DesiredBundleResource, super::BundleResource), BundleApplyError> {
    let key = ResourceKey::new(
        resource.resource_type().clone(),
        resource.metadata().name().clone(),
    );
    let canonical_spec = CanonicalSpec::from_fields([(
        "spec",
        String::from_utf8(resource.spec().to_canonical_bytes())
            .map_err(|_| BundleApplyError::CanonicalSpec)?,
    )])?;
    let desired = DesiredBundleResource {
        key: key.clone(),
        metadata: resource.metadata().clone(),
        spec: resource.spec().clone(),
    };
    let planned = super::BundleResource::new(key.clone(), canonical_spec);
    Ok((key, desired, planned))
}

fn persist_effect(
    desired: &BTreeMap<ResourceKey, DesiredBundleResource>,
    configuration_generation: ConfigurationGeneration,
    key: ResourceKey,
    operation: ResourceApplyOperation,
    ordinary: Option<OrdinaryMutationVerb>,
    triggers_reconcile: bool,
) -> Result<BundleApplyEffect, BundleApplyError> {
    let resource = desired
        .get(&key)
        .cloned()
        .ok_or(BundleApplyError::DesiredResourceMissing)?;
    Ok(BundleApplyEffect::PersistResource {
        authority: ordinary.map(|verb| MutationAuthority::for_resource(&key, verb)),
        resource,
        operation,
        managed_by: ManagementAgent::Configuration,
        configuration_generation,
        triggers_reconcile,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::{
        CanonicalJsonObject, ResourceBundleGenerationId, ResourceName, ResourceTypeName,
        SchemaFingerprint, ZoneId,
    };
    use d2b_contracts::{BundleMetadata, BundleResource as InputBundleResource};

    fn digest(byte: char) -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}", byte.to_string().repeat(64))).unwrap()
    }

    fn generation(byte: char) -> ResourceBundleGenerationId {
        ResourceBundleGenerationId::parse(format!("sha256:{}", byte.to_string().repeat(64)))
            .unwrap()
    }

    fn now() -> Timestamp {
        Timestamp::parse("2026-07-22T21:00:00.000Z").unwrap()
    }

    fn key(resource_type: &str, name: &str) -> ResourceKey {
        ResourceKey::new(
            ResourceTypeName::parse(resource_type).unwrap(),
            ResourceName::parse(name).unwrap(),
        )
    }

    fn resource(resource_type: &str, name: &str, value: &str) -> InputBundleResource {
        InputBundleResource::new(
            ResourceTypeName::parse(resource_type).unwrap(),
            BundleMetadata::new(
                ResourceName::parse(name).unwrap(),
                ZoneId::parse("work").unwrap(),
                None,
                BTreeMap::new(),
                BTreeMap::new(),
            )
            .unwrap(),
            CanonicalJsonObject::parse(format!(r#"{{"value":"{value}"}}"#).as_bytes()).unwrap(),
        )
        .unwrap()
    }

    fn bundle(resources: Vec<InputBundleResource>) -> ZoneBundle {
        ZoneBundle::build(
            ZoneId::parse("work").unwrap(),
            digest('a'),
            resources,
            BTreeMap::new(),
        )
        .unwrap()
    }

    fn stored(
        resource_type: &str,
        name: &str,
        agent: ManagementAgent,
        configured: bool,
        value: &str,
    ) -> StoredResource {
        StoredResource::new(
            key(resource_type, name),
            agent,
            configured.then(|| generation('1')),
            CanonicalSpec::from_fields([("spec", format!(r#"{{"value":"{value}"}}"#))]).unwrap(),
        )
    }

    fn service() -> ConfigurationService {
        ConfigurationService::restore(
            ZoneId::parse("work").unwrap(),
            super::super::GenerationRecord::new(
                generation('1'),
                ConfigurationGeneration::new(1).unwrap(),
                None,
                super::super::RetainedGenerations::default_value(),
                Vec::new(),
            )
            .unwrap(),
            super::super::StagedBundleIntegrity::Verified,
        )
    }

    fn planned(outcome: BundleApplyOutcome) -> BundleApplyPlan {
        match outcome {
            BundleApplyOutcome::Planned(plan) => plan,
            BundleApplyOutcome::Unchanged { .. } => panic!("expected planned apply"),
        }
    }

    #[test]
    fn foreign_name_conflict_is_degraded_without_blocking_other_resources() {
        let mut service = service();
        let input = bundle(vec![
            resource("Volume", "conflict", "new"),
            resource("Volume", "created", "new"),
        ]);
        let store = vec![stored(
            "Volume",
            "conflict",
            ManagementAgent::Controller,
            false,
            "old",
        )];
        let plan = planned(begin_bundle_apply(&mut service, &input, &store, &now()).unwrap());
        assert_eq!(plan.name_conflicts().len(), 1);
        assert_eq!(
            plan.name_conflicts()[0].condition(),
            "Degraded/name-conflict"
        );
        assert_eq!(
            plan.activation()
                .planned_intents()
                .iter()
                .map(ConfigurationIntent::key)
                .collect::<Vec<_>>(),
            vec![&key("Volume", "created")]
        );
        let proof = commit_bundle_apply(&mut service, plan, &now()).unwrap();
        let effects = release_bundle_apply_effects(&mut service, proof).unwrap();
        assert!(effects.iter().any(|effect| matches!(
            effect,
            BundleApplyEffect::AppendNameConflictAudit(conflict)
                if conflict.key() == &key("Volume", "conflict")
        )));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            BundleApplyEffect::PersistResource { resource, .. }
                if resource.key() == &key("Volume", "created")
        )));
    }

    #[test]
    fn absent_diff_deletes_only_configuration_owned_resources() {
        let mut service = service();
        let store = vec![
            stored(
                "Volume",
                "removed",
                ManagementAgent::Configuration,
                true,
                "old",
            ),
            stored(
                "Volume",
                "dynamic",
                ManagementAgent::Controller,
                false,
                "old",
            ),
            stored(
                "Credential",
                "api-created",
                ManagementAgent::Api,
                false,
                "old",
            ),
        ];
        let plan =
            planned(begin_bundle_apply(&mut service, &bundle(Vec::new()), &store, &now()).unwrap());
        let proof = commit_bundle_apply(&mut service, plan, &now()).unwrap();
        let effects = release_bundle_apply_effects(&mut service, proof).unwrap();
        let deleted: Vec<&ResourceKey> = effects
            .iter()
            .filter_map(|effect| match effect {
                BundleApplyEffect::DeleteResource { key, .. } => Some(key),
                _ => None,
            })
            .collect();
        assert_eq!(deleted, vec![&key("Volume", "removed")]);
    }

    #[test]
    fn unchanged_resource_refreshes_generation_without_reconcile() {
        let input = bundle(vec![resource("Volume", "state", "same")]);
        let mut service = service();
        let store = vec![stored(
            "Volume",
            "state",
            ManagementAgent::Configuration,
            true,
            "same",
        )];
        let plan = planned(begin_bundle_apply(&mut service, &input, &store, &now()).unwrap());
        let proof = commit_bundle_apply(&mut service, plan, &now()).unwrap();
        let effects = release_bundle_apply_effects(&mut service, proof).unwrap();
        assert!(effects.iter().any(|effect| matches!(
            effect,
            BundleApplyEffect::PersistResource {
                operation: ResourceApplyOperation::RefreshConfigurationGeneration,
                managed_by: ManagementAgent::Configuration,
                configuration_generation,
                authority: None,
                triggers_reconcile: false,
                ..
            } if configuration_generation.get() == 2
        )));
        assert!(matches!(
            effects.last(),
            Some(BundleApplyEffect::NotifyReconcile)
        ));
    }

    #[test]
    fn credential_lifecycle_requires_ordinary_and_exact_admin_authority() {
        let mut service = service();
        let input = bundle(vec![resource("Credential", "access", "new")]);
        let plan = planned(begin_bundle_apply(&mut service, &input, &[], &now()).unwrap());
        let proof = commit_bundle_apply(&mut service, plan, &now()).unwrap();
        let effects = release_bundle_apply_effects(&mut service, proof).unwrap();
        let authority = effects.iter().find_map(|effect| match effect {
            BundleApplyEffect::PersistResource { authority, .. } => *authority,
            _ => None,
        });
        assert_eq!(
            authority,
            Some(MutationAuthority {
                ordinary: OrdinaryMutationVerb::Create,
                admin_credential_subresource: Some(OrdinaryMutationVerb::Create),
            })
        );
    }

    #[test]
    fn incident_hold_and_finalizer_timeout_never_force_cleanup() {
        let held =
            removed_resource_disposition(RemovedResourceObservation::new(true, 0, 0, 10_000, 60));
        assert_eq!(held, RemovedResourceDisposition::DeferredByIncidentHold);
        assert!(!held.permits_delete());

        let timed_out =
            removed_resource_disposition(RemovedResourceObservation::new(false, 1, 0, 61, 60));
        assert_eq!(timed_out.label(), "Degraded/finalizer-timeout");
        assert!(!timed_out.permits_delete());

        let children =
            removed_resource_disposition(RemovedResourceObservation::new(false, 0, 2, 1, 60));
        assert_eq!(
            children,
            RemovedResourceDisposition::AwaitingControllerChildren
        );
        assert!(!children.permits_delete());

        assert!(
            removed_resource_disposition(RemovedResourceObservation::new(false, 0, 0, 1, 60))
                .permits_delete()
        );
    }

    #[test]
    fn apply_debug_output_contains_no_identity_or_spec_value() {
        let mut service = service();
        let input = bundle(vec![resource("Credential", "secret-name", "secret-value")]);
        let plan = planned(begin_bundle_apply(&mut service, &input, &[], &now()).unwrap());
        let rendered = format!("{plan:?}");
        assert!(!rendered.contains("secret-name"));
        assert!(!rendered.contains("secret-value"));
        assert!(!rendered.contains("sha256:"));
    }
}
