//! One Guest agent end-to-end over a faked vsock transport.
//!
//! The test drives the reference Guest agent - the security-key frontend - on
//! the toolkit's Guest base, enrolling through the landed `zone-bootstrap` and
//! `zone-enroll` handlers of a real [`ZoneServiceServer`]. Only the socket is
//! faked: both sides carry the contract's native-vsock framing over an
//! in-memory duplex, so the enrollment, the admission consumption, the served
//! frame, and the drain are the production paths.

use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_contracts_resource::v3::{ResourceUid, ZoneId};
use d2b_contracts_zone_session::v3::component_session::LimitProfile;
use d2b_contracts_zone_session::v3::zone_routing::{
    ZoneLabelId, ZoneLinkControllerGeneration, ZonePath, ZoneTreeEdge,
};
use d2b_contracts_zone_session::v3::zone_session::{
    ZoneBootstrapCall, ZoneBootstrapReply, ZoneEnrollCall, ZoneEnrollReply, ZoneEnrollmentIdentity,
};
use d2b_provider_toolkit::{
    AllocatorEnrollment, GuestError, GuestLink, GuestLinkFuture, GuestPlacement,
    run_guest,
};
use d2b_session::{OwnedTransport, TransportPacket};
use d2b_zone_routing::enrollment::{ZoneEnrollmentAuthority, ZoneEnrollmentExpectation};
use d2b_zone_routing::service::{ZoneBootstrapRequest, ZoneEnrollRequest, ZoneServiceServer};

const LINK_UID: &str = "11111111-1111-4111-8111-111111111111";
const SCHEMA_FINGERPRINT: [u8; 32] = [0x11; 32];
const CHANNEL_BINDING: [u8; 32] = [0x22; 32];
const PEER_FINGERPRINT: [u8; 32] = [0x33; 32];
const ISSUED_AT_UNIX_MS: u64 = 1_700_000_000_000;
const NOW_UNIX_MS: u64 = 1_700_000_000_500;
const REPORT: [u8; 64] = [0xab; 64];

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

/// The allocator side of the faked link: a real Zone service over a duplex.
async fn serve_allocator(stream: tokio::io::DuplexStream) {
    let now = Arc::new(AtomicU64::new(NOW_UNIX_MS));
    let clock: Arc<dyn Fn() -> u64 + Send + Sync> = {
        let now = Arc::clone(&now);
        Arc::new(move || now.load(Ordering::Acquire))
    };
    let authority = ZoneEnrollmentAuthority::with_lifetime(clock, 30_000).expect("an authority");
    let expected = expectation();
    let mut server = ZoneServiceServer::new(zone_path(&["k0"]), vec![edge()])
        .expect("a sealed topology");
    let mut transport = d2b_session_unix::FramedVsockTransport::new(stream);

    let bootstrap = transport
        .receive(64 * 1024)
        .await
        .expect("a bootstrap call");
    let call = ZoneBootstrapCall::decode(bootstrap.as_bytes()).expect("a bootstrap call");
    assert_eq!(call.issuance, 1);
    let (verifier, evidence) = authority.issue(expected.clone()).expect("issued");
    let request = ZoneBootstrapRequest::new(call, NOW_UNIX_MS)
        .with_runtime_admission(verifier, evidence, &expected)
        .expect("a runtime admission");
    let reply = server.zone_bootstrap(&request);
    assert!(matches!(reply, ZoneBootstrapReply::Admitted { .. }));
    transport
        .send(TransportPacket::new(reply.encode().expect("an encodable reply")))
        .await
        .expect("the bootstrap reply is sent");

    let enroll = transport.receive(64 * 1024).await.expect("an enroll call");
    let call = ZoneEnrollCall::decode(enroll.as_bytes()).expect("an enroll call");
    assert_eq!(call.observed_peer_fingerprint, PEER_FINGERPRINT);
    let (verifier, evidence) = authority.issue(expected.clone()).expect("issued");
    let request = ZoneEnrollRequest::new(call, NOW_UNIX_MS)
        .with_runtime_admission(verifier, evidence, &expected)
        .expect("a runtime admission");
    let reply = server.zone_enroll(&request);
    assert_eq!(
        reply,
        ZoneEnrollReply::Enrolled {
            zone: expected.zone().clone(),
            generation: 1,
        }
    );
    transport
        .send(TransportPacket::new(reply.encode().expect("an encodable reply")))
        .await
        .expect("the enrollment reply is sent");

    // The enrolled session now carries one CTAPHID report to the frontend.
    transport
        .send(TransportPacket::new(REPORT.to_vec()))
        .await
        .expect("the report is sent");

    // Close the session: the allocator owns its end of the link, and the base
    // drains once the session ends.
    drop(transport);
    assert_eq!(server.in_flight(), 0);
    assert_eq!(server.audit_events().count(), 2, "one record per handler");
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
            let transport: Box<dyn OwnedTransport> =
                Box::new(d2b_session_unix::FramedVsockTransport::new(stream));
            Ok(transport)
        })
    }
}

#[test]
fn the_security_key_frontend_enrolls_serves_and_drains_over_a_faked_vsock_transport() {
    let (client, server) = tokio::io::duplex(64 * 1024);
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
    assert!(GuestPlacement::new(identity(), 0, 300_000, ISSUED_AT_UNIX_MS, PEER_FINGERPRINT).is_err());
}
