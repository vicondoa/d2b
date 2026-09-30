//! Zone-wide Quota ResourceType contract, and the decision its ceilings make.
//!
//! `Quota` is a scarce-resource scope claim: one accepted row states the
//! Zone-wide ceilings every other row is measured against. This module owns
//! three things, and nothing else in the tree decides them a second time:
//!
//! 1. the row's own desired-state shape - the ceilings, the per-type count
//!    ceilings, and whether exceeding them refuses or only reports;
//! 2. the Zone's usage census, counted from committed rows alone;
//! 3. the one admission decision a candidate gets, in the shared
//!    [`AdmissionDecision`] vocabulary, so a refusal names its stage and
//!    reason instead of a provider-local error string.
//!
//! # The refusal happens before the row exists (R8, KTD6)
//!
//! [`admit`] is a pure function of the prior accepted policy, the prior
//! accepted census, and the candidate. The manager calls it at the admission
//! seam, before anything is persisted, so a refused candidate leaves no
//! desired row to clean up later: a quota checked after the commit is a
//! report, not a limit.
//!
//! # What each boundary can decide
//!
//! Counts and owner depth are decidable from the candidate's identity and the
//! committed rows, so they are refused at the mutation boundary. CPU, memory,
//! and storage are the *declared budget* of a typed request, and a type's own
//! contract is what states it, so they are refused by [`admit_budget`] at the
//! effect boundary (KTD8) where that typed request exists. Neither boundary
//! guesses the other's input: a resource name is never read to decide what a
//! resource costs.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use d2b_contracts_resource::v3::{
    AdmissionDecision, AdmissionStage, CanonicalJsonValue, RefusalReason, ResourceRef,
    ResourceTypeName, ResourceUid, Timestamp,
};

/// Largest Zone-wide resource count a Quota row may declare.
pub const MAX_QUOTA_RESOURCES: u32 = 65_536;
/// Largest per-type resource count a Quota row may declare.
pub const MAX_QUOTA_RESOURCES_PER_TYPE: u32 = 65_536;
/// Deepest owner chain a Quota row may admit.
pub const MAX_QUOTA_OWNER_DEPTH: u32 = 32;
/// Largest number of per-type ceilings one Quota row may declare.
pub const MAX_QUOTA_PER_TYPE_CEILINGS: usize = 64;

/// The only scope a Zone-wide Quota row may declare.
pub const QUOTA_SCOPE_ZONE: &str = "zone";

/// Whether a ceiling refuses a candidate or only reports the excess.
///
/// A hard quota is enforcement: exceeding it is an admission refusal. A soft
/// quota is the reporting form this type's status contract already carried -
/// the candidate is admitted and the excess is published as `overQuota` on
/// the row's own status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum QuotaEnforcementPolicy {
    /// Exceeding a ceiling refuses the candidate.
    Hard,
    /// Exceeding a ceiling is reported and nothing is refused.
    Soft,
}

/// Why an accepted Quota row could not be read as ceilings.
///
/// Every variant is a refusal to decide, never a default. A ceiling this
/// decoder cannot read would leave the Zone without a limit, so the row is
/// refused rather than admitted with its authority dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaError {
    /// The desired-state bytes are not a JSON object.
    UnreadableDesiredState,
    /// A field this type does not declare is present.
    UnknownField,
    /// A field this type requires is absent.
    MissingField,
    /// A field is present with a value the contract does not admit.
    InvalidField,
    /// A ceiling is outside the frozen bound the type admits.
    CeilingOutOfRange,
    /// More per-type ceilings than the type admits.
    TooManyTypeCeilings,
    /// The committed rows name an owner that is not itself committed.
    UnresolvedOwner,
    /// The committed rows' owner links form a cycle.
    OwnerCycle,
}

impl core::fmt::Display for QuotaError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::UnreadableDesiredState => "quota-desired-state-unreadable",
            Self::UnknownField => "quota-field-unknown",
            Self::MissingField => "quota-field-missing",
            Self::InvalidField => "quota-field-invalid",
            Self::CeilingOutOfRange => "quota-ceiling-out-of-range",
            Self::TooManyTypeCeilings => "quota-type-ceilings-too-many",
            Self::UnresolvedOwner => "quota-owner-unresolved",
            Self::OwnerCycle => "quota-owner-cycle",
        })
    }
}

impl std::error::Error for QuotaError {}

/// The Zone-wide resource ceilings of one accepted Quota row.
///
/// A `None` ceiling is unbounded, which is the row's own way of saying a
/// dimension is not metered. It is never a zero: a declared zero is refused
/// by [`QuotaCeilings::new`], so a Zone cannot express "no access at all"
/// with a missing number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaCeilings {
    max_resources: u32,
    max_resources_per_type: u32,
    max_owner_depth: u32,
    max_cpu: Option<u32>,
    max_memory_mib: Option<u32>,
    max_storage_gib: Option<u32>,
}

impl QuotaCeilings {
    /// Construct ceilings after checking the frozen bounds the type admits.
    pub const fn new(
        max_resources: u32,
        max_resources_per_type: u32,
        max_owner_depth: u32,
        max_cpu: Option<u32>,
        max_memory_mib: Option<u32>,
        max_storage_gib: Option<u32>,
    ) -> Result<Self, QuotaError> {
        if max_resources == 0 || max_resources > MAX_QUOTA_RESOURCES {
            return Err(QuotaError::CeilingOutOfRange);
        }
        if max_resources_per_type == 0 || max_resources_per_type > MAX_QUOTA_RESOURCES_PER_TYPE {
            return Err(QuotaError::CeilingOutOfRange);
        }
        if max_owner_depth == 0 || max_owner_depth > MAX_QUOTA_OWNER_DEPTH {
            return Err(QuotaError::CeilingOutOfRange);
        }
        Ok(Self {
            max_resources,
            max_resources_per_type,
            max_owner_depth,
            max_cpu,
            max_memory_mib,
            max_storage_gib,
        })
    }

    /// The Zone-wide row-count ceiling.
    pub const fn max_resources(&self) -> u32 {
        self.max_resources
    }

    /// The ceiling a type without its own entry is measured against.
    pub const fn max_resources_per_type(&self) -> u32 {
        self.max_resources_per_type
    }

    /// The deepest owner chain the Zone admits.
    pub const fn max_owner_depth(&self) -> u32 {
        self.max_owner_depth
    }

    /// The CPU ceiling, when the Zone meters CPU.
    pub const fn max_cpu(&self) -> Option<u32> {
        self.max_cpu
    }

    /// The memory ceiling in MiB, when the Zone meters memory.
    pub const fn max_memory_mib(&self) -> Option<u32> {
        self.max_memory_mib
    }

    /// The storage ceiling in GiB, when the Zone meters storage.
    pub const fn max_storage_gib(&self) -> Option<u32> {
        self.max_storage_gib
    }
}

/// The CPU, memory, and storage one accounted resource contributes.
///
/// The units are this contract's own: CPU count, MiB, and GiB. A resource
/// contributes what its own type contract declares; a resource with no
/// metered dimension contributes zero rather than an assumed amount.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ZoneBudget {
    /// CPU units.
    pub cpu: u32,
    /// Memory in MiB.
    pub memory_mib: u32,
    /// Storage in GiB.
    pub storage_gib: u32,
}

impl ZoneBudget {
    /// A budget that meters nothing.
    pub const ZERO: Self = Self {
        cpu: 0,
        memory_mib: 0,
        storage_gib: 0,
    };

    /// Add two budgets, or report that the sum cannot be represented.
    ///
    /// An unrepresentable sum is a refusal to decide, never a wrap: a
    /// wrapped total would admit a candidate that exceeds the ceiling.
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match (
            self.cpu.checked_add(other.cpu),
            self.memory_mib.checked_add(other.memory_mib),
            self.storage_gib.checked_add(other.storage_gib),
        ) {
            (Some(cpu), Some(memory_mib), Some(storage_gib)) => {
                Some(Self { cpu, memory_mib, storage_gib })
            }
            _ => None,
        }
    }

    /// Whether this total is past a metered ceiling.
    fn exceeds(self, ceilings: &QuotaCeilings) -> bool {
        [
            (ceilings.max_cpu(), self.cpu),
            (ceilings.max_memory_mib(), self.memory_mib),
            (ceilings.max_storage_gib(), self.storage_gib),
        ]
        .into_iter()
        .any(|(ceiling, used)| ceiling.is_some_and(|ceiling| used > ceiling))
    }
}

/// The Zone's usage, counted from committed rows alone.
///
/// Nothing here is remembered from an earlier reconcile: the census is
/// derived, so a row that was deleted stops consuming ceiling, and a row that
/// was committed is counted even if no driver has run for it yet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ZoneUsage {
    resources: u32,
    by_type: BTreeMap<ResourceTypeName, u32>,
    budget: ZoneBudget,
    deepest_owner_depth: u32,
    uids: BTreeMap<ResourceRef, ResourceUid>,
    owners: BTreeMap<ResourceUid, Option<ResourceUid>>,
}

impl ZoneUsage {
    /// Count the Zone's usage from its committed rows.
    ///
    /// Each row contributes its exact reference, its store-assigned
    /// identity, and the owner its durable owner column names. A row whose
    /// owner links form a cycle is an undecidable census: it is refused
    /// rather than counted as a root, because a cycle is a chain of unbounded
    /// length under a finite ceiling.
    pub fn census(rows: &[(ResourceRef, ResourceUid, Option<ResourceUid>)]) -> Result<Self, QuotaError> {
        let mut usage = Self::default();
        for (reference, uid, owner) in rows {
            usage.resources = usage
                .resources
                .checked_add(1)
                .ok_or(QuotaError::CeilingOutOfRange)?;
            let type_name = reference.resource_type();
            let count = usage.by_type.entry(type_name.clone()).or_insert(0);
            *count = count.checked_add(1).ok_or(QuotaError::CeilingOutOfRange)?;
            usage.uids.insert(reference.clone(), uid.clone());
            usage.owners.insert(uid.clone(), owner.clone());
        }
        for (_, uid, _) in rows {
            usage.deepest_owner_depth = usage.deepest_owner_depth.max(usage.depth_of(uid)?);
        }
        Ok(usage)
    }

    /// Account one resource's declared budget into this census.
    pub fn with_budget(mut self, budget: ZoneBudget) -> Result<Self, QuotaError> {
        self.budget = self
            .budget
            .checked_add(budget)
            .ok_or(QuotaError::CeilingOutOfRange)?;
        Ok(self)
    }

    /// The Zone-wide row count.
    pub const fn resources(&self) -> u32 {
        self.resources
    }

    /// The rows counted for one type.
    pub fn count_of(&self, type_name: &ResourceTypeName) -> u32 {
        self.by_type.get(type_name).copied().unwrap_or(0)
    }

    /// Whether the census already counts this exact row.
    ///
    /// This is the one answer to "does the Zone already hold this": an
    /// `Ensure` for a row the census holds changes that row rather than
    /// adding one, so it is measured as neutral use instead of growth.
    pub fn holds(&self, reference: &ResourceRef) -> bool {
        self.uids.contains_key(reference)
    }

    /// The types the census counted, in name order.
    pub fn types(&self) -> impl Iterator<Item = &ResourceTypeName> {
        self.by_type.keys()
    }

    /// The accounted CPU, memory, and storage.
    pub const fn budget(&self) -> ZoneBudget {
        self.budget
    }

    /// The deepest owner chain among the committed rows.
    pub const fn owner_depth(&self) -> u32 {
        self.deepest_owner_depth
    }

    /// The depth a candidate with this declared owner would produce.
    ///
    /// A candidate with no owner is a root at depth one. A candidate whose
    /// owner is not a committed row is refused: the chain it would extend
    /// cannot be measured, and admitting it would admit an unbounded depth
    /// under a finite ceiling.
    pub fn depth_with_owner(&self, owner: Option<&ResourceRef>) -> Result<u32, QuotaError> {
        let Some(owner) = owner else {
            return Ok(1);
        };
        let uid = self.uids.get(owner).ok_or(QuotaError::UnresolvedOwner)?;
        Ok(self.depth_of(uid)? + 1)
    }

    /// The chain length above one committed row.
    fn depth_of(&self, uid: &ResourceUid) -> Result<u32, QuotaError> {
        let mut depth = 0_u32;
        let mut current = self.owners.get(uid).cloned().flatten();
        while let Some(node) = current {
            if node == *uid {
                return Err(QuotaError::OwnerCycle);
            }
            depth += 1;
            if depth >= MAX_QUOTA_OWNER_DEPTH {
                // A chain at least as deep as any admissible ceiling is
                // reported at the ceiling rather than walked to its end: the
                // decision is the same refusal, and the walk terminates on
                // committed data.
                return Ok(MAX_QUOTA_OWNER_DEPTH);
            }
            current = self.owners.get(&node).cloned().flatten();
        }
        Ok(depth)
    }
}

/// What one candidate does to the Zone's usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuotaEffect {
    /// The candidate adds one row of its type at this depth with this budget.
    Adds {
        /// The owner chain length the candidate would produce.
        owner_depth: u32,
        /// The budget the candidate's own type contract declares.
        budget: ZoneBudget,
    },
    /// The candidate removes use: a delete, or a spec that meters less.
    Reduces,
    /// The candidate changes no metered dimension.
    Neutral,
}

/// One candidate measured against the Zone's accepted ceilings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaRequest {
    target: ResourceRef,
    effect: QuotaEffect,
}

impl QuotaRequest {
    /// Measure a candidate that adds use.
    pub const fn adds(target: ResourceRef, owner_depth: u32, budget: ZoneBudget) -> Self {
        Self { target, effect: QuotaEffect::Adds { owner_depth, budget } }
    }

    /// Measure a candidate that removes use.
    pub const fn reduces(target: ResourceRef) -> Self {
        Self { target, effect: QuotaEffect::Reduces }
    }

    /// Measure a candidate that changes no metered dimension.
    pub const fn neutral(target: ResourceRef) -> Self {
        Self { target, effect: QuotaEffect::Neutral }
    }

    /// The exact resource this request is for.
    pub const fn target(&self) -> &ResourceRef {
        &self.target
    }

    /// What the candidate does to the Zone's usage.
    pub const fn effect(&self) -> QuotaEffect {
        self.effect
    }
}

/// One accepted Quota row: what the Zone admits, and how it is enforced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaPolicy {
    ceilings: QuotaCeilings,
    per_type_ceilings: BTreeMap<ResourceTypeName, u32>,
    enforcement: QuotaEnforcementPolicy,
    row: Option<ResourceRef>,
}

impl QuotaPolicy {
    /// Construct the accepted policy of one row.
    pub fn new(
        ceilings: QuotaCeilings,
        per_type_ceilings: BTreeMap<ResourceTypeName, u32>,
        enforcement: QuotaEnforcementPolicy,
    ) -> Result<Self, QuotaError> {
        if per_type_ceilings.len() > MAX_QUOTA_PER_TYPE_CEILINGS {
            return Err(QuotaError::TooManyTypeCeilings);
        }
        for count in per_type_ceilings.values() {
            if *count == 0 || *count > MAX_QUOTA_RESOURCES_PER_TYPE {
                return Err(QuotaError::CeilingOutOfRange);
            }
        }
        Ok(Self { ceilings, per_type_ceilings, enforcement, row: None })
    }

    /// Record the committed row these ceilings were read from.
    ///
    /// The binding is by type, not by name: every `Quota` row *is* the limit
    /// rather than something the limit measures, so a Zone that could not
    /// lower, add, or remove its own ceiling would be permanently unable to
    /// recover from an excess. An unbound policy measures every row, which is
    /// the safe direction - it can only refuse more, never admit more.
    pub fn bind_to(mut self, row: ResourceRef) -> Self {
        self.row = Some(row);
        self
    }

    /// The committed row these ceilings were read from, and with it the type
    /// these ceilings govern.
    pub const fn row(&self) -> Option<&ResourceRef> {
        self.row.as_ref()
    }

    /// Whether this candidate is a limit rather than something a limit
    /// measures.
    fn measures(&self, target: &ResourceRef) -> bool {
        !self
            .row
            .as_ref()
            .is_some_and(|row| row.resource_type() == target.resource_type())
    }

    /// The Zone-wide ceilings.
    pub const fn ceilings(&self) -> &QuotaCeilings {
        &self.ceilings
    }

    /// Whether exceeding a ceiling refuses or only reports.
    pub const fn enforcement(&self) -> QuotaEnforcementPolicy {
        self.enforcement
    }

    /// The count ceiling that applies to one type.
    ///
    /// A type with its own ceiling is measured against it; every other type
    /// is measured against the Zone-wide per-type ceiling.
    pub fn type_ceiling(&self, type_name: &ResourceTypeName) -> u32 {
        self.per_type_ceilings
            .get(type_name)
            .copied()
            .unwrap_or_else(|| self.ceilings.max_resources_per_type())
    }

    /// The types this row meters individually.
    pub fn per_type_ceilings(&self) -> &BTreeMap<ResourceTypeName, u32> {
        &self.per_type_ceilings
    }

    /// Read the accepted policy out of one Quota row's desired-state bytes.
    ///
    /// The bytes are the canonical desired-state object the manager stores,
    /// so the universal layer's `providerRef` and `updatePolicy` fields are
    /// tolerated and the rest must be exactly the fields this type declares.
    /// A field this decoder does not know is refused rather than ignored: a
    /// ceiling written under a name this row does not implement is not a
    /// ceiling the Zone is enforcing.
    pub fn decode(desired_state: &[u8]) -> Result<Self, QuotaError> {
        let value =
            CanonicalJsonValue::parse(desired_state).map_err(|_| QuotaError::UnreadableDesiredState)?;
        let Some(fields) = value.as_object() else {
            return Err(QuotaError::UnreadableDesiredState);
        };
        for key in fields.keys() {
            if !matches!(
                key.as_str(),
                "ceilings" | "perTypeCeilings" | "scope" | "enforcementPolicy" | "providerRef"
                    | "updatePolicy" | "provider"
            ) {
                return Err(QuotaError::UnknownField);
            }
        }
        if let Some(found) = fields.get("providerRef")
            && !matches!(found, CanonicalJsonValue::String(_))
        {
            return Err(QuotaError::InvalidField);
        }
        for universal in ["updatePolicy", "provider"] {
            if let Some(found) = fields.get(universal)
                && !matches!(found, CanonicalJsonValue::Object(_))
            {
                return Err(QuotaError::InvalidField);
            }
        }
        let ceilings =
            decode_ceilings(fields.get("ceilings").ok_or(QuotaError::MissingField)?)?;
        let per_type_ceilings = decode_per_type_ceilings(
            fields.get("perTypeCeilings").ok_or(QuotaError::MissingField)?,
        )?;
        match fields.get("scope") {
            Some(CanonicalJsonValue::String(scope)) if scope == QUOTA_SCOPE_ZONE => {}
            Some(_) => return Err(QuotaError::InvalidField),
            None => return Err(QuotaError::MissingField),
        }
        let enforcement = match fields.get("enforcementPolicy") {
            Some(CanonicalJsonValue::String(policy)) if policy == "hard" => {
                QuotaEnforcementPolicy::Hard
            }
            Some(CanonicalJsonValue::String(policy)) if policy == "soft" => {
                QuotaEnforcementPolicy::Soft
            }
            Some(_) => return Err(QuotaError::InvalidField),
            None => return Err(QuotaError::MissingField),
        };
        Self::new(ceilings, per_type_ceilings, enforcement)
    }
}

/// Decide one candidate against the Zone's accepted ceilings.
///
/// The decision is a pure function of the prior accepted policy, the prior
/// accepted census, and the candidate. A candidate that only removes use is
/// always admitted: a limit must never be the reason a Zone cannot recover,
/// and a delete that frees ceiling can never exceed it. A soft quota refuses
/// nothing, which is what makes it a report rather than a limit.
pub fn admit(policy: &QuotaPolicy, usage: &ZoneUsage, request: &QuotaRequest) -> AdmissionDecision {
    if !policy.measures(request.target()) {
        return AdmissionDecision::Admitted;
    }
    let QuotaEffect::Adds { owner_depth, budget } = request.effect else {
        return AdmissionDecision::Admitted;
    };
    if policy.enforcement == QuotaEnforcementPolicy::Soft || !exceeds(policy, usage, request, owner_depth, budget) {
        return AdmissionDecision::Admitted;
    }
    AdmissionDecision::refuse(AdmissionStage::Admit, RefusalReason::LimitExceedsCeiling)
}

/// Decide one candidate's declared budget against the Zone's ceilings.
///
/// This is the boundary the typed request exists at (KTD8): a resource's
/// budget is stated by its own type contract, so it is measured where that
/// contract is read rather than guessed from a resource name at the mutation
/// seam. The counts and the owner depth are re-measured here too, because an
/// effect that runs under a row the Zone already counts must not be the way
/// past a ceiling the mutation seam already refused.
pub fn admit_budget(
    policy: &QuotaPolicy,
    usage: &ZoneUsage,
    target: &ResourceRef,
    budget: ZoneBudget,
    owner_depth: u32,
) -> AdmissionDecision {
    admit(policy, usage, &QuotaRequest::adds(target.clone(), owner_depth, budget))
}

/// Whether the candidate would push the Zone past a ceiling.
fn exceeds(
    policy: &QuotaPolicy,
    usage: &ZoneUsage,
    request: &QuotaRequest,
    owner_depth: u32,
    budget: ZoneBudget,
) -> bool {
    let ceilings = policy.ceilings();
    if usage
        .resources
        .checked_add(1)
        .is_none_or(|total| total > ceilings.max_resources())
    {
        return true;
    }
    let type_name = request.target.resource_type();
    if usage
        .count_of(type_name)
        .checked_add(1)
        .is_none_or(|total| total > policy.type_ceiling(type_name))
    {
        return true;
    }
    if owner_depth > ceilings.max_owner_depth() {
        return true;
    }
    match usage.budget().checked_add(budget) {
        Some(total) => total.exceeds(ceilings),
        None => true,
    }
}

/// Whether the Zone is past a ceiling, and which types are.
///
/// The reporting half of a soft quota and the published state of a hard one:
/// the same arithmetic the refusal uses, reported instead of enforced. The
/// committed census is reported whether or not a candidate is pending, so a
/// Zone that is already over its ceiling says so on its own status.
pub fn over_quota(
    policy: &QuotaPolicy,
    usage: &ZoneUsage,
    pending: Option<&QuotaRequest>,
) -> (bool, Vec<ResourceTypeName>) {
    let ceilings = policy.ceilings();
    let mut over_types: Vec<ResourceTypeName> = usage
        .types()
        .filter(|type_name| usage.count_of(type_name) > policy.type_ceiling(type_name))
        .cloned()
        .collect();
    let mut over = !over_types.is_empty()
        || usage.resources > ceilings.max_resources()
        || usage.budget().exceeds(ceilings);
    if let Some(request) = pending
        && let QuotaEffect::Adds { owner_depth, budget } = request.effect
    {
        let type_name = request.target.resource_type();
        if usage.count_of(type_name) >= policy.type_ceiling(type_name) {
            over_types.push(type_name.clone());
        }
        over |= exceeds(policy, usage, request, owner_depth, budget);
    }
    (over, over_types)
}

/// Read the owner a candidate's metadata envelope declares.
///
/// The envelope is the manager's own metadata object, so `ownerRef` is read
/// from it directly. Absent or null means a root row. A value that is not a
/// canonical reference, or an envelope that is not an object, is refused: an
/// unreadable owner is not a root.
pub fn owner_of_metadata(metadata: &[u8]) -> Result<Option<ResourceRef>, QuotaError> {
    if metadata.is_empty() {
        return Ok(None);
    }
    let value =
        CanonicalJsonValue::parse(metadata).map_err(|_| QuotaError::UnreadableDesiredState)?;
    let Some(fields) = value.as_object() else {
        return Err(QuotaError::UnreadableDesiredState);
    };
    match fields.get("ownerRef") {
        None | Some(CanonicalJsonValue::Null) => Ok(None),
        Some(CanonicalJsonValue::String(reference)) => {
            ResourceRef::parse(reference).map(Some).map_err(|_| QuotaError::InvalidField)
        }
        Some(_) => Err(QuotaError::InvalidField),
    }
}

/// ResourceType-common Quota status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuotaStatusResource {
    used_resources: u32,
    used_cpu: Option<u32>,
    used_memory_mib: Option<u32>,
    used_storage_gib: Option<u32>,
    over_quota: bool,
    over_quota_types: Vec<ResourceTypeName>,
    last_checked_at: Option<Timestamp>,
    dependent_count: u32,
}

impl QuotaStatusResource {
    /// Return resources currently counted.
    pub const fn used_resources(&self) -> u32 {
        self.used_resources
    }

    /// Return dependent resource count.
    pub const fn dependent_count(&self) -> u32 {
        self.dependent_count
    }

    /// Whether a soft quota is currently exceeded.
    pub const fn over_quota(&self) -> bool {
        self.over_quota
    }

    /// Render the status the committed census supports.
    ///
    /// The reported figures are the committed census, optionally advanced by
    /// the candidate being measured, so a status never claims usage a row
    /// does not have. A dimension the Zone does not meter is absent rather
    /// than zero, because the two say different things to a reader.
    pub fn of(
        policy: &QuotaPolicy,
        usage: &ZoneUsage,
        pending: Option<&QuotaRequest>,
        dependent_count: u32,
        checked_at: Option<Timestamp>,
    ) -> Self {
        let (over_quota, over_quota_types) = over_quota(policy, usage, pending);
        let ceilings = policy.ceilings();
        let budget = usage.budget();
        Self {
            used_resources: usage.resources(),
            used_cpu: ceilings.max_cpu().map(|_| budget.cpu),
            used_memory_mib: ceilings.max_memory_mib().map(|_| budget.memory_mib),
            used_storage_gib: ceilings.max_storage_gib().map(|_| budget.storage_gib),
            over_quota,
            over_quota_types,
            last_checked_at: checked_at,
            dependent_count,
        }
    }
}

/// Alias used by generic status adapters.
pub type QuotaStatus = QuotaStatusResource;

/// Read the ceiling object.
fn decode_ceilings(value: &CanonicalJsonValue) -> Result<QuotaCeilings, QuotaError> {
    let Some(fields) = value.as_object() else {
        return Err(QuotaError::InvalidField);
    };
    for key in fields.keys() {
        if !matches!(
            key.as_str(),
            "maxResources" | "maxResourcesPerType" | "maxOwnerDepth" | "maxCpu" | "maxMemoryMib"
                | "maxStorageGib"
        ) {
            return Err(QuotaError::UnknownField);
        }
    }
    QuotaCeilings::new(
        required_count(fields, "maxResources")?,
        required_count(fields, "maxResourcesPerType")?,
        required_count(fields, "maxOwnerDepth")?,
        optional_count(fields, "maxCpu")?,
        optional_count(fields, "maxMemoryMib")?,
        optional_count(fields, "maxStorageGib")?,
    )
}

/// Read the per-type ceiling object.
///
/// The contract leaves each per-type value an opaque object; the count
/// ceiling is what this row implements inside it, and an inner field this
/// decoder does not know is refused rather than skipped, so a ceiling written
/// under another name cannot pass as an enforced one.
fn decode_per_type_ceilings(
    value: &CanonicalJsonValue,
) -> Result<BTreeMap<ResourceTypeName, u32>, QuotaError> {
    let Some(fields) = value.as_object() else {
        return Err(QuotaError::InvalidField);
    };
    if fields.len() > MAX_QUOTA_PER_TYPE_CEILINGS {
        return Err(QuotaError::TooManyTypeCeilings);
    }
    let mut ceilings = BTreeMap::new();
    for (type_name, entry) in fields {
        let type_name = ResourceTypeName::parse(type_name.as_str())
            .map_err(|_| QuotaError::InvalidField)?;
        let Some(entry) = entry.as_object() else {
            return Err(QuotaError::InvalidField);
        };
        for key in entry.keys() {
            if key != "maxResources" {
                return Err(QuotaError::UnknownField);
            }
        }
        match entry.get("maxResources") {
            None => continue,
            Some(CanonicalJsonValue::Integer(count)) => {
                let count =
                    u32::try_from(*count).map_err(|_| QuotaError::CeilingOutOfRange)?;
                if count == 0 {
                    return Err(QuotaError::CeilingOutOfRange);
                }
                ceilings.insert(type_name, count);
            }
            Some(_) => return Err(QuotaError::InvalidField),
        }
    }
    Ok(ceilings)
}

/// Read one required positive count.
fn required_count(
    fields: &BTreeMap<String, CanonicalJsonValue>,
    key: &str,
) -> Result<u32, QuotaError> {
    match fields.get(key) {
        Some(CanonicalJsonValue::Integer(count)) => {
            u32::try_from(*count).map_err(|_| QuotaError::CeilingOutOfRange)
        }
        Some(_) => Err(QuotaError::InvalidField),
        None => Err(QuotaError::MissingField),
    }
}

/// Read one optional positive count, where null means unbounded.
fn optional_count(
    fields: &BTreeMap<String, CanonicalJsonValue>,
    key: &str,
) -> Result<Option<u32>, QuotaError> {
    match fields.get(key) {
        Some(CanonicalJsonValue::Null) => Ok(None),
        Some(CanonicalJsonValue::Integer(count)) => {
            u32::try_from(*count).map(Some).map_err(|_| QuotaError::CeilingOutOfRange)
        }
        Some(_) => Err(QuotaError::InvalidField),
        None => Err(QuotaError::MissingField),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUEST_UID: &str = "1b4e28ba-2fa1-41d2-883f-0016d3cca427";
    const ROLE_UID: &str = "2b4e28ba-2fa1-41d2-883f-0016d3cca427";
    const VOLUME_UID: &str = "3b4e28ba-2fa1-41d2-883f-0016d3cca427";

    fn type_of(value: &str) -> ResourceTypeName {
        ResourceTypeName::parse(value).expect("a registered resource type")
    }

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).expect("a canonical uuid")
    }

    fn reference(value: &str) -> ResourceRef {
        ResourceRef::parse(value).expect("a canonical reference")
    }

    /// A Zone with the given ceilings.
    fn zone(per_type: u32, max_resources: u32, max_owner_depth: u32) -> QuotaPolicy {
        QuotaPolicy::new(
            QuotaCeilings::new(max_resources, per_type, max_owner_depth, None, None, None)
                .expect("ceilings are in range"),
            BTreeMap::new(),
            QuotaEnforcementPolicy::Hard,
        )
        .expect("the policy validates")
    }

    /// A committed `Guest` and a `Role` it owns.
    fn census() -> ZoneUsage {
        ZoneUsage::census(&[
            (reference("Guest/vm"), uid(GUEST_UID), None),
            (reference("Role/operator"), uid(ROLE_UID), Some(uid(GUEST_UID))),
        ])
        .expect("the committed rows form a decidable census")
    }

    #[test]
    fn a_candidate_past_a_count_ceiling_is_refused_and_one_inside_it_is_admitted() {
        let policy = zone(1, 3, 4);
        let usage = census();
        assert_eq!(
            admit(&policy, &usage, &QuotaRequest::adds(reference("Volume/data"), 1, ZoneBudget::ZERO)),
            AdmissionDecision::Admitted,
            "a third row of a type the Zone admits is inside every ceiling"
        );
        assert_eq!(
            admit(&policy, &usage, &QuotaRequest::adds(reference("Guest/second"), 1, ZoneBudget::ZERO)),
            AdmissionDecision::refuse(AdmissionStage::Admit, RefusalReason::LimitExceedsCeiling),
            "the per-type ceiling of one is reached before the Zone-wide three"
        );
        let narrow = zone(1, 2, 4);
        assert_eq!(
            admit(&narrow, &usage, &QuotaRequest::adds(reference("Volume/data"), 1, ZoneBudget::ZERO)),
            AdmissionDecision::refuse(AdmissionStage::Admit, RefusalReason::LimitExceedsCeiling),
            "two committed rows exhaust a Zone-wide ceiling of two"
        );
    }

    #[test]
    fn a_per_type_ceiling_overrides_the_zone_wide_one_and_a_soft_quota_refuses_nothing() {
        let hard = QuotaPolicy::new(
            QuotaCeilings::new(8, 8, 4, None, None, None).expect("ceilings are in range"),
            BTreeMap::from([(type_of("Guest"), 1)]),
            QuotaEnforcementPolicy::Hard,
        )
        .expect("the policy validates");
        let usage = census();
        assert_eq!(hard.type_ceiling(&type_of("Guest")), 1);
        let over = QuotaRequest::adds(reference("Guest/second"), 1, ZoneBudget::ZERO);
        assert_eq!(
            admit(&hard, &usage, &over),
            AdmissionDecision::refuse(AdmissionStage::Admit, RefusalReason::LimitExceedsCeiling)
        );
        assert_eq!(over_quota(&hard, &usage, Some(&over)).1, vec![type_of("Guest")]);
        let soft = QuotaPolicy::new(
            *hard.ceilings(),
            hard.per_type_ceilings().clone(),
            QuotaEnforcementPolicy::Soft,
        )
        .expect("the policy validates");
        assert_eq!(
            admit(&soft, &usage, &over),
            AdmissionDecision::Admitted,
            "a soft quota reports the excess instead of refusing the candidate"
        );
        assert!(
            over_quota(&soft, &usage, Some(&over)).0,
            "the same excess is still published on the status"
        );
    }

    #[test]
    fn removing_use_is_never_refused_so_a_zone_can_always_recover() {
        let policy = zone(1, 2, 1);
        let usage = census();
        assert_eq!(
            admit(&policy, &usage, &QuotaRequest::reduces(reference("Guest/vm"))),
            AdmissionDecision::Admitted
        );
        assert_eq!(
            admit(&policy, &usage, &QuotaRequest::neutral(reference("Guest/vm"))),
            AdmissionDecision::Admitted
        );
        assert_eq!(
            admit(&policy, &usage, &QuotaRequest::adds(reference("Guest/second"), 1, ZoneBudget::ZERO)),
            AdmissionDecision::refuse(AdmissionStage::Admit, RefusalReason::LimitExceedsCeiling),
            "the same Zone still refuses growth, so the recovery is a delete"
        );
    }

    #[test]
    fn a_policy_never_measures_its_own_type() {
        let policy = zone(1, 1, 1).bind_to(reference("Quota/zone"));
        let usage = census();
        for ceiling in [reference("Quota/zone"), reference("Quota/tighter")] {
            assert_eq!(
                admit(&policy, &usage, &QuotaRequest::adds(ceiling, 1, ZoneBudget::ZERO)),
                AdmissionDecision::Admitted,
                "the rows that state the limit are never refused by it"
            );
        }
        assert_eq!(
            admit(&policy, &usage, &QuotaRequest::adds(reference("Guest/second"), 1, ZoneBudget::ZERO)),
            AdmissionDecision::refuse(AdmissionStage::Admit, RefusalReason::LimitExceedsCeiling)
        );
    }

    #[test]
    fn a_declared_budget_is_refused_at_the_effect_boundary_and_a_silent_one_is_not() {
        let policy = QuotaPolicy::new(
            QuotaCeilings::new(8, 8, 4, Some(1), None, None).expect("ceilings are in range"),
            BTreeMap::new(),
            QuotaEnforcementPolicy::Hard,
        )
        .expect("the policy validates");
        let usage = ZoneUsage::census(&[(reference("Guest/vm"), uid(GUEST_UID), None)])
            .expect("the census is decidable")
            .with_budget(ZoneBudget { cpu: 1, memory_mib: 0, storage_gib: 0 })
            .expect("the budget is representable");
        assert_eq!(
            admit_budget(
                &policy,
                &usage,
                &reference("Guest/second"),
                ZoneBudget { cpu: 1, memory_mib: 0, storage_gib: 0 },
                1
            ),
            AdmissionDecision::refuse(AdmissionStage::Admit, RefusalReason::LimitExceedsCeiling)
        );
        assert_eq!(
            admit_budget(&policy, &usage, &reference("Guest/second"), ZoneBudget::ZERO, 1),
            AdmissionDecision::Admitted
        );
        assert_eq!(
            usage.budget().checked_add(ZoneBudget { cpu: u32::MAX, memory_mib: 1, storage_gib: 0 }),
            None,
            "an unrepresentable total is never wrapped into an admitted one"
        );
    }

    #[test]
    fn the_owner_chain_a_candidate_extends_is_measured_and_an_unreadable_one_is_refused() {
        let policy = zone(8, 8, 2);
        let usage = census();
        assert_eq!(usage.owner_depth(), 1);
        assert_eq!(usage.depth_with_owner(None).expect("a root is decidable"), 1);
        assert_eq!(
            usage
                .depth_with_owner(Some(&reference("Role/operator")))
                .expect("the committed chain resolves"),
            2
        );
        assert_eq!(
            usage.depth_with_owner(Some(&reference("Role/absent"))),
            Err(QuotaError::UnresolvedOwner)
        );
        assert_eq!(
            admit(
                &policy,
                &usage,
                &QuotaRequest::adds(reference("Role/deeper"), 3, ZoneBudget::ZERO)
            ),
            AdmissionDecision::refuse(AdmissionStage::Admit, RefusalReason::LimitExceedsCeiling)
        );
    }

    #[test]
    fn a_cycle_in_the_committed_owner_column_is_refused_rather_than_counted_as_a_root() {
        let rows = [
            (reference("Guest/vm"), uid(GUEST_UID), Some(uid(ROLE_UID))),
            (reference("Role/operator"), uid(ROLE_UID), Some(uid(GUEST_UID))),
        ];
        assert_eq!(ZoneUsage::census(&rows), Err(QuotaError::OwnerCycle));
    }

    #[test]
    fn the_metadata_envelope_names_the_owner_or_says_it_is_a_root() {
        assert_eq!(owner_of_metadata(br#"{"ownerRef":null}"#), Ok(None));
        assert_eq!(owner_of_metadata(br#"{}"#), Ok(None));
        assert_eq!(owner_of_metadata(&[]), Ok(None));
        assert_eq!(
            owner_of_metadata(br#"{"ownerRef":"Guest/vm"}"#),
            Ok(Some(reference("Guest/vm")))
        );
        assert_eq!(owner_of_metadata(br#"{"ownerRef":7}"#), Err(QuotaError::InvalidField));
        assert_eq!(
            owner_of_metadata(br#"{"ownerRef":"not a ref"}"#),
            Err(QuotaError::InvalidField)
        );
        assert_eq!(owner_of_metadata(b"[]"), Err(QuotaError::UnreadableDesiredState));
    }

    /// The canonical desired-state object a hard Quota row stores.
    const HARD_ROW: &[u8] = br#"{"ceilings":{"maxCpu":8,"maxMemoryMib":4096,"maxOwnerDepth":3,"maxResources":16,"maxResourcesPerType":4,"maxStorageGib":null},"enforcementPolicy":"hard","perTypeCeilings":{"Guest":{"maxResources":2}},"providerRef":"Provider/system-core","scope":"zone"}"#;

    #[test]
    fn an_accepted_row_decodes_into_exactly_the_ceilings_it_declares() {
        let policy = QuotaPolicy::decode(HARD_ROW).expect("the row decodes");
        assert_eq!(policy.enforcement(), QuotaEnforcementPolicy::Hard);
        assert_eq!(policy.ceilings().max_resources(), 16);
        assert_eq!(policy.ceilings().max_resources_per_type(), 4);
        assert_eq!(policy.ceilings().max_owner_depth(), 3);
        assert_eq!(policy.ceilings().max_cpu(), Some(8));
        assert_eq!(policy.ceilings().max_memory_mib(), Some(4096));
        assert_eq!(policy.ceilings().max_storage_gib(), None);
        assert_eq!(policy.type_ceiling(&type_of("Guest")), 2);
        assert_eq!(policy.type_ceiling(&type_of("Volume")), 4);
    }

    #[test]
    fn a_ceiling_this_decoder_cannot_read_is_refused_rather_than_admitted_without_it() {
        assert_eq!(
            QuotaPolicy::decode(
                br#"{"ceilings":{"maxCpu":null,"maxMemoryMib":null,"maxOwnerDepth":3,"maxResources":16,"maxResourcesPerType":4,"maxStorageGib":null},"enforcementPolicy":"hard","maxBandwidth":1,"perTypeCeilings":{},"scope":"zone"}"#
            ),
            Err(QuotaError::UnknownField)
        );
        assert_eq!(
            QuotaPolicy::decode(
                br#"{"ceilings":{"maxCpu":null,"maxMemoryMib":null,"maxOwnerDepth":3,"maxResources":16,"maxResourcesPerType":4},"enforcementPolicy":"hard","perTypeCeilings":{},"scope":"zone"}"#
            ),
            Err(QuotaError::MissingField)
        );
        assert_eq!(
            QuotaPolicy::decode(
                br#"{"ceilings":{"maxCpu":null,"maxMemoryMib":null,"maxOwnerDepth":3,"maxResources":0,"maxResourcesPerType":4,"maxStorageGib":null},"enforcementPolicy":"hard","perTypeCeilings":{},"scope":"zone"}"#
            ),
            Err(QuotaError::CeilingOutOfRange)
        );
        assert_eq!(
            QuotaPolicy::decode(
                br#"{"ceilings":{"maxCpu":null,"maxMemoryMib":null,"maxOwnerDepth":3,"maxResources":16,"maxResourcesPerType":4,"maxStorageGib":null},"enforcementPolicy":"soft","perTypeCeilings":{},"scope":"host"}"#
            ),
            Err(QuotaError::InvalidField)
        );
        assert_eq!(
            QuotaPolicy::decode(
                br#"{"ceilings":{"maxCpu":null,"maxMemoryMib":null,"maxOwnerDepth":3,"maxResources":16,"maxResourcesPerType":4,"maxStorageGib":null},"enforcementPolicy":"hard","perTypeCeilings":{"Guest":{"maxGuests":2}},"scope":"zone"}"#
            ),
            Err(QuotaError::UnknownField)
        );
        assert_eq!(QuotaPolicy::decode(b"[]"), Err(QuotaError::UnreadableDesiredState));
    }

    #[test]
    fn the_status_reports_the_committed_census_and_only_meters_a_declared_dimension() {
        let policy = QuotaPolicy::decode(HARD_ROW).expect("the row decodes");
        let usage = ZoneUsage::census(&[(reference("Guest/vm"), uid(GUEST_UID), None)])
            .expect("the census is decidable")
            .with_budget(ZoneBudget { cpu: 2, memory_mib: 0, storage_gib: 0 })
            .expect("the budget is representable");
        let status = QuotaStatusResource::of(
            &policy,
            &usage,
            Some(&QuotaRequest::adds(reference("Guest/second"), 1, ZoneBudget::ZERO)),
            1,
            Some(Timestamp::parse("2026-09-30T12:00:00.000Z").expect("a canonical timestamp")),
        );
        assert_eq!(status.used_resources(), 1);
        assert_eq!(status.used_cpu, Some(2));
        assert_eq!(status.used_memory_mib, Some(0));
        assert_eq!(status.used_storage_gib, None, "an unmetered dimension is absent, not zero");
        assert!(!status.over_quota(), "one row of a two-row Guest ceiling is inside it");
        assert_eq!(status.dependent_count, 1);

        let over = ZoneUsage::census(&[
            (reference("Guest/vm"), uid(GUEST_UID), None),
            (reference("Guest/second"), uid(ROLE_UID), None),
            (reference("Guest/third"), uid(VOLUME_UID), None),
        ])
        .expect("the census is decidable");
        let reported = QuotaStatusResource::of(&policy, &over, None, 0, None);
        assert!(
            reported.over_quota(),
            "a Zone already past its per-type ceiling says so without a pending candidate"
        );
    }
}
