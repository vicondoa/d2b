//! Zone-scoped authorization over the native Role and RoleBinding evaluator.

use std::sync::{Mutex, MutexGuard};

use d2b_contracts_resource::v3::{ControllerGeneration, ZoneId};
use d2b_contracts_resource::v3::identity::{
    AuthenticatedSubjectContext,
    EvidenceClass,
    Locality,
};
use d2b_resource_api::authz::{
    AuthorizationDenial, AuthorizationPolicyError, AuthorizationState, NativeAuthorizer, PolicySet,
    SessionVerb,
};
use d2b_core_controller::controller_assignment::{
    AssignmentError, AssignmentVerb, ControllerAssignmentRegistry,
};
use d2b_session::{
    OperationMember, SessionAuthorizationRequest, SessionError, SessionOperation,
    contract::{AuthorizationLease, SessionErrorCode},
};

use crate::{
    registry::{RouteKey, RouteTarget},
    router::ResourceCall,
};

struct AuthorizationRuntime {
    native: std::sync::Arc<NativeAuthorizer>,
    state: AuthorizationState,
}

/// Single-owner native authorizer and trusted policy state for one bus.
pub struct BusAuthorizer {
    runtime: Mutex<AuthorizationRuntime>,
    assignments: Option<std::sync::Arc<Mutex<ControllerAssignmentRegistry>>>,
}

impl BusAuthorizer {
    /// Consume the evaluator and trusted policy state into one private owner.
    pub fn new(
        native: NativeAuthorizer,
        state: AuthorizationState,
    ) -> Result<Self, AuthorizationError> {
        Self::from_shared(std::sync::Arc::new(native), state)
    }

    /// Construct an authorizer over a native evaluator already shared with a
    /// generated Resource API service.
    pub fn from_shared(
        native: std::sync::Arc<NativeAuthorizer>,
        state: AuthorizationState,
    ) -> Result<Self, AuthorizationError> {
        if state.snapshot.policy_revision == 0 {
            return Err(AuthorizationError::PolicyRevisionZero);
        }
        Ok(Self {
            runtime: Mutex::new(AuthorizationRuntime { native, state }),
            assignments: None,
        })
    }

    /// Bind the Core-owned assignment registry used to fence controller
    /// watches and mutations at the existing bus admission seam.
    pub fn with_assignment_registry(
        mut self,
        assignments: std::sync::Arc<Mutex<ControllerAssignmentRegistry>>,
    ) -> Self {
        self.assignments = Some(assignments);
        self
    }

    /// Borrow the single native authorizer shared with the Resource API.
    ///
    /// The bus and generated resource handlers must evaluate the same policy
    /// instance and store-bound mutation authority.  Returning the existing
    /// `Arc` prevents the daemon from accidentally constructing a parallel
    /// authority for one Zone.
    pub fn native_authorizer(&self) -> std::sync::Arc<NativeAuthorizer> {
        std::sync::Arc::clone(&self.lock().native)
    }

    pub(crate) fn controller_generation(&self) -> Option<ControllerGeneration> {
        self.lock().state.snapshot.controller_generation
    }

    /// Install a new durable policy and its exact trusted revision state.
    pub fn replace_policy(
        &self,
        policy: PolicySet,
        state: AuthorizationState,
    ) -> Result<(), AuthorizationError> {
        let mut runtime = self.lock();
        if state.zone_policy_revision < runtime.state.zone_policy_revision {
            return Err(AuthorizationError::Native(
                d2b_resource_api::authz::AuthorizationDenial::PolicyRevisionChanged,
            ));
        }
        runtime.native.replace_policy(policy, &state)?;
        runtime.state = state;
        Ok(())
    }

    /// Mark policy unavailable and invalidate cached decisions.
    pub fn mark_policy_unavailable(&self) {
        self.lock().native.mark_policy_unavailable();
    }

    pub(crate) fn authorize_connect(
        &self,
        context: &AuthenticatedSubjectContext,
        zone: &ZoneId,
    ) -> Result<(), AuthorizationError> {
        ensure_zone(context, zone)?;
        let runtime = self.lock();
        let capabilities = runtime
            .native
            .positive_capabilities(context, zone, &runtime.state)?;
        require(&capabilities.session_verbs, SessionVerb::Connect)
    }

    pub(crate) fn authorize_dispatch(
        &self,
        context: &AuthenticatedSubjectContext,
        route: &RouteKey,
        resource_call: Option<&ResourceCall>,
        stream: bool,
    ) -> Result<(), AuthorizationError> {
        ensure_zone(context, route.zone())?;
        ensure_session_binding(context, route)?;
        let relay = relay_origin(context)?;
        let runtime = self.lock();
        let capabilities =
            runtime
                .native
                .positive_capabilities(context, route.zone(), &runtime.state)?;
        require(&capabilities.session_verbs, SessionVerb::Connect)?;

        let diagnostic = diagnostic_verb(route)?;
        let target_verb = if let Some(verb) = diagnostic {
            if stream || resource_call.is_some() {
                return Err(AuthorizationError::DiagnosticBinding);
            }
            verb
        } else if stream {
            SessionVerb::OpenStream
        } else {
            SessionVerb::Invoke
        };
        require(&capabilities.session_verbs, target_verb)?;
        if relay {
            require(&capabilities.session_verbs, SessionVerb::Relay)
                .map_err(|_| AuthorizationError::RelayGrantMissing)?;
        }

        if let Some(call) = resource_call {
            self.authorize_assignment(context, call)?;
            let request = call.authorization_request(route.zone().clone())?;
            runtime
                .native
                .authorize(context, &request, &runtime.state)?;
        }
        Ok(())
    }

    fn authorize_assignment(
        &self,
        context: &AuthenticatedSubjectContext,
        call: &ResourceCall,
    ) -> Result<(), AuthorizationError> {
        let Some(assignment) = call.assignment() else {
            return Ok(());
        };
        if context.reconnect_generation() != assignment.session_generation()
            || context.provider_generation() != Some(assignment.provider_generation())
            || context.controller_generation() != Some(assignment.controller_generation())
        {
            return Err(AuthorizationError::Assignment(
                AssignmentError::SessionBindingMismatch,
            ));
        }
        if let Some(target) = assignment.target().execution_ref()
            && context.execution_ref() != Some(target)
            && context.subject_ref() != target
        {
            return Err(AuthorizationError::Assignment(
                AssignmentError::TargetMismatch,
            ));
        }
        let verb = match call {
            ResourceCall::List(_) => AssignmentVerb::List,
            ResourceCall::Watch(_) => AssignmentVerb::Watch,
            ResourceCall::ScopedCommitBatch { .. } => AssignmentVerb::CommitBatch,
            _ => return Err(AuthorizationError::Assignment(AssignmentError::VerbNotAllowed)),
        };
        if let Some(registry) = &self.assignments {
            registry
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .validate_scope(assignment, verb)
                .map_err(AuthorizationError::Assignment)?;
        }
        Ok(())
    }

    pub(crate) fn authorize_cancel(
        &self,
        context: &AuthenticatedSubjectContext,
        route: &RouteKey,
    ) -> Result<(), AuthorizationError> {
        ensure_zone(context, route.zone())?;
        ensure_session_binding(context, route)?;
        let relay = relay_origin(context)?;
        let runtime = self.lock();
        let capabilities =
            runtime
                .native
                .positive_capabilities(context, route.zone(), &runtime.state)?;
        require(&capabilities.session_verbs, SessionVerb::Connect)?;
        require(&capabilities.session_verbs, SessionVerb::Cancel)?;
        if relay {
            require(&capabilities.session_verbs, SessionVerb::Relay)
                .map_err(|_| AuthorizationError::RelayGrantMissing)?;
        }
        Ok(())
    }

    pub(crate) fn authenticate_session(
        &self,
        context: &AuthenticatedSubjectContext,
        zone: &ZoneId,
        now_tick: u64,
    ) -> d2b_session::Result<AuthorizationLease> {
        ensure_zone(context, zone).map_err(Self::session_denied)?;
        let mut runtime = self.lock();
        runtime.state.now_tick = now_tick;
        let capabilities = runtime
            .native
            .positive_capabilities(context, zone, &runtime.state)
            .map_err(Self::session_denied)?;
        require(&capabilities.session_verbs, SessionVerb::Connect).map_err(Self::session_denied)?;
        Self::session_lease(Self::effective_policy_revision(&runtime.state), now_tick)
    }

    pub(crate) fn authorize_session(
        &self,
        context: &AuthenticatedSubjectContext,
        request: &SessionAuthorizationRequest,
        previous_lease: AuthorizationLease,
        now_tick: u64,
    ) -> d2b_session::Result<AuthorizationLease> {
        let mut runtime = self.lock();
        if previous_lease.policy_revision() != Self::effective_policy_revision(&runtime.state) {
            return Err(SessionError::new(SessionErrorCode::PolicyDenied));
        }
        runtime.state.now_tick = now_tick;
        let zone = ZoneId::parse(context.zone_ref().name().as_str())
            .map_err(|_| SessionError::new(SessionErrorCode::PolicyDenied))?;
        let capabilities = runtime
            .native
            .positive_capabilities(context, &zone, &runtime.state)
            .map_err(Self::session_denied)?;
        require(&capabilities.session_verbs, SessionVerb::Connect).map_err(Self::session_denied)?;
        require(&capabilities.session_verbs, request.verb()).map_err(Self::session_denied)?;
        Self::session_lease(Self::effective_policy_revision(&runtime.state), now_tick)
    }

    fn effective_policy_revision(state: &AuthorizationState) -> u64 {
        state
            .zone_policy_revision
            .get()
            .max(state.snapshot.policy_revision)
    }

    fn lock(&self) -> MutexGuard<'_, AuthorizationRuntime> {
        self.runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn session_lease(
        policy_revision: u64,
        now_tick: u64,
    ) -> d2b_session::Result<AuthorizationLease> {
        AuthorizationLease::new(policy_revision, now_tick.saturating_add(10_000))
            .map_err(SessionError::from)
    }

    fn session_denied<T>(_error: T) -> SessionError {
        SessionError::new(SessionErrorCode::PolicyDenied)
    }
}

impl core::fmt::Debug for BusAuthorizer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("BusAuthorizer(<redacted>)")
    }
}

fn ensure_zone(
    context: &AuthenticatedSubjectContext,
    zone: &ZoneId,
) -> Result<(), AuthorizationError> {
    if context.zone_ref().resource_type().as_str() != "Zone"
        || context.zone_ref().name().as_str() != zone.as_str()
    {
        return Err(AuthorizationError::ZoneMismatch);
    }
    Ok(())
}

fn ensure_session_binding(
    context: &AuthenticatedSubjectContext,
    route: &RouteKey,
) -> Result<(), AuthorizationError> {
    if context.service() != route.service()
        || context.schema_fingerprint() != route.schema()
        || context.reconnect_generation() != route.generations().session()
        || context.provider_generation() != route.generations().provider()
        || context.controller_generation() != route.generations().controller()
        || matches!(
            route.target(),
            RouteTarget::Provider(provider) if context.provider_ref() != Some(provider)
        )
    {
        return Err(AuthorizationError::SessionBindingMismatch);
    }
    Ok(())
}

fn relay_origin(context: &AuthenticatedSubjectContext) -> Result<bool, AuthorizationError> {
    match context.transport_binding().locality() {
        Locality::Local => Ok(false),
        Locality::AdjacentZone
            if context.evidence_class() == EvidenceClass::EnrolledKk
                && matches!(
                    context.subject_ref().resource_type().as_str(),
                    "Zone" | "ZoneLink"
                ) =>
        {
            Ok(true)
        }
        Locality::AdjacentZone | Locality::Remote => Err(AuthorizationError::RelayOriginInvalid),
    }
}

fn diagnostic_verb(route: &RouteKey) -> Result<Option<SessionVerb>, AuthorizationError> {
    let member = if route.member().is_method() {
        OperationMember::method(route.member().as_str())
    } else {
        OperationMember::stream(route.member().as_str())
    }
    .map_err(|_| AuthorizationError::DiagnosticBinding)?;
    let operation = SessionOperation::new(route.service().clone(), member)
        .map_err(|_| AuthorizationError::DiagnosticBinding)?;
    let verb = operation.required_verb(SessionVerb::Invoke);
    if matches!(verb, SessionVerb::AuditExport | SessionVerb::SupportBundle) {
        Ok(Some(verb))
    } else {
        Ok(None)
    }
}

fn require(
    capabilities: &std::collections::BTreeSet<SessionVerb>,
    verb: SessionVerb,
) -> Result<(), AuthorizationError> {
    if capabilities.contains(&verb) {
        Ok(())
    } else {
        Err(AuthorizationError::SessionVerbMissing(verb))
    }
}

/// Closed bus authorization failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationError {
    PolicyRevisionZero,
    ZoneMismatch,
    SessionBindingMismatch,
    RelayOriginInvalid,
    RelayGrantMissing,
    DiagnosticBinding,
    SessionVerbMissing(SessionVerb),
    Native(AuthorizationDenial),
    Policy(AuthorizationPolicyError),
    InvalidResourceCall,
    Assignment(AssignmentError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationErrorClass {
    MissingGrant,
    PolicyUnavailable,
    PolicyRevisionChanged,
    SessionBinding,
    RelayDenied,
    BootstrapDenied,
    UnknownResource,
    InvalidPolicy,
    InvalidRequest,
}

impl AuthorizationErrorClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingGrant => "missing-grant",
            Self::PolicyUnavailable => "policy-unavailable",
            Self::PolicyRevisionChanged => "policy-revision-changed",
            Self::SessionBinding => "session-binding",
            Self::RelayDenied => "relay-denied",
            Self::BootstrapDenied => "bootstrap-denied",
            Self::UnknownResource => "unknown-resource",
            Self::InvalidPolicy => "invalid-policy",
            Self::InvalidRequest => "invalid-request",
        }
    }
}

pub const fn session_verb_name(verb: SessionVerb) -> &'static str {
    match verb {
        SessionVerb::Connect => "connect",
        SessionVerb::Invoke => "invoke",
        SessionVerb::OpenStream => "open-stream",
        SessionVerb::Relay => "relay",
        SessionVerb::Attach => "attach",
        SessionVerb::Cancel => "cancel",
        SessionVerb::Observe => "observe",
        SessionVerb::AuditExport => "audit-export",
        SessionVerb::SupportBundle => "support-bundle",
    }
}

impl AuthorizationError {
    pub const fn class(self) -> AuthorizationErrorClass {
        match self {
            Self::SessionVerbMissing(_) => AuthorizationErrorClass::MissingGrant,
            Self::Native(AuthorizationDenial::PolicyUnavailable) => {
                AuthorizationErrorClass::PolicyUnavailable
            }
            Self::Native(AuthorizationDenial::PolicyRevisionChanged) => {
                AuthorizationErrorClass::PolicyRevisionChanged
            }
            Self::ZoneMismatch
            | Self::SessionBindingMismatch
            | Self::DiagnosticBinding
            | Self::Assignment(_)
            | Self::Native(AuthorizationDenial::ZoneMismatch) => {
                AuthorizationErrorClass::SessionBinding
            }
            Self::RelayOriginInvalid
            | Self::RelayGrantMissing
            | Self::Native(AuthorizationDenial::RelayOriginInvalid)
            | Self::Native(AuthorizationDenial::RelayGrantMissing)
            | Self::Native(AuthorizationDenial::RelayTargetGrantMissing) => {
                AuthorizationErrorClass::RelayDenied
            }
            Self::Native(AuthorizationDenial::BootstrapDenied) => {
                AuthorizationErrorClass::BootstrapDenied
            }
            Self::Native(AuthorizationDenial::UnknownResourceType) => {
                AuthorizationErrorClass::UnknownResource
            }
            Self::PolicyRevisionZero | Self::Policy(_) => AuthorizationErrorClass::InvalidPolicy,
            Self::InvalidResourceCall => AuthorizationErrorClass::InvalidRequest,
            Self::Native(AuthorizationDenial::NoMatchingGrant) => {
                AuthorizationErrorClass::MissingGrant
            }
        }
    }

    pub const fn required_verb(self) -> Option<SessionVerb> {
        match self {
            Self::SessionVerbMissing(verb) => Some(verb),
            _ => None,
        }
    }

    pub const fn native_denial(self) -> Option<AuthorizationDenial> {
        match self {
            Self::Native(denial) => Some(denial),
            _ => None,
        }
    }

    pub const fn remediation(self) -> &'static str {
        match self.class() {
            AuthorizationErrorClass::MissingGrant => "inspect-role-binding",
            AuthorizationErrorClass::PolicyUnavailable => "retry-policy-load",
            AuthorizationErrorClass::PolicyRevisionChanged => "retry-current-generation",
            AuthorizationErrorClass::SessionBinding => "reconnect-current-generation",
            AuthorizationErrorClass::RelayDenied => "inspect-relay-grant",
            AuthorizationErrorClass::BootstrapDenied => "complete-enrollment",
            AuthorizationErrorClass::UnknownResource => "repair-configuration",
            AuthorizationErrorClass::InvalidPolicy => "repair-policy",
            AuthorizationErrorClass::InvalidRequest => "repair-request",
        }
    }
}

impl core::fmt::Display for AuthorizationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "authorization-denied class={} remediation={}",
            self.class().as_str(),
            self.remediation()
        )?;
        if let Some(verb) = self.required_verb() {
            write!(f, " required-verb={}", session_verb_name(verb))?;
        }
        Ok(())
    }
}

impl std::error::Error for AuthorizationError {}

impl From<AuthorizationDenial> for AuthorizationError {
    fn from(value: AuthorizationDenial) -> Self {
        match value {
            AuthorizationDenial::ZoneMismatch => Self::ZoneMismatch,
            AuthorizationDenial::RelayOriginInvalid => Self::RelayOriginInvalid,
            AuthorizationDenial::RelayGrantMissing => Self::RelayGrantMissing,
            other => Self::Native(other),
        }
    }
}

impl From<AuthorizationPolicyError> for AuthorizationError {
    fn from(value: AuthorizationPolicyError) -> Self {
        Self::Policy(value)
    }
}

impl From<crate::router::BusError> for AuthorizationError {
    fn from(_value: crate::router::BusError) -> Self {
        Self::InvalidResourceCall
    }
}

#[cfg(test)]
mod tests {
    use d2b_contracts_provider::v3::{
        ArtifactDigest, ArtifactDigestSet, BinaryRef, CompatibilityRange, ComponentDescriptor,
        ComponentExecution, ComponentTargetCapability, ComponentType, ControllerInstanceScope,
        ControllerTargetKind, EffectPortClass, PolicyEvaluation, ProviderManifest,
        ResourceApiBinding, RevocationState, SignatureState, TargetRuntimeArtifacts, TrustEvidence,
        UpgradeDisposition, UpgradePolicy,
    };
    use d2b_contracts_resource::v3::{
    ConfigurationGeneration,
    ControllerGeneration,
    ResourceEnvelope,
    ResourceGeneration,
    ResourceRef,
    ResourceTypeName,
    ResourceUid,
    SchemaFingerprint,
    SchemaVersion,
    ZoneRevision,
};
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::identity::{
    BindingDigest,
    ReconnectGeneration,
    ServiceName,
    SessionBinding,
    SessionPurpose,
    TranscriptHash,
    TransportBinding,
};
    use d2b_resource_api::authz::{
        ApiCatalog, BindingScope, BootstrapPhase, BoundSubject, CompiledRole, CompiledRoleBinding,
        PolicyRule, RelayGrantAuthority, ResourceVerb,
    };
    use d2b_resource_store::PolicySnapshot;
    use d2b_core_controller::controller_assignment::{
        AssignmentIdentity, AssignmentRequest, ControllerAssignmentRegistry, ControllerRoleContract,
        ScopedResourceMutation, ScopedResourceQuery,
    };

    use super::*;
    use crate::{
        registry::{RouteGenerations, RouteMember, RouteTarget},
        router::{ResourceCall, ResourceQuery},
    };

    #[test]
    fn denials_expose_only_closed_class_verb_and_remediation() {
        let missing = AuthorizationError::SessionVerbMissing(SessionVerb::AuditExport);
        assert_eq!(missing.class(), AuthorizationErrorClass::MissingGrant);
        assert_eq!(missing.required_verb(), Some(SessionVerb::AuditExport));
        assert_eq!(missing.remediation(), "inspect-role-binding");
        assert_eq!(
            missing.to_string(),
            "authorization-denied class=missing-grant remediation=inspect-role-binding required-verb=audit-export"
        );

        let unavailable = AuthorizationError::Native(AuthorizationDenial::PolicyUnavailable);
        assert_eq!(
            unavailable.class(),
            AuthorizationErrorClass::PolicyUnavailable
        );
        assert_eq!(
            unavailable.native_denial(),
            Some(AuthorizationDenial::PolicyUnavailable)
        );
        assert_eq!(unavailable.remediation(), "retry-policy-load");
    }

    const SUBJECT_UID: &str = "11111111-1111-4111-8111-111111111111";

    fn fingerprint(value: char) -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}", value.to_string().repeat(64))).unwrap()
    }

    fn context(
        zone: &str,
        service: &str,
        locality: Locality,
        evidence: EvidenceClass,
    ) -> AuthenticatedSubjectContext {
        context_with_session(zone, service, locality, evidence, 1)
    }

    fn context_with_session(
        zone: &str,
        service: &str,
        locality: Locality,
        evidence: EvidenceClass,
        session_generation: u64,
    ) -> AuthenticatedSubjectContext {
        AuthenticatedSubjectContext::new(
            ResourceRef::parse(if locality == Locality::Local {
                "User/alice"
            } else {
                "ZoneLink/parent"
            })
            .unwrap(),
            ResourceUid::parse(SUBJECT_UID).unwrap(),
            ResourceRef::parse(&format!("Zone/{zone}")).unwrap(),
            evidence,
            SessionPurpose::parse("zone-bus").unwrap(),
            ServiceName::parse(service).unwrap(),
            SessionBinding::new(
                fingerprint('1'),
                TransportBinding::new(
                    locality,
                    BindingDigest::parse(format!("sha256:{}", "2".repeat(64))).unwrap(),
                ),
                ReconnectGeneration::new(session_generation).unwrap(),
                TranscriptHash::from_bytes([3; 32]),
            ),
        )
        .with_provider_ref(ResourceRef::parse("Provider/system-core").unwrap())
        .with_provider_generation(ResourceGeneration::new(2).unwrap())
        .with_controller_generation(ControllerGeneration::new(3).unwrap())
    }

    fn route(zone: &str, service: &str, member: RouteMember) -> RouteKey {
        route_with_session(zone, service, member, 1)
    }

    fn route_with_session(
        zone: &str,
        service: &str,
        member: RouteMember,
        session_generation: u64,
    ) -> RouteKey {
        RouteKey::new(
            ZoneId::parse(zone).unwrap(),
            ServiceName::parse(service).unwrap(),
            member,
            RouteTarget::provider(ResourceRef::parse("Provider/system-core").unwrap()).unwrap(),
            fingerprint('1'),
            RouteGenerations::new(
                Some(ResourceGeneration::new(2).unwrap()),
                Some(ControllerGeneration::new(3).unwrap()),
                ReconnectGeneration::new(session_generation).unwrap(),
            ),
        )
    }

    fn state(revision: u64) -> AuthorizationState {
        AuthorizationState {
            snapshot: PolicySnapshot {
                policy_revision: revision,
                api_catalog_revision: 1,
                active_configuration_revision: ConfigurationGeneration::new(1).unwrap(),
                controller_generation: Some(ControllerGeneration::new(3).unwrap()),
            },
            zone_policy_revision: ZoneRevision::new(revision),
            bootstrap_phase: BootstrapPhase::Disabled,
            now_tick: revision,
        }
    }

    fn policy(
        revision: u64,
        context: &AuthenticatedSubjectContext,
        session_verbs: &[SessionVerb],
        resource_types: &[ResourceTypeName],
        resource_verbs: &[ResourceVerb],
        scoped_zones: &[ZoneId],
    ) -> PolicySet {
        let catalog = ApiCatalog::standard();
        let rule = PolicyRule::new(
            &catalog,
            resource_types.iter().cloned(),
            resource_verbs.iter().copied(),
            session_verbs.iter().copied(),
            [],
            [],
            scoped_zones.iter().cloned(),
            [],
        )
        .unwrap();
        let role = CompiledRole::new(
            ResourceRef::parse("Role/authorization-test").unwrap(),
            vec![rule],
        )
        .unwrap();
        let binding = CompiledRoleBinding::new(
            role.role_ref.clone(),
            [BoundSubject {
                subject_ref: context.subject_ref().clone(),
                subject_uid: context.subject_uid().clone(),
            }],
            BindingScope::default(),
            if session_verbs.contains(&SessionVerb::Relay) {
                RelayGrantAuthority::CoreGenerated
            } else {
                RelayGrantAuthority::None
            },
        )
        .unwrap();
        PolicySet::new(&catalog, revision, vec![role], vec![binding]).unwrap()
    }

    fn authorizer(
        context: &AuthenticatedSubjectContext,
        session_verbs: &[SessionVerb],
        resource_verbs: &[ResourceVerb],
    ) -> BusAuthorizer {
        let resource_types = (!resource_verbs.is_empty())
            .then(|| ResourceTypeName::parse("Host").unwrap())
            .into_iter()
            .collect::<Vec<_>>();
        let native = NativeAuthorizer::new(
            ApiCatalog::standard(),
            Some(policy(
                1,
                context,
                session_verbs,
                &resource_types,
                resource_verbs,
                &[],
            )),
        )
        .unwrap();
        BusAuthorizer::new(native, state(1)).unwrap()
    }

    const ASSIGNMENT_DIGEST: &str =
        "sha256:0000000000000000000000000000000000000000000000000000000000000000";

    fn assignment_digest() -> ArtifactDigest {
        ArtifactDigest::parse(ASSIGNMENT_DIGEST).unwrap()
    }

    fn assignment_fingerprint() -> SchemaFingerprint {
        SchemaFingerprint::parse(ASSIGNMENT_DIGEST).unwrap()
    }

    fn assignment_manifest() -> ProviderManifest {
        let process = ResourceTypeName::parse("Process").unwrap();
        let component = ComponentDescriptor::new(
            BoundedToken::parse("process-controller").unwrap(),
            ComponentType::Controller,
            [process.clone()],
            [],
            [d2b_contracts_resource::v3::ExecutionDomain::System],
            8,
            assignment_digest(),
            [],
            false,
        )
        .unwrap()
        .with_execution(ComponentExecution::Launchable {
            binary_ref: BinaryRef::parse("process-controller").unwrap(),
        })
        .with_controller_placement(
            ControllerInstanceScope::PerResourceTarget,
            [ControllerTargetKind::Host, ControllerTargetKind::Guest],
        )
        .unwrap()
        .with_target_capabilities([
            ComponentTargetCapability::new(
                ControllerTargetKind::Host,
                assignment_digest(),
                [EffectPortClass::Process],
            )
            .unwrap(),
            ComponentTargetCapability::new(
                ControllerTargetKind::Guest,
                assignment_digest(),
                [EffectPortClass::Process],
            )
            .unwrap(),
        ])
        .unwrap();
        let binding = ResourceApiBinding::new_with_placement(
            process,
            SchemaVersion::new(1, 0).unwrap(),
            assignment_fingerprint(),
            SchemaVersion::new(1, 0).unwrap(),
            assignment_fingerprint(),
            Default::default(),
            None,
            None,
            d2b_contracts_resource::v3::PlacementAnchor::ExecutionRef,
        )
        .unwrap();
        let trust = TrustEvidence {
            publisher: BoundedToken::parse("trusted").unwrap(),
            root_epoch: 1,
            publisher_trusted: true,
            signature: SignatureState::Valid,
            revocation: RevocationState::Clear,
            emergency_deny: false,
            provenance: PolicyEvaluation::Accepted,
            sbom: PolicyEvaluation::Accepted,
            license: PolicyEvaluation::Accepted,
            vulnerability: PolicyEvaluation::Accepted,
            conformance: PolicyEvaluation::Accepted,
            support_channel: BoundedToken::parse("stable").unwrap(),
        };
        ProviderManifest::new(
            d2b_contracts_resource::v3::ArtifactId::parse("provider-runtime").unwrap(),
            ArtifactDigestSet {
                executable: assignment_digest(),
                config: assignment_digest(),
                schema: assignment_digest(),
                service: assignment_digest(),
            },
            trust,
            CompatibilityRange {
                api_major: 3,
                api_minor: 0,
                descriptor_fingerprint: assignment_fingerprint(),
                state_schema_version: SchemaVersion::new(1, 0).unwrap(),
            },
            [component],
            [binding],
            [],
            UpgradePolicy {
                drain_before_upgrade: true,
                max_automatic_disposition: UpgradeDisposition::InPlace,
                preserves_durable_state: true,
            },
        )
        .unwrap()
        .with_target_runtime_artifacts([
            TargetRuntimeArtifacts::new(
                ControllerTargetKind::Host,
                assignment_digest(),
                assignment_digest(),
            )
            .unwrap(),
            TargetRuntimeArtifacts::new(
                ControllerTargetKind::Guest,
                assignment_digest(),
                assignment_digest(),
            )
            .unwrap(),
        ])
        .unwrap()
    }

    fn assignment_role() -> ControllerRoleContract {
        ControllerRoleContract::from_signed_manifest(
            ResourceRef::parse("Provider/system-core").unwrap(),
            ResourceRef::parse("Process/process-controller").unwrap(),
            &assignment_manifest(),
        )
        .unwrap()
    }

    fn assignment_resource() -> ResourceEnvelope {
        let value = serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "Process",
            "metadata": {
                "name": "process-one",
                "zone": "dev",
                "uid": "123e4567-e89b-42d3-a456-426614174000",
                "generation": 1,
                "revision": 1,
                "ownerRef": null,
                "finalizers": [],
                "deletionRequestedAt": null,
                "createdAt": "2026-07-22T00:00:00.000Z",
                "updatedAt": "2026-07-22T00:00:00.000Z",
                "managedBy": "api",
                "configurationGeneration": null,
                "controllerGeneration": null,
                "providerGeneration": null
            },
            "spec": {
                "providerRef": "Provider/system-core",
                "executionRef": "Host/host-system",
                "argv": ["true"]
            },
            "status": {
                "completedAt": null,
                "conditions": [],
                "lastReconciledAt": null,
                "observedGeneration": 0,
                "outcome": null,
                "phase": "Pending",
                "resource": {},
                "startedAt": null,
                "update": {
                    "dependencies": {"count": 0, "refs": []},
                    "disruption": "None",
                    "lastAssessedAt": null,
                    "observedGeneration": 0,
                    "operationId": null,
                    "owned": {"count": 0, "refs": []},
                    "preserveState": true,
                    "reasons": [],
                    "state": "Unknown",
                    "targetGeneration": 1
                }
            }
        });
        ResourceEnvelope::from_json(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    fn assignment_fixture() -> (
        AuthenticatedSubjectContext,
        std::sync::Arc<Mutex<ControllerAssignmentRegistry>>,
        AssignmentIdentity,
        ScopedResourceQuery,
        ScopedResourceMutation,
        ScopedResourceMutation,
    ) {
        let resource = assignment_resource();
        let role = assignment_role();
        let mut registry = ControllerAssignmentRegistry::default();
        let lease = registry
            .admit(AssignmentRequest::new(
                &resource,
                &role,
                ResourceGeneration::new(2).unwrap(),
                ControllerGeneration::new(3).unwrap(),
                ReconnectGeneration::new(4).unwrap(),
                true,
            ))
            .unwrap();
        let query = lease
            .query(
                vec![ResourceTypeName::parse("Process").unwrap()],
                vec![resource.metadata().name().clone()],
                Vec::new(),
            )
            .unwrap();
        let mutation = lease
            .mutation(
                ResourceRef::parse("Process/process-one").unwrap(),
                AssignmentVerb::UpdateStatus,
            )
            .unwrap();
        let unsupported = lease
            .mutation(
                ResourceRef::parse("Process/process-one").unwrap(),
                AssignmentVerb::Get,
            )
            .unwrap();
        let identity = lease.identity().clone();
        let context = context_with_session(
            "dev",
            "d2b.resource.v3",
            Locality::Local,
            EvidenceClass::UnixPeer,
            4,
        )
        .with_execution_ref(ResourceRef::parse("Host/host-system").unwrap());
        (
            context,
            std::sync::Arc::new(Mutex::new(registry)),
            identity,
            query,
            mutation,
            unsupported,
        )
    }

    fn assignment_authorizer(
        context: &AuthenticatedSubjectContext,
        registry: std::sync::Arc<Mutex<ControllerAssignmentRegistry>>,
        resource_verbs: &[ResourceVerb],
    ) -> BusAuthorizer {
        let native = NativeAuthorizer::new(
            ApiCatalog::standard(),
            Some(policy(
                1,
                context,
                &[
                    SessionVerb::Connect,
                    SessionVerb::Invoke,
                    SessionVerb::OpenStream,
                ],
                &[ResourceTypeName::parse("Process").unwrap()],
                resource_verbs,
                &[],
            )),
        )
        .unwrap();
        BusAuthorizer::new(native, state(1))
            .unwrap()
            .with_assignment_registry(registry)
    }

    #[test]
    fn assignment_authorization_uses_registry_for_scoped_commit_and_watch() {
        let (context, registry, identity, query, mutation, unsupported) = assignment_fixture();
        let watch_call = ResourceCall::Watch(ResourceQuery::from_scoped(query).unwrap());
        let commit_call = ResourceCall::ScopedCommitBatch {
            assignment: identity.clone(),
            mutations: vec![mutation],
        };
        let authorizer = assignment_authorizer(
            &context,
            std::sync::Arc::clone(&registry),
            &[
                ResourceVerb::Watch,
                ResourceVerb::UpdateStatus,
                ResourceVerb::Delete,
            ],
        );
        let watch_route = route_with_session(
            "dev",
            "d2b.resource.v3",
            RouteMember::stream("ResourceService/Watch").unwrap(),
            4,
        );
        let commit_route = route_with_session(
            "dev",
            "d2b.resource.v3",
            RouteMember::method("ResourceService/CommitBatch").unwrap(),
            4,
        );
        assert_eq!(
            authorizer.authorize_dispatch(&context, &watch_route, Some(&watch_call), true),
            Ok(())
        );
        assert_eq!(
            authorizer.authorize_dispatch(&context, &commit_route, Some(&commit_call), false),
            Ok(())
        );

        let unsupported_call = ResourceCall::ScopedCommitBatch {
            assignment: identity,
            mutations: vec![unsupported],
        };
        assert_eq!(
            authorizer.authorize_dispatch(
                &context,
                &commit_route,
                Some(&unsupported_call),
                false,
            ),
            Err(AuthorizationError::InvalidResourceCall)
        );
    }

    #[test]
    fn assignment_authorization_preserves_owner_child_scope_for_processes() {
        let resource = assignment_resource();
        let role = assignment_role();
        let mut registry = ControllerAssignmentRegistry::default();
        let lease = registry
            .admit(AssignmentRequest::new(
                &resource,
                &role,
                ResourceGeneration::new(2).unwrap(),
                ControllerGeneration::new(3).unwrap(),
                ReconnectGeneration::new(4).unwrap(),
                true,
            ))
            .unwrap();
        let query = lease
            .child_query(
                vec![ResourceTypeName::parse("Process").unwrap()],
                Vec::new(),
                Vec::new(),
            )
            .unwrap();
        let query = ResourceQuery::from_scoped(query).unwrap();
        let owner_scope = query.scope().unwrap().owner_child().unwrap();
        assert_eq!(owner_scope.owner_ref(), lease.resource_ref());
        assert_eq!(owner_scope.owner_uid(), resource.metadata().uid());
        assert_eq!(query.filters()[0].field(), "owner.resourceUid");

        let child = lease
            .child_mutation(
                ResourceRef::parse("Process/process-child").unwrap(),
                AssignmentVerb::Create,
            )
            .unwrap();
        let identity = lease.identity().clone();
        let call = ResourceCall::ScopedCommitBatch {
            assignment: identity.clone(),
            mutations: vec![child],
        };
        let context = context_with_session(
            "dev",
            "d2b.resource.v3",
            Locality::Local,
            EvidenceClass::UnixPeer,
            4,
        )
        .with_execution_ref(ResourceRef::parse("Host/host-system").unwrap());
        let registry = std::sync::Arc::new(Mutex::new(registry));
        let authorizer = assignment_authorizer(
            &context,
            std::sync::Arc::clone(&registry),
            &[
                ResourceVerb::Watch,
                ResourceVerb::Create,
                ResourceVerb::UpdateSpec,
                ResourceVerb::Delete,
            ],
        );
        let route = route_with_session(
            "dev",
            "d2b.resource.v3",
            RouteMember::method("ResourceService/CommitBatch").unwrap(),
            4,
        );
        assert_eq!(
            authorizer.authorize_dispatch(&context, &route, Some(&call), false),
            Ok(())
        );
    }

    #[test]
    fn assignment_authorization_rejects_session_and_target_mismatch() {
        let (valid_context, registry, _identity, query, _, _) = assignment_fixture();
        let call = ResourceCall::Watch(ResourceQuery::from_scoped(query).unwrap());
        let route = route_with_session(
            "dev",
            "d2b.resource.v3",
            RouteMember::stream("ResourceService/Watch").unwrap(),
            1,
        );
        let session_mismatch = context(
            "dev",
            "d2b.resource.v3",
            Locality::Local,
            EvidenceClass::UnixPeer,
        )
        .with_execution_ref(ResourceRef::parse("Host/host-system").unwrap());
        let authorizer = assignment_authorizer(
            &session_mismatch,
            std::sync::Arc::clone(&registry),
            &[ResourceVerb::Watch],
        );
        assert_eq!(
            authorizer.authorize_dispatch(&session_mismatch, &route, Some(&call), true),
            Err(AuthorizationError::Assignment(
                AssignmentError::SessionBindingMismatch
            ))
        );

        let target_mismatch = valid_context
            .clone()
            .with_execution_ref(ResourceRef::parse("Host/other").unwrap());
        let target_authorizer = assignment_authorizer(
            &target_mismatch,
            registry,
            &[ResourceVerb::Watch],
        );
        let route = route_with_session(
            "dev",
            "d2b.resource.v3",
            RouteMember::stream("ResourceService/Watch").unwrap(),
            4,
        );
        assert_eq!(
            target_authorizer.authorize_dispatch(&target_mismatch, &route, Some(&call), true),
            Err(AuthorizationError::Assignment(
                AssignmentError::TargetMismatch
            ))
        );
    }

    #[test]
    fn assignment_authorization_rejects_revoked_and_stale_registry_scope() {
        let (context, registry, identity, _query, mutation, _) = assignment_fixture();
        let commit_call = ResourceCall::ScopedCommitBatch {
            assignment: identity.clone(),
            mutations: vec![mutation],
        };
        let commit_route = route_with_session(
            "dev",
            "d2b.resource.v3",
            RouteMember::method("ResourceService/CommitBatch").unwrap(),
            4,
        );
        registry
            .lock()
            .unwrap()
            .revoke_session(ReconnectGeneration::new(4).unwrap());
        let revoked_authorizer = assignment_authorizer(
            &context,
            std::sync::Arc::clone(&registry),
            &[ResourceVerb::UpdateStatus],
        );
        assert_eq!(
            revoked_authorizer.authorize_dispatch(
                &context,
                &commit_route,
                Some(&commit_call),
                false,
            ),
            Err(AuthorizationError::Assignment(
                AssignmentError::SessionRevoked
            ))
        );

        let (context, registry, identity, query, _, _) = assignment_fixture();
        let watch_call = ResourceCall::Watch(ResourceQuery::from_scoped(query).unwrap());
        registry
            .lock()
            .unwrap()
            .begin_drain(&identity)
            .unwrap();
        let stale_authorizer =
            assignment_authorizer(&context, registry, &[ResourceVerb::Watch]);
        let watch_route = route_with_session(
            "dev",
            "d2b.resource.v3",
            RouteMember::stream("ResourceService/Watch").unwrap(),
            4,
        );
        assert_eq!(
            stale_authorizer.authorize_dispatch(&context, &watch_route, Some(&watch_call), true),
            Err(AuthorizationError::Assignment(
                AssignmentError::StaleAssignment
            ))
        );
    }

    #[test]
    fn cross_zone_dispatch_fails_only_on_authenticated_zone_inequality() {
        let claims = context(
            "dev",
            "d2b.resource.v3",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let dev_authorizer = authorizer(&claims, &[SessionVerb::Connect, SessionVerb::Invoke], &[]);
        let personal_route = route(
            "personal",
            "d2b.resource.v3",
            RouteMember::method("ResourceService/Get").unwrap(),
        );

        assert_eq!(claims.zone_ref().name().as_str(), "dev");
        assert_eq!(personal_route.zone().as_str(), "personal");
        assert_eq!(claims.service(), personal_route.service());
        assert_eq!(claims.schema_fingerprint(), personal_route.schema());
        assert_eq!(
            claims.reconnect_generation(),
            personal_route.generations().session()
        );
        assert_eq!(
            dev_authorizer.authorize_dispatch(&claims, &personal_route, None, false),
            Err(AuthorizationError::ZoneMismatch)
        );

        let personal_claims = context(
            "personal",
            "d2b.resource.v3",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let personal_authorizer = authorizer(
            &personal_claims,
            &[SessionVerb::Connect, SessionVerb::Invoke],
            &[],
        );
        assert_eq!(
            personal_authorizer.authorize_dispatch(&personal_claims, &personal_route, None, false,),
            Ok(())
        );
    }

    #[test]
    fn cross_zone_connect_precedes_policy_availability() {
        let claims = context(
            "dev",
            "d2b.echo.v3",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let authorizer = authorizer(&claims, &[SessionVerb::Connect], &[]);
        authorizer.mark_policy_unavailable();
        assert_eq!(
            authorizer.authorize_connect(&claims, &ZoneId::parse("personal").unwrap()),
            Err(AuthorizationError::ZoneMismatch)
        );
    }

    #[test]
    fn cross_zone_cancel_fails_only_on_authenticated_zone_inequality() {
        let claims = context(
            "dev",
            "d2b.echo.v3",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let authorizer = authorizer(&claims, &[SessionVerb::Connect, SessionVerb::Cancel], &[]);
        let personal_route = route(
            "personal",
            "d2b.echo.v3",
            RouteMember::method("EchoService/Call").unwrap(),
        );

        assert_eq!(claims.service(), personal_route.service());
        assert_eq!(claims.schema_fingerprint(), personal_route.schema());
        assert_eq!(
            claims.reconnect_generation(),
            personal_route.generations().session()
        );
        assert_eq!(
            authorizer.authorize_cancel(&claims, &personal_route),
            Err(AuthorizationError::ZoneMismatch)
        );
    }

    #[test]
    fn zero_policy_revision_is_not_a_bus_bootstrap_escape_hatch() {
        let native = NativeAuthorizer::new(ApiCatalog::standard(), None).unwrap();
        assert!(matches!(
            BusAuthorizer::new(native, state(0)),
            Err(AuthorizationError::PolicyRevisionZero)
        ));
    }

    #[tokio::test]
    async fn policy_commit_fences_the_previous_session_lease() {
        let claims = context(
            "dev",
            "d2b.echo.v3",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let authorizer = authorizer(&claims, &[SessionVerb::Connect, SessionVerb::Invoke], &[]);
        let lease = authorizer
            .authenticate_session(&claims, &ZoneId::parse("dev").unwrap(), 1)
            .unwrap();
        let request = SessionAuthorizationRequest::new(
            SessionVerb::Invoke,
            claims.service().clone(),
            "EchoService/Call",
            ZoneId::parse("dev").unwrap(),
            None,
        )
        .unwrap();
        let replacement = policy(
            1,
            &claims,
            &[SessionVerb::Connect, SessionVerb::Invoke],
            &[],
            &[],
            &[],
        );
        let mut fenced_state = state(1);
        fenced_state.zone_policy_revision = ZoneRevision::new(2);
        authorizer.replace_policy(replacement, fenced_state).unwrap();
        let error = authorizer
            .authorize_session(&claims, &request, lease, 2)
            .unwrap_err();
        assert_eq!(error.code(), d2b_session::contract::SessionErrorCode::PolicyDenied);
    }

    #[test]
    fn policy_revision_cannot_move_backwards() {
        let claims = context(
            "dev",
            "d2b.echo.v3",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let authorizer = authorizer(&claims, &[SessionVerb::Connect], &[]);
        let replacement = policy(1, &claims, &[SessionVerb::Connect], &[], &[], &[]);
        let mut stale = state(1);
        stale.zone_policy_revision = ZoneRevision::new(0);
        assert_eq!(
            authorizer.replace_policy(replacement, stale),
            Err(AuthorizationError::Native(
                AuthorizationDenial::PolicyRevisionChanged
            ))
        );
    }

    #[test]
    fn every_session_binding_dimension_is_checked() {
        let claims = context(
            "dev",
            "d2b.echo.v3",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let authorizer = authorizer(&claims, &[SessionVerb::Connect, SessionVerb::Invoke], &[]);
        let exact = route(
            "dev",
            "d2b.echo.v3",
            RouteMember::method("EchoService/Call").unwrap(),
        );
        assert_eq!(
            authorizer.authorize_dispatch(&claims, &exact, None, false),
            Ok(())
        );
        let mutations = [
            RouteKey::new(
                exact.zone().clone(),
                ServiceName::parse("d2b.other.v3").unwrap(),
                exact.member().clone(),
                exact.target().clone(),
                exact.schema().clone(),
                exact.generations(),
            ),
            RouteKey::new(
                exact.zone().clone(),
                exact.service().clone(),
                exact.member().clone(),
                exact.target().clone(),
                fingerprint('9'),
                exact.generations(),
            ),
            RouteKey::new(
                exact.zone().clone(),
                exact.service().clone(),
                exact.member().clone(),
                exact.target().clone(),
                exact.schema().clone(),
                RouteGenerations::new(
                    exact.generations().provider(),
                    exact.generations().controller(),
                    ReconnectGeneration::new(2).unwrap(),
                ),
            ),
            RouteKey::new(
                exact.zone().clone(),
                exact.service().clone(),
                exact.member().clone(),
                exact.target().clone(),
                exact.schema().clone(),
                RouteGenerations::new(
                    Some(ResourceGeneration::new(9).unwrap()),
                    exact.generations().controller(),
                    exact.generations().session(),
                ),
            ),
            RouteKey::new(
                exact.zone().clone(),
                exact.service().clone(),
                exact.member().clone(),
                exact.target().clone(),
                exact.schema().clone(),
                RouteGenerations::new(
                    exact.generations().provider(),
                    Some(ControllerGeneration::new(9).unwrap()),
                    exact.generations().session(),
                ),
            ),
            RouteKey::new(
                exact.zone().clone(),
                exact.service().clone(),
                exact.member().clone(),
                RouteTarget::provider(ResourceRef::parse("Provider/other").unwrap()).unwrap(),
                exact.schema().clone(),
                exact.generations(),
            ),
        ];
        for mutation in mutations {
            assert_eq!(
                authorizer.authorize_dispatch(&claims, &mutation, None, false),
                Err(AuthorizationError::SessionBindingMismatch)
            );
        }
    }

    #[test]
    fn invalid_relay_evidence_is_denied_before_policy_lookup() {
        let claims = context(
            "dev",
            "d2b.echo.v3",
            Locality::AdjacentZone,
            EvidenceClass::BootstrapIkpsk2,
        );
        let authorizer = authorizer(
            &claims,
            &[
                SessionVerb::Connect,
                SessionVerb::Invoke,
                SessionVerb::Relay,
            ],
            &[],
        );
        authorizer.mark_policy_unavailable();
        assert_eq!(
            authorizer.authorize_dispatch(
                &claims,
                &route(
                    "dev",
                    "d2b.echo.v3",
                    RouteMember::method("EchoService/Call").unwrap(),
                ),
                None,
                false,
            ),
            Err(AuthorizationError::RelayOriginInvalid)
        );
    }

    #[test]
    fn relay_grant_and_forwarded_target_grant_fail_independently() {
        let claims = context(
            "dev",
            "d2b.resource.v3",
            Locality::AdjacentZone,
            EvidenceClass::EnrolledKk,
        );
        let target_route = route(
            "dev",
            "d2b.resource.v3",
            RouteMember::method("ResourceService/Get").unwrap(),
        );
        let call = ResourceCall::Get(ResourceRef::parse("Host/system").unwrap());
        let no_relay = authorizer(
            &claims,
            &[SessionVerb::Connect, SessionVerb::Invoke],
            &[ResourceVerb::Get],
        );
        assert_eq!(
            no_relay.authorize_dispatch(&claims, &target_route, Some(&call), false),
            Err(AuthorizationError::RelayGrantMissing)
        );

        let no_target = authorizer(
            &claims,
            &[
                SessionVerb::Connect,
                SessionVerb::Invoke,
                SessionVerb::Relay,
            ],
            &[],
        );
        assert_eq!(
            no_target.authorize_dispatch(&claims, &target_route, Some(&call), false),
            Err(AuthorizationError::Native(
                AuthorizationDenial::RelayTargetGrantMissing
            ))
        );
    }

    #[test]
    fn diagnostic_verbs_are_exact_and_cannot_carry_resource_authority() {
        for (service, member, verb) in [
            (
                "d2b.audit.v3",
                "AuditService/Export",
                SessionVerb::AuditExport,
            ),
            (
                "d2b.support.v3",
                "SupportService/GenerateBundle",
                SessionVerb::SupportBundle,
            ),
        ] {
            let claims = context("dev", service, Locality::Local, EvidenceClass::UnixPeer);
            let exact_authorizer = authorizer(&claims, &[SessionVerb::Connect, verb], &[]);
            let exact = route("dev", service, RouteMember::method(member).unwrap());
            assert_eq!(
                exact_authorizer.authorize_dispatch(&claims, &exact, None, false),
                Ok(())
            );
            assert_eq!(
                authorizer(&claims, &[SessionVerb::Connect], &[])
                    .authorize_dispatch(&claims, &exact, None, false),
                Err(AuthorizationError::SessionVerbMissing(verb))
            );
            assert_eq!(
                exact_authorizer.authorize_dispatch(
                    &claims,
                    &exact,
                    Some(&ResourceCall::Get(
                        ResourceRef::parse("Host/system").unwrap()
                    )),
                    false,
                ),
                Err(AuthorizationError::DiagnosticBinding)
            );
            let near_miss = route(
                "dev",
                service,
                RouteMember::method("OtherService/Other").unwrap(),
            );
            assert_eq!(
                exact_authorizer.authorize_dispatch(&claims, &near_miss, None, false),
                Err(AuthorizationError::DiagnosticBinding)
            );
        }
    }

    #[test]
    fn each_dispatch_session_verb_is_independently_required() {
        let claims = context(
            "dev",
            "d2b.echo.v3",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let no_connect = authorizer(&claims, &[], &[]);
        assert_eq!(
            no_connect.authorize_connect(&claims, &ZoneId::parse("dev").unwrap()),
            Err(AuthorizationError::SessionVerbMissing(SessionVerb::Connect))
        );

        let connect_only = authorizer(&claims, &[SessionVerb::Connect], &[]);
        assert_eq!(
            connect_only.authorize_dispatch(
                &claims,
                &route(
                    "dev",
                    "d2b.echo.v3",
                    RouteMember::method("EchoService/Call").unwrap(),
                ),
                None,
                false,
            ),
            Err(AuthorizationError::SessionVerbMissing(SessionVerb::Invoke))
        );
        assert_eq!(
            connect_only.authorize_dispatch(
                &claims,
                &route(
                    "dev",
                    "d2b.echo.v3",
                    RouteMember::stream("EchoService/Stream").unwrap(),
                ),
                None,
                true,
            ),
            Err(AuthorizationError::SessionVerbMissing(
                SessionVerb::OpenStream
            ))
        );
        assert_eq!(
            connect_only.authorize_cancel(
                &claims,
                &route(
                    "dev",
                    "d2b.echo.v3",
                    RouteMember::method("EchoService/Call").unwrap(),
                ),
            ),
            Err(AuthorizationError::SessionVerbMissing(SessionVerb::Cancel))
        );
    }

    #[test]
    fn native_policy_outage_revision_denial_and_unknown_type_are_preserved() {
        let claims = context(
            "dev",
            "d2b.resource.v3",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let target_route = route(
            "dev",
            "d2b.resource.v3",
            RouteMember::method("ResourceService/Get").unwrap(),
        );
        let no_resource_grant =
            authorizer(&claims, &[SessionVerb::Connect, SessionVerb::Invoke], &[]);
        assert_eq!(
            no_resource_grant.authorize_dispatch(
                &claims,
                &target_route,
                Some(&ResourceCall::Get(
                    ResourceRef::parse("Host/system").unwrap()
                )),
                false,
            ),
            Err(AuthorizationError::Native(
                AuthorizationDenial::NoMatchingGrant
            ))
        );
        let outage = authorizer(
            &claims,
            &[SessionVerb::Connect, SessionVerb::Invoke],
            &[ResourceVerb::Get],
        );
        outage.mark_policy_unavailable();
        assert_eq!(
            outage.authorize_connect(&claims, &ZoneId::parse("dev").unwrap()),
            Err(AuthorizationError::Native(
                AuthorizationDenial::PolicyUnavailable
            ))
        );

        let mismatched_native = NativeAuthorizer::new(
            ApiCatalog::standard(),
            Some(policy(1, &claims, &[SessionVerb::Connect], &[], &[], &[])),
        )
        .unwrap();
        let mismatched = BusAuthorizer::new(mismatched_native, state(2)).unwrap();
        assert_eq!(
            mismatched.authorize_connect(&claims, &ZoneId::parse("dev").unwrap()),
            Err(AuthorizationError::Native(
                AuthorizationDenial::PolicyRevisionChanged
            ))
        );

        let extension_call = ResourceCall::InspectSchema(
            ResourceTypeName::parse("example.d2bus.org.Widget").unwrap(),
        );
        assert_eq!(
            authorizer(&claims, &[SessionVerb::Connect, SessionVerb::Invoke], &[],)
                .authorize_dispatch(
                    &claims,
                    &route(
                        "dev",
                        "d2b.resource.v3",
                        RouteMember::method("ResourceService/InspectSchema").unwrap(),
                    ),
                    Some(&extension_call),
                    false,
                ),
            Err(AuthorizationError::Native(
                AuthorizationDenial::UnknownResourceType
            ))
        );
    }

    #[test]
    fn malformed_resource_call_and_policy_replacement_fail_closed() {
        let claims = context(
            "dev",
            "d2b.resource.v3",
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let authorizer = authorizer(&claims, &[SessionVerb::Connect, SessionVerb::Invoke], &[]);
        let empty_batch = ResourceCall::CommitBatch(Vec::new());
        assert_eq!(
            authorizer.authorize_dispatch(
                &claims,
                &route(
                    "dev",
                    "d2b.resource.v3",
                    RouteMember::method("ResourceService/CommitBatch").unwrap(),
                ),
                Some(&empty_batch),
                false,
            ),
            Err(AuthorizationError::InvalidResourceCall)
        );

        let replacement = policy(2, &claims, &[SessionVerb::Connect], &[], &[], &[]);
        assert_eq!(
            authorizer.replace_policy(replacement, state(3)),
            Err(AuthorizationError::Policy(
                AuthorizationPolicyError::PolicyStateRevisionMismatch
            ))
        );

        let extended_catalog =
            ApiCatalog::with_extensions([
                ResourceTypeName::parse("example.d2bus.org.Widget").unwrap()
            ])
            .unwrap();
        let foreign_catalog_policy =
            PolicySet::new(&extended_catalog, 2, Vec::new(), Vec::new()).unwrap();
        assert_eq!(
            authorizer.replace_policy(foreign_catalog_policy, state(2)),
            Err(AuthorizationError::Policy(
                AuthorizationPolicyError::CatalogMismatch
            ))
        );
    }

    #[test]
    fn nameless_query_is_not_an_authorization_target_omission() {
        let query = ResourceQuery::new(
            vec![ResourceTypeName::parse("Host").unwrap()],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let request = ResourceCall::List(query)
            .authorization_request(ZoneId::parse("dev").unwrap())
            .unwrap();
        assert_eq!(request.targets.len(), 1);
        assert!(request.targets[0].resource_name.is_none());
    }
}
