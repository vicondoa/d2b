//! Zone-scoped authorization over the native Role and RoleBinding evaluator.

use std::sync::{Mutex, MutexGuard};

use d2b_contracts::v3::{AuthenticatedSubjectContext, EvidenceClass, Locality, ZoneId};
use d2b_resource_api::authz::{
    AuthorizationDenial, AuthorizationPolicyError, AuthorizationState, NativeAuthorizer, PolicySet,
    SessionVerb,
};
use d2b_session::{OperationMember, SessionOperation};

use crate::{
    registry::{RouteKey, RouteTarget},
    router::ResourceCall,
};

struct AuthorizationRuntime {
    native: NativeAuthorizer,
    state: AuthorizationState,
}

/// Single-owner native authorizer and trusted policy state for one bus.
pub struct BusAuthorizer {
    runtime: Mutex<AuthorizationRuntime>,
}

impl BusAuthorizer {
    /// Consume the evaluator and trusted policy state into one private owner.
    pub fn new(
        native: NativeAuthorizer,
        state: AuthorizationState,
    ) -> Result<Self, AuthorizationError> {
        if state.snapshot.policy_revision == 0 {
            return Err(AuthorizationError::PolicyRevisionZero);
        }
        Ok(Self {
            runtime: Mutex::new(AuthorizationRuntime { native, state }),
        })
    }

    /// Install a new durable policy and its exact trusted revision state.
    pub fn replace_policy(
        &self,
        policy: PolicySet,
        state: AuthorizationState,
    ) -> Result<(), AuthorizationError> {
        let mut runtime = self.lock();
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
            let request = call.authorization_request(route.zone().clone())?;
            runtime
                .native
                .authorize(context, &request, &runtime.state)?;
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

    fn lock(&self) -> MutexGuard<'_, AuthorizationRuntime> {
        self.runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
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
    use d2b_contracts::v3::{
        BindingDigest, ConfigurationGeneration, ControllerGeneration, ReconnectGeneration,
        ResourceGeneration, ResourceRef, ResourceTypeName, ResourceUid, SchemaFingerprint,
        ServiceName, SessionBinding, SessionPurpose, TranscriptHash, TransportBinding,
        ZoneRevision,
    };
    use d2b_resource_api::authz::{
        ApiCatalog, BindingScope, BootstrapPhase, BoundSubject, CompiledRole, CompiledRoleBinding,
        PolicyRule, RelayGrantAuthority, ResourceVerb,
    };
    use d2b_resource_store::PolicySnapshot;

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
                ReconnectGeneration::new(1).unwrap(),
                TranscriptHash::from_bytes([3; 32]),
            ),
        )
        .with_provider_ref(ResourceRef::parse("Provider/system-core").unwrap())
        .with_provider_generation(ResourceGeneration::new(2).unwrap())
        .with_controller_generation(ControllerGeneration::new(3).unwrap())
    }

    fn route(zone: &str, service: &str, member: RouteMember) -> RouteKey {
        RouteKey::new(
            ZoneId::parse(zone).unwrap(),
            ServiceName::parse(service).unwrap(),
            member,
            RouteTarget::provider(ResourceRef::parse("Provider/system-core").unwrap()).unwrap(),
            fingerprint('1'),
            RouteGenerations::new(
                Some(ResourceGeneration::new(2).unwrap()),
                Some(ControllerGeneration::new(3).unwrap()),
                ReconnectGeneration::new(1).unwrap(),
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
