//! Zone-scoped authorization over the native Role and RoleBinding evaluator.

use std::sync::{Mutex, MutexGuard};

use d2b_contracts::v3::{AuthenticatedSubjectContext, EvidenceClass, Locality, ZoneId};
use d2b_resource_api::authz::{
    AuthorizationDenial, AuthorizationPolicyError, AuthorizationState, NativeAuthorizer, PolicySet,
    SessionVerb,
};

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
    let service = route.service().as_str();
    let member = route.member().as_str();
    match (service, member) {
        ("d2b.audit.v3", "AuditService/Export") if route.member().is_method() => {
            Ok(Some(SessionVerb::AuditExport))
        }
        ("d2b.support.v3", "SupportService/GenerateBundle") if route.member().is_method() => {
            Ok(Some(SessionVerb::SupportBundle))
        }
        ("d2b.audit.v3" | "d2b.support.v3", _) => Err(AuthorizationError::DiagnosticBinding),
        _ => Ok(None),
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

impl core::fmt::Display for AuthorizationError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::PolicyRevisionZero => "bus authorization requires durable policy",
            Self::ZoneMismatch => "authenticated subject and route Zone differ",
            Self::SessionBindingMismatch => "route differs from the authenticated session binding",
            Self::RelayOriginInvalid => "relay origin is not an enrolled adjacent Zone",
            Self::RelayGrantMissing => "relay authorization is missing",
            Self::DiagnosticBinding => "diagnostic verb is not bound to its exact service method",
            Self::SessionVerbMissing(_) => "required session verb is missing",
            Self::Native(_) => "native authorization denied the request",
            Self::Policy(_) => "native authorization policy is invalid",
            Self::InvalidResourceCall => "resource authorization input is invalid",
        })
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
