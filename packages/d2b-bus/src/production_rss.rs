//! Fixed authenticated bus harness used by the production RSS fixture.
//!
//! The harness has no caller-controlled identity or route inputs. It installs
//! one fixed Zone-local caller and one fixed Provider endpoint through the
//! registrar's existing registration authority, then exposes only the
//! resulting route and ingress to the fixture.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use d2b_contracts::v3::{
    AuthenticatedSubjectContext, BindingDigest, ConfigurationGeneration, ControllerGeneration,
    EvidenceClass, Locality, ReconnectGeneration, ResourceGeneration, ResourceRef,
    ResourceTypeName, ResourceUid, SchemaFingerprint, ServiceName, SessionBinding, SessionPurpose,
    TranscriptHash, TransportBinding, ZoneId, ZoneRevision,
};
use d2b_resource_api::authz::{
    ApiCatalog, BindingScope, BootstrapPhase, BoundSubject, CompiledRole, CompiledRoleBinding,
    NativeAuthorizer, PolicyRule, PolicySet, ResourceVerb, SessionVerb,
};
use d2b_resource_store::PolicySnapshot;

use crate::{
    BusAuthorizer, BusConfig, BusEndpoint, BusError, BusIngress, BusResponse, DeliveredInvocation,
    DeliveredStream, EndpointError, ManualClock, RouteGenerations, RouteKey, RouteMember,
    RouteTarget, ZoneBus, ZoneRegistrar, registry::SessionRegistration, streams::IncomingStream,
};

const CALLER_UID: &str = "11111111-1111-4111-8111-111111111111";
const ENDPOINT_UID: &str = "22222222-2222-4222-8222-222222222222";

struct Endpoint {
    incoming: Arc<Mutex<Vec<IncomingStream>>>,
}

#[async_trait]
impl BusEndpoint for Endpoint {
    async fn invoke(&self, _request: DeliveredInvocation) -> Result<BusResponse, EndpointError> {
        Err(EndpointError::Unavailable)
    }

    async fn open_stream(&self, request: DeliveredStream) -> Result<(), EndpointError> {
        self.incoming
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request.into_incoming());
        Ok(())
    }
}

/// Existing Zone bus authority plus one fixed production-watch route.
pub struct ProductionWatchHarness {
    _bus: ZoneBus,
    _registrar: ZoneRegistrar,
    caller: BusIngress,
    _endpoint: BusIngress,
    route: RouteKey,
    endpoint: Arc<Endpoint>,
}

impl core::fmt::Debug for ProductionWatchHarness {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ProductionWatchHarness(<redacted>)")
    }
}

impl ProductionWatchHarness {
    /// Install the fixed authenticated fixture route with caller-supplied
    /// transport limits only.
    pub fn new(config: BusConfig) -> Result<Self, BusError> {
        let zone = ZoneId::parse("dev").expect("fixed Zone");
        let schema = fingerprint('1');
        let generations = RouteGenerations::new(
            Some(ResourceGeneration::new(2).expect("fixed generation")),
            Some(ControllerGeneration::new(3).expect("fixed generation")),
            ReconnectGeneration::new(1).expect("fixed generation"),
        );
        let route = RouteKey::new(
            zone.clone(),
            ServiceName::parse("d2b.resource.v3").expect("fixed service"),
            RouteMember::stream("ResourceService/Watch").expect("fixed route member"),
            RouteTarget::provider(
                ResourceRef::parse("Provider/system-core").expect("fixed target"),
            )
            .expect("fixed target kind"),
            schema.clone(),
            generations,
        );
        let caller = context(
            "User/alice",
            CALLER_UID,
            schema.clone(),
            generations,
            Locality::Local,
            EvidenceClass::UnixPeer,
        );
        let endpoint_context = context(
            "Provider/system-core",
            ENDPOINT_UID,
            schema,
            generations,
            Locality::Local,
            EvidenceClass::EnrolledKk,
        );
        let subjects = vec![bound_subject(&caller), bound_subject(&endpoint_context)];
        let policy = policy(&subjects);
        let native =
            NativeAuthorizer::new(ApiCatalog::standard(), Some(policy)).expect("fixed policy");
        let authorizer = BusAuthorizer::new(native, state()).expect("fixed bus authorizer");
        let clock = Arc::new(ManualClock::new(1));
        let (bus, mut registrar) = ZoneBus::with_clock(zone, authorizer, config, clock)?;
        let incoming = Arc::new(Mutex::new(Vec::new()));
        let endpoint = Arc::new(Endpoint {
            incoming: Arc::clone(&incoming),
        });
        let endpoint_ingress = registrar.register(SessionRegistration::new(
            endpoint_context,
            vec![route.clone()],
            endpoint.clone(),
        ))?;
        let caller_ingress = registrar.register(SessionRegistration::new(
            caller,
            Vec::new(),
            endpoint.clone(),
        ))?;
        Ok(Self {
            _bus: bus,
            _registrar: registrar,
            caller: caller_ingress,
            _endpoint: endpoint_ingress,
            route,
            endpoint,
        })
    }

    /// Borrow the fixed caller ingress.
    pub const fn caller(&self) -> &BusIngress {
        &self.caller
    }

    /// Borrow the fixed ResourceService/Watch route.
    pub const fn route(&self) -> &RouteKey {
        &self.route
    }

    /// Take the next controller-side named stream.
    pub fn take_incoming(&self) -> Option<IncomingStream> {
        self.endpoint
            .incoming
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop()
    }
}

fn context(
    subject_ref: &str,
    uid: &str,
    schema: SchemaFingerprint,
    generations: RouteGenerations,
    locality: Locality,
    evidence: EvidenceClass,
) -> AuthenticatedSubjectContext {
    let provider_generation = generations.provider().expect("fixed provider generation");
    let controller_generation = generations
        .controller()
        .expect("fixed controller generation");
    AuthenticatedSubjectContext::new(
        ResourceRef::parse(subject_ref).expect("fixed subject"),
        ResourceUid::parse(uid).expect("fixed subject UID"),
        ResourceRef::parse("Zone/dev").expect("fixed Zone ref"),
        evidence,
        SessionPurpose::parse("zone-bus").expect("fixed purpose"),
        ServiceName::parse("d2b.resource.v3").expect("fixed service"),
        SessionBinding::new(
            schema,
            TransportBinding::new(locality, digest('2')),
            generations.session(),
            TranscriptHash::from_bytes([3; 32]),
        ),
    )
    .with_provider_ref(ResourceRef::parse("Provider/system-core").expect("fixed Provider ref"))
    .with_provider_generation(provider_generation)
    .with_controller_generation(controller_generation)
}

fn state() -> d2b_resource_api::authz::AuthorizationState {
    d2b_resource_api::authz::AuthorizationState {
        snapshot: PolicySnapshot {
            policy_revision: 1,
            api_catalog_revision: 1,
            active_configuration_revision: ConfigurationGeneration::new(1)
                .expect("fixed configuration generation"),
            controller_generation: Some(ControllerGeneration::new(3).expect("fixed generation")),
        },
        zone_policy_revision: ZoneRevision::new(1),
        bootstrap_phase: BootstrapPhase::Disabled,
        now_tick: 1,
    }
}

fn policy(subjects: &[BoundSubject]) -> PolicySet {
    let catalog = ApiCatalog::standard();
    let rule = PolicyRule::new(
        &catalog,
        [ResourceTypeName::parse("Host").expect("fixed ResourceType")],
        [ResourceVerb::Watch],
        [SessionVerb::Connect, SessionVerb::OpenStream],
        [],
        [],
        [ZoneId::parse("dev").expect("fixed Zone")],
        [],
    )
    .expect("fixed policy rule");
    let role = CompiledRole::new(
        ResourceRef::parse("Role/production-watch").expect("fixed role"),
        vec![rule],
    )
    .expect("fixed role");
    let binding = CompiledRoleBinding::new(
        role.role_ref.clone(),
        subjects.iter().cloned(),
        BindingScope::default(),
        d2b_resource_api::authz::RelayGrantAuthority::None,
    )
    .expect("fixed role binding");
    PolicySet::new(&catalog, 1, vec![role], vec![binding]).expect("fixed policy")
}

fn bound_subject(context: &AuthenticatedSubjectContext) -> BoundSubject {
    BoundSubject {
        subject_ref: context.subject_ref().clone(),
        subject_uid: context.subject_uid().clone(),
    }
}

fn fingerprint(value: char) -> SchemaFingerprint {
    SchemaFingerprint::parse(format!("sha256:{}", value.to_string().repeat(64)))
        .expect("fixed schema fingerprint")
}

fn digest(value: char) -> BindingDigest {
    BindingDigest::parse(format!("sha256:{}", value.to_string().repeat(64)))
        .expect("fixed binding digest")
}
