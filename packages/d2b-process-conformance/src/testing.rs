//! Shared test doubles and fixtures for the Process conformance suite.
//!
//! Both Provider crates build their controller over the same scripted
//! effect port so the suite can assert the neutral obligations without a
//! systemd bus, a broker socket, a privileged host, or a real process.

use std::collections::BTreeSet;
use std::sync::Mutex;

use d2b_contracts_resource::v3::execution_policy::{BoundedToken, ExecutionDomain};
use d2b_contracts_resource::v3::{
    ControllerGeneration, FreshnessTuple, ResourceGeneration, ResourceRef, ResourceUid, StoreIncarnation,
    ZoneId, ZoneRevision,
};

use crate::error::ProcessConformanceError;
use crate::identity::{
    ConfigurationDigest, IdentityBinding, ObservedIdentity, PidfdEvidence, ProcessIdentityDigest,
    WaitReapOwner,
};
use crate::port::{AdoptionCandidate, LaunchedProcess, ProcessLaunchEffectPort, StopClass};
use crate::ticket::{
    CompiledDigests, GuestExecutionBinding, LaunchTicket, OperationBinding, ReadinessExpectation,
};

/// Drive a future to completion on the calling thread.
///
/// The conformance suite is hermetic and never waits on I/O or wall time,
/// so a busy-free single-poll driver is sufficient and keeps the crate free
/// of an async runtime dependency. The driver itself lives in
/// `d2b_core::test_support`, once.
pub use d2b_core::test_support::block_on;

/// One recorded effect-port call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PortCall {
    /// [`ProcessLaunchEffectPort::launch`] was called.
    Launch,
    /// [`ProcessLaunchEffectPort::observe`] was called.
    Observe,
    /// [`ProcessLaunchEffectPort::open_pidfd`] was called.
    OpenPidfd,
    /// [`ProcessLaunchEffectPort::stop`] was called.
    Stop(StopClass),
}

/// A scripted, recording [`ProcessLaunchEffectPort`].
#[derive(Debug)]
pub struct ScriptedEffectPort {
    identity: ProcessIdentityDigest,
    launch_observed: ObservedIdentity,
    launch_wait_owner: WaitReapOwner,
    launch_error: Option<ProcessConformanceError>,
    candidate: Option<AdoptionCandidate>,
    calls: Mutex<Vec<PortCall>>,
}

impl ScriptedEffectPort {
    /// Build a port that launches successfully, verifying `verified`.
    pub fn launching(
        verified: impl IntoIterator<Item = IdentityBinding>,
        wait_owner: WaitReapOwner,
    ) -> Self {
        Self {
            identity: ProcessIdentityDigest::from_bytes([0x11; 32]),
            launch_observed: ObservedIdentity::from_verified(verified),
            launch_wait_owner: wait_owner,
            launch_error: None,
            candidate: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Build a port whose launch fails with `error`.
    pub fn failing(error: ProcessConformanceError, wait_owner: WaitReapOwner) -> Self {
        Self {
            identity: ProcessIdentityDigest::from_bytes([0x11; 32]),
            launch_observed: ObservedIdentity::default(),
            launch_wait_owner: wait_owner,
            launch_error: Some(error),
            candidate: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Script an already running process for adoption.
    pub fn with_candidate(
        mut self,
        verified: impl IntoIterator<Item = IdentityBinding>,
        wait_owner: WaitReapOwner,
    ) -> Self {
        self.candidate = Some(AdoptionCandidate {
            identity: self.identity,
            observed: ObservedIdentity::from_verified(verified),
            wait_reap_owner: wait_owner,
        });
        self
    }

    /// Return every recorded call in order.
    ///
    /// The suite drives every port single-threaded (the crate's hand-rolled
    /// poller), so the lock is never contended; `try_lock` keeps the test
    /// double free of blocking locks and fails closed (empty record) if a
    /// future test ever contends.
    pub fn calls(&self) -> Vec<PortCall> {
        self.calls
            .try_lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }

    fn record(&self, call: PortCall) {
        if let Ok(mut calls) = self.calls.try_lock() {
            calls.push(call);
        }
    }
}

impl ProcessLaunchEffectPort for ScriptedEffectPort {
    async fn launch(
        &self,
        _ticket: &LaunchTicket,
    ) -> Result<LaunchedProcess, ProcessConformanceError> {
        self.record(PortCall::Launch);
        if let Some(error) = self.launch_error {
            return Err(error);
        }
        Ok(LaunchedProcess {
            identity: self.identity,
            observed: self.launch_observed.clone(),
            pidfd: PidfdEvidence::held(),
            wait_reap_owner: self.launch_wait_owner,
        })
    }

    async fn observe(
        &self,
        _ticket: &LaunchTicket,
    ) -> Result<Option<AdoptionCandidate>, ProcessConformanceError> {
        self.record(PortCall::Observe);
        Ok(self.candidate.clone())
    }

    async fn open_pidfd(
        &self,
        _candidate: &AdoptionCandidate,
    ) -> Result<PidfdEvidence, ProcessConformanceError> {
        self.record(PortCall::OpenPidfd);
        Ok(PidfdEvidence::held())
    }

    async fn stop(
        &self,
        _identity: &ProcessIdentityDigest,
        class: StopClass,
    ) -> Result<(), ProcessConformanceError> {
        self.record(PortCall::Stop(class));
        Ok(())
    }
}

/// Canonical launch-ticket fixtures.
pub mod fixtures {
    use super::*;

    /// A stable operation UID.
    pub fn operation_uid() -> ResourceUid {
        ResourceUid::parse("6f9619ff-8b86-4d01-b42d-00cf4fc964ff").expect("valid fixture uid")
    }

    fn token(value: &str) -> BoundedToken {
        BoundedToken::parse(value).expect("valid fixture token")
    }

    fn digest(seed: u8) -> ConfigurationDigest {
        ConfigurationDigest::from_bytes([seed; 32])
    }

    /// The canonical compiled digest set.
    pub fn compiled_digests() -> CompiledDigests {
        CompiledDigests {
            sandbox: digest(1),
            budget: digest(2),
            mounts: digest(3),
            devices: digest(4),
            network: digest(5),
            endpoints: digest(6),
            fd_table: digest(7),
        }
    }

    /// A mutable launch-ticket fixture.
    #[derive(Debug, Clone)]
    pub struct TicketBuilder {
        process_ref: ResourceRef,
        execution_ref: ResourceRef,
        domain: ExecutionDomain,
        deadline_ms: u32,
        user_ref: Option<ResourceRef>,
        selected_provider: BoundedToken,
        expected_identity: BTreeSet<IdentityBinding>,
        guest_execution_binding: bool,
        readiness: ReadinessExpectation,
    }

    impl TicketBuilder {
        /// Override the Process reference.
        pub fn process_ref(mut self, value: ResourceRef) -> Self {
            self.process_ref = value;
            self
        }

        /// Override the Host or Guest reference.
        pub fn execution_ref(mut self, value: ResourceRef) -> Self {
            self.execution_ref = value;
            self
        }

        /// Override the execution domain.
        pub fn domain(mut self, value: ExecutionDomain) -> Self {
            self.domain = value;
            self
        }

        /// Override the exact user reference.
        pub fn user_ref(mut self, value: Option<ResourceRef>) -> Self {
            self.user_ref = value;
            self
        }

        /// Override the selected Process Provider.
        pub fn selected_provider(mut self, value: &str) -> Self {
            self.selected_provider = token(value);
            self
        }

        /// Override the expected identity bindings.
        pub fn expected_identity(
            mut self,
            value: impl IntoIterator<Item = IdentityBinding>,
        ) -> Self {
            self.expected_identity = value.into_iter().collect();
            self
        }

        /// Omit the default Guest target binding for negative tests.
        pub fn without_guest_execution_binding(mut self) -> Self {
            self.guest_execution_binding = false;
            self
        }

        /// Override the ticket's readiness expectation.
        pub fn with_readiness(mut self, readiness: ReadinessExpectation) -> Self {
            self.readiness = readiness;
            self
        }

        /// Override the operation deadline in milliseconds (fixture default:
        /// thirty seconds).
        pub fn with_operation_deadline(mut self, deadline_ms: u32) -> Self {
            self.deadline_ms = deadline_ms;
            self
        }

        /// Build the ticket.
        pub fn build(self) -> Result<LaunchTicket, ProcessConformanceError> {
            let is_guest = self.execution_ref.resource_type().as_str() == "Guest";
            let ticket = LaunchTicket::new(
                self.process_ref,
                operation_uid(),
                ResourceGeneration::new(1).expect("nonzero"),
                ControllerGeneration::new(1).expect("nonzero"),
                token("system-systemd"),
                token("controller"),
                token("controller-main"),
                self.execution_ref,
                self.domain,
                self.user_ref,
                self.selected_provider,
                compiled_digests(),
                OperationBinding::new(operation_uid(), self.deadline_ms)?,
                self.expected_identity,
            )?
            .with_readiness(self.readiness);
            if is_guest && self.guest_execution_binding {
                ticket.with_guest_execution_binding(GuestExecutionBinding::new(
                    ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000")
                        .expect("fixture Guest UID"),
                    ConfigurationDigest::from_bytes([8; 32]),
                    d2b_contracts_resource::v3::identity::ReconnectGeneration::new(1)
                        .expect("fixture session generation"),
                    1,
                    ResourceGeneration::new(1).expect("fixture Provider generation"),
                    ControllerGeneration::new(1).expect("fixture controller generation"),
                )?)
            } else {
                Ok(ticket)
            }
        }
    }

    /// A system-domain Host ticket selecting `system-systemd`.
    pub fn ticket_builder() -> TicketBuilder {
        TicketBuilder {
            process_ref: ResourceRef::parse("Process/controller-main").expect("valid fixture ref"),
            execution_ref: ResourceRef::parse("Host/host-system").expect("valid fixture ref"),
            domain: ExecutionDomain::System,
            deadline_ms: 30_000,
            user_ref: None,
            selected_provider: token("system-systemd"),
            expected_identity: BTreeSet::from([IdentityBinding::Cgroup]),
            guest_execution_binding: true,
            readiness: ReadinessExpectation::None,
        }
    }
}


/// Fixtures for the one resolved Process execution plan.
///
/// Every value here is built the way production builds it: an accepted graph
/// with a real `Role`, `RoleBinding`, and source decision, a private execution
/// table with the plan's own types, and a typed invocation composed by
/// [`d2b_core::execution_plan::resolve_execution_plan`]. Nothing constructs a
/// resolved plan directly, so a fixture that resolves is evidence that the
/// real resolution path admits the request rather than evidence that a
/// hand-built struct would.
pub mod plan_fixtures {
    use super::*;
    use d2b_contracts_resource::v3::binding::{
        BindingArbitration, BindingAuthorization, BindingKey, BindingKind,
        BindingRealizationFacet, BindingRealizationSupport, BindingSlot, RequestedRights,
        SourceAdmission, SourceReservation, admit_binding_request,
    };
    use d2b_contracts_resource::v3::process::NamespaceClass;
    use d2b_contracts_resource::v3::execution_policy_resource::{
        ALL_CONFINEMENT_FACETS, BackendSupport, BudgetCeiling, BudgetRequest, ConfinementFacet,
        ExecutionInstance, ExecutionInstanceKind, ExecutionPolicySpec, ExecutionRequirements,
        PolicyCapabilities, PolicyIdentity, PolicyNamespaces, PolicyRoot,
        PolicySeccomp,
    };
    use d2b_contracts_resource::v3::operation::{
        CallableOperation, OperationImplementation, PayloadProvenance,
    };
    use d2b_contracts_resource::v3::{
        AuditMode, AuthoritySubject, AuthoritySubjectKind, BrokerRequirement, DesiredDigest,
        DesiredRevision, OperationAudit, OperationAuthority, OperationBounds, OperationDomain,
        OperationFds, OperationSurface, PayloadSchema, ResourceTypeName, SecretAccess,
    };
    use d2b_core::execution_plan::{
        BindingPlanRequest, EffectPlanRequest, ExecutionPlan, PrivateBacking, PrivateExecutionTable,
        PrivatePath, PlannedDestination, PlannedExecutable, PlannedIdentity, PlannedSource,
        PlannedView, admit_parameters, resolve_execution_plan,
    };
    use d2b_core::resource_authority::{AcceptedGraph, TransportIdentity};

    use super::fixtures::operation_uid;
    use crate::plan::{BindingPreparation, PreparedBinding, ProcessResourceRequest, ProcessSubject};

    /// The Zone every plan fixture resolves in.
    pub const ZONE: &str = "pubzone";
    /// The store generation every plan fixture resolves in.
    pub const STORE: &str = "store-generation-1";
    /// The exact Volume a storage fixture binds.
    pub const SOURCE: &str = "Volume/data";
    /// The `Operation` a Process launch is admitted under.
    pub const OPERATION: &str = "Operation/launch-worker";
    /// The Process Provider that owns the launch template.
    pub const PROVIDER: &str = "Provider/process";
    /// The stable consumer slot a Process claims.
    pub const SLOT: &str = "root";
    /// The `ResourceType` a Volume relationship's own row carries.
    pub const BINDING_ROW_TYPE: &str = "VolumeBinding";
    /// The named view a storage fixture presents.
    pub const VIEW: &str = "root";
    /// The source path the broker privately resolves.
    pub const SOURCE_PATH: &str = "/var/lib/d2b/volumes/data";
    /// The view path the broker privately resolves.
    pub const VIEW_PATH: &str = "/var/lib/d2b/volumes/data/root";
    /// The destination the broker privately resolves for the consumer.
    pub const DESTINATION_PATH: &str = "/run/d2b/pubzone/worker/mnt/data";
    /// The program the broker privately resolves for the template.
    pub const PROGRAM: &str = "/nix/store/2r1m-worker/bin/worker";
    /// The Volume's store-assigned identity.
    pub const SOURCE_UID: &str = "11111111-1111-4111-8111-111111111111";
    /// The consumer's store-assigned identity.
    pub const CONSUMER_UID: &str = "22222222-2222-4222-8222-222222222222";

    fn reference(value: &str) -> ResourceRef {
        ResourceRef::parse(value).expect("the fixture references are canonical")
    }

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).expect("the fixture uids are canonical")
    }

    fn token(value: &str) -> BoundedToken {
        BoundedToken::parse(value).expect("the fixture tokens are canonical")
    }

    fn zone() -> ZoneId {
        ZoneId::parse(ZONE).expect("the fixture Zone is canonical")
    }

    fn incarnation() -> StoreIncarnation {
        StoreIncarnation::parse(STORE).expect("the fixture store generation is bounded")
    }

    fn slot() -> BindingSlot {
        BindingSlot::parse(SLOT).expect("the fixture slot is a bounded token")
    }

    /// The consumer identity every fixture binds against.
    pub fn consumer() -> ResourceRef {
        reference("Process/worker")
    }

    /// The committed consumer identity for a row of either lifetime.
    pub fn subject(reference_value: &str) -> ProcessSubject {
        ProcessSubject::new(
            reference(reference_value),
            uid(CONSUMER_UID),
            zone(),
            ZoneRevision::new(1),
        )
        .expect("an execution instance reference")
    }

    /// The exact relationship a storage fixture claims.
    pub fn volume_key(consumer_ref: &ResourceRef) -> BindingKey {
        BindingKey::new(
            zone(),
            BindingKind::Volume,
            reference(SOURCE),
            uid(SOURCE_UID),
            consumer_ref.clone(),
            uid(CONSUMER_UID),
            slot(),
        )
        .expect("a well-formed relationship key")
    }

    /// The typed claim a Process row makes for its storage.
    pub fn volume_claim(consumer_ref: &ResourceRef) -> ProcessResourceRequest {
        ProcessResourceRequest::new(
            volume_key(consumer_ref),
            RequestedRights::Observe,
            vec![BindingRealizationFacet::FilesystemPresentation],
            None,
        )
    }

    /// A committed row state for one identity.
    pub fn freshness(resource_uid: &str, revision: u64, digest: &str) -> FreshnessTuple {
        let mut wanted = DesiredRevision::INITIAL;
        for _ in 0..revision {
            wanted = wanted.try_next().expect("the desired revision has room");
        }
        FreshnessTuple::new(
            zone(),
            incarnation(),
            reference(SOURCE),
            uid(resource_uid),
            wanted,
            DesiredDigest::of(digest.as_bytes()),
        )
    }

    /// The committed state of the storage source.
    pub fn source_freshness() -> FreshnessTuple {
        freshness(SOURCE_UID, 1, "volume-1")
    }

    /// The committed state of the consumer row.
    pub fn consumer_freshness() -> FreshnessTuple {
        freshness(CONSUMER_UID, 1, "consumer-1")
    }

    /// The source's own decision over the fixture relationship.
    pub fn source_decision(consumer_ref: &ResourceRef) -> SourceAdmission {
        SourceAdmission::new(
            volume_key(consumer_ref),
            vec![RequestedRights::Observe],
            BindingArbitration::Shared,
        )
        .expect("the source decision is well formed")
    }

    /// The realization support the fixture backend declares.
    pub fn support() -> BindingRealizationSupport {
        BindingRealizationSupport::new(vec![BindingRealizationFacet::FilesystemPresentation])
            .expect("the realization support is unique")
    }

    /// The admission the source-side path produces for the fixture.
    pub fn volume_admission(
        consumer_ref: &ResourceRef,
    ) -> d2b_contracts_resource::v3::binding::BindingAdmission {
        admit_binding_request(
            &volume_key(consumer_ref),
            RequestedRights::Observe,
            &[BindingRealizationFacet::FilesystemPresentation],
            &BindingAuthorization::granted(),
            &source_decision(consumer_ref),
            &support(),
            &[source_freshness(), consumer_freshness()],
        )
        .expect("an admitted relationship")
    }

    /// The source-owned reservation an admitted relationship holds.
    pub fn volume_reservation() -> SourceReservation {
        SourceReservation::new(zone(), uid(SOURCE_UID), token("reservation-1"))
    }

    /// The prepared evidence a storage fixture carries once its source side is
    /// complete.
    pub fn prepared_binding(
        consumer_ref: &ResourceRef,
        preparation: BindingPreparation,
    ) -> PreparedBinding {
        PreparedBinding::new(
            reference(SOURCE),
            uid(SOURCE_UID),
            RequestedRights::Observe,
            vec![BindingRealizationFacet::FilesystemPresentation],
            volume_admission(consumer_ref),
            volume_reservation(),
            "binding-request-digest-fixture".to_owned(),
            preparation,
        )
        .expect("a well-formed prepared relationship")
    }

    /// The prior accepted graph: a real `Role`, a real `RoleBinding` naming
    /// the consumer and the binding row, and a real source decision.
    pub fn accepted_graph(consumer_ref: &ResourceRef) -> AcceptedGraph {
        d2b_core::test_support::AcceptedRelationship::new(
            zone(),
            incarnation(),
            ResourceTypeName::parse(BINDING_ROW_TYPE)
                .expect("the binding row type is a standard type"),
            consumer_ref.clone(),
            source_decision(consumer_ref),
            support(),
        )
        .accepted_graph()
    }

    fn subject_of(consumer_ref: &ResourceRef) -> AuthoritySubject {
        AuthoritySubject::named(
            AuthoritySubjectKind::of_reference(consumer_ref)
                .unwrap_or(AuthoritySubjectKind::Process),
            consumer_ref.clone(),
        )
    }

    /// The committed `Operation` contract a Process launch is admitted under.
    pub fn operation() -> CallableOperation {
        let payload = PayloadSchema::parse(serde_json::json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "servingWorker": { "type": "boolean" } },
        }))
        .expect("the fixture payload schema validates");
        let audit =
            OperationAudit::new(true, AuditMode::Yes, Vec::new(), Vec::new(), token("worker"))
                .expect("the audit facet is bounded");
        let authority = OperationAuthority::new(
            OperationSurface::Broker,
            OperationDomain::Host,
            d2b_contracts_resource::v3::execution_policy::BoundedText::parse("d2b-launcher")
                .expect("bounded text without control characters"),
            BrokerRequirement::Yes,
        );
        let fds = OperationFds::new(Vec::new(), Vec::new(), Vec::new())
            .expect("the fd contract is bounded");
        CallableOperation::new(
            OperationImplementation::trusted_executable_template(
                reference(PROVIDER),
                token("worker"),
            )
            .expect("a Provider reference is a declared implementation"),
            payload,
            None,
            true,
            SecretAccess::None,
            audit,
            None,
            authority,
            fds,
            OperationBounds::default(),
            PayloadProvenance::Request,
        )
        .expect("the fixture operation contract is well formed")
    }

    /// The broker's own private execution values for the fixture.
    pub fn private_table(consumer_ref: &ResourceRef) -> PrivateExecutionTable {
        let source = PlannedSource::new(
            reference(SOURCE),
            uid(SOURCE_UID),
            BindingKind::Volume,
            source_freshness(),
            PrivateBacking::Filesystem,
            PrivatePath::parse(SOURCE_PATH).expect("an absolute private path"),
            vec![PlannedView::new(
                token(VIEW),
                RequestedRights::Observe,
                PrivatePath::parse(VIEW_PATH).expect("an absolute path"),
            )],
        )
        .expect("the resolved source is well formed");
        let identity = PlannedIdentity::new(consumer_ref.clone(), 4242, 4242, vec![], None)
            .expect("the resolved identity is well formed");
        let destination = PlannedDestination::new(
            volume_key(consumer_ref).address(),
            BindingRealizationFacet::FilesystemPresentation,
            PrivatePath::parse(DESTINATION_PATH).expect("an absolute path"),
            true,
        );
        let executable = PlannedExecutable::new(
            OperationImplementation::trusted_executable_template(
                reference(PROVIDER),
                token("worker"),
            )
            .expect("a Provider reference is a declared implementation"),
            token("worker"),
            PrivatePath::parse(PROGRAM).expect("an absolute program path"),
            vec![PROGRAM.to_owned()],
            vec!["PATH=/usr/bin".to_owned()],
        )
        .expect("the trusted executable is well formed");
        PrivateExecutionTable::empty()
            .with_source(source)
            .with_observed(consumer_freshness())
            .with_identity(identity)
            .with_destination(destination)
            .with_executable(reference(OPERATION), executable)
    }

    /// The typed invocation the broker resolves one Process launch from.
    pub fn plan_request(consumer_ref: &ResourceRef) -> EffectPlanRequest {
        let callable = operation();
        let parameters = admit_parameters(
            &callable,
            &d2b_contracts_resource::v3::CanonicalJsonObject::parse(
                &serde_json::to_vec(&serde_json::json!({ "servingWorker": false }))
                    .expect("the fixture parameters serialize"),
            )
            .expect("the fixture is a canonical JSON object"),
        )
        .expect("the fixture parameters are admitted");
        EffectPlanRequest::new(
            reference(OPERATION),
            callable,
            subject_of(consumer_ref),
            vec![BindingPlanRequest::new(
                volume_key(consumer_ref),
                RequestedRights::Observe,
                vec![BindingRealizationFacet::FilesystemPresentation],
                None,
            )],
            parameters,
            vec![source_freshness(), consumer_freshness()],
            TransportIdentity::Broker,
            None,
        )
        .expect("a well-formed plan request")
    }

    /// Resolve the broker's own execution plan for one consumer.
    pub fn resolved_execution(consumer_ref: &ResourceRef) -> ExecutionPlan {
        resolve_execution_plan(
            &plan_request(consumer_ref),
            &accepted_graph(consumer_ref),
            &private_table(consumer_ref),
        )
        .expect("the broker resolves the fixture relationship")
    }

    /// The exact identity the fixture policy authorizes.
    pub const USER: &str = "User/worker";

    /// Whether the accepted graph's `RoleBinding` subject vocabulary can name
    /// a run-to-completion consumer.
    ///
    /// Both Process lifetimes are the same converted resource type and already
    /// share one preparation path and one policy path, so the closed subject
    /// list names both and a one-shot consumer's leg is authorized by the same
    /// contract that authorizes a long-running one. The probe reads that
    /// closed list rather than restating it.
    pub fn one_shot_leg_is_bindable() -> bool {
        d2b_contracts_zone_session::v3::role_binding::BINDABLE_SUBJECT_TYPES
            .contains(&"EphemeralProcess")
    }

    /// A confinement policy that admits a namespace-isolated, read-only-root
    /// worker under exactly one admitted identity.
    pub fn policy() -> ExecutionPolicySpec {
        ExecutionPolicySpec::new(
            PolicyNamespaces::new(vec![NamespaceClass::User, NamespaceClass::Mount])
                .expect("namespaces are bounded"),
            PolicyCapabilities::new(Vec::new()).expect("capabilities are bounded"),
            true,
            PolicyIdentity::new(Some(reference(USER)), true)
                .expect("the identity is well formed"),
            PolicyRoot::new(true, true),
            PolicySeccomp::new(None).expect("the seccomp facet is well formed"),
            Some(0o022),
        )
        .expect("a well-formed policy")
    }

    /// A backend that enforces every facet the fixture policy requires.
    pub fn backend_support() -> BackendSupport {
        BackendSupport::new(ALL_CONFINEMENT_FACETS.to_vec())
            .expect("a well-formed backend support set")
    }

    /// The instance request for a row of either lifetime.
    pub fn instance(kind: ExecutionInstanceKind) -> ExecutionInstance {
        ExecutionInstance::new(
            kind,
            Some(reference(USER)),
            BudgetRequest::new(500, 1 << 30, 256, 1024).expect("a well-formed budget"),
        )
        .expect("a well-formed instance")
    }

    /// The provider's declared requirements.
    pub fn requirements() -> ExecutionRequirements {
        ExecutionRequirements::new(
            vec![NamespaceClass::User],
            Vec::new(),
            true,
            None,
            vec![ConfinementFacet::UserNamespace, ConfinementFacet::MountNamespace],
        )
        .expect("a well-formed requirement set")
    }

    /// The ceiling the fixture budget sits under.
    pub fn ceiling() -> BudgetCeiling {
        BudgetCeiling::new(1_000, 1 << 31, 1_024, 4_096)
    }

    /// The shared identity the Process tickets already use.
    pub fn shared_operation_uid() -> ResourceUid {
        operation_uid()
    }
}