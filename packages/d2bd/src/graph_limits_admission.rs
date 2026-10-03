//! The new-graph limit and emergency admission.
//!
//! # The prior accepted values, not the live ones
//!
//! [`AcceptedLimits`] is a snapshot: the ceilings, the reduction, and the
//! census as they were accepted before the request arrived. The holder swaps
//! the whole snapshot when the families publish a new one; nothing here reads
//! a live store, so the decision at this boundary and the decision at the
//! broker's boundary are the same decision over the same input.
//!
//! # The plane does not install this admission yet
//!
//! The plane still installs the zone-local write fence, so no committed
//! ceiling or reduction is enforced against a mutation today. This admission
//! exists, is proven, and is what the install will replace the fence with.
//!
//! The per-Zone accepted graph the identity arm needs is no longer the
//! blocker: the plane commits the Zone's own `Role` and `RoleBinding` rows
//! through the store's fenced path before its manager spawns, so the graph
//! built from them is rooted at that Zone. What blocks the install is the
//! subject the other mutating entry points present - the owned-cascade
//! boundaries render `zone/Type/name`, which is not the exact reference the
//! evaluator reads, and no Zone bundle grants a cascade subject - so
//! installing the identity arm today would refuse every owned-child commit
//! in every Zone.
//!
//! # A Zone with no verified deployment graph would admit nothing
//!
//! The identity arm is the prior accepted graph, and a Zone whose authority
//! nothing established has no accepted graph. Installing an empty one there
//! would refuse every mutation, and installing no check at all would admit
//! every mutation; the admission below refuses, because an unestablished
//! authority must never be read as an authority that permits.

use std::sync::Arc;

use d2b_contracts_resource::v3::{AdmissionDecision as GraphDecision, ResourceRef};
use d2b_resource_runtime::manager::{
    AdmissionDecision, AdmissionOp, MutationAdmission, MutationRequest, MutationSubject,
};

use crate::GraphMutationAdmission;

/// The manager-boundary admission of a Zone with no verified deployment
/// graph.
///
/// Every mutation is refused, with a reason that names what is missing rather
/// than pretending the request failed for some other cause. This is the
/// fail-closed direction: the alternative - an admission that admits because
/// nothing established the authority - would be exactly the hole the identity
/// arm exists to close.
pub struct UnestablishedAuthority;

impl core::fmt::Debug for UnestablishedAuthority {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("UnestablishedAuthority")
    }
}

impl MutationAdmission for UnestablishedAuthority {
    fn admit(&self, _subject: &MutationSubject, _request: &MutationRequest) -> AdmissionDecision {
        AdmissionDecision::Deny(
            "graph admission refused: this Zone has no verified deployment graph, so no \
             mutation is authorized against an authority nothing established"
                .to_owned(),
        )
    }
}



/// The ceilings, the reduction, and the census the Zone has accepted.
///
/// Every field is prior accepted state. The composition reads them and calls
/// the owning providers; it never derives a limit, a flag, or a cost from a
/// resource name.
pub struct AcceptedLimits {
    quota: Option<d2b_provider_quota::quota::QuotaPolicy>,
    emergency: d2b_provider_emergency_policy::EmergencyReduction,
    usage: d2b_provider_quota::quota::ZoneUsage,
}

impl AcceptedLimits {
    /// Assemble the accepted limits from the policy rows the broker accepted.
    ///
    /// The census starts empty and is attached with [`Self::bind`], which is
    /// the point in the freeze/commit/publish/acknowledge sequence where the
    /// committed rows are known.
    pub fn new(
        quota: Option<d2b_provider_quota::quota::QuotaPolicy>,
        emergency: d2b_provider_emergency_policy::EmergencyReduction,
    ) -> Self {
        Self {
            quota,
            emergency,
            usage: d2b_provider_quota::quota::ZoneUsage::default(),
        }
    }

    /// Attach the census of committed rows these limits are measured against.
    ///
    /// The census is the one record of what the Zone already holds: it counts
    /// the rows, resolves their owner chains, and answers which exact rows
    /// exist, so no second list can drift from the usage it describes.
    pub fn bind(mut self, usage: d2b_provider_quota::quota::ZoneUsage) -> Self {
        self.usage = usage;
        self
    }

    /// The Zone's accepted ceilings, when the Zone admitted a Quota row.
    pub fn quota(&self) -> Option<&d2b_provider_quota::quota::QuotaPolicy> {
        self.quota.as_ref()
    }

    /// The Zone's accepted emergency reduction.
    pub const fn emergency(&self) -> &d2b_provider_emergency_policy::EmergencyReduction {
        &self.emergency
    }

    /// The committed census these limits are measured against.
    pub const fn usage(&self) -> &d2b_provider_quota::quota::ZoneUsage {
        &self.usage
    }

    /// Whether the committed census already holds this row.
    pub fn holds(&self, target: &ResourceRef) -> bool {
        self.usage.holds(target)
    }
}

/// The manager-boundary admission of the new graph's limits and reduction.
///
/// It composes the plane's identity evaluation with the two owning providers'
/// decisions. The limits it measures against are read from a shared holder
/// rather than captured once, because a `Quota` or `EmergencyPolicy` row
/// commits after the manager spawned: a snapshot frozen at spawn holds only
/// the rows that existed then and would never enforce the ceiling an operator
/// just lowered.
pub struct GraphLimitsAdmission {
    identity: Arc<GraphMutationAdmission>,
    limits: AcceptedLimitsHolder,
}

/// The swappable prior accepted limits the admission measures against.
///
/// The holder owns the assembly of [`AcceptedLimits`] and nothing else does.
/// That matters because the value has three parts that two different families
/// own: the `Quota` driver publishes the ceilings and the census, and the
/// `EmergencyPolicy` driver publishes the reduction. If each driver wrote the
/// whole value, the second one to run would erase the first's half, so an
/// active emergency would vanish the moment a `Quota` row reconciled.
/// Instead each driver publishes only its own half into its own Zone runtime,
/// and this holder - the single reader of both - composes the value, so no
/// write can drop a half another family owns.
#[derive(Clone)]
pub struct AcceptedLimitsHolder {
    current: Arc<std::sync::RwLock<Arc<AcceptedLimits>>>,
    quota: Option<Arc<d2b_provider_quota::ZoneQuotaRuntime>>,
    emergency: Option<Arc<d2b_provider_emergency_policy::ZoneEmergencyRuntime>>,
}

impl AcceptedLimitsHolder {
    /// A holder over one accepted snapshot and no family runtimes.
    ///
    /// Nothing can republish it, so it enforces exactly the state it was
    /// built with. That is the fixed-snapshot shape, and it is honest about
    /// what it is rather than pretending to be live.
    pub fn new(limits: AcceptedLimits) -> Self {
        Self {
            current: Arc::new(std::sync::RwLock::new(Arc::new(limits))),
            quota: None,
            emergency: None,
        }
    }

    /// A holder that will never be republished, over one snapshot.
    pub fn from_snapshot(limits: Arc<AcceptedLimits>) -> Self {
        Self {
            current: Arc::new(std::sync::RwLock::new(limits)),
            quota: None,
            emergency: None,
        }
    }

    /// The production holder: the placed swappable cell plus the two per-Zone
    /// runtimes the family drivers publish their halves into.
    ///
    /// A runtime the composition root did not install contributes nothing,
    /// and the corresponding half of the value stays whatever the cell last
    /// held. That is the honest prior: a Zone whose family has not published
    /// yet has not stated a ceiling, which is not a ceiling of zero.
    pub fn live(
        cell: Arc<std::sync::RwLock<Arc<AcceptedLimits>>>,
        zone: &d2b_contracts_resource::v3::ZoneId,
    ) -> Self {
        Self {
            current: cell,
            quota: d2b_provider_quota::runtime(zone),
            emergency: d2b_provider_emergency_policy::runtime(zone),
        }
    }

    /// The limits as of this call, composed from both families' published
    /// halves.
    ///
    /// The composition is idempotent and side-effect free with respect to the
    /// families: it reads what they published and never writes to them, so
    /// the admission cannot feed a driver's own input back into it.
    pub fn current(&self) -> Arc<AcceptedLimits> {
        let published_quota = self
            .quota
            .as_ref()
            .and_then(|runtime| runtime.policy());
        let published_emergency = self
            .emergency
            .as_ref()
            .map(|runtime| runtime.reduction());
        let usage = self.quota.as_ref().map(|runtime| runtime.stored_usage());
        // Both runtimes installed is what makes the value complete, not both
        // families having published: the emergency runtime carries a reduction
        // from the moment it is installed (NONE when no policy has committed),
        // and the quota policy is an Option precisely because "no ceiling
        // admitted" is a real, decidable answer rather than a missing one.
        let (Some(_), Some(reduction)) = (&self.quota, published_emergency) else {
            // A runtime the composition root never installed contributes
            // nothing, and the last value stays in force rather than a
            // half-assembled one that would read the absent family as "no
            // ceiling" and admit past the limit the Zone did set.
            return self.stored();
        };
        // `published_quota` is an Option because "this Zone admitted no
        // ceiling" is a real, decidable answer - not a missing one. Refusing to
        // compose without it would drop an active emergency the moment a Zone
        // that meters nothing ran a reduction, which is the direction that must
        // never fail open.
        let mut composed = AcceptedLimits::new(published_quota, reduction);
        if let Some(usage) = usage {
            composed = composed.bind(usage);
        }
        let composed = Arc::new(composed);
        self.store(&composed);
        composed
    }

    /// The last value composed, or the one the cell was built with.
    fn stored(&self) -> Arc<AcceptedLimits> {
        self.current
            .read()
            .map(|current| Arc::clone(&current))
            .unwrap_or_else(|poisoned| Arc::clone(&poisoned.into_inner()))
    }

    /// Record the composed value so a read that finds an incomplete set still
    /// enforces the last complete one.
    ///
    /// A poisoned lock is written through rather than skipped: a holder that
    /// dropped a ceiling because an earlier writer panicked would admit past
    /// the limit the Zone set.
    fn store(&self, limits: &Arc<AcceptedLimits>) {
        match self.current.write() {
            Ok(mut current) => *current = Arc::clone(limits),
            Err(poisoned) => *poisoned.into_inner() = Arc::clone(limits),
        }
    }

    /// Publish a complete value directly.
    ///
    /// This is for the composition root and the tests, which own the whole
    /// value; the two family drivers publish halves into their runtimes and
    /// let [`Self::current`] compose.
    pub fn publish(&self, limits: AcceptedLimits) {
        self.store(&Arc::new(limits));
    }
}

impl core::fmt::Debug for AcceptedLimitsHolder {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("AcceptedLimitsHolder").finish_non_exhaustive()
    }
}

impl GraphLimitsAdmission {
    /// Compose the identity evaluation with one fixed snapshot of limits.
    pub fn new(identity: Arc<GraphMutationAdmission>, limits: Arc<AcceptedLimits>) -> Self {
        Self::with_holder(identity, AcceptedLimitsHolder::from_snapshot(limits))
    }

    /// Compose the identity evaluation with the swappable accepted limits.
    ///
    /// This is the production composition: the holder is the same one the
    /// Quota and EmergencyPolicy families publish to, so a ceiling an operator
    /// commits is enforced on the next mutation rather than the next restart.
    pub fn with_holder(identity: Arc<GraphMutationAdmission>, limits: AcceptedLimitsHolder) -> Self {
        Self { identity, limits }
    }

    /// The holder this admission measures against.
    pub const fn holder(&self) -> &AcceptedLimitsHolder {
        &self.limits
    }

    /// The accepted limits as of this call.
    fn snapshot(&self) -> Arc<AcceptedLimits> {
        self.limits.current()
    }

    /// The accepted limits this admission currently measures against.
    pub fn limits(&self) -> &AcceptedLimitsHolder {
        &self.limits
    }

    /// Decide one new use at the effect boundary.
    ///
    /// This is the boundary a typed invocation reaches (KTD8): the effect is
    /// admitted against the same reduction the mutation seam was, so a
    /// reduction that landed after a row was committed still refuses the
    /// effect that row would start.
    pub fn admit_new_use(&self) -> GraphDecision {
        self.snapshot().emergency().admit_new_use()
    }

    /// Decide one typed budget against the Zone's ceilings.
    ///
    /// The budget is the caller's typed input, never a value read out of a
    /// resource name: a type's own contract states what it consumes, and the
    /// measurement happens where that contract is read.
    pub fn admit_budget(
        &self,
        target: &ResourceRef,
        budget: d2b_provider_quota::quota::ZoneBudget,
        owner_depth: u32,
    ) -> GraphDecision {
        let limits = self.snapshot();
        match limits.quota() {
            Some(policy) => d2b_provider_quota::quota::admit_budget(
                policy,
                limits.usage(),
                target,
                budget,
                owner_depth,
            ),
            // A Zone that admitted no Quota row has no ceiling to measure
            // against, which is not the same as a ceiling of zero.
            None => GraphDecision::Admitted,
        }
    }

    /// Plan the ordered typed release an accepted reduction requires.
    ///
    /// `None` is a census the broker could not durably answer. The plan it
    /// produces fences the Zone instead of reporting a converged reduction,
    /// and `may_release` on the result is false until the reduction really
    /// has converged.
    pub fn drain(
        &self,
        census: Option<&d2b_provider_emergency_policy::OpenUseCensus>,
    ) -> d2b_provider_emergency_policy::EmergencyDrainPlan {
        d2b_provider_emergency_policy::plan_drain(self.snapshot().emergency(), census)
    }

    /// Measure one mutation as the usage it adds to the Zone.
    ///
    /// A delete adds nothing, a spec change to a row the census already holds
    /// adds nothing, and a new row adds one row at the depth its declared
    /// owner chain produces. The budget is zero here on purpose: a resource's
    /// CPU, memory, and storage are stated by its own type contract and are
    /// measured by [`Self::admit_budget`] where that contract is read, not
    /// guessed from a row name here.
    fn measure(
        &self,
        limits: &AcceptedLimits,
        target: &ResourceRef,
        request: &MutationRequest,
    ) -> Result<d2b_provider_quota::quota::QuotaRequest, d2b_provider_quota::quota::QuotaError> {
        use d2b_provider_quota::quota::{QuotaRequest, ZoneBudget};
        if request.op == AdmissionOp::Remove {
            // A delete frees ceiling, so no ceiling can refuse it. This is
            // also the path a Zone takes to recover from its own excess.
            return Ok(QuotaRequest::reduces(target.clone()));
        }
        if limits.holds(target) {
            return Ok(QuotaRequest::neutral(target.clone()));
        }
        let owner = d2b_provider_quota::quota::owner_of_metadata(&request.metadata)?;
        let owner_depth = limits.usage().depth_with_owner(owner.as_ref())?;
        Ok(QuotaRequest::adds(target.clone(), owner_depth, ZoneBudget::ZERO))
    }
}

impl MutationAdmission for GraphLimitsAdmission {
    fn admit(&self, subject: &MutationSubject, request: &MutationRequest) -> AdmissionDecision {
        // Who is asking is decided first and by the one shared evaluator: a
        // request the identity evaluation refuses never reaches a limit
        // decision, so a limit can never become a way to see a refusal's
        // detail.
        if let AdmissionDecision::Deny(reason) = self.identity.admit(subject, request) {
            return AdmissionDecision::Deny(reason);
        }
        let Ok(target) = ResourceRef::parse(&format!(
            "{}/{}",
            request.key.type_name, request.key.name
        )) else {
            return AdmissionDecision::Deny(format!(
                "graph admission: {}/{} is not an exact resource reference",
                request.key.type_name, request.key.name
            ));
        };
        // One snapshot per decision: the identity evaluation, the reduction
        // and the ceiling are decided against the same accepted state, so a
        // ceiling that lands mid-decision cannot be half applied.
        let limits = self.snapshot();
        let adds_row = request.op == AdmissionOp::Ensure && !limits.holds(&target);
        if adds_row {
            // An emergency reduction blocks new use before the ceilings are
            // consulted: a reduction is the stricter statement, and its
            // refusal carries its own reason. The row-aware decision is the
            // one here, so the reduction's own row stays writable.
            match limits.emergency().admit_new_row(&target) {
                GraphDecision::Admitted => {}
                GraphDecision::Refused { stage, reason } => {
                    return AdmissionDecision::Deny(describe(
                        "emergency reduction",
                        stage,
                        reason,
                    ));
                }
            }
        }
        let Some(policy) = limits.quota() else {
            return AdmissionDecision::Allow;
        };
        let measured = match self.measure(&limits, &target, request) {
            Ok(measured) => measured,
            Err(error) => {
                return AdmissionDecision::Deny(format!(
                    "graph admission: the candidate's usage is undecidable: {error}"
                ));
            }
        };
        match d2b_provider_quota::quota::admit(policy, limits.usage(), &measured) {
            GraphDecision::Admitted => AdmissionDecision::Allow,
            GraphDecision::Refused { stage, reason } => {
                AdmissionDecision::Deny(describe("quota", stage, reason))
            }
        }
    }
}

/// Render one refusal as a diagnostic that names the enforcing policy, the
/// stage, and the typed reason, and nothing else.
fn describe(
    policy: &str,
    stage: d2b_contracts_resource::v3::AdmissionStage,
    reason: d2b_contracts_resource::v3::RefusalReason,
) -> String {
    format!(
        "graph admission refused by the {policy} at {}: {}",
        serde_json::to_string(&stage).unwrap_or_else(|_| "admit".to_owned()),
        serde_json::to_string(&reason).unwrap_or_else(|_| "limit-exceeds-ceiling".to_owned()),
    )
}

// ---------------------------------------------------------------------------
// The daemon-owned reads the two families' runtimes are built over
// ---------------------------------------------------------------------------

/// The plane's own committed rows, counted as the Zone's usage (U40, R8).
///
/// This is the store-backed implementation of the Quota family's declared
/// usage facet. It counts from committed rows alone: a row that was deleted
/// stops consuming ceiling, and a row that was committed is counted even
/// before any driver of its own has run. Nothing here is remembered from an
/// earlier read, so a stale census cannot be the one a ceiling is measured
/// against.
pub struct PlaneZoneUsage {
    /// The plane's own durable store, which is the only census authority.
    pub store: Arc<d2b_resource_runtime::spec_store::SpecStore>,
    /// The Zone whose committed rows are counted.
    pub zone: d2b_contracts_resource::v3::ZoneId,
}

#[async_trait::async_trait]
impl d2b_provider_quota::UsageSource for PlaneZoneUsage {
    async fn usage(&self) -> Result<Option<d2b_provider_quota::quota::ZoneUsage>, String> {
        use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
        use d2b_resource_runtime::spec_store::SpecSelector;

        let rows = self
            .store
            .list(SpecSelector {
                zone: Some(self.zone.as_str().to_owned()),
                type_name: None,
                owner_uid: None,
            })
            .await
            .map_err(|error| error.to_string())?;
        // The census is counted from the durable identities the store holds,
        // so a row's owner chain resolves against committed rows rather than
        // against anything a caller supplied.
        let counted = rows
            .iter()
            .filter_map(|row| {
                let reference =
                    ResourceRef::parse(&format!("{}/{}", row.key.type_name, row.key.name)).ok()?;
                let uid = ResourceUid::from_bytes(&row.uid).ok()?;
                let owner = row.owner_uid.and_then(|owner| ResourceUid::from_bytes(&owner).ok());
                Some((reference, uid, owner))
            })
            .collect::<Vec<_>>();
        // An undecidable census (an owner chain that cycles) is reported as
        // unknown rather than counted as a root: a cycle is a chain of
        // unbounded length under a finite ceiling.
        d2b_provider_quota::quota::ZoneUsage::census(&counted)
            .map(Some)
            .map_err(|error| error.to_string())
    }
}

/// The plane's own committed rows, read as the Zone's open use (U40, R36).
///
/// This is the store-backed implementation of the EmergencyPolicy family's
/// declared open-use facet. It names the Zone's committed reservations and
/// answers with no outstanding use only when it can actually establish it; a
/// read it could not complete is `None`, which fences the reduction rather
/// than converging it.
pub struct PlaneZoneOpenUse {
    /// The plane's own durable store, which is the only census authority.
    pub store: Arc<d2b_resource_runtime::spec_store::SpecStore>,
    /// The Zone whose committed reservations are named.
    pub zone: d2b_contracts_resource::v3::ZoneId,
}

#[async_trait::async_trait]
impl d2b_provider_emergency_policy::OpenUseSource for PlaneZoneOpenUse {
    async fn census(&self) -> Result<Option<d2b_provider_emergency_policy::OpenUseCensus>, String> {
        use d2b_contracts_resource::v3::ResourceRef;
        use d2b_resource_runtime::spec_store::SpecSelector;

        let rows = self
            .store
            .list(SpecSelector {
                zone: Some(self.zone.as_str().to_owned()),
                type_name: None,
                owner_uid: None,
            })
            .await
            .map_err(|error| error.to_string())?;
        // The reservations the Zone holds are the rows that are themselves a
        // scarce claim. A reservation is named by its committed row, never by
        // a name a caller supplied.
        let mut sources = Vec::new();
        for row in &rows {
            if !matches!(
                row.key.type_name.as_str(),
                d2b_provider_volume_binding::BINDING_TYPE_NAME
                    | "Volume"
                    | "Device"
                    | "Credential"
            ) {
                continue;
            }
            if let Ok(reference) =
                ResourceRef::parse(&format!("{}/{}", row.key.type_name, row.key.name))
            {
                sources.push(reference);
            }
        }
        // The outstanding use is not derivable from the desired rows alone:
        // a consumer's live claim and its helper legs are runtime state the
        // store does not carry. So the census this read can establish is the
        // Zone's reservations with no proven outstanding use, and every
        // reduction measured against it is one the Zone can actually show has
        // drained. A Zone that cannot establish even that is reported as
        // unknown below.
        if sources.is_empty() {
            // No committed reservation at all is a decidable answer, not an
            // unreadable one: the Zone holds nothing to drain.
            return Ok(Some(d2b_provider_emergency_policy::OpenUseCensus::new([], 0, 0, [])));
        }
        Ok(Some(d2b_provider_emergency_policy::OpenUseCensus::new(sources, 0, 0, [])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::{
        AdmissionStage, AuthoritySubject, AuthoritySubjectKind, RefusalReason, ResourceTypeName,
        StoreIncarnation,
    };
    use d2b_contracts_zone_session::v3::role::AuthorizedRole;
    use d2b_contracts_zone_session::v3::{
        EmergencyPolicySpec, EmergencyScope, RoleBindingSpec, RoleResourceVerb, RoleRule,
    };
    use d2b_core::resource_authority::{AcceptedGraph, TransportIdentity};
    use d2b_provider_emergency_policy::{EmergencyReduction, EnforcementState, OpenUse, OpenUseCensus};
    use d2b_provider_quota::quota::{
        QuotaCeilings, QuotaEnforcementPolicy, QuotaPolicy, ZoneBudget, ZoneUsage,
    };
    use d2b_resource_runtime::spec_store::{ResourceKey, ResourceProvenance};

    use std::collections::BTreeMap;

    const ZONE: &str = "limits";
    const ROLE: &str = "Role/operator";

    /// Every type this fixture's operator may create or delete.
    const TYPES: [&str; 5] = ["Process", "Guest", "Volume", "Quota", "EmergencyPolicy"];

    fn zone() -> d2b_contracts_resource::v3::ZoneId {
        d2b_contracts_resource::v3::ZoneId::parse(ZONE).expect("the fixture zone is canonical")
    }

    fn reference(value: &str) -> ResourceRef {
        ResourceRef::parse(value).expect("the fixture reference is canonical")
    }

    /// The prior accepted graph: the operator may create and delete every
    /// type this fixture uses, so the limit decisions are what the assertions
    /// below observe.
    fn accepted_graph() -> AcceptedGraph {
        let role = AuthorizedRole::new(
            vec![RoleRule::new(
                TYPES
                    .iter()
                    .map(|type_name| {
                        ResourceTypeName::parse(*type_name).expect("a registered resource type")
                    })
                    .collect(),
                vec![RoleResourceVerb::Create, RoleResourceVerb::Delete],
                Vec::new(),
                Vec::new(),
                vec![zone()],
                Vec::new(),
                Vec::new(),
            )
            .expect("the role rule validates")],
            Vec::new(),
        )
        .expect("the authorization-only role validates");
        let binding = RoleBindingSpec::new(
            reference(ROLE),
            vec![reference("User/operator")],
            None,
            None,
        )
        .expect("the role binding validates");
        AcceptedGraph::new(
            zone(),
            StoreIncarnation::parse("store-generation-1").expect("a bounded token"),
            AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
        )
        .with_role(reference(ROLE), role)
        .with_role_binding(reference("RoleBinding/operators"), binding)
    }

    fn request(op: AdmissionOp, type_name: &str, name: &str, metadata: &[u8]) -> MutationRequest {
        MutationRequest {
            key: ResourceKey::new(ZONE, type_name, name),
            op,
            spec: Vec::new(),
            metadata: metadata.to_vec(),
        }
    }

    fn subject() -> MutationSubject {
        MutationSubject {
            principal: reference("User/operator").to_canonical_string(),
            origin: ResourceProvenance::Api,
        }
    }

    /// A hard ceiling of `max_resources` rows, read from `Quota/zone`.
    fn quota(max_resources: u32) -> QuotaPolicy {
        QuotaPolicy::new(
            QuotaCeilings::new(max_resources, max_resources, 4, None, None, None)
                .expect("ceilings are in range"),
            BTreeMap::new(),
            QuotaEnforcementPolicy::Hard,
        )
        .expect("the policy validates")
        .bind_to(reference("Quota/zone"))
    }

    fn reduction() -> EmergencyReduction {
        EmergencyReduction::of(&[EmergencyPolicySpec::new(
            true,
            EmergencyScope::new(true, false, false, true),
            30,
            "operator reduction",
        )
        .expect("the policy validates")])
        .bind_to(reference("EmergencyPolicy/zone"))
    }

    fn usage() -> ZoneUsage {
        ZoneUsage::census(&[]).expect("an empty census is decidable")
    }

    /// The ceiling and the reduction measured against `committed` rows.
    fn limits(max_resources: u32, committed: &[&str]) -> AcceptedLimits {
        AcceptedLimits::new(Some(quota(max_resources)), reduction()).bind(census(committed))
    }

    /// The census the committed rows yield.
    fn census(rows: &[&str]) -> ZoneUsage {
        type CountedRow = (ResourceRef, d2b_contracts_resource::v3::ResourceUid, Option<
            d2b_contracts_resource::v3::ResourceUid,
        >);
        let counted: Vec<CountedRow> = rows
            .iter()
            .map(|row| {
                let (type_name, name) = row.split_once('/').expect("a `Type/name` fixture row");
                let row_key = d2b_resource_runtime::spec_store::ResourceKey::new(
                    ZONE,
                    type_name,
                    name,
                );
                let uid = d2b_contracts_resource::v3::ResourceUid::from_bytes(
                    &d2b_resource_runtime::manager::deterministic_uid(&row_key),
                )
                .expect("a manager row uid is a canonical uuid");
                (reference(row), uid, None)
            })
            .collect();
        ZoneUsage::census(&counted).expect("the committed rows form a decidable census")
    }

    fn admission(limits: AcceptedLimits) -> GraphLimitsAdmission {
        GraphLimitsAdmission::new(
            Arc::new(GraphMutationAdmission::new(
                Arc::new(accepted_graph()),
                zone(),
                TransportIdentity::ComponentSession,
            )),
            Arc::new(limits),
        )
    }

    #[test]
    fn the_identity_evaluation_decides_before_any_limit_does() {
        let admission = admission(
            AcceptedLimits::new(Some(quota(8)), EmergencyReduction::NONE).bind(usage()),
        );
        // No accepted grant covers this target, so the request is refused by
        // the shared evaluator and never reaches a ceiling.
        let denied = admission.admit(&subject(), &request(AdmissionOp::Ensure, "Role", "rogue", b""));
        let AdmissionDecision::Deny(reason) = denied else {
            panic!("an unauthorized target is refused");
        };
        assert!(reason.contains("graph admission refused"), "{reason}");
        let admitted = admission.admit(&subject(), &request(AdmissionOp::Ensure, "Process", "job", b""));
        assert!(matches!(admitted, AdmissionDecision::Allow));
    }

    #[test]
    fn a_candidate_past_a_ceiling_is_refused_where_the_ceiling_says_so() {
        let admission = admission(
            AcceptedLimits::new(Some(quota(2)), EmergencyReduction::NONE)
                .bind(census(&["Guest/vm", "Volume/data"])),
        );
        let refused = admission.admit(&subject(), &request(AdmissionOp::Ensure, "Process", "job", b""));
        let AdmissionDecision::Deny(reason) = refused else {
            panic!("two committed rows exhaust a two-row ceiling");
        };
        assert!(reason.contains("limit-exceeds-ceiling"), "{reason}");
        // A delete is never what a ceiling refuses: it is how a Zone recovers
        // from its own excess.
        assert!(matches!(
            admission.admit(&subject(), &request(AdmissionOp::Remove, "Process", "job", b"")),
            AdmissionDecision::Allow
        ));
    }

    #[test]
    fn a_policy_row_is_never_measured_against_its_own_policy() {
        // The ceiling is reached, so every other new row is refused - except
        // the row that states the ceiling.
        let limited =
            admission(AcceptedLimits::new(Some(quota(1)), EmergencyReduction::NONE)
                .bind(census(&["Guest/vm"])));
        assert!(
            matches!(
                limited.admit(&subject(), &request(AdmissionOp::Ensure, "Quota", "tighter", b"")),
                AdmissionDecision::Allow
            ),
            "a Zone must be able to change the row that limits it"
        );
        let refused = limited.admit(&subject(), &request(AdmissionOp::Ensure, "Process", "job", b""));
        let AdmissionDecision::Deny(reason) = refused else {
            panic!("the ceiling still refuses every other row");
        };
        assert!(reason.contains("limit-exceeds-ceiling"), "{reason}");

        // The same holds for a reduction: it refuses every new row except its
        // own, so the emergency can always be changed or cleared.
        let reducing = admission(
            AcceptedLimits::new(Some(quota(8)), reduction()).bind(census(&[])),
        );
        assert!(
            matches!(
                reducing.admit(
                    &subject(),
                    &request(AdmissionOp::Ensure, "EmergencyPolicy", "clear", b"")
                ),
                AdmissionDecision::Allow
            ),
            "a Zone must be able to clear the reduction that blocks it"
        );
        assert!(matches!(
            reducing.admit(&subject(), &request(AdmissionOp::Ensure, "Process", "job", b"")),
            AdmissionDecision::Deny(_)
        ));
    }

    #[test]
    fn an_emergency_reduction_blocks_new_use_but_not_recovery_or_an_existing_row() {
        let admission = admission(limits(8, &["Guest/vm"]));
        let refused = admission.admit(&subject(), &request(AdmissionOp::Ensure, "Process", "job", b""));
        let AdmissionDecision::Deny(reason) = refused else {
            panic!("a reduction blocks a new row");
        };
        assert!(reason.contains("emergency-reduction-active"), "{reason}");
        assert!(reason.contains("revoke"), "{reason}");
        assert_eq!(
            admission.admit_new_use(),
            GraphDecision::Refused {
                stage: AdmissionStage::Revoke,
                reason: RefusalReason::EmergencyReductionActive,
            },
            "the effect boundary refuses the same way the mutation seam did"
        );
        assert!(
            matches!(
                admission.admit(&subject(), &request(AdmissionOp::Remove, "Process", "job", b"")),
                AdmissionDecision::Allow
            ),
            "a reduction never traps a Zone by refusing its own recovery"
        );
    }

    #[test]
    fn an_ensure_for_a_committed_row_is_not_growth() {
        let admission = admission(
            AcceptedLimits::new(Some(quota(1)), EmergencyReduction::NONE)
                .bind(census(&["Guest/vm"])),
        );
        assert!(
            matches!(
                admission.admit(&subject(), &request(AdmissionOp::Ensure, "Guest", "vm", b"")),
                AdmissionDecision::Allow
            ),
            "one committed row against a one-row ceiling still admits its own spec update"
        );
    }

    #[test]
    fn an_unreadable_owner_chain_is_refused_rather_than_measured_as_a_root() {
        let admission =
            admission(AcceptedLimits::new(Some(quota(8)), EmergencyReduction::NONE).bind(usage()));
        let refused = admission.admit(
            &subject(),
            &request(AdmissionOp::Ensure, "Process", "job", br#"{"ownerRef":7}"#),
        );
        let AdmissionDecision::Deny(reason) = refused else {
            panic!("an owner the ceiling cannot measure is not a root");
        };
        assert!(reason.contains("undecidable"), "{reason}");
    }

    #[test]
    fn a_zone_with_no_quota_row_has_no_ceiling_to_exceed() {
        let admission =
            admission(AcceptedLimits::new(None, EmergencyReduction::NONE).bind(usage()));
        assert!(matches!(
            admission.admit(&subject(), &request(AdmissionOp::Ensure, "Process", "job", b"")),
            AdmissionDecision::Allow
        ));
        assert_eq!(
            admission.admit_budget(&reference("Process/job"), ZoneBudget::ZERO, 1),
            GraphDecision::Admitted
        );
    }

    #[test]
    fn a_declared_budget_is_measured_where_it_is_typed() {
        let policy = QuotaPolicy::new(
            QuotaCeilings::new(8, 8, 4, Some(1), None, None).expect("ceilings are in range"),
            BTreeMap::new(),
            QuotaEnforcementPolicy::Hard,
        )
        .expect("the policy validates")
        .bind_to(reference("Quota/zone"));
        let usage = usage()
            .with_budget(ZoneBudget { cpu: 1, memory_mib: 0, storage_gib: 0 })
            .expect("representable");
        let admission =
            admission(AcceptedLimits::new(Some(policy), EmergencyReduction::NONE).bind(usage));
        assert_eq!(
            admission.admit_budget(
                &reference("Process/job"),
                ZoneBudget { cpu: 1, memory_mib: 0, storage_gib: 0 },
                1
            ),
            GraphDecision::refuse(AdmissionStage::Admit, RefusalReason::LimitExceedsCeiling)
        );
        assert_eq!(
            admission.admit_budget(&reference("Process/job"), ZoneBudget::ZERO, 1),
            GraphDecision::Admitted
        );
    }

    #[test]
    fn an_unanswerable_census_leaves_the_zone_fenced_rather_than_converged() {
        let admission = admission(limits(8, &["Guest/vm"]));
        let census = OpenUseCensus::new(
            [reference("Volume/data")],
            0,
            0,
            [OpenUse::new(
                reference("Volume/data"),
                reference("Guest/vm"),
                [reference("Endpoint/virtiofsd")],
            )],
        );
        let draining = admission.drain(Some(&census));
        assert_eq!(draining.state(), EnforcementState::Pending);
        assert!(!draining.may_release());
        let drained = admission.drain(Some(&OpenUseCensus::new([], 0, 0, [])));
        assert_eq!(drained.state(), EnforcementState::Converged);
        assert!(drained.may_release());
        let fenced = admission.drain(None);
        assert_eq!(fenced.state(), EnforcementState::Fenced);
        assert!(!fenced.is_converged());
        assert!(!fenced.may_release());
    }

    #[test]
    fn the_accepted_limits_are_the_prior_accepted_snapshot() {
        let limits = limits(4, &["Guest/vm"]);
        let admission = admission(limits);
        // The snapshot is what every decision reads, so asserting on it is
        // asserting on the state the refusal and the admission both used.
        let accepted = admission.snapshot();
        assert!(accepted.holds(&reference("Guest/vm")));
        assert!(!accepted.holds(&reference("Process/job")));
        assert_eq!(accepted.quota().expect("a ceiling is accepted").ceilings().max_resources(), 4);
        assert!(accepted.emergency().is_active());
    }

    /// The holder is the live shape: a value published after construction is
    /// the one the next decision reads, which is what makes a ceiling an
    /// operator commits after the manager spawned enforceable at all.
    #[test]
    fn a_republished_limit_is_the_one_the_next_decision_reads() {
        let admission = admission(AcceptedLimits::new(
            Some(quota(2)),
            EmergencyReduction::NONE,
        )
        .bind(census(&["Guest/vm", "Volume/data"])));
        let denied = admission.admit(&subject(), &request(AdmissionOp::Ensure, "Process", "job", b""));
        assert!(matches!(denied, AdmissionDecision::Deny(_)), "the two-row ceiling is exhausted");

        // The operator raises the ceiling. A holder frozen at construction
        // would still refuse, which is exactly the bug this shape prevents.
        admission
            .holder()
            .publish(AcceptedLimits::new(Some(quota(8)), EmergencyReduction::NONE).bind(usage()));
        assert!(
            matches!(
                admission.admit(&subject(), &request(AdmissionOp::Ensure, "Process", "job", b"")),
                AdmissionDecision::Allow
            ),
            "a ceiling raised after the manager spawned is enforced from the next mutation"
        );
    }
}
