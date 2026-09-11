//! Credential resource driver (U12): the v3 `ResourceDriver` conversion of
//! the daemon-owned Credential controller path for the three Credential
//! Providers (secret-service, entra, managed-identity; R3, R4, R30).
//!
//! The driver keeps the old controller behavior and nothing else: reconcile
//! reports Provider readiness for every Credential and, for the
//! managed-identity Provider, derives the co-located agent Process child
//! exactly as the preserved `managed_identity_agent_payload` did and ensures
//! it through the manager-routed child ensure (the child spec is committed
//! BEFORE the child actor exists, F1). The agent is never spawned here: it
//! is a Process resource whose lifetime belongs to the Process driver
//! (KTD13). Delete preserves the revocation-first ordering - the provider
//! RevokeToken call is confirmed (or already confirmed) before any owned
//! Process child is marked deleting - and fails closed when the session
//! generation is missing or no longer current (R28). The manager holds the
//! Credential row until its owned children retire (F3), so the durable
//! deleting mark stays observable for the whole teardown exactly as the old
//! revoke-finalizer ordering made it.
//!
//! Conversion mapping (spec section 13):
//! - `describe` -> [`CredentialDriverFactory`] registration under `Credential`.
//! - `validate_spec` -> [`ResourceDriver::validate`].
//! - `observe` -> [`ResourceDriver::recover`].
//! - finalizer enrollment + agent child minting + provider readiness ->
//!   [`ResourceDriver::reconcile`].
//! - `prepare_finalize`/`execute_finalize`/`finalize` -> [`ResourceDriver::delete`].
//! - `UpdateStatus` -> `ctx.set_status` (in-memory only, R11).
//!
//! One input has no durable home in the new runtime and is flagged for the
//! merged wiring: the old revocation gate read the lease facts from the
//! Credential's persisted status (`/status/resource/credential/...`), which
//! R11 deletes. The driver reads them from
//! [`CredentialDriverEffects::lease_facts`] instead; `None` reproduces the
//! old "no lease state" case (skip revocation) byte for byte.
#![allow(dead_code)]

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use d2b_contracts_provider::v3::credential::{
    CredentialLeaseState, CredentialSpec, PlacementBinding,
};
use d2b_contracts_provider::v3::credential_controller::CredentialProviderKind;
use d2b_contracts_resource::v3::{
    CanonicalJsonObject, CanonicalJsonValue, ControllerGeneration, DesiredLifecycle, ResourceRef,
    ResourceSpec, ResourceUid,
    execution_policy::{BoundedToken, BudgetSpec, DurationMs, ExecutionDomain},
    identity::ReconnectGeneration,
    process::{
        AdoptionPolicy, EnvironmentClass, ExecutionSpec, HealthCheckSpec, NamespaceClass,
        NetworkUsageSpec, ProcessClass, ProcessSpec, ReadinessClass, ReadinessSpec,
        RestartPolicySpec, SandboxSpec, TelemetrySpec,
    },
};
use d2b_resource_runtime::context::{
    ChildEnsure, ResourceContext, SpecDecoder, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{DriverFailure, DriverOp, FailureClass};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};

use crate::credential_resource_runtime::{
    CredentialResourceRuntimeError, CredentialRevocationEvidence, CredentialRevocationInputs,
    CredentialRevocationOutcome, CredentialRevocationRequest, CredentialSession,
    credential_provider_kind,
};

/// The one resource type this factory serves (KTD4 Phase A).
pub(crate) const CREDENTIAL_TYPE_NAME: &str = "Credential";

/// Deterministic owned-child resource type (the managed-identity agent).
const PROCESS_TYPE_NAME: &str = "Process";

/// The Process Provider the agent child runs under (old agent payload).
const AGENT_PROCESS_PROVIDER_REF: &str = "Provider/system-minijail";

/// Non-secret annotation the old agent payload carried: the Provider that
/// owns the supervised controller route. The Process launch path keys the
/// co-located agent intent on it.
pub(crate) const CONTROLLER_PROVIDER_REF_ANNOTATION: &str = "d2b.d2bus.org/controller-provider-ref";
/// Immutable identity of the Controller Provider annotation.
pub(crate) const CONTROLLER_PROVIDER_UID_ANNOTATION: &str = "d2b.d2bus.org/controller-provider-uid";
/// Controller Provider generation annotation.
pub(crate) const CONTROLLER_PROVIDER_GENERATION_ANNOTATION: &str =
    "d2b.d2bus.org/controller-provider-generation";

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialDriverErrorKind {
    /// The durable spec did not decode as the closed Credential contract, or
    /// its scope is invalid for the selected Provider kind.
    SpecInvalid,
    /// The spec selects a Provider this driver does not own.
    ProviderUnsupported,
    /// The Credential Provider is not ready.
    ProviderUnavailable,
    /// The managed-identity agent Process child is not (yet) serving, or the
    /// execution target it must realize on is not ready.
    AgentUnavailable,
    /// A manager child mutation failed.
    ChildMutation,
    /// The revocation identity was rejected (wrong zone/Provider/generation
    /// or a zero session generation): fail closed, never retry blindly.
    RevocationIdentity,
    /// Revocation did not confirm (no live session, uncertain outcome, or a
    /// provider-side failure). Cleanup must not proceed (R28).
    RevocationUnconfirmed,
}

impl CredentialDriverErrorKind {
    const fn class(self) -> FailureClass {
        match self {
            Self::SpecInvalid | Self::ProviderUnsupported | Self::RevocationIdentity => {
                FailureClass::Terminal
            }
            Self::ProviderUnavailable
            | Self::AgentUnavailable
            | Self::ChildMutation
            | Self::RevocationUnconfirmed => FailureClass::Retryable,
        }
    }
}

/// Typed driver failure; redacted at the erased boundary through
/// [`ResourceDriver::classify_error`] (R13). Classified exactly as the old
/// `CredentialResourceRuntimeError`: `InvalidResource` was terminal,
/// everything else retryable.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CredentialDriverError {
    kind: CredentialDriverErrorKind,
    op: DriverOp,
}

impl CredentialDriverError {
    fn new(kind: CredentialDriverErrorKind, op: DriverOp) -> Self {
        Self { kind, op }
    }
}

impl core::fmt::Display for CredentialDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self.kind {
            CredentialDriverErrorKind::SpecInvalid => "credential-spec-invalid",
            CredentialDriverErrorKind::ProviderUnsupported => "credential-provider-unsupported",
            CredentialDriverErrorKind::ProviderUnavailable => "credential-provider-unavailable",
            CredentialDriverErrorKind::AgentUnavailable => "credential-agent-unavailable",
            CredentialDriverErrorKind::ChildMutation => "credential-child-mutation-failed",
            CredentialDriverErrorKind::RevocationIdentity => {
                "credential-revocation-identity-rejected"
            }
            CredentialDriverErrorKind::RevocationUnconfirmed => "credential-revocation-unconfirmed",
        })
    }
}

impl std::error::Error for CredentialDriverError {}

/// Typed in-memory status projection (R11: never persisted). Carries the
/// exact phase/outcome classification the old durable status published, so
/// the conversion stays observable without a status store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CredentialDriverStatus {
    /// Old `Degraded` / `credential-provider-unavailable`.
    ProviderUnavailable,
    /// Old `Pending` / `credential-agent-pending`.
    AgentPending,
    /// Old `Degraded` / `credential-agent-unavailable`.
    AgentUnavailable,
    /// Old `Degraded` / `credential-agent-draining`.
    AgentDraining,
    /// Old `Ready` / `success`.
    Ready,
    /// Old `Pending` / `credential-lease-revoked`.
    LeaseRevoked {
        evidence: CredentialRevocationEvidence,
    },
    /// Old `Degraded` / `credential-revocation-uncertain`. `evidence` is
    /// absent when no revocation call could be made at all (the old status
    /// candidate's `revocation: null` case).
    RevocationUncertain {
        evidence: Option<CredentialRevocationEvidence>,
    },
}

impl CredentialDriverStatus {
    /// The phase the old status candidate published.
    pub(crate) const fn phase(&self) -> &'static str {
        match self {
            Self::ProviderUnavailable
            | Self::AgentUnavailable
            | Self::AgentDraining
            | Self::RevocationUncertain { .. } => "Degraded",
            Self::AgentPending | Self::LeaseRevoked { .. } => "Pending",
            Self::Ready => "Ready",
        }
    }

    /// The outcome code the old status candidate published.
    pub(crate) const fn outcome_code(&self) -> &'static str {
        match self {
            Self::ProviderUnavailable => "credential-provider-unavailable",
            Self::AgentPending => "credential-agent-pending",
            Self::AgentUnavailable => "credential-agent-unavailable",
            Self::AgentDraining => "credential-agent-draining",
            Self::Ready => "success",
            Self::LeaseRevoked { .. } => "credential-lease-revoked",
            Self::RevocationUncertain { .. } => "credential-revocation-uncertain",
        }
    }
}

// ---------------------------------------------------------------------------
// Decoded spec envelope
// ---------------------------------------------------------------------------

/// The spec-store envelope for one Credential row (KTD2), exactly as
/// persisted: the universal desired-state layer (`providerRef`) plus the
/// typed Credential base fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CredentialSpecEnvelope {
    /// The exact stored spec bytes; never rewritten by this driver.
    pub(crate) raw: Vec<u8>,
    provider_ref: Option<ResourceRef>,
    base: CanonicalJsonObject,
}

/// The manager-wired decode hook for Credential rows.
pub(crate) fn credential_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes).map(|spec| CredentialSpecEnvelope {
            raw: bytes.to_vec(),
            provider_ref: spec.provider_ref().cloned(),
            base: spec.base().clone(),
        })
    })
}

// ---------------------------------------------------------------------------
// Provider effect port
// ---------------------------------------------------------------------------

/// The old runner's dependency snapshot facts for one Credential: the
/// Provider row (readiness is `phase == Ready` at its current generation)
/// and the declared execution target row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CredentialDependencyFacts {
    /// Provider row uid (the old agent annotation value).
    pub(crate) provider_uid: String,
    /// Provider row generation (the revocation request binds it).
    pub(crate) provider_generation: u64,
    /// Provider row is `Ready` at its current generation.
    pub(crate) provider_ready: bool,
    /// Execution target row is `Ready` at its current generation.
    pub(crate) execution_ready: bool,
}

/// The provider-side lease facts the old reconciler read from the durable
/// status (`/status/resource/credential/{leaseState,rotationGeneration}`).
///
/// The new runtime keeps no durable status (R11), so the daemon's live
/// lease source supplies them; `None` is exactly the old "no lease state"
/// case, where the old code skipped revocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CredentialLeaseFacts {
    pub(crate) state: CredentialLeaseState,
    pub(crate) rotation_generation: u64,
}

/// The provider-facing effect surface the Credential driver needs. The
/// production implementation delegates to the preserved Provider reads and
/// the ProviderSupervisor session handoff registry; test doubles implement
/// the same seam (R4).
#[async_trait::async_trait]
pub(crate) trait CredentialDriverEffects: Send + Sync + 'static {
    /// Provider + execution-target facts. `None` when the Provider row is
    /// not observable to this daemon (the old controller was never started
    /// without it; deletion still fails closed rather than guessing).
    async fn dependency_facts(
        &self,
        provider_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> Option<CredentialDependencyFacts>;

    /// Provider-side lease facts for one Credential row.
    async fn lease_facts(&self, credential_ref: &ResourceRef) -> Option<CredentialLeaseFacts>;

    /// Whether the managed-identity agent Process is live (target-local
    /// evidence behind the port, like the binding socket probe).
    async fn agent_ready(&self, agent_ref: &ResourceRef) -> bool;

    /// The authenticated Provider session for one Credential Provider (R28).
    /// The session owns the exact generation binding; `None` means no
    /// session surface exists at all and revocation fails closed.
    fn session(&self, provider_ref: &ResourceRef) -> Option<Arc<dyn CredentialSession>>;
}

/// Boxed future returned by one production dependency probe: resolving the
/// Provider and target rows is store-backed (the old dependency snapshots
/// were assembled from the same reads), so the port cannot be a sync
/// closure.
pub(crate) type DependencyFactsFuture<'a> =
    Pin<Box<dyn Future<Output = Option<CredentialDependencyFacts>> + Send + 'a>>;

/// Boxed future of one production lease-fact read.
pub(crate) type LeaseFactsFuture<'a> =
    Pin<Box<dyn Future<Output = Option<CredentialLeaseFacts>> + Send + 'a>>;

/// Boxed future of one production agent-readiness probe.
pub(crate) type AgentReadyFuture<'a> = Pin<Box<dyn Future<Output = bool> + Send + 'a>>;

/// The production effects over the preserved provider reads and the
/// ProviderSupervisor handoff registry. The merge-owner wiring supplies the
/// closures (the same inputs the old `start_u10_controller_runners`
/// assembled) and the daemon's session registry.
pub(crate) struct ProductionCredentialDriverEffects {
    facts: Arc<
        dyn for<'a> Fn(&'a ResourceRef, &'a ResourceRef) -> DependencyFactsFuture<'a> + Send + Sync,
    >,
    lease: Arc<dyn for<'a> Fn(&'a ResourceRef) -> LeaseFactsFuture<'a> + Send + Sync>,
    agent: Arc<dyn for<'a> Fn(&'a ResourceRef) -> AgentReadyFuture<'a> + Send + Sync>,
    sessions: crate::credential_resource_runtime::CredentialSessionRegistry,
}

impl ProductionCredentialDriverEffects {
    pub(crate) fn new(
        facts: Arc<
            dyn for<'a> Fn(&'a ResourceRef, &'a ResourceRef) -> DependencyFactsFuture<'a>
                + Send
                + Sync,
        >,
        lease: Arc<dyn for<'a> Fn(&'a ResourceRef) -> LeaseFactsFuture<'a> + Send + Sync>,
        agent: Arc<dyn for<'a> Fn(&'a ResourceRef) -> AgentReadyFuture<'a> + Send + Sync>,
        sessions: crate::credential_resource_runtime::CredentialSessionRegistry,
    ) -> Self {
        Self {
            facts,
            lease,
            agent,
            sessions,
        }
    }
}

#[async_trait::async_trait]
impl CredentialDriverEffects for ProductionCredentialDriverEffects {
    async fn dependency_facts(
        &self,
        provider_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> Option<CredentialDependencyFacts> {
        (self.facts)(provider_ref, execution_ref).await
    }

    async fn lease_facts(&self, credential_ref: &ResourceRef) -> Option<CredentialLeaseFacts> {
        (self.lease)(credential_ref).await
    }

    async fn agent_ready(&self, agent_ref: &ResourceRef) -> bool {
        (self.agent)(agent_ref).await
    }

    fn session(&self, provider_ref: &ResourceRef) -> Option<Arc<dyn CredentialSession>> {
        Some(self.sessions.for_provider(provider_ref.clone()))
    }
}

// ---------------------------------------------------------------------------
// Factory (merge-owner wiring shape)
// ---------------------------------------------------------------------------

/// Everything the composition unit must construct to instantiate the
/// Credential driver factory for one zone: the preserved provider effects
/// and the zone-authority controller generation (KTD7).
pub(crate) struct CredentialDriverArgs {
    pub(crate) zone: String,
    /// Zone controller generation folded into every revocation request
    /// (old `policy_snapshot.controller_generation`).
    pub(crate) controller_generation: ControllerGeneration,
    pub(crate) effects: Arc<dyn CredentialDriverEffects>,
}

/// [`ResourceDriverFactory`] for the `Credential` resource type.
/// Construction is infallible by contract (R3).
pub(crate) struct CredentialDriverFactory {
    types: [ResourceTypeName; 1],
    args: CredentialDriverArgs,
}

impl CredentialDriverFactory {
    pub(crate) fn new(args: CredentialDriverArgs) -> Self {
        Self {
            types: [ResourceTypeName::new(CREDENTIAL_TYPE_NAME)],
            args,
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for CredentialDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(CredentialDriver::new(CredentialDriverArgs {
            zone: self.args.zone.clone(),
            controller_generation: self.args.controller_generation,
            effects: Arc::clone(&self.args.effects),
        }))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One Credential resource's driver.
#[derive(Clone)]
pub(crate) struct CredentialDriver {
    zone: String,
    controller_generation: ControllerGeneration,
    effects: Arc<dyn CredentialDriverEffects>,
}

impl CredentialDriver {
    pub(crate) fn new(args: CredentialDriverArgs) -> Self {
        Self {
            zone: args.zone,
            controller_generation: args.controller_generation,
            effects: args.effects,
        }
    }

    fn error(&self, kind: CredentialDriverErrorKind, op: DriverOp) -> CredentialDriverError {
        CredentialDriverError::new(kind, op)
    }

    /// Decode the stored envelope and the strict typed Credential spec.
    fn decoded_spec(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<(CredentialSpecEnvelope, CredentialSpec), CredentialDriverError> {
        let envelope = ctx
            .spec::<CredentialSpecEnvelope>()
            .map_err(|_| self.error(CredentialDriverErrorKind::SpecInvalid, op))?;
        let spec = serde_json::from_slice::<CredentialSpec>(&envelope.base.to_canonical_bytes())
            .map_err(|_| self.error(CredentialDriverErrorKind::SpecInvalid, op))?;
        Ok((envelope.clone(), spec))
    }

    /// The Credential Provider this row selects, restricted to the three
    /// Providers this driver owns (the old per-provider controller filter
    /// folded into one type-level driver).
    fn provider_of(
        &self,
        envelope: &CredentialSpecEnvelope,
        op: DriverOp,
    ) -> Result<(ResourceRef, CredentialProviderKind), CredentialDriverError> {
        let provider_ref = envelope
            .provider_ref
            .clone()
            .ok_or_else(|| self.error(CredentialDriverErrorKind::SpecInvalid, op))?;
        let kind = credential_provider_kind(&provider_ref)
            .ok_or_else(|| self.error(CredentialDriverErrorKind::ProviderUnsupported, op))?;
        Ok((provider_ref, kind))
    }

    /// The declared execution target (old `credential_execution_ref`).
    fn execution_ref<'a>(
        &self,
        spec: &'a CredentialSpec,
        op: DriverOp,
    ) -> Result<&'a ResourceRef, CredentialDriverError> {
        let execution_ref = spec
            .scope()
            .execution_ref()
            .ok_or_else(|| self.error(CredentialDriverErrorKind::SpecInvalid, op))?;
        if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
            return Err(self.error(CredentialDriverErrorKind::SpecInvalid, op));
        }
        Ok(execution_ref)
    }

    /// The deterministic agent Process child reference
    /// (`Process/mi-agent-<credential>`), exactly as the old payload minted
    /// it (old `managed_identity_agent_ref`).
    fn agent_ref(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<ResourceRef, CredentialDriverError> {
        let value = format!("{PROCESS_TYPE_NAME}/mi-agent-{}", ctx.key().name);
        ResourceRef::parse(&value)
            .map_err(|_| self.error(CredentialDriverErrorKind::SpecInvalid, op))
    }

    /// The credential's own reference.
    fn credential_ref(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<ResourceRef, CredentialDriverError> {
        ResourceRef::parse(&format!("{}/{}", ctx.key().type_name, ctx.key().name))
            .map_err(|_| self.error(CredentialDriverErrorKind::SpecInvalid, op))
    }

    /// Derive the agent Process child spec + metadata (old
    /// `managed_identity_agent_payload`). The Process spec stays argv-free
    /// and the launch parameters travel in the signed template the Process
    /// provider resolves (KTD13): this driver never spawns anything.
    fn agent_child(
        &self,
        ctx: &ResourceContext,
        spec: &CredentialSpec,
        provider_ref: &ResourceRef,
        facts: &CredentialDependencyFacts,
        op: DriverOp,
    ) -> Result<ChildEnsure, CredentialDriverError> {
        let invalid = || self.error(CredentialDriverErrorKind::SpecInvalid, op);
        let execution_ref = self.execution_ref(spec, op)?.clone();
        if spec.scope().domain_filter() == Some(ExecutionDomain::User) {
            return Err(invalid());
        }
        let placement = match execution_ref.resource_type().as_str() {
            "Host" => PlacementBinding::HostSystem,
            "Guest" => PlacementBinding::GuestAgent,
            _ => return Err(invalid()),
        };
        let zone_ref = format!("Zone/{}", self.zone);
        let placement = d2b_provider_credential_managed_identity::ManagedIdentityPlacement::new(
            placement,
            execution_ref.clone(),
            ResourceRef::parse(&zone_ref).map_err(|_| invalid())?,
        )
        .map_err(|_| invalid())?;
        let controller =
            d2b_provider_credential_managed_identity::ManagedIdentityController::new(placement);
        let agent = controller
            .plan_agent(self.credential_ref(ctx, op)?, true, true)
            .map_err(|_| invalid())?
            .ok_or_else(invalid)?;
        let execution = ExecutionSpec::new(
            agent.execution_ref().clone(),
            Some(ExecutionDomain::System),
            None,
            ProcessClass::Service,
            BoundedToken::parse(agent.binary()).map_err(|_| invalid())?,
            None,
            Vec::new(),
            Vec::new(),
            SandboxSpec::new(
                vec![
                    NamespaceClass::Mount,
                    NamespaceClass::Pid,
                    NamespaceClass::Ipc,
                ],
                Vec::new(),
                BoundedToken::parse("strict").map_err(|_| invalid())?,
                true,
                false,
                EnvironmentClass::Minimal,
                true,
                Some("0022".to_owned()),
                0,
                None,
            )
            .map_err(|_| invalid())?,
            BudgetSpec::default(),
            Some(NetworkUsageSpec::new(None, Vec::new(), false).map_err(|_| invalid())?),
            Vec::new(),
            TelemetrySpec::default(),
        )
        .map_err(|_| invalid())?;
        let process = ProcessSpec::new(
            execution,
            DesiredLifecycle::Running,
            RestartPolicySpec::default(),
            ReadinessSpec::new(
                DurationMs::parse("0s", 0, 300_000).map_err(|_| invalid())?,
                DurationMs::parse("30s", 1_000, 300_000).map_err(|_| invalid())?,
                3,
                1,
                ReadinessClass::ProviderDefined,
            )
            .map_err(|_| invalid())?,
            HealthCheckSpec::default(),
            AdoptionPolicy::AdoptOnRestart,
            DurationMs::parse("30s", 0, 3_600_000).map_err(|_| invalid())?,
        )
        .map_err(|_| invalid())?;
        let mut process_spec = serde_json::to_value(process).map_err(|_| invalid())?;
        process_spec.as_object_mut().ok_or_else(invalid)?.insert(
            "providerRef".to_owned(),
            serde_json::Value::String(AGENT_PROCESS_PROVIDER_REF.to_owned()),
        );
        let spec_bytes = canonical_bytes(&process_spec).map_err(|_| invalid())?;
        let metadata = canonical_bytes(&serde_json::json!({
            "ownerRef": self.credential_ref(ctx, op)?.to_canonical_string(),
            "labels": {},
            "annotations": {
                CONTROLLER_PROVIDER_REF_ANNOTATION: provider_ref.to_canonical_string(),
                CONTROLLER_PROVIDER_UID_ANNOTATION: facts.provider_uid,
                CONTROLLER_PROVIDER_GENERATION_ANNOTATION: facts.provider_generation,
            },
        }))
        .map_err(|_| invalid())?;
        Ok(ChildEnsure {
            type_name: ResourceTypeName::new(PROCESS_TYPE_NAME),
            name: self.agent_ref(ctx, op)?.name().as_str().to_owned(),
            spec: spec_bytes,
            metadata,
        })
    }

    /// Whether this actor already confirmed a revocation for its current
    /// incarnation (the old durable `leaseState: Revoked` read back from its
    /// own status; here the in-memory status slot, R11).
    fn lease_already_revoked(ctx: &ResourceContext) -> bool {
        matches!(
            ctx.status::<CredentialDriverStatus>(),
            Some(CredentialDriverStatus::LeaseRevoked { .. })
        )
    }

    /// The old revocation gate: revoke only when the lease is `Active` or
    /// `Unknown`; `Expired`, `Revoked`, and "no lease fact" skip revocation
    /// exactly as the old status read did.
    fn lease_needs_revocation(lease: Option<CredentialLeaseFacts>) -> bool {
        matches!(
            lease.map(|facts| facts.state),
            Some(CredentialLeaseState::Active | CredentialLeaseState::Unknown)
        )
    }

    /// The owned Process children of this Credential, through the manager
    /// (old `owned_processes`: owner-scoped Process list).
    async fn owned_processes(
        &self,
        ctx: &mut ResourceContext,
        op: DriverOp,
    ) -> Result<Vec<d2b_resource_runtime::identity::StoredDesiredResource>, CredentialDriverError>
    {
        let children = ctx
            .children()
            .await
            .map_err(|_| self.error(CredentialDriverErrorKind::ChildMutation, op))?;
        Ok(children
            .into_iter()
            .filter(|child| child.key.type_name == PROCESS_TYPE_NAME)
            .collect())
    }

    /// Build one revocation request for this Credential (R28): the request
    /// binds the exact session generation, provider generation, and
    /// controller generation; a zero/unknown generation fails closed here.
    fn revocation_request(
        &self,
        ctx: &ResourceContext,
        spec: &CredentialSpec,
        provider_ref: &ResourceRef,
        provider_generation: u64,
        rotation_generation: u64,
        session_generation: ReconnectGeneration,
        op: DriverOp,
    ) -> Result<CredentialRevocationRequest, CredentialDriverError> {
        let invalid = || self.error(CredentialDriverErrorKind::RevocationIdentity, op);
        let zone = d2b_contracts_resource::v3::ZoneId::parse(ctx.key().zone.as_str())
            .map_err(|_| invalid())?;
        let credential_uid = resource_uid(ctx.uid()).map_err(|_| invalid())?;
        let credential_generation =
            d2b_contracts_resource::v3::ResourceGeneration::new(ctx.generation())
                .map_err(|_| invalid())?;
        let provider_generation =
            d2b_contracts_resource::v3::ResourceGeneration::new(provider_generation)
                .map_err(|_| invalid())?;
        CredentialRevocationRequest::new(CredentialRevocationInputs {
            zone,
            credential_ref: self.credential_ref(ctx, op)?,
            credential_uid,
            credential_generation,
            user_ref: spec.scope().user_ref().cloned(),
            provider_ref: provider_ref.clone(),
            provider_generation,
            controller_generation: self.controller_generation,
            session_generation,
            rotation_generation,
        })
        .map_err(|error| match error {
            CredentialResourceRuntimeError::InvalidResource => invalid(),
            _ => self.error(CredentialDriverErrorKind::RevocationUnconfirmed, op),
        })
    }

    /// One confirmed-or-skipped revocation pass (old `execute_finalize`'s
    /// lease branch). Returns `Ok(())` when cleanup may proceed.
    async fn revoke_lease(
        &self,
        ctx: &mut ResourceContext,
        spec: &CredentialSpec,
        provider_ref: &ResourceRef,
        lease: Option<CredentialLeaseFacts>,
        op: DriverOp,
    ) -> Result<(), CredentialDriverError> {
        if Self::lease_already_revoked(ctx) || !Self::lease_needs_revocation(lease) {
            return Ok(());
        }
        let unconfirmed = || self.error(CredentialDriverErrorKind::RevocationUnconfirmed, op);
        let Some(session) = self.effects.session(provider_ref) else {
            ctx.set_status(CredentialDriverStatus::RevocationUncertain { evidence: None });
            return Err(unconfirmed());
        };
        // Fail closed (R28): a missing live session generation means no
        // authenticated revoker exists, so no cleanup may run.
        let Some(session_generation) = session.session_generation() else {
            ctx.set_status(CredentialDriverStatus::RevocationUncertain { evidence: None });
            return Err(unconfirmed());
        };
        let execution_ref = self.execution_ref(spec, op)?.clone();
        let Some(facts) = self
            .effects
            .dependency_facts(provider_ref, &execution_ref)
            .await
        else {
            ctx.set_status(CredentialDriverStatus::RevocationUncertain { evidence: None });
            return Err(unconfirmed());
        };
        let rotation_generation = lease
            .map(|facts| facts.rotation_generation)
            .filter(|generation| *generation != 0)
            .unwrap_or(1);
        let request = self.revocation_request(
            ctx,
            spec,
            provider_ref,
            facts.provider_generation,
            rotation_generation,
            session_generation,
            op,
        )?;
        let outcome = session
            .revoke_credential(&request)
            .await
            .map_err(|error| match error {
                CredentialResourceRuntimeError::InvalidResource => {
                    self.error(CredentialDriverErrorKind::RevocationIdentity, op)
                }
                _ => unconfirmed(),
            })?;
        let evidence = CredentialRevocationEvidence::confirmed(&request, outcome);
        match outcome {
            CredentialRevocationOutcome::Revoked | CredentialRevocationOutcome::AlreadyRevoked => {
                // The old reconciler persisted this evidence in the durable
                // status; the new runtime keeps no status store (R11), so the
                // journal is where the confirmed revocation stays observable.
                tracing::info!(
                    credential = %self.credential_ref(ctx, op)?.to_canonical_string(),
                    operation_id = %evidence.operation_id(),
                    outcome = evidence.outcome_code(),
                    session_generation = evidence.session_generation().get(),
                    "credential lease revocation confirmed",
                );
                ctx.set_status(CredentialDriverStatus::LeaseRevoked { evidence });
                Ok(())
            }
            CredentialRevocationOutcome::Uncertain => {
                tracing::warn!(
                    credential = %self.credential_ref(ctx, op)?.to_canonical_string(),
                    operation_id = %request.operation_id(),
                    session_generation = request.session_generation().get(),
                    "credential lease revocation unconfirmed; cleanup withheld",
                );
                ctx.set_status(CredentialDriverStatus::RevocationUncertain {
                    evidence: Some(evidence),
                });
                Err(unconfirmed())
            }
        }
    }
}

/// Canonical JSON bytes (the store contract's deterministic encoding: the
/// manager compares child specs byte-wise for `Unchanged`).
fn canonical_bytes(value: &serde_json::Value) -> Result<Vec<u8>, ()> {
    let bytes = serde_json::to_vec(value).map_err(|_| ())?;
    CanonicalJsonValue::parse(&bytes)
        .map(|value| value.to_canonical_bytes())
        .map_err(|_| ())
}

/// Map the new store's 16-byte deterministic uid onto the contracts crate's
/// UUIDv4-shaped `ResourceUid` (version nibble 4, RFC 9562 variant).
fn resource_uid(bytes: &[u8; 16]) -> Result<ResourceUid, ()> {
    let mut bytes = *bytes;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    );
    ResourceUid::parse(text).map_err(|_| ())
}

/// The per-kind scope checks (old `credential_scope_valid`).
fn credential_scope_valid(kind: CredentialProviderKind, spec: &CredentialSpec) -> bool {
    let Some(execution_ref) = spec.scope().execution_ref() else {
        return false;
    };
    match kind {
        CredentialProviderKind::SecretService => {
            spec.scope().domain_filter() == Some(ExecutionDomain::User)
                && spec.scope().user_ref().is_some()
                && matches!(execution_ref.resource_type().as_str(), "Host" | "Guest")
        }
        CredentialProviderKind::Entra => {
            execution_ref.resource_type().as_str() == "Guest"
                && spec.scope().domain_filter() != Some(ExecutionDomain::User)
        }
        CredentialProviderKind::ManagedIdentity => {
            matches!(execution_ref.resource_type().as_str(), "Host" | "Guest")
                && spec.scope().domain_filter() != Some(ExecutionDomain::User)
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriver for CredentialDriver {
    type Error = CredentialDriverError;

    fn classify_error(&self, error: &CredentialDriverError) -> DriverFailure {
        match error.kind.class() {
            FailureClass::Retryable => DriverFailure::retryable(error.op),
            FailureClass::Terminal => DriverFailure::terminal(error.op),
        }
    }

    /// Spec decode, Provider selection, and the per-kind scope checks (old
    /// `validate_spec` + `credential_scope_valid`).
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let (envelope, spec) = self.decoded_spec(ctx, DriverOp::Validate)?;
        let (_, kind) = self.provider_of(&envelope, DriverOp::Validate)?;
        if !credential_scope_valid(kind, &spec) {
            return Err(self.error(CredentialDriverErrorKind::SpecInvalid, DriverOp::Validate));
        }
        Ok(())
    }

    /// Discovery/adoption (F2). A Credential has no host-side realization to
    /// adopt; the managed-identity agent Process is the only adopted
    /// resource, and only while it is already serving. Everything else
    /// waits for reconcile, exactly as the old no-op `observe` did.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let (envelope, _) = self.decoded_spec(ctx, DriverOp::Recover)?;
        let (_, kind) = self.provider_of(&envelope, DriverOp::Recover)?;
        if kind != CredentialProviderKind::ManagedIdentity {
            return Ok(RecoveryOutcome::Missing);
        }
        let agent_ref = self.agent_ref(ctx, DriverOp::Recover)?;
        let agent_key = ResourceKey::new(&self.zone, PROCESS_TYPE_NAME, agent_ref.name().as_str());
        let present = self
            .owned_processes(ctx, DriverOp::Recover)
            .await?
            .iter()
            .any(|child| child.key == agent_key);
        if present && self.effects.agent_ready(&agent_ref).await {
            ctx.set_status(CredentialDriverStatus::Ready);
            Ok(RecoveryOutcome::Adopted)
        } else {
            Ok(RecoveryOutcome::Missing)
        }
    }

    /// One reconcile pass (old `plan` + `reconcile` + `execute_effect`): the
    /// Provider must be ready, and the managed-identity Provider additionally
    /// derives + ensures its agent Process child and reports the child-phase
    /// classification. The resource never goes ready before its effects are.
    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let (envelope, spec) = self.decoded_spec(ctx, DriverOp::Reconcile)?;
        let (provider_ref, kind) = self.provider_of(&envelope, DriverOp::Reconcile)?;
        let execution_ref = self.execution_ref(&spec, DriverOp::Reconcile)?.clone();
        let Some(facts) = self
            .effects
            .dependency_facts(&provider_ref, &execution_ref)
            .await
        else {
            ctx.set_status(CredentialDriverStatus::ProviderUnavailable);
            return Err(self.error(
                CredentialDriverErrorKind::ProviderUnavailable,
                DriverOp::Reconcile,
            ));
        };
        if !facts.provider_ready {
            ctx.set_status(CredentialDriverStatus::ProviderUnavailable);
            return Err(self.error(
                CredentialDriverErrorKind::ProviderUnavailable,
                DriverOp::Reconcile,
            ));
        }
        if kind != CredentialProviderKind::ManagedIdentity {
            ctx.set_status(CredentialDriverStatus::Ready);
            return Ok(ReconcileOutcome::Satisfied);
        }
        // The agent child is minted only after the Provider and the declared
        // execution target are both ready (old `managed_identity_child_batch`
        // dependency gate).
        if !facts.execution_ready {
            ctx.set_status(CredentialDriverStatus::AgentPending);
            return Err(self.error(
                CredentialDriverErrorKind::AgentUnavailable,
                DriverOp::Reconcile,
            ));
        }
        let child = self.agent_child(ctx, &spec, &provider_ref, &facts, DriverOp::Reconcile)?;
        let agent_ref = self.agent_ref(ctx, DriverOp::Reconcile)?;
        let agent_key = ResourceKey::new(&self.zone, PROCESS_TYPE_NAME, agent_ref.name().as_str());
        let owned = self.owned_processes(ctx, DriverOp::Reconcile).await?;
        match owned.iter().find(|row| row.key == agent_key) {
            Some(row) if row.deleting => {
                ctx.set_status(CredentialDriverStatus::AgentDraining);
                Err(self.error(
                    CredentialDriverErrorKind::AgentUnavailable,
                    DriverOp::Reconcile,
                ))
            }
            Some(row) if row.spec != child.spec || row.metadata != child.metadata => {
                // The persisted child drifted from the derived agent shape
                // (old `managed_identity_agent_matches`): delete it; the next
                // pass recreates the desired child. A deleting row rejects a
                // late ensure, so this must happen before any ensure.
                ctx.delete(&row.key).await.map_err(|_| {
                    self.error(
                        CredentialDriverErrorKind::ChildMutation,
                        DriverOp::Reconcile,
                    )
                })?;
                ctx.set_status(CredentialDriverStatus::AgentPending);
                Err(self.error(
                    CredentialDriverErrorKind::AgentUnavailable,
                    DriverOp::Reconcile,
                ))
            }
            Some(_) if !self.effects.agent_ready(&agent_ref).await => {
                ctx.set_status(CredentialDriverStatus::AgentUnavailable);
                Err(self.error(
                    CredentialDriverErrorKind::AgentUnavailable,
                    DriverOp::Reconcile,
                ))
            }
            Some(_) => {
                ctx.set_status(CredentialDriverStatus::Ready);
                Ok(ReconcileOutcome::Satisfied)
            }
            None => {
                // The agent child is the Credential's realization: it is
                // minted through the manager (commit-before-spawn, F1) and
                // never spawned here (KTD13).
                ctx.ensure_child(child).await.map_err(|_| {
                    self.error(
                        CredentialDriverErrorKind::ChildMutation,
                        DriverOp::Reconcile,
                    )
                })?;
                ctx.set_status(CredentialDriverStatus::AgentPending);
                Err(self.error(
                    CredentialDriverErrorKind::AgentUnavailable,
                    DriverOp::Reconcile,
                ))
            }
        }
    }

    /// Teardown (old `prepare_finalize`/`execute_finalize`/`finalize` fold).
    /// Revocation-first ordering is preserved: the provider RevokeToken call
    /// must confirm (or have confirmed) before any owned Process child is
    /// marked deleting, and any missing/mismatched session generation fails
    /// the pass closed with no child deletion (R28). Idempotent under retry
    /// (R10): a child already marked deleting is skipped and the manager
    /// holds this row until every owned child retires (F3).
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let (envelope, spec) = self.decoded_spec(ctx, DriverOp::Delete)?;
        let (provider_ref, _kind) = self.provider_of(&envelope, DriverOp::Delete)?;
        let credential_ref = self.credential_ref(ctx, DriverOp::Delete)?;
        let lease = self.effects.lease_facts(&credential_ref).await;
        self.revoke_lease(ctx, &spec, &provider_ref, lease, DriverOp::Delete)
            .await?;
        for child in self.owned_processes(ctx, DriverOp::Delete).await? {
            if child.deleting {
                continue;
            }
            ctx.delete(&child.key).await.map_err(|_| {
                self.error(CredentialDriverErrorKind::ChildMutation, DriverOp::Delete)
            })?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over a scripted effect port and a recording
// manager endpoint with one shared ordered log (R4; revocation ordering,
// child minting, and fail-closed session binding observed as they happen).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use d2b_contracts_provider::v3::credential::CredentialLeaseState;
    use d2b_contracts_resource::v3::identity::ReconnectGeneration;
    use d2b_contracts_resource::v3::process::ProcessSpec;
    use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ResourceSpec};
    use d2b_resource_runtime::context::{
        ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, WatchId,
        WatchRegistration,
    };
    use d2b_resource_runtime::driver::{
        ReconcileOutcome, RecoveryOutcome, ResourceDriver, ResourceDriverFactory,
    };
    use d2b_resource_runtime::error::{FailureClass, ResourceError};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};
    use d2b_resource_runtime::spec_store::EnsureOutcome;
    use d2b_resource_runtime::target::TargetHandle;
    use parking_lot::Mutex;

    use crate::credential_resource_runtime::{
        CredentialResourceRuntimeError, CredentialRevocationOutcome, CredentialRevocationRequest,
        CredentialSession,
    };

    use super::{
        CONTROLLER_PROVIDER_GENERATION_ANNOTATION, CONTROLLER_PROVIDER_REF_ANNOTATION,
        CONTROLLER_PROVIDER_UID_ANNOTATION, CREDENTIAL_TYPE_NAME, CredentialDependencyFacts,
        CredentialDriver, CredentialDriverArgs, CredentialDriverFactory, CredentialDriverStatus,
        CredentialLeaseFacts, credential_spec_decoder,
    };

    const MI_PROVIDER: &str = "Provider/credential-managed-identity";
    const SECRET_SERVICE_PROVIDER: &str = "Provider/credential-secret-service";
    const AGENT_KEY: &str = "Process/mi-agent-relay";

    type Log = Arc<Mutex<Vec<String>>>;

    fn log() -> Log {
        Arc::new(Mutex::new(Vec::new()))
    }

    // -- fakes ---------------------------------------------------------------

    /// Scripted effect port. Every call lands in the shared ordered log so
    /// revocation-before-child-deletion is observable.
    struct FakeEffects {
        log: Log,
        facts: Mutex<Option<CredentialDependencyFacts>>,
        lease: Mutex<Option<CredentialLeaseFacts>>,
        agent_ready: Mutex<bool>,
        session: Mutex<Option<Arc<dyn CredentialSession>>>,
    }

    impl FakeEffects {
        fn new(log: Log) -> Arc<Self> {
            Arc::new(Self {
                log,
                facts: Mutex::new(Some(facts(true, true))),
                lease: Mutex::new(None),
                agent_ready: Mutex::new(true),
                session: Mutex::new(Some(Arc::new(RecordingSession::new(Some(7))))),
            })
        }

        fn set_facts(&self, value: Option<CredentialDependencyFacts>) {
            *self.facts.lock() = value;
        }

        fn set_lease(&self, value: Option<CredentialLeaseFacts>) {
            *self.lease.lock() = value;
        }

        fn set_agent_ready(&self, value: bool) {
            *self.agent_ready.lock() = value;
        }

        fn set_session(&self, value: Option<Arc<dyn CredentialSession>>) {
            *self.session.lock() = value;
        }
    }

    #[async_trait::async_trait]
    impl super::CredentialDriverEffects for FakeEffects {
        async fn dependency_facts(
            &self,
            _provider_ref: &ResourceRef,
            _execution_ref: &ResourceRef,
        ) -> Option<CredentialDependencyFacts> {
            self.log.lock().push("dependency-facts".to_owned());
            self.facts.lock().clone()
        }

        async fn lease_facts(&self, _credential_ref: &ResourceRef) -> Option<CredentialLeaseFacts> {
            self.log.lock().push("lease-facts".to_owned());
            *self.lease.lock()
        }

        async fn agent_ready(&self, _agent_ref: &ResourceRef) -> bool {
            self.log.lock().push("agent-ready".to_owned());
            *self.agent_ready.lock()
        }

        fn session(&self, _provider_ref: &ResourceRef) -> Option<Arc<dyn CredentialSession>> {
            self.log.lock().push("session".to_owned());
            self.session.lock().clone()
        }
    }

    fn facts(provider_ready: bool, execution_ready: bool) -> CredentialDependencyFacts {
        CredentialDependencyFacts {
            provider_uid: "223e4567-e89b-42d3-a456-426614174000".to_owned(),
            provider_generation: 1,
            provider_ready,
            execution_ready,
        }
    }

    /// Session double that binds the generation exactly like the real
    /// `ComponentCredentialSession`: a request carrying a different
    /// generation is `Uncertain`, never `Revoked`.
    struct RecordingSession {
        generation: Option<ReconnectGeneration>,
        operations: Mutex<Vec<String>>,
    }

    impl RecordingSession {
        fn new(generation: Option<u64>) -> Self {
            Self {
                generation: generation.map(|value| ReconnectGeneration::new(value).unwrap()),
                operations: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl CredentialSession for RecordingSession {
        fn session_generation(&self) -> Option<ReconnectGeneration> {
            self.generation
        }

        async fn revoke_credential(
            &self,
            request: &CredentialRevocationRequest,
        ) -> Result<CredentialRevocationOutcome, CredentialResourceRuntimeError> {
            if Some(request.session_generation()) != self.generation {
                return Ok(CredentialRevocationOutcome::Uncertain);
            }
            let mut operations = self.operations.lock();
            let operation_id = request.operation_id().to_owned();
            if operations.contains(&operation_id) {
                return Ok(CredentialRevocationOutcome::AlreadyRevoked);
            }
            operations.push(operation_id);
            Ok(CredentialRevocationOutcome::Revoked)
        }
    }

    /// Recording manager: child mutations, deletes, and reads share the
    /// ordered log with the effect port.
    struct RecordingManager {
        log: Log,
        children: Mutex<Vec<StoredDesiredResource>>,
        ensured: Mutex<Vec<ChildEnsure>>,
    }

    impl RecordingManager {
        fn new(log: Log) -> Arc<Self> {
            Arc::new(Self {
                log,
                children: Mutex::new(Vec::new()),
                ensured: Mutex::new(Vec::new()),
            })
        }

        fn with_child(log: Log, child: StoredDesiredResource) -> Arc<Self> {
            let manager = Self::new(log);
            manager.children.lock().push(child);
            manager
        }

        fn ensured(&self) -> Vec<ChildEnsure> {
            self.ensured.lock().clone()
        }

        fn children(&self) -> Vec<StoredDesiredResource> {
            self.children.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl ManagerEndpoint for RecordingManager {
        async fn ensure_child(
            &self,
            parent: &ResourceKey,
            child: ChildEnsure,
        ) -> Result<EnsureOutcome, ResourceError> {
            self.log.lock().push(format!("ensure:{}", child.name));
            let key = ResourceKey::new(&parent.zone, child.type_name.as_str(), &child.name);
            let row = StoredDesiredResource {
                key: key.clone(),
                uid: [0x51; 16],
                generation: 1,
                owner_uid: Some([0x42; 16]),
                provenance: ResourceProvenance::Resource,
                deleting: false,
                spec: child.spec.clone(),
                metadata: child.metadata.clone(),
                created_at: 0,
            };
            self.ensured.lock().push(child);
            let mut children = self.children.lock();
            match children.iter().position(|existing| existing.key == key) {
                Some(index) if children[index].deleting => Err(ResourceError::DeletingConflict {
                    zone: key.zone,
                    type_name: key.type_name,
                    name: key.name,
                }),
                Some(index) => {
                    children[index] = row.clone();
                    Ok(EnsureOutcome::Unchanged(row))
                }
                None => {
                    children.push(row.clone());
                    Ok(EnsureOutcome::Created(row))
                }
            }
        }

        async fn get(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<StoredDesiredResource>, ResourceError> {
            self.log
                .lock()
                .push(format!("get:{}/{}", key.type_name, key.name));
            Ok(self
                .children
                .lock()
                .iter()
                .find(|child| child.key == *key)
                .cloned())
        }

        async fn view(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, ResourceError> {
            // Desired rows only: this fixture publishes no runtime status, so
            // it serves no observed state.
            Ok(None)
        }

        async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
            self.log
                .lock()
                .push(format!("delete:{}/{}", key.type_name, key.name));
            for child in self.children.lock().iter_mut() {
                if child.key == *key {
                    child.deleting = true;
                }
            }
            Ok(())
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
            self.log.lock().push("list-owned".to_owned());
            Ok(self.children.lock().clone())
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: WatchRegistration,
        ) -> Result<WatchId, ResourceError> {
            Ok(WatchId(1))
        }

        async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
            Ok(())
        }
    }

    struct NullRequeue;

    impl RequeueScheduler for NullRequeue {
        fn schedule(&self, _key: ResourceKey, _after: std::time::Duration) -> RequeueId {
            RequeueId(0)
        }

        fn cancel(&self, _id: RequeueId) {}
    }

    // -- fixtures ------------------------------------------------------------

    fn credential_spec_json(provider_ref: &str, execution_ref: &str) -> serde_json::Value {
        serde_json::json!({
            "providerRef": provider_ref,
            "scope": {"executionRef": execution_ref, "domainFilter": "system"},
            "allowedOperations": ["acquire-token"],
            "audience": "relay"
        })
    }

    fn row(provider_ref: &str) -> StoredDesiredResource {
        row_with(provider_ref, "Guest/gateway", false)
    }

    fn row_with(provider_ref: &str, execution_ref: &str, deleting: bool) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("dev", CREDENTIAL_TYPE_NAME, "relay"),
            uid: [0x42; 16],
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Nix,
            deleting,
            spec: serde_json::to_vec(&credential_spec_json(provider_ref, execution_ref))
                .expect("spec bytes"),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    fn agent_child(deleting: bool) -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new("dev", "Process", "mi-agent-relay"),
            uid: [0x51; 16],
            generation: 1,
            owner_uid: Some([0x42; 16]),
            provenance: ResourceProvenance::Resource,
            deleting,
            spec: Vec::new(),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    fn context(row: StoredDesiredResource, manager: Arc<RecordingManager>) -> ResourceContext {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        ResourceContext::new(
            row,
            TargetHandle::Host,
            credential_spec_decoder(),
            manager,
            Arc::new(NullRequeue),
            effects_tx,
            notify_tx,
        )
    }

    fn driver(effects: Arc<FakeEffects>) -> CredentialDriver {
        CredentialDriver::new(CredentialDriverArgs {
            zone: "dev".to_owned(),
            controller_generation: ControllerGeneration::new(1).unwrap(),
            effects,
        })
    }

    // -- tests ---------------------------------------------------------------

    #[test]
    fn factory_registers_only_the_credential_resource_type() {
        let factory = CredentialDriverFactory::new(CredentialDriverArgs {
            zone: "dev".to_owned(),
            controller_generation: ControllerGeneration::new(1).unwrap(),
            effects: FakeEffects::new(log()),
        });
        assert_eq!(factory.resource_types().len(), 1);
        assert_eq!(factory.resource_types()[0].as_str(), CREDENTIAL_TYPE_NAME);
    }

    #[tokio::test]
    async fn factory_created_driver_validates_through_the_erased_boundary() {
        let factory = CredentialDriverFactory::new(CredentialDriverArgs {
            zone: "dev".to_owned(),
            controller_generation: ControllerGeneration::new(1).unwrap(),
            effects: FakeEffects::new(log()),
        });
        let mut ctx = context(row(MI_PROVIDER), RecordingManager::new(log()));
        let mut driver = factory.create(ctx.key()).await;
        d2b_resource_runtime::driver::DynResourceDriver::validate(&mut *driver, &mut ctx)
            .await
            .expect("valid credential");
    }

    #[tokio::test]
    async fn validate_rejects_a_provider_outside_the_credential_family() {
        let mut ctx = context(
            row_with("Provider/volume-virtiofs", "Guest/gateway", false),
            RecordingManager::new(log()),
        );
        let error = driver(FakeEffects::new(log()))
            .validate(&mut ctx)
            .await
            .expect_err("unsupported provider");
        assert_eq!(error.to_string(), "credential-provider-unsupported");
        assert!(matches!(
            driver(FakeEffects::new(log()))
                .classify_error(&error)
                .class(),
            FailureClass::Terminal
        ));
    }

    #[tokio::test]
    async fn validate_rejects_scope_mismatches_per_provider_kind() {
        // Entra credentials execute on a Guest and never carry a user domain.
        let mut ctx = context(
            row_with("Provider/credential-entra", "Host/host-system", false),
            RecordingManager::new(log()),
        );
        driver(FakeEffects::new(log()))
            .validate(&mut ctx)
            .await
            .expect_err("entra host scope");

        // Secret-service credentials require the user domain and a user ref.
        let secret_service = serde_json::json!({
            "providerRef": SECRET_SERVICE_PROVIDER,
            "scope": {"executionRef": "Host/host-system", "domainFilter": "system"},
            "allowedOperations": ["acquire-token"],
            "audience": "relay"
        });
        let mut ctx = context(
            StoredDesiredResource {
                spec: serde_json::to_vec(&secret_service).expect("spec bytes"),
                ..row(SECRET_SERVICE_PROVIDER)
            },
            RecordingManager::new(log()),
        );
        driver(FakeEffects::new(log()))
            .validate(&mut ctx)
            .await
            .expect_err("secret-service system scope");

        let user_domain = serde_json::json!({
            "providerRef": SECRET_SERVICE_PROVIDER,
            "scope": {
                "executionRef": "Host/host-system",
                "domainFilter": "user",
                "userRef": "User/alice"
            },
            "allowedOperations": ["acquire-token"],
            "audience": "relay"
        });
        let mut ctx = context(
            StoredDesiredResource {
                spec: serde_json::to_vec(&user_domain).expect("spec bytes"),
                ..row(SECRET_SERVICE_PROVIDER)
            },
            RecordingManager::new(log()),
        );
        driver(FakeEffects::new(log()))
            .validate(&mut ctx)
            .await
            .expect("secret-service user scope");
    }

    #[tokio::test]
    async fn reconcile_reports_provider_unavailable_and_never_goes_ready() {
        let effects = FakeEffects::new(log());
        effects.set_facts(None);
        let mut ctx = context(row(MI_PROVIDER), RecordingManager::new(log()));
        let error = driver(Arc::clone(&effects))
            .reconcile(&mut ctx)
            .await
            .expect_err("no provider facts");
        assert!(matches!(
            driver(Arc::clone(&effects)).classify_error(&error).class(),
            FailureClass::Retryable
        ));
        assert_eq!(
            ctx.status::<CredentialDriverStatus>(),
            Some(&CredentialDriverStatus::ProviderUnavailable)
        );

        let effects = FakeEffects::new(log());
        effects.set_facts(Some(facts(false, true)));
        let mut ctx = context(row(MI_PROVIDER), RecordingManager::new(log()));
        driver(Arc::clone(&effects))
            .reconcile(&mut ctx)
            .await
            .expect_err("provider not ready");
        assert_eq!(
            ctx.status::<CredentialDriverStatus>(),
            Some(&CredentialDriverStatus::ProviderUnavailable)
        );
    }

    #[tokio::test]
    async fn managed_identity_reconcile_gates_the_agent_on_the_execution_target() {
        let log = log();
        let effects = FakeEffects::new(Arc::clone(&log));
        effects.set_facts(Some(facts(true, false)));
        let manager = RecordingManager::new(Arc::clone(&log));
        let mut ctx = context(row(MI_PROVIDER), Arc::clone(&manager));
        driver(Arc::clone(&effects))
            .reconcile(&mut ctx)
            .await
            .expect_err("execution target not ready");
        assert!(
            manager.ensured().is_empty(),
            "no child before the gate opens"
        );
        assert_eq!(
            ctx.status::<CredentialDriverStatus>(),
            Some(&CredentialDriverStatus::AgentPending)
        );
    }

    #[tokio::test]
    async fn managed_identity_reconcile_ensures_the_agent_until_it_is_ready() {
        let log = log();
        let effects = FakeEffects::new(Arc::clone(&log));
        effects.set_agent_ready(false);
        let manager = RecordingManager::new(Arc::clone(&log));
        let mut ctx = context(row(MI_PROVIDER), Arc::clone(&manager));
        let mut driver = driver(Arc::clone(&effects));

        driver
            .reconcile(&mut ctx)
            .await
            .expect_err("agent child absent");
        assert_eq!(
            ctx.status::<CredentialDriverStatus>(),
            Some(&CredentialDriverStatus::AgentPending)
        );
        let ensured = manager.ensured();
        assert_eq!(ensured.len(), 1);
        assert_eq!(ensured[0].type_name.as_str(), "Process");
        assert_eq!(ensured[0].name, "mi-agent-relay");
        assert_eq!(
            manager
                .children()
                .first()
                .map(|child| child.key.type_name.as_str()),
            Some("Process")
        );

        // The child exists now and the probe reports it unready.
        driver
            .reconcile(&mut ctx)
            .await
            .expect_err("agent not ready");
        assert_eq!(
            ctx.status::<CredentialDriverStatus>(),
            Some(&CredentialDriverStatus::AgentUnavailable)
        );

        effects.set_agent_ready(true);
        assert_eq!(
            driver.reconcile(&mut ctx).await.expect("agent ready"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            ctx.status::<CredentialDriverStatus>(),
            Some(&CredentialDriverStatus::Ready)
        );
        assert_eq!(
            ctx.status::<CredentialDriverStatus>()
                .map(CredentialDriverStatus::outcome_code),
            Some("success")
        );
    }

    #[tokio::test]
    async fn managed_identity_agent_spec_is_owner_bound_egress_denied_and_annotation_tagged() {
        let log = log();
        let effects = FakeEffects::new(Arc::clone(&log));
        let manager = RecordingManager::new(Arc::clone(&log));
        let mut ctx = context(row(MI_PROVIDER), Arc::clone(&manager));
        driver(Arc::clone(&effects))
            .reconcile(&mut ctx)
            .await
            .expect_err("the agent child is minted on the first pass");
        assert_eq!(
            driver(Arc::clone(&effects))
                .reconcile(&mut ctx)
                .await
                .expect("agent ready"),
            ReconcileOutcome::Satisfied
        );
        let ensured = manager.ensured();
        let child = ensured.first().expect("agent child");

        // The Process spec is the signed-template shape (argv-free, KTD13).
        let spec =
            serde_json::from_slice::<ResourceSpec>(&child.spec).expect("Process ResourceSpec");
        assert_eq!(
            spec.provider_ref().map(ResourceRef::to_canonical_string),
            Some("Provider/system-minijail".to_owned())
        );
        let process =
            serde_json::from_slice::<ProcessSpec>(&spec.base().to_canonical_bytes()).expect("spec");
        assert_eq!(
            process.execution().template().as_str(),
            "d2b-managed-identity-agent"
        );
        assert_eq!(
            process.execution().execution_ref().to_canonical_string(),
            "Guest/gateway"
        );
        let network = process.execution().network_usage().expect("network policy");
        assert!(!network.allow_egress());

        // The metadata envelope binds the owner and the controller Provider
        // annotations the launch path resolves the agent intent through.
        let metadata =
            serde_json::from_slice::<serde_json::Value>(&child.metadata).expect("metadata");
        assert_eq!(
            metadata.get("ownerRef").and_then(serde_json::Value::as_str),
            Some("Credential/relay")
        );
        let annotations = metadata
            .get("annotations")
            .and_then(serde_json::Value::as_object)
            .expect("annotations");
        assert_eq!(
            annotations
                .get(CONTROLLER_PROVIDER_REF_ANNOTATION)
                .and_then(serde_json::Value::as_str),
            Some(MI_PROVIDER)
        );
        assert_eq!(
            annotations
                .get(CONTROLLER_PROVIDER_UID_ANNOTATION)
                .and_then(serde_json::Value::as_str),
            Some("223e4567-e89b-42d3-a456-426614174000")
        );
        assert_eq!(
            annotations
                .get(CONTROLLER_PROVIDER_GENERATION_ANNOTATION)
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
    }

    #[tokio::test]
    async fn non_managed_identity_credentials_reconcile_without_children() {
        let log = log();
        let effects = FakeEffects::new(Arc::clone(&log));
        let manager = RecordingManager::new(Arc::clone(&log));
        let mut ctx = context(row(SECRET_SERVICE_PROVIDER), Arc::clone(&manager));
        assert_eq!(
            driver(Arc::clone(&effects))
                .reconcile(&mut ctx)
                .await
                .expect("secret-service ready"),
            ReconcileOutcome::Satisfied
        );
        assert!(manager.ensured().is_empty());
        assert_eq!(
            ctx.status::<CredentialDriverStatus>(),
            Some(&CredentialDriverStatus::Ready)
        );
    }

    #[tokio::test]
    async fn managed_identity_recover_adopts_a_ready_agent_and_waits_otherwise() {
        let log = log();
        let effects = FakeEffects::new(Arc::clone(&log));
        let manager = RecordingManager::with_child(Arc::clone(&log), agent_child(false));
        let mut ctx = context(row(MI_PROVIDER), Arc::clone(&manager));
        assert_eq!(
            driver(Arc::clone(&effects))
                .recover(&mut ctx)
                .await
                .expect("recover"),
            RecoveryOutcome::Adopted
        );

        effects.set_agent_ready(false);
        assert_eq!(
            driver(Arc::clone(&effects))
                .recover(&mut ctx)
                .await
                .expect("recover"),
            RecoveryOutcome::Missing
        );

        let empty = RecordingManager::new(log);
        let mut ctx = context(row(MI_PROVIDER), empty);
        assert_eq!(
            driver(Arc::clone(&effects))
                .recover(&mut ctx)
                .await
                .expect("recover"),
            RecoveryOutcome::Missing
        );
    }

    #[tokio::test]
    async fn delete_revokes_before_marking_the_agent_child_deleting() {
        let log = log();
        let effects = FakeEffects::new(Arc::clone(&log));
        effects.set_lease(Some(CredentialLeaseFacts {
            state: CredentialLeaseState::Active,
            rotation_generation: 1,
        }));
        let manager = RecordingManager::with_child(Arc::clone(&log), agent_child(false));
        let mut ctx = context(
            row_with(MI_PROVIDER, "Guest/gateway", true),
            Arc::clone(&manager),
        );
        driver(Arc::clone(&effects))
            .delete(&mut ctx)
            .await
            .expect("delete");

        let calls = log.lock().clone();
        let session = calls
            .iter()
            .position(|call| call == "session")
            .expect("revoke");
        let child = calls
            .iter()
            .position(|call| call == "delete:Process/mi-agent-relay")
            .expect("child delete");
        assert!(
            session < child,
            "revocation precedes child deletion: {calls:?}"
        );
        assert!(matches!(
            ctx.status::<CredentialDriverStatus>(),
            Some(CredentialDriverStatus::LeaseRevoked { .. })
        ));
        assert!(manager.children()[0].deleting);
    }

    #[tokio::test]
    async fn delete_fails_closed_without_a_live_session_generation() {
        let log = log();
        let effects = FakeEffects::new(Arc::clone(&log));
        effects.set_lease(Some(CredentialLeaseFacts {
            state: CredentialLeaseState::Active,
            rotation_generation: 1,
        }));
        effects.set_session(Some(Arc::new(RecordingSession::new(None))));
        let manager = RecordingManager::with_child(Arc::clone(&log), agent_child(false));
        let mut ctx = context(
            row_with(MI_PROVIDER, "Guest/gateway", true),
            Arc::clone(&manager),
        );
        let mut delete = driver(Arc::clone(&effects));
        let error = delete.delete(&mut ctx).await.expect_err("fail closed");
        assert!(matches!(
            CredentialDriver::classify_error(&delete, &error).class(),
            FailureClass::Retryable
        ));
        assert!(
            matches!(
                ctx.status::<CredentialDriverStatus>(),
                Some(CredentialDriverStatus::RevocationUncertain { .. })
            ),
            "uncertain revocation retains the lease"
        );
        assert!(!manager.children()[0].deleting, "no child deletion");
        assert!(effects.session.lock().is_some());
    }

    #[tokio::test]
    async fn delete_fails_closed_on_a_stale_session_generation() {
        let log = log();
        let effects = FakeEffects::new(Arc::clone(&log));
        effects.set_lease(Some(CredentialLeaseFacts {
            state: CredentialLeaseState::Unknown,
            rotation_generation: 3,
        }));
        // The session reports generation 7 while the request carries the
        // generation the session handed out before the reconnect: the
        // revocation is `Uncertain`, never a confirmed cleanup trigger.
        effects.set_session(Some(Arc::new(MismatchedSession::new(7, 8))));
        let manager = RecordingManager::with_child(Arc::clone(&log), agent_child(false));
        let mut ctx = context(
            row_with(MI_PROVIDER, "Guest/gateway", true),
            Arc::clone(&manager),
        );
        let error = driver(Arc::clone(&effects))
            .delete(&mut ctx)
            .await
            .expect_err("fail closed");
        assert_eq!(error.to_string(), "credential-revocation-unconfirmed");
        let status = ctx.status::<CredentialDriverStatus>().expect("status");
        assert_eq!(status.phase(), "Degraded");
        assert_eq!(status.outcome_code(), "credential-revocation-uncertain");
        assert!(!manager.children()[0].deleting);
    }

    /// A session whose generation advances between the driver's read and the
    /// revoke call (the reconnect race the request binding exists for).
    struct MismatchedSession {
        reported: ReconnectGeneration,
        actual: ReconnectGeneration,
    }

    impl MismatchedSession {
        fn new(reported: u64, actual: u64) -> Self {
            Self {
                reported: ReconnectGeneration::new(reported).unwrap(),
                actual: ReconnectGeneration::new(actual).unwrap(),
            }
        }
    }

    #[async_trait::async_trait]
    impl CredentialSession for MismatchedSession {
        fn session_generation(&self) -> Option<ReconnectGeneration> {
            Some(self.reported)
        }

        async fn revoke_credential(
            &self,
            request: &CredentialRevocationRequest,
        ) -> Result<CredentialRevocationOutcome, CredentialResourceRuntimeError> {
            if request.session_generation() != self.actual {
                return Ok(CredentialRevocationOutcome::Uncertain);
            }
            Ok(CredentialRevocationOutcome::Revoked)
        }
    }

    #[tokio::test]
    async fn delete_skips_revocation_without_lease_facts_and_retires_the_child() {
        let log = log();
        let effects = FakeEffects::new(Arc::clone(&log));
        let manager = RecordingManager::with_child(Arc::clone(&log), agent_child(false));
        let mut ctx = context(
            row_with(MI_PROVIDER, "Guest/gateway", true),
            Arc::clone(&manager),
        );
        driver(Arc::clone(&effects))
            .delete(&mut ctx)
            .await
            .expect("delete");
        let calls = log.lock().clone();
        assert!(!calls.iter().any(|call| call == "session"), "{calls:?}");
        assert!(
            calls
                .iter()
                .any(|call| call == "delete:Process/mi-agent-relay")
        );
        assert!(ctx.status::<CredentialDriverStatus>().is_none());
    }

    #[tokio::test]
    async fn delete_is_idempotent_under_retry() {
        let log = log();
        let effects = FakeEffects::new(Arc::clone(&log));
        effects.set_lease(Some(CredentialLeaseFacts {
            state: CredentialLeaseState::Active,
            rotation_generation: 1,
        }));
        let manager = RecordingManager::with_child(Arc::clone(&log), agent_child(false));
        let mut delete = driver(Arc::clone(&effects));

        let mut first = context(
            row_with(MI_PROVIDER, "Guest/gateway", true),
            Arc::clone(&manager),
        );
        delete.delete(&mut first).await.expect("first pass");
        let (first_operation, first_evidence) = match first.status::<CredentialDriverStatus>() {
            Some(CredentialDriverStatus::LeaseRevoked { evidence }) => (
                evidence.operation_id().to_owned(),
                (evidence.session_generation().get(), evidence.outcome_code()),
            ),
            other => panic!("expected confirmed revocation, got {other:?}"),
        };
        assert_eq!(first_evidence.0, 7);
        assert_eq!(first_evidence.1, "revoked");

        // The second pass (fresh actor incarnation) derives the same durable
        // revocation identity and the provider reports AlreadyRevoked.
        let mut second = context(
            row_with(MI_PROVIDER, "Guest/gateway", true),
            Arc::clone(&manager),
        );
        delete.delete(&mut second).await.expect("second pass");
        let evidence_second = match second.status::<CredentialDriverStatus>() {
            Some(CredentialDriverStatus::LeaseRevoked { evidence }) => {
                (evidence.operation_id().to_owned(), evidence.outcome_code())
            }
            other => panic!("expected confirmed revocation, got {other:?}"),
        };
        assert_eq!(
            first_operation, evidence_second.0,
            "operation id is durable"
        );
        assert_eq!(evidence_second.1, "already-revoked");
        assert_eq!(
            log.lock()
                .iter()
                .filter(|call| call.as_str() == "delete:Process/mi-agent-relay")
                .count(),
            1,
            "an already-deleting child is not re-deleted"
        );
    }

    #[tokio::test]
    async fn delete_converges_without_children() {
        let log = log();
        let effects = FakeEffects::new(Arc::clone(&log));
        let manager = RecordingManager::new(Arc::clone(&log));
        let mut ctx = context(
            row_with(MI_PROVIDER, "Guest/gateway", true),
            Arc::clone(&manager),
        );
        driver(Arc::clone(&effects))
            .delete(&mut ctx)
            .await
            .expect("delete");
        assert!(manager.ensured().is_empty());
    }

    #[tokio::test]
    async fn reconcile_reports_agent_draining_for_a_deleting_child() {
        let log = log();
        let effects = FakeEffects::new(Arc::clone(&log));
        let manager = RecordingManager::with_child(Arc::clone(&log), agent_child(true));
        let mut ctx = context(row(MI_PROVIDER), manager);
        driver(Arc::clone(&effects))
            .reconcile(&mut ctx)
            .await
            .expect_err("draining agent");
        assert_eq!(
            ctx.status::<CredentialDriverStatus>(),
            Some(&CredentialDriverStatus::AgentDraining)
        );
    }

    #[tokio::test]
    async fn reconcile_deletes_a_drifted_agent_child_before_recreating_it() {
        let log = log();
        let effects = FakeEffects::new(Arc::clone(&log));
        // A child row whose persisted spec is not the derived agent shape
        // (old `managed_identity_agent_matches` drift).
        let mut drifted = agent_child(false);
        drifted.spec = b"{\"drifted\":true}".to_vec();
        let manager = RecordingManager::with_child(Arc::clone(&log), drifted);
        let mut ctx = context(row(MI_PROVIDER), Arc::clone(&manager));
        driver(Arc::clone(&effects))
            .reconcile(&mut ctx)
            .await
            .expect_err("drifted child");
        let calls = log.lock().clone();
        assert!(
            calls
                .iter()
                .any(|call| call == "delete:Process/mi-agent-relay"),
            "{calls:?}"
        );
        assert!(
            !calls.iter().any(|call| call == "ensure:mi-agent-relay"),
            "a drifted child is deleted, not re-ensured in place: {calls:?}"
        );
        assert_eq!(
            ctx.status::<CredentialDriverStatus>(),
            Some(&CredentialDriverStatus::AgentPending)
        );
    }
}
