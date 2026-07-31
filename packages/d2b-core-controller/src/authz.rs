//! Revision-bound core authorization index publication and evaluation.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::v3::{
    AuthenticatedSubjectContext, ControllerGeneration, EvidenceClass, Locality, ResourceGeneration,
    ResourceName, ResourceRef, ResourceTypeName, ResourceUid, SchemaFingerprint, ServiceName,
    SessionPurpose,
};

/// Borrowed subject evidence admitted by the authenticated session adapter.
///
/// There is deliberately no public constructor. A contract value constructed
/// by an ordinary caller cannot become authorization evidence. The future
/// production ComponentSession adapter must mint this wrapper inside this
/// crate after transport authentication and authoritative subject resolution.
pub struct CoreAuthorizationSubject<'a> {
    context: &'a AuthenticatedSubjectContext,
}

impl<'a> CoreAuthorizationSubject<'a> {
    #[cfg(test)]
    pub(crate) const fn from_authenticated_session(
        context: &'a AuthenticatedSubjectContext,
    ) -> Self {
        Self { context }
    }
}

impl core::fmt::Debug for CoreAuthorizationSubject<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("CoreAuthorizationSubject(<redacted>)")
    }
}

/// Closed core resource verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CoreResourceVerb {
    Get,
    List,
    Watch,
    Create,
    UpdateSpec,
    UpdateStatus,
    UpdateMetadata,
    UpdateFinalizers,
    Delete,
}

/// One exact permission. An absent name matches only a nameless request and is
/// never a wildcard.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExactPermission {
    resource_type: ResourceTypeName,
    resource_name: Option<ResourceName>,
    verb: CoreResourceVerb,
}

impl ExactPermission {
    /// Construct an exact named or nameless permission.
    pub const fn new(
        resource_type: ResourceTypeName,
        resource_name: Option<ResourceName>,
        verb: CoreResourceVerb,
    ) -> Self {
        Self {
            resource_type,
            resource_name,
            verb,
        }
    }
}

impl core::fmt::Debug for ExactPermission {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ExactPermission")
            .field("has_resource_type", &true)
            .field("has_resource_name", &self.resource_name.is_some())
            .field("verb", &self.verb)
            .finish()
    }
}

/// Exact authenticated subject and session constraints from committed policy.
#[derive(Clone, PartialEq, Eq)]
pub struct SubjectPolicy {
    subject_ref: ResourceRef,
    subject_uid: ResourceUid,
    zone_ref: ResourceRef,
    evidence_class: EvidenceClass,
    provider_ref: Option<ResourceRef>,
    process_ref: Option<ResourceRef>,
    controller_generation: Option<ControllerGeneration>,
    provider_generation: Option<ResourceGeneration>,
    session_purpose: SessionPurpose,
    service: ServiceName,
    schema_fingerprint: SchemaFingerprint,
    permissions: BTreeSet<ExactPermission>,
}

impl SubjectPolicy {
    /// Construct a committed exact binding. Empty permission sets fail closed.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        subject_ref: ResourceRef,
        subject_uid: ResourceUid,
        zone_ref: ResourceRef,
        evidence_class: EvidenceClass,
        provider_ref: Option<ResourceRef>,
        process_ref: Option<ResourceRef>,
        controller_generation: Option<ControllerGeneration>,
        provider_generation: Option<ResourceGeneration>,
        session_purpose: SessionPurpose,
        service: ServiceName,
        schema_fingerprint: SchemaFingerprint,
        permissions: impl IntoIterator<Item = ExactPermission>,
    ) -> Result<Self, AuthorizationError> {
        let permissions: BTreeSet<_> = permissions.into_iter().collect();
        if zone_ref.resource_type().as_str() != "Zone" || permissions.is_empty() {
            return Err(AuthorizationError::PolicyInvalid);
        }
        Ok(Self {
            subject_ref,
            subject_uid,
            zone_ref,
            evidence_class,
            provider_ref,
            process_ref,
            controller_generation,
            provider_generation,
            session_purpose,
            service,
            schema_fingerprint,
            permissions,
        })
    }

    fn matches(&self, context: &AuthenticatedSubjectContext) -> bool {
        self.subject_ref == *context.subject_ref()
            && self.subject_uid == *context.subject_uid()
            && self.zone_ref == *context.zone_ref()
            && self.evidence_class == context.evidence_class()
            && self.provider_ref.as_ref() == context.provider_ref()
            && self.process_ref.as_ref() == context.process_ref()
            && self.controller_generation == context.controller_generation()
            && self.provider_generation == context.provider_generation()
            && &self.session_purpose == context.session_purpose()
            && &self.service == context.service()
            && &self.schema_fingerprint == context.schema_fingerprint()
            && context.transport_binding().locality() == Locality::Local
    }
}

impl core::fmt::Debug for SubjectPolicy {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("SubjectPolicy")
            .field("evidence_class", &self.evidence_class)
            .field("permission_count", &self.permissions.len())
            .field("has_provider", &self.provider_ref.is_some())
            .field("has_process", &self.process_ref.is_some())
            .finish_non_exhaustive()
    }
}

/// One exact authorization request.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationRequest {
    permission: ExactPermission,
}

impl AuthorizationRequest {
    /// Request one exact named or nameless operation.
    pub const fn new(permission: ExactPermission) -> Self {
        Self { permission }
    }
}

impl core::fmt::Debug for AuthorizationRequest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AuthorizationRequest")
            .field("has_resource_type", &true)
            .field(
                "has_resource_name",
                &self.permission.resource_name.is_some(),
            )
            .field("verb", &self.permission.verb)
            .finish()
    }
}

/// Closed authorization refusal reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationError {
    PolicyUnavailable,
    PolicyInvalid,
    PolicyRevisionMismatch,
    SubjectNotBound,
    SessionMismatch,
    PermissionDenied,
}

impl AuthorizationError {
    /// Return the stable, redacted reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::PolicyUnavailable => "authorization-policy-unavailable",
            Self::PolicyInvalid => "authorization-policy-invalid",
            Self::PolicyRevisionMismatch => "authorization-policy-revision-mismatch",
            Self::SubjectNotBound => "authorization-subject-not-bound",
            Self::SessionMismatch => "authorization-session-mismatch",
            Self::PermissionDenied => "authorization-denied",
        }
    }
}

impl core::fmt::Display for AuthorizationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AuthorizationError {}

/// Revision-bound exact authorization index.
#[derive(Default)]
pub struct AuthorizationHandler {
    policy_revision: u64,
    policies: BTreeMap<(ResourceRef, ResourceUid), SubjectPolicy>,
}

impl core::fmt::Debug for AuthorizationHandler {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AuthorizationHandler")
            .field("policy_revision", &self.policy_revision)
            .field("subject_count", &self.policies.len())
            .finish()
    }
}

impl AuthorizationHandler {
    /// Replace the complete index after the matching policy commit.
    pub fn publish_after_commit(
        &mut self,
        policy_revision: u64,
        policies: impl IntoIterator<Item = SubjectPolicy>,
    ) -> Result<(), AuthorizationError> {
        if policy_revision == 0 || policy_revision <= self.policy_revision {
            return Err(AuthorizationError::PolicyRevisionMismatch);
        }
        let mut replacement = BTreeMap::new();
        for policy in policies {
            let key = (policy.subject_ref.clone(), policy.subject_uid.clone());
            if replacement.insert(key, policy).is_some() {
                return Err(AuthorizationError::PolicyInvalid);
            }
        }
        if replacement.is_empty() {
            return Err(AuthorizationError::PolicyInvalid);
        }
        self.policy_revision = policy_revision;
        self.policies = replacement;
        Ok(())
    }

    /// Evaluate a request against authenticated evidence and the exact revision.
    pub fn authorize(
        &self,
        subject: &CoreAuthorizationSubject<'_>,
        policy_revision: u64,
        request: &AuthorizationRequest,
    ) -> Result<(), AuthorizationError> {
        let context = subject.context;
        if self.policy_revision == 0 {
            return Err(AuthorizationError::PolicyUnavailable);
        }
        if policy_revision != self.policy_revision {
            return Err(AuthorizationError::PolicyRevisionMismatch);
        }
        let policy = self
            .policies
            .get(&(context.subject_ref().clone(), context.subject_uid().clone()))
            .ok_or(AuthorizationError::SubjectNotBound)?;
        if !policy.matches(context) {
            return Err(AuthorizationError::SessionMismatch);
        }
        if !policy.permissions.contains(&request.permission) {
            return Err(AuthorizationError::PermissionDenied);
        }
        Ok(())
    }

    /// Invalidate all authority immediately when policy state is unavailable.
    pub fn invalidate(&mut self) {
        self.policy_revision = 0;
        self.policies.clear();
    }

    /// Return the active committed policy revision, or zero when unavailable.
    pub const fn policy_revision(&self) -> u64 {
        self.policy_revision
    }
}

#[cfg(test)]
mod tests {
    use d2b_contracts::v3::{
        BindingDigest, ReconnectGeneration, SchemaFingerprint, ServiceName, SessionBinding,
        SessionPurpose, TranscriptHash, TransportBinding,
    };

    use super::*;

    const SUBJECT_UID: &str = "123e4567-e89b-42d3-a456-426614174000";

    fn digest() -> String {
        format!("sha256:{}", "0".repeat(64))
    }

    fn permission(name: Option<&str>, verb: CoreResourceVerb) -> ExactPermission {
        ExactPermission::new(
            ResourceTypeName::parse("Provider").unwrap(),
            name.map(|value| ResourceName::parse(value).unwrap()),
            verb,
        )
    }

    fn context(locality: Locality, uid: &str) -> AuthenticatedSubjectContext {
        AuthenticatedSubjectContext::new(
            ResourceRef::parse("Provider/system-core").unwrap(),
            ResourceUid::parse(uid).unwrap(),
            ResourceRef::parse("Zone/work").unwrap(),
            EvidenceClass::UnixPeer,
            SessionPurpose::parse("resource-api").unwrap(),
            ServiceName::parse("d2b.resource.v3").unwrap(),
            SessionBinding::new(
                SchemaFingerprint::parse(digest()).unwrap(),
                TransportBinding::new(locality, BindingDigest::parse(digest()).unwrap()),
                ReconnectGeneration::new(1).unwrap(),
                TranscriptHash::parse_hex(&"00".repeat(32)).unwrap(),
            ),
        )
        .with_provider_ref(ResourceRef::parse("Provider/system-core").unwrap())
        .with_process_ref(ResourceRef::parse("Process/core-controller").unwrap())
        .with_controller_generation(ControllerGeneration::new(1).unwrap())
        .with_provider_generation(ResourceGeneration::new(1).unwrap())
    }

    fn policy(uid: &str) -> SubjectPolicy {
        SubjectPolicy::new(
            ResourceRef::parse("Provider/system-core").unwrap(),
            ResourceUid::parse(uid).unwrap(),
            ResourceRef::parse("Zone/work").unwrap(),
            EvidenceClass::UnixPeer,
            Some(ResourceRef::parse("Provider/system-core").unwrap()),
            Some(ResourceRef::parse("Process/core-controller").unwrap()),
            Some(ControllerGeneration::new(1).unwrap()),
            Some(ResourceGeneration::new(1).unwrap()),
            SessionPurpose::parse("resource-api").unwrap(),
            ServiceName::parse("d2b.resource.v3").unwrap(),
            SchemaFingerprint::parse(digest()).unwrap(),
            [permission(Some("system-core"), CoreResourceVerb::Get)],
        )
        .unwrap()
    }

    #[test]
    fn exact_authenticated_binding_and_permission_are_allowed() {
        let mut handler = AuthorizationHandler::default();
        handler
            .publish_after_commit(7, [policy(SUBJECT_UID)])
            .unwrap();
        let context = context(Locality::Local, SUBJECT_UID);
        assert_eq!(
            handler.authorize(
                &CoreAuthorizationSubject::from_authenticated_session(&context),
                7,
                &AuthorizationRequest::new(permission(Some("system-core"), CoreResourceVerb::Get,)),
            ),
            Ok(())
        );
    }

    #[test]
    fn unavailable_and_stale_policy_are_rejected() {
        let mut handler = AuthorizationHandler::default();
        let request = AuthorizationRequest::new(permission(None, CoreResourceVerb::List));
        let context = context(Locality::Local, SUBJECT_UID);
        assert_eq!(
            handler.authorize(
                &CoreAuthorizationSubject::from_authenticated_session(&context),
                1,
                &request,
            ),
            Err(AuthorizationError::PolicyUnavailable)
        );
        handler
            .publish_after_commit(7, [policy(SUBJECT_UID)])
            .unwrap();
        assert_eq!(
            handler.authorize(
                &CoreAuthorizationSubject::from_authenticated_session(&context),
                6,
                &request,
            ),
            Err(AuthorizationError::PolicyRevisionMismatch)
        );
    }

    #[test]
    fn invalid_policy_publication_is_rejected_atomically() {
        let mut handler = AuthorizationHandler::default();
        assert_eq!(
            handler.publish_after_commit(1, []),
            Err(AuthorizationError::PolicyInvalid)
        );
        assert_eq!(handler.policy_revision(), 0);

        let policy = policy(SUBJECT_UID);
        assert_eq!(
            handler.publish_after_commit(1, [policy.clone(), policy]),
            Err(AuthorizationError::PolicyInvalid)
        );
        assert_eq!(handler.policy_revision(), 0);
    }

    #[test]
    fn caller_identity_and_remote_transport_cannot_supply_authority() {
        let mut handler = AuthorizationHandler::default();
        handler
            .publish_after_commit(7, [policy(SUBJECT_UID)])
            .unwrap();
        let request =
            AuthorizationRequest::new(permission(Some("system-core"), CoreResourceVerb::Get));
        let unbound = context(Locality::Local, "123e4567-e89b-42d3-a456-426614174001");
        assert_eq!(
            handler.authorize(
                &CoreAuthorizationSubject::from_authenticated_session(&unbound),
                7,
                &request,
            ),
            Err(AuthorizationError::SubjectNotBound)
        );
        let remote = context(Locality::Remote, SUBJECT_UID);
        assert_eq!(
            handler.authorize(
                &CoreAuthorizationSubject::from_authenticated_session(&remote),
                7,
                &request,
            ),
            Err(AuthorizationError::SessionMismatch)
        );
    }

    #[test]
    fn wrong_name_verb_and_nameless_fallback_are_all_denied() {
        let mut handler = AuthorizationHandler::default();
        handler
            .publish_after_commit(7, [policy(SUBJECT_UID)])
            .unwrap();
        for denied in [
            permission(Some("other"), CoreResourceVerb::Get),
            permission(Some("system-core"), CoreResourceVerb::UpdateSpec),
            permission(None, CoreResourceVerb::Get),
        ] {
            let context = context(Locality::Local, SUBJECT_UID);
            assert_eq!(
                handler.authorize(
                    &CoreAuthorizationSubject::from_authenticated_session(&context),
                    7,
                    &AuthorizationRequest::new(denied),
                ),
                Err(AuthorizationError::PermissionDenied)
            );
        }
    }

    #[test]
    fn diagnostics_never_render_subject_or_resource_names() {
        let policy = policy(SUBJECT_UID);
        let rendered = format!("{policy:?}");
        assert!(!rendered.contains("system-core"));
        assert!(!rendered.contains(SUBJECT_UID));
    }
}
