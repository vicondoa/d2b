//! The new-graph limit and emergency admission (U40, KTD4/KTD6-KTD10; R8, R36).
//!
//! This is the composition's whole contribution to a limit decision, and it
//! holds no limit of its own. It names the prior accepted ceiling policy, the
//! prior accepted emergency reduction, and the census of committed rows, and
//! defers every rule to the two owning provider crates:
//!
//! - `d2b_provider_quota` decides whether a candidate fits the Zone's
//!   accepted ceilings, and from which boundary a budget is measured;
//! - `d2b_provider_emergency_policy` decides what an accepted reduction
//!   refuses and the ordered typed release it drives existing use through.
//!
//! The daemon composes those two decisions behind the one identity
//! evaluation of [`GraphMutationAdmission`], in the order the graph requires:
//! who is asking, then whether the Zone's emergency reduction admits new use,
//! then whether the candidate fits the Zone's ceilings. A refusal at any
//! stage is a refusal of the whole request, and it happens *before* the
//! manager persists anything, so a refused candidate leaves no desired row
//! behind (KTD6).
//!
//! # A policy row is never measured against its own policy
//!
//! An operator must always be able to change or clear the `Quota` and
//! `EmergencyPolicy` rows; a limit that refused its own type would leave a
//! Zone permanently unable to recover. Each owning provider therefore
//! remembers the type its accepted policy was read from and exempts that
//! type, and this composition passes the target through untouched, so the
//! exemption stays with the provider that owns the type name rather than
//! being spelled out here.
//!
//! # The prior accepted values, not the live ones
//!
//! [`AcceptedLimits`] is a snapshot: the ceilings, the reduction, and the
//! census as they were accepted before the request arrived. The plane swaps
//! the whole snapshot when the broker acknowledges a new accepted graph
//! (KTD6-KTD7); nothing here reads a live store, so the decision at this
//! boundary and the decision at the broker's boundary are the same decision
//! over the same input.
//!
//! U34 installs this in place of the admission the plane installs today
//! (`SystemZoneWriteFence` for the string-subject entry points, and
//! `AllowAll` for everything the new graph will serve), and wires
//! [`GraphLimitsAdmission::admit_new_use`] and
//! [`GraphLimitsAdmission::admit_budget`] into the broker's effect admission
//! and [`GraphLimitsAdmission::drain`] into the pre-drain stage, in the same
//! cutover that removes the old entry points.

use std::sync::Arc;

use d2b_contracts_resource::v3::{AdmissionDecision as GraphDecision, ResourceRef};
use d2b_resource_runtime::manager::{
    AdmissionDecision, AdmissionOp, MutationAdmission, MutationRequest, MutationSubject,
};

use crate::GraphMutationAdmission;

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
/// decisions. It is not installed in production yet: the unchanged entry
/// points still install what they installed before this unit, and U34
/// replaces them with this in the same step.
pub struct GraphLimitsAdmission {
    identity: Arc<GraphMutationAdmission>,
    limits: Arc<AcceptedLimits>,
}

impl GraphLimitsAdmission {
    /// Compose the identity evaluation with the accepted limits.
    pub fn new(identity: Arc<GraphMutationAdmission>, limits: Arc<AcceptedLimits>) -> Self {
        Self { identity, limits }
    }

    /// The accepted limits this admission measures against.
    pub fn limits(&self) -> &AcceptedLimits {
        &self.limits
    }

    /// Decide one new use at the effect boundary.
    ///
    /// This is the boundary a typed invocation reaches (KTD8): the effect is
    /// admitted against the same reduction the mutation seam was, so a
    /// reduction that landed after a row was committed still refuses the
    /// effect that row would start.
    pub fn admit_new_use(&self) -> GraphDecision {
        self.limits.emergency().admit_new_use()
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
        match self.limits.quota() {
            Some(policy) => d2b_provider_quota::quota::admit_budget(
                policy,
                self.limits.usage(),
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
        d2b_provider_emergency_policy::plan_drain(self.limits.emergency(), census)
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
        target: &ResourceRef,
        request: &MutationRequest,
    ) -> Result<d2b_provider_quota::quota::QuotaRequest, d2b_provider_quota::quota::QuotaError> {
        use d2b_provider_quota::quota::{QuotaRequest, ZoneBudget};
        if request.op == AdmissionOp::Remove {
            // A delete frees ceiling, so no ceiling can refuse it. This is
            // also the path a Zone takes to recover from its own excess.
            return Ok(QuotaRequest::reduces(target.clone()));
        }
        if self.limits.holds(target) {
            return Ok(QuotaRequest::neutral(target.clone()));
        }
        let owner = d2b_provider_quota::quota::owner_of_metadata(&request.metadata)?;
        let owner_depth = self.limits.usage().depth_with_owner(owner.as_ref())?;
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
        let adds_row = request.op == AdmissionOp::Ensure && !self.limits.holds(&target);
        if adds_row {
            // An emergency reduction blocks new use before the ceilings are
            // consulted: a reduction is the stricter statement, and its
            // refusal carries its own reason. The row-aware decision is the
            // one here, so the reduction's own row stays writable.
            match self.limits.emergency().admit_new_row(&target) {
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
        let Some(policy) = self.limits.quota() else {
            return AdmissionDecision::Allow;
        };
        let measured = match self.measure(&target, request) {
            Ok(measured) => measured,
            Err(error) => {
                return AdmissionDecision::Deny(format!(
                    "graph admission: the candidate's usage is undecidable: {error}"
                ));
            }
        };
        match d2b_provider_quota::quota::admit(policy, self.limits.usage(), &measured) {
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
        assert!(admission.limits().holds(&reference("Guest/vm")));
        assert!(!admission.limits().holds(&reference("Process/job")));
        assert_eq!(admission.limits().quota().expect("a ceiling is accepted").ceilings().max_resources(), 4);
        assert!(admission.limits().emergency().is_active());
    }
}
