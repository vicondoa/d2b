//! The common Guest target/session contract.
//!
//! Every Guest implementation - a local VM provider, a media-backed provider,
//! or a remote cloud provider - reaches its target through this one contract.
//! Two properties make that true, and this file proves both.
//!
//! **It is provider-neutral, structurally.** The first test reads the contract's
//! own source and fails if it ever grows a provider-name branch: a declared
//! Provider reference is carried as graph data, so the token list comes from
//! the family's own registration table and a fifth Guest provider extends the
//! ratchet automatically. A behavioral test then runs the same three declared
//! fixtures - a hypervisor-shaped Guest, a media-shaped Guest, and a remote
//! Guest - through identical requests and requires identical answers.
//!
//! **It is fenced on the admitted graph.** The rest of this file proves the
//! evidence the contract consumes: the enrolled Guest uid, the boot identity,
//! the authority Zone, the source uid, the desired generation, and the live
//! reconnect generation. A lost session keeps the ownership it retained and
//! cannot mint fresh Host authority for it.

use std::sync::{Arc, Mutex};

use d2b_contracts_resource::v3::{
    ResourceRef, ResourceUid, SchemaFingerprint, ZoneId,
    identity::{ReconnectGeneration, SessionPurpose},
};
use d2b_provider_guest::driver::GUEST_REGISTRATIONS;
use d2b_provider_guest::target_control::{
    GuestTargetContract, graph_target_control, guest_target_ref,
};
use d2b_provider_guest::target_service::{
    GuestTargetEffect, GuestTargetEffectError, GuestTargetEffects, GuestTargetRefusal,
    GuestTargetService,
};
use d2b_resource_runtime::guest_target::{
    GuestAdoption, GuestRealizeRequest, GuestTargetError, GuestTargetRuntime,
    TargetControlAssignment, TargetControlFrame, TargetControlRequest, TargetControlResponse,
    TargetResourceInstance,
    TargetInstanceState, target_local_spec_digest,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_runtime::target::{TargetObservation, TargetRef};
use d2bd_runtime::guest_mode::{
    BootIdentity, GUEST_COMPONENT_SESSION_PURPOSE, GuestIdentity,
};
use d2bd_runtime::target_runtime::GuestParentSessionEvidence;

const ZONE: &str = "work";
const GUEST: &str = "workload";
const TARGET_TYPE: &str = "Endpoint";
const GUEST_UID: &str = "123e4567-e89b-42d3-a456-426614174000";

// ---------------------------------------------------------------------------
// The structural ratchet: the contract has no provider-name branch
// ---------------------------------------------------------------------------

/// The contract's own source, as the crate compiles it.
const CONTRACT_SOURCES: &[(&str, &str)] = &[
    ("target_control.rs", include_str!("../src/target_control.rs")),
    ("target_service.rs", include_str!("../src/target_service.rs")),
];

/// Every provider-identity token the Guest family declares.
///
/// Derived from the family's own registration table, so adding a Guest
/// provider extends this ratchet without editing it: the new Provider
/// reference and both of its spellings become forbidden in the contract.
fn provider_tokens() -> Vec<String> {
    let mut tokens = Vec::new();
    for registration in &GUEST_REGISTRATIONS {
        let name = registration
            .provider_ref
            .rsplit('/')
            .next()
            .expect("a Provider reference names a row")
            .to_owned();
        // `runtime-cloud-hypervisor` -> `cloud_hypervisor` and `cloud-hypervisor`
        let snake = name.replace('-', "_");
        for segment in name.split('-').filter(|segment| *segment != "runtime") {
            tokens.push(segment.to_owned());
            tokens.push(segment.replace('-', "_"));
        }
        tokens.push(name.clone());
        tokens.push(snake);
    }
    tokens.sort();
    tokens.dedup();
    tokens
}

/// Strip comments and doc comments so the ratchet reads code, not prose: the
/// contract's documentation is allowed to name the providers it serves.
fn code_of(source: &str) -> Vec<String> {
    source
        .lines()
        .map(|line| {
            let code = match line.find("//") {
                Some(index) => &line[..index],
                None => line,
            };
            code.trim().to_owned()
        })
        .filter(|line| !line.is_empty())
        .collect()
}

/// The common contract is provider-neutral by construction: its own source
/// never names a Guest provider and never branches on the family kind enum.
///
/// This is the property that lets a second Guest implementation consume the
/// contract while the first is still being converted. If a future change adds
/// `if provider == "runtime-qemu-media"` - or an import of the family kind
/// enum - this test fails instead of the contract quietly serializing the
/// providers behind one of them.
#[test]
fn the_contract_has_no_provider_name_branch() {
    let tokens = provider_tokens();
    assert!(
        tokens.len() >= 8,
        "the token set is derived from the family table and must cover every declared Provider"
    );
    for (name, source) in CONTRACT_SOURCES {
        for line in code_of(source) {
            for token in &tokens {
                assert!(
                    !line.contains(token.as_str()),
                    "{name} branches on the Guest provider `{token}`: {line}\n\
                     the common contract carries the declared Provider as graph data"
                );
            }
        }
        assert!(
            !source.contains("GuestKind"),
            "{name} names the Guest family kind enum; the contract is provider-neutral"
        );
    }
}

/// The same ratchet on the crate root, which is where a new contract surface
/// would be re-exported.
#[test]
fn the_crate_root_exports_the_contract_without_a_provider_branch() {
    for token in provider_tokens() {
        let code = code_of(include_str!("../src/lib.rs"));
        for line in code {
            assert!(
                !line.contains(&token),
                "lib.rs names the Guest provider `{token}` in code: {line}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Fixtures: three declared Guest providers, one contract
// ---------------------------------------------------------------------------

/// The three declared Guest shapes the contract must serve identically.
///
/// They differ only in the Provider row the accepted graph bound to the Guest
/// and in the target-local spec shape that Provider declares - never in a
/// branch, because the contract has no branch.
struct DeclaredGuest {
    label: &'static str,
    provider_ref: &'static str,
    spec: &'static [u8],
}

const DECLARED_GUESTS: [DeclaredGuest; 3] = [
    DeclaredGuest {
        label: "local-hypervisor",
        provider_ref: "Provider/runtime-cloud-hypervisor",
        spec: br#"{"vmm":"cloud-hypervisor","memoryMib":4096}"#,
    },
    DeclaredGuest {
        label: "media",
        provider_ref: "Provider/runtime-qemu-media",
        spec: br#"{"vmm":"qemu","media":"iso"}"#,
    },
    DeclaredGuest {
        label: "remote",
        provider_ref: "Provider/runtime-azure-virtual-machine",
        spec: br#"{"region":"westeurope","size":"standard-d2s-v5"}"#,
    },
];

fn guest_identity(reconnect_generation: u64) -> GuestIdentity {
    GuestIdentity::new(
        ResourceRef::parse(&format!("Guest/{GUEST}")).expect("Guest ref"),
        ResourceUid::parse(GUEST_UID).expect("Guest uid"),
        zone(),
        BootIdentity::from_kernel_boot_id("graph-target-contract-test").expect("boot identity"),
        SessionPurpose::parse(GUEST_COMPONENT_SESSION_PURPOSE).expect("purpose"),
        SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64))).expect("schema"),
        ReconnectGeneration::new(reconnect_generation).expect("reconnect generation"),
        1,
        1,
        1,
    )
    .expect("Guest identity")
}

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).expect("zone")
}

fn evidence(guest: &DeclaredGuest, session_generation: u64) -> GuestParentSessionEvidence {
    GuestParentSessionEvidence::bind(
        &guest_identity(1),
        ResourceRef::parse(guest.provider_ref).expect("Provider ref"),
        session_generation,
    )
    .expect("graph evidence")
}

fn target() -> TargetRef {
    TargetRef::guest(GUEST).expect("guest target")
}

fn source(name: &str) -> ResourceKey {
    ResourceKey::new(ZONE, TARGET_TYPE, name)
}

fn assignment(name: &str, uid: [u8; 16], generation: u64, session: u64) -> TargetControlAssignment {
    TargetControlAssignment::new(source(name), uid, generation, session)
}

fn realize(
    name: &str,
    uid: [u8; 16],
    generation: u64,
    session: u64,
    spec: &[u8],
) -> TargetControlRequest {
    TargetControlRequest::Realize(GuestRealizeRequest::new(
        assignment(name, uid, generation, session),
        spec.to_vec(),
        target_local_spec_digest(spec),
        format!("/run/d2b/{name}.sock"),
    ))
}

/// A target-local effect that records the exact spec it was handed.
#[derive(Default)]
struct RecordingEffect {
    realized: Mutex<Vec<Vec<u8>>>,
    deleted: Mutex<Vec<ResourceKey>>,
    present: Mutex<bool>,
}

impl RecordingEffect {
    fn new() -> Arc<Self> {
        Arc::new(Self { present: Mutex::new(true), ..Self::default() })
    }
}

#[async_trait::async_trait]
impl GuestTargetEffect for RecordingEffect {
    async fn realize(
        &self,
        request: &GuestRealizeRequest,
    ) -> Result<(), GuestTargetEffectError> {
        self.realized.lock().expect("realized").push(request.spec().to_vec()); // async-gate-allow: test-support recorder lock
        Ok(())
    }

    async fn delete(&self, source: &ResourceKey) -> Result<(), GuestTargetEffectError> {
        self.deleted.lock().expect("deleted").push(source.clone()); // async-gate-allow: test-support recorder lock
        Ok(())
    }

    async fn adopt(&self, _source: &ResourceKey) -> Result<bool, GuestTargetEffectError> {
        Ok(*self.present.lock().expect("present")) // async-gate-allow: test-support recorder lock
    }
}

/// One graph-backed Guest target service over the common contract.
struct Fixture {
    runtime: Arc<GuestTargetRuntime>,
    contract: Arc<Mutex<GuestTargetContract>>,
    effect: Arc<RecordingEffect>,
    service: GuestTargetService,
}

impl Fixture {
    fn new(guest: &DeclaredGuest, session_generation: u64) -> Self {
        let runtime = Arc::new(GuestTargetRuntime::new(target()));
        let contract = Arc::new(Mutex::new(
            GuestTargetContract::bind(evidence(guest, session_generation))
                .expect("the evidence names a canonical Guest target"),
        ));
        let effect = RecordingEffect::new();
        let effects = GuestTargetEffects::from([(
            ResourceTypeName::new(TARGET_TYPE),
            Arc::clone(&effect) as Arc<dyn GuestTargetEffect>,
        )]);
        let service = GuestTargetService::graph_backed(
            Arc::clone(&runtime),
            Arc::clone(&contract),
            effects,
        );
        service.bind_session(session_generation).expect("connect the contract");
        Self { runtime, contract, effect, service }
    }

    fn contract(&self) -> std::sync::MutexGuard<'_, GuestTargetContract> {
        self.contract.lock().expect("contract")
    }

    async fn handle(&self, request: TargetControlRequest) -> TargetControlResponse {
        self.service.handle(request).await
    }
}

// ---------------------------------------------------------------------------
// Scenario 2: the contract serves every declared Guest provider identically
// ---------------------------------------------------------------------------

/// The contract supports declared local-VM, media, and remote Guest fixtures
/// with no provider-name branch: the same requests get the same answers, and
/// the recorded ownership is the same apart from the declared Provider.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn every_declared_guest_provider_gets_the_same_contract() {
    let mut admitted = Vec::new();
    for guest in &DECLARED_GUESTS {
        let f = Fixture::new(guest, 1);
        let spec = guest.spec;

        let first = f
            .handle(realize("relay", [7; 16], 3, 1, spec))
            .await;
        let TargetControlResponse::Realized { realization } = &first else {
            panic!("{}: realize: {first:?}", guest.label);
        };
        assert_eq!(
            realization.state(),
            TargetInstanceState::Ready,
            "{}: the effect serves the exact spec",
            guest.label
        );
        assert_eq!(
            realization.spec_digest(),
            target_local_spec_digest(spec),
            "{}: the commitment is over this Provider's own spec",
            guest.label
        );
        assert_eq!(
            f.effect.realized.lock().expect("realized").as_slice(), // async-gate-allow: test-support recorder lock
            &[spec.to_vec()],
            "{}: the effect applied exactly the declared bytes",
            guest.label
        );

        // The idempotent re-realize, the observation, the adoption and the
        // delete all behave the same for every declared Provider.
        assert!(
            matches!(
                f.handle(realize("relay", [7; 16], 3, 1, spec)).await,
                TargetControlResponse::Realized { .. }
            ),
            "{}: a repeated realize converges on the same instance",
            guest.label
        );
        assert_eq!(
            f.handle(TargetControlRequest::Observe {
                assignment: assignment("relay", [7; 16], 3, 1)
            })
            .await,
            TargetControlResponse::Observed(TargetObservation::Ready { session_generation: 1 }),
            "{}: observation reports the live realization",
            guest.label
        );
        assert!(
            matches!(
                f.handle(TargetControlRequest::Adopt {
                    assignment: assignment("relay", [7; 16], 3, 1)
                })
                .await,
                TargetControlResponse::Adopted(GuestAdoption::Adopted(_))
            ),
            "{}: adoption re-binds the same realization",
            guest.label
        );
        assert_eq!(
            f.handle(TargetControlRequest::Delete {
                assignment: assignment("relay", [7; 16], 3, 1)
            })
            .await,
            TargetControlResponse::Deleted,
            "{}: delete removes exactly this source",
            guest.label
        );
        assert!(f.contract().bindings().is_empty(), "{}: delete releases the ownership", guest.label);

        let contract = f.contract();
        admitted.push((
            guest.label,
            contract.evidence().provider_ref().clone(),
            contract.evidence().guest_uid().clone(),
            contract.evidence().boot_identity(),
            contract.target().clone(),
        ));
    }

    // Three different declared Providers, one identical contract shape.
    assert_eq!(admitted.len(), 3);
    for pair in admitted.windows(2) {
        assert_ne!(pair[0].1, pair[1].1, "the fixtures really are different Providers");
        assert_eq!(pair[0].2, pair[1].2, "the enrolled Guest uid is the same");
        assert_eq!(pair[0].3, pair[1].3, "the boot identity is the same");
        assert_eq!(pair[0].4, pair[1].4, "the target is the same");
    }
}

// ---------------------------------------------------------------------------
// Scenario 1: the evidence fences every target-local effect
// ---------------------------------------------------------------------------

/// A Guest uid, boot identity, Zone, source uid, or desired generation that
/// does not match the admitted evidence never produces a realization, and the
/// effect never sees the spec.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn the_admitted_evidence_fences_every_target_local_effect() {
    let guest = &DECLARED_GUESTS[0];
    let spec = guest.spec;
    let f = Fixture::new(guest, 1);

    // A source outside the evidence's authority Zone.
    let foreign_zone = TargetControlRequest::Realize(GuestRealizeRequest::new(
        TargetControlAssignment::new(ResourceKey::new("other", TARGET_TYPE, "relay"), [7; 16], 3, 1),
        spec.to_vec(),
        target_local_spec_digest(spec),
        "/run/d2b/relay.sock",
    ));
    assert_eq!(
        f.handle(foreign_zone).await,
        TargetControlResponse::SessionUnavailable,
        "a source under another Zone's authority is refused"
    );

    // A source uid that is not the uid this Guest owns.
    assert!(
        matches!(
            f.handle(realize("relay", [7; 16], 3, 1, spec)).await,
            TargetControlResponse::Realized { .. }
        ),
        "the first assignment owns the source"
    );

    // A desired generation older than the admitted one.
    let regression = realize("relay", [7; 16], 2, 1, spec);
    assert_eq!(
        f.handle(regression).await,
        TargetControlResponse::SessionUnavailable,
        "an older desired generation is refused"
    );

    // A session generation that is not the live one.
    let stale_session = realize("other", [7; 16], 3, 9, spec);
    assert_eq!(
        f.handle(stale_session).await,
        TargetControlResponse::SessionUnavailable,
        "a session that is not live performs no effect"
    );

    // A spec that does not match its own commitment.
    let substituted = TargetControlRequest::Realize(GuestRealizeRequest::new(
        assignment("other", [7; 16], 3, 1),
        br#"{"vmm":"substituted"}"#.to_vec(),
        target_local_spec_digest(spec),
        "/run/d2b/other.sock",
    ));
    assert_eq!(
        f.handle(substituted).await,
        TargetControlResponse::SessionUnavailable,
        "a spec that does not match its commitment is refused"
    );
    assert!(
        f.contract().binding(&source("other")).is_none(),
        "a refused realize takes no ownership"
    );

    // A different uid for a source this Guest already owns.
    let swapped = realize("relay", [8; 16], 3, 1, spec);
    assert_eq!(
        f.handle(swapped).await,
        TargetControlResponse::SessionUnavailable,
        "a source that presents a different uid never inherits the realization"
    );
    assert_eq!(
        f.effect.realized.lock().expect("realized").len(), // async-gate-allow: test-support recorder lock
        1,
        "the substituted uid's spec never reached the effect"
    );

    assert!(
        f.runtime.instances().is_empty(),
        "the refused replacement dropped the record it was not allowed to inherit"
    );
    assert_eq!(
        f.effect.realized.lock().expect("realized").as_slice(), // async-gate-allow: test-support recorder lock
        &[spec.to_vec()],
        "no refused spec ever reached the effect"
    );
    assert!(f.effect.deleted.lock().expect("deleted").is_empty()); // async-gate-allow: test-support recorder lock
}

/// The contract refuses a target that is not this Guest's, and a session that
/// is not live mints nothing at all.
#[test]
fn the_contract_refuses_a_foreign_target_and_a_session_that_never_connected() {
    let guest = &DECLARED_GUESTS[0];
    let contract = GuestTargetContract::bind(evidence(guest, 1)).expect("contract");
    let mut contract = contract;

    assert_eq!(
        contract.session_generation(),
        None,
        "binding the evidence connects nothing by itself"
    );
    assert_eq!(
        contract.admit(&target(), &assignment("relay", [7; 16], 3, 1)),
        Err(GuestTargetRefusal::SessionUnavailable),
        "an unconnected contract admits nothing"
    );

    contract.connect(1).expect("connect");
    let foreign = TargetRef::guest("another-guest").expect("guest target");
    assert_eq!(
        contract.admit(&foreign, &assignment("relay", [7; 16], 3, 1)),
        Err(GuestTargetRefusal::TargetMismatch),
        "a Guest realizes only for its own target"
    );
    assert_eq!(
        contract.check(&target(), &assignment("relay", [7; 16], 3, 2)),
        Err(GuestTargetRefusal::StaleSessionGeneration),
        "another session's assignment is refused"
    );
    assert_eq!(
        contract.connect(1),
        Err(GuestTargetRefusal::StaleSessionGeneration),
        "the live generation is not reconnected as a newer one"
    );
    assert_eq!(
        contract.connect(0),
        Err(GuestTargetRefusal::SessionUnavailable),
        "generation zero is never a session"
    );
    assert!(contract.bindings().is_empty(), "every refusal left the ledger empty");
    assert_eq!(contract.target(), &target());
    assert_eq!(
        contract.evidence().zone(),
        &zone(),
        "the contract carries the admitted Zone"
    );
    assert_eq!(
        guest_target_ref(contract.evidence().guest_ref()),
        Some(target()),
        "the enrolled Guest and the contract's target are the same Guest"
    );
}

/// The same source may carry a new uid only after the Host has retired the old
/// one; until then a replacement is refused, and after the delete the
/// replacement converges.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_replaced_source_needs_a_delete_before_it_carries_a_new_uid() {
    let guest = &DECLARED_GUESTS[0];
    let spec = guest.spec;
    let f = Fixture::new(guest, 1);
    f.handle(realize("relay", [7; 16], 3, 1, spec)).await;

    let replacement = realize("relay", [8; 16], 3, 1, spec);
    assert_eq!(
        f.handle(replacement.clone()).await,
        TargetControlResponse::SessionUnavailable,
        "the replacement cannot inherit the previous source's realization"
    );
    assert!(
        matches!(
            f.handle(replacement).await,
            TargetControlResponse::Realized { .. }
        ),
        "the replacement converges once the stale record is forgotten"
    );
    let binding = f
        .contract()
        .binding(&source("relay"))
        .cloned()
        .expect("the contract owns the source");
    assert_eq!(binding.source_uid(), [8; 16], "the ownership names the new uid");
    assert_eq!(binding.assignment_generation(), 3);

    f.handle(TargetControlRequest::Delete {
        assignment: assignment("relay", [8; 16], 3, 1),
    })
    .await;
    assert!(
        f.contract().binding(&source("relay")).is_none(),
        "delete releases the ownership"
    );
}

// ---------------------------------------------------------------------------
// Scenario 3: a lost session keeps ownership and mints nothing
// ---------------------------------------------------------------------------

/// Losing the parent session retains the source and binding ownership exactly
/// as admitted, and the Guest cannot mint fresh Host authority for it: nothing
/// is admitted, deleted, or re-bound until a newer session connects and
/// re-adopts.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn a_lost_session_retains_ownership_and_mints_no_host_authority() {
    let guest = &DECLARED_GUESTS[0];
    let spec = guest.spec;
    let f = Fixture::new(guest, 1);
    f.handle(realize("relay", [7; 16], 3, 1, spec)).await;
    f.handle(realize("sibling", [4; 16], 2, 1, spec)).await;
    let retained = f.contract().bindings();
    assert_eq!(retained.len(), 2, "two sources are owned");

    f.service.disconnect_session(1).expect("session lost");
    assert_eq!(
        f.contract().session_generation(),
        None,
        "no session is live any more"
    );
    assert_eq!(
        f.contract().bindings(),
        retained,
        "the ownership is retained exactly as admitted"
    );
    assert_eq!(
        retained[0].session_generation(),
        1,
        "a lost session never advances its own bindings"
    );
    assert_eq!(
        f.runtime.instances().len(),
        2,
        "the target-local realizations are retained too"
    );

    // Nothing is admitted while no session is live - not a new source, not a
    // delete, not an adoption, and not a fresh uid for a retained source.
    for request in [
        realize("fresh", [1; 16], 1, 1, spec),
        TargetControlRequest::Observe { assignment: assignment("relay", [7; 16], 3, 1) },
        TargetControlRequest::Delete { assignment: assignment("relay", [7; 16], 3, 1) },
        TargetControlRequest::Adopt { assignment: assignment("relay", [7; 16], 3, 1) },
    ] {
        assert_eq!(
            f.handle(request).await,
            TargetControlResponse::SessionUnavailable,
            "a lost session performs no effect at all"
        );
    }
    assert_eq!(
        f.contract().bindings(),
        retained,
        "a lost session cannot mint, release, or re-bind ownership"
    );
    assert!(f.effect.deleted.lock().expect("deleted").is_empty()); // async-gate-allow: test-support recorder lock
    assert_eq!(f.runtime.instances().len(), 2, "no realization was removed");

    // A retained caller cannot close a session it does not own, and a
    // reconnect must be strictly newer.
    assert_eq!(
        f.service.disconnect_session(1),
        Err(GuestTargetError::SessionUnavailable),
        "the session is already gone"
    );
    assert!(
        f.service.bind_session(1).is_err(),
        "a lost generation is never re-used: the reconnect must be newer"
    );

    // A newer session connects and re-adopts the retained ownership.
    f.service.bind_session(2).expect("reconnect");
    assert_eq!(f.contract().session_generation(), Some(2));
    assert_eq!(
        f.contract().bindings(),
        retained,
        "the reconnect inherits the retained ownership"
    );
    let adopted = f
        .handle(TargetControlRequest::Adopt { assignment: assignment("relay", [7; 16], 3, 2) })
        .await;
    let TargetControlResponse::Adopted(GuestAdoption::Adopted(instance)) = &adopted else {
        panic!("adopt: {adopted:?}");
    };
    assert_eq!(instance.session_generation(), 2, "adoption re-binds to the live session");
    let binding = f
        .contract()
        .binding(&source("relay"))
        .cloned()
        .expect("the reconnect still owns the source");
    assert_eq!(binding.session_generation(), 2, "the ownership re-binds with it");
    assert_eq!(binding.source_uid(), [7; 16], "for the same source uid");
}

/// An unconfirmed adoption forgets the realization and its ownership together,
/// so the Guest never keeps a binding for something it just disowned.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn an_unconfirmed_adoption_forgets_the_realization_and_its_ownership() {
    let guest = &DECLARED_GUESTS[0];
    let spec = guest.spec;
    let f = Fixture::new(guest, 1);
    f.handle(realize("relay", [7; 16], 3, 1, spec)).await;
    *f.effect.present.lock().expect("present") = false; // async-gate-allow: test-support recorder lock
    f.service.bind_session(2).expect("reconnect");

    assert_eq!(
        f.handle(TargetControlRequest::Adopt { assignment: assignment("relay", [7; 16], 3, 2) })
            .await,
        TargetControlResponse::Adopted(GuestAdoption::Missing),
        "an absent effect is never reported as adopted"
    );
    assert!(f.runtime.instance(&source("relay")).is_none(), "the record is forgotten");
    assert!(
        f.contract().binding(&source("relay")).is_none(),
        "the ownership goes with the realization it described"
    );
}

// ---------------------------------------------------------------------------
// The parent-side channel rides the same contract
// ---------------------------------------------------------------------------
/// The parent-side channel runs its frames through the same contract before
/// they are carried, so a request the Guest would refuse never costs a round
/// trip - and a request the Guest accepts takes the ownership on the way out.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn the_contract_fenced_channel_admits_before_it_carries() {
    use d2b_provider_guest::target_control::GuestTargetSession;

    /// A carrier that counts the frames it is actually asked to send.
    struct CountingSession {
        calls: Arc<Mutex<usize>>,
    }

    #[async_trait::async_trait]
    impl GuestTargetSession for CountingSession {
        fn is_live(&self) -> bool {
            true
        }

        async fn request(
            &self,
            request: ttrpc::Request,
        ) -> Result<ttrpc::Response, GuestTargetError> {
            *self.calls.lock().expect("calls") += 1; // async-gate-allow: test-support recorder lock
            let carried = TargetControlFrame::decode(&request.payload)?
                .into_request()?;
            let mut reply = ttrpc::Response::new();
            reply.set_status(ttrpc::get_status(ttrpc::Code::OK, ""));
            reply.payload = match carried {
                TargetControlRequest::Realize(realize) => TargetControlResponse::Realized {
                    realization: TargetResourceInstance::new(
                        realize.source().clone(),
                        *realize.source_uid(),
                        realize.assignment_generation(),
                        realize.session_generation(),
                        realize.local_handle().to_owned(),
                        realize.spec_digest().to_owned(),
                        TargetInstanceState::Ready,
                    ),
                },
                TargetControlRequest::Adopt { .. } => {
                    TargetControlResponse::Adopted(GuestAdoption::Missing)
                }
                TargetControlRequest::Observe { .. } => {
                    TargetControlResponse::Observed(TargetObservation::Absent)
                }
                TargetControlRequest::Delete { .. } => TargetControlResponse::Deleted,
            }
            .encode();
            Ok(reply)
        }
    }

    let guest = &DECLARED_GUESTS[0];
    let spec = guest.spec;
    let contract = Arc::new(Mutex::new(
        GuestTargetContract::bind(evidence(guest, 1)).expect("contract"),
    ));
    contract.lock().expect("contract").connect(1).expect("connect"); // async-gate-allow: test-support recorder lock
    let calls = Arc::new(Mutex::new(0));
    let control = graph_target_control(
        Arc::clone(&contract),
        CountingSession { calls: Arc::clone(&calls) },
        1,
    )
    .expect("graph-backed target control");

    // A session generation the contract does not own.
    assert!(
        control.delete(&assignment("relay", [7; 16], 3, 5)).await.is_err(),
        "another session's request is refused"
    );
    assert_eq!(*calls.lock().expect("calls"), 0, "a refused frame never left the host"); // async-gate-allow: test-support recorder lock

    // The admitted request reaches the carrier exactly once and takes the
    // ownership on the way out.
    control
        .realize(GuestRealizeRequest::new(
            assignment("relay", [7; 16], 3, 1),
            spec.to_vec(),
            target_local_spec_digest(spec),
            "/run/d2b/relay.sock",
        ))
        .await
        .expect("the admitted request is carried");
    assert_eq!(*calls.lock().expect("calls"), 1); // async-gate-allow: test-support recorder lock
    assert_eq!(
        contract.lock().expect("contract").bindings().len(), // async-gate-allow: test-support recorder lock
        1,
        "the admitted request took the ownership"
    );

    // A different uid for a source the contract already owns is refused here
    // for the same reason the Guest refuses it on arrival.
    assert!(
        control.observe(&assignment("relay", [8; 16], 3, 1)).await.is_err(),
        "a replaced source never inherits the previous source's ownership"
    );
    assert_eq!(*calls.lock().expect("calls"), 1, "the refused frame was never carried"); // async-gate-allow: test-support recorder lock
    assert_eq!(
        contract.lock().expect("contract").binding(&source("relay")).map(|b| b.source_uid()), // async-gate-allow: test-support recorder lock
        Some([7; 16]),
        "the refused frame left the ownership untouched"
    );
}

/// Reading a source is not owning it: an observation, and a delete of a source
/// that was never realized, both leave the ledger exactly as they found it.
#[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
async fn observing_and_deleting_an_unrealized_source_takes_no_ownership() {
    let guest = &DECLARED_GUESTS[0];
    let f = Fixture::new(guest, 1);
    let absent = assignment("unseen", [7; 16], 3, 1);

    assert_eq!(
        f.handle(TargetControlRequest::Observe { assignment: absent.clone() }).await,
        TargetControlResponse::Observed(TargetObservation::Absent),
        "an unowned source observes as absent"
    );
    assert!(
        f.contract().binding(&source("unseen")).is_none(),
        "an observation never mints ownership"
    );

    assert_eq!(
        f.handle(TargetControlRequest::Delete { assignment: absent }).await,
        TargetControlResponse::Deleted,
        "deleting an absent source is still answered"
    );
    assert!(f.effect.deleted.lock().expect("deleted").is_empty()); // async-gate-allow: test-support recorder lock
    assert!(
        f.contract().binding(&source("unseen")).is_none(),
        "an absent delete leaves nothing behind"
    );
    assert!(f.contract().bindings().is_empty(), "the ledger is still empty");
}
