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
//! to a system-homed type ([`SystemZoneWriteFence`]) with the same terminal,
//! named refusal shape the plane partition uses.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts_resource::v3::{
    CommandSpec, OperationSpec, PayloadSchema, ResourceRef, SeccompProfileSpec,
    SECCOMP_PROFILE_RESOURCE_TYPE, canonical_json_bytes,
};
use d2b_contracts_zone_session::v3::{RoleBindingSpec, RoleResourceVerb, RoleSpec};
use d2b_resource_runtime::identity::ResourceTypeName;
use d2b_resource_runtime::manager::{
    AdmissionDecision, MutationAdmission, MutationRequest, MutationSubject, deterministic_uid,
};
use d2b_resource_runtime::provider::ProviderDirectory;
use d2b_resource_runtime::spec_store::{
    EnsureOutcome, ResourceKey, ResourceProvenance, SpecSelector, SpecStore, StoredDesiredResource,
};

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
pub const SYSTEM_HOMED_TYPES: &[&str] = &["Command", "Operation", "SeccompProfile"];
/// Maximum bytes of one seeded resource name.
pub const MAX_SEED_NAME_BYTES: usize = 63;
/// The subject types a RoleBinding may grant, resolved by the session layer.
pub const BINDABLE_SUBJECT_TYPES: &[&str] = &[
    "Zone", "User", "Provider", "Host", "Guest", "Process", "Group",
];
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
    /// The role spec: rules, authority facet, and optional posture facet.
    pub spec: RoleSpec,
}

/// One declared SeccompProfile row.
#[derive(Debug, Clone)]
pub struct SeedProfile {
    /// Zone-local profile name.
    pub name: String,
    /// The inline posture content.
    pub spec: SeccompProfileSpec,
}

/// One declared Command row.
#[derive(Debug, Clone)]
pub struct SeedCommand {
    /// Zone-local command name.
    pub name: String,
    /// The declared launch shape.
    pub spec: CommandSpec,
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
    /// Declared roles.
    pub roles: Vec<SeedRole>,
    /// Declared launch shapes.
    pub commands: Vec<SeedCommand>,
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
    let role = RoleSpec::new(vec![rule]).expect("the publisher role is valid");
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
        roles: vec![SeedRole {
            name: "operation-publisher".to_owned(),
            spec: role,
        }],
        commands: Vec::new(),
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
        for role in &self.declarations.roles {
            let row = PendingRow::new("Role", &role.name, encode(&role.spec)?)?;
            committed.insert_row(&row.key, row.spec.clone());
            rows.push(row);
        }
        // 3. Commands.
        for command in &self.declarations.commands {
            let row = PendingRow::new("Command", &command.name, encode(&command.spec)?)?;
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
        // 5. The controller materializes the spawn operations. Each one is
        // authorized against the committed bindings and roles, so the
        // authorization can only come from a committed self-binding.
        let mut materialized_specs = Vec::new();
        let mut materialized = Vec::new();
        for command in &self.declarations.commands {
            let operation = self.materialize(command, &committed)?;
            let row = PendingRow::new(
                "Operation",
                operation.name(),
                encode(operation.spec())?,
            )?;
            committed.insert_row(&row.key, row.spec.clone());
            materialized.push(row.reference());
            materialized_specs.push((row.reference(), operation));
            rows.push(row);
        }
        // 6. Operator bindings from the host contract.
        for binding in &self.declarations.operator_bindings {
            let row = PendingRow::new("RoleBinding", &binding.name, encode(&binding.spec)?)?;
            committed.insert_row(&row.key, row.spec.clone());
            rows.push(row);
        }
        // Declare-then-validate: every reference resolves over the committed
        // set as a whole, before the first write.
        self.validate(&committed, providers, &materialized_specs, &rows)?;
        let mut report = SeedReport {
            committed: Vec::with_capacity(rows.len()),
            materialized,
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

    // -- Materialization ---------------------------------------------------

    /// Build the spawn operation one command materializes, authorized by the
    /// controller's committed self-binding alone.
    fn materialize(
        &self,
        command: &SeedCommand,
        committed: &CommittedSet,
    ) -> Result<MaterializedOperation, SeedError> {
        let controller = self
            .declarations
            .controller
            .as_ref()
            .ok_or(SeedError::UnauthorizedMaterialization {
                controller: "none".to_owned(),
                command: command.name.clone(),
            })?;
        let command_ref = ResourceRef::parse(format!("Command/{}", command.name).as_str())
            .map_err(|_| SeedError::InvalidRow {
                row: "Command",
                name: command.name.clone(),
                reason: "the command name is not a resource name",
            })?;
        if !self.controller_may_materialize(controller, &command_ref, committed) {
            return Err(SeedError::UnauthorizedMaterialization {
                controller: controller.to_canonical_string(),
                command: command_ref.to_canonical_string(),
            });
        }
        let name = materialized_operation_name(&command.name)?;
        let spec = OperationSpec::new(
            Some(command_ref),
            clone_payload(command.spec.params()),
            true,
            secret_access_ceiling(command.spec.params()),
            audit_facet(command.spec.params()),
            None,
            spawn_authority(),
            Default::default(),
            Default::default(),
            d2b_contracts_resource::v3::PayloadProvenance::Derived,
            None,
        )
        .map_err(|_| SeedError::InvalidRow {
            row: "Operation",
            name: name.clone(),
            reason: "the materialized operation facets are invalid",
        })?;
        Ok(MaterializedOperation { name, spec })
    }

    /// Whether one committed self-binding grants the controller `create` on
    /// `Operation` scoped to this command.
    ///
    /// Materialization is authorized by the controller's own self-binding
    /// alone: an operator binding from the host contract is a grant to the
    /// subject it names, never a second way to authorize the seed to write
    /// the operation rows a provider's commands materialize.
    fn controller_may_materialize(
        &self,
        controller: &ResourceRef,
        command: &ResourceRef,
        committed: &CommittedSet,
    ) -> bool {
        let binding_names = self.self_binding_rows();
        for (name, spec) in &binding_names {
            if !spec.subjects().contains(controller) {
                continue;
            }
            let Some(role) = self
                .declarations
                .roles
                .iter()
                .find(|role| format!("Role/{}", role.name) == spec.role_ref().to_canonical_string())
            else {
                continue;
            };
            if !role.spec.command_refs().contains(command) {
                continue;
            }
            let grants_create = role.spec.rules().iter().any(|rule| {
                rule.verbs().contains(&RoleResourceVerb::Create)
                    && rule
                        .resource_types()
                        .iter()
                        .any(|type_name| type_name.as_str() == "Operation")
            });
            if grants_create && committed.contains_str(&role_ref(&role.name)) {
                let _ = name;
                return true;
            }
        }
        false
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
        materialized: &[(String, MaterializedOperation)],
        rows: &[PendingRow],
    ) -> Result<(), SeedError> {
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
            for command in role.spec.command_refs() {
                require_committed(committed, &row, "commandRefs", command)?;
            }
            if let Some(posture) = role.spec.posture() {
                require_committed(committed, &row, "posture.seccompRef", posture.seccomp_ref())?;
                let principal = posture.principal_ref().name().to_owned();
                if !valid_principal_name(&principal)
                    || self.allocation.get(&principal).is_none()
                {
                    return Err(SeedError::PrincipalNotAllocated {
                        row: row.clone(),
                        principal,
                    });
                }
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
        for command in &self.declarations.commands {
            let row = command_ref(&command.name);
            require_committed(committed, &row, "roleRef", command.spec.role_ref())?;
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
        for (row, operation) in materialized {
            let Some(owner) = operation.spec().owner_ref() else {
                continue;
            };
            require_committed(committed, row, "ownerRef", owner)?;
            let Some(command_spec) = self
                .declarations
                .commands
                .iter()
                .find(|candidate| command_ref(&candidate.name) == owner.to_canonical_string())
            else {
                return Err(SeedError::UnresolvedRef {
                    row: row.clone(),
                    field: "ownerRef",
                    missing: owner.to_canonical_string(),
                });
            };
            let declared = encode(command_spec.spec.params())?;
            let materialized = encode(operation.spec().payload_schema())?;
            if declared != materialized {
                return Err(SeedError::MaterializedPayloadDrift { row: row.clone() });
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

    fn contains_str(&self, reference: &str) -> bool {
        self.specs.contains_key(reference)
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

/// One materialized spawn operation.
struct MaterializedOperation {
    name: String,
    spec: OperationSpec,
}

impl MaterializedOperation {
    fn name(&self) -> &str {
        &self.name
    }

    fn spec(&self) -> &OperationSpec {
        &self.spec
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
        "Command" => "Command",
        "Operation" => "Operation",
        "SeccompProfile" => "SeccompProfile",
        _ => "unknown",
    }
}

fn role_ref(name: &str) -> String {
    format!("Role/{name}")
}

fn command_ref(name: &str) -> String {
    format!("Command/{name}")
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

/// The canonical operation name one command materializes.
///
/// `Operation/process-run-<command>`: the resource-name grammar has no dot,
/// so the dotted spawn-operation spelling of the design is carried by the
/// hyphen. A command whose materialized name would exceed the bound refuses.
fn materialized_operation_name(command: &str) -> Result<String, SeedError> {
    let name = format!("process-run-{command}");
    if !valid_resource_name(&name) {
        return Err(SeedError::MaterializedNameOverBound {
            command: command.to_owned(),
        });
    }
    Ok(name)
}

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

fn clone_payload(schema: &PayloadSchema) -> PayloadSchema {
    PayloadSchema::parse(schema.as_value().clone()).expect("a validated payload schema clones")
}

/// The secret-access ceiling a payload implies: a payload with write-only
/// fields needs at least redacted access, a plain payload none.
fn secret_access_ceiling(
    schema: &PayloadSchema,
) -> d2b_contracts_resource::v3::SecretAccess {
    if schema
        .property_names()
        .any(|name| schema.is_write_only(name))
    {
        d2b_contracts_resource::v3::SecretAccess::RedactedOnly
    } else {
        d2b_contracts_resource::v3::SecretAccess::None
    }
}

fn audit_facet(schema: &PayloadSchema) -> d2b_contracts_resource::v3::OperationAudit {
    use d2b_contracts_resource::v3::{
        AuditMode, BoundedText, BoundedToken, OperationAudit,
    };
    let retained = schema
        .property_names()
        .filter(|name| !schema.is_write_only(name))
        .map(|name| BoundedText::parse(name).expect("payload property names are bounded text"))
        .collect::<Vec<_>>();
    let redaction = schema
        .property_names()
        .filter(|name| schema.is_write_only(name))
        .map(|name| BoundedText::parse(name).expect("payload property names are bounded text"))
        .collect::<Vec<_>>();
    OperationAudit::new(
        true,
        AuditMode::Yes,
        retained,
        redaction,
        BoundedToken::parse("spawn").expect("static token"),
    )
    .expect("bounded audit facet")
}

fn spawn_authority() -> d2b_contracts_resource::v3::OperationAuthority {
    use d2b_contracts_resource::v3::{
        BoundedText, BrokerRequirement, OperationAuthority, OperationDomain, OperationSurface,
    };
    OperationAuthority::new(
        OperationSurface::Broker,
        OperationDomain::Host,
        BoundedText::parse("process-controller").expect("static text"),
        BrokerRequirement::Yes,
    )
}

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
    /// Materialization was attempted without the controller's self-binding
    /// authorizing it.
    UnauthorizedMaterialization {
        /// The controller identity.
        controller: String,
        /// The command being materialized.
        command: String,
    },
    /// A command's materialized operation name exceeds the resource-name
    /// bound.
    MaterializedNameOverBound {
        /// The command.
        command: String,
    },
    /// One row reference is declared twice.
    DuplicateRow(String),
    /// A materialized operation's payload schema diverged from its command's.
    MaterializedPayloadDrift {
        /// The materialized operation.
        row: String,
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
            Self::UnauthorizedMaterialization { controller, command } => write!(
                formatter,
                "foundation seed refused materializing {command}: {controller}'s self-binding \
                 does not grant it"
            ),
            Self::MaterializedNameOverBound { command } => write!(
                formatter,
                "foundation seed refused materializing {command}: the operation name exceeds the \
                 resource-name bound"
            ),
            Self::DuplicateRow(row) => write!(
                formatter,
                "foundation seed refused {row}: the reference is declared twice"
            ),
            Self::MaterializedPayloadDrift { row } => write!(
                formatter,
                "foundation seed refused {row}: the materialized payload schema drifted from its \
                 command"
            ),
            Self::Encoding => formatter.write_str("foundation seed refused an unencodable row"),
        }
    }
}

impl std::error::Error for SeedError {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::{
        CommandArgvSlot, CommandExec, CommandIntent, DeviceBind, DeviceNodeKind, SeccompCgroups,
        SeccompDeviceAccess, SeccompNamespaces,
    };
    use d2b_contracts_resource::v3::{BoundedText, BoundedToken};
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
            .register_driver(&d2b_provider_command::command_descriptor())
            .expect("Command registers");
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
            d2b_provider_command::command_spec_decoder()
        }

        fn factory(
            &self,
        ) -> std::sync::Arc<dyn d2b_resource_runtime::driver::ResourceDriverFactory> {
            std::sync::Arc::new(d2b_provider_command::CommandDriverFactory::new())
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

    fn payload() -> PayloadSchema {
        PayloadSchema::parse(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["socketPath"],
            "properties": {
                "socketPath": { "type": "string" },
                "supervisorToken": { "type": "string", "writeOnly": true }
            }
        }))
        .expect("payload schema")
    }

    fn command(name: &str, role: &str) -> SeedCommand {
        let spec = CommandSpec::new(
            CommandExec::parse("/usr/lib/d2b/libexec/virtiofsd").expect("exec"),
            vec![
                CommandArgvSlot::parse("--socket-path").expect("slot"),
                CommandArgvSlot::parse("{socketPath}").expect("slot"),
            ],
            payload(),
            ResourceRef::parse(role).expect("role ref"),
            CommandIntent::new(
                BoundedText::parse("<zone>/<command>/<name>").expect("grammar"),
                BoundedToken::parse("per-bundle-entry").expect("mint"),
            ),
        )
        .expect("command spec");
        SeedCommand {
            name: name.to_owned(),
            spec,
        }
    }

    /// The role the process controller runs under: `create` on `Operation`,
    /// scoped to the declared commands.
    fn publisher_role(commands: &[SeedCommand]) -> SeedRole {
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
            "commandRefs": commands
                .iter()
                .map(|command| format!("Command/{}", command.name))
                .collect::<Vec<_>>(),
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
            "posture": {
                "seccompRef": "SeccompProfile/worker",
                "principalRef": "Principal/d2b-zonert",
                "capabilities": [],
                "namespaces": {},
                "mounts": [],
                "umask": null,
                "userNs": false
            },
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
                    d2b_contracts_resource::v3::DeviceNodePath::parse("/dev/null").expect("path"),
                    DeviceNodeKind::Char,
                    1,
                    3,
                    SeccompDeviceAccess::ReadWrite,
                )],
            )
            .expect("profile spec"),
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

    fn make_declarations(commands: Vec<SeedCommand>, controller: bool) -> FoundationDeclarations {
        FoundationDeclarations {
            providers: vec![provider()],
            profiles: vec![profile()],
            roles: vec![publisher_role(&commands), worker_role()],
            commands,
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
    async fn the_seed_commits_the_policy_rows_in_declaration_order() {
        let fixture = make_fixture();
        let commands = vec![command("virtiofsd-worker", "Role/operation-publisher")];
        let report = run(&fixture, make_declarations(commands, true))
            .await
            .expect("seed runs");
        assert_eq!(
            report.committed,
            vec![
                "Zone/system",
                "SeccompProfile/worker",
                "Role/operation-publisher",
                "Role/worker",
                "Command/virtiofsd-worker",
                "RoleBinding/system-minijail-self-operation-publisher",
                "Operation/process-run-virtiofsd-worker",
            ]
        );
        assert_eq!(
            report.materialized,
            vec!["Operation/process-run-virtiofsd-worker"]
        );
        // The materialized operation carries the command's payload contract.
        let operation = row_spec(&fixture.store, "Operation/process-run-virtiofsd-worker")
            .await
            .expect("operation row");
        let spec: OperationSpec = serde_json::from_slice(&operation).expect("operation spec");
        assert_eq!(
            spec.owner_ref().map(ResourceRef::to_canonical_string),
            Some("Command/virtiofsd-worker".to_owned())
        );
        assert_eq!(
            serde_json::to_value(spec.payload_schema()).expect("payload"),
            serde_json::to_value(payload()).expect("payload")
        );
        assert_ne!(
            spec.secret_access(),
            d2b_contracts_resource::v3::SecretAccess::None
        );
        // The role's posture row resolves the committed profile and principal.
        let role = row_spec(&fixture.store, "Role/worker").await.expect("role row");
        let spec: RoleSpec = serde_json::from_slice(&role).expect("role spec");
        assert!(spec.posture().is_some());
        // A restart re-seeds idempotently: every row's bytes are current.
        let second = run(
            &fixture,
            make_declarations(vec![command("virtiofsd-worker", "Role/operation-publisher")], true),
        )
        .await
        .expect("second seed runs");
        assert_eq!(second.unchanged, second.committed.len());
    }

    #[tokio::test]
    async fn an_unresolved_command_role_ref_is_refused() {
        let fixture = make_fixture();
        let error = run(
            &fixture,
            make_declarations(vec![command("virtiofsd-worker", "Role/missing")], true),
        )
        .await
        .expect_err("unresolved role reference");
        assert_eq!(
            error,
            SeedError::UnresolvedRef {
                row: "Command/virtiofsd-worker".to_owned(),
                field: "roleRef",
                missing: "Role/missing".to_owned(),
            }
        );
    }

    #[tokio::test]
    async fn unresolved_seccomp_and_principal_refs_are_refused() {
        let fixture = make_fixture();
        let commands = vec![command("virtiofsd-worker", "Role/operation-publisher")];
        let mut declarations = make_declarations(commands.clone(), true);
        declarations.profiles.clear();
        let error = run(&fixture, declarations)
            .await
            .expect_err("unresolved seccomp reference");
        assert_eq!(
            error,
            SeedError::UnresolvedRef {
                row: "Role/worker".to_owned(),
                field: "posture.seccompRef",
                missing: "SeccompProfile/worker".to_owned(),
            }
        );

        let mut declarations = make_declarations(commands, true);
        let role = json!({
            "rules": [{
                "resourceTypes": ["Operation"], "verbs": ["create"], "subresources": [],
                "resourceNames": [], "zones": [], "executionRefs": [], "sessionVerbs": []
            }],
            "posture": {
                "seccompRef": "SeccompProfile/worker",
                "principalRef": "Principal/not-allocated",
                "capabilities": [], "namespaces": {}, "mounts": [], "umask": null, "userNs": false
            },
        });
        declarations.roles[1].spec = serde_json::from_value(role).expect("role with posture");
        let error = run(&fixture, declarations)
            .await
            .expect_err("unallocated principal");
        assert_eq!(
            error,
            SeedError::PrincipalNotAllocated {
                row: "Role/worker".to_owned(),
                principal: "not-allocated".to_owned(),
            }
        );
    }

    #[tokio::test]
    async fn materialization_requires_the_controller_self_binding() {
        let fixture = make_fixture();
        let commands = vec![command("virtiofsd-worker", "Role/operation-publisher")];
        // No controller identity at all: nothing materializes.
        let error = run(&fixture, make_declarations(commands.clone(), false))
            .await
            .expect_err("missing controller");
        assert!(matches!(
            error,
            SeedError::UnauthorizedMaterialization { .. }
        ));
        // A controller whose role carries no commandRefs cannot materialize.
        let mut unscoped = make_declarations(commands.clone(), true);
        let role = json!({
            "rules": [{
                "resourceTypes": ["Operation"], "verbs": ["create"], "subresources": [],
                "resourceNames": [], "zones": [], "executionRefs": [], "sessionVerbs": []
            }]
        });
        unscoped.roles[0].spec = serde_json::from_value(role).expect("role without commandRefs");
        let error = run(&fixture, unscoped)
            .await
            .expect_err("unscoped controller");
        assert!(matches!(
            error,
            SeedError::UnauthorizedMaterialization { .. }
        ));
        // The declared self-binding authorizes exactly the scoped command.
        let report = run(&fixture, make_declarations(commands, true))
            .await
            .expect("authorized materialization");
        assert_eq!(report.materialized.len(), 1);
    }

    #[tokio::test]
    async fn materialization_is_not_authorized_by_an_operator_binding() {
        let fixture = make_fixture();
        let mut declarations = make_declarations(
            vec![command("virtiofsd-worker", "Role/operation-publisher")],
            true,
        );
        // The controller keeps its identity but loses its self-binding, and
        // an operator binding names the same controller against a role that
        // carries the commandRefs and `create` on Operation: the operator
        // binding is a grant to its subject, not the seed's authority to
        // write the materialized operation rows.
        declarations.providers[0].self_bindings.clear();
        declarations.operator_bindings = vec![SeedBinding {
            name: "operator".to_owned(),
            spec: serde_json::from_value(json!({
                "roleRef": "Role/operation-publisher",
                "subjects": ["Provider/system-minijail"]
            }))
            .expect("operator binding"),
        }];
        let error = run(&fixture, declarations)
            .await
            .expect_err("an operator binding must not authorize materialization");
        assert_eq!(
            error,
            SeedError::UnauthorizedMaterialization {
                controller: "Provider/system-minijail".to_owned(),
                command: "Command/virtiofsd-worker".to_owned(),
            }
        );
    }

    #[tokio::test]
    async fn undeclared_verbs_and_unknown_types_are_refused() {
        let fixture = make_fixture();
        let mut declarations =
            make_declarations(vec![command("virtiofsd-worker", "Role/operation-publisher")], true);
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
            "commandRefs": ["Command/virtiofsd-worker"]
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
            make_declarations(vec![command("virtiofsd-worker", "Role/operation-publisher")], true);
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
            "commandRefs": ["Command/virtiofsd-worker"]
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
    async fn self_binding_scope_escapes_and_scope_outside_the_role_are_refused() {
        let fixture = make_fixture();
        let mut declarations = make_declarations(
            vec![command("virtiofsd-worker", "Role/operation-publisher")],
            true,
        );
        declarations.providers[0].self_bindings[0].role_ref =
            ResourceRef::parse("Role/other").expect("role");
        let error = run(&fixture, declarations)
            .await
            .expect_err("escaped binding");
        assert!(matches!(error, SeedError::SelfBindingEscaped { .. }));

        let fixture = make_fixture();
        let mut declarations = make_declarations(
            vec![command("virtiofsd-worker", "Role/operation-publisher")],
            true,
        );
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
}
