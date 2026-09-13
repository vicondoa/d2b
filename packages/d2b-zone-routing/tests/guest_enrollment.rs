//! Guest enrollment served over a faked vsock transport.
//!
//! The end-to-end test drives the reference Guest agent - the security-key
//! frontend - on the toolkit's Guest base, enrolling through the production
//! serving runtime ([`ZoneEnrollmentServer`]) over the real `zone-bootstrap`
//! and `zone-enroll` handlers. Only the socket is faked: both sides carry the
//! contract's native-vsock framing over an in-memory duplex, so the
//! enrollment, the placement lookup, the admission minting and consumption,
//! the served frame, and the drain are the production paths.
//!
//! The remaining tests drive the same runtime directly, because they are about
//! what the runtime refuses: an absent placement, a revoked authority, a
//! replayed bootstrap, a frame that is not a call, and the bounded call
//! allowance.

use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use d2b_bus::session::ZoneLinkState;
use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_contracts_resource::v3::{ResourceUid, ZoneId};
use d2b_contracts_zone_session::v3::component_session::LimitProfile;
use d2b_contracts_zone_session::v3::zone_routing::{
    ZoneLabelId, ZoneLinkControllerGeneration, ZonePath, ZoneTreeEdge,
};
use d2b_contracts_zone_session::v3::zone_session::{
    ZoneBootstrapCall, ZoneBootstrapReply, ZoneEnrollCall, ZoneEnrollReply, ZoneEnrollmentIdentity,
    ZoneEnrollmentRefusal,
};
use d2b_provider_toolkit::{
    AllocatorEnrollment, GuestError, GuestLink, GuestLinkFuture, GuestPlacement, run_guest,
};
use d2b_session::{OwnedTransport, TransportPacket};
use d2b_session_unix::FramedVsockTransport;
use d2b_zone_routing::enrollment::ZoneEnrollmentExpectation;
use d2b_zone_routing::serving::{
    ZONE_ENROLLMENT_CALLS_MAX, ZoneEnrollmentPlacements, ZoneEnrollmentServeError,
    ZoneEnrollmentServer,
};

const LINK_UID: &str = "11111111-1111-4111-8111-111111111111";
const SCHEMA_FINGERPRINT: [u8; 32] = [0x11; 32];
const CHANNEL_BINDING: [u8; 32] = [0x22; 32];
const PEER_FINGERPRINT: [u8; 32] = [0x33; 32];
const ISSUED_AT_UNIX_MS: u64 = 1_700_000_000_000;
const NOW_UNIX_MS: u64 = 1_700_000_000_500;
const REPORT: [u8; 64] = [0xab; 64];
const FRAME_LIMIT: usize = 64 * 1024;

fn zone_path(labels: &[&str]) -> ZonePath {
    ZonePath::new(
        labels
            .iter()
            .map(|label| ZoneLabelId::parse(*label).expect("a valid label"))
            .collect(),
    )
    .expect("a valid zone path")
}

fn edge() -> ZoneTreeEdge {
    ZoneTreeEdge::new(zone_path(&["k0"]), zone_path(&["k1", "k0"])).expect("a direct child edge")
}

fn identity() -> ZoneEnrollmentIdentity {
    ZoneEnrollmentIdentity {
        zone_link_uid: ResourceUid::parse(LINK_UID).expect("a valid UID"),
        edge: edge(),
        controller_generation: ZoneLinkControllerGeneration::parse("controller-1")
            .expect("a valid generation"),
        reconnect_generation: ReconnectGeneration::new(7).expect("a valid generation"),
        schema_fingerprint: SCHEMA_FINGERPRINT,
    }
}

fn placement() -> GuestPlacement {
    GuestPlacement::new(identity(), 1, 300_000, ISSUED_AT_UNIX_MS, PEER_FINGERPRINT)
        .expect("a valid placement")
}

fn expectation() -> ZoneEnrollmentExpectation {
    ZoneEnrollmentExpectation::for_enrolled_guest_session(
        ZoneId::parse("zone-k1").expect("a valid zone"),
        ResourceUid::parse(LINK_UID).expect("a valid UID"),
        edge(),
        ZoneLinkControllerGeneration::parse("controller-1").expect("a valid generation"),
        PEER_FINGERPRINT,
        [0x44; 32],
        SCHEMA_FINGERPRINT,
        CHANNEL_BINDING,
        ReconnectGeneration::new(7).expect("a valid generation"),
        LimitProfile::remote_default(),
    )
    .expect("the contract's enrolled guest session profile")
}

fn clock() -> Arc<dyn Fn() -> u64 + Send + Sync> {
    Arc::new(|| NOW_UNIX_MS)
}

fn placements(expected: Option<ZoneEnrollmentExpectation>) -> ZoneEnrollmentPlacements {
    Arc::new(move |_| expected.clone())
}

fn runtime(placements: ZoneEnrollmentPlacements) -> ZoneEnrollmentServer {
    ZoneEnrollmentServer::new(zone_path(&["k0"]), vec![edge()], clock(), placements)
        .expect("a sealed topology")
}

fn bootstrap_bytes(issuance: u64) -> Vec<u8> {
    ZoneBootstrapCall::new(identity(), issuance, 300_000, ISSUED_AT_UNIX_MS)
        .encode()
        .expect("an encodable bootstrap call")
}

fn enroll_bytes() -> Vec<u8> {
    ZoneEnrollCall::new(identity(), PEER_FINGERPRINT, NOW_UNIX_MS)
        .encode()
        .expect("an encodable enroll call")
}

async fn send(transport: &mut FramedVsockTransport<tokio::io::DuplexStream>, bytes: Vec<u8>) {
    transport
        .send(TransportPacket::new(bytes))
        .await
        .expect("the call is sent");
}

/// Serve one connection on its own task.
///
/// The task owns the transport, so when the runtime stops serving the peer
/// observes the close exactly as it would from a real accept loop, and the
/// caller can drive the peer side concurrently.
fn serve_once(
    mut server: ZoneEnrollmentServer,
    stream: tokio::io::DuplexStream,
) -> tokio::task::JoinHandle<(ZoneEnrollmentServer, Result<(), ZoneEnrollmentServeError>)> {
    tokio::spawn(async move {
        let mut transport = FramedVsockTransport::new(stream);
        let outcome = server.serve(&mut transport).await;
        drop(transport);
        (server, outcome)
    })
}

async fn bootstrap_reply(
    transport: &mut FramedVsockTransport<tokio::io::DuplexStream>,
) -> ZoneBootstrapReply {
    let reply = transport.receive(FRAME_LIMIT).await.expect("a reply");
    ZoneBootstrapReply::decode(reply.as_bytes()).expect("a bootstrap reply")
}

async fn enroll_reply(
    transport: &mut FramedVsockTransport<tokio::io::DuplexStream>,
) -> ZoneEnrollReply {
    let reply = transport.receive(FRAME_LIMIT).await.expect("a reply");
    ZoneEnrollReply::decode(reply.as_bytes()).expect("an enroll reply")
}

/// The allocator side of the faked link: the production serving runtime.
async fn serve_allocator(stream: tokio::io::DuplexStream) {
    let mut server = runtime(placements(Some(expectation())));
    let mut transport = FramedVsockTransport::new(stream);
    server
        .serve(&mut transport)
        .await
        .expect("the frontend enrolled");
    assert_eq!(
        server.link_state(&zone_path(&["k1", "k0"])),
        Some(ZoneLinkState::Ready),
        "the enrolled link is tracked at Ready"
    );

    // The enrolled session now carries one CTAPHID report to the frontend.
    transport
        .send(TransportPacket::new(REPORT.to_vec()))
        .await
        .expect("the report is sent");

    // Close the session: the allocator owns its end of the link, and the base
    // drains once the session ends.
    drop(transport);
    assert_eq!(server.audit_events().count(), 2, "one record per handler");
}

#[test]
fn the_security_key_frontend_enrolls_serves_and_drains_over_a_faked_vsock_transport() {
    let (client, server) = tokio::io::duplex(FRAME_LIMIT);
    let injected = Arc::new(Mutex::new(Vec::new()));
    let allocator = {
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("the allocator runtime builds");
            runtime.block_on(serve_allocator(server));
        })
    };

    let agent = d2b_sk_frontend::SecurityKeyFrontend::new(FakeHidDevice {
        injected: Arc::clone(&injected),
    });
    let status = run_guest(
        agent,
        Box::new(DuplexLink {
            client: Mutex::new(Some(client)),
        }),
        Arc::new(AllocatorEnrollment::new(placement())),
    );

    assert_eq!(status, 0, "the guest lifecycle completes");
    allocator.join().expect("the allocator task completes");
    assert_eq!(
        injected.lock().expect("injected reports").as_slice(),
        &[REPORT],
        "the frontend injected exactly the report the enrolled session carried"
    );
}

#[test]
fn a_frontend_without_a_placement_is_refused_before_any_session() {
    // The placement is what the allocator minted; a zero fingerprint can never
    // be one, so the enrollment refuses before a frame crosses the link.
    assert!(GuestPlacement::new(identity(), 1, 300_000, ISSUED_AT_UNIX_MS, [0; 32]).is_err());
    assert!(
        GuestPlacement::new(identity(), 0, 300_000, ISSUED_AT_UNIX_MS, PEER_FINGERPRINT).is_err()
    );
}

#[tokio::test]
async fn an_absent_placement_refuses_the_named_admission_absent() {
    let (client, server_stream) = tokio::io::duplex(FRAME_LIMIT);
    let mut client = FramedVsockTransport::new(client);
    let serving = serve_once(runtime(placements(None)), server_stream);

    send(&mut client, bootstrap_bytes(1)).await;
    let reply = bootstrap_reply(&mut client).await;
    drop(client);
    let (server, outcome) = serving.await.expect("the serving task completes");

    assert_eq!(
        reply,
        ZoneBootstrapReply::Refused {
            reason: ZoneEnrollmentRefusal::AdmissionAbsent
        }
    );
    assert_eq!(outcome, Err(ZoneEnrollmentServeError::Transport));
    assert_eq!(
        server.link_state(&zone_path(&["k1", "k0"])),
        None,
        "a refused bootstrap leaves no tracked link"
    );
}

#[tokio::test]
async fn a_revoked_authority_refuses_the_named_policy_denial() {
    let (client, server_stream) = tokio::io::duplex(FRAME_LIMIT);
    let mut client = FramedVsockTransport::new(client);
    let server = runtime(placements(Some(expectation())));
    server.revoke();
    let serving = serve_once(server, server_stream);

    send(&mut client, bootstrap_bytes(1)).await;
    let reply = bootstrap_reply(&mut client).await;
    drop(client);
    let (server, outcome) = serving.await.expect("the serving task completes");

    assert_eq!(
        reply,
        ZoneBootstrapReply::Refused {
            reason: ZoneEnrollmentRefusal::PolicyDenial
        }
    );
    assert_eq!(outcome, Err(ZoneEnrollmentServeError::Transport));
    assert_eq!(server.link_state(&zone_path(&["k1", "k0"])), None);
}

#[tokio::test]
async fn an_admitted_bootstrap_refuses_a_replay_then_the_link_enrolls() {
    let (client, server_stream) = tokio::io::duplex(FRAME_LIMIT);
    let mut client = FramedVsockTransport::new(client);
    let serving = serve_once(runtime(placements(Some(expectation()))), server_stream);

    send(&mut client, bootstrap_bytes(1)).await;
    let admitted = bootstrap_reply(&mut client).await;
    // A second bootstrap on a link that is already past Unenrolled is the
    // state machine's refusal, not the runtime's: the FSM, not the frame
    // order, is what makes one bootstrap out of many admissible.
    send(&mut client, bootstrap_bytes(1)).await;
    let replayed = bootstrap_reply(&mut client).await;
    send(&mut client, enroll_bytes()).await;
    let enrolled = enroll_reply(&mut client).await;
    drop(client);
    let (server, outcome) = serving.await.expect("the serving task completes");

    assert_eq!(
        admitted,
        ZoneBootstrapReply::Admitted {
            expires_at_unix_ms: ISSUED_AT_UNIX_MS + 300_000
        }
    );
    assert_eq!(
        replayed,
        ZoneBootstrapReply::Refused {
            reason: ZoneEnrollmentRefusal::InvalidTransition
        }
    );
    assert_eq!(
        enrolled,
        ZoneEnrollReply::Enrolled {
            zone: ZoneId::parse("zone-k1").expect("a valid zone"),
            generation: 1,
        }
    );
    outcome.expect("the peer enrolled");
    assert_eq!(
        server.link_state(&zone_path(&["k1", "k0"])),
        Some(ZoneLinkState::Ready)
    );
}

#[tokio::test]
async fn a_frame_that_is_not_a_call_ends_the_connection() {
    let (client, server_stream) = tokio::io::duplex(FRAME_LIMIT);
    let mut client = FramedVsockTransport::new(client);
    let serving = serve_once(runtime(placements(Some(expectation()))), server_stream);

    send(&mut client, b"not an enrollment call".to_vec()).await;
    drop(client);
    let (_server, outcome) = serving.await.expect("the serving task completes");

    assert_eq!(outcome, Err(ZoneEnrollmentServeError::Malformed));
}

#[tokio::test]
async fn the_call_allowance_is_bounded() {
    let (client, server_stream) = tokio::io::duplex(FRAME_LIMIT);
    let mut client = FramedVsockTransport::new(client);
    let serving = serve_once(runtime(placements(None)), server_stream);

    let mut replies = 0_u32;
    loop {
        if client
            .send(TransportPacket::new(bootstrap_bytes(1)))
            .await
            .is_err()
        {
            break;
        }
        if client.receive(FRAME_LIMIT).await.is_err() {
            break;
        }
        replies += 1;
    }
    drop(client);
    let (_server, outcome) = serving.await.expect("the serving task completes");

    assert_eq!(
        outcome,
        Err(ZoneEnrollmentServeError::Exhausted),
        "the runtime stops at its bounded allowance"
    );
    assert_eq!(replies, ZONE_ENROLLMENT_CALLS_MAX);
}

/// The frontend's virtual HID device, standing in for `/dev/uhid`.
struct FakeHidDevice {
    injected: Arc<Mutex<Vec<[u8; 64]>>>,
}

#[async_trait::async_trait]
impl d2b_sk_frontend::HidDevice for FakeHidDevice {
    async fn open(_path: &Path, _vm_id: &str) -> io::Result<Self> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "test device"))
    }

    async fn read_report(&mut self) -> io::Result<Option<[u8; 64]>> {
        std::future::pending().await
    }

    async fn send_report(&mut self, report: &[u8; 64]) -> io::Result<()> {
        self.injected
            .lock()
            .expect("injected reports")
            .push(*report);
        Ok(())
    }
}

/// A Guest link over an in-memory duplex wearing the real vsock framing.
struct DuplexLink {
    client: Mutex<Option<tokio::io::DuplexStream>>,
}

impl GuestLink for DuplexLink {
    fn connect(&self) -> GuestLinkFuture {
        let client = self.client.lock().expect("link").take();
        Box::pin(async move {
            let stream = client.ok_or(GuestError::LinkUnavailable)?;
            let transport: Box<dyn OwnedTransport> = Box::new(FramedVsockTransport::new(stream));
            Ok(transport)
        })
    }
}
