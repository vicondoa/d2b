use std::{
    collections::BTreeMap,
    error::Error,
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};

use d2b_contracts::{
    v2_component_session::{BoundedVec, EndpointRole},
    v2_identity::ProviderType,
    v2_provider::{
        AdoptionState, AuthorizedProviderScope, Fingerprint, MutationReceipt, MutationState,
        ObservationReason, ObservedLifecycleState, OperationBinding, OperationId,
        PROVIDER_SCHEMA_VERSION, PlanId, PlannedResourceClass, Provider, ProviderAuthority,
        ProviderCallContext, ProviderCapabilitySet, ProviderDescriptor, ProviderFailure,
        ProviderFailureKind, ProviderFuture, ProviderHealth, ProviderHealthReason,
        ProviderHealthState, ProviderMethod, ProviderObservation, ProviderOperationInput,
        ProviderOperationRequest, ProviderPlan, ProviderRemediation, ProviderResult, RetryClass,
        SubstrateProvider,
    },
};
use d2b_provider::{ProviderClock, SystemProviderClock};
use tokio::{
    sync::Mutex as AsyncMutex,
    time::{Instant, timeout_at},
};

use crate::{
    HostApplyInspection, HostApplyOutcome, HostCheckReport, HostCheckRequest,
    HostDescriptorBinding, HostFindingSeverity, HostOperationOwner, HostPlanRequest, HostPortError,
    HostRemediationPlan, HostRemediationPlanDisposition, HostSubstrateConfiguration,
    HostSubstrateInspection, HostSubstratePort,
};

const MAX_CACHED_CHECKS: usize = 128;
const MAX_CACHED_PLANS: usize = 128;
const MAX_CACHED_APPLIES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostProviderConstructionError {
    DescriptorInvalid,
    CapabilityMismatch,
    ImplementationMismatch,
    PlacementMismatch,
}

impl fmt::Display for HostProviderConstructionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DescriptorInvalid => "host substrate provider descriptor is invalid",
            Self::CapabilityMismatch => {
                "host substrate provider capabilities do not match the canonical method set"
            }
            Self::ImplementationMismatch => {
                "host substrate provider implementation identifier is invalid"
            }
            Self::PlacementMismatch => {
                "host substrate provider must run in the local-root controller"
            }
        })
    }
}

impl Error for HostProviderConstructionError {}

#[derive(Clone)]
struct CheckCacheEntry {
    binding: OperationBinding,
    observation: ProviderObservation,
    expires_at_unix_ms: u64,
}

#[derive(Clone)]
struct StoredPlan {
    canonical: ProviderPlan,
    semantic: HostRemediationPlan,
}

#[derive(Clone)]
struct ApplyCacheEntry {
    binding: OperationBinding,
    plan: ProviderPlan,
    receipt: MutationReceipt,
    expires_at_unix_ms: u64,
}

#[derive(Default)]
struct ProviderState {
    checks: BTreeMap<OperationId, CheckCacheEntry>,
    plan_keys: BTreeMap<OperationId, PlanId>,
    plans: BTreeMap<PlanId, StoredPlan>,
    applies: BTreeMap<OperationId, ApplyCacheEntry>,
    latest_report: Option<HostCheckReport>,
    latest_plan: Option<HostRemediationPlan>,
    latest_apply: Option<HostApplyInspection>,
}

struct HostProviderCore {
    descriptor: ProviderDescriptor,
    descriptor_binding: HostDescriptorBinding,
    configuration: HostSubstrateConfiguration,
    port: Arc<dyn HostSubstratePort>,
    clock: Arc<dyn ProviderClock>,
    state: Mutex<ProviderState>,
    operation_gate: AsyncMutex<()>,
}

impl fmt::Debug for HostProviderCore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostProviderCore")
            .field("configuration", &self.configuration)
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

pub(crate) fn validate_host_descriptor(
    descriptor: &ProviderDescriptor,
    configuration: HostSubstrateConfiguration,
) -> Result<(), HostProviderConstructionError> {
    descriptor
        .validate()
        .map_err(|_| HostProviderConstructionError::DescriptorInvalid)?;
    if descriptor.authority != ProviderAuthority::Substrate
        || descriptor.provider_type() != ProviderType::Substrate
    {
        return Err(HostProviderConstructionError::DescriptorInvalid);
    }
    if descriptor.implementation_id.as_str() != configuration.substrate().implementation_id() {
        return Err(HostProviderConstructionError::ImplementationMismatch);
    }
    let exact_methods = [
        ProviderMethod::SubstrateCheck,
        ProviderMethod::SubstratePlanRemediation,
        ProviderMethod::SubstrateApply,
    ];
    if descriptor.capabilities.as_slice().len() != exact_methods.len()
        || !exact_methods
            .into_iter()
            .all(|method| descriptor.capabilities.contains_method(method))
    {
        return Err(HostProviderConstructionError::CapabilityMismatch);
    }
    if !matches!(
        descriptor.placement,
        d2b_contracts::v2_provider::ProviderPlacement::TrustedFirstPartyInProcess {
            controller_role: EndpointRole::LocalRootController,
            ..
        }
    ) {
        return Err(HostProviderConstructionError::PlacementMismatch);
    }
    Ok(())
}

impl HostProviderCore {
    fn new(
        descriptor: ProviderDescriptor,
        configuration: HostSubstrateConfiguration,
        port: Arc<dyn HostSubstratePort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, HostProviderConstructionError> {
        validate_host_descriptor(&descriptor, configuration)?;
        let descriptor_binding = HostDescriptorBinding::from_descriptor(&descriptor);
        Ok(Self {
            descriptor,
            descriptor_binding,
            configuration,
            port,
            clock,
            state: Mutex::new(ProviderState::default()),
            operation_gate: AsyncMutex::new(()),
        })
    }

    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn capabilities(&self) -> ProviderCapabilitySet {
        self.descriptor.capabilities.clone()
    }

    fn failure(
        &self,
        context: &ProviderCallContext<'_>,
        kind: ProviderFailureKind,
        retry: RetryClass,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderFailure {
        ProviderFailure {
            kind,
            retry,
            provider_type: ProviderType::Substrate,
            binding: context.operation.binding(),
            correlation_id: context.operation.correlation_id.clone(),
            occurred_at_unix_ms: self.clock.now_unix_ms(),
            reason,
            remediation,
        }
    }

    fn cancelled(&self, context: &ProviderCallContext<'_>) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::Cancelled,
            RetryClass::SameOperation,
            ProviderHealthReason::SessionDisconnected,
            ProviderRemediation::RetryBounded,
        )
    }

    fn deadline(&self, context: &ProviderCallContext<'_>) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::DeadlineExpired,
            RetryClass::SameOperation,
            ProviderHealthReason::HealthTimeout,
            ProviderRemediation::RetryBounded,
        )
    }

    fn invalid(&self, context: &ProviderCallContext<'_>) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::InvalidRequest,
            RetryClass::Never,
            ProviderHealthReason::CapabilityMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    fn invariant(&self, context: &ProviderCallContext<'_>) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::InvariantViolation,
            RetryClass::Never,
            ProviderHealthReason::CapabilityMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    fn capacity(&self, context: &ProviderCallContext<'_>) -> ProviderFailure {
        self.failure(
            context,
            ProviderFailureKind::Unavailable,
            RetryClass::SameOperation,
            ProviderHealthReason::QueuePressure,
            ProviderRemediation::RetryBounded,
        )
    }

    fn map_port_error(
        &self,
        context: &ProviderCallContext<'_>,
        error: HostPortError,
    ) -> ProviderFailure {
        match error {
            HostPortError::Denied => self.failure(
                context,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::OperatorInteraction,
            ),
            HostPortError::Unavailable => self.failure(
                context,
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ),
            HostPortError::StaleGeneration => self.failure(
                context,
                ProviderFailureKind::RegistryChanged,
                RetryClass::Never,
                ProviderHealthReason::GenerationMismatch,
                ProviderRemediation::ReplaceGeneration,
            ),
            HostPortError::Cancelled => self.cancelled(context),
            HostPortError::DeadlineExpired => self.deadline(context),
            HostPortError::InvalidResponse => self.invariant(context),
        }
    }

    fn preflight_context(
        &self,
        context: &ProviderCallContext<'_>,
        expected: ProviderMethod,
    ) -> ProviderResult<u64> {
        if context.cancelled {
            return Err(self.cancelled(context));
        }
        if context.monotonic_deadline_remaining_ms == 0 {
            return Err(self.deadline(context));
        }
        let now = self.clock.now_unix_ms();
        if now >= context.operation.expires_at_unix_ms {
            return Err(self.deadline(context));
        }
        if context.operation.provider_generation != self.descriptor.registry_generation {
            return Err(self.failure(
                context,
                ProviderFailureKind::RegistryChanged,
                RetryClass::Never,
                ProviderHealthReason::GenerationMismatch,
                ProviderRemediation::ReplaceGeneration,
            ));
        }
        if context.validate().is_err()
            || context.operation.validate(&self.descriptor, now).is_err()
            || context.operation.method != expected
        {
            return Err(self.invalid(context));
        }
        let expected_role = match self.descriptor.placement {
            d2b_contracts::v2_provider::ProviderPlacement::TrustedFirstPartyInProcess {
                controller_role,
                ..
            } => controller_role,
            d2b_contracts::v2_provider::ProviderPlacement::ProviderAgent { .. }
            | d2b_contracts::v2_provider::ProviderPlacement::UserAgent { .. } => {
                return Err(self.invalid(context));
            }
        };
        if context.peer_role != expected_role
            || !matches!(
                context.operation.scope,
                AuthorizedProviderScope::Realm { .. }
            )
        {
            return Err(self.failure(
                context,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::OperatorInteraction,
            ));
        }
        Ok(now)
    }

    fn preflight_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        expected: ProviderMethod,
    ) -> ProviderResult<u64> {
        let now = self.preflight_context(context, expected)?;
        if request.context != *context.operation
            || request
                .validate_method(&self.descriptor, now, expected)
                .is_err()
            || !matches!(
                request.target,
                d2b_contracts::v2_provider::ProviderTarget::Realm { .. }
            )
            || request.target.workload_id().is_some()
            || !matches!(request.input, ProviderOperationInput::NoInput)
        {
            return Err(self.invalid(context));
        }
        Ok(now)
    }

    fn operation_deadline(&self, context: &ProviderCallContext<'_>, now: u64) -> Instant {
        let monotonic = Duration::from_millis(u64::from(context.monotonic_deadline_remaining_ms));
        let wall_clock =
            Duration::from_millis(context.operation.expires_at_unix_ms.saturating_sub(now));
        Instant::now() + monotonic.min(wall_clock)
    }

    fn deadline_remaining_ms(deadline: Instant) -> Option<u32> {
        let remaining = deadline.checked_duration_since(Instant::now())?;
        if remaining.is_zero() {
            return None;
        }
        let millis = remaining.as_millis().max(1);
        Some(u32::try_from(millis).unwrap_or(u32::MAX))
    }

    fn prune_operation_caches(&self, state: &mut ProviderState, now: u64) {
        state
            .checks
            .retain(|_, entry| entry.expires_at_unix_ms > now);
        state
            .applies
            .retain(|_, entry| entry.expires_at_unix_ms > now);
    }

    fn owner(&self, context: &ProviderCallContext<'_>) -> HostOperationOwner {
        HostOperationOwner {
            realm_id: context.operation.scope.realm_id().clone(),
            principal: context.operation.principal.clone(),
        }
    }

    fn health_from_report(&self, report: Option<&HostCheckReport>, now: u64) -> ProviderHealth {
        let (state, reason, remediation) = match report {
            Some(report)
                if report
                    .support()
                    .confirms_required_capabilities(self.configuration.check_profile())
                    && report
                        .findings()
                        .iter()
                        .all(|finding| finding.severity() == HostFindingSeverity::Advisory) =>
            {
                (
                    ProviderHealthState::Healthy,
                    ProviderHealthReason::None,
                    ProviderRemediation::None,
                )
            }
            Some(report)
                if report
                    .support()
                    .has_unsupported_required_capability(self.configuration.check_profile())
                    || report
                        .findings()
                        .iter()
                        .any(|finding| finding.severity() != HostFindingSeverity::Advisory) =>
            {
                (
                    ProviderHealthState::Degraded,
                    ProviderHealthReason::ProviderDegraded,
                    ProviderRemediation::RepairConfiguration,
                )
            }
            Some(_) | None => (
                ProviderHealthState::Degraded,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::InspectProvider,
            ),
        };
        ProviderHealth {
            provider_id: self.descriptor.provider_id.clone(),
            registry_generation: self.descriptor.registry_generation,
            observed_at_unix_ms: now,
            state,
            reason,
            remediation,
        }
    }

    fn observation_for_report(
        &self,
        request: &ProviderOperationRequest,
        report: &HostCheckReport,
    ) -> ProviderObservation {
        let health = self.health_from_report(Some(report), report.observed_at_unix_ms());
        let reason = if report
            .support()
            .has_unsupported_required_capability(self.configuration.check_profile())
            || report
                .findings()
                .iter()
                .any(|finding| finding.severity() != HostFindingSeverity::Advisory)
        {
            ObservationReason::ConfigurationMismatch
        } else if report
            .support()
            .has_missing_required_evidence(self.configuration.check_profile())
        {
            ObservationReason::MissingEvidence
        } else {
            ObservationReason::None
        };
        let lifecycle = if health.state == ProviderHealthState::Healthy {
            ObservedLifecycleState::Ready
        } else {
            ObservedLifecycleState::Unknown
        };
        ProviderObservation {
            provider_id: self.descriptor.provider_id.clone(),
            provider_generation: self.descriptor.registry_generation,
            realm_id: request.target.realm_id().clone(),
            workload_id: None,
            handle_id: None,
            resource_generation: None,
            observed_at_unix_ms: report.observed_at_unix_ms(),
            lifecycle,
            adoption: AdoptionState::NotAttempted,
            reason,
            health,
        }
    }

    fn validate_report(
        &self,
        context: &ProviderCallContext<'_>,
        report: &HostCheckReport,
        owner: &HostOperationOwner,
        now: u64,
    ) -> ProviderResult<()> {
        if report.validate().is_err()
            || report.configuration() != self.configuration
            || report.descriptor() != &self.descriptor_binding
            || report.owner() != owner
            || report.operation() != &context.operation.binding()
            || report.observed_at_unix_ms() < context.operation.issued_at_unix_ms
            || report.observed_at_unix_ms() > now
        {
            Err(self.invariant(context))
        } else {
            Ok(())
        }
    }

    async fn check(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderObservation> {
        let admitted_at =
            self.preflight_request(context, request, ProviderMethod::SubstrateCheck)?;
        let deadline = self.operation_deadline(context, admitted_at);
        let binding = context.operation.binding();
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            self.prune_operation_caches(&mut state, admitted_at);
            if let Some(cached) = state.checks.get(&binding.operation_id) {
                return if cached.binding == binding {
                    Ok(cached.observation.clone())
                } else {
                    Err(self.invalid(context))
                };
            }
            if state
                .checks
                .values()
                .any(|cached| cached.binding.idempotency_key == binding.idempotency_key)
            {
                return Err(self.invalid(context));
            }
            if state.checks.len() >= MAX_CACHED_CHECKS {
                return Err(self.capacity(context));
            }
        }
        let _gate = timeout_at(deadline, self.operation_gate.lock())
            .await
            .map_err(|_| self.deadline(context))?;
        let started_at = self.clock.now_unix_ms();
        let deadline_remaining_ms =
            Self::deadline_remaining_ms(deadline).ok_or_else(|| self.deadline(context))?;
        if started_at >= context.operation.expires_at_unix_ms {
            return Err(self.deadline(context));
        }
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            self.prune_operation_caches(&mut state, started_at);
            if let Some(cached) = state.checks.get(&binding.operation_id) {
                return if cached.binding == binding {
                    Ok(cached.observation.clone())
                } else {
                    Err(self.invalid(context))
                };
            }
            if state
                .checks
                .values()
                .any(|cached| cached.binding.idempotency_key == binding.idempotency_key)
            {
                return Err(self.invalid(context));
            }
            if state.checks.len() >= MAX_CACHED_CHECKS {
                return Err(self.capacity(context));
            }
        }
        let owner = self.owner(context);
        let port_request = HostCheckRequest::new(
            self.configuration,
            self.descriptor_binding.clone(),
            owner.clone(),
            binding.clone(),
            deadline_remaining_ms,
        );
        let report = match timeout_at(deadline, self.port.check(port_request)).await {
            Ok(Ok(report)) => report,
            Ok(Err(error)) => return Err(self.map_port_error(context, error)),
            Err(_) => return Err(self.deadline(context)),
        };
        let completed_at = self.clock.now_unix_ms();
        if completed_at >= context.operation.expires_at_unix_ms {
            return Err(self.deadline(context));
        }
        self.validate_report(context, &report, &owner, completed_at)?;
        let observation = self.observation_for_report(request, &report);
        if observation.validate().is_err() {
            return Err(self.invariant(context));
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        self.prune_operation_caches(&mut state, completed_at);
        if let Some(cached) = state.checks.get(&binding.operation_id) {
            return if cached.binding == binding {
                Ok(cached.observation.clone())
            } else {
                Err(self.invalid(context))
            };
        }
        if state
            .checks
            .values()
            .any(|cached| cached.binding.idempotency_key == binding.idempotency_key)
        {
            return Err(self.invalid(context));
        }
        if state.checks.len() >= MAX_CACHED_CHECKS {
            return Err(self.capacity(context));
        }
        if state
            .latest_plan
            .as_ref()
            .is_some_and(|plan| plan.report_fingerprint() != report.report_fingerprint())
        {
            state.latest_plan = None;
        }
        state.latest_apply = None;
        state.latest_report = Some(report.clone());
        state.checks.insert(
            binding.operation_id.clone(),
            CheckCacheEntry {
                binding,
                observation: observation.clone(),
                expires_at_unix_ms: context.operation.expires_at_unix_ms,
            },
        );
        Ok(observation)
    }

    fn validate_semantic_plan(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &HostRemediationPlan,
        owner: &HostOperationOwner,
        expected_report_fingerprint: Option<&Fingerprint>,
        now: u64,
    ) -> ProviderResult<()> {
        if plan.validate().is_err()
            || plan.configuration() != self.configuration
            || plan.descriptor() != &self.descriptor_binding
            || plan.owner() != owner
            || plan.operation() != &context.operation.binding()
            || expected_report_fingerprint
                .is_some_and(|expected| plan.report_fingerprint() != expected)
            || plan.created_at_unix_ms() < context.operation.issued_at_unix_ms
            || plan.created_at_unix_ms() > now
            || plan.expires_at_unix_ms() <= now
            || plan.expires_at_unix_ms() > context.operation.expires_at_unix_ms
        {
            Err(self.invariant(context))
        } else {
            Ok(())
        }
    }

    fn canonical_plan(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        semantic: &HostRemediationPlan,
        now: u64,
    ) -> ProviderResult<ProviderPlan> {
        let resource_values =
            if semantic.disposition() == HostRemediationPlanDisposition::Authorized {
                vec![PlannedResourceClass::SubstrateRemediation]
            } else {
                Vec::new()
            };
        let resources = BoundedVec::new(resource_values).map_err(|_| self.invariant(context))?;
        let plan = ProviderPlan {
            schema_version: PROVIDER_SCHEMA_VERSION,
            plan_id: semantic.remediation_id().as_plan_id(),
            binding: context.operation.binding(),
            realm_id: request.target.realm_id().clone(),
            workload_id: None,
            method: ProviderMethod::SubstratePlanRemediation,
            configuration_fingerprint: request.expected_configuration_fingerprint.clone(),
            created_at_unix_ms: semantic.created_at_unix_ms(),
            expires_at_unix_ms: semantic.expires_at_unix_ms(),
            resources,
        };
        plan.validate(request, now)
            .map_err(|_| self.invariant(context))?;
        Ok(plan)
    }

    async fn plan_remediation(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderPlan> {
        let admitted_at =
            self.preflight_request(context, request, ProviderMethod::SubstratePlanRemediation)?;
        let deadline = self.operation_deadline(context, admitted_at);
        let binding = context.operation.binding();
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            self.prune_operation_caches(&mut state, admitted_at);
            self.prune_expired_plans(&mut state, admitted_at);
            if let Some(plan_id) = state.plan_keys.get(&binding.operation_id) {
                let stored = state
                    .plans
                    .get(plan_id)
                    .ok_or_else(|| self.invariant(context))?;
                return if stored.semantic.operation() == &binding {
                    Ok(stored.canonical.clone())
                } else {
                    Err(self.invalid(context))
                };
            }
            if state.plans.values().any(|stored| {
                stored.semantic.operation().idempotency_key == binding.idempotency_key
            }) {
                return Err(self.invalid(context));
            }
            if state.plans.len() >= MAX_CACHED_PLANS {
                return Err(self.capacity(context));
            }
        }
        let _gate = timeout_at(deadline, self.operation_gate.lock())
            .await
            .map_err(|_| self.deadline(context))?;
        let started_at = self.clock.now_unix_ms();
        let deadline_remaining_ms =
            Self::deadline_remaining_ms(deadline).ok_or_else(|| self.deadline(context))?;
        if started_at >= context.operation.expires_at_unix_ms {
            return Err(self.deadline(context));
        }
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            self.prune_operation_caches(&mut state, started_at);
            self.prune_expired_plans(&mut state, started_at);
            if let Some(plan_id) = state.plan_keys.get(&binding.operation_id) {
                let stored = state
                    .plans
                    .get(plan_id)
                    .ok_or_else(|| self.invariant(context))?;
                return if stored.semantic.operation() == &binding {
                    Ok(stored.canonical.clone())
                } else {
                    Err(self.invalid(context))
                };
            }
            if state.plans.values().any(|stored| {
                stored.semantic.operation().idempotency_key == binding.idempotency_key
            }) {
                return Err(self.invalid(context));
            }
            if state.plans.len() >= MAX_CACHED_PLANS {
                return Err(self.capacity(context));
            }
        }
        let owner = self.owner(context);
        let latest_report_fingerprint = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .latest_report
            .as_ref()
            .filter(|report| {
                report.owner() == &owner && report.descriptor() == &self.descriptor_binding
            })
            .map(|report| report.report_fingerprint().clone());
        let port_request = HostPlanRequest::new(
            self.configuration,
            self.descriptor_binding.clone(),
            owner.clone(),
            binding.clone(),
            latest_report_fingerprint.clone(),
            deadline_remaining_ms,
        );
        let semantic = match timeout_at(deadline, self.port.plan_remediation(port_request)).await {
            Ok(Ok(plan)) => plan,
            Ok(Err(error)) => return Err(self.map_port_error(context, error)),
            Err(_) => return Err(self.deadline(context)),
        };
        let completed_at = self.clock.now_unix_ms();
        if completed_at >= context.operation.expires_at_unix_ms {
            return Err(self.deadline(context));
        }
        self.validate_semantic_plan(
            context,
            &semantic,
            &owner,
            latest_report_fingerprint.as_ref(),
            completed_at,
        )?;
        let canonical = self.canonical_plan(context, request, &semantic, completed_at)?;
        let plan_id = canonical.plan_id.clone();
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        self.prune_operation_caches(&mut state, completed_at);
        self.prune_expired_plans(&mut state, completed_at);
        if let Some(existing_id) = state.plan_keys.get(&binding.operation_id) {
            let existing = state
                .plans
                .get(existing_id)
                .ok_or_else(|| self.invariant(context))?;
            return if existing.semantic.operation() == &binding {
                Ok(existing.canonical.clone())
            } else {
                Err(self.invalid(context))
            };
        }
        if state
            .plans
            .values()
            .any(|stored| stored.semantic.operation().idempotency_key == binding.idempotency_key)
        {
            return Err(self.invalid(context));
        }
        if let Some(existing) = state.plans.get(&plan_id)
            && (existing.canonical != canonical || existing.semantic != semantic)
        {
            return Err(self.invariant(context));
        }
        if state.plans.len() >= MAX_CACHED_PLANS {
            return Err(self.capacity(context));
        }
        state.latest_apply = None;
        state.latest_plan = Some(semantic.clone());
        state
            .plan_keys
            .insert(binding.operation_id.clone(), plan_id.clone());
        state.plans.insert(
            plan_id,
            StoredPlan {
                canonical: canonical.clone(),
                semantic,
            },
        );
        Ok(canonical)
    }

    fn prune_expired_plans(&self, state: &mut ProviderState, now: u64) {
        state
            .plans
            .retain(|_, plan| plan.semantic.expires_at_unix_ms() > now);
        state
            .plan_keys
            .retain(|_, plan_id| state.plans.contains_key(plan_id));
        if state
            .latest_plan
            .as_ref()
            .is_some_and(|plan| plan.expires_at_unix_ms() <= now)
        {
            state.latest_plan = None;
        }
    }

    fn receipt(
        &self,
        context: &ProviderCallContext<'_>,
        outcome: HostApplyOutcome,
        now: u64,
    ) -> MutationReceipt {
        let (state, observation_required_before_retry) = match outcome {
            HostApplyOutcome::Applied => (MutationState::Applied, false),
            HostApplyOutcome::AlreadyApplied => (MutationState::AlreadyApplied, false),
            HostApplyOutcome::NotApplicable => (MutationState::NotApplicable, false),
            HostApplyOutcome::CancelledBeforeMutation => {
                (MutationState::CancelledBeforeMutation, false)
            }
            HostApplyOutcome::CompletionAmbiguous => (MutationState::CompletionAmbiguous, true),
        };
        MutationReceipt {
            binding: context.operation.binding(),
            state,
            observed_at_unix_ms: now,
            observation_required_before_retry,
        }
    }

    async fn apply(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &ProviderPlan,
    ) -> ProviderResult<MutationReceipt> {
        let admitted_at = self.preflight_context(context, ProviderMethod::SubstrateApply)?;
        let deadline = self.operation_deadline(context, admitted_at);
        let apply_binding = context.operation.binding();
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            self.prune_operation_caches(&mut state, admitted_at);
            if let Some(cached) = state.applies.get(&apply_binding.operation_id) {
                return if cached.binding == apply_binding && cached.plan == *plan {
                    Ok(cached.receipt.clone())
                } else {
                    Err(self.invalid(context))
                };
            }
            if state
                .applies
                .values()
                .any(|cached| cached.binding.idempotency_key == apply_binding.idempotency_key)
            {
                return Err(self.invalid(context));
            }
            if state.applies.len() >= MAX_CACHED_APPLIES {
                return Err(self.capacity(context));
            }
        }
        let _gate = timeout_at(deadline, self.operation_gate.lock())
            .await
            .map_err(|_| self.deadline(context))?;
        let started_at = self.clock.now_unix_ms();
        if started_at >= context.operation.expires_at_unix_ms
            || Self::deadline_remaining_ms(deadline).is_none()
        {
            return Err(self.deadline(context));
        }
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            self.prune_operation_caches(&mut state, started_at);
            if let Some(cached) = state.applies.get(&apply_binding.operation_id) {
                return if cached.binding == apply_binding && cached.plan == *plan {
                    Ok(cached.receipt.clone())
                } else {
                    Err(self.invalid(context))
                };
            }
            if state
                .applies
                .values()
                .any(|cached| cached.binding.idempotency_key == apply_binding.idempotency_key)
            {
                return Err(self.invalid(context));
            }
            if state.applies.len() >= MAX_CACHED_APPLIES {
                return Err(self.capacity(context));
            }
        }
        let owner = self.owner(context);
        let (stored, latest_report_matches) = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            self.prune_expired_plans(&mut state, started_at);
            let stored = state
                .plans
                .get(&plan.plan_id)
                .cloned()
                .ok_or_else(|| self.invalid(context))?;
            let latest_report_matches = state.latest_report.as_ref().is_none_or(|report| {
                report.report_fingerprint() == stored.semantic.report_fingerprint()
            });
            (stored, latest_report_matches)
        };
        if !latest_report_matches
            || stored.canonical != *plan
            || stored.semantic.descriptor() != &self.descriptor_binding
            || stored.semantic.owner() != &owner
            || stored.semantic.expires_at_unix_ms() <= started_at
            || plan.binding.provider_generation != self.descriptor.registry_generation
            || plan.configuration_fingerprint != self.descriptor.configuration_schema_fingerprint
            || plan.realm_id != *context.operation.scope.realm_id()
            || plan.workload_id.is_some()
            || plan.method != ProviderMethod::SubstratePlanRemediation
        {
            return Err(self.invalid(context));
        }
        let outcome =
            if stored.semantic.disposition() == HostRemediationPlanDisposition::NotApplicable {
                HostApplyOutcome::NotApplicable
            } else {
                match timeout_at(
                    deadline,
                    self.port.apply(stored.semantic.remediation_id().clone()),
                )
                .await
                {
                    Ok(Ok(outcome)) => outcome,
                    Ok(Err(error)) => return Err(self.map_port_error(context, error)),
                    Err(_) => HostApplyOutcome::CompletionAmbiguous,
                }
            };
        let observed_at = self.clock.now_unix_ms();
        let receipt = self.receipt(context, outcome, observed_at);
        if receipt.validate().is_err() {
            return Err(self.invariant(context));
        }
        let inspection = HostApplyInspection::new(
            stored.semantic.remediation_id().clone(),
            outcome,
            observed_at,
        );
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        self.prune_operation_caches(&mut state, observed_at);
        if let Some(cached) = state.applies.get(&apply_binding.operation_id) {
            return if cached.binding == apply_binding && cached.plan == *plan {
                Ok(cached.receipt.clone())
            } else {
                Err(self.invalid(context))
            };
        }
        if state
            .applies
            .values()
            .any(|cached| cached.binding.idempotency_key == apply_binding.idempotency_key)
        {
            return Err(self.invalid(context));
        }
        if state.applies.len() >= MAX_CACHED_APPLIES {
            return Err(self.capacity(context));
        }
        state.latest_plan = Some(stored.semantic.clone());
        state.latest_apply = Some(inspection);
        state.applies.insert(
            apply_binding.operation_id.clone(),
            ApplyCacheEntry {
                binding: apply_binding,
                plan: plan.clone(),
                receipt: receipt.clone(),
                expires_at_unix_ms: context.operation.expires_at_unix_ms,
            },
        );
        Ok(receipt)
    }

    async fn inspect(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<HostSubstrateInspection> {
        let now = self.preflight_request(context, request, ProviderMethod::SubstrateCheck)?;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        self.prune_operation_caches(&mut state, now);
        self.prune_expired_plans(&mut state, now);
        Ok(HostSubstrateInspection::new(
            self.configuration,
            self.descriptor_binding.clone(),
            state.latest_report.clone(),
            state.latest_plan.clone(),
            state.latest_apply.clone(),
        ))
    }

    fn health(&self, context: &ProviderCallContext<'_>) -> ProviderResult<ProviderHealth> {
        let now = self.preflight_context(context, context.operation.method)?;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        self.prune_operation_caches(&mut state, now);
        self.prune_expired_plans(&mut state, now);
        let mut health = self.health_from_report(state.latest_report.as_ref(), now);
        if state
            .latest_apply
            .as_ref()
            .is_some_and(|apply| apply.outcome() == HostApplyOutcome::CompletionAmbiguous)
        {
            health.state = ProviderHealthState::Degraded;
            health.reason = ProviderHealthReason::ProviderDegraded;
            health.remediation = ProviderRemediation::InspectProvider;
        }
        if health.validate().is_err() {
            Err(self.invariant(context))
        } else {
            Ok(health)
        }
    }
}

#[derive(Clone)]
pub struct NixOsSubstrateProvider {
    core: Arc<HostProviderCore>,
}

impl fmt::Debug for NixOsSubstrateProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NixOsSubstrateProvider")
            .field("core", &self.core)
            .finish()
    }
}

impl NixOsSubstrateProvider {
    pub fn new(
        descriptor: ProviderDescriptor,
        port: Arc<dyn HostSubstratePort>,
    ) -> Result<Self, HostProviderConstructionError> {
        Self::with_clock(descriptor, port, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        descriptor: ProviderDescriptor,
        port: Arc<dyn HostSubstratePort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, HostProviderConstructionError> {
        Ok(Self {
            core: Arc::new(HostProviderCore::new(
                descriptor,
                HostSubstrateConfiguration::nixos(),
                port,
                clock,
            )?),
        })
    }

    pub async fn inspect(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<HostSubstrateInspection> {
        self.core.inspect(context, request).await
    }
}

#[derive(Clone)]
pub struct LinuxSubstrateProvider {
    core: Arc<HostProviderCore>,
}

impl fmt::Debug for LinuxSubstrateProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LinuxSubstrateProvider")
            .field("core", &self.core)
            .finish()
    }
}

impl LinuxSubstrateProvider {
    pub fn new(
        descriptor: ProviderDescriptor,
        port: Arc<dyn HostSubstratePort>,
    ) -> Result<Self, HostProviderConstructionError> {
        Self::with_clock(descriptor, port, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        descriptor: ProviderDescriptor,
        port: Arc<dyn HostSubstratePort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, HostProviderConstructionError> {
        Ok(Self {
            core: Arc::new(HostProviderCore::new(
                descriptor,
                HostSubstrateConfiguration::generic_linux(),
                port,
                clock,
            )?),
        })
    }

    pub async fn inspect(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<HostSubstrateInspection> {
        self.core.inspect(context, request).await
    }
}

pub type NixosSubstrateProvider = NixOsSubstrateProvider;
pub type GenericLinuxSubstrateProvider = LinuxSubstrateProvider;

macro_rules! implement_provider {
    ($provider:ty) => {
        impl Provider for $provider {
            fn descriptor(&self) -> ProviderDescriptor {
                self.core.descriptor()
            }

            fn health<'a>(
                &'a self,
                context: &'a ProviderCallContext<'a>,
            ) -> ProviderFuture<'a, ProviderHealth> {
                Box::pin(async move { self.core.health(context) })
            }
        }

        impl SubstrateProvider for $provider {
            fn capabilities(&self) -> ProviderCapabilitySet {
                self.core.capabilities()
            }

            fn check<'a>(
                &'a self,
                context: &'a ProviderCallContext<'a>,
                request: &'a ProviderOperationRequest,
            ) -> ProviderFuture<'a, ProviderObservation> {
                Box::pin(self.core.check(context, request))
            }

            fn plan_remediation<'a>(
                &'a self,
                context: &'a ProviderCallContext<'a>,
                request: &'a ProviderOperationRequest,
            ) -> ProviderFuture<'a, ProviderPlan> {
                Box::pin(self.core.plan_remediation(context, request))
            }

            fn apply<'a>(
                &'a self,
                context: &'a ProviderCallContext<'a>,
                plan: &'a ProviderPlan,
            ) -> ProviderFuture<'a, MutationReceipt> {
                Box::pin(self.core.apply(context, plan))
            }
        }
    };
}

implement_provider!(NixOsSubstrateProvider);
implement_provider!(LinuxSubstrateProvider);
