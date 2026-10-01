//! Coverage for the Endpoint provider's `EndpointBinding` admission and exact
//! endpoint delivery (U18).
//!
//! Every case is a failure the new model exists to prevent:
//!
//! 1. **AE7** - a consumer that received one exact compositor endpoint must not
//!    be able to reach a different absolute socket, a sibling socket, or the
//!    runtime directory that happens to contain the admitted one. Those
//!    alternates are CONSTRUCTED here and each is refused by name; none of the
//!    cases asserts that a field is missing.
//! 2. A replaced socket inode must invalidate a prepared delivery until the
//!    NEW exact endpoint is prepared.
//! 3. **AE19** - an ACL mask nullified by a later mode reconciliation, and a
//!    missing ancestor traverse bit, must both leave the relationship not
//!    prepared. ACL presence is never the acceptance criterion.
//! 4. Endpoint teardown must precede producer/helper removal, and only after
//!    every consumer has detached.
//!
//! The remaining cases cover the admission refusals that make those four
//! properties reachable at all: the endpoint's own consumer policy, its own
//! attachment capacity, the execution target's child support ceiling, the
//! consumer-slot index, and the dependency fence.

use d2b_contracts_resource::v3::{
    AdmissionStage, BindingArbitration, BindingAuthorization, BindingConsumerKind,
    BindingContractError, BindingKey, BindingKind, BindingLifecycleState, BindingRealizationFacet,
    BindingSlot, BindingSupportEntry, ChildSupportCeiling,
    BoundedToken, ChildRequestDefaults, DesiredDigest, DesiredRevision, EndpointAttachmentKind,
    DefaultedSource, EndpointBindingRequest, FreshnessTuple, RefusalReason, RequestedRights,
    ResourceGeneration, ResourceRef, ResourceUid, StoreIncarnation, ZoneId,
    admit_binding_row_refs,
};
use d2b_provider_endpoint::endpoint::{
    EndpointAttachmentPolicy, EndpointClass, EndpointConsumerPolicy, EndpointLifecyclePolicy,
    EndpointLocality, EndpointOperation, EndpointSpec, EndpointTransport, EndpointVisibility,
};
use d2b_contracts_resource::v3::endpoint_binding::{
    EndpointBindingSpec, EndpointExecutionParentInput,
};
use d2b_provider_endpoint::{
    BindingReadiness, DeclaredEndpointBinding, DeliveryFenceViolation, DeliveryForm,
    EndpointAccessObservation, EndpointBindingAdmission, EndpointBindingError,
    EndpointBindingRegistry, EndpointConsumerTarget, EndpointDelivery, EndpointProvenance,
    EndpointSocketIdentity, ParentInputOutcome, canonical_binding_row, canonical_binding_rows,
    declared_delivery_form, endpoint_binding_support, endpoint_binding_support_ceiling,
    ensure_realizable, fence_delivery_environment, fence_delivery_environment_all,
    fence_delivery_payload, fence_delivery_payload_all, required_right_bits,
};

const ZONE: &str = "work";
const ENDPOINT_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const PRODUCER_UID: &str = "223e4567-e89b-42d3-a456-426614174001";
const CONSUMER_UID: &str = "323e4567-e89b-42d3-a456-426614174002";
const SECOND_UID: &str = "423e4567-e89b-42d3-a456-426614174003";
const THIRD_UID: &str = "523e4567-e89b-42d3-a456-426614174004";

/// The exact compositor socket the endpoint owner resolved privately.
const COMPOSITOR: (u64, u64) = (0xfd00, 0x5150);
/// A different socket on the same host: the sibling a consumer must not reach.
const SIBLING: (u64, u64) = (0xfd00, 0x5151);

/// The host session runtime directory, spelled the way a launch payload spells
/// it.
///
/// It is built here as a REAL path so the fence cases fence a real value; the
/// registry never sees it, and neither does any consumer.
fn runtime_dir() -> String {
    "/run/user/1000".to_owned()
}

/// An alternate absolute socket outside the admitted runtime directory.
fn alternate_absolute_socket() -> String {
    "/run/d2b/attacker.sock".to_owned()
}

/// A sibling socket directly inside the same runtime directory.
fn sibling_socket() -> String {
    format!("{}/wayland-1", runtime_dir())
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("registered resource reference")
}

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("canonical resource uid")
}

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).expect("zone")
}

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("bounded token")
}

fn generation(value: u64) -> ResourceGeneration {
    ResourceGeneration::new(value).expect("nonzero generation")
}

fn freshness(
    resource: &str,
    identity: &str,
    revision: DesiredRevision,
    payload: &str,
) -> FreshnessTuple {
    FreshnessTuple::new(
        zone(),
        StoreIncarnation::parse("store-one").expect("store incarnation"),
        reference(resource),
        uid(identity),
        revision,
        DesiredDigest::of(payload.as_bytes()),
    )
}

fn endpoint_freshness() -> FreshnessTuple {
    freshness(
        "Endpoint/compositor",
        ENDPOINT_UID,
        DesiredRevision::INITIAL,
        "endpoint",
    )
}

fn consumer_freshness(consumer: &str, identity: &str, revision: DesiredRevision) -> FreshnessTuple {
    freshness(consumer, identity, revision, "consumer")
}

fn dependencies() -> Vec<FreshnessTuple> {
    vec![
        endpoint_freshness(),
        consumer_freshness("Process/frontend", CONSUMER_UID, DesiredRevision::INITIAL),
    ]
}

fn slot(value: &str) -> BindingSlot {
    BindingSlot::parse(value).expect("bounded slot")
}

/// The endpoint the host compositor publishes, as the owner declared it.
fn compositor_spec(policy: EndpointConsumerPolicy) -> EndpointSpec {
    EndpointSpec::new(
        reference("Provider/display"),
        reference("Process/compositor"),
        EndpointClass::Service,
        EndpointTransport::Unix,
        token("compositor"),
        None,
        EndpointLocality::HostLocal,
        EndpointVisibility::Owner,
        EndpointAttachmentPolicy::new(true, 4).expect("attachment policy"),
        policy,
        EndpointLifecyclePolicy::RecycleWithProducer,
    )
    .expect("endpoint spec")
}

fn unrestricted_spec() -> EndpointSpec {
    compositor_spec(EndpointConsumerPolicy::unrestricted())
}

fn provenance_for(spec: &EndpointSpec) -> EndpointProvenance {
    EndpointProvenance::new(
        spec,
        reference("Endpoint/compositor"),
        zone(),
        uid(ENDPOINT_UID),
        generation(1),
        uid(PRODUCER_UID),
    )
    .expect("endpoint row reference")
}

fn provenance() -> EndpointProvenance {
    provenance_for(&unrestricted_spec())
}

fn request(
    consumer: &str,
    slot_name: &str,
    attachment: EndpointAttachmentKind,
) -> Result<EndpointBindingRequest, BindingContractError> {
    EndpointBindingRequest::new(
        reference("Endpoint/compositor"),
        reference(consumer),
        slot(slot_name),
        attachment,
        token("display"),
    )
}

fn target(name: &str) -> EndpointConsumerTarget {
    EndpointConsumerTarget::new(reference(name)).expect("execution target")
}

struct Inputs {
    spec: EndpointSpec,
    provenance: EndpointProvenance,
    consumer: String,
    identity: String,
    attachment: EndpointAttachmentKind,
    component: Option<&'static str>,
    deps: Vec<FreshnessTuple>,
}

impl Inputs {
    fn new(consumer: &str, identity: &str, attachment: EndpointAttachmentKind) -> Self {
        Self {
            spec: unrestricted_spec(),
            provenance: provenance(),
            consumer: consumer.to_owned(),
            identity: identity.to_owned(),
            attachment,
            component: None,
            deps: vec![
                endpoint_freshness(),
                consumer_freshness(consumer, identity, DesiredRevision::INITIAL),
            ],
        }
    }

    fn with(mut self, spec: EndpointSpec) -> Self {
        self.provenance = provenance_for(&spec);
        self.spec = spec;
        self
    }

    fn with_component(mut self, component: &'static str) -> Self {
        self.component = Some(component);
        self
    }

    fn with_dependencies(mut self, deps: Vec<FreshnessTuple>) -> Self {
        self.deps = deps;
        self
    }

    fn admit(
        &self,
        registry: &mut EndpointBindingRegistry,
    ) -> Result<BindingKey, EndpointBindingError> {
        Ok(registry
            .admit(self.build(BindingAuthorization::granted()))?
            .key()
            .clone())
    }

    fn build(&self, authorization: BindingAuthorization) -> EndpointBindingAdmission {
        EndpointBindingAdmission::new(
            zone(),
            uid(ENDPOINT_UID),
            uid(&self.identity),
            request(&self.consumer, "compositor", self.attachment).expect("endpoint binding request"),
            self.provenance.clone(),
            self.spec.clone(),
            self.component.map(token),
            target("Host/desktop"),
            authorization,
            self.deps.clone(),
        )
    }
}

/// A registry holding the compositor, its unrestricted policy, and a child
/// support ceiling for the `Host/desktop` execution target.
fn registry() -> EndpointBindingRegistry {
    registry_for(&unrestricted_spec())
}

fn registry_for(spec: &EndpointSpec) -> EndpointBindingRegistry {
    let mut registry = EndpointBindingRegistry::new(zone());
    registry
        .declare_endpoint(provenance_for(spec), spec.clone())
        .expect("declare the exact endpoint");
    registry.record_ceiling(
        target("Host/desktop"),
        endpoint_binding_support_ceiling(spec).expect("child support ceiling"),
    );
    registry
}

/// A registry with one admitted, prepared relationship ready to use.
fn prepared_registry(attachment: EndpointAttachmentKind) -> (EndpointBindingRegistry, BindingKey) {
    let mut registry = registry();
    let inputs = Inputs::new("Process/frontend", CONSUMER_UID, attachment);
    let key = inputs.admit(&mut registry).expect("admit the relationship");
    registry
        .prepare(&key, &observation(COMPOSITOR), delivery(attachment))
        .expect("prepare the exact delivery");
    (registry, key)
}

fn socket(identity: (u64, u64)) -> EndpointSocketIdentity {
    EndpointSocketIdentity::new(identity.0, identity.1)
}

/// What the kernel applies when the grant is effective: read and write on the
/// exact socket, a traverse bit on every ancestor, and no listing right on the
/// containing directory.
fn observation(identity: (u64, u64)) -> EndpointAccessObservation {
    EndpointAccessObservation::new(socket(identity), 0o7, 0o1, false, true)
}

fn delivery(attachment: EndpointAttachmentKind) -> EndpointDelivery {
    match declared_delivery_form(attachment) {
        DeliveryForm::Descriptor => EndpointDelivery::Descriptor { fd_slot: 3 },
        DeliveryForm::PrivateSocketPresentation => EndpointDelivery::PrivateSocketPresentation {
            destination_slot: token("compositor-socket"),
        },
    }
}

// ---------------------------------------------------------------------------
// Scenario 1 (AE7): the exact endpoint, and nothing else
// ---------------------------------------------------------------------------

/// AE7: an alternate absolute socket, a sibling socket, and the runtime
/// directory are all constructed here and all refused.
///
/// The consumer is handed ONE exact delivery. The payloads below are what a
/// workload would actually put in its environment to redirect access - an
/// absolute path, a sibling name, and the directory itself - and each is
/// refused by name, so none of them silently composes into a grant.
#[test]
fn alternate_absolute_socket_sibling_socket_and_runtime_directory_are_refused() {
    let delivery = delivery(EndpointAttachmentKind::Attach);
    let admitted_host_path = format!("{}/compositor-socket", runtime_dir());

    for (label, payload) in [
        ("alternate absolute socket", alternate_absolute_socket()),
        ("sibling socket", sibling_socket()),
        ("runtime directory", runtime_dir()),
        ("the admitted endpoint, spelled as a host path", admitted_host_path),
    ] {
        assert_eq!(
            fence_delivery_payload(&payload, &delivery),
            Err(DeliveryFenceViolation::AbsoluteHostPath),
            "{label} must not redirect a consumer to a host path"
        );
    }

    // The relative spellings of the same redirections are refused too, so
    // dropping the leading `/` does not help.
    for (label, payload) in [
        ("sibling socket", "wayland-1"),
        ("nested sibling", "pulse/native"),
        ("the containing directory", "."),
        ("a second destination", "wayland-0"),
    ] {
        assert_eq!(
            fence_delivery_payload(payload, &delivery),
            Err(DeliveryFenceViolation::ForeignDestination),
            "{label} must not name a destination this relationship does not own"
        );
    }

    // The one value that IS this relationship's own destination passes, so the
    // refusals above are a real fence and not a blanket denial.
    assert_eq!(
        fence_delivery_payload("compositor-socket", &delivery),
        Ok(())
    );
}

/// AE7: walking out of the one destination this relationship owns is refused.
///
/// `..` is the remaining spelling a relative path can use once the absolute
/// form is gone, so it carries its own reason rather than being folded into
/// the "foreign destination" class.
#[test]
fn a_relative_component_cannot_escape_the_admitted_destination() {
    let delivery = delivery(EndpointAttachmentKind::Attach);
    for payload in [
        "../wayland-1",
        "compositor-socket/../wayland-1",
        "./../../run/d2b/attacker.sock",
    ] {
        assert_eq!(
            fence_delivery_payload(payload, &delivery),
            Err(DeliveryFenceViolation::PathEscape),
            "{payload:?} must not escape the admitted destination"
        );
    }
}

/// AE7: a descriptor delivery has no destination, so no pathname at all is
/// honoured on it.
///
/// This is the compositor case a connect-only consumer gets. Handing it a
/// destination - its own or anyone else's - would re-open the directory
/// authority R23 removed, so every one of them is refused.
#[test]
fn a_descriptor_delivery_honours_no_pathname() {
    let delivery = delivery(EndpointAttachmentKind::Connect);
    for payload in [
        ("compositor-socket", DeliveryFenceViolation::ForeignDestination),
        ("wayland-0", DeliveryFenceViolation::ForeignDestination),
        ("compositor-socket/x", DeliveryFenceViolation::ForeignDestination),
        (
            &alternate_absolute_socket(),
            DeliveryFenceViolation::AbsoluteHostPath,
        ),
    ] {
        assert_eq!(
            fence_delivery_payload(payload.0, &delivery),
            Err(payload.1),
            "a descriptor delivery must not accept the pathname {:?}",
            payload.0
        );
    }
}

/// AE7: a whole launch payload is fenced, and the fence judges the VALUE.
///
/// Environment entries are split on the launch payload's own `KEY=VALUE`
/// shape - a structural split, not a list of recognised names - and the value
/// is then fenced. An undeclared variable naming an alternate socket is
/// therefore refused exactly as a familiar one is, which is what makes a name
/// this crate has never heard of no different from a known one.
#[test]
fn a_launch_payload_is_fenced_without_interpreting_variable_names() {
    let delivery = delivery(EndpointAttachmentKind::Attach);
    for (label, entry, expected) in [
        (
            "a recognised session variable naming the runtime directory",
            "XDG_RUNTIME_DIR=/run/user/1000",
            DeliveryFenceViolation::AbsoluteHostPath,
        ),
        (
            "a recognised audio variable naming the runtime directory",
            "PIPEWIRE_RUNTIME_DIR=/run/user/1000",
            DeliveryFenceViolation::AbsoluteHostPath,
        ),
        (
            "an undeclared variable naming an alternate socket",
            "D2B_SOMETHING_UNDECLARED=/run/d2b/attacker.sock",
            DeliveryFenceViolation::AbsoluteHostPath,
        ),
        (
            "a recognised variable naming the sibling socket",
            "WAYLAND_DISPLAY=wayland-1",
            DeliveryFenceViolation::ForeignDestination,
        ),
        (
            "a recognised variable with an empty value",
            "WAYLAND_DISPLAY=",
            DeliveryFenceViolation::ForeignDestination,
        ),
    ] {
        assert_eq!(
            fence_delivery_environment(entry, &delivery),
            Err(expected),
            "{label} must be refused"
        );
    }
    // An argv argument is a locator or it is nothing: a flag carrying the
    // admitted name is still a flag, not the delivered pathname.
    assert_eq!(
        fence_delivery_payload("--display=compositor-socket", &delivery),
        Err(DeliveryFenceViolation::ForeignDestination)
    );
    assert_eq!(
        fence_delivery_environment("WAYLAND_DISPLAY=compositor-socket", &delivery),
        Ok(()),
        "the relationship's own destination is the only value that survives"
    );
    assert_eq!(
        fence_delivery_environment_all(
            [
                "WAYLAND_DISPLAY=compositor-socket",
                "D2B_SOMETHING_UNDECLARED=/run/d2b/attacker.sock",
            ],
            &delivery
        ),
        Err(DeliveryFenceViolation::AbsoluteHostPath),
        "a second entry reaching past the destination refuses the whole block"
    );
    assert_eq!(
        fence_delivery_payload_all(["compositor-socket"], &delivery),
        Ok(()),
        "the one locator this relationship delivers is accepted"
    );
    assert_eq!(
        fence_delivery_payload_all(["compositor-socket", "wayland-1"], &delivery),
        Err(DeliveryFenceViolation::ForeignDestination),
        "a set of proposed locators must contain only the admitted one"
    );
}

/// AE7: the payload fence is not the whole property.
///
/// Even a payload the fence accepts cannot name a different inode, because
/// preparation pins the exact endpoint the owner resolved. A private-socket
/// delivery for a `connect` request - which declares a descriptor - is
/// refused outright rather than honoured as a downgrade.
#[test]
fn preparation_refuses_a_delivery_the_attachment_kind_did_not_declare() {
    let (mut registry, key) = prepared_registry(EndpointAttachmentKind::Connect);
    let downgraded = EndpointDelivery::PrivateSocketPresentation {
        destination_slot: token("compositor-socket"),
    };
    assert_eq!(
        registry.prepare(&key, &observation(COMPOSITOR), downgraded),
        Err(EndpointBindingError::UnsupportedFacet),
        "a connect request must not be downgraded into a named presentation"
    );
    assert_eq!(
        registry.binding(&key).and_then(|binding| binding.delivery()).cloned(),
        Some(EndpointDelivery::Descriptor { fd_slot: 3 }),
        "the refused delivery must not have replaced the prepared one"
    );
}

/// A request against an endpoint the registry never declared is refused
/// before anything is prepared, so the relationship can never name a socket
/// the owner did not resolve.
#[test]
fn admission_against_an_undeclared_endpoint_is_refused() {
    let mut registry = registry();
    let undeclared = EndpointProvenance::new(
        &unrestricted_spec(),
        reference("Endpoint/undeclared"),
        zone(),
        uid(SECOND_UID),
        generation(1),
        uid(PRODUCER_UID),
    )
    .expect("endpoint row reference");
    let admission = EndpointBindingAdmission::new(
        zone(),
        uid(SECOND_UID),
        uid(CONSUMER_UID),
        request("Process/frontend", "compositor", EndpointAttachmentKind::Connect)
            .expect("endpoint binding request"),
        undeclared,
        unrestricted_spec(),
        None,
        target("Host/desktop"),
        BindingAuthorization::granted(),
        dependencies(),
    );
    assert_eq!(
        registry.admit(admission),
        Err(EndpointBindingError::ForeignEndpointIdentity)
    );
}

/// A request the graph did not authorize is refused at the authorize stage,
/// before the endpoint's own policy is even consulted.
#[test]
fn an_unauthorized_request_never_reaches_the_endpoint() {
    let mut registry = registry();
    let inputs = Inputs::new("Process/frontend", CONSUMER_UID, EndpointAttachmentKind::Connect);
    let outcome = registry.admit(inputs.build(BindingAuthorization::absent()));
    let Err(EndpointBindingError::Contract(refusal)) = outcome else {
        panic!("an absent grant must refuse, got {outcome:?}");
    };
    assert_eq!(refusal.stage(), AdmissionStage::Authorize);
    assert_eq!(refusal.reason(), d2b_contracts_resource::v3::RefusalReason::IdentityNotAuthorized);
}

// ---------------------------------------------------------------------------
// Scenario 2: a replaced inode invalidates readiness
// ---------------------------------------------------------------------------

/// A replaced socket inode invalidates the prepared delivery until the new
/// exact endpoint is prepared.
///
/// A producer that recycles its socket leaves the same pathname pointing at a
/// different inode. Cached readiness must not survive that: the relationship
/// stops reporting a usable delivery, and preparing the new exact endpoint is
/// a separate, explicit act.
#[test]
fn a_replaced_inode_invalidates_readiness_until_the_new_endpoint_is_prepared() {
    let (mut registry, key) = prepared_registry(EndpointAttachmentKind::Connect);
    let prepared = registry.readiness(&key).expect("readiness");
    assert!(prepared.proves_effect());
    assert_eq!(prepared.socket(), Some(socket(COMPOSITOR)));

    assert_eq!(
        registry.observe(&key, &observation(SIBLING)),
        Err(EndpointBindingError::EndpointIdentityReplaced),
        "a replaced inode must not read as the same exact endpoint"
    );
    let after = registry.readiness(&key).expect("readiness");
    assert_eq!(
        after.socket(),
        None,
        "the pinned identity is dropped: the endpoint it named is gone"
    );
    assert_eq!(after.state(), BindingLifecycleState::Degraded);
    assert!(
        !after.proves_effect(),
        "a relationship whose inode was replaced must stop proving its effect"
    );
    assert!(!after.attached());
    assert_eq!(
        registry.observe(&key, &observation(SIBLING)),
        Err(EndpointBindingError::UnexpectedState),
        "a degraded relationship has no pinned inode left to re-check"
    );
    assert_eq!(
        registry.binding(&key).and_then(|binding| binding.delivery()),
        None,
        "no delivery exists for an endpoint that was replaced"
    );
    assert_eq!(
        registry.prepare(&key, &observation(SIBLING), delivery(EndpointAttachmentKind::Connect)),
        Err(EndpointBindingError::UnexpectedState),
        "the replaced endpoint needs a fresh admission before it can be prepared again"
    );
}

/// A restart cannot remint access from cached readiness (R41).
#[test]
fn recovery_refuses_an_admission_whose_dependency_revisions_moved() {
    let (mut registry, key) = prepared_registry(EndpointAttachmentKind::Connect);
    assert_eq!(
        registry.recover(&key, &dependencies()).expect("recover").state(),
        BindingLifecycleState::Prepared,
        "current evidence recovers the prepared state"
    );
    let moved = vec![
        endpoint_freshness(),
        consumer_freshness(
            "Process/frontend",
            CONSUMER_UID,
            DesiredRevision::INITIAL.try_next().expect("advance"),
        ),
    ];
    assert_eq!(
        registry.recover(&key, &moved),
        Err(EndpointBindingError::StaleAuthority),
        "a moved consumer revision must not be recoverable from cached readiness"
    );
}

/// A dependency fence that names only one of the two committed rows is
/// refused at admission, before any delivery could exist.
#[test]
fn a_one_sided_dependency_fence_is_refused() {
    let mut registry = registry();
    let inputs = Inputs::new("Process/frontend", CONSUMER_UID, EndpointAttachmentKind::Connect)
        .with_dependencies(vec![endpoint_freshness()]);
    assert_eq!(
        inputs.admit(&mut registry),
        Err(EndpointBindingError::StaleAuthority),
        "a fence naming only the endpoint row would keep the admission alive across a consumer change"
    );
}

// ---------------------------------------------------------------------------
// Scenario 3 (AE19): effective access, not ACL presence
// ---------------------------------------------------------------------------

/// AE19: a mask a later mode reconciliation nullified leaves the relationship
/// not prepared.
///
/// The observation carries what the KERNEL applies - the named ACL entry
/// already ANDed with the mask - and each case below is the shape a
/// `setfacl`-then-`chmod` sequence leaves behind. Reading the declared mode,
/// or the presence of the ACL entry, would pass every one of them; the
/// effective bits do not.
#[test]
fn a_nullified_acl_mask_leaves_the_relationship_not_prepared() {
    let mut registry = registry();
    let inputs = Inputs::new("Process/frontend", CONSUMER_UID, EndpointAttachmentKind::Connect);
    let key = inputs.admit(&mut registry).expect("admit");
    let exact = delivery(EndpointAttachmentKind::Connect);

    // The mask was recomputed from a 0700 mode: the named entry survives in
    // the ACL and contributes nothing.
    let nullified = EndpointAccessObservation::new(socket(COMPOSITOR), 0o0, 0o1, false, true);
    assert_eq!(
        registry.prepare(&key, &nullified, exact.clone()),
        Err(EndpointBindingError::EffectiveAccessMissing),
        "a nullified mask is not an effective grant"
    );

    // Read survived and write did not: a connect needs both, so this is a
    // short grant rather than "close enough".
    let read_only = EndpointAccessObservation::new(socket(COMPOSITOR), 0o4, 0o1, false, true);
    assert_eq!(
        registry.prepare(&key, &read_only, exact.clone()),
        Err(EndpointBindingError::EffectiveAccessMissing)
    );

    // The socket grant is fine and the ancestor traverse is not: a correct
    // grant one level below a root that has none is no grant at all.
    let no_traverse = EndpointAccessObservation::new(socket(COMPOSITOR), 0o7, 0o0, false, true);
    assert_eq!(
        registry.prepare(&key, &no_traverse, exact.clone()),
        Err(EndpointBindingError::EffectiveAccessMissing),
        "every ancestor needs an effective traverse bit"
    );

    assert_eq!(
        registry.readiness(&key).expect("readiness").state(),
        BindingLifecycleState::Admitted,
        "a refused preparation must not advance the relationship"
    );
    assert_eq!(registry.binding(&key).and_then(|b| b.socket()), None);

    // Re-establishing the effective grant is what makes it prepared, and only
    // then.
    registry
        .prepare(&key, &observation(COMPOSITOR), exact)
        .expect("re-preparing the exact endpoint");
    let ready = registry.readiness(&key).expect("readiness");
    assert!(ready.observation().prepare().is_complete());
    assert!(ready.proves_effect());
}

/// AE19: a consumer that can enumerate the socket's parent holds directory
/// authority, and the relationship refuses it.
///
/// Traverse on the ancestors is necessary; listing the containing directory is
/// the authority R23 removed. A prepared relationship therefore requires that
/// listing is NOT available, which is what makes "only that endpoint is
/// admitted" a property of the delivery rather than a hope about the host
/// posture.
#[test]
fn a_listable_container_directory_is_refused() {
    let mut registry = registry();
    let inputs = Inputs::new("Process/frontend", CONSUMER_UID, EndpointAttachmentKind::Connect);
    let key = inputs.admit(&mut registry).expect("admit");
    let listable = EndpointAccessObservation::new(socket(COMPOSITOR), 0o7, 0o7, true, true);
    assert_eq!(
        registry.prepare(
            &key,
            &listable,
            delivery(EndpointAttachmentKind::Connect)
        ),
        Err(EndpointBindingError::EffectiveAccessMissing),
        "a consumer that can list the containing directory holds directory authority"
    );
}

/// An attach to an exact endpoint that is not accepting is not prepared, even
/// though its inode and its effective ACL are correct.
#[test]
fn an_attach_to_a_quiet_endpoint_is_refused() {
    let mut registry = registry();
    let inputs = Inputs::new("Process/frontend", CONSUMER_UID, EndpointAttachmentKind::Attach);
    let key = inputs.admit(&mut registry).expect("admit");
    let quiet = EndpointAccessObservation::new(socket(COMPOSITOR), 0o7, 0o1, false, false);
    assert_eq!(
        registry.prepare(
            &key,
            &quiet,
            delivery(EndpointAttachmentKind::Attach)
        ),
        Err(EndpointBindingError::EndpointNotAccepting)
    );
}

// ---------------------------------------------------------------------------
// Scenario 4: teardown order
// ---------------------------------------------------------------------------

/// Endpoint teardown precedes producer/helper removal, and only after every
/// consumer has detached.
///
/// The whole ordered sequence is driven here, and each step refuses the one
/// that would skip ahead: the endpoint cannot be torn down while a consumer is
/// attached, and the producer cannot be retired before the endpoint is gone.
#[test]
fn endpoint_teardown_precedes_producer_removal_after_consumer_detach() {
    let (mut registry, key) = prepared_registry(EndpointAttachmentKind::Connect);
    let provenance = provenance();
    assert!(
        registry
            .readiness(&key)
            .expect("readiness")
            .socket()
            .is_some()
    );

    registry.complete(&key).expect("consumer-side completion");
    let active: BindingReadiness = registry.readiness(&key).expect("readiness");
    assert_eq!(active.state(), BindingLifecycleState::Active);
    assert_eq!(active.observation().release(), d2b_contracts_resource::v3::ReleaseOutcome::Outstanding);
    assert!(active.attached());

    assert_eq!(
        registry.teardown_endpoint(&provenance),
        Err(EndpointBindingError::ConsumerStillAttached),
        "an attached consumer refuses endpoint teardown"
    );
    assert_eq!(
        registry.retire_producer(&provenance),
        Err(EndpointBindingError::EndpointNotTornDown),
        "the producer outlives the endpoint it owns"
    );

    registry.revoke(&key).expect("block new use");
    assert_eq!(
        registry.readiness(&key).expect("readiness").state(),
        BindingLifecycleState::Revoking
    );
    registry.drain(&key).expect("drive outstanding use closed");
    assert_eq!(
        registry.readiness(&key).expect("readiness").observation().release(),
        d2b_contracts_resource::v3::ReleaseOutcome::Draining
    );

    let detached = registry.detach(&key).expect("consumer detached");
    assert!(!detached.attached());
    assert_eq!(
        registry.binding(&key).and_then(|binding| binding.delivery()),
        None,
        "a detached consumer keeps no delivery"
    );
    assert_eq!(
        registry.release(&key).expect("release").state(),
        BindingLifecycleState::Released
    );

    let teardown = registry.teardown_endpoint(&provenance).expect("endpoint teardown");
    assert_eq!(teardown.detached_consumers(), 1);
    assert!(registry.endpoint_torn_down(&provenance));
    assert_eq!(
        registry.retire_producer(&provenance),
        Ok(()),
        "producer removal follows the endpoint it owns"
    );
    assert_eq!(registry.live_bindings(&provenance), 0);
    assert_eq!(registry.retire_producer(&provenance), Err(
        EndpointBindingError::ForeignEndpointIdentity
    ));
}

/// One consumer leaving never tears the endpoint out from under another that
/// still uses it.
#[test]
fn one_consumer_leaving_never_tears_down_an_endpoint_another_still_uses() {
    let mut registry = registry();
    let first = Inputs::new("Process/frontend", CONSUMER_UID, EndpointAttachmentKind::Connect)
        .admit(&mut registry)
        .expect("first admit");
    let second = Inputs::new("Process/second", SECOND_UID, EndpointAttachmentKind::Connect)
        .admit(&mut registry)
        .expect("second admit");
    let provenance = provenance();
    for key in [&first, &second] {
        registry
            .prepare(
                key,
                &observation(COMPOSITOR),
                delivery(EndpointAttachmentKind::Connect),
            )
            .expect("prepare");
        registry.complete(key).expect("complete");
        registry.revoke(key).expect("revoke");
        registry.drain(key).expect("drain");
    }
    assert_eq!(registry.live_bindings(&provenance), 2);

    registry.detach(&first).expect("first detaches");
    registry.release(&first).expect("first releases");
    assert_eq!(registry.live_bindings(&provenance), 1);
    assert_eq!(
        registry.teardown_endpoint(&provenance),
        Err(EndpointBindingError::ConsumerStillAttached),
        "the second consumer still holds the endpoint"
    );

    registry.detach(&second).expect("second detaches");
    registry.release(&second).expect("second releases");
    assert_eq!(registry.live_bindings(&provenance), 0);
    assert!(registry.teardown_endpoint(&provenance).is_ok());
}

// ---------------------------------------------------------------------------
// The endpoint's own declaration is the admission boundary
// ---------------------------------------------------------------------------

/// The endpoint's own consumer policy decides, and an empty allowlist is the
/// deliberately unconstrained policy rather than a deny-all.
#[test]
fn the_endpoints_own_consumer_policy_decides() {
    let narrow = EndpointConsumerPolicy::new(
        vec![reference("Process/frontend")],
        vec![token("display")],
        vec![EndpointOperation::Resolve],
    )
    .expect("narrow consumer policy");
    let spec = compositor_spec(narrow);
    let mut registry = EndpointBindingRegistry::new(zone());
    registry
        .declare_endpoint(provenance_for(&spec), spec.clone())
        .expect("declare");
    // The target's ceiling admits both endpoint rights, so the refusal below
    // is the ENDPOINT's own operation policy and not the target's bound.
    registry.record_ceiling(
        target("Host/desktop"),
        ChildSupportCeiling::new(vec![BindingSupportEntry::new(
            BindingKind::Endpoint,
            vec![RequestedRights::Consume, RequestedRights::Observe],
        )
        .expect("support entry")])
        .expect("child support ceiling"),
    );

    let admitted = registry
        .admit(Inputs::new("Process/frontend", CONSUMER_UID, EndpointAttachmentKind::Connect)
            .with(spec.clone())
            .with_component("display")
            .build(BindingAuthorization::granted()))
        .expect("the allowed consumer, component, and operation are admitted");
    assert_eq!(
        admitted.component().map(BoundedToken::as_str),
        Some("display")
    );
    assert_eq!(admitted.declared_form(), DeliveryForm::Descriptor);
    assert_eq!(admitted.provenance().endpoint_uid(), &uid(ENDPOINT_UID));

    // A different consumer, a different component, and a different operation
    // are each refused by the endpoint's own policy, at the authorize stage.
    for (consumer, identity, component, attachment, expected) in [
        (
            "Process/other",
            THIRD_UID,
            "display",
            EndpointAttachmentKind::Connect,
            EndpointBindingError::ConsumerNotAllowed,
        ),
        (
            "Process/frontend",
            CONSUMER_UID,
            "record",
            EndpointAttachmentKind::Connect,
            EndpointBindingError::ComponentNotAllowed,
        ),
        (
            "Process/frontend",
            CONSUMER_UID,
            "display",
            EndpointAttachmentKind::Listen,
            EndpointBindingError::OperationNotAllowed,
        ),
    ] {
        let mut registry = EndpointBindingRegistry::new(zone());
        registry
            .declare_endpoint(provenance_for(&spec), spec.clone())
            .expect("declare");
        registry.record_ceiling(
            target("Host/desktop"),
            ChildSupportCeiling::new(vec![BindingSupportEntry::new(
                BindingKind::Endpoint,
                vec![RequestedRights::Consume, RequestedRights::Observe],
            )
            .expect("support entry")])
            .expect("child support ceiling"),
        );
        let outcome = registry.admit(
            Inputs::new(consumer, identity, attachment)
                .with(spec.clone())
                .with_component(component)
                .build(BindingAuthorization::granted()),
        );
        assert_eq!(outcome.unwrap_err(), expected, "consumer {consumer}");
        assert_eq!(expected.stage(), AdmissionStage::Authorize);
    }
}

/// An attach is refused by the endpoint's own attachment capacity, and the
/// capacity is counted on the endpoint rather than on anything the consumer
/// says.
#[test]
fn attachment_capacity_is_the_endpoints_own_ceiling() {
    let policy = EndpointConsumerPolicy::new(
        Vec::new(),
        Vec::new(),
        vec![
            EndpointOperation::Resolve,
            EndpointOperation::Attach,
            EndpointOperation::Observe,
        ],
    )
    .expect("policy");
    let spec = EndpointSpec::new(
        reference("Provider/display"),
        reference("Process/compositor"),
        EndpointClass::Service,
        EndpointTransport::Unix,
        token("compositor"),
        None,
        EndpointLocality::HostLocal,
        EndpointVisibility::Owner,
        EndpointAttachmentPolicy::new(true, 1).expect("attachment policy"),
        policy,
        EndpointLifecyclePolicy::RecycleWithProducer,
    )
    .expect("endpoint spec");
    let mut registry = registry_for(&spec);

    Inputs::new("Process/frontend", CONSUMER_UID, EndpointAttachmentKind::Attach)
        .with(spec.clone())
        .admit(&mut registry)
        .expect("the first attach fits");
    assert_eq!(
        Inputs::new("Process/second", SECOND_UID, EndpointAttachmentKind::Attach)
            .with(spec.clone())
            .admit(&mut registry),
        Err(EndpointBindingError::AttachmentRefused),
        "the endpoint's own capacity refuses the second attach"
    );
}

/// The execution target's child support ceiling bounds children and grants
/// the parent nothing; a parent's defaults shape one child and create no
/// relationship (R16, AE31, AE33).
#[test]
fn a_child_support_ceiling_creates_no_relationship() {
    let spec = unrestricted_spec();
    let provenance = provenance();
    let mut registry = registry_for(&spec);
    let ceiling = endpoint_binding_support_ceiling(&spec).expect("ceiling");
    assert!(ceiling.admits(BindingKind::Endpoint, RequestedRights::Consume));
    let defaults = ChildRequestDefaults::new(
        reference("Process/frontend"),
        DefaultedSource::new(BindingKind::Endpoint, reference("Endpoint/compositor"), None)
            .expect("defaulted source"),
    )
    .expect("child defaults");
    let target = target("Host/desktop");

    let classified = registry
        .classify_parent_input(
            target.clone(),
            &EndpointExecutionParentInput::ChildSupportCeiling(ceiling),
            None,
        )
        .expect("classify the ceiling");
    assert!(matches!(
        classified,
        ParentInputOutcome::Ceiling { admits: true }
    ));
    assert_eq!(registry.live_bindings(&provenance), 0);

    let classified = registry
        .classify_parent_input(
            target,
            &EndpointExecutionParentInput::ChildRequestDefaults(defaults),
            None,
        )
        .expect("classify the defaults");
    match classified {
        ParentInputOutcome::ChildDefault { child_ref } => {
            assert_eq!(child_ref, reference("Process/frontend"));
        }
        other => panic!("expected a child default, got {other:?}"),
    }
    assert_eq!(
        registry.live_bindings(&provenance),
        0,
        "a ceiling and a default create no binding, no reservation, and no access"
    );
}

/// A target with no recorded ceiling refuses a child request, and a ceiling
/// that omits the endpoint kind refuses it too.
#[test]
fn a_target_without_a_matching_ceiling_refuses_the_child_request() {
    let mut registry = EndpointBindingRegistry::new(zone());
    registry
        .declare_endpoint(provenance(), unrestricted_spec())
        .expect("declare");
    let inputs = Inputs::new("Process/frontend", CONSUMER_UID, EndpointAttachmentKind::Connect);
    assert_eq!(
        inputs.admit(&mut registry),
        Err(EndpointBindingError::TargetSupportMissing),
        "no recorded ceiling means no admitted child request"
    );
}

/// One consumer slot holds one relationship: a second, differently shaped
/// declaration is refused while the slot is live, and a payload change is
/// refused until the old use is closed.
#[test]
fn one_consumer_slot_holds_one_relationship() {
    let mut registry = registry();
    let key = Inputs::new("Process/frontend", CONSUMER_UID, EndpointAttachmentKind::Connect)
        .admit(&mut registry)
        .expect("admit");
    let resident = registry.binding(&key).expect("admitted relationship");
    assert_eq!(
        registry.check_slot(key_ref(&key), &resident_fingerprint(&key, resident)),
        Ok(d2b_contracts_resource::v3::BindingSlotDecision::Coalesced),
        "an identical declaration coalesces"
    );
    let attach_fingerprint = request("Process/frontend", "compositor", EndpointAttachmentKind::Attach)
        .expect("request")
        .fingerprint();
    assert_eq!(
        registry.check_slot(key_ref(&key), &attach_fingerprint),
        Err(EndpointBindingError::Contract(_contract_refusal())),
        "a different payload for a live slot is refused before any mutation"
    );
}

/// A `Host` is not an admitted `EndpointBinding` consumer: a host-level
/// endpoint need is a child support ceiling or an admitted realization leg,
/// never a binding whose consumer is the Host.
#[test]
fn a_host_is_not_an_admitted_endpoint_consumer() {
    assert!(!BindingKind::Endpoint.admits_consumer(BindingConsumerKind::Host));
    assert_eq!(
        request("Host/desktop", "compositor", EndpointAttachmentKind::Connect).unwrap_err(),
        BindingContractError::UnsupportedConsumerKind
    );
    assert!(BindingKind::Endpoint.admits_consumer(BindingConsumerKind::Guest));
    assert!(BindingKind::Endpoint.admits_consumer(BindingConsumerKind::Process));
}

/// The declared realization support is exactly the two exact-endpoint facets,
/// and no request needs a facet this provider cannot enforce.
#[test]
fn the_declared_support_is_the_two_exact_endpoint_facets() {
    let support = endpoint_binding_support();
    for attachment in [
        EndpointAttachmentKind::Connect,
        EndpointAttachmentKind::Listen,
        EndpointAttachmentKind::Attach,
    ] {
        for facet in request("Process/frontend", "compositor", attachment)
            .expect("request")
            .required_facets()
        {
            assert!(
                support.realizes(*facet),
                "{attachment:?} requires an undeclared facet"
            );
        }
    }
    assert!(!support.realizes(BindingRealizationFacet::FilesystemPresentation));
    assert!(!support.realizes(BindingRealizationFacet::ConsumerDeviceSlot));
    assert!(!support.realizes(BindingRealizationFacet::NamespaceInterface));
}

/// A required right maps onto the exact POSIX bits the exact endpoint must
/// apply, and each delivery form declares its own facets.
#[test]
fn required_rights_map_onto_the_exact_effective_bits() {
    assert_eq!(required_right_bits(RequestedRights::Consume), 0o6);
    assert_eq!(required_right_bits(RequestedRights::Observe), 0o4);
    let named = delivery(EndpointAttachmentKind::Attach);
    assert_eq!(
        named.required_facets(),
        &[
            BindingRealizationFacet::EndpointDescriptor,
            BindingRealizationFacet::EndpointPathname,
        ]
    );
    assert!(named.has_destination());
    let described = delivery(EndpointAttachmentKind::Connect);
    assert_eq!(
        described.required_facets(),
        &[BindingRealizationFacet::EndpointDescriptor]
    );
    assert!(!described.has_destination());
}

/// A lifecycle step out of order is refused, and a refusal never echoes the
/// socket it was protecting.
#[test]
fn lifecycle_steps_out_of_order_are_refused_and_stay_path_free() {
    let (mut registry, key) = prepared_registry(EndpointAttachmentKind::Connect);
    assert_eq!(
        registry.revoke(&key),
        Err(EndpointBindingError::UnexpectedState),
        "a prepared relationship is not yet active"
    );
    assert_eq!(
        registry.detach(&key),
        Err(EndpointBindingError::UnexpectedState),
        "an undrained relationship is not detachable"
    );
    assert_eq!(
        registry.revoke(&key).unwrap_err().stage(),
        AdmissionStage::Activate
    );
    let rendered = format!("{:?}", EndpointBindingError::EffectiveAccessMissing);
    assert!(!rendered.contains("run/user"), "{rendered} must not echo a path");
    assert!(!rendered.contains("wayland"), "{rendered} must not echo a socket");
    assert!(!rendered.contains("0x"), "{rendered} must not echo an inode");
}

// ---------------------------------------------------------------------------
// Source row -> committed `EndpointBinding` rows
// ---------------------------------------------------------------------------

/// The compositor endpoint, with the policy and attachment capacity the case
/// under test needs.
fn compositor_spec_with(
    policy: EndpointConsumerPolicy,
    attachments: EndpointAttachmentPolicy,
) -> EndpointSpec {
    EndpointSpec::new(
        reference("Provider/display"),
        reference("Process/compositor"),
        EndpointClass::Service,
        EndpointTransport::Unix,
        token("compositor"),
        None,
        EndpointLocality::HostLocal,
        EndpointVisibility::Owner,
        attachments,
        policy,
        EndpointLifecyclePolicy::RecycleWithProducer,
    )
    .expect("endpoint spec")
}

/// The three consumer kinds the endpoint family admits, all named by the
/// endpoint's own subject allowlist.
fn declared_consumers() -> Vec<ResourceRef> {
    vec![
        reference("Process/frontend"),
        reference("EphemeralProcess/helper"),
        reference("Guest/work-vm"),
    ]
}

/// A policy that names every consumer above and admits the operations the
/// three attachment kinds perform.
fn declared_consumer_policy() -> EndpointConsumerPolicy {
    EndpointConsumerPolicy::new(declared_consumers(), Vec::new(), Vec::new())
        .expect("consumer policy")
}

fn declared_endpoint() -> ResourceRef {
    reference("Endpoint/compositor")
}

/// Three deliveries over the three attachment kinds: one connect, one listen,
/// and one attach, each to a different consumer in its own slot.
fn three_deliveries() -> [DeclaredEndpointBinding; 3] {
    [
        DeclaredEndpointBinding::new(
            target("Process/frontend"),
            slot("compositor"),
            EndpointAttachmentKind::Connect,
        ),
        DeclaredEndpointBinding::new(
            target("EphemeralProcess/helper"),
            slot("display"),
            EndpointAttachmentKind::Listen,
        ),
        DeclaredEndpointBinding::new(
            target("Guest/work-vm"),
            slot("seat"),
            EndpointAttachmentKind::Attach,
        ),
    ]
}

/// Every declared delivery becomes exactly one committed row, and each row
/// carries the source's own decision about the relationship it rides on.
#[test]
fn every_declared_delivery_commits_exactly_one_row() {
    let spec = compositor_spec_with(
        declared_consumer_policy(),
        EndpointAttachmentPolicy::new(true, 2).expect("attachment policy"),
    );
    let deliveries = three_deliveries();
    let rows = canonical_binding_rows(&zone(), &spec, &declared_endpoint(), &deliveries)
        .expect("derived rows");
    assert_eq!(rows.len(), deliveries.len());

    for (row, delivery) in rows.iter().zip(&deliveries) {
        // The committed bytes decode back through the family's own strict
        // wire decoder, and describe the delivery the source declared.
        let decoded: EndpointBindingSpec =
            serde_json::from_slice(row.spec()).expect("canonical EndpointBindingSpec bytes");
        assert_eq!(decoded.endpoint_ref(), &declared_endpoint());
        assert_eq!(decoded.execution_ref(), delivery.consumer().as_ref());
        assert_eq!(decoded.slot().as_str(), delivery.slot().as_str());
        assert_eq!(decoded.attachment(), &delivery.attachment());
        assert_eq!(row.request().slot(), delivery.slot());
        // The row's own references are the ones the shared row rule admits.
        admit_binding_row_refs(BindingKind::Endpoint, decoded.endpoint_ref(), decoded.execution_ref())
            .expect("a row naming a consumer the kind does not admit is refused here");
        // The decision is the source's own: the right the kind requests, the
        // shared arbitration, and exactly the facets the delivery rides on.
        let decision = decoded.source();
        assert_eq!(
            decision.admitted_rights(),
            [delivery.attachment().requested_rights()].as_slice()
        );
        assert_eq!(decision.arbitration(), BindingArbitration::Shared);
        assert_eq!(
            decision.realized_facets(),
            delivery.attachment().required_facets()
        );
    }

    // A connect and a listen reach the endpoint through the verified
    // descriptor alone, so neither commits the private pathname the family
    // declares; an attach addresses a display or a stream by name and commits
    // both.
    let facets: Vec<&[BindingRealizationFacet]> = rows
        .iter()
        .map(|row| row.request().required_facets())
        .collect();
    assert_eq!(
        facets,
        [
            &[BindingRealizationFacet::EndpointDescriptor][..],
            &[BindingRealizationFacet::EndpointDescriptor][..],
            &[
                BindingRealizationFacet::EndpointDescriptor,
                BindingRealizationFacet::EndpointPathname
            ][..],
        ]
    );
    for row in &rows {
        let decoded: EndpointBindingSpec =
            serde_json::from_slice(row.spec()).expect("canonical EndpointBindingSpec bytes");
        assert_eq!(decoded.source().realized_facets(), row.request().required_facets());
    }
}

/// The row the source derives is admitted by this family's own admission
/// path: the relationship that gets minted and the row a boundary reads back
/// are the same relationship, reached by the same key.
#[test]
fn the_derived_row_is_admitted_by_this_familys_own_admission_path() {
    let spec = compositor_spec_with(
        declared_consumer_policy(),
        EndpointAttachmentPolicy::new(true, 2).expect("attachment policy"),
    );
    for delivery in three_deliveries() {
        let row = canonical_binding_row(&zone(), &spec, &declared_endpoint(), &delivery)
            .expect("derived row");
        let mut registry = registry_for(&spec);
        let consumer = delivery.consumer().as_ref().to_canonical_string();
        let admitted = registry
            .admit(EndpointBindingAdmission::new(
                zone(),
                uid(ENDPOINT_UID),
                uid(CONSUMER_UID),
                row.request().clone(),
                provenance_for(&spec),
                spec.clone(),
                None,
                target("Host/desktop"),
                BindingAuthorization::granted(),
                vec![
                    endpoint_freshness(),
                    consumer_freshness(&consumer, CONSUMER_UID, DesiredRevision::INITIAL),
                ],
            ))
            .expect("this family's admission accepts the row it derived");
        let decoded: EndpointBindingSpec =
            serde_json::from_slice(row.spec()).expect("canonical EndpointBindingSpec bytes");
        assert_eq!(
            admitted.key(),
            &decoded
                .key(zone(), uid(ENDPOINT_UID), uid(CONSUMER_UID))
                .expect("the committed row's own KTD3 key"),
            "the admitted relationship and the committed row are one relationship"
        );
        assert_eq!(
            decoded.source().admitted_rights(),
            [admitted.evidence().admission().rights()].as_slice(),
            "the committed decision is the admission the source minted"
        );
        assert_eq!(
            decoded.source().arbitration(),
            admitted.evidence().admission().arbitration()
        );
    }
}

/// A source row that declares no delivery commits no row: the absence of a
/// declared relationship stays absent rather than becoming a default one.
#[test]
fn a_source_row_that_declares_no_delivery_commits_no_row() {
    let spec = compositor_spec_with(
        declared_consumer_policy(),
        EndpointAttachmentPolicy::new(true, 2).expect("attachment policy"),
    );
    assert!(
        canonical_binding_rows(&zone(), &spec, &declared_endpoint(), &[])
            .expect("no delivery, no refusal")
            .is_empty()
    );
    // The same source row with its deliveries derives them, so the empty
    // result is the absence of a declaration and not a policy that admits
    // nothing.
    assert_eq!(
        canonical_binding_rows(&zone(), &spec, &declared_endpoint(), &three_deliveries())
            .expect("derived rows")
            .len(),
        3
    );
}

/// The consumer is whatever the kind admits and never the `Host`: a
/// `Process` frontend and an `EphemeralProcess` helper are both derivable,
/// and a host-side need is an admitted realization leg rather than a binding
/// whose consumer is the Host.
#[test]
fn the_consumer_is_anything_the_kind_admits_but_never_the_host() {
    let spec = compositor_spec_with(
        declared_consumer_policy(),
        EndpointAttachmentPolicy::new(true, 2).expect("attachment policy"),
    );
    for consumer in [
        "Process/frontend",
        "EphemeralProcess/helper",
        "Guest/work-vm",
    ] {
        let row = canonical_binding_row(
            &zone(),
            &spec,
            &declared_endpoint(),
            &DeclaredEndpointBinding::new(
                target(consumer),
                slot("compositor"),
                EndpointAttachmentKind::Connect,
            ),
        )
        .unwrap_or_else(|error| panic!("{consumer} is an admitted consumer: {error}"));
        let decoded: EndpointBindingSpec =
            serde_json::from_slice(row.spec()).expect("canonical EndpointBindingSpec bytes");
        assert_eq!(decoded.execution_ref(), &reference(consumer));
    }
    assert_eq!(
        canonical_binding_row(
            &zone(),
            &spec,
            &declared_endpoint(),
            &DeclaredEndpointBinding::new(
                target("Host/desktop"),
                slot("compositor"),
                EndpointAttachmentKind::Connect,
            ),
        )
        .expect_err("a Host is never the consumer of an EndpointBinding"),
        EndpointBindingError::InvalidRequest
    );
}

/// The endpoint's own declaration refuses the deliveries it does not admit:
/// a consumer it does not name, an operation it never declared, an endpoint
/// with no attachment capacity, and a bound capability that is not an
/// `Endpoint`.
#[test]
fn the_endpoints_own_declaration_refuses_what_it_does_not_admit() {
    let attachments = EndpointAttachmentPolicy::new(true, 2).expect("attachment policy");
    // A consumer the endpoint's subject allowlist does not name, even though
 // the binding kind admits its resource type.
    let one_consumer = compositor_spec_with(
        EndpointConsumerPolicy::new(vec![reference("Process/frontend")], Vec::new(), Vec::new())
            .expect("consumer policy"),
        attachments,
    );
    assert_eq!(
        canonical_binding_row(
            &zone(),
            &one_consumer,
            &declared_endpoint(),
            &DeclaredEndpointBinding::new(
                target("EphemeralProcess/helper"),
                slot("compositor"),
                EndpointAttachmentKind::Connect,
            ),
        )
        .expect_err("the endpoint names one consumer"),
        EndpointBindingError::ConsumerNotAllowed
    );
    // An `attach` reaches the endpoint through the `attach` operation, which
 // this endpoint never declared.
    let resolve_only = compositor_spec_with(
        EndpointConsumerPolicy::new(
            declared_consumers(),
            Vec::new(),
            vec![EndpointOperation::Resolve],
        )
        .expect("consumer policy"),
        attachments,
    );
    assert_eq!(
        canonical_binding_row(
            &zone(),
            &resolve_only,
            &declared_endpoint(),
            &DeclaredEndpointBinding::new(
                target("Process/frontend"),
                slot("compositor"),
                EndpointAttachmentKind::Attach,
            ),
        )
        .expect_err("the endpoint declares no attach operation"),
        EndpointBindingError::OperationNotAllowed
    );
    // An endpoint that declares no attachment capacity admits no attachment.
    let no_attachments = compositor_spec_with(
        declared_consumer_policy(),
        EndpointAttachmentPolicy::new(false, 0).expect("attachment policy"),
    );
    assert_eq!(
        canonical_binding_row(
            &zone(),
            &no_attachments,
            &declared_endpoint(),
            &DeclaredEndpointBinding::new(
                target("Process/frontend"),
                slot("compositor"),
                EndpointAttachmentKind::Attach,
            ),
        )
        .expect_err("the endpoint admits no attachment"),
        EndpointBindingError::AttachmentRefused
    );
    // A bound capability that is not an `Endpoint` is not this family's row.
    assert_eq!(
        canonical_binding_row(
            &zone(),
            &unrestricted_spec(),
            &reference("Volume/state"),
            &DeclaredEndpointBinding::new(
                target("Process/frontend"),
                slot("compositor"),
                EndpointAttachmentKind::Connect,
            ),
        )
        .expect_err("an endpoint binding binds an Endpoint"),
        EndpointBindingError::Contract(d2b_contracts_resource::v3::BindingRefusal::new(
            AdmissionStage::Admit,
            RefusalReason::IdentityNotAuthorized,
        ))
    );
}

/// A facet this family never declared cannot be committed, and a set that
/// mixes a declared facet with an undeclared one is refused whole rather
/// than committed in part.
#[test]
fn a_facet_this_family_never_declared_is_refused() {
    assert_eq!(
        ensure_realizable(&[
            BindingRealizationFacet::EndpointDescriptor,
            BindingRealizationFacet::EndpointPathname,
        ]),
        Ok(())
    );
    for facet in [
        BindingRealizationFacet::FilesystemPresentation,
        BindingRealizationFacet::ConsumerDeviceSlot,
        BindingRealizationFacet::DeviceAttachment,
        BindingRealizationFacet::NamespaceInterface,
        BindingRealizationFacet::SharedFabric,
        BindingRealizationFacet::CredentialDelivery,
    ] {
        assert_eq!(
            ensure_realizable(&[facet]),
            Err(EndpointBindingError::UnsupportedFacet),
            "{facet:?} is not a facet this family can realize"
        );
    }
    assert_eq!(
        ensure_realizable(&[
            BindingRealizationFacet::EndpointDescriptor,
            BindingRealizationFacet::FilesystemPresentation,
        ]),
        Err(EndpointBindingError::UnsupportedFacet)
    );
}

/// A relationship keeps one row name across passes, reordering the
 /// declarations never churns an identity, and two deliveries claiming one
 /// consumer slot are one relationship declared twice rather than two.
#[test]
fn a_relationship_keeps_one_row_name_and_one_slot_holds_one_row() {
    let spec = compositor_spec_with(
        declared_consumer_policy(),
        EndpointAttachmentPolicy::new(true, 2).expect("attachment policy"),
    );
    let deliveries = three_deliveries();
    let first =
        canonical_binding_rows(&zone(), &spec, &declared_endpoint(), &deliveries).expect("rows");
    // Deriving again from the same committed source row changes nothing, so a
    // restart re-ensures the same rows instead of churning identities.
    assert_eq!(
        first,
        canonical_binding_rows(&zone(), &spec, &declared_endpoint(), &deliveries).expect("rows")
    );
    // Declaration order is not identity: the same three relationships in
    // another order keep the same three row names.
    let reordered = [deliveries[2].clone(), deliveries[0].clone(), deliveries[1].clone()];
    let mut names: Vec<String> = first
        .iter()
        .map(|row| row.name().as_str().to_owned())
        .collect();
    let mut reordered_names: Vec<String> =
        canonical_binding_rows(&zone(), &spec, &declared_endpoint(), &reordered)
            .expect("rows")
            .iter()
            .map(|row| row.name().as_str().to_owned())
            .collect();
    names.sort();
    reordered_names.sort();
    assert_eq!(names, reordered_names);
    assert_eq!(names.len(), 3, "distinct relationships never collide");
    assert!(names.iter().all(|name| name.starts_with("endpoint-binding-")));

    // The slot address IS the relationship: a second delivery claiming the one
 // consumer slot is refused rather than committed as a second row.
    let twice = [
        deliveries[0].clone(),
        DeclaredEndpointBinding::new(
            target("Process/frontend"),
            slot("compositor"),
            EndpointAttachmentKind::Listen,
        ),
    ];
    assert_eq!(
        canonical_binding_rows(&zone(), &spec, &declared_endpoint(), &twice)
            .expect_err("one consumer slot holds one relationship"),
        EndpointBindingError::InvalidRequest
    );
}

fn key_ref(key: &BindingKey) -> &BindingKey {
    key
}

fn _contract_refusal() -> d2b_contracts_resource::v3::BindingRefusal {
    d2b_contracts_resource::v3::BindingRefusal::new(
        AdmissionStage::Admit,
        d2b_contracts_resource::v3::RefusalReason::ConflictingDeclaration,
    )
}

fn resident_fingerprint(
    _key: &BindingKey,
    resident: &d2b_provider_endpoint::AdmittedEndpointBinding,
) -> d2b_contracts_resource::v3::BindingSpecFingerprint {
    request("Process/frontend", "compositor", resident.attachment())
        .expect("request")
        .fingerprint()
}
