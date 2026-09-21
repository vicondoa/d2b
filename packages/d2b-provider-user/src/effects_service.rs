//! The provider-owned implementation of the User family's driver effects
//! (U5): the family serves its effects from this crate instead of a
//! daemon-built port.
//!
//! Two surfaces share one implementation value:
//!
//! - the driver's typed seam, [`UserDriverEffects`], which the family's
//!   driver holds (the factory builds it from the same facets). Its one
//!   method (`observe_user`) runs the bounded local-account probe through
//!   the preserved `UserReconciler`, exactly as the daemon-side adapter
//!   did: an account the machine does not resolve reports the ordinary
//!   `Pending`/`Absent` status rather than failing, and a discovery that
//!   cannot complete fails the call the way the driver classifies today
//!   (retryable `system-core-user-discovery-failed`);
//! - the declared zone-plane service [`USER_EFFECTS_SERVICE`], hosted per
//!   zone by the daemon through [`UserEffectsServiceFactory`]. Its one
//!   method (`inspect-user`) answers the family's bounded discovery
//!   observation for one declared User identity - the same probe the
//!   driver effects reconcile over.
//!
//! Everything the effects read crosses the provider boundary as declared
//! facets ([`crate::facets`]); every probe input is host state this crate
//! reads itself. Nothing here names a daemon state type.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::CanonicalJsonObject;
use d2b_contracts_resource::v3::user::{OsUsername, UserSpec, USER_RESOURCE_TYPE};
use d2b_provider_system_core::{UserDiscoveryEffectPort, UserReconciler, UserStatusReport};
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
};
use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::driver::UserDriverEffects;
use crate::facets::UserEffectFacets;

/// The User family's declared effects service.
///
/// One zone-plane method, `inspect-user`: it answers the family's bounded
/// discovery observation for one declared User identity - the reference and
/// OS username the payload names, resolved through the crate's own
/// local-account probe over the preserved `UserReconciler` (the same probe
/// and classification the driver's `observe_user` reconciles over). The
/// observation is served from the crate's own probe, so it proves the
/// family's probe runs inside the owning crate (U5); a discovery that
/// cannot complete refuses with its own closed code instead of answering a
/// half-built report.
///
/// The service is declared on the `User` descriptor alone; the family's
/// driver effects (the typed seam) stay the driver's object, not a hosted
/// method surface.
pub const USER_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "user.d2bus.org/effects",
    methods: &[ServiceMethod::zone_plane("inspect-user")],
    attach_kinds: &[],
    streams: &[],
    endpoint_policy: None,
};

/// The one `inspect-user` response payload: the family's bounded discovery
/// observation. The report is the reconciler's typed projection serialized
/// through its own serde contract, extended with the family's identity
/// surface; the refusal is unreachable and names its own code.
fn inspect_user_response(
    username: &OsUsername,
    report: &UserStatusReport,
) -> Result<EffectResponse, EffectServiceError> {
    let declined = |reason: &'static str| EffectServiceError::Declined {
        service: USER_EFFECTS_SERVICE.id.to_owned(),
        reason: reason.to_owned(),
    };
    let mut payload =
        serde_json::to_value(report).map_err(|_| declined("inspect-user-response-invalid"))?;
    let Some(object) = payload.as_object_mut() else {
        return Err(declined("inspect-user-response-invalid"));
    };
    object.insert("family".to_owned(), serde_json::Value::String("user".to_owned()));
    object.insert("resourceType".to_owned(), serde_json::Value::String("User".to_owned()));
    object.insert("username".to_owned(), serde_json::Value::String(username.as_str().to_owned()));
    let payload = serde_json::from_value::<CanonicalJsonObject>(payload)
        .map_err(|_| declined("inspect-user-response-invalid"))?;
    Ok(EffectResponse::new(payload))
}

/// The request contract the `inspect-user` method serves: one declared User
/// identity - the row reference and the OS username the probe resolves.
/// Both ride the canonical payload and are validated through the closed
/// contract types, never a caller-supplied path or handle, so a malformed
/// identity refuses before any probe runs.
struct InspectUserRequest {
    user_ref: d2b_contracts_resource::v3::ResourceRef,
    username: OsUsername,
}

impl InspectUserRequest {
    /// Decode and validate one request from the canonical payload.
    fn parse(payload: &CanonicalJsonObject) -> Result<Self, EffectServiceError> {
        let declined = |reason: &'static str| EffectServiceError::Declined {
            service: USER_EFFECTS_SERVICE.id.to_owned(),
            reason: reason.to_owned(),
        };
        let reference = match payload.get("userRef") {
            Some(d2b_contracts_resource::v3::CanonicalJsonValue::String(spelling)) => {
                d2b_contracts_resource::v3::ResourceRef::parse(spelling.as_str())
                    .map_err(|_| declined("inspect-user-request-invalid"))?
            }
            _ => return Err(declined("inspect-user-request-invalid")),
        };
        if reference.resource_type().as_str() != USER_RESOURCE_TYPE {
            return Err(declined("inspect-user-request-invalid"));
        }
        let username = match payload.get("osUsername") {
            Some(d2b_contracts_resource::v3::CanonicalJsonValue::String(value)) => {
                OsUsername::parse(value.as_str()).map_err(|_| declined("inspect-user-request-invalid"))?
            }
            _ => return Err(declined("inspect-user-request-invalid")),
        };
        Ok(Self { user_ref: reference, username })
    }
}

/// Serve the `inspect-user` method: run the family's bounded probe for the
/// declared identity and answer its discovery observation. A discovery that
/// cannot complete refuses with its own closed code instead of answering a
/// half-built report; the driver seam keeps the same refusal mapping it
/// always had (the row's own reconcile classifies it).
async fn serve_inspect_user(
    reconciler: &UserReconciler<Arc<dyn UserDiscoveryEffectPort>>,
    payload: &CanonicalJsonObject,
) -> Result<EffectResponse, EffectServiceError> {
    let declined = |reason: &'static str| EffectServiceError::Declined {
        service: USER_EFFECTS_SERVICE.id.to_owned(),
        reason: reason.to_owned(),
    };
    let request = InspectUserRequest::parse(payload)?;
    let spec = UserSpec::minimal(request.username.clone());
    let report = reconciler
        .reconcile(&request.user_ref, &spec)
        .await
        .map_err(|_| declined("inspect-user-discovery-failed"))?;
    inspect_user_response(&request.username, &report)
}

/// The provider-owned User effects (U5), built from the daemon-supplied
/// facets.
///
/// One value serves both the driver's typed seam and the declared hosted
/// service: the factory constructs it from the same [`UserEffectFacets`]
/// the composition root supplies, so the hosted surface and the driver
/// observe the same probe. The preserved `UserReconciler` over the
/// facet-carried [`UserDiscoveryEffectPort`] is the report and
/// classification authority both surfaces reconcile over; the crate's
/// production probe implements that port, and the scripted probe scripts
/// it, so composition and scripting cross the same boundary.
pub struct UserEffectsService {
    reconciler: UserReconciler<Arc<dyn UserDiscoveryEffectPort>>,
}

impl UserEffectsService {
    /// Build the effects over one zone's daemon-supplied facet set (R2):
    /// the facet-carried probe rides the seam the composition root
    /// supplies, never a daemon handle.
    pub fn new(facets: UserEffectFacets) -> Self {
        Self {
            reconciler: UserReconciler::new(facets.probe),
        }
    }
}

#[async_trait]
impl UserDriverEffects for UserEffectsService {
    async fn observe_user(
        &self,
        user_ref: &d2b_contracts_resource::v3::ResourceRef,
        spec: &d2b_contracts_resource::v3::user::UserSpec,
    ) -> Result<UserStatusReport, String> {
        self.reconciler
            .reconcile(user_ref, spec)
            .await
            .map_err(|error| error.to_string())
    }
}

#[async_trait]
impl EffectService for UserEffectsService {
    async fn handle(
        &self,
        invocation: ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        // The declaration's method gates admission at the hosting side; the
        // service serves its one declared zone-plane method from the
        // family's bounded probe.
        serve_inspect_user(&self.reconciler, invocation.payload).await
    }
}

/// The composition-root factory that hosts the User effects service in one
/// zone (R5): the daemon registers one per zone, carrying that zone's facet
/// set, and the host rebuilds the service from it on respawn.
pub struct UserEffectsServiceFactory {
    facets: UserEffectFacets,
}

impl UserEffectsServiceFactory {
    /// Build the factory from one zone's facet set.
    pub fn new(facets: UserEffectFacets) -> Self {
        Self { facets }
    }
}

impl EffectServiceFactory for UserEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(UserEffectsService::new(self.facets.clone()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    use d2b_contracts_resource::v3::{
        CanonicalJsonValue, ResourcePhase, ResourceRef,
        user::{OsUsername, UserSpec},
    };
    use d2b_provider_system_core::{
        SystemCoreError, UserDiscoveryCondition, UserIdentityDigest,
    };
    use crate::test_support::{ScriptedProbe, recording_facets};

    fn user_ref() -> ResourceRef {
        ResourceRef::parse("User/alice").expect("user ref")
    }

    fn minimal_spec() -> UserSpec {
        UserSpec::minimal(OsUsername::parse("alice").expect("username"))
    }

    /// The service over a scripted probe: the same `UserDiscoveryEffectPort`
    /// boundary the production composition root composes crosses, scripted
    /// through the declared facet set.
    fn service(probe: Arc<ScriptedProbe>) -> UserEffectsService {
        UserEffectsService::new(recording_facets(probe))
    }

    // -- driver seam: observation parity -------------------------------------

    /// A user row reconciles from the probe inside the provider crate and
    /// publishes the same observation as before: the report is the
    /// reconciler's typed projection over the probe's bounded discovery.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn observe_user_publishes_the_discovery_observation() {
        let probe = ScriptedProbe::new();
        let report = service(probe.clone())
            .observe_user(&user_ref(), &minimal_spec())
            .await
            .expect("observe succeeds");
        assert_eq!(report.user_ref, user_ref());
        assert_eq!(report.provider, "system-core");
        assert_eq!(report.phase, ResourcePhase::Ready);
        assert_eq!(report.discovery, UserDiscoveryCondition::Discovered);
        assert_eq!(
            report.identity,
            Some(UserIdentityDigest::from_bytes([0x5a; 32]))
        );
        assert_eq!(
            probe.discovered_names(),
            vec![OsUsername::parse("alice").expect("username")],
            "the probe is run exactly as the preserved reconciler does"
        );
    }

    /// Refusal parity: a row whose account is absent does not fail - the
    /// reconciler reports the ordinary `Pending`/`Absent` status, exactly
    /// the classification the row publishes today, and the driver answers
    /// `RetryScheduled` so the pass re-checks on its cadence.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn observe_user_reports_absent_when_the_account_is_absent() {
        let probe = ScriptedProbe::new();
        probe.set_absent(true);
        let report = service(probe)
            .observe_user(&user_ref(), &minimal_spec())
            .await
            .expect("absent is an ordinary status, not a failure");
        assert_eq!(report.phase, ResourcePhase::Pending);
        assert_eq!(report.discovery, UserDiscoveryCondition::Absent);
        assert_eq!(report.identity, None);
    }

    /// Error parity: a discovery that cannot complete fails the call the
    /// way it always did - the driver maps the error to the retryable
    /// `system-core-user-discovery-failed` classification.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn observe_user_fails_when_the_probe_cannot_complete() {
        let probe = ScriptedProbe::new();
        probe.set_failing(true);
        let error = service(probe)
            .observe_user(&user_ref(), &minimal_spec())
            .await
            .expect_err("the discovery failure surfaces");
        assert_eq!(
            error,
            SystemCoreError::DiscoveryUnavailable.to_string(),
            "the failure is the discovery classification the driver maps to the retryable \
             `system-core-user-discovery-failed` kind"
        );
    }

    // -- hosted surface: inspect-user ----------------------------------------

    fn canonical(payload: serde_json::Value) -> CanonicalJsonObject {
        serde_json::from_value(payload).expect("canonical payload")
    }

    fn invocation<'a>(
        payload: &'a CanonicalJsonObject,
        resources: &'a mut d2b_resource_runtime::context::ServiceResourceContext,
    ) -> ServiceInvocation<'a> {
        ServiceInvocation {
            zone: "work",
            invocation_id: "invocation-u5",
            payload,
            resources,
            state_cells: &[],
            kernel: None,
            request_fds: &[],
            response_fds: USER_EFFECTS_SERVICE.methods[0].response_fds,
            payload_schema: None,
        }
    }

    fn string_field(payload: &CanonicalJsonObject, key: &str) -> String {
        match payload.get(key) {
            Some(CanonicalJsonValue::String(value)) => value.clone(),
            other => panic!("field {key} is not a canonical string: {other:?}"),
        }
    }

    /// The hosted `inspect-user` method answers the family's bounded
    /// discovery observation for the declared identity over the
    /// facet-carried probe.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn inspect_user_answers_the_bounded_discovery_observation() {
        let service = service(ScriptedProbe::new());
        let payload = canonical(serde_json::json!({
            "userRef": "User/alice",
            "osUsername": "alice",
        }));
        let mut resources = d2b_resource_runtime::context::ServiceResourceContext::fail_closed();
        let response = service
            .handle(invocation(&payload, &mut resources))
            .await
            .expect("served");
        assert_eq!(string_field(&response.payload, "family"), "user");
        assert_eq!(string_field(&response.payload, "resourceType"), "User");
        assert_eq!(string_field(&response.payload, "username"), "alice");
        assert_eq!(string_field(&response.payload, "userRef"), "User/alice");
        assert_eq!(string_field(&response.payload, "provider"), "system-core");
        assert_eq!(string_field(&response.payload, "phase"), "Ready");
        assert_eq!(string_field(&response.payload, "discovery"), "discovered");
        assert_eq!(
            string_field(&response.payload, "identity"),
            UserIdentityDigest::from_bytes([0x5a; 32]).to_hex()
        );
    }

    /// The hosted `inspect-user` method reports the same absent-account
    /// classification a row would: `Pending` with the `absent` discovery
    /// condition and no identity.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn inspect_user_reports_absent_for_a_missing_account() {
        let probe = ScriptedProbe::new();
        probe.set_absent(true);
        let payload = canonical(serde_json::json!({
            "userRef": "User/alice",
            "osUsername": "alice",
        }));
        let mut resources = d2b_resource_runtime::context::ServiceResourceContext::fail_closed();
        let response = service(probe)
            .handle(invocation(&payload, &mut resources))
            .await
            .expect("served");
        assert_eq!(string_field(&response.payload, "phase"), "Pending");
        assert_eq!(string_field(&response.payload, "discovery"), "absent");
        match response.payload.get("identity") {
            Some(CanonicalJsonValue::Null) => {}
            other => panic!("identity is not null for an absent account: {other:?}"),
        }
    }

    /// A discovery that cannot complete refuses with its own closed code
    /// instead of answering a half-built report.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn inspect_user_refuses_when_the_probe_cannot_complete() {
        let probe = ScriptedProbe::new();
        probe.set_failing(true);
        let payload = canonical(serde_json::json!({
            "userRef": "User/alice",
            "osUsername": "alice",
        }));
        let mut resources = d2b_resource_runtime::context::ServiceResourceContext::fail_closed();
        let error = service(probe)
            .handle(invocation(&payload, &mut resources))
            .await
            .expect_err("refused");
        assert_eq!(
            error,
            EffectServiceError::Declined {
                service: USER_EFFECTS_SERVICE.id.to_owned(),
                reason: "inspect-user-discovery-failed".to_owned(),
            }
        );
    }

    /// A malformed identity refuses before any probe runs.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn inspect_user_refuses_a_malformed_request() {
        for payload in [
            canonical(serde_json::json!({})),
            canonical(serde_json::json!({
                "userRef": "Guest/alice",
                "osUsername": "alice",
            })),
            canonical(serde_json::json!({
                "userRef": "User/alice",
                "osUsername": "",
            })),
        ] {
            let mut resources =
                d2b_resource_runtime::context::ServiceResourceContext::fail_closed();
            let error = service(ScriptedProbe::new())
                .handle(invocation(&payload, &mut resources))
                .await
                .expect_err("refused");
            assert_eq!(
                error,
                EffectServiceError::Declined {
                    service: USER_EFFECTS_SERVICE.id.to_owned(),
                    reason: "inspect-user-request-invalid".to_owned(),
                }
            );
        }
    }

    // -- factory ---------------------------------------------------------------

    /// The composition-root factory rebuilds the same implementation value
    /// from the facet set the driver factory is built from: the built
    /// service answers `inspect-user` over the supplied facet-carried
    /// probe, with the scripted discovery observation and the requested
    /// username.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn the_factory_builds_the_service_over_the_facets() {
        let probe = ScriptedProbe::new();
        let factory = UserEffectsServiceFactory::new(recording_facets(probe.clone()));
        let service = factory.build();
        let payload = canonical(serde_json::json!({
            "userRef": "User/alice",
            "osUsername": "alice",
        }));
        let mut resources = d2b_resource_runtime::context::ServiceResourceContext::fail_closed();
        let response = service
            .handle(invocation(&payload, &mut resources))
            .await
            .expect("served");
        assert_eq!(string_field(&response.payload, "family"), "user");
        assert_eq!(string_field(&response.payload, "resourceType"), "User");
        assert_eq!(string_field(&response.payload, "username"), "alice");
        assert_eq!(string_field(&response.payload, "phase"), "Ready");
        assert_eq!(string_field(&response.payload, "discovery"), "discovered");
        assert_eq!(
            string_field(&response.payload, "identity"),
            UserIdentityDigest::from_bytes([0x5a; 32]).to_hex()
        );
        assert_eq!(
            probe.discovered_names(),
            vec![OsUsername::parse("alice").expect("username")],
            "the factory-built service reconciles over the supplied facets"
        );
    }
}
