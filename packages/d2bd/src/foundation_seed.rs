//! The foundation seed: the committed policy rows of the system zone.
//!
//! The seed is the first policy publication. It commits, in the order the
//! resource plane depends on: the system zone itself, the declared provider
//! identities, the declared posture rows, roles, and commands, the provider
//! self-bindings, the spawn operations the process controller materializes
//! from those commands, and the operator bindings the host contract carries.
//!
//! Resolution is declare-then-validate: every declared row is collected
//! first - including the operations materialized from the commands - and the
//! reference check then runs over that committed set as a whole. Nothing is
//! written until the whole set validates, so the
//! Command-to-Role-to-Operation cycle resolves without an ordering hack and a
//! refused seed leaves the store exactly as it was.
//!
//! The seed owns the system zone's rows: a zone-local plane refuses a write
//! to a system-homed type with the same terminal,
//! named refusal shape the plane partition uses.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts_resource::v3::{
    AdmissionStage, AuthoritySubject, AuthoritySubjectKind, ExecutionPolicySpec,
    RefusalReason, ResourceRef, StoreIncarnation, ZoneId, canonical_json_bytes,
    execution_policy_resource::EXECUTION_POLICY_RESOURCE_TYPE,
};
use d2b_contracts_resource::v3::AdmissionDecision as GraphAdmissionDecision;
use d2b_core::resource_authority::{
    AcceptedGraph, GraphAuthority, GraphMutation, MutationKind, MutationSubjectEvidence,
    TransportIdentity,
};
use d2b_contracts_broker::broker_wire::{
    AuthorityCursor, AuthorityProjectionRow, AuthoritySnapshot, publication_snapshot_digest,
};
use d2b_contracts_resource::v3::{
    CanonicalJsonObject, CanonicalJsonValue, DesiredDigest, DesiredRevision, ResourceUid,
};
use d2b_core::resource_authority::ProjectionRow;
use d2b_core::execution_plan::binding_row_ref;
use d2b_provider_seccomp_profile::{ SECCOMP_PROFILE_RESOURCE_TYPE, SeccompProfileSpec };
use d2b_contracts_zone_session::v3::{RoleBindingSpec, RoleResourceVerb, role::AuthorizedRole};
use d2b_resource_runtime::identity::ResourceTypeName;
use d2b_resource_runtime::manager::{
    AdmissionDecision, MutationAdmission, MutationRequest, MutationSubject, deterministic_uid,
};
use d2b_resource_runtime::provider::ProviderDirectory;
use d2b_resource_runtime::spec_store::{
    EnsureOutcome, ResourceKey, ResourceProvenance, SpecSelector, SpecStore, StoredDesiredResource,
};
use crate::resource_plane_v3::GraphMutationAdmission;

use crate::principal_allocation::{PrincipalAllocation, valid_principal_name};

/// The reserved zone the foundation rows are homed in.
///
/// The durable authority's home: the foundation plane commits these rows
/// into its own store under the reserved zone name, and no zone-local plane
/// may ever carry them itself. The name itself is declared once, in the
/// identity vocabulary the readers select.
pub const SYSTEM_ZONE: &str = d2b_contracts::identity::SYSTEM_ZONE_NAME;
/// The system-homed policy types: rows only the foundation seed writes.
///
/// The zone-control vocabulary (Zone, ZoneLink, Provider, Role, RoleBinding,
/// Quota, EmergencyPolicy) still travels in each zone's compiled bundle and
/// joins this set type by type as its rows move onto the seed.
pub const SYSTEM_HOMED_TYPES: &[&str] = &["Operation", "SeccompProfile", "ExecutionPolicy"];
/// Maximum bytes of one seeded resource name.
pub const MAX_SEED_NAME_BYTES: usize = 63;
/// The subject types a RoleBinding may grant, resolved by the session layer.
///
/// Re-exported from the contract rather than restated. This module used to
/// keep its own copy of the closed list, which drifted from the contract's: it
/// carried a `Group` entry the contract did not, so the seed could commit a
/// RoleBinding row that the contract decoder would then refuse. Re-exporting
/// makes that class of divergence impossible rather than merely fixed once.
pub use d2b_contracts_zone_session::v3::BINDABLE_SUBJECT_TYPES;
/// The resource types an execution selector may name.
pub const EXECUTION_SUBJECT_TYPES: &[&str] = &["Host", "Guest", "EphemeralProcess"];
/// The default metadata envelope every seeded row carries.
const SEED_METADATA: &[u8] = br#"{"annotations":{},"labels":{},"ownerRef":null}"#;

/// One write a zone-local plane refuses because the row is system-homed.
///
/// Only the foundation plane - the plane the composition opened with the
/// foundation seed - may carry a system-homed row; every other plane refuses
/// the write terminally, naming the type and the caller, the same shape
/// [`d2b_contracts::identity::WrongPlane`] uses for the manager/legacy split.
///
/// This is the plane's write fence while the manager-boundary graph admission
/// waits for a per-Zone accepted graph to exist. The quota and emergency
/// enforcement does not depend on it: both families publish into the limits
/// holder their own drivers read, and the admission that would consult that
/// holder is the piece that is not installed yet.
pub struct SystemZoneWriteFence {
    foundation: bool,
}

impl SystemZoneWriteFence {
    /// Fence one plane's writes: `foundation` marks the plane the seed runs on.
    pub const fn new(foundation: bool) -> Self {
        Self { foundation }
    }
}

impl MutationAdmission for SystemZoneWriteFence {
    fn admit(&self, subject: &MutationSubject, request: &MutationRequest) -> AdmissionDecision {
        if self.foundation || !SYSTEM_HOMED_TYPES.contains(&request.key.type_name.as_str()) {
            return AdmissionDecision::Allow;
        }
        AdmissionDecision::Deny(format!(
            "wrong plane: {} is committed by the foundation seed, not by the zone-local plane \
             (caller: {})",
            request.key.type_name, subject.principal
        ))
    }
}

/// One declared provider identity the seed resolves references against.
///
/// Provider identity publication itself is the plane's KTD7 seed; the
/// foundation seed only needs the identity and the principals it declares, so
/// the identity enters the committed set without a `Provider` row (whose spec
/// the zone bundle owns).
#[derive(Debug, Clone)]
pub struct SeedProvider {
    /// The provider identity.
    pub provider_ref: ResourceRef,
    /// Host principal names this provider's workers run as (name only; the
    /// numbers come from the committed allocation).
    pub principals: Vec<String>,
    /// The roles this provider declares for itself.
    pub roles: Vec<ResourceRef>,
    /// The provider's structurally scoped self-bindings.
    pub self_bindings: Vec<SeedSelfBinding>,
}

/// One self-binding: the declaring provider bound to a role it declares.
#[derive(Debug, Clone)]
pub struct SeedSelfBinding {
    /// The binding subject; must be the declaring provider.
    pub subject_ref: ResourceRef,
    /// The role the subject is bound to; must be one the provider declared.
    pub role_ref: ResourceRef,
}

/// One declared Role row.
#[derive(Debug, Clone)]
pub struct SeedRole {
    /// Zone-local role name.
    pub name: String,
    /// The authorization-only role spec: rules and declared operations.
    pub spec: AuthorizedRole,
}

/// One declared SeccompProfile row.
#[derive(Debug, Clone)]
pub struct SeedProfile {
    /// Zone-local profile name.
    pub name: String,
    /// The inline posture content.
    pub spec: SeccompProfileSpec,
}


/// One declared ExecutionPolicy row.
///
/// An `ExecutionPolicy` row states reusable confinement: the isolation
/// classes an execution instance must run behind, the capability ceiling,
/// the restrictions it may not weaken, the identity it may resolve to, and
/// the syscall filter it must load. It grants no storage, device, network,
/// endpoint, or credential access, so a policy the seed commits carries no
/// attachment authority of its own.
#[derive(Debug, Clone)]
pub struct SeedPolicy {
    /// Zone-local policy name.
    pub name: String,
    /// The declared confinement.
    pub spec: ExecutionPolicySpec,
}
/// One declared RoleBinding row.
#[derive(Debug, Clone)]
pub struct SeedBinding {
    /// Zone-local binding name.
    pub name: String,
    /// The binding spec.
    pub spec: RoleBindingSpec,
}

/// The rows one seed run commits, as declared by the composition root.
#[derive(Debug, Clone, Default)]
pub struct FoundationDeclarations {
    /// Declared provider identities.
    pub providers: Vec<SeedProvider>,
    /// Declared posture rows.
    pub profiles: Vec<SeedProfile>,
    /// Declared reusable-confinement rows.
    pub policies: Vec<SeedPolicy>,
    /// Declared roles.
    pub roles: Vec<SeedRole>,
    /// Operator bindings from the host contract (provenance `nix`).
    pub operator_bindings: Vec<SeedBinding>,
    /// The process controller whose self-binding authorizes materialization.
    pub controller: Option<ResourceRef>,
}

/// The core built-in vocabulary the composition root declares.
///
/// The system vocabulary this unit owns: the fixed process provider (whose
/// self-binding authorizes the process controller's materialization), the
/// `operation-publisher` role scoped to the declared commands, and the one
/// host principal the zone store is owned by. Families declare their own
/// commands, postures, and worker roles from their own crates as they land;
/// this set grows by declaration, never by a table in shared code.
pub fn core_declarations() -> FoundationDeclarations {
    use d2b_contracts_resource::v3::BoundedText;
    use d2b_contracts_zone_session::v3::RoleRule;

    let provider_ref = ResourceRef::parse(d2b_provider_process_minijail::PROVIDER_REF)
        .expect("the process provider reference is canonical");
    let role_ref = ResourceRef::parse("Role/operation-publisher")
        .expect("the publisher role reference is canonical");
    let rule = RoleRule::new(
        vec![
            d2b_contracts_resource::v3::ResourceTypeName::parse("Operation")
                .expect("the Operation type name is canonical"),
        ],
        vec![RoleResourceVerb::Create],
        vec![BoundedText::parse("create").expect("static selector")],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("the publisher rule is valid");
    let role = AuthorizedRole::new(vec![rule], Vec::new()).expect("the publisher role is valid");
    FoundationDeclarations {
        providers: vec![SeedProvider {
            provider_ref: provider_ref.clone(),
            principals: vec!["d2b-zonert".to_owned()],
            roles: vec![role_ref.clone()],
            self_bindings: vec![SeedSelfBinding {
                subject_ref: provider_ref.clone(),
                role_ref: role_ref.clone(),
            }],
        }],
        profiles: Vec::new(),
        // No core confinement is declared yet. The vocabulary grows by
        // declaration from the family that owns the posture it converts,
        // never by a table here; the field is the path those families
        // declare through.
        policies: Vec::new(),
        roles: vec![SeedRole {
            name: "operation-publisher".to_owned(),
            spec: role,
        }],
        operator_bindings: Vec::new(),
        controller: Some(provider_ref),
    }
}

/// What one seed run committed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SeedReport {
    /// Every committed reference, in write order.
    pub committed: Vec<String>,
    /// The operation rows materialized from commands.
    pub materialized: Vec<String>,
    /// Rows whose committed bytes were already current.
    pub unchanged: usize,
}

/// The foundation seed.
pub struct FoundationSeed {
    declarations: FoundationDeclarations,
    allocation: PrincipalAllocation,
}

impl FoundationSeed {
    /// Assemble one seed run.
    pub fn new(
        declarations: FoundationDeclarations,
        allocation: PrincipalAllocation,
    ) -> Self {
        Self {
            declarations,
            allocation,
        }
    }

    /// Commit the declared policy rows into the system zone's store.
    ///
    /// The registry supplies the declared verb set each Role rule is
    /// constrained to. The store is written only after every declared row
    /// validates, so a refusal is atomic.
    pub async fn run(
        &self,
        store: &SpecStore,
        providers: &ProviderDirectory,
    ) -> Result<SeedReport, SeedError> {
        // Durable rows from earlier boots resolve references exactly as the
        // declared set does: a restart revalidates against everything.
        let mut committed = CommittedSet::default();
        for row in store
            .list(SpecSelector {
                zone: Some(SYSTEM_ZONE.to_owned()),
                type_name: None,
                owner_uid: None,
            })
            .await
            .map_err(|error| SeedError::Store(error.to_string()))?
        {
            if !row.deleting {
                committed.insert_row(&row.key, row.spec.clone());
            }
        }
        let mut rows: Vec<PendingRow> = Vec::new();
        // 0. The system zone itself.
        let zone_row = PendingRow::new("Zone", SYSTEM_ZONE, b"{}".to_vec())?;
        committed.insert_row(&zone_row.key, zone_row.spec.clone());
        rows.push(zone_row);
        // 1. Provider identities: resolvable refs, not rows.
        for provider in &self.declarations.providers {
            committed.insert_ref(&provider.provider_ref);
        }
        // 2. Posture rows, then roles (a role's posture points at a profile).
        for profile in &self.declarations.profiles {
            let row = PendingRow::new(
                SECCOMP_PROFILE_RESOURCE_TYPE,
                &profile.name,
                encode(&profile.spec)?,
            )?;
            committed.insert_row(&row.key, row.spec.clone());
            rows.push(row);
        }
        // 2b. Reusable confinement. A policy selects a `SeccompProfile` and
        // optionally a `User`, so it is collected after the posture rows and
        // before the roles that may select it; the reference check below
        // then resolves both over the committed set as a whole.
        for policy in &self.declarations.policies {
            let row = PendingRow::new(
                EXECUTION_POLICY_RESOURCE_TYPE,
                &policy.name,
                encode(&policy.spec)?,
            )?;
            committed.insert_row(&row.key, row.spec.clone());
            rows.push(row);
        }
        for role in &self.declarations.roles {
            let row = PendingRow::new("Role", &role.name, encode(&role.spec)?)?;
            committed.insert_row(&row.key, row.spec.clone());
            rows.push(row);
        }
        // 4. Self-bindings, framework-generated in declaration order.
        for provider in &self.declarations.providers {
            for binding in &provider.self_bindings {
                self.check_self_binding_scope(provider, binding)?;
                let name = self_binding_name(provider, binding)?;
                let spec = self_binding_spec(provider, binding)?;
                let row = PendingRow::new("RoleBinding", &name, encode(&spec)?)?;
                committed.insert_row(&row.key, row.spec.clone());
                rows.push(row);
            }
        }
        // 6. Operator bindings from the host contract.
        for binding in &self.declarations.operator_bindings {
            let row = PendingRow::new("RoleBinding", &binding.name, encode(&binding.spec)?)?;
            committed.insert_row(&row.key, row.spec.clone());
            rows.push(row);
        }
        // Declare-then-validate: every reference resolves over the committed
        // set as a whole, before the first write.
        self.validate(&committed, providers, &rows)?;
        // Every seeded row is admitted through the one evaluator before the
        // first write, against the verified deployment graph itself. There
        // is no bootstrap-operation allowlist to extend: the seed's authority
        // is the deployment root identity, and a row committed under any
        // other subject is refused with its stage and reason.
        self.admit_verified(&rows)?;
        let mut report = SeedReport {
            committed: Vec::with_capacity(rows.len()),
            materialized: Vec::new(),
            unchanged: 0,
        };
        for row in &rows {
            match self.write(store, row).await? {
                EnsureOutcome::Unchanged(_) => report.unchanged += 1,
                EnsureOutcome::Created(_) | EnsureOutcome::Updated(_) => {}
            }
            report.committed.push(row.reference());
        }
        Ok(report)
    }

    async fn write(&self, store: &SpecStore, row: &PendingRow) -> Result<EnsureOutcome, SeedError> {
        let stored = StoredDesiredResource {
            key: row.key.clone(),
            uid: deterministic_uid(&row.key),
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Nix,
            deleting: false,
            spec: row.spec.clone(),
            metadata: SEED_METADATA.to_vec(),
            created_at: 0,
        };
        store
            .ensure(stored)
            .await
            .map_err(|error| SeedError::Store(error.to_string()))
    }

    // -- Verified deployment graph ----------------------------------------

    /// Admit every seeded row through the one evaluator, as the verified
    /// deployment graph's own mutation.
    ///
    /// The seed is the initial desired graph, so it is admitted rather than
    /// waved through: each row is presented as a `Create` mutation whose
    /// initiating subject is the deployment root the verified graph
    /// established. A row admitted by anything else - a bootstrap operation
    /// list, a transport identity, a provider's self-binding - is refused
    /// with the stage and reason the evaluator named, and nothing is written.
    fn admit_verified(&self, rows: &[PendingRow]) -> Result<(), SeedError> {
        let accepted = verified_deployment_graph()?;
        let evidence = MutationSubjectEvidence::new(
            AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
            TransportIdentity::Daemon,
        );
        for row in rows {
            let target = ResourceRef::parse(row.reference().as_str()).map_err(|_| {
                SeedError::InvalidRow {
                    row: static_type(row.key.type_name.as_str()),
                    name: row.key.name.clone(),
                    reason: "the row reference is not a resource reference",
                }
            })?;
            let request = GraphMutation::new(
                accepted.zone().clone(),
                evidence.clone(),
                MutationKind::Create,
                target,
            );
            if let GraphAdmissionDecision::Refused { stage, reason } =
                GraphAuthority::admit_mutation(&request, &accepted)
            {
                return Err(SeedError::GraphRefused {
                    row: row.reference(),
                    stage,
                    reason,
                });
            }
        }
        Ok(())
    }

    /// Every declared binding row (self-bindings included) as (name, spec).
    fn binding_rows(&self) -> Vec<(String, RoleBindingSpec)> {
        let mut rows = self.self_binding_rows();
        for binding in &self.declarations.operator_bindings {
            rows.push((binding.name.clone(), binding.spec.clone()));
        }
        rows
    }

    /// Every declared provider self-binding row as (name, spec).
    fn self_binding_rows(&self) -> Vec<(String, RoleBindingSpec)> {
        let mut rows = Vec::new();
        for provider in &self.declarations.providers {
            for binding in &provider.self_bindings {
                if let (Ok(name), Ok(spec)) =
                    (self_binding_name(provider, binding), self_binding_spec(provider, binding))
                {
                    rows.push((name, spec));
                }
            }
        }
        rows
    }

    // -- Validation --------------------------------------------------------

    fn check_self_binding_scope(
        &self,
        provider: &SeedProvider,
        binding: &SeedSelfBinding,
    ) -> Result<(), SeedError> {
        let scope_ok = binding.subject_ref == provider.provider_ref
            && provider.roles.contains(&binding.role_ref)
            && self
                .declarations
                .roles
                .iter()
                .any(|role| role_ref(&role.name) == binding.role_ref.to_canonical_string());
        if scope_ok {
            Ok(())
        } else {
            Err(SeedError::SelfBindingEscaped {
                provider: provider.provider_ref.to_canonical_string(),
                subject: binding.subject_ref.to_canonical_string(),
                role: binding.role_ref.to_canonical_string(),
            })
        }
    }

    fn validate(
        &self,
        committed: &CommittedSet,
        providers: &ProviderDirectory,
        rows: &[PendingRow],
    ) -> Result<(), SeedError> {
        // A policy's references resolve over the same committed set as every
        // other seeded row, so a policy selecting a profile or an identity
        // the seed does not commit is refused before the first write rather
        // than landing as a row whose selection can never resolve.
        for policy in &self.declarations.policies {
            let row = format!("{}/{}", EXECUTION_POLICY_RESOURCE_TYPE, policy.name);
            if let Some(profile) = policy.spec.seccomp().profile_ref() {
                require_committed(committed, &row, "seccomp.profileRef", profile)?;
            }
            if let Some(identity) = policy.spec.identity().user_ref() {
                require_committed(committed, &row, "identity.userRef", identity)?;
            }
        }
        for role in &self.declarations.roles {
            let row = role_ref(&role.name);
            for rule in role.spec.rules() {
                for type_name in rule.resource_types() {
                    let declared = providers
                        .declared_verbs(&ResourceTypeName::new(type_name.as_str()))
                        .ok_or_else(|| SeedError::UnknownResourceType {
                            row: row.clone(),
                            type_name: type_name.as_str().to_owned(),
                        })?;
                    for verb in rule.verbs() {
                        let spelling = verb_spelling(*verb);
                        if !declared.iter().any(|declared| declared == spelling) {
                            return Err(SeedError::UndeclaredVerb {
                                row: row.clone(),
                                type_name: type_name.as_str().to_owned(),
                                verb: spelling.to_owned(),
                            });
                        }
                    }
                }
            }
            for operation in role.spec.operation_refs() {
                require_committed(committed, &row, "operationRefs", operation)?;
            }
        }
        // Every declared principal resolves through the committed allocation.
        for provider in &self.declarations.providers {
            for principal in &provider.principals {
                if !valid_principal_name(principal) || self.allocation.get(principal).is_none() {
                    return Err(SeedError::PrincipalNotAllocated {
                        row: provider.provider_ref.to_canonical_string(),
                        principal: principal.clone(),
                    });
                }
            }
        }
        for (name, spec) in self.binding_rows() {
            let row = format!("RoleBinding/{name}");
            require_committed(committed, &row, "roleRef", spec.role_ref())?;
            let role = self
                .declarations
                .roles
                .iter()
                .find(|role| role_ref(&role.name) == spec.role_ref().to_canonical_string());
            let rule_types: BTreeSet<String> = role
                .map(|role| {
                    role.spec
                        .rules()
                        .iter()
                        .flat_map(|rule| rule.resource_types())
                        .map(|type_name| type_name.as_str().to_owned())
                        .collect()
                })
                .unwrap_or_default();
            for subject in spec.subjects() {
                let subject_type = subject.resource_type().as_str();
                if !BINDABLE_SUBJECT_TYPES.contains(&subject_type) {
                    return Err(SeedError::UnbindableSubject {
                        row: row.clone(),
                        subject: subject.to_canonical_string(),
                    });
                }
                if subject_type == "Provider" {
                    require_committed(committed, &row, "subjects", subject)?;
                }
            }
            for resource in spec.resource_refs() {
                if !rule_types.contains(resource.resource_type().as_str()) {
                    return Err(SeedError::ScopeOutsideRole {
                        row: row.clone(),
                        field: "resourceRefs",
                        reference: resource.to_canonical_string(),
                    });
                }
            }
            for execution in spec.execution_refs() {
                if !EXECUTION_SUBJECT_TYPES.contains(&execution.resource_type().as_str()) {
                    return Err(SeedError::ScopeOutsideRole {
                        row: row.clone(),
                        field: "executionRefs",
                        reference: execution.to_canonical_string(),
                    });
                }
            }
        }
        // Every declared row is written exactly once.
        let mut seen = BTreeSet::new();
        for row in rows {
            if !seen.insert(row.reference()) {
                return Err(SeedError::DuplicateRow(row.reference()));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Committed set and row shapes
// ---------------------------------------------------------------------------

#[derive(Default)]
struct CommittedSet {
    specs: BTreeMap<String, Vec<u8>>,
}

impl CommittedSet {
    fn insert_row(&mut self, key: &ResourceKey, spec: Vec<u8>) {
        self.specs
            .insert(format!("{}/{}", key.type_name, key.name), spec);
    }

    fn insert_ref(&mut self, reference: &ResourceRef) {
        self.specs
            .entry(reference.to_canonical_string())
            .or_default();
    }

    fn contains(&self, reference: &ResourceRef) -> bool {
        self.specs.contains_key(&reference.to_canonical_string())
    }

}

struct PendingRow {
    key: ResourceKey,
    spec: Vec<u8>,
}

impl PendingRow {
    fn new(resource_type: &str, name: &str, spec: Vec<u8>) -> Result<Self, SeedError> {
        if !valid_resource_name(name) {
            return Err(SeedError::InvalidRow {
                row: static_type(resource_type),
                name: name.to_owned(),
                reason: "the row name is not a resource name",
            });
        }
        Ok(Self {
            key: ResourceKey::new(SYSTEM_ZONE, resource_type, name),
            spec,
        })
    }

    fn reference(&self) -> String {
        format!("{}/{}", self.key.type_name, self.key.name)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The store incarnation the bootstrap graph is seeded in.
///
/// The seed opens a store that has no authority journal yet, so its own
/// generation is the one the deployment root is admitted in. The U34
/// cutover reads the live journal incarnation instead.
const FOUNDATION_STORE: &str = "foundation-1";

/// The prior accepted graph the seed's rows are admitted against.
///
/// It carries the deployment root and nothing else: an empty authorization
/// set, so no row can be admitted by a grant that is itself being seeded,
/// and no binding source, so no row can create a relationship.
fn verified_deployment_graph() -> Result<AcceptedGraph, SeedError> {
    let zone = ZoneId::parse(SYSTEM_ZONE)
        .map_err(|_| SeedError::GraphRefused {
            row: SYSTEM_ZONE.to_owned(),
            stage: AdmissionStage::Authorize,
            reason: RefusalReason::StoreIncarnationMismatch,
        })?;
    let store = StoreIncarnation::parse(FOUNDATION_STORE).map_err(|_| SeedError::GraphRefused {
        row: SYSTEM_ZONE.to_owned(),
        stage: AdmissionStage::Authorize,
        reason: RefusalReason::StoreIncarnationMismatch,
    })?;
    Ok(AcceptedGraph::new(
        zone,
        store,
        AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
    ))
}

fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, SeedError> {
    canonical_json_bytes(value).map_err(|_| SeedError::Encoding)
}

fn valid_resource_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_SEED_NAME_BYTES
        && name.bytes().enumerate().all(|(index, byte)| {
            if index == 0 {
                byte.is_ascii_lowercase()
            } else {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
            }
        })
}

fn static_type(resource_type: &str) -> &'static str {
    match resource_type {
        "Zone" => "Zone",
        "Role" => "Role",
        "RoleBinding" => "RoleBinding",
        "Operation" => "Operation",
        "SeccompProfile" => "SeccompProfile",
        _ => "unknown",
    }
}

fn role_ref(name: &str) -> String {
    format!("Role/{name}")
}

fn self_binding_name(
    provider: &SeedProvider,
    binding: &SeedSelfBinding,
) -> Result<String, SeedError> {
    let name = format!(
        "{}-self-{}",
        provider.provider_ref.name().as_str(),
        binding.role_ref.name().as_str()
    );
    if !valid_resource_name(&name) {
        return Err(SeedError::InvalidRow {
            row: "RoleBinding",
            name,
            reason: "the framework-generated binding name is over the resource-name bound",
        });
    }
    Ok(name)
}

fn self_binding_spec(
    provider: &SeedProvider,
    binding: &SeedSelfBinding,
) -> Result<RoleBindingSpec, SeedError> {
    RoleBindingSpec::new(binding.role_ref.clone(), vec![binding.subject_ref.clone()], None, None)
        .map_err(|_| SeedError::InvalidRow {
            row: "RoleBinding",
            name: format!("{}-self-binding", provider.provider_ref.name()),
            reason: "the self-binding spec is invalid",
        })
}
/// `Operation/process-run-<command>`: the resource-name grammar has no dot,
/// so the dotted spawn-operation spelling of the design is carried by the
/// hyphen. A command whose materialized name would exceed the bound refuses.
fn require_committed(
    committed: &CommittedSet,
    row: &str,
    field: &'static str,
    reference: &ResourceRef,
) -> Result<(), SeedError> {
    if committed.contains(reference) {
        Ok(())
    } else {
        Err(SeedError::UnresolvedRef {
            row: row.to_owned(),
            field,
            missing: reference.to_canonical_string(),
        })
    }
}

fn verb_spelling(verb: RoleResourceVerb) -> &'static str {
    match verb {
        RoleResourceVerb::Get => "get",
        RoleResourceVerb::List => "list",
        RoleResourceVerb::Watch => "watch",
        RoleResourceVerb::Create => "create",
        RoleResourceVerb::UpdateSpec => "update-spec",
        RoleResourceVerb::UpdateStatus => "update-status",
        RoleResourceVerb::UpdateMetadata => "update-metadata",
        RoleResourceVerb::UpdateFinalizers => "update-finalizers",
        RoleResourceVerb::Delete => "delete",
        RoleResourceVerb::UseCredential => "use-credential",
        RoleResourceVerb::AdminCredential => "admin-credential",
    }
}

/// The secret-access ceiling a payload implies: a payload with write-only
/// fields needs at least redacted access, a plain payload none.
/// One refused seed run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedError {
    /// The durable store refused a read or write.
    Store(String),
    /// A declared row is malformed (name, facets, or encoding).
    InvalidRow {
        /// The resource type of the row.
        row: &'static str,
        /// The declared name.
        name: String,
        /// Why the row is invalid.
        reason: &'static str,
    },
    /// One declared reference does not resolve over the committed set.
    UnresolvedRef {
        /// The declared row carrying the reference.
        row: String,
        /// The field name carrying it.
        field: &'static str,
        /// The unresolved reference.
        missing: String,
    },
    /// A Role rule names a resource type no registered driver declares.
    UnknownResourceType {
        /// The declared role.
        row: String,
        /// The unknown type.
        type_name: String,
    },
    /// A Role rule grants a verb the type's declaration does not carry.
    UndeclaredVerb {
        /// The declared role.
        row: String,
        /// The resource type.
        type_name: String,
        /// The undeclared verb.
        verb: String,
    },
    /// A Role posture names a principal the committed allocation does not
    /// carry.
    PrincipalNotAllocated {
        /// The declared role.
        row: String,
        /// The unallocated principal name.
        principal: String,
    },
    /// A self-binding escapes its declaring provider's scope.
    SelfBindingEscaped {
        /// The declaring provider.
        provider: String,
        /// The binding subject.
        subject: String,
        /// The bound role.
        role: String,
    },
    /// A binding subject is not a bindable subject type.
    UnbindableSubject {
        /// The declared binding.
        row: String,
        /// The refused subject.
        subject: String,
    },
    /// A binding scope entry is outside the bound Role's own rules.
    ScopeOutsideRole {
        /// The declared binding.
        row: String,
        /// The scope field.
        field: &'static str,
        /// The refused entry.
        reference: String,
    },
    /// One row reference is declared twice.
    DuplicateRow(String),
    /// One seeded row is refused by the verified deployment graph.
    ///
    /// The seed is admitted, not waved through: a row presented under any
    /// subject other than the verified deployment root is refused here with
    /// the enforcing stage and the reason the evaluator named.
    GraphRefused {
        /// The row reference that was refused.
        row: String,
        /// The stage the evaluator refused at.
        stage: AdmissionStage,
        /// Why it refused.
        reason: RefusalReason,
    },
    /// A contract value failed to encode canonically.
    Encoding,
}

impl core::fmt::Display for SeedError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "foundation seed store failure: {error}"),
            Self::InvalidRow { row, name, reason } => {
                write!(formatter, "foundation seed refused {row}/{name}: {reason}")
            }
            Self::UnresolvedRef { row, field, missing } => write!(
                formatter,
                "foundation seed refused {row}: {field} names uncommitted {missing}"
            ),
            Self::UnknownResourceType { row, type_name } => write!(
                formatter,
                "foundation seed refused {row}: {type_name} has no registered driver"
            ),
            Self::UndeclaredVerb {
                row,
                type_name,
                verb,
            } => write!(
                formatter,
                "foundation seed refused {row}: {type_name} does not declare verb {verb}"
            ),
            Self::PrincipalNotAllocated { row, principal } => write!(
                formatter,
                "foundation seed refused {row}: principal {principal} has no committed allocation"
            ),
            Self::SelfBindingEscaped {
                provider,
                subject,
                role,
            } => write!(
                formatter,
                "foundation seed refused {provider} self-binding: subject {subject} role {role} \
                 escape the declaring provider's scope"
            ),
            Self::UnbindableSubject { row, subject } => write!(
                formatter,
                "foundation seed refused {row}: {subject} is not a bindable subject"
            ),
            Self::ScopeOutsideRole {
                row,
                field,
                reference,
            } => write!(
                formatter,
                "foundation seed refused {row}: {field} entry {reference} is outside the role"
            ),
            Self::DuplicateRow(row) => write!(
                formatter,
                "foundation seed refused {row}: the reference is declared twice"
            ),
            Self::GraphRefused { row, stage, reason } => write!(
                formatter,
                "foundation seed refused {row}: the verified deployment graph refused it at \
                 {stage:?} for {reason:?}"
            ),
            Self::Encoding => formatter.write_str("foundation seed refused an unencodable row"),
        }
    }
}

impl std::error::Error for SeedError {}
// ---------------------------------------------------------------------------
// The verified deployment bootstrap (U31, KTD7)
// ---------------------------------------------------------------------------

/// Deployment-root-relative name of the verified deployment graph the daemon
/// and the broker both bootstrap from.
///
/// The deployment root is the same directory the ownership-bounded reset
/// reads (`/var/lib/d2b` by default), so one deployment publishes one
/// graph and both halves of the trust root agree on where it lives.
pub const DEPLOYMENT_BOOTSTRAP_FILE: &str = "deployment-bootstrap.json";

/// The document schema tag this release verifies.
///
/// A document carrying any other tag is an artifact of another contract
/// version and is refused; there is no compatibility parse and no default
/// for a missing or unknown tag (R43).
pub const DEPLOYMENT_BOOTSTRAP_SCHEMA: &str = "d2b-deployment-bootstrap/1";

/// The domain tag framing the deployment graph's own self-hash.
///
/// This is the same framed-digest profile the Nix bundle compiler and the
/// artifact catalog already cross with this package, so the deployment
/// graph is self-hashed by the same mechanism that verifies every other
/// verified artifact rather than by a second digest spelling.
pub const DEPLOYMENT_BOOTSTRAP_DIGEST_DOMAIN: &str = "d2b:v3:deployment-bootstrap";

/// The bounded read for the deployment bootstrap document.
pub const MAX_DEPLOYMENT_BOOTSTRAP_BYTES: usize = 4 * 1024 * 1024;

/// The environment variable naming the deployment root.
///
/// The daemon and the broker share one deployment root, so both halves read
/// the same verified document rather than each resolving their own.
pub const DEPLOYMENT_ROOT_ENV: &str = "D2B_DEPLOYMENT_ROOT";

/// The deployment root used when the environment names none.
pub const DEFAULT_DEPLOYMENT_ROOT: &str = "/var/lib/d2b";

/// A refusal of the verified deployment bootstrap.
///
/// Every variant names the enforcing stage and the reason. A startup that
/// cannot produce a verified graph refuses; there is no permissive
/// fallback that lets a provider or an effect begin anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapRefusal {
    /// The document is absent, unreadable, or larger than the bound.
    DocumentUnreadable {
        /// The deployment-root-relative path that was read.
        path: String,
    },
    /// The document's own schema tag is not this release's contract.
    SchemaUnsupported {
        /// The tag the document carried.
        observed: String,
    },
    /// The document's self-hash does not cover its own bytes.
    DigestMismatch {
        /// The digest the document claimed.
        claimed: String,
    },
    /// The document describes a Zone this bootstrap does not own.
    ZoneMismatch {
        /// The Zone the document named.
        observed: String,
    },
    /// The document names an implementation this build does not compile.
    UnknownImplementation {
        /// The implementation identity the document declared.
        implementation: String,
    },
    /// The document names no deployment implementation at all.
    NoImplementations,
    /// The document carries a state Volume reference that is not a resource
    /// reference, so the foundations could not publish it.
    InvalidStateVolume {
        /// The state Volume reference as written.
        observed: String,
    },
    /// A required foundation RoleBinding is absent from the verified graph.
    ///
    /// This is the refusal that keeps a graph with no grants from starting:
    /// the deployment cannot fall back to admitting everything, so the
    /// daemon refuses to open its planes at all.
    MissingFoundationBinding {
        /// The RoleBinding reference the foundations require.
        reference: String,
    },
    /// One authority row is declared twice in the document.
    DuplicateRow {
        /// The duplicated resource reference.
        reference: String,
    },
    /// A publication step requires a row that the plan never publishes.
    UnresolvedRequirement {
        /// The step whose requirement cannot resolve.
        step: String,
        /// The requirement it names.
        requirement: String,
    },
    /// A publication step requires a row published after it.
    ///
    /// This is the state-Volume cycle made explicit: a step that needs a
    /// row that only exists once the step itself has run cannot be ordered,
    /// and the bootstrap refuses rather than publishing one of the two
    /// first and leaving the other unadmitted.
    PublicationCycle {
        /// The step whose requirement points forward or at itself.
        step: String,
        /// The requirement it names.
        requirement: String,
    },
}

impl core::fmt::Display for BootstrapRefusal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DocumentUnreadable { path } => write!(
                formatter,
                "deployment bootstrap refused: {path} is absent, unreadable, or over the read \
                 bound"
            ),
            Self::SchemaUnsupported { observed } => write!(
                formatter,
                "deployment bootstrap refused: schema {observed} is not \
                 {DEPLOYMENT_BOOTSTRAP_SCHEMA}"
            ),
            Self::DigestMismatch { claimed } => write!(
                formatter,
                "deployment bootstrap refused: the document does not hash to its claimed digest \
                 {claimed}"
            ),
            Self::ZoneMismatch { observed } => write!(
                formatter,
                "deployment bootstrap refused: the document describes Zone {observed}, not the \
                 foundation Zone {SYSTEM_ZONE}"
            ),
            Self::UnknownImplementation { implementation } => write!(
                formatter,
                "deployment bootstrap refused: implementation {implementation} is declared by \
                 the deployment but compiled by no provider declaration"
            ),
            Self::NoImplementations => formatter.write_str(
                "deployment bootstrap refused: the document declares no implementation",
            ),
            Self::InvalidStateVolume { observed } => write!(
                formatter,
                "deployment bootstrap refused: state Volume {observed} is not a resource \
                 reference"
            ),
            Self::MissingFoundationBinding { reference } => write!(
                formatter,
                "deployment bootstrap refused: the verified graph carries no {reference}, so the \
                 foundations it publishes would be unadmitted"
            ),
            Self::DuplicateRow { reference } => write!(
                formatter,
                "deployment bootstrap refused: {reference} is declared twice"
            ),
            Self::UnresolvedRequirement { step, requirement } => write!(
                formatter,
                "deployment bootstrap refused: {step} requires {requirement}, which the plan \
                 never publishes"
            ),
            Self::PublicationCycle { step, requirement } => write!(
                formatter,
                "deployment bootstrap refused: {step} requires {requirement}, which is not \
                 published before it"
            ),
        }
    }
}

impl std::error::Error for BootstrapRefusal {}

/// One authority row of the verified deployment graph.
///
/// The row travels as the canonical bytes the deployment accepted, so the
/// graph decides exactly what those bytes decide everywhere else. The
/// reference is the exact resource reference the row was accepted for.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BootstrapAuthorityRow {
    /// The exact resource reference the row was accepted for.
    pub reference: String,
    /// The row's canonical admitted bytes.
    pub admitted: serde_json::Value,
}

/// The verified new deployment graph, as published beside the deployment
/// root.
///
/// The document is self-hashed over its own canonical bytes with
/// `graphDigest` cleared, so a graph whose authority rows or declared
/// implementations were edited after verification fails closed before a
/// single provider is published. Its Zone is the foundation Zone and its
/// root subject is the deployment root, so nothing in it can authorize its
/// own introduction.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentBootstrap {
    /// The document's own schema tag.
    pub schema_version: String,
    /// The foundation Zone this graph bootstraps.
    pub zone: ZoneId,
    /// The store incarnation the deployment is verified in.
    pub store_incarnation: StoreIncarnation,
    /// The deployment's own state Volume, published with the foundations.
    ///
    /// Required component state is an ordinary Volume (R13), and the
    /// deployment's own state cannot be one that waits for the providers
    /// that write it: it is published in the foundation layer so every
    /// declared provider starts against state that already exists.
    pub state_volume: String,
    /// The implementation identities this deployment publishes.
    ///
    /// Each entry must be an identity a compiled provider declaration
    /// binds. There is no separate configurable allowlist: the compiled
    /// declaration table is the whole set of implementations that exist.
    pub implementations: Vec<String>,
    /// The accepted `Role` rows, as canonical bytes.
    pub roles: Vec<BootstrapAuthorityRow>,
    /// The accepted `RoleBinding` rows, as canonical bytes.
    pub role_bindings: Vec<BootstrapAuthorityRow>,
    /// The framed digest over the canonical bytes of this document
    /// without `graphDigest`.
    pub graph_digest: String,
}

impl DeploymentBootstrap {
    /// Decode and verify the document bytes.
    ///
    /// The three refusals this can return before any provider is published
    /// are deliberately distinct: an artifact of another contract version,
    /// a document whose bytes were edited after verification, and a
    /// document describing a Zone this bootstrap does not own.
    pub fn decode(bytes: &[u8], path: &str) -> Result<Self, BootstrapRefusal> {
        if bytes.is_empty() || bytes.len() > MAX_DEPLOYMENT_BOOTSTRAP_BYTES {
            return Err(BootstrapRefusal::DocumentUnreadable {
                path: path.to_owned(),
            });
        }
        let graph: Self = serde_json::from_slice(bytes).map_err(|_| {
            BootstrapRefusal::DocumentUnreadable {
                path: path.to_owned(),
            }
        })?;
        graph.verify()?;
        Ok(graph)
    }

    /// Verify the document's own schema tag, Zone, and self-hash.
    pub fn verify(&self) -> Result<(), BootstrapRefusal> {
        if self.schema_version != DEPLOYMENT_BOOTSTRAP_SCHEMA {
            return Err(BootstrapRefusal::SchemaUnsupported {
                observed: self.schema_version.clone(),
            });
        }
        if self.zone.as_str() != SYSTEM_ZONE {
            return Err(BootstrapRefusal::ZoneMismatch {
                observed: self.zone.as_str().to_owned(),
            });
        }
        let bytes = Self::canonical_bytes_without_digest(self)?;
        let observed = d2b_contracts_resource::v3::framed_canonical_digest(
            DEPLOYMENT_BOOTSTRAP_DIGEST_DOMAIN,
            &bytes,
        );
        if observed != self.graph_digest {
            return Err(BootstrapRefusal::DigestMismatch {
                claimed: self.graph_digest.clone(),
            });
        }
        Ok(())
    }

    /// The canonical bytes the self-hash covers: this document with its own
    /// `graphDigest` field removed.
    ///
    /// The publisher hashes the document it is about to write, which has no
    /// digest field yet, so verification removes the field rather than
    /// blanking it. Clearing it instead would hash a different byte string
    /// from the one the publisher hashed, and the daemon and the Activation
    /// family would disagree about the same document.
    pub(crate) fn canonical_bytes_without_digest(
        graph: &DeploymentBootstrap,
    ) -> Result<Vec<u8>, BootstrapRefusal> {
        let rendered = serde_json::to_vec(graph).map_err(|_| {
            BootstrapRefusal::DocumentUnreadable {
                path: DEPLOYMENT_BOOTSTRAP_FILE.to_owned(),
            }
        })?;
        let mut document = CanonicalJsonValue::parse(&rendered).map_err(|_| {
            BootstrapRefusal::DocumentUnreadable {
                path: DEPLOYMENT_BOOTSTRAP_FILE.to_owned(),
            }
        })?;
        let CanonicalJsonValue::Object(fields) = &mut document else {
            return Err(BootstrapRefusal::DocumentUnreadable {
                path: DEPLOYMENT_BOOTSTRAP_FILE.to_owned(),
            });
        };
        if fields.remove("graphDigest").is_none() {
            return Err(BootstrapRefusal::DocumentUnreadable {
                path: DEPLOYMENT_BOOTSTRAP_FILE.to_owned(),
            });
        }
        Ok(document.to_canonical_bytes())
    }

    /// The deployment root this graph publishes.
    pub fn deployment_root() -> std::path::PathBuf {
        std::path::PathBuf::from(
            std::env::var(DEPLOYMENT_ROOT_ENV)
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_DEPLOYMENT_ROOT.to_owned()),
        )
    }

    /// Read, decode, and verify the graph published at the deployment root.
    ///
    /// This is the daemon's and the Guest's only way to learn what it is
    /// deploying: there is no fallback document and no default graph, so a
    /// deployment that did not publish a verified graph has nothing to boot.
    ///
    /// The read is on the async filesystem driver rather than a blocking
    /// `std::fs` call, because it runs on the boot path inside the resource
    /// plane's async open, and a blocking read there would occupy a runtime
    /// worker for the length of the deployment root's I/O.
    pub async fn read_from_deployment_root(
        root: &std::path::Path,
    ) -> Result<Self, BootstrapRefusal> {
        let relative = DEPLOYMENT_BOOTSTRAP_FILE.to_owned();
        let bytes = read_deployment_bootstrap_bytes(root).await?;
        Self::decode(&bytes, &relative)
    }

    /// The deployment's own state Volume, as the exact reference the
    /// foundations publish.
    pub fn state_volume_ref(&self) -> Result<ResourceRef, BootstrapRefusal> {
        ResourceRef::parse(self.state_volume.as_str()).map_err(|_| {
            BootstrapRefusal::InvalidStateVolume {
                observed: self.state_volume.clone(),
            }
        })
    }

    /// The accepted authority rows, decoded to their canonical objects.
    ///
    /// The returned objects own their bytes, so the projection rows built
    /// from them never point into a collection this call already dropped.
    fn decoded_rows(&self) -> Result<Vec<(ResourceRef, CanonicalJsonObject)>, BootstrapRefusal>
    {
        let mut decoded: Vec<(ResourceRef, CanonicalJsonObject)> = Vec::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for rows in [&self.roles, &self.role_bindings] {
            for row in rows {
                let reference = ResourceRef::parse(row.reference.as_str()).map_err(|error| {
                    BootstrapRefusal::UnresolvedRequirement {
                        step: DEPLOYMENT_BOOTSTRAP_FILE.to_owned(),
                        requirement: format!("{} ({error})", row.reference),
                    }
                })?;
                if !seen.insert(reference.to_canonical_string()) {
                    return Err(BootstrapRefusal::DuplicateRow {
                        reference: row.reference.clone(),
                    });
                }
                let admitted =
                    serde_json::from_value::<CanonicalJsonObject>(row.admitted.clone()).map_err(
                        |error| BootstrapRefusal::UnresolvedRequirement {
                            step: row.reference.clone(),
                            requirement: format!("canonical row bytes ({error})"),
                        },
                    )?;
                decoded.push((reference, admitted));
            }
        }
        Ok(decoded)
    }

    /// The prior accepted graph this deployment bootstraps from.
    ///
    /// Built through [`AcceptedGraph::from_canonical_rows`] so the graph
    /// decides exactly what the same rows decide at every other boundary,
    /// under the deployment root subject rather than under any grant the
    /// document itself introduces.
    pub fn accepted_graph(&self) -> Result<AcceptedGraph, BootstrapRefusal> {
        let decoded = self.decoded_rows()?;
        let rows = decoded
            .iter()
            .map(|(reference, admitted)| ProjectionRow::new(reference, admitted));
        AcceptedGraph::from_canonical_rows(
            self.zone.clone(),
            self.store_incarnation.clone(),
            AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
            rows,
        )
        .map_err(|_| BootstrapRefusal::DocumentUnreadable {
            path: DEPLOYMENT_BOOTSTRAP_FILE.to_owned(),
        })
    }

    /// The authority projection rows this verified deployment publishes.
    ///
    /// The set is this document's own authority rows, in the committed
    /// canonical bytes the document carries. The broker stores the bytes it
    /// accepted and re-evaluates policy against them, so a summary this
    /// publisher shaped instead would be a second authority rather than the
    /// one that was verified.
    ///
    /// A binding row's relationship identity is resolved here and nowhere
    /// else. The accepted graph's own key names the two uids - the manager is
    /// the party that resolved them and it resolves them once - so a row whose
    /// exact relationship the graph did not accept is published without one:
    /// the broker then holds no key for it and refuses that relationship,
    /// which is the correct answer for an absence and not for a committed
    /// relationship.
    pub fn authority_rows(&self) -> Result<Vec<AuthorityProjectionRow>, BootstrapRefusal> {
        // One relationship identity per accepted source, under the exact row
        // reference that relationship's own key renders. Two sources whose
        // keys render one reference would be a row the broker could key two
        // ways, so the collision refuses rather than resolves by iteration.
        let graph = self.accepted_graph()?;
        let mut identities: BTreeMap<String, (ResourceUid, ResourceUid)> = BTreeMap::new();
        for (key, _source) in graph.sources() {
            let row = binding_row_ref(key).ok_or_else(|| BootstrapRefusal::UnresolvedRequirement {
                step: DEPLOYMENT_BOOTSTRAP_FILE.to_owned(),
                requirement: format!(
                    "the accepted relationship {} renders no canonical row reference",
                    key.consumer_ref().to_canonical_string()
                ),
            })?;
            let identity = (key.source_uid().clone(), key.consumer_uid().clone());
            if identities.insert(row.to_canonical_string(), identity).is_some() {
                return Err(BootstrapRefusal::DuplicateRow {
                    reference: row.to_canonical_string(),
                });
            }
        }
        let decoded = self.decoded_rows()?;
        let mut rows = Vec::with_capacity(decoded.len());
        for (reference, admitted) in decoded {
            let bytes = admitted.to_canonical_bytes();
            let (source_uid, consumer_uid) = identities
                .get(&reference.to_canonical_string())
                .map_or((None, None), |(source, consumer)| {
                    (Some(source.clone()), Some(consumer.clone()))
                });
            rows.push(AuthorityProjectionRow {
                resource_ref: reference,
                desired_revision: DesiredRevision::INITIAL,
                desired_digest: DesiredDigest::of(&bytes),
                admitted,
                source_uid,
                consumer_uid,
            });
        }
        Ok(rows)
    }
}

/// The implementation identities this build actually compiles.
///
/// The generated provider registration table is the whole list: it is
/// emitted from each provider crate's own declaration, so an identity that
/// is not in it has no compiled implementation behind it and the deployment
/// that names it is refused. Nothing configurable adds to this set, which is
/// what removes the separate bootstrap allowlist (R11, R12).
///
/// # A provider's own declaration is the only source
///
/// The table is generated from each provider crate's `registrations.json`,
/// and this function reads only that table. Nothing here derives an
/// implementation identity from a resource reference, a projection owner, a
/// catalog, or a package name: a shared crate naming a provider-crate
/// identifier is what the layout gate refuses, and more importantly such a
/// push would publish an identity no declaration had registered, so the
/// deployment that published it would be refused by the very check it was
/// meant to satisfy. A provider that does not declare itself is not a
/// deployment implementation, which is correct: the framework's own
/// execution providers are bound by the foundation seed's own
/// self-bindings, under the `Provider/<name>` resource reference their rows
/// commit, and that binding is not an implementation publication.
pub fn compiled_implementations() -> Vec<&'static str> {
    let mut identities: Vec<&'static str> =
        crate::resource_plane_v3::PROVIDER_REGISTRATIONS
            .iter()
            .map(|registration| registration.provider_ref)
            .collect();
    identities.sort_unstable();
    identities.dedup();
    identities
}

/// Which layer of the bootstrap publication a step belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PublicationLayer {
    /// The fixed foundations: the Zone, the deployment's state Volume, the
    /// declared policy rows, the provider self-bindings, the materialized
    /// operations, and the operator bindings.
    Foundations,
    /// The declared providers, published only once every foundation they
    /// read has been published under an accepted graph.
    DeclaredProviders,
}

impl PublicationLayer {
    /// The stable label for this layer.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Foundations => "foundations",
            Self::DeclaredProviders => "declared-providers",
        }
    }
}

/// One row or implementation the bootstrap publishes, and the exact rows it
/// must already be published under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationStep {
    /// The layer this step publishes in.
    pub layer: PublicationLayer,
    /// The exact resource reference, or the exact implementation identity.
    pub reference: String,
    /// The exact references this step reads before it may be published.
    pub requires: Vec<String>,
}

impl PublicationStep {
    /// A foundation step with no prerequisites.
    pub fn foundation(reference: impl Into<String>) -> Self {
        Self {
            layer: PublicationLayer::Foundations,
            reference: reference.into(),
            requires: Vec::new(),
        }
    }

    /// A step that publishes `reference` once every named requirement is
    /// already published.
    pub fn step(
        layer: PublicationLayer,
        reference: impl Into<String>,
        requires: Vec<String>,
    ) -> Self {
        Self {
            layer,
            reference: reference.into(),
            requires,
        }
    }
}

/// The ordered publication plan the bootstrap publishes.
///
/// The plan is ordered, and every step's requirements must already appear
/// strictly earlier in it. That is the whole cycle argument: a plan whose
/// steps are ordered and whose requirements all point backwards cannot
/// contain a cycle, and a plan that does contain one is refused with the
/// exact step and requirement that close it. The deployment's own state
/// Volume is a foundation step precisely so a provider that needs it never
/// waits on a row only that provider can create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationPlan {
    steps: Vec<PublicationStep>,
}

impl PublicationPlan {
    /// Assemble a plan from its steps, refusing an unorderable one.
    pub fn new(steps: Vec<PublicationStep>) -> Result<Self, BootstrapRefusal> {
        let plan = Self { steps };
        plan.verify()?;
        Ok(plan)
    }

    /// The steps, in publication order.
    pub fn steps(&self) -> &[PublicationStep] {
        &self.steps
    }

    /// The steps of one layer, in publication order.
    pub fn layer(&self, layer: PublicationLayer) -> Vec<&PublicationStep> {
        self.steps.iter().filter(|step| step.layer == layer).collect()
    }

    /// The index a reference publishes at, when the plan publishes it.
    pub fn position(&self, reference: &str) -> Option<usize> {
        self.steps
            .iter()
            .position(|step| step.reference == reference)
    }

    /// Refuse a plan whose requirements are not published before the step
    /// that reads them.
    ///
    /// A requirement naming a row the plan never publishes and a
    /// requirement naming a row the plan publishes later are refused
    /// separately: the first is an unresolvable reference, the second is
    /// the cycle.
    pub fn verify(&self) -> Result<(), BootstrapRefusal> {
        let mut published: BTreeMap<&str, usize> = BTreeMap::new();
        for (index, step) in self.steps.iter().enumerate() {
            for requirement in &step.requires {
                match published.get(requirement.as_str()) {
                    None if self.position(requirement).is_none() => {
                        return Err(BootstrapRefusal::UnresolvedRequirement {
                            step: step.reference.clone(),
                            requirement: requirement.clone(),
                        });
                    }
                    None => {
                        return Err(BootstrapRefusal::PublicationCycle {
                            step: step.reference.clone(),
                            requirement: requirement.clone(),
                        });
                    }
                    Some(earlier) if *earlier >= index => {
                        return Err(BootstrapRefusal::PublicationCycle {
                            step: step.reference.clone(),
                            requirement: requirement.clone(),
                        });
                    }
                    Some(_) => {}
                }
            }
            published.insert(step.reference.as_str(), index);
        }
        Ok(())
    }
}

/// The verified deployment graph the daemon has published.
///
/// Publication is the whole of this unit's contribution: it is the point at
/// which the verified graph, the bound implementations, and the ordered
/// plan become the one accepted root every provider and effect reads. It is
/// only constructible through [`DeploymentBootstrap::publish`], so a
/// deployment that failed verification has no published root to run under.
#[derive(Debug, Clone)]
pub struct PublishedBootstrap {
    accepted: AcceptedGraph,
    plan: PublicationPlan,
    implementations: Vec<String>,
    /// The authority rows this publication installs at the broker (U7,
    /// KTD7).
    ///
    /// They are resolved once, here, from the verified document's own
    /// committed bytes: a caller cannot name a row, a revision, or a
    /// relationship identity this publication did not derive from the graph
    /// it verified.
    authority_rows: Vec<AuthorityProjectionRow>,
}

impl PublishedBootstrap {
    /// The accepted graph every later decision reads.
    pub fn accepted(&self) -> &AcceptedGraph {
        &self.accepted
    }

    /// The ordered publication plan.
    pub fn plan(&self) -> &PublicationPlan {
        &self.plan
    }

    /// The implementation identities this deployment published.
    pub fn implementations(&self) -> &[String] {
        &self.implementations
    }

    /// The store incarnation this deployment is verified in.
    pub fn store_incarnation(&self) -> &StoreIncarnation {
        self.accepted.store()
    }

    /// The mutation admission this publication installs.
    ///
    /// It is the plane's existing identity evaluator over the accepted
    /// graph, so a request is decided by the grants the verified graph
    /// actually carries. There is no permissive constructor on this type:
    /// an admission can only be obtained from a graph that published every
    /// foundation binding it required.
    pub fn admission(&self, transport: TransportIdentity) -> GraphMutationAdmission {
        GraphMutationAdmission::new(
            std::sync::Arc::new(self.accepted.clone()),
            self.accepted.zone().clone(),
            transport,
        )
    }

    /// The deployment identity this publication installs at a broker that
    /// is already running.
    ///
    /// The identity carries the accepted root subject and the cursor the
    /// publication commits at, so a later switch is decided against the same
    /// root the cold start established.
    pub fn identity(
        &self,
        cursor: AuthorityCursor,
        snapshot_digest: DesiredDigest,
    ) -> DeploymentIdentity {
        DeploymentIdentity {
            zone: self.accepted.zone().clone(),
            store_incarnation: self.accepted.store().clone(),
            root_subject: AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
            cursor,
            snapshot_digest,
        }
    }

    /// The bounded snapshot document and the deployment identity it installs.
    ///
    /// A cold start installs the accepted root at the initial cursor, which is
    /// exactly where a broker holding no accepted projection already stands:
    /// the publication therefore advances no sequence and contradicts no
    /// digest, and a daemon that restarts republishes the same document rather
    /// than moving the authority under a running Zone.
    pub fn authority_publication(&self) -> (DeploymentIdentity, AuthoritySnapshot) {
        let snapshot =
            DeploymentIdentity::initial(self.accepted.zone().clone(), self.accepted.store().clone())
                .snapshot(self.authority_rows.clone());
        let identity = self.identity(snapshot.cursor.clone(), publication_snapshot_digest(&snapshot));
        (identity, snapshot)
    }

    /// The authority rows this publication installs at the broker.
    pub fn authority_rows(&self) -> &[AuthorityProjectionRow] {
        &self.authority_rows
    }
}

impl DeploymentBootstrap {
    /// Verify the graph, bind its implementations, and publish the ordered
    /// plan.
    ///
    /// Nothing here is best-effort. An implementation this build does not
    /// compile, an unorderable plan, or a graph missing one of the
    /// foundation RoleBindings the declarations require all refuse, and a
    /// refusal means the caller has no published root and must not start a
    /// provider.
    pub fn publish(
        &self,
        declarations: &FoundationDeclarations,
    ) -> Result<PublishedBootstrap, BootstrapRefusal> {
        self.verify()?;
        if self.implementations.is_empty() {
            return Err(BootstrapRefusal::NoImplementations);
        }
        let compiled = compiled_implementations();
        for implementation in &self.implementations {
            if !compiled.contains(&implementation.as_str()) {
                return Err(BootstrapRefusal::UnknownImplementation {
                    implementation: implementation.clone(),
                });
            }
        }
        let state_volume = self.state_volume_ref()?;
        let accepted = self.accepted_graph()?;
        let plan = self.publication_plan(declarations, &state_volume)?;
        let bound = accepted
            .role_bindings()
            .map(|(reference, _)| reference.to_canonical_string())
            .collect::<BTreeSet<_>>();
        for required in required_foundation_bindings(declarations) {
            if !bound.contains(&required) {
                return Err(BootstrapRefusal::MissingFoundationBinding {
                    reference: required,
                });
            }
        }
        Ok(PublishedBootstrap {
            accepted,
            plan,
            implementations: self.implementations.clone(),
            authority_rows: self.authority_rows()?,
        })
    }

    /// The ordered publication plan for this deployment.
    ///
    /// Foundations come first, in the order the seed itself commits them,
    /// and every declared provider follows. A provider's step names the
    /// exact foundation rows it reads: its own self-binding and the
    /// deployment's state Volume.
    pub fn publication_plan(
        &self,
        declarations: &FoundationDeclarations,
        state_volume: &ResourceRef,
    ) -> Result<PublicationPlan, BootstrapRefusal> {
        let state_volume_ref = state_volume.to_canonical_string();
        let mut steps = Vec::new();
        // 0. The foundation Zone itself.
        steps.push(PublicationStep::foundation(format!(
            "Zone/{SYSTEM_ZONE}"
        )));
        // 1. The deployment's own state Volume. It is a foundation row, so
        // a provider that stores component state in it never waits on a row
        // only that provider could create.
        steps.push(PublicationStep::foundation(state_volume_ref.clone()));
        // 2. The declared policy vocabulary, then the roles that select it.
        for profile in &declarations.profiles {
            steps.push(PublicationStep::foundation(format!(
                "{SECCOMP_PROFILE_RESOURCE_TYPE}/{}",
                profile.name
            )));
        }
        for policy in &declarations.policies {
            steps.push(PublicationStep::foundation(format!(
                "{EXECUTION_POLICY_RESOURCE_TYPE}/{}",
                policy.name
            )));
        }
        for role in &declarations.roles {
            steps.push(PublicationStep::foundation(role_ref(&role.name)));
        }
        // 3. The provider self-bindings, each requiring the role it binds.
        for provider in &declarations.providers {
            for binding in &provider.self_bindings {
                let name = bound_binding_name(provider, binding)?;
                steps.push(PublicationStep::step(
                    PublicationLayer::Foundations,
                    format!("RoleBinding/{name}"),
                    vec![binding.role_ref.to_canonical_string()],
                ));
        }
        }
        // 5. The operator bindings from the host contract.
        for binding in &declarations.operator_bindings {
            steps.push(PublicationStep::step(
                PublicationLayer::Foundations,
                format!("RoleBinding/{}", binding.name),
                vec![binding.spec.role_ref().to_canonical_string()],
            ));
        }
        // 6. The declared providers, last: every one of them reads its own
        // self-binding and the deployment's state Volume.
        for implementation in &self.implementations {
            steps.push(PublicationStep::step(
                PublicationLayer::DeclaredProviders,
                implementation.clone(),
                vec![state_volume_ref.clone()],
            ));
        }
        for provider in &declarations.providers {
            let mut requires = vec![state_volume_ref.clone()];
            for binding in &provider.self_bindings {
                requires.push(format!(
                    "RoleBinding/{}",
                    bound_binding_name(provider, binding)?
                ));
            }
            steps.push(PublicationStep::step(
                PublicationLayer::DeclaredProviders,
                provider.provider_ref.to_canonical_string(),
                requires,
            ));
        }
        PublicationPlan::new(steps)
    }
}

/// The exact RoleBinding references the foundations require to exist before
/// any provider may publish.
///
/// This is the set whose absence refuses startup. It is derived from the
/// declarations the seed commits, not from a table in shared code, so a
/// deployment cannot declare a provider whose authorizing binding is absent
/// from the graph it published.
pub fn required_foundation_bindings(declarations: &FoundationDeclarations) -> Vec<String> {
    let mut required: Vec<String> = Vec::new();
    for provider in &declarations.providers {
        for binding in &provider.self_bindings {
            required.push(format!(
                "RoleBinding/{}",
                bound_binding_name(provider, binding).unwrap_or_default()
            ));
        }
    }
    for binding in &declarations.operator_bindings {
        required.push(format!("RoleBinding/{}", binding.name));
    }
    required.sort();
    required.dedup();
    required
}

/// Read the deployment root's verified graph bytes on the async driver.
///
/// Both the daemon and the Guest verify the same document, and both read it
/// through this one function, so the two views cannot come from different
/// bytes and neither read blocks a runtime worker.
pub async fn read_deployment_bootstrap_bytes(
    root: &std::path::Path,
) -> Result<Vec<u8>, BootstrapRefusal> {
    let path = root.join(DEPLOYMENT_BOOTSTRAP_FILE);
    let unreadable = || BootstrapRefusal::DocumentUnreadable {
        path: DEPLOYMENT_BOOTSTRAP_FILE.to_owned(),
    };
    let bytes = tokio::fs::read(&path).await.map_err(|_| unreadable())?;
    if bytes.is_empty() || bytes.len() > MAX_DEPLOYMENT_BOOTSTRAP_BYTES {
        return Err(unreadable());
    }
    Ok(bytes)
}

/// The framework-generated binding name, as a bootstrap refusal when the
/// seed would refuse it.
fn bound_binding_name(
    provider: &SeedProvider,
    binding: &SeedSelfBinding,
) -> Result<String, BootstrapRefusal> {
    self_binding_name(provider, binding).map_err(|error| BootstrapRefusal::UnresolvedRequirement {
        step: provider.provider_ref.to_canonical_string(),
        requirement: format!("RoleBinding/{error}"),
    })
}

// ---------------------------------------------------------------------------
// The deployment identity switch (U31, KTD6-KTD7)
// ---------------------------------------------------------------------------

/// The stage one deployment-identity switch is in.
///
/// The order is freeze, commit, publish, acknowledge. A broker that
/// already holds an accepted identity only ever moves to a new one through
/// that order, so there is no path on which a provider begins effects under
/// an identity the broker has not acknowledged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationStage {
    /// No switch is in flight.
    Idle,
    /// The Zone's new-effect admission is frozen and the candidate is
    /// being validated against the prior accepted graph.
    Prepare,
    /// The candidate was admitted; the broker advances its projection.
    Commit,
    /// The advanced projection is published to accepted visibility.
    Publish,
    /// The published identity is awaiting the broker's acknowledgment.
    Acknowledge,
}

impl PublicationStage {
    /// The stable label for this stage.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Prepare => "prepare",
            Self::Commit => "commit",
            Self::Publish => "publish",
            Self::Acknowledge => "acknowledge",
        }
    }
}

/// One deployment identity: the Zone, the store incarnation, the accepted
/// root subject, and the cursor plus snapshot digest the broker holds for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentIdentity {
    /// The Zone this identity describes.
    pub zone: ZoneId,
    /// The store incarnation this identity was accepted in.
    pub store_incarnation: StoreIncarnation,
    /// The deployment root the graph bootstraps.
    pub root_subject: AuthoritySubject,
    /// The accepted cursor.
    pub cursor: AuthorityCursor,
    /// The digest of the published snapshot document.
    pub snapshot_digest: DesiredDigest,
}

impl DeploymentIdentity {
    /// The initial identity for a freshly initialized store: the deployment
    /// root and the initial cursor.
    pub fn initial(
        zone: ZoneId,
        store_incarnation: StoreIncarnation,
    ) -> Self {
        Self {
            zone,
            store_incarnation,
            root_subject: AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
            cursor: AuthorityCursor::initial(),
            snapshot_digest: DesiredDigest::of(&[]),
        }
    }

    /// The bounded snapshot document this identity publishes.
    pub fn snapshot(&self, rows: Vec<AuthorityProjectionRow>) -> AuthoritySnapshot {
        AuthoritySnapshot {
            zone: self.zone.as_str().to_owned(),
            store_incarnation: self.store_incarnation.clone(),
            cursor: self.cursor.clone(),
            root_subject: self.root_subject.clone(),
            rows,
            outstanding: None,
        }
    }
}

/// A refusal of one deployment-identity switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwitchRefusal {
    /// The candidate describes a different Zone.
    ZoneMismatch {
        /// The Zone the candidate named.
        observed: String,
    },
    /// The candidate names a different store incarnation.
    ///
    /// A store incarnation is an identity, not an ordered counter: moving
    /// between incarnations is the ownership-bounded reset's job, never an
    /// ordinary publication's.
    IncarnationMismatch {
        /// The incarnation the broker holds.
        accepted: String,
        /// The incarnation the candidate named.
        observed: String,
    },
    /// A switch is already in flight for this Zone.
    Busy,
    /// A protocol step was attempted out of order.
    OutOfOrder {
        /// The stage the switch is in.
        stage: PublicationStage,
        /// The step that was attempted.
        action: &'static str,
    },
    /// The candidate's cursor moves below the accepted cursor.
    SequenceRegression {
        /// The sequence the broker has accepted.
        accepted: u64,
        /// The sequence the candidate named.
        observed: u64,
    },
    /// The candidate contradicts the accepted digest at its own sequence.
    DigestContradiction {
        /// The sequence the contradiction was observed at.
        sequence: u64,
    },
    /// The candidate carries a row the prior accepted graph refuses.
    GraphRefused {
        /// The exact resource reference that was refused.
        row: String,
        /// The stage the evaluator refused at.
        stage: AdmissionStage,
        /// Why it refused.
        reason: RefusalReason,
    },
}

impl core::fmt::Display for SwitchRefusal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ZoneMismatch { observed } => {
                write!(formatter, "deployment identity refused: Zone {observed}")
            }
            Self::IncarnationMismatch { accepted, observed } => write!(
                formatter,
                "deployment identity refused: store incarnation {observed} is not the accepted \
                 {accepted}; a different incarnation requires an explicit reset"
            ),
            Self::Busy => {
                formatter.write_str("deployment identity refused: a switch is already in flight")
            }
            Self::OutOfOrder { stage, action } => write!(
                formatter,
                "deployment identity refused: {action} is not the next step after {}",
                stage.as_str()
            ),
            Self::SequenceRegression { accepted, observed } => write!(
                formatter,
                "deployment identity refused: sequence {observed} is below the accepted \
                 {accepted}"
            ),
            Self::DigestContradiction { sequence } => write!(
                formatter,
                "deployment identity refused: sequence {sequence} contradicts the accepted digest"
            ),
            Self::GraphRefused { row, stage, reason } => write!(
                formatter,
                "deployment identity refused {row}: the prior accepted graph refused it at \
                 {stage:?} for {reason:?}"
            ),
        }
    }
}

impl std::error::Error for SwitchRefusal {}

/// One deployment-identity switch on a broker that already holds an
/// accepted identity.
///
/// This is the daemon half of the frozen publication protocol, restricted to
/// what changes when the *deployment* changes rather than when a desired row
/// changes: the same Zone, the same store incarnation, a cursor that never
/// moves backwards, and a root subject the broker already accepted. The
/// broker re-evaluates every candidate row against the prior accepted graph
/// (never against a grant the candidate itself introduces), so an update
/// that arrives while the broker is already running is admitted on the same
/// terms as the cold start.
#[derive(Debug, Clone)]
pub struct DeploymentIdentitySwitch {
    accepted: DeploymentIdentity,
    pending: Option<DeploymentIdentity>,
    stage: PublicationStage,
}

impl DeploymentIdentitySwitch {
    /// Open the switch for a broker that already holds `accepted`.
    pub const fn open(accepted: DeploymentIdentity) -> Self {
        Self {
            accepted,
            pending: None,
            stage: PublicationStage::Idle,
        }
    }

    /// The identity the broker currently holds.
    pub const fn accepted(&self) -> &DeploymentIdentity {
        &self.accepted
    }

    /// The stage this switch is in.
    pub const fn stage(&self) -> PublicationStage {
        self.stage
    }

    /// Begin the switch: freeze the Zone and validate the candidate.
    ///
    /// Every candidate row is admitted against the prior accepted graph, so
    /// a running broker refuses the same rows a cold start would refuse. The
    /// candidate's Zone and store incarnation must be the ones the broker
    /// already accepted.
    pub fn begin(
        &mut self,
        candidate: DeploymentIdentity,
        rows: &[ResourceRef],
        prior: &AcceptedGraph,
    ) -> Result<(), SwitchRefusal> {
        if self.pending.is_some() {
            return Err(SwitchRefusal::Busy);
        }
        if candidate.zone != self.accepted.zone {
            return Err(SwitchRefusal::ZoneMismatch {
                observed: candidate.zone.as_str().to_owned(),
            });
        }
        if candidate.store_incarnation != self.accepted.store_incarnation {
            return Err(SwitchRefusal::IncarnationMismatch {
                accepted: self.accepted.store_incarnation.as_str().to_owned(),
                observed: candidate.store_incarnation.as_str().to_owned(),
            });
        }
        let evidence = MutationSubjectEvidence::new(
            candidate.root_subject.clone(),
            TransportIdentity::Daemon,
        );
        for row in rows {
            let request = GraphMutation::new(
                candidate.zone.clone(),
                evidence.clone(),
                MutationKind::Create,
                row.clone(),
            );
            if let GraphAdmissionDecision::Refused { stage, reason } =
                GraphAuthority::admit_mutation(&request, prior)
            {
                return Err(SwitchRefusal::GraphRefused {
                    row: row.to_canonical_string(),
                    stage,
                    reason,
                });
            }
        }
        self.pending = Some(candidate);
        self.stage = PublicationStage::Prepare;
        Ok(())
    }

    /// The frozen prepare completed; the broker advances its projection.
    pub fn freeze(&mut self) -> Result<(), SwitchRefusal> {
        self.expect(PublicationStage::Prepare, "freeze")?;
        self.stage = PublicationStage::Commit;
        Ok(())
    }

    /// The projection advanced; publish it to accepted visibility.
    ///
    /// The cursor rules are enforced here, at the point the new identity
    /// becomes visible: a candidate below the accepted sequence, or one that
    /// contradicts the accepted digest at its own sequence, never reaches
    /// publication.
    pub fn commit(&mut self) -> Result<(), SwitchRefusal> {
        self.expect(PublicationStage::Commit, "commit")?;
        let candidate = self
            .pending
            .as_ref()
            .ok_or(SwitchRefusal::OutOfOrder {
                stage: self.stage,
                action: "commit",
            })?;
        let accepted_sequence = self.accepted.cursor.sequence.get();
        let candidate_sequence = candidate.cursor.sequence.get();
        if candidate_sequence < accepted_sequence {
            return Err(SwitchRefusal::SequenceRegression {
                accepted: accepted_sequence,
                observed: candidate_sequence,
            });
        }
        if candidate_sequence == accepted_sequence
            && candidate.snapshot_digest != self.accepted.snapshot_digest
        {
            return Err(SwitchRefusal::DigestContradiction {
                sequence: candidate_sequence,
            });
        }
        self.stage = PublicationStage::Publish;
        Ok(())
    }

    /// The projection is published; the broker acknowledges it.
    pub fn publish(&mut self) -> Result<(), SwitchRefusal> {
        self.expect(PublicationStage::Publish, "publish")?;
        self.stage = PublicationStage::Acknowledge;
        Ok(())
    }

    /// The broker acknowledged; the candidate becomes the accepted
    /// identity and the Zone unfreezes.
    pub fn acknowledge(&mut self) -> Result<DeploymentIdentity, SwitchRefusal> {
        self.expect(PublicationStage::Acknowledge, "acknowledge")?;
        let acknowledged = self
            .pending
            .take()
            .ok_or(SwitchRefusal::OutOfOrder {
                stage: self.stage,
                action: "acknowledge",
            })?;
        self.accepted = acknowledged.clone();
        self.stage = PublicationStage::Idle;
        Ok(acknowledged)
    }

    fn expect(&self, stage: PublicationStage, action: &'static str) -> Result<(), SwitchRefusal> {
        if self.stage == stage {
            Ok(())
        } else {
            Err(SwitchRefusal::OutOfOrder { stage: self.stage, action })
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
use d2b_provider_seccomp_profile::{ DeviceBind, DeviceNodeKind, SeccompCgroups, SeccompDeviceAccess, SeccompNamespaces };
    use d2b_contracts_resource::v3::{BoundedToken, CapabilityClass, NamespaceClass, OPERATION_RESOURCE_TYPE, PolicyCapabilities, PolicyIdentity, PolicyNamespaces, PolicyRoot, PolicySeccomp};
    use serde_json::json;

    struct Fixture {
        _dir: tempfile::TempDir,
        store: SpecStore,
        providers: ProviderDirectory,
        allocation: PrincipalAllocation,
    }

    fn providers() -> ProviderDirectory {
        let mut providers = ProviderDirectory::new();
        providers
            .register_driver(&d2b_provider_zone::zone_descriptor())
            .expect("Zone registers");
        providers
            .register_driver(&d2b_provider_role::role_descriptor())
            .expect("Role registers");
        providers
            .register_driver(&d2b_provider_role_binding::role_binding_descriptor())
            .expect("RoleBinding registers");
        providers
            .register_driver(&d2b_provider_operation::operation_descriptor())
            .expect("Operation registers");
        providers
            .register_driver(&d2b_provider_seccomp_profile::seccomp_profile_descriptor())
            .expect("SeccompProfile registers");
        providers
            .register_driver(&RestrictedDriver)
            .expect("Restricted registers");
        providers
    }

    /// A stub registration with a narrower declared verb set: the seed's
    /// rule check reads the registry, so a rule may not widen it. The type is
    /// a standard catalog entry the seed's fixtures do not otherwise drive.
    struct RestrictedDriver;

    impl d2b_resource_runtime::provider::DriverRegistration for RestrictedDriver {
        fn resource_type(&self) -> ResourceTypeName {
            ResourceTypeName::new("Quota")
        }

        fn requires_plane_registration(&self) -> bool {
            false
        }

        fn operation_refs(&self) -> Vec<String> {
            Vec::new()
        }

        fn declared_verbs(&self) -> Vec<String> {
            vec!["get".to_owned()]
        }

        fn decoder(&self) -> std::sync::Arc<dyn d2b_resource_runtime::context::SpecDecoder> {
            d2b_resource_runtime::metadata::metadata_spec_decoder()
        }

        fn factory(
            &self,
        ) -> std::sync::Arc<dyn d2b_resource_runtime::driver::ResourceDriverFactory> {
            std::sync::Arc::new(
                d2b_resource_runtime::metadata::MetadataDriverFactory::new(
                    ResourceTypeName::new("Quota"),
                ),
            )
        }
    }

    fn make_fixture() -> Fixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let store =
            SpecStore::open(dir.path().join("spec-store.sqlite3")).expect("spec store opens");
        Fixture {
            _dir: dir,
            store,
            providers: providers(),
            allocation: PrincipalAllocation::committed().expect("committed allocation"),
        }
    }

    /// The role the process controller runs under: `create` on `Operation`,
    /// scoped to the declared commands.
    fn publisher_role() -> SeedRole {
        let role = json!({
            "rules": [{
                "resourceTypes": ["Operation"],
                "verbs": ["create"],
                "subresources": [],
                "resourceNames": [],
                "zones": [],
                "executionRefs": [],
                "sessionVerbs": []
            }],
        });
        SeedRole {
            name: "operation-publisher".to_owned(),
            spec: serde_json::from_value(role).expect("publisher role"),
        }
    }

    fn worker_role() -> SeedRole {
        let role = json!({
            "rules": [{
                "resourceTypes": ["Operation"],
                "verbs": ["create"],
                "subresources": [],
                "resourceNames": [],
                "zones": [],
                "executionRefs": [],
                "sessionVerbs": []
            }],
                    });
        SeedRole {
            name: "worker".to_owned(),
            spec: serde_json::from_value(role).expect("worker role"),
        }
    }

    fn profile() -> SeedProfile {
        SeedProfile {
            name: "worker".to_owned(),
            spec: SeccompProfileSpec::new(
                vec![BoundedToken::parse("read").expect("syscall")],
                SeccompNamespaces::default(),
                SeccompCgroups::default(),
                vec![DeviceBind::new(
                    d2b_provider_seccomp_profile::DeviceNodePath::parse("/dev/null").expect("path"),
                    DeviceNodeKind::Char,
                    1,
                    3,
                    SeccompDeviceAccess::ReadWrite,
                )],
            )
            .expect("profile spec"),
        }
    }

    /// The declared reusable confinement: the isolation classes an instance
    /// must run behind, a capability ceiling, the restrictions it may not
    /// weaken, the syscall filter it must load, and a umask. It selects the
    /// seeded posture row and names no attachment of its own.
    fn policy() -> SeedPolicy {
        SeedPolicy {
            name: "worker".to_owned(),
            spec: ExecutionPolicySpec::new(
                PolicyNamespaces::new(vec![NamespaceClass::User, NamespaceClass::Mount])
                    .expect("namespace set"),
                PolicyCapabilities::new(vec![CapabilityClass::NetworkBind])
                    .expect("capability ceiling"),
                true,
                PolicyIdentity::new(None, false).expect("identity rules"),
                PolicyRoot::new(true, true),
                PolicySeccomp::new(Some(
                    ResourceRef::parse("SeccompProfile/worker").expect("profile ref"),
                ))
                .expect("syscall filter selection"),
                Some(0o077),
            )
            .expect("policy spec"),
        }
    }


    fn provider() -> SeedProvider {
        SeedProvider {
            provider_ref: ResourceRef::parse("Provider/system-minijail").expect("provider"),
            principals: vec!["d2b-zonert".to_owned()],
            roles: vec![
                ResourceRef::parse("Role/operation-publisher").expect("role"),
                ResourceRef::parse("Role/worker").expect("role"),
            ],
            self_bindings: vec![SeedSelfBinding {
                subject_ref: ResourceRef::parse("Provider/system-minijail").expect("subject"),
                role_ref: ResourceRef::parse("Role/operation-publisher").expect("role"),
            }],
        }
    }

    fn make_declarations(controller: bool) -> FoundationDeclarations {
        FoundationDeclarations {
            providers: vec![provider()],
            profiles: vec![profile()],
            policies: vec![policy()],
            roles: vec![publisher_role(), worker_role()],
            operator_bindings: Vec::new(),
            controller: controller
                .then(|| ResourceRef::parse("Provider/system-minijail").expect("controller")),
        }
    }

    async fn run(
        fixture: &Fixture,
        declarations: FoundationDeclarations,
    ) -> Result<SeedReport, SeedError> {
        FoundationSeed::new(declarations, fixture.allocation.clone())
            .run(&fixture.store, &fixture.providers)
            .await
    }

    async fn row_spec(store: &SpecStore, reference: &str) -> Option<Vec<u8>> {
        let (type_name, name) = reference.split_once('/')?;
        let key = ResourceKey::new(SYSTEM_ZONE, type_name, name);
        store.get(key).await.ok().map(|row| row.spec)
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn the_seed_commits_the_policy_rows_in_declaration_order() {
        let fixture = make_fixture();
        let report = run(&fixture, make_declarations(true))
            .await
            .expect("seed runs");
        assert_eq!(
            report.committed,
            vec![
                "Zone/system",
                "SeccompProfile/worker",
                "ExecutionPolicy/worker",
                "Role/operation-publisher",
                "Role/worker",
                "RoleBinding/system-minijail-self-operation-publisher",
            ]
        );
        // A restart re-seeds idempotently: every row's bytes are current.
        let second = run(
            &fixture,
            make_declarations(true),
        )
        .await
        .expect("second seed runs");
        assert_eq!(second.unchanged, second.committed.len());
    }

    /// The seed resolves every declared reference before its first write:
    /// an unresolved `operationRefs` entry and an unallocated provider
    /// principal are each refused terminally, naming the row and the missing
    /// target.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn unresolved_operation_refs_and_unallocated_principals_are_refused() {
        let fixture = make_fixture();
        let mut declarations = make_declarations(true);
        let role = json!({
            "rules": [{
                "resourceTypes": ["Operation"], "verbs": ["create"], "subresources": [],
                "resourceNames": [], "zones": [], "executionRefs": [], "sessionVerbs": []
            }],
            "operationRefs": ["Operation/absent"],
        });
        declarations.roles[1].spec = serde_json::from_value(role).expect("role with operation ref");
        let error = run(&fixture, declarations)
            .await
            .expect_err("unresolved operation reference");
        assert_eq!(
            error,
            SeedError::UnresolvedRef {
                row: "Role/worker".to_owned(),
                field: "operationRefs",
                missing: "Operation/absent".to_owned(),
            }
        );

        let mut declarations = make_declarations(true);
        declarations.providers[0].principals = vec!["not-allocated".to_owned()];
        let error = run(&fixture, declarations)
            .await
            .expect_err("unallocated principal");
        assert_eq!(
            error,
            SeedError::PrincipalNotAllocated {
                row: "Provider/system-minijail".to_owned(),
                principal: "not-allocated".to_owned(),
            }
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn undeclared_verbs_and_unknown_types_are_refused() {
        let fixture = make_fixture();
        let mut declarations =
            make_declarations(true);
        let role = json!({
            "rules": [
                {
                    "resourceTypes": ["Operation"], "verbs": ["create"], "subresources": [],
                    "resourceNames": [], "zones": [], "executionRefs": [], "sessionVerbs": []
                },
                {
                    "resourceTypes": ["Quota"], "verbs": ["create"], "subresources": [],
                    "resourceNames": [], "zones": [], "executionRefs": [], "sessionVerbs": []
                }
            ],
        });
        declarations.roles[0].spec =
            serde_json::from_value(role).expect("role with undeclared verb");
        let error = run(&fixture, declarations)
            .await
            .expect_err("undeclared verb");
        assert_eq!(
            error,
            SeedError::UndeclaredVerb {
                row: "Role/operation-publisher".to_owned(),
                type_name: "Quota".to_owned(),
                verb: "create".to_owned(),
            }
        );

        let fixture = make_fixture();
        let mut declarations =
            make_declarations(true);
        let role = json!({
            "rules": [
                {
                    "resourceTypes": ["Operation"], "verbs": ["create"], "subresources": [],
                    "resourceNames": [], "zones": [], "executionRefs": [], "sessionVerbs": []
                },
                {
                    "resourceTypes": ["other.d2bus.org.Thing"], "verbs": ["get"], "subresources": [],
                    "resourceNames": [], "zones": [], "executionRefs": [], "sessionVerbs": []
                }
            ],
        });
        declarations.roles[0].spec = serde_json::from_value(role).expect("role with unknown type");
        let error = run(&fixture, declarations)
            .await
            .expect_err("unknown type");
        assert_eq!(
            error,
            SeedError::UnknownResourceType {
                row: "Role/operation-publisher".to_owned(),
                type_name: "other.d2bus.org.Thing".to_owned(),
            }
        );
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn self_binding_scope_escapes_and_scope_outside_the_role_are_refused() {
        let fixture = make_fixture();
        let mut declarations = make_declarations(true);
        declarations.providers[0].self_bindings[0].role_ref =
            ResourceRef::parse("Role/other").expect("role");
        let error = run(&fixture, declarations)
            .await
            .expect_err("escaped binding");
        assert!(matches!(error, SeedError::SelfBindingEscaped { .. }));

        let fixture = make_fixture();
        let mut declarations = make_declarations(true);
        declarations.operator_bindings = vec![SeedBinding {
            name: "operator".to_owned(),
            spec: serde_json::from_value(json!({
                "roleRef": "Role/operation-publisher",
                "subjects": ["User/alice"],
                "resourceRefs": ["Volume/media-library"]
            }))
            .expect("operator binding"),
        }];
        let error = run(&fixture, declarations)
            .await
            .expect_err("scope outside the role");
        assert!(matches!(
            error,
            SeedError::ScopeOutsideRole {
                field: "resourceRefs",
                ..
            }
        ));
    }

    /// A seeded `ExecutionPolicy` row is committed through the same admitted
    /// path as every other foundation row, not waved through: the row's
    /// bytes land, and the one mutation that wrote it is the one the
    /// evaluator admits - under the verified deployment root, and under
    /// nothing else.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn a_seeded_execution_policy_row_is_admitted_not_bypassed() {
        let fixture = make_fixture();
        let report = run(
            &fixture,
            make_declarations(true),
        )
        .await
        .expect("seed runs");
        assert!(
            report.committed.contains(&"ExecutionPolicy/worker".to_owned()),
            "the policy row is committed through the seed: {:?}",
            report.committed
        );
        // The committed bytes are the contract's own row, not a copy.
        let spec = row_spec(&fixture.store, "ExecutionPolicy/worker")
            .await
            .expect("policy row");
        let policy: ExecutionPolicySpec =
            serde_json::from_slice(&spec).expect("committed policy decodes as the contract");
        assert!(policy.no_new_privileges());
        assert_eq!(policy.umask(), Some(0o077));
        assert_eq!(
            policy.seccomp().profile_ref().map(ResourceRef::to_canonical_string),
            Some("SeccompProfile/worker".to_owned()),
            "the policy selects the seeded posture row"
        );

        // The write is a decision, not a bypass: the identical mutation the
        // seed made is admitted against the verified deployment graph by its
        // root, and refused for any other subject, including another
        // unresourced bootstrap identity. That is what makes the commit
        // "admitted" rather than "allowed because the seed wrote it".
        let accepted = verified_deployment_graph().expect("verified deployment graph");
        let target = ResourceRef::parse("ExecutionPolicy/worker").expect("policy reference");
        let request = |subject: AuthoritySubject| {
            GraphMutation::new(
                accepted.zone().clone(),
                MutationSubjectEvidence::new(subject, TransportIdentity::Daemon),
                MutationKind::Create,
                target.clone(),
            )
        };
        assert_eq!(
            GraphAuthority::admit_mutation(
                &request(AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap)),
                &accepted
            ),
            GraphAdmissionDecision::Admitted,
            "the verified deployment root is admitted"
        );
        assert_eq!(
            GraphAuthority::admit_mutation(
                &request(AuthoritySubject::unresourced(AuthoritySubjectKind::Operator)),
                &accepted
            ),
            GraphAdmissionDecision::refuse(
                AdmissionStage::Authorize,
                RefusalReason::IdentityNotAuthorized
            ),
            "a privileged transport is never a subject, and no other identity is admitted"
        );
    }

    /// Declare-then-validate holds for the new type: a policy selecting a
    /// `SeccompProfile` the seed does not commit is refused before the first
    /// write, and nothing lands in the store.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn an_unresolved_execution_policy_selection_is_refused_before_any_write() {
        let fixture = make_fixture();
        let mut declarations = make_declarations(true);
        declarations.policies[0].spec = ExecutionPolicySpec::new(
            PolicyNamespaces::new(vec![NamespaceClass::User]).expect("namespace set"),
            PolicyCapabilities::new(Vec::new()).expect("capability ceiling"),
            true,
            PolicyIdentity::new(None, false).expect("identity rules"),
            PolicyRoot::new(true, false),
            PolicySeccomp::new(Some(
                ResourceRef::parse("SeccompProfile/absent").expect("profile ref"),
            ))
            .expect("syscall filter selection"),
            None,
        )
        .expect("policy spec");
        let error = run(&fixture, declarations)
            .await
            .expect_err("an unresolved policy selection");
        assert_eq!(
            error,
            SeedError::UnresolvedRef {
                row: "ExecutionPolicy/worker".to_owned(),
                field: "seccomp.profileRef",
                missing: "SeccompProfile/absent".to_owned(),
            }
        );
        assert!(
            row_spec(&fixture.store, "ExecutionPolicy/worker")
                .await
                .is_none(),
            "a refused seed leaves the store exactly as it was"
        );
    }

    /// The policy row is system-homed like every other row the foundation
    /// seed commits, so the manager read path falls back to the system zone
    /// for it.
    ///
    /// Membership is what the read path depends on. The write-side refusal for
    /// these types is the graph admission's, which decides from the prior
    /// accepted graph rather than from this list.
    #[test]
    fn an_execution_policy_row_is_system_homed() {
        assert!(SYSTEM_HOMED_TYPES.contains(&EXECUTION_POLICY_RESOURCE_TYPE));
        assert!(
            SYSTEM_HOMED_TYPES.contains(&OPERATION_RESOURCE_TYPE),
            "the materialized spawn operations stay system-homed beside it"
        );
    }
}

// ---------------------------------------------------------------------------
// Verified deployment bootstrap tests (U31)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod deployment_bootstrap_tests {
    use super::*;
    use d2b_contracts_broker::broker_wire::AuthorityCursor;
    use d2b_contracts_resource::v3::{AuthoritySubject, AuthoritySubjectKind};

    const STATE_VOLUME: &str = "Volume/d2b-state";

    /// The publisher Role the fixed foundations declare.
    fn publisher_role() -> serde_json::Value {
        serde_json::json!({
            "rules": [{
                "resourceTypes": ["Operation"],
                "verbs": ["create"],
                "subresources": [],
                "resourceNames": [],
                "zones": [],
                "executionRefs": [],
                "sessionVerbs": []
            }],
            "operationRefs": [],
        })
    }

    /// The Process provider's self-binding.
    fn publisher_binding() -> serde_json::Value {
        serde_json::json!({
            "roleRef": "Role/operation-publisher",
            "subjects": ["Provider/system-minijail"],
        })
    }

    /// A complete, internally consistent verified deployment graph.
    fn graph() -> DeploymentBootstrap {
        let mut graph = DeploymentBootstrap {
            schema_version: DEPLOYMENT_BOOTSTRAP_SCHEMA.to_owned(),
            zone: ZoneId::parse(SYSTEM_ZONE).expect("foundation zone"),
            store_incarnation: StoreIncarnation::parse("foundation-1").expect("store"),
            state_volume: STATE_VOLUME.to_owned(),
            implementations: vec!["activation-nixos".to_owned()],
            roles: vec![BootstrapAuthorityRow {
                reference: "Role/operation-publisher".to_owned(),
                admitted: publisher_role(),
            }],
            role_bindings: vec![BootstrapAuthorityRow {
                reference: "RoleBinding/system-minijail-self-operation-publisher".to_owned(),
                admitted: publisher_binding(),
            }],
            graph_digest: String::new(),
        };
        graph.graph_digest = seal(&graph);
        graph
    }

    /// Hash a document exactly as the deployment's publisher does.
    fn seal(graph: &DeploymentBootstrap) -> String {
        let bytes = DeploymentBootstrap::canonical_bytes_without_digest(graph)
            .expect("canonical bytes without the digest field");
        d2b_contracts_resource::v3::framed_canonical_digest(
            DEPLOYMENT_BOOTSTRAP_DIGEST_DOMAIN,
            &bytes,
        )
    }

    /// Render a document the way the Nix publisher writes it.
    fn render(graph: &DeploymentBootstrap) -> Vec<u8> {
        serde_json::to_vec(graph).expect("render graph")
    }

    fn reference(value: &str) -> ResourceRef {
        ResourceRef::parse(value).expect("canonical reference")
    }

    // -- Scenario 2: three distinct startup refusals ------------------------

    /// A graph whose authority rows were edited after verification is refused
    /// with a digest mismatch, and the mismatch is distinguishable from the
    /// other two refusals.
    #[test]
    fn a_tampered_deployment_graph_refuses_startup() {
        let mut tampered = graph();
        // Widen the publisher role after the digest was taken: the row now
        // grants every Operation verb rather than only `create`.
        tampered.roles[0].admitted["rules"][0]["verbs"] =
            serde_json::json!(["get", "list", "create", "delete"]);
        assert_eq!(
            tampered.verify(),
            Err(BootstrapRefusal::DigestMismatch {
                claimed: tampered.graph_digest.clone(),
            }),
            "an edited row must fail closed before any provider is published"
        );
    }

    /// A document carrying another contract version is refused, and refused
    /// differently from a tampered one: the bytes may be perfectly
    /// self-consistent and the deployment still refuses.
    #[test]
    fn an_old_artifact_refuses_startup() {
        let mut old = graph();
        old.schema_version = "d2b-deployment-bootstrap/0".to_owned();
        old.graph_digest = seal(&old);
        assert_eq!(
            old.verify(),
            Err(BootstrapRefusal::SchemaUnsupported {
                observed: "d2b-deployment-bootstrap/0".to_owned(),
            }),
            "another contract version is refused even when the digest matches"
        );
    }

    /// A document naming an implementation this build does not compile is
    /// refused even though its bytes verify, which is the whole point of
    /// binding the generated declarations instead of an allowlist.
    #[test]
    fn an_unknown_implementation_refuses_startup() {
        let mut unknown = graph();
        unknown.implementations.push("provider-from-another-release".to_owned());
        unknown.graph_digest = seal(&unknown);
        assert_eq!(
            unknown.publish(&core_declarations()).err(),
            Some(BootstrapRefusal::UnknownImplementation {
                implementation: "provider-from-another-release".to_owned(),
            }),
            "an implementation no compiled declaration binds must refuse"
        );
    }

    // -- Scenario 5: no silent permissive fallback --------------------------

    /// A verified graph missing the foundation RoleBinding refuses startup
    /// instead of producing an admission. The permissive path is shown to be
    /// unreachable, not merely unused: the same Create mutation the
    /// foundations need is Refused against the incomplete graph even from the
    /// deployment root, and Admitted against the complete one.
    #[test]
    fn a_graph_missing_a_foundation_binding_refuses_instead_of_admitting_everything() {
        let mut incomplete = graph();
        incomplete.role_bindings.clear();
        incomplete.graph_digest = seal(&incomplete);
        let declarations = core_declarations();
        assert_eq!(
            incomplete.publish(&declarations).err(),
            Some(BootstrapRefusal::MissingFoundationBinding {
                reference: "RoleBinding/system-minijail-self-operation-publisher".to_owned(),
            }),
            "startup refuses rather than falling back to a permissive admission"
        );
        // The permissive alternative is genuinely unreachable: the same
        // mutation is refused against the incomplete graph and admitted
        // against the complete one.
        let target = reference("Operation/process-run-virtiofsd-worker");
        // The subject is the Process provider itself, not the deployment
        // root: the deployment root bootstraps the graph and is admitted
        // without a grant by design, so only a provider subject shows
        // whether the graph's own RoleBindings carry the authority.
        let provider = AuthoritySubject::named(
            AuthoritySubjectKind::Provider,
            reference("Provider/system-minijail"),
        );
        let evidence = MutationSubjectEvidence::new(provider, TransportIdentity::Daemon);
        let decide = |accepted: &AcceptedGraph| {
            GraphAuthority::admit_mutation(
                &GraphMutation::new(
                    accepted.zone().clone(),
                    evidence.clone(),
                    MutationKind::Create,
                    target.clone(),
                ),
                accepted,
            )
        };
        assert!(
            matches!(
                decide(&incomplete.accepted_graph().expect("decoded graph")),
                GraphAdmissionDecision::Refused { .. }
            ),
            "an incomplete graph grants nothing, so no AllowAll-equivalent answer exists"
        );
        assert!(
            matches!(
                decide(&graph().accepted_graph().expect("decoded graph")),
                GraphAdmissionDecision::Admitted
            ),
            "the complete verified graph admits the same mutation"
        );
    }

    // -- Scenario 1: ordering and the state-Volume cycle --------------------

    /// Empty fresh state publishes every fixed foundation before any declared
    /// provider, and the deployment's own state Volume is one of them, so no
    /// provider waits on a row only a provider could create.
    #[test]
    fn fresh_state_publishes_the_foundations_before_the_declared_providers() {
        let published = graph().publish(&core_declarations()).expect("publishes");
        let plan = published.plan();
        let state_volume = plan
            .position(STATE_VOLUME)
            .expect("the deployment's state Volume is published");
        let foundations = plan.layer(PublicationLayer::Foundations);
        assert!(
            foundations.iter().all(|step| step.layer == PublicationLayer::Foundations),
            "the foundation layer holds only foundation steps"
        );
        let first_provider = plan
            .steps()
            .iter()
            .position(|step| step.layer == PublicationLayer::DeclaredProviders)
            .expect("at least one declared provider step");
        assert!(
            state_volume < first_provider,
            "the state Volume publishes before any declared provider, so no provider waits on it"
        );
        // Every provider step names the state Volume it reads.
        for step in plan.layer(PublicationLayer::DeclaredProviders) {
            assert!(
                step.requires.iter().any(|requirement| requirement == STATE_VOLUME),
                "{} reads the deployment's state Volume",
                step.reference
            );
        }
        assert!(
            plan.verify().is_ok(),
            "an ordered plan whose requirements all point backwards cannot contain a cycle"
        );
        plan.verify().expect("the published plan is orderable");
    }

    /// The cycle check is real, not decorative: a plan in which a provider
    /// publishes before the state Volume it requires is refused, naming the
    /// exact step and requirement that close the cycle.
    #[test]
    fn a_state_volume_cycle_is_refused_by_name() {
        let steps = vec![
            PublicationStep::step(
                PublicationLayer::DeclaredProviders,
                "volume-local",
                vec![STATE_VOLUME.to_owned()],
            ),
            PublicationStep::foundation(STATE_VOLUME),
        ];
        assert_eq!(
            PublicationPlan::new(steps),
            Err(BootstrapRefusal::PublicationCycle {
                step: "volume-local".to_owned(),
                requirement: STATE_VOLUME.to_owned(),
            }),
            "a provider that needs a row published after it is refused, not retried"
        );
    }

    /// A requirement the plan never publishes at all is a different refusal
    /// from a cycle, so an operator can tell an unresolvable reference from an
    /// unorderable one.
    #[test]
    fn an_unresolvable_requirement_is_refused_separately_from_a_cycle() {
        let steps = vec![PublicationStep::step(
            PublicationLayer::Foundations,
            "Role/orphan",
            vec!["Volume/absent".to_owned()],
        )];
        assert_eq!(
            PublicationPlan::new(steps),
            Err(BootstrapRefusal::UnresolvedRequirement {
                step: "Role/orphan".to_owned(),
                requirement: "Volume/absent".to_owned(),
            }),
        );
    }

    // -- Scenario 2 (decode): the bytes on disk are the authority -----------

    /// The document the daemon reads is bounded and self-verifying: an absent
    /// or empty file is a refusal, and the rendered bytes verify.
    #[test]
    fn the_deployment_root_document_round_trips_through_its_own_digest() {
        let graph = graph();
        let decoded =
            DeploymentBootstrap::decode(&render(&graph), DEPLOYMENT_BOOTSTRAP_FILE)
                .expect("a self-consistent document verifies");
        assert_eq!(decoded, graph);
        assert_eq!(
            DeploymentBootstrap::decode(b"", DEPLOYMENT_BOOTSTRAP_FILE),
            Err(BootstrapRefusal::DocumentUnreadable {
                path: DEPLOYMENT_BOOTSTRAP_FILE.to_owned(),
            }),
            "an absent document leaves the daemon with no accepted root"
        );
    }

    // -- Scenario 4: a running broker switches accepted deployment identity --

    /// A broker that already holds an accepted identity takes a new one
    /// through freeze, commit, publish, acknowledge - and every forbidden
    /// transition is refused rather than ordered around.
    #[test]
    fn a_running_broker_switches_accepted_deployment_identity_through_the_frozen_protocol() {
        let initial = DeploymentIdentity::initial(
            ZoneId::parse(SYSTEM_ZONE).expect("zone"),
            StoreIncarnation::parse("foundation-1").expect("store"),
        );
        let published = graph().publish(&core_declarations()).expect("publishes");
        let prior = published.accepted().clone();
        let mut switch = DeploymentIdentitySwitch::open(initial.clone());

        // A new identity over the same Zone and incarnation, at a strictly
        // higher sequence.
        let next = published.identity(
            AuthorityCursor {
                sequence: AuthorityCursor::initial()
                    .sequence
                    .try_next()
                    .expect("the sequence advances"),
                digest: DesiredDigest::of(b"deployment-update"),
            },
            DesiredDigest::of(b"deployment-update"),
        );
        let rows = vec![reference("Role/operation-publisher")];
        switch
            .begin(next.clone(), &rows, &prior)
            .expect("the candidate rows are admitted against the prior accepted graph");
        assert_eq!(switch.stage(), PublicationStage::Prepare);
        // Publishing before the freeze is refused: the Zone is not frozen, so
        // a new identity could not be ordered against concurrent use.
        assert_eq!(
            switch.commit(),
            Err(SwitchRefusal::OutOfOrder {
                stage: PublicationStage::Prepare,
                action: "commit",
            }),
            "a running broker does not skip the freeze"
        );
        switch.freeze().expect("freeze");
        assert_eq!(switch.stage(), PublicationStage::Commit);
        // A second switch while one is in flight is refused.
        assert_eq!(
            switch.begin(next.clone(), &rows, &prior),
            Err(SwitchRefusal::Busy),
            "the broker serializes one authority switch at a time"
        );
        switch.commit().expect("commit");
        switch.publish().expect("publish");
        assert_eq!(switch.stage(), PublicationStage::Acknowledge);
        let acknowledged = switch.acknowledge().expect("acknowledge");
        assert_eq!(acknowledged, next, "the acknowledged identity becomes accepted");
        assert_eq!(switch.accepted(), &next);
        assert_eq!(switch.stage(), PublicationStage::Idle);
    }

    /// The identity rules are refusals, not clamps: a different store
    /// incarnation needs an explicit reset, and a cursor below the accepted
    /// sequence never reaches publication.
    #[test]
    fn a_refused_identity_switch_leaves_the_accepted_identity_in_place() {
        let initial = DeploymentIdentity::initial(
            ZoneId::parse(SYSTEM_ZONE).expect("zone"),
            StoreIncarnation::parse("foundation-1").expect("store"),
        );
        let prior = graph().accepted_graph().expect("decoded graph");
        let mut switch = DeploymentIdentitySwitch::open(initial.clone());

        let mut other_store = initial.clone();
        other_store.store_incarnation = StoreIncarnation::parse("foundation-2").expect("store");
        assert_eq!(
            switch.begin(other_store, &[], &prior),
            Err(SwitchRefusal::IncarnationMismatch {
                accepted: "foundation-1".to_owned(),
                observed: "foundation-2".to_owned(),
            }),
            "a store incarnation is an identity, not an ordered counter"
        );
        assert_eq!(switch.accepted(), &initial, "a refused switch changes nothing");

        // A candidate at the accepted sequence but a different digest
        // contradicts what the broker already accepted, so it never reaches
        // publication.
        let mut contradictory = initial.clone();
        contradictory.snapshot_digest = DesiredDigest::of(b"another-deployment");
        switch.begin(contradictory, &[], &prior).expect("begin");
        switch.freeze().expect("freeze");
        assert_eq!(
            switch.commit(),
            Err(SwitchRefusal::DigestContradiction { sequence: 0 }),
            "a candidate contradicting the accepted digest at its own sequence is refused"
        );
        assert_eq!(
            switch.accepted(),
            &initial,
            "the accepted identity is untouched by the refused switch"
        );
    }

    // -- The producer: what the verified document publishes ---------------

    /// The published row set is the document's own authority rows, each
    /// carrying the bytes the broker accepted and the digest over exactly
    /// those bytes, so the two legs cannot disagree about what was published.
    #[test]
    fn the_published_rows_are_the_documents_own_committed_bytes() {
        let published = graph().publish(&core_declarations()).expect("publishes");
        let rows = published.authority_rows();
        assert_eq!(
            rows.iter()
                .map(|row| row.resource_ref.to_canonical_string())
                .collect::<Vec<_>>(),
            [
                "Role/operation-publisher",
                "RoleBinding/system-minijail-self-operation-publisher"
            ],
            "the set is the document's authority rows and nothing else"
        );
        for row in rows {
            assert_eq!(
                row.desired_digest,
                DesiredDigest::of(&row.admitted.to_canonical_bytes()),
                "each row's digest covers the bytes the broker stores"
            );
            assert_eq!(
                row.desired_revision,
                DesiredRevision::INITIAL,
                "a cold start installs each row at the revision it committed at"
            );
        }
    }

    /// The snapshot a cold start installs stands at the initial cursor, where
    /// a broker holding no accepted projection already is, and its identity
    /// names the deployment root the broker admits a bootstrap subject
    /// against.
    #[test]
    fn a_cold_start_snapshot_installs_the_accepted_root() {
        let published = graph().publish(&core_declarations()).expect("publishes");
        let (identity, snapshot) = published.authority_publication();
        assert_eq!(snapshot.zone, SYSTEM_ZONE);
        assert_eq!(snapshot.store_incarnation.as_str(), "foundation-1");
        assert_eq!(snapshot.cursor, AuthorityCursor::initial());
        assert_eq!(snapshot.outstanding, None);
        assert_eq!(
            identity.root_subject,
            AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap)
        );
        assert_eq!(
            identity.snapshot_digest,
            publication_snapshot_digest(&snapshot),
            "the identity names the digest of the document it publishes"
        );
        assert_eq!(
            snapshot.rows,
            published.authority_rows(),
            "the identity and the snapshot describe one document"
        );
    }

    /// A row whose relationship the verified document resolved nothing for
    /// is published without an identity rather than with one invented here.
    /// The broker then holds no key for it, which refuses that relationship -
    /// the correct answer for an absence, and never a fabricated grant.
    #[test]
    fn a_binding_row_the_document_resolved_nothing_for_is_published_unresolved() {
        let mut document = graph();
        document.role_bindings.push(BootstrapAuthorityRow {
            reference: "VolumeBinding/d2b-state".to_owned(),
            admitted: serde_json::json!({
                "volumeRef": "Volume/d2b-state",
                "executionRef": "Host/work",
                "slot": "d2b-state",
                "rights": "Consume",
                "requiredFacets": [],
                "source": {
                    "admittedRights": ["Consume"],
                    "arbitration": "Shared",
                    "realizedFacets": []
                }
            }),
        });
        document.graph_digest = seal(&document);
        let published = document.publish(&core_declarations()).expect("publishes");
        let row = published
            .authority_rows()
            .iter()
            .find(|row| row.resource_ref.to_canonical_string() == "VolumeBinding/d2b-state")
            .expect("the binding row travels");
        assert_eq!(
            (row.source_uid.clone(), row.consumer_uid.clone()),
            (None, None),
            "no uid is invented for a relationship the verified document never resolved"
        );
    }
}
