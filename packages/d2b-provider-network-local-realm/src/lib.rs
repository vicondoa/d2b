//! Realm-local network provider implementation boundary.
//!
//! Nix and the generated bundle remain the sole authority for CIDRs, interface
//! names, DHCP data, and firewall policy. This crate accepts only opaque
//! generated network, lease, role, and semantic resource identifiers and
//! forwards them to an injected async effect port. It never calls a broker.

#![forbid(unsafe_code)]
#![allow(clippy::result_large_err)]

use std::{collections::BTreeMap, error::Error, fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::BoundedVec,
    v2_identity::{ProviderId, ProviderType, RealmId, RoleId},
    v2_provider::{
        AdoptionRequest, AdoptionState, AuthorizedProviderScope, Fingerprint, Generation, HandleId,
        HandleOwner, IdempotencyKey, ImplementationId, MAX_PROVIDER_PLAN_RESOURCES,
        MutationReceipt, MutationState, ObservationReason, ObservedLifecycleState,
        OperationBinding, PlanId, PlannedResourceClass, PrincipalRef, Provider,
        ProviderCallContext, ProviderCapabilitySet, ProviderDescriptor, ProviderFactoryKey,
        ProviderFailure, ProviderFailureKind, ProviderFuture, ProviderHandle, ProviderHealth,
        ProviderHealthReason, ProviderHealthState, ProviderMethod, ProviderObservation,
        ProviderOperationRequest, ProviderPlacement, ProviderPlan, ProviderRemediation,
        ProviderResult, ProviderTarget, RetryClass,
    },
};
use d2b_provider::{
    FactoryError, NetworkProvider, ProviderClock, ProviderFactory, ProviderInstance,
    SystemProviderClock,
};
use d2b_provider_toolkit::ProviderValues;
use tokio::sync::Mutex;

const MAX_TRACKED_OPERATIONS: usize = 128;
const PLAN_TTL_MS: u64 = 30_000;
pub const IMPLEMENTATION_ID: &str = "local-realm";

pub fn implementation_id() -> ImplementationId {
    ImplementationId::parse(IMPLEMENTATION_ID)
        .unwrap_or_else(|_| unreachable!("local-realm implementation ID is valid"))
}

pub fn provider_factory_key() -> ProviderFactoryKey {
    ProviderFactoryKey {
        provider_type: ProviderType::Network,
        implementation_id: implementation_id(),
    }
}

macro_rules! opaque_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, OpaqueIdError> {
                let value = value.into();
                let valid = !value.is_empty()
                    && value.len() <= 64
                    && value.as_bytes()[0].is_ascii_lowercase()
                    && value.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    });
                if valid {
                    Ok(Self(value))
                } else {
                    Err(OpaqueIdError)
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_tuple(stringify!($name))
                    .field(&"<redacted>")
                    .finish()
            }
        }
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpaqueIdError;

impl fmt::Display for OpaqueIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("invalid opaque network identifier")
    }
}

impl Error for OpaqueIdError {}

opaque_id!(NetworkId, "An opaque generated realm network identifier.");
opaque_id!(
    NetworkLeaseId,
    "An opaque allocator-issued network lease identifier."
);
opaque_id!(
    NetworkResourceId,
    "An opaque generated network semantic resource identifier."
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkLiveCapabilities {
    pub bridge: bool,
    pub tap: bool,
    pub net_vm: bool,
    pub nat: bool,
    pub dhcp: bool,
    pub nftables: bool,
    pub netlink: bool,
    pub external_attachment: bool,
}

impl NetworkLiveCapabilities {
    pub const REQUIRED: Self = Self {
        bridge: true,
        tap: true,
        net_vm: true,
        nat: true,
        dhcp: true,
        nftables: true,
        netlink: true,
        external_attachment: true,
    };

    const fn is_complete(self) -> bool {
        self.bridge
            && self.tap
            && self.net_vm
            && self.nat
            && self.dhcp
            && self.nftables
            && self.netlink
            && self.external_attachment
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct LocalRealmNetworkBinding {
    pub realm_id: RealmId,
    pub network_id: NetworkId,
    pub allocator_lease_id: NetworkLeaseId,
    pub bridge_set_id: NetworkResourceId,
    pub tap_set_id: NetworkResourceId,
    pub net_vm_role_id: RoleId,
    pub nat_policy_id: NetworkResourceId,
    pub dhcp_policy_id: NetworkResourceId,
    pub nft_policy_id: NetworkResourceId,
    pub netlink_policy_id: NetworkResourceId,
    pub external_attachment_id: Option<NetworkResourceId>,
    pub resource_generation: Generation,
}

impl fmt::Debug for LocalRealmNetworkBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalRealmNetworkBinding")
            .field("resource_generation", &self.resource_generation)
            .field(
                "external_attachment_configured",
                &self.external_attachment_id.is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl LocalRealmNetworkBinding {
    fn owner(&self) -> HandleOwner {
        HandleOwner::RealmController {
            realm_id: self.realm_id.clone(),
        }
    }

    fn resources_are_distinct(&self) -> bool {
        let mut resources = vec![
            &self.bridge_set_id,
            &self.tap_set_id,
            &self.nat_policy_id,
            &self.dhcp_policy_id,
            &self.nft_policy_id,
            &self.netlink_policy_id,
        ];
        if let Some(external) = &self.external_attachment_id {
            resources.push(external);
        }
        resources
            .iter()
            .enumerate()
            .all(|(index, resource)| resources[index + 1..].iter().all(|other| resource != other))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NetworkResourceBinding {
    pub provider_id: ProviderId,
    pub realm_id: RealmId,
    pub owner: HandleOwner,
    pub provider_generation: Generation,
    pub resource_generation: Generation,
    pub configuration_fingerprint: Fingerprint,
}

impl fmt::Debug for NetworkResourceBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkResourceBinding")
            .field("provider_generation", &self.provider_generation)
            .field("resource_generation", &self.resource_generation)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NetworkTopology {
    pub network_id: NetworkId,
    pub allocator_lease_id: NetworkLeaseId,
    pub bridge_set_id: NetworkResourceId,
    pub tap_set_id: NetworkResourceId,
    pub net_vm_role_id: RoleId,
    pub nat_policy_id: NetworkResourceId,
    pub dhcp_policy_id: NetworkResourceId,
    pub nft_policy_id: NetworkResourceId,
    pub netlink_policy_id: NetworkResourceId,
    pub external_attachment_id: Option<NetworkResourceId>,
}

impl fmt::Debug for NetworkTopology {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkTopology")
            .field(
                "semantic_resource_count",
                &(9 + usize::from(self.external_attachment_id.is_some())),
            )
            .field(
                "external_attachment_configured",
                &self.external_attachment_id.is_some(),
            )
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct NetworkEffectContext {
    pub operation: OperationBinding,
    pub scope: AuthorizedProviderScope,
    pub principal: PrincipalRef,
    pub authorization_decision_digest: Fingerprint,
    pub resource: NetworkResourceBinding,
    pub deadline_remaining_ms: u32,
}

impl fmt::Debug for NetworkEffectContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkEffectContext")
            .field("provider_generation", &self.operation.provider_generation)
            .field("resource", &self.resource)
            .field("deadline_remaining_ms", &self.deadline_remaining_ms)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct NetworkEffectRequest {
    pub context: NetworkEffectContext,
    pub topology: NetworkTopology,
    pub plan_id: Option<PlanId>,
    pub handle_id: Option<HandleId>,
}

impl fmt::Debug for NetworkEffectRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkEffectRequest")
            .field("context", &self.context)
            .field("topology", &self.topology)
            .field("has_plan", &self.plan_id.is_some())
            .field("has_handle", &self.handle_id.is_some())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NetworkEffectPlan {
    pub plan_id: PlanId,
    pub resource: NetworkResourceBinding,
}

impl fmt::Debug for NetworkEffectPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkEffectPlan")
            .field("resource", &self.resource)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NetworkEffectHandle {
    pub handle_id: HandleId,
    pub resource: NetworkResourceBinding,
}

impl fmt::Debug for NetworkEffectHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkEffectHandle")
            .field("resource", &self.resource)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkEffectHealth {
    Healthy,
    Degraded,
    Unavailable,
    Failed,
}

#[derive(Clone, PartialEq, Eq)]
pub struct NetworkEffectObservation {
    pub resource: NetworkResourceBinding,
    pub lifecycle: ObservedLifecycleState,
    pub health: NetworkEffectHealth,
}

impl fmt::Debug for NetworkEffectObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkEffectObservation")
            .field("resource", &self.resource)
            .field("lifecycle", &self.lifecycle)
            .field("health", &self.health)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkAdoptionRejection {
    IdentityMismatch,
    ConfigurationMismatch,
    GenerationMismatch,
    OwnerMismatch,
    MissingEvidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkAdoptionState {
    Adopted { lifecycle: ObservedLifecycleState },
    Rejected(NetworkAdoptionRejection),
    Ambiguous,
}

#[derive(Clone, PartialEq, Eq)]
pub struct NetworkAdoptionOutcome {
    pub resource: NetworkResourceBinding,
    pub state: NetworkAdoptionState,
}

impl fmt::Debug for NetworkAdoptionOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkAdoptionOutcome")
            .field("resource", &self.resource)
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NetworkMutationOutcome {
    pub resource: NetworkResourceBinding,
    pub state: MutationState,
}

impl fmt::Debug for NetworkMutationOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NetworkMutationOutcome")
            .field("resource", &self.resource)
            .field("state", &self.state)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkEffectError {
    Unavailable,
    Rejected,
    Ambiguous,
    Cancelled,
}

impl fmt::Display for NetworkEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "network effect unavailable",
            Self::Rejected => "network effect rejected",
            Self::Ambiguous => "network effect completion ambiguous",
            Self::Cancelled => "network effect cancelled",
        })
    }
}

impl Error for NetworkEffectError {}

#[async_trait]
pub trait NetworkEffectPort: Send + Sync {
    fn live_capabilities(&self) -> NetworkLiveCapabilities;

    async fn health(
        &self,
        request: &NetworkEffectRequest,
    ) -> Result<NetworkEffectHealth, NetworkEffectError>;

    async fn plan(
        &self,
        request: &NetworkEffectRequest,
    ) -> Result<NetworkEffectPlan, NetworkEffectError>;

    async fn ensure(
        &self,
        request: &NetworkEffectRequest,
    ) -> Result<NetworkEffectHandle, NetworkEffectError>;

    async fn inspect(
        &self,
        request: &NetworkEffectRequest,
    ) -> Result<NetworkEffectObservation, NetworkEffectError>;

    async fn adopt(
        &self,
        request: &NetworkEffectRequest,
    ) -> Result<NetworkAdoptionOutcome, NetworkEffectError>;

    async fn destroy(
        &self,
        request: &NetworkEffectRequest,
    ) -> Result<NetworkMutationOutcome, NetworkEffectError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkBuildError {
    InvalidDescriptor,
    WrongProviderType,
    WrongImplementation,
    WrongPlacement,
    ScopeMismatch,
    DuplicateResource,
    MissingLiveCapability,
}

impl fmt::Display for NetworkBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDescriptor => "invalid network provider descriptor",
            Self::WrongProviderType => "descriptor is not a network provider",
            Self::WrongImplementation => "network implementation is not local-realm",
            Self::WrongPlacement => "local network provider must run in the owning controller",
            Self::ScopeMismatch => "network binding is outside the configured provider scope",
            Self::DuplicateResource => "network semantic resource identifiers must be distinct",
            Self::MissingLiveCapability => "required live local-network capability is unavailable",
        })
    }
}

impl Error for NetworkBuildError {}

#[derive(Clone)]
pub struct LocalRealmNetworkFactory {
    binding: LocalRealmNetworkBinding,
    effects: Arc<dyn NetworkEffectPort>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for LocalRealmNetworkFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalRealmNetworkFactory")
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl LocalRealmNetworkFactory {
    pub fn new(binding: LocalRealmNetworkBinding, effects: Arc<dyn NetworkEffectPort>) -> Self {
        Self::with_clock(binding, effects, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        binding: LocalRealmNetworkBinding,
        effects: Arc<dyn NetworkEffectPort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Self {
        Self {
            binding,
            effects,
            clock,
        }
    }
}

impl ProviderFactory for LocalRealmNetworkFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != ProviderType::Network
            || descriptor.implementation_id != implementation_id()
        {
            return Err(FactoryError::Rejected);
        }
        let provider = LocalRealmNetworkProvider::with_clock(
            descriptor.clone(),
            self.binding.clone(),
            self.effects.clone(),
            self.clock.clone(),
        )
        .map_err(|error| match error {
            NetworkBuildError::MissingLiveCapability => FactoryError::Unavailable,
            _ => FactoryError::Rejected,
        })?;
        Ok(ProviderInstance::Network(Arc::new(provider)))
    }
}

#[derive(Clone)]
enum CachedResult {
    Plan {
        method: ProviderMethod,
        digest: Fingerprint,
        value: Box<ProviderPlan>,
    },
    Handle {
        method: ProviderMethod,
        digest: Fingerprint,
        value: Box<ProviderHandle>,
    },
    Receipt {
        method: ProviderMethod,
        digest: Fingerprint,
        value: Box<MutationReceipt>,
    },
}

impl CachedResult {
    fn matches(&self, method: ProviderMethod, digest: &Fingerprint) -> bool {
        match self {
            Self::Plan {
                method: cached,
                digest: cached_digest,
                ..
            }
            | Self::Handle {
                method: cached,
                digest: cached_digest,
                ..
            }
            | Self::Receipt {
                method: cached,
                digest: cached_digest,
                ..
            } => *cached == method && cached_digest == digest,
        }
    }
}

#[derive(Default)]
struct ProviderState {
    operations: BTreeMap<IdempotencyKey, CachedResult>,
    handles: BTreeMap<HandleId, ProviderHandle>,
}

pub struct LocalRealmNetworkProvider {
    descriptor: ProviderDescriptor,
    binding: LocalRealmNetworkBinding,
    effects: Arc<dyn NetworkEffectPort>,
    clock: Arc<dyn ProviderClock>,
    state: Mutex<ProviderState>,
}

impl fmt::Debug for LocalRealmNetworkProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalRealmNetworkProvider")
            .field("descriptor", &self.descriptor)
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}

impl LocalRealmNetworkProvider {
    pub fn new(
        descriptor: ProviderDescriptor,
        binding: LocalRealmNetworkBinding,
        effects: Arc<dyn NetworkEffectPort>,
    ) -> Result<Self, NetworkBuildError> {
        Self::with_clock(descriptor, binding, effects, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        descriptor: ProviderDescriptor,
        binding: LocalRealmNetworkBinding,
        effects: Arc<dyn NetworkEffectPort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, NetworkBuildError> {
        descriptor
            .validate()
            .map_err(|_| NetworkBuildError::InvalidDescriptor)?;
        if descriptor.provider_type() != ProviderType::Network {
            return Err(NetworkBuildError::WrongProviderType);
        }
        if descriptor.implementation_id != implementation_id() {
            return Err(NetworkBuildError::WrongImplementation);
        }
        if !matches!(
            descriptor.placement,
            ProviderPlacement::TrustedFirstPartyInProcess { .. }
        ) {
            return Err(NetworkBuildError::WrongPlacement);
        }
        if descriptor.placement.realm_id() != &binding.realm_id {
            return Err(NetworkBuildError::ScopeMismatch);
        }
        if !binding.resources_are_distinct() {
            return Err(NetworkBuildError::DuplicateResource);
        }
        if !effects.live_capabilities().is_complete() {
            return Err(NetworkBuildError::MissingLiveCapability);
        }
        Ok(Self {
            descriptor,
            binding,
            effects,
            clock,
            state: Mutex::new(ProviderState::default()),
        })
    }

    fn now(&self) -> u64 {
        self.clock.now_unix_ms()
    }

    fn resource_binding(&self) -> NetworkResourceBinding {
        NetworkResourceBinding {
            provider_id: self.descriptor.provider_id.clone(),
            realm_id: self.binding.realm_id.clone(),
            owner: self.binding.owner(),
            provider_generation: self.descriptor.registry_generation,
            resource_generation: self.binding.resource_generation,
            configuration_fingerprint: self.descriptor.configuration_schema_fingerprint.clone(),
        }
    }

    fn topology(&self) -> NetworkTopology {
        NetworkTopology {
            network_id: self.binding.network_id.clone(),
            allocator_lease_id: self.binding.allocator_lease_id.clone(),
            bridge_set_id: self.binding.bridge_set_id.clone(),
            tap_set_id: self.binding.tap_set_id.clone(),
            net_vm_role_id: self.binding.net_vm_role_id.clone(),
            nat_policy_id: self.binding.nat_policy_id.clone(),
            dhcp_policy_id: self.binding.dhcp_policy_id.clone(),
            nft_policy_id: self.binding.nft_policy_id.clone(),
            netlink_policy_id: self.binding.netlink_policy_id.clone(),
            external_attachment_id: self.binding.external_attachment_id.clone(),
        }
    }

    fn failure(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        kind: ProviderFailureKind,
        retry: RetryClass,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderFailure {
        ProviderFailure {
            kind,
            retry,
            provider_type: ProviderType::Network,
            binding: OperationBinding {
                operation_id: operation.operation_id.clone(),
                idempotency_key: operation.idempotency_key.clone(),
                request_digest: operation.request_digest.clone(),
                provider_id: self.descriptor.provider_id.clone(),
                provider_generation: self.descriptor.registry_generation,
            },
            correlation_id: operation.correlation_id.clone(),
            occurred_at_unix_ms: self.now(),
            reason,
            remediation,
        }
    }

    fn invalid_request(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
    ) -> ProviderFailure {
        self.failure(
            operation,
            ProviderFailureKind::InvalidRequest,
            RetryClass::Never,
            ProviderHealthReason::CapabilityMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    fn validate_call(
        &self,
        context: &ProviderCallContext<'_>,
        expected: ProviderMethod,
    ) -> ProviderResult<()> {
        if context.cancelled {
            return Err(self.effect_failure(context.operation, NetworkEffectError::Cancelled));
        }
        if context.monotonic_deadline_remaining_ms == 0 {
            return Err(self.deadline_failure(context.operation, false));
        }
        context
            .validate()
            .map_err(|_| self.invalid_request(context.operation))?;
        context
            .operation
            .validate(&self.descriptor, self.now())
            .map_err(|_| self.invalid_request(context.operation))?;
        if context.operation.method != expected
            || context.operation.scope.realm_id() != &self.binding.realm_id
            || context.operation.scope.workload_id().is_some()
        {
            return Err(self.invalid_request(context.operation));
        }
        let ProviderPlacement::TrustedFirstPartyInProcess {
            controller_role, ..
        } = &self.descriptor.placement
        else {
            return Err(self.invalid_request(context.operation));
        };
        if context.peer_role != *controller_role {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::ReEnrollPeer,
            ));
        }
        Ok(())
    }

    fn validate_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        expected: ProviderMethod,
    ) -> ProviderResult<()> {
        self.validate_call(context, expected)?;
        if context.operation != &request.context {
            return Err(self.invalid_request(context.operation));
        }
        request
            .validate_method(&self.descriptor, self.now(), expected)
            .map_err(|_| self.invalid_request(context.operation))?;
        if request.target.realm_id() != &self.binding.realm_id
            || request.target.workload_id().is_some()
        {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::UnauthorizedScope,
                RetryClass::Never,
                ProviderHealthReason::IdentityMismatch,
                ProviderRemediation::ReEnrollPeer,
            ));
        }
        Ok(())
    }

    fn validate_plan(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &ProviderPlan,
    ) -> ProviderResult<()> {
        self.validate_call(context, ProviderMethod::NetworkEnsure)?;
        if plan.schema_version != d2b_contracts::v2_provider::PROVIDER_SCHEMA_VERSION
            || plan.binding.provider_id != self.descriptor.provider_id
            || plan.binding.provider_generation != self.descriptor.registry_generation
            || plan.realm_id != self.binding.realm_id
            || plan.workload_id.is_some()
            || plan.method != ProviderMethod::NetworkPlan
            || plan.configuration_fingerprint != self.descriptor.configuration_schema_fingerprint
            || plan.created_at_unix_ms > self.now()
            || plan.expires_at_unix_ms <= self.now()
            || plan.resources.as_slice() != [PlannedResourceClass::Network]
        {
            return Err(self.invalid_request(context.operation));
        }
        Ok(())
    }

    fn effect_request(
        &self,
        context: &ProviderCallContext<'_>,
        plan_id: Option<PlanId>,
        handle_id: Option<HandleId>,
    ) -> NetworkEffectRequest {
        NetworkEffectRequest {
            context: NetworkEffectContext {
                operation: context.operation.binding(),
                scope: context.operation.scope.clone(),
                principal: context.operation.principal.clone(),
                authorization_decision_digest: context
                    .operation
                    .authorization_decision_digest
                    .clone(),
                resource: self.resource_binding(),
                deadline_remaining_ms: context.monotonic_deadline_remaining_ms,
            },
            topology: self.topology(),
            plan_id,
            handle_id,
        }
    }

    fn effect_failure(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        error: NetworkEffectError,
    ) -> ProviderFailure {
        match error {
            NetworkEffectError::Unavailable => self.failure(
                operation,
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ),
            NetworkEffectError::Rejected => self.failure(
                operation,
                ProviderFailureKind::InvariantViolation,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
            NetworkEffectError::Ambiguous => self.failure(
                operation,
                ProviderFailureKind::AmbiguousMutation,
                RetryClass::AfterObservation,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::InspectProvider,
            ),
            NetworkEffectError::Cancelled => self.failure(
                operation,
                ProviderFailureKind::Cancelled,
                RetryClass::SameOperation,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ),
        }
    }

    fn deadline_failure(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        mutation: bool,
    ) -> ProviderFailure {
        if mutation {
            self.failure(
                operation,
                ProviderFailureKind::AmbiguousMutation,
                RetryClass::AfterObservation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::InspectProvider,
            )
        } else {
            self.failure(
                operation,
                ProviderFailureKind::DeadlineExpired,
                RetryClass::SameOperation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            )
        }
    }

    async fn run_effect<T>(
        &self,
        context: &ProviderCallContext<'_>,
        mutation: bool,
        future: impl std::future::Future<Output = Result<T, NetworkEffectError>>,
    ) -> ProviderResult<T> {
        tokio::time::timeout(
            Duration::from_millis(u64::from(context.monotonic_deadline_remaining_ms)),
            future,
        )
        .await
        .map_err(|_| self.deadline_failure(context.operation, mutation))?
        .map_err(|error| self.effect_failure(context.operation, error))
    }

    fn validate_effect_binding(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        resource: &NetworkResourceBinding,
    ) -> ProviderResult<()> {
        if resource == &self.resource_binding() {
            Ok(())
        } else {
            Err(self.failure(
                operation,
                ProviderFailureKind::InvariantViolation,
                RetryClass::Never,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ))
        }
    }

    fn values(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
    ) -> ProviderResult<ProviderValues> {
        ProviderValues::new(&self.descriptor, self.now())
            .map_err(|_| self.invalid_request(operation))
    }

    fn health_fields(
        health: NetworkEffectHealth,
    ) -> (
        ProviderHealthState,
        ProviderHealthReason,
        ProviderRemediation,
    ) {
        match health {
            NetworkEffectHealth::Healthy => (
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            NetworkEffectHealth::Degraded => (
                ProviderHealthState::Degraded,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::InspectProvider,
            ),
            NetworkEffectHealth::Unavailable => (
                ProviderHealthState::Unavailable,
                ProviderHealthReason::SessionDisconnected,
                ProviderRemediation::RetryBounded,
            ),
            NetworkEffectHealth::Failed => (
                ProviderHealthState::Failed,
                ProviderHealthReason::ConfigurationMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
        }
    }

    fn adoption_fields(
        state: NetworkAdoptionState,
    ) -> (
        ObservedLifecycleState,
        AdoptionState,
        ObservationReason,
        ProviderHealthState,
        ProviderHealthReason,
        ProviderRemediation,
    ) {
        match state {
            NetworkAdoptionState::Adopted { lifecycle } => (
                lifecycle,
                AdoptionState::Adopted,
                ObservationReason::None,
                ProviderHealthState::Healthy,
                ProviderHealthReason::None,
                ProviderRemediation::None,
            ),
            NetworkAdoptionState::Rejected(reason) => {
                let (observation, health, remediation) = match reason {
                    NetworkAdoptionRejection::IdentityMismatch
                    | NetworkAdoptionRejection::OwnerMismatch
                    | NetworkAdoptionRejection::MissingEvidence => (
                        ObservationReason::IdentityMismatch,
                        ProviderHealthReason::IdentityMismatch,
                        ProviderRemediation::ReEnrollPeer,
                    ),
                    NetworkAdoptionRejection::ConfigurationMismatch => (
                        ObservationReason::ConfigurationMismatch,
                        ProviderHealthReason::ConfigurationMismatch,
                        ProviderRemediation::RepairConfiguration,
                    ),
                    NetworkAdoptionRejection::GenerationMismatch => (
                        ObservationReason::GenerationMismatch,
                        ProviderHealthReason::GenerationMismatch,
                        ProviderRemediation::ReplaceGeneration,
                    ),
                };
                (
                    ObservedLifecycleState::Unknown,
                    AdoptionState::Rejected,
                    observation,
                    ProviderHealthState::Failed,
                    health,
                    remediation,
                )
            }
            NetworkAdoptionState::Ambiguous => (
                ObservedLifecycleState::Quarantined,
                AdoptionState::Ambiguous,
                ObservationReason::MultipleCandidates,
                ProviderHealthState::Failed,
                ProviderHealthReason::AdoptionAmbiguous,
                ProviderRemediation::OperatorInteraction,
            ),
        }
    }

    fn target_handle_id(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        target: &ProviderTarget,
        required: bool,
    ) -> ProviderResult<Option<HandleId>> {
        match target {
            ProviderTarget::Handle {
                handle_id,
                handle_generation,
                workload_id: None,
                ..
            } if *handle_generation == self.binding.resource_generation => {
                Ok(Some(handle_id.clone()))
            }
            ProviderTarget::Handle { .. } => Err(self.invalid_request(operation)),
            ProviderTarget::Realm { .. } if !required => Ok(None),
            ProviderTarget::Realm { .. } | ProviderTarget::Workload { .. } => {
                Err(self.invalid_request(operation))
            }
        }
    }

    async fn known_handle(
        &self,
        operation: &d2b_contracts::v2_provider::ProviderOperationContext,
        handle_id: Option<&HandleId>,
    ) -> ProviderResult<Option<ProviderHandle>> {
        let Some(handle_id) = handle_id else {
            return Ok(None);
        };
        let state = self.state.lock().await;
        let Some(handle) = state.handles.get(handle_id).cloned() else {
            return Err(self.invalid_request(operation));
        };
        if handle.owner != self.binding.owner()
            || handle.provider_id != self.descriptor.provider_id
            || handle.provider_generation != self.descriptor.registry_generation
            || handle.realm_id != self.binding.realm_id
            || handle.workload_id.is_some()
            || handle.resource_generation != self.binding.resource_generation
            || handle.configuration_fingerprint != self.descriptor.configuration_schema_fingerprint
        {
            return Err(self.invalid_request(operation));
        }
        Ok(Some(handle))
    }

    async fn health_inner(
        &self,
        context: &ProviderCallContext<'_>,
    ) -> ProviderResult<ProviderHealth> {
        self.validate_call(context, context.operation.method)?;
        let request = self.effect_request(context, None, None);
        let health = self
            .run_effect(context, false, self.effects.health(&request))
            .await?;
        let (state, reason, remediation) = Self::health_fields(health);
        self.values(context.operation)?
            .health(state, reason, remediation)
            .map_err(|_| self.invalid_request(context.operation))
    }

    async fn plan_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderPlan> {
        self.validate_request(context, request, ProviderMethod::NetworkPlan)?;
        if !matches!(request.target, ProviderTarget::Realm { .. }) {
            return Err(self.invalid_request(context.operation));
        }
        let mut state = self.state.lock().await;
        if let Some(cached) = state.operations.get(&context.operation.idempotency_key) {
            if !cached.matches(
                ProviderMethod::NetworkPlan,
                &context.operation.request_digest,
            ) {
                return Err(self.invalid_request(context.operation));
            }
            if let CachedResult::Plan { value, .. } = cached {
                return Ok(value.as_ref().clone());
            }
            return Err(self.invalid_request(context.operation));
        }
        if state.operations.len() >= MAX_TRACKED_OPERATIONS {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::Unavailable,
                RetryClass::AfterObservation,
                ProviderHealthReason::QueuePressure,
                ProviderRemediation::InspectProvider,
            ));
        }
        let effect_request = self.effect_request(context, None, None);
        let effect_plan = self
            .run_effect(context, false, self.effects.plan(&effect_request))
            .await?;
        self.validate_effect_binding(context.operation, &effect_plan.resource)?;
        let resources =
            BoundedVec::<PlannedResourceClass, 0, MAX_PROVIDER_PLAN_RESOURCES>::new(vec![
                PlannedResourceClass::Network,
            ])
            .map_err(|_| self.invalid_request(context.operation))?;
        let expires = self
            .now()
            .saturating_add(PLAN_TTL_MS)
            .min(request.context.expires_at_unix_ms);
        let plan = self
            .values(context.operation)?
            .plan(request, effect_plan.plan_id, expires, resources)
            .map_err(|_| self.invalid_request(context.operation))?;
        state.operations.insert(
            context.operation.idempotency_key.clone(),
            CachedResult::Plan {
                method: ProviderMethod::NetworkPlan,
                digest: context.operation.request_digest.clone(),
                value: Box::new(plan.clone()),
            },
        );
        Ok(plan)
    }

    async fn ensure_inner(
        &self,
        context: &ProviderCallContext<'_>,
        plan: &ProviderPlan,
    ) -> ProviderResult<ProviderHandle> {
        self.validate_plan(context, plan)?;
        let mut state = self.state.lock().await;
        if let Some(cached) = state.operations.get(&context.operation.idempotency_key) {
            if !cached.matches(
                ProviderMethod::NetworkEnsure,
                &context.operation.request_digest,
            ) {
                return Err(self.invalid_request(context.operation));
            }
            if let CachedResult::Handle { value, .. } = cached {
                return Ok(value.as_ref().clone());
            }
            return Err(self.invalid_request(context.operation));
        }
        if state.operations.len() >= MAX_TRACKED_OPERATIONS {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::Unavailable,
                RetryClass::AfterObservation,
                ProviderHealthReason::QueuePressure,
                ProviderRemediation::InspectProvider,
            ));
        }
        let effect_request = self.effect_request(context, Some(plan.plan_id.clone()), None);
        let ensured = self
            .run_effect(context, true, self.effects.ensure(&effect_request))
            .await?;
        self.validate_effect_binding(context.operation, &ensured.resource)?;
        let handle = self
            .values(context.operation)?
            .handle_from_plan(
                plan,
                ensured.handle_id,
                self.binding.owner(),
                self.binding.resource_generation,
                None,
            )
            .map_err(|_| self.invalid_request(context.operation))?;
        state
            .handles
            .insert(handle.handle_id.clone(), handle.clone());
        state.operations.insert(
            context.operation.idempotency_key.clone(),
            CachedResult::Handle {
                method: ProviderMethod::NetworkEnsure,
                digest: context.operation.request_digest.clone(),
                value: Box::new(handle.clone()),
            },
        );
        Ok(handle)
    }

    async fn inspect_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<ProviderObservation> {
        self.validate_request(context, request, ProviderMethod::NetworkInspect)?;
        let handle_id = self.target_handle_id(context.operation, &request.target, false)?;
        let handle = self
            .known_handle(context.operation, handle_id.as_ref())
            .await?;
        let effect_request = self.effect_request(context, None, handle_id);
        let observed = self
            .run_effect(context, false, self.effects.inspect(&effect_request))
            .await?;
        self.validate_effect_binding(context.operation, &observed.resource)?;
        let (health_state, health_reason, remediation) = Self::health_fields(observed.health);
        self.values(context.operation)?
            .observation(
                context.operation,
                handle.as_ref(),
                observed.lifecycle,
                AdoptionState::NotAttempted,
                ObservationReason::None,
                health_state,
                health_reason,
                remediation,
            )
            .map_err(|_| self.invalid_request(context.operation))
    }

    async fn adopt_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &AdoptionRequest,
    ) -> ProviderResult<ProviderObservation> {
        self.validate_call(context, ProviderMethod::NetworkAdopt)?;
        if context.operation != &request.context {
            return Err(self.invalid_request(context.operation));
        }
        request
            .validate(&self.descriptor, self.now())
            .map_err(|_| {
                self.failure(
                    context.operation,
                    ProviderFailureKind::AdoptionRejected,
                    RetryClass::AfterObservation,
                    ProviderHealthReason::GenerationMismatch,
                    ProviderRemediation::ReplaceGeneration,
                )
            })?;
        if request.handle.realm_id != self.binding.realm_id
            || request.handle.workload_id.is_some()
            || request.expected_owner != self.binding.owner()
            || request.expected_configuration_fingerprint
                != self.descriptor.configuration_schema_fingerprint
            || request.expected_resource_generation != self.binding.resource_generation
        {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::AdoptionRejected,
                RetryClass::AfterObservation,
                ProviderHealthReason::GenerationMismatch,
                ProviderRemediation::ReplaceGeneration,
            ));
        }
        let effect_request =
            self.effect_request(context, None, Some(request.handle.handle_id.clone()));
        let outcome = self
            .run_effect(context, false, self.effects.adopt(&effect_request))
            .await?;
        self.validate_effect_binding(context.operation, &outcome.resource)?;
        let (lifecycle, adoption, reason, health_state, health_reason, remediation) =
            Self::adoption_fields(outcome.state);
        if adoption == AdoptionState::Adopted {
            self.state
                .lock()
                .await
                .handles
                .insert(request.handle.handle_id.clone(), request.handle.clone());
        }
        self.values(context.operation)?
            .observation(
                context.operation,
                Some(&request.handle),
                lifecycle,
                adoption,
                reason,
                health_state,
                health_reason,
                remediation,
            )
            .map_err(|_| self.invalid_request(context.operation))
    }

    async fn release_inner(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
    ) -> ProviderResult<MutationReceipt> {
        self.validate_request(context, request, ProviderMethod::NetworkRelease)?;
        let handle_id = self
            .target_handle_id(context.operation, &request.target, true)?
            .ok_or_else(|| self.invalid_request(context.operation))?;
        let mut state = self.state.lock().await;
        if let Some(cached) = state.operations.get(&context.operation.idempotency_key) {
            if !cached.matches(
                ProviderMethod::NetworkRelease,
                &context.operation.request_digest,
            ) {
                return Err(self.invalid_request(context.operation));
            }
            if let CachedResult::Receipt { value, .. } = cached {
                return Ok(value.as_ref().clone());
            }
            return Err(self.invalid_request(context.operation));
        }
        let Some(handle) = state.handles.get(&handle_id) else {
            return Err(self.invalid_request(context.operation));
        };
        if handle.resource_generation != self.binding.resource_generation {
            return Err(self.invalid_request(context.operation));
        }
        if state.operations.len() >= MAX_TRACKED_OPERATIONS {
            return Err(self.failure(
                context.operation,
                ProviderFailureKind::Unavailable,
                RetryClass::AfterObservation,
                ProviderHealthReason::QueuePressure,
                ProviderRemediation::InspectProvider,
            ));
        }
        let effect_request = self.effect_request(context, None, Some(handle_id));
        let outcome = self
            .run_effect(context, true, self.effects.destroy(&effect_request))
            .await?;
        self.validate_effect_binding(context.operation, &outcome.resource)?;
        let receipt = self
            .values(context.operation)?
            .receipt(context.operation, outcome.state)
            .map_err(|_| self.invalid_request(context.operation))?;
        state.operations.insert(
            context.operation.idempotency_key.clone(),
            CachedResult::Receipt {
                method: ProviderMethod::NetworkRelease,
                digest: context.operation.request_digest.clone(),
                value: Box::new(receipt.clone()),
            },
        );
        Ok(receipt)
    }
}

impl Provider for LocalRealmNetworkProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(async move { self.health_inner(context).await })
    }
}

impl NetworkProvider for LocalRealmNetworkProvider {
    fn capabilities(&self) -> ProviderCapabilitySet {
        self.descriptor.capabilities.clone()
    }

    fn plan<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderPlan> {
        Box::pin(async move { self.plan_inner(context, request).await })
    }

    fn ensure<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        plan: &'a ProviderPlan,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(async move { self.ensure_inner(context, plan).await })
    }

    fn inspect<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(async move { self.inspect_inner(context, request).await })
    }

    fn adopt<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a AdoptionRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(async move { self.adopt_inner(context, request).await })
    }

    fn release<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, MutationReceipt> {
        Box::pin(async move { self.release_inner(context, request).await })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Mutex as StdMutex,
            atomic::{AtomicU64, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use d2b_contracts::{
        v2_component_session::{EndpointRole, ServicePackage},
        v2_provider::{
            PROVIDER_SCHEMA_VERSION, ProviderApiVersion, ProviderAuthority, ProviderCapability,
            ProviderOperationContext, ProviderOperationInput,
        },
    };

    use super::*;

    const NOW: u64 = 1_700_000_000_000;

    #[derive(Debug)]
    struct TestClock;

    impl ProviderClock for TestClock {
        fn now_unix_ms(&self) -> u64 {
            NOW
        }
    }

    #[derive(Default)]
    struct FakeEffects {
        calls: AtomicUsize,
        plan_calls: AtomicUsize,
        ensure_calls: AtomicUsize,
        adopt_calls: AtomicUsize,
        destroy_calls: AtomicUsize,
        delay_ms: AtomicU64,
        last: StdMutex<Option<NetworkEffectRequest>>,
    }

    impl FakeEffects {
        fn record(&self, request: &NetworkEffectRequest) {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last.lock().expect("last request lock") = Some(request.clone());
        }

        async fn delay(&self) {
            let delay = self.delay_ms.load(Ordering::SeqCst);
            if delay > 0 {
                tokio::time::sleep(Duration::from_millis(delay)).await;
            }
        }
    }

    #[async_trait]
    impl NetworkEffectPort for FakeEffects {
        fn live_capabilities(&self) -> NetworkLiveCapabilities {
            NetworkLiveCapabilities::REQUIRED
        }

        async fn health(
            &self,
            request: &NetworkEffectRequest,
        ) -> Result<NetworkEffectHealth, NetworkEffectError> {
            self.record(request);
            Ok(NetworkEffectHealth::Healthy)
        }

        async fn plan(
            &self,
            request: &NetworkEffectRequest,
        ) -> Result<NetworkEffectPlan, NetworkEffectError> {
            self.record(request);
            self.plan_calls.fetch_add(1, Ordering::SeqCst);
            Ok(NetworkEffectPlan {
                plan_id: PlanId::parse("network-plan").expect("plan"),
                resource: request.context.resource.clone(),
            })
        }

        async fn ensure(
            &self,
            request: &NetworkEffectRequest,
        ) -> Result<NetworkEffectHandle, NetworkEffectError> {
            self.record(request);
            self.ensure_calls.fetch_add(1, Ordering::SeqCst);
            self.delay().await;
            Ok(NetworkEffectHandle {
                handle_id: HandleId::parse("network-handle").expect("handle"),
                resource: request.context.resource.clone(),
            })
        }

        async fn inspect(
            &self,
            request: &NetworkEffectRequest,
        ) -> Result<NetworkEffectObservation, NetworkEffectError> {
            self.record(request);
            Ok(NetworkEffectObservation {
                resource: request.context.resource.clone(),
                lifecycle: ObservedLifecycleState::Ready,
                health: NetworkEffectHealth::Healthy,
            })
        }

        async fn adopt(
            &self,
            request: &NetworkEffectRequest,
        ) -> Result<NetworkAdoptionOutcome, NetworkEffectError> {
            self.record(request);
            self.adopt_calls.fetch_add(1, Ordering::SeqCst);
            Ok(NetworkAdoptionOutcome {
                resource: request.context.resource.clone(),
                state: NetworkAdoptionState::Adopted {
                    lifecycle: ObservedLifecycleState::Ready,
                },
            })
        }

        async fn destroy(
            &self,
            request: &NetworkEffectRequest,
        ) -> Result<NetworkMutationOutcome, NetworkEffectError> {
            self.record(request);
            self.destroy_calls.fetch_add(1, Ordering::SeqCst);
            Ok(NetworkMutationOutcome {
                resource: request.context.resource.clone(),
                state: MutationState::Applied,
            })
        }
    }

    fn short_id(letter: char) -> String {
        format!("{}a", letter.to_string().repeat(19))
    }

    fn fingerprint(value: u8) -> Fingerprint {
        Fingerprint::parse(format!("{value:064x}")).expect("fingerprint")
    }

    fn descriptor() -> ProviderDescriptor {
        ProviderDescriptor {
            schema_version: PROVIDER_SCHEMA_VERSION,
            provider_id: ProviderId::parse(short_id('b')).expect("provider"),
            authority: ProviderAuthority::Network,
            implementation_id: implementation_id(),
            api_version: ProviderApiVersion::V2,
            capabilities: ProviderCapabilitySet::new(vec![
                ProviderCapability(ProviderMethod::NetworkPlan),
                ProviderCapability(ProviderMethod::NetworkEnsure),
                ProviderCapability(ProviderMethod::NetworkInspect),
                ProviderCapability(ProviderMethod::NetworkAdopt),
                ProviderCapability(ProviderMethod::NetworkRelease),
            ])
            .expect("capabilities"),
            configuration_schema_fingerprint: fingerprint(1),
            configured_scope_digest: fingerprint(2),
            registry_generation: Generation::new(1).expect("generation"),
            placement: ProviderPlacement::TrustedFirstPartyInProcess {
                realm_id: RealmId::parse(short_id('a')).expect("realm"),
                controller_role: EndpointRole::RealmController,
            },
        }
    }

    fn binding() -> LocalRealmNetworkBinding {
        LocalRealmNetworkBinding {
            realm_id: RealmId::parse(short_id('a')).expect("realm"),
            network_id: NetworkId::parse("realm-network").expect("network"),
            allocator_lease_id: NetworkLeaseId::parse("network-lease").expect("lease"),
            bridge_set_id: NetworkResourceId::parse("bridge-set").expect("resource"),
            tap_set_id: NetworkResourceId::parse("tap-set").expect("resource"),
            net_vm_role_id: RoleId::parse(short_id('d')).expect("role"),
            nat_policy_id: NetworkResourceId::parse("nat-policy").expect("resource"),
            dhcp_policy_id: NetworkResourceId::parse("dhcp-policy").expect("resource"),
            nft_policy_id: NetworkResourceId::parse("nft-policy").expect("resource"),
            netlink_policy_id: NetworkResourceId::parse("netlink-policy").expect("resource"),
            external_attachment_id: Some(
                NetworkResourceId::parse("external-attachment").expect("resource"),
            ),
            resource_generation: Generation::new(1).expect("generation"),
        }
    }

    fn provider(effects: Arc<FakeEffects>) -> LocalRealmNetworkProvider {
        LocalRealmNetworkProvider::with_clock(descriptor(), binding(), effects, Arc::new(TestClock))
            .expect("provider")
    }

    fn operation(method: ProviderMethod, id: &str) -> ProviderOperationContext {
        let descriptor = descriptor();
        ProviderOperationContext {
            schema_version: PROVIDER_SCHEMA_VERSION,
            operation_id: d2b_contracts::v2_provider::OperationId::parse(id).expect("operation"),
            idempotency_key: IdempotencyKey::parse(format!("{id}-idempotency"))
                .expect("idempotency"),
            request_digest: fingerprint(match method {
                ProviderMethod::NetworkPlan => 3,
                ProviderMethod::NetworkEnsure => 4,
                ProviderMethod::NetworkInspect => 5,
                ProviderMethod::NetworkAdopt => 6,
                ProviderMethod::NetworkRelease => 7,
                _ => 8,
            }),
            scope: AuthorizedProviderScope::Realm {
                realm_id: binding().realm_id,
            },
            principal: PrincipalRef::parse("network-principal").expect("principal"),
            provider_id: descriptor.provider_id,
            provider_type: ProviderType::Network,
            provider_generation: Generation::new(1).expect("generation"),
            capability: ProviderCapability(method),
            method,
            policy_epoch: Generation::new(1).expect("generation"),
            authorization_decision_digest: fingerprint(9),
            issued_at_unix_ms: NOW - 1_000,
            expires_at_unix_ms: NOW + 60_000,
            correlation_id: d2b_contracts::v2_provider::CorrelationId::parse("network-correlation")
                .expect("correlation"),
            trace_id: fingerprint(10),
        }
    }

    fn request(method: ProviderMethod, input: ProviderOperationInput) -> ProviderOperationRequest {
        ProviderOperationRequest {
            context: operation(method, "network-operation"),
            target: ProviderTarget::Realm {
                realm_id: binding().realm_id,
            },
            expected_configuration_fingerprint: fingerprint(1),
            input,
        }
    }

    fn call_context<'a>(
        operation: &'a ProviderOperationContext,
        deadline_ms: u32,
        cancelled: bool,
    ) -> ProviderCallContext<'a> {
        ProviderCallContext {
            operation,
            peer_role: EndpointRole::RealmController,
            service: ServicePackage::ProviderV2,
            monotonic_deadline_remaining_ms: deadline_ms,
            cancelled,
        }
    }

    async fn planned_and_ensured(
        provider: &LocalRealmNetworkProvider,
    ) -> (ProviderPlan, ProviderHandle) {
        let request = request(ProviderMethod::NetworkPlan, ProviderOperationInput::NoInput);
        let plan_context = call_context(&request.context, 30_000, false);
        let plan = provider.plan(&plan_context, &request).await.expect("plan");
        let ensure_operation = operation(ProviderMethod::NetworkEnsure, "network-ensure");
        let ensure_context = call_context(&ensure_operation, 30_000, false);
        let handle = provider
            .ensure(&ensure_context, &plan)
            .await
            .expect("ensure");
        (plan, handle)
    }

    #[test]
    fn advertises_canonical_live_capabilities() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects);
        assert_eq!(provider.capabilities(), descriptor().capabilities);
        assert!(NetworkLiveCapabilities::REQUIRED.is_complete());
    }

    #[test]
    fn factory_registers_and_rejects_wrong_descriptor_axis() {
        let effects = Arc::new(FakeEffects::default());
        let factory = Arc::new(LocalRealmNetworkFactory::with_clock(
            binding(),
            effects,
            Arc::new(TestClock),
        ));
        let descriptor = descriptor();
        let key = provider_factory_key();
        assert_eq!(key.provider_type, ProviderType::Network);
        assert_eq!(key.implementation_id, implementation_id());

        let mut wrong_type = descriptor.clone();
        wrong_type.authority = ProviderAuthority::Storage;
        assert!(matches!(
            factory.construct(&wrong_type),
            Err(FactoryError::Rejected)
        ));

        let mut wrong_implementation = descriptor.clone();
        wrong_implementation.implementation_id =
            ImplementationId::parse("other-network").expect("implementation");
        assert!(matches!(
            factory.construct(&wrong_implementation),
            Err(FactoryError::Rejected)
        ));

        let mut builder = d2b_provider::ProviderRegistryBuilder::new(
            descriptor.registry_generation,
            fingerprint(11),
            NOW,
        );
        builder
            .register_factory(key, factory)
            .expect("register factory")
            .register_instance(descriptor.clone())
            .expect("register provider");
        let registry = builder.finish().expect("registry");
        assert_eq!(
            registry
                .instance(&descriptor.provider_id)
                .expect("instance")
                .descriptor(),
            descriptor
        );
    }

    #[tokio::test]
    async fn plan_and_ensure_preserve_nix_authority_and_idempotency() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let request = request(ProviderMethod::NetworkPlan, ProviderOperationInput::NoInput);
        let context = call_context(&request.context, 30_000, false);
        let first = provider.plan(&context, &request).await.expect("plan");
        let second = provider
            .plan(&context, &request)
            .await
            .expect("repeat plan");
        assert_eq!(first, second);
        assert_eq!(effects.plan_calls.load(Ordering::SeqCst), 1);

        let ensure_operation = operation(ProviderMethod::NetworkEnsure, "network-ensure");
        let ensure_context = call_context(&ensure_operation, 30_000, false);
        let first_handle = provider
            .ensure(&ensure_context, &first)
            .await
            .expect("ensure");
        let second_handle = provider
            .ensure(&ensure_context, &first)
            .await
            .expect("repeat ensure");
        assert_eq!(first_handle, second_handle);
        assert_eq!(effects.ensure_calls.load(Ordering::SeqCst), 1);

        let captured = effects
            .last
            .lock()
            .expect("last request lock")
            .clone()
            .expect("request");
        assert_eq!(captured.topology.network_id, binding().network_id);
        assert_eq!(
            captured.topology.allocator_lease_id,
            binding().allocator_lease_id
        );
        assert_eq!(captured.topology.nft_policy_id, binding().nft_policy_id);
    }

    #[tokio::test]
    async fn wrong_input_and_cancellation_have_zero_effect() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let wrong = request(
            ProviderMethod::NetworkPlan,
            ProviderOperationInput::StorageSnapshot {
                snapshot_id: d2b_contracts::v2_provider::StorageSnapshotId::parse("snapshot")
                    .expect("snapshot"),
            },
        );
        let wrong_context = call_context(&wrong.context, 30_000, false);
        assert!(provider.plan(&wrong_context, &wrong).await.is_err());

        let cancelled = request(
            ProviderMethod::NetworkInspect,
            ProviderOperationInput::NoInput,
        );
        let cancelled_context = call_context(&cancelled.context, 30_000, true);
        let cancelled_error = provider
            .inspect(&cancelled_context, &cancelled)
            .await
            .expect_err("cancelled");
        assert_eq!(cancelled_error.kind, ProviderFailureKind::Cancelled);
        assert_eq!(effects.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn stale_adoption_is_rejected_without_effect() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let (_, handle) = planned_and_ensured(&provider).await;
        let adoption = AdoptionRequest {
            context: operation(ProviderMethod::NetworkAdopt, "network-adopt"),
            handle: handle.clone(),
            expected_owner: handle.owner.clone(),
            expected_configuration_fingerprint: handle.configuration_fingerprint.clone(),
            expected_resource_generation: Generation::new(2).expect("stale"),
        };
        let context = call_context(&adoption.context, 30_000, false);
        let before = effects.calls.load(Ordering::SeqCst);
        let error = provider
            .adopt(&context, &adoption)
            .await
            .expect_err("stale adoption");
        assert_eq!(error.kind, ProviderFailureKind::AdoptionRejected);
        assert_eq!(effects.calls.load(Ordering::SeqCst), before);
        assert_eq!(effects.adopt_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn inspect_adopt_and_release_use_the_bound_handle() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let (_, handle) = planned_and_ensured(&provider).await;

        let mut inspect = request(
            ProviderMethod::NetworkInspect,
            ProviderOperationInput::NoInput,
        );
        inspect.context = operation(ProviderMethod::NetworkInspect, "network-inspect");
        inspect.target = ProviderTarget::Handle {
            realm_id: binding().realm_id,
            workload_id: None,
            handle_id: handle.handle_id.clone(),
            handle_generation: handle.resource_generation,
        };
        let inspect_context = call_context(&inspect.context, 30_000, false);
        let observation = provider
            .inspect(&inspect_context, &inspect)
            .await
            .expect("inspect");
        assert_eq!(observation.handle_id.as_ref(), Some(&handle.handle_id));

        let adoption = AdoptionRequest {
            context: operation(ProviderMethod::NetworkAdopt, "network-adopt-valid"),
            handle: handle.clone(),
            expected_owner: handle.owner.clone(),
            expected_configuration_fingerprint: handle.configuration_fingerprint.clone(),
            expected_resource_generation: handle.resource_generation,
        };
        let adopt_context = call_context(&adoption.context, 30_000, false);
        let adopted = provider
            .adopt(&adopt_context, &adoption)
            .await
            .expect("adopt");
        assert_eq!(adopted.adoption, AdoptionState::Adopted);

        let mut release = request(
            ProviderMethod::NetworkRelease,
            ProviderOperationInput::NoInput,
        );
        release.context = operation(ProviderMethod::NetworkRelease, "network-release");
        release.target = ProviderTarget::Handle {
            realm_id: binding().realm_id,
            workload_id: None,
            handle_id: handle.handle_id.clone(),
            handle_generation: handle.resource_generation,
        };
        let release_context = call_context(&release.context, 30_000, false);
        let first = provider
            .release(&release_context, &release)
            .await
            .expect("release");
        let second = provider
            .release(&release_context, &release)
            .await
            .expect("repeat release");
        assert_eq!(first, second);
        assert_eq!(effects.adopt_calls.load(Ordering::SeqCst), 1);
        assert_eq!(effects.destroy_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn ensure_deadline_is_ambiguous_and_bounded() {
        let effects = Arc::new(FakeEffects::default());
        let provider = provider(effects.clone());
        let request = request(ProviderMethod::NetworkPlan, ProviderOperationInput::NoInput);
        let plan_context = call_context(&request.context, 30_000, false);
        let plan = provider.plan(&plan_context, &request).await.expect("plan");
        effects.delay_ms.store(25, Ordering::SeqCst);
        let ensure_operation = operation(ProviderMethod::NetworkEnsure, "network-ensure");
        let ensure_context = call_context(&ensure_operation, 1, false);
        let error = provider
            .ensure(&ensure_context, &plan)
            .await
            .expect_err("deadline");
        assert_eq!(error.kind, ProviderFailureKind::AmbiguousMutation);
        assert_eq!(effects.ensure_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn boundary_cannot_encode_cidrs_ifnames_rules_paths_or_acls() {
        for forbidden in [
            "10.20.0.0/24",
            "ip saddr 10.20.0.0/24 accept",
            "/run/d2b/net",
            "u:alice:rwx",
        ] {
            assert!(NetworkResourceId::parse(forbidden).is_err(), "{forbidden}");
        }
        let rendered = format!(
            "{:?} {:?} {}",
            binding(),
            NetworkEffectError::Rejected,
            NetworkBuildError::ScopeMismatch
        );
        for forbidden in ["10.20.", "br-work-lan", "/run/", "u:alice", "ip saddr"] {
            assert!(!rendered.contains(forbidden), "{rendered}");
        }
    }
}
