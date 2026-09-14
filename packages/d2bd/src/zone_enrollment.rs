//! The daemon's guest-enrollment serving.
//!
//! A Guest agent enrolls by dialing the host: the security-key frontend's
//! `SK_VSOCK_PORT` default and the security-key guest module's `vsockPort`
//! option both declare the guest-to-host port as 14320, and the port is
//! excluded from the `d2b-link` ZoneLink allocation range, so no allocator
//! hands it to a link. A guest-to-host connection on that port arrives,
//! through Cloud Hypervisor's hybrid vsock, on the host side of the guest's
//! own socket family - `<vm vsock socket>_<port>` - and this module binds that
//! endpoint per guest and serves it with [`ZoneEnrollmentServer`].
//!
//! # What declares an endpoint
//!
//! Nothing is bound for a guest whose enrollment facts are incomplete. An
//! endpoint comes up only for a guest that has all of:
//!
//! - a committed `ZoneLink` row in the guest's own Zone whose transport
//!   settings name that Guest. `guestRef` is the vsock transport's own
//!   declared setting and no other transport Provider in the tree names a
//!   Guest in its settings, so a row carrying one is an enrollment link;
//! - the guest's provisioned identity: the host-published session descriptor
//!   and enrolled public key under the VM state root, which are the facts the
//!   allocator itself pinned;
//! - the trusted VM intent that resolves the guest's vsock socket.
//!
//! A deployment that declares no such guest enrollment binds nothing and is
//! unchanged.
//!
//! # The expectation
//!
//! Every field of the enrollment expectation is composed from those committed
//! facts, never from the wire: the link identity, the parent/child edge, and
//! the controller generation come from the committed `ZoneLink` row and the
//! sealed topology; the pinned peer fingerprint is the digest of the guest's
//! own enrolled public key; the session profile is the guest identity's own
//! lowered enrolled-session profile; and the opaque allocator binding is a
//! digest of the placement this daemon committed. The wire carries only the
//! identity the peer claims and the keys it observed, and the handler refuses
//! anything that does not match the composed expectation.

use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixListener as StdUnixListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use d2b_contracts_resource::v3::{ResourceUid, ZoneId};
use d2b_contracts_zone_session::v3::ZoneLinkSpec;
use d2b_contracts_zone_session::v3::zone_routing::{
    ZoneLinkControllerGeneration, ZonePath, ZoneTreeEdge,
};
use d2b_contracts_zone_session::v3::zone_session::{ZoneEnrollmentIdentity, ZoneEnrollmentRefusal};
use d2b_session_unix::FramedVsockTransport;
use d2b_zone_routing::enrollment::ZoneEnrollmentExpectation;
use d2b_zone_routing::serving::{ZoneEnrollmentPlacements, ZoneEnrollmentServer};
use sha2::{Digest, Sha256};
use tokio::net::UnixListener;
use tokio::sync::Mutex;

use d2bd_runtime::guest_mode::GuestIdentity;

/// The guest-to-host port the enrollment is served on.
///
/// No service package in the tree declares an enrollment port: the number the
/// tree declares twice is the security-key frontend's `D2B_SK_VSOCK_PORT`
/// default and the security-key guest module's `vsockPort` option default, and
/// it is outside the `d2b-link` ZoneLink allocation range (`14420-14499`), so
/// it can never be handed to a link either.
pub(crate) const ZONE_ENROLLMENT_PORT: u32 = 14_320;

/// Socket mode for one bound enrollment endpoint.
///
/// The hypervisor that bridges the guest's connection is not the daemon, so
/// the endpoint is group-readable and group-writable exactly as the in-tree
/// guest-to-host vsock relays are.
const ZONE_ENROLLMENT_SOCKET_MODE: u32 = 0o660;

/// One guest's enrollment endpoint, composed from committed declarations.
#[derive(Clone)]
pub(crate) struct GuestEnrollmentEndpoint {
    vsock_host_socket: PathBuf,
    expectation: ZoneEnrollmentExpectation,
    socket_owner: (u32, u32),
}

impl GuestEnrollmentEndpoint {
    /// Compose one endpoint from the committed facts of one enrollment link.
    ///
    /// Returns the expectation's own refusal when the facts cannot express
    /// the contract's enrolled-session profile, so an incomplete placement
    /// never becomes a bound endpoint.
    pub(crate) fn new(
        zone: ZoneId,
        link_uid: ResourceUid,
        edge: ZoneTreeEdge,
        controller_generation: ZoneLinkControllerGeneration,
        vsock_host_socket: PathBuf,
        identity: &GuestIdentity,
        guest_public: [u8; 32],
        socket_owner: (u32, u32),
    ) -> Result<Self, ZoneEnrollmentRefusal> {
        let profile = identity.endpoint_policy();
        let expectation = ZoneEnrollmentExpectation::for_enrolled_guest_session(
            zone,
            link_uid,
            edge.clone(),
            controller_generation.clone(),
            peer_key_fingerprint(guest_public),
            allocator_binding(&edge, &controller_generation, identity),
            profile.schema_fingerprint,
            profile.transport_binding.channel_binding,
            identity.reconnect_generation(),
            profile.limits,
        )?;
        Ok(Self {
            vsock_host_socket,
            expectation,
            socket_owner,
        })
    }

    /// Whether the link identity a peer named is this endpoint's link.
    ///
    /// Only the link is compared here: the full identity is the handler's
    /// comparison, so a peer that names this link with a substituted session
    /// profile is refused by the handler's own closed reason rather than
    /// silently matched to another placement.
    fn admits(&self, identity: &ZoneEnrollmentIdentity) -> bool {
        self.expectation.admits_link(identity)
    }

    /// The host-side path of this guest's enrollment endpoint.
    pub(crate) fn socket_path(&self) -> PathBuf {
        vsock_endpoint_path(&self.vsock_host_socket)
    }
}

/// The declared facts of one enrollment link, read from a committed row.
pub(crate) struct EnrollmentLinkFacts {
    /// The committed ZoneLink resource identity the enrollment is for.
    pub(crate) link_uid: ResourceUid,
    /// The controller generation that authorized the link.
    pub(crate) controller_generation: ZoneLinkControllerGeneration,
    /// The Guest the link's transport settings name.
    pub(crate) guest_name: String,
}

/// Read the enrollment facts one committed `ZoneLink` row declares.
///
/// Returns `None` for a row this daemon does not serve: a row of another
/// Zone, a row whose transport settings name no Guest (the vsock transport's
/// own `guestRef` is the only transport setting in the tree that names one),
/// or a row whose identity cannot be resolved. A row that is not served is
/// skipped rather than partially served.
pub(crate) fn declared_enrollment_link(
    link: &serde_json::Value,
    zone: &ZoneId,
) -> Option<EnrollmentLinkFacts> {
    let spec: ZoneLinkSpec = serde_json::from_value(link.get("spec")?.clone()).ok()?;
    if spec.validate_child_zone(zone).is_err() {
        return None;
    }
    let settings: serde_json::Value =
        serde_json::from_slice(&spec.transport_settings().to_canonical_bytes()).ok()?;
    let guest_name = settings
        .get("guestRef")?
        .as_str()?
        .strip_prefix("Guest/")?
        .to_owned();
    if guest_name.is_empty() {
        return None;
    }
    let link_uid = link
        .get("metadata")
        .and_then(|metadata| metadata.get("uid"))
        .and_then(serde_json::Value::as_str)
        .and_then(|value| ResourceUid::parse(value.to_owned()).ok())?;
    let controller_generation = ZoneLinkControllerGeneration::parse(format!(
        "zonelink-{}",
        link_uid.as_str().replace('-', "")
    ))
    .ok()?;
    Some(EnrollmentLinkFacts {
        link_uid,
        controller_generation,
        guest_name,
    })
}

/// Bind and serve every declared enrollment endpoint of one Zone.
///
/// Returns how many endpoints were bound. The Zone's runtime is shared by all
/// of them, because the enrollment state machine of each child link lives in
/// it and two connections for one link must not race.
pub(crate) fn serve_guest_enrollments(
    local_root: ZonePath,
    edges: Vec<ZoneTreeEdge>,
    endpoints: Vec<GuestEnrollmentEndpoint>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
) -> usize {
    if endpoints.is_empty() {
        return 0;
    }
    let placements: ZoneEnrollmentPlacements = {
        let endpoints = endpoints.clone();
        Arc::new(move |identity: &ZoneEnrollmentIdentity| {
            endpoints
                .iter()
                .find(|endpoint| endpoint.admits(identity))
                .map(|endpoint| endpoint.expectation.clone())
        })
    };
    let server = match ZoneEnrollmentServer::new(local_root, edges, clock, placements) {
        Ok(server) => Arc::new(Mutex::new(server)),
        Err(error) => {
            tracing::warn!(
                error = ?error,
                "guest enrollment refused: the sealed topology was refused"
            );
            return 0;
        }
    };
    let mut bound = 0;
    for endpoint in endpoints {
        let path = endpoint.socket_path();
        match bind_enrollment_endpoint(&path, endpoint.socket_owner) {
            Ok(listener) => {
                tracing::info!(endpoint = %path.display(), "guest enrollment endpoint bound");
                spawn_accept_loop(listener, Arc::clone(&server), path);
                bound += 1;
            }
            Err(error) => {
                tracing::warn!(
                    endpoint = %path.display(),
                    error = %error,
                    "guest enrollment endpoint refused"
                );
            }
        }
    }
    bound
}

/// Accept guest enrollment connections on one bound endpoint.
///
/// Binding needs no reactor and the endpoint is therefore handed over as the
/// standard-library listener; accepting does, so the reactor enters here,
/// where the daemon's runtime already is.
fn spawn_accept_loop(
    listener: StdUnixListener,
    server: Arc<Mutex<ZoneEnrollmentServer>>,
    path: PathBuf,
) {
    tokio::spawn(async move {
        if let Err(error) = listener.set_nonblocking(true) {
            tracing::warn!(
                endpoint = %path.display(),
                error = %error,
                "guest enrollment endpoint could not be made nonblocking"
            );
            return;
        }
        let listener = match UnixListener::from_std(listener) {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!(
                    endpoint = %path.display(),
                    error = %error,
                    "guest enrollment endpoint refused by the runtime"
                );
                return;
            }
        };
        loop {
            let (stream, _peer) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(error) => {
                    // An endpoint that cannot accept is not retried into a hot
                    // loop: it ends here, and the daemon's next start binds it
                    // again with a live socket.
                    tracing::warn!(
                        endpoint = %path.display(),
                        error = %error,
                        "guest enrollment endpoint stopped accepting"
                    );
                    break;
                }
            };
            let server = Arc::clone(&server);
            let path = path.clone();
            tokio::spawn(async move {
                let mut transport = FramedVsockTransport::new(stream);
                let mut server = server.lock().await;
                if let Err(error) = server.serve(&mut transport).await {
                    tracing::debug!(
                        endpoint = %path.display(),
                        reason = error.as_str(),
                        "guest enrollment connection ended without enrolling"
                    );
                }
            });
        }
    });
}

/// Bind one endpoint, replacing a socket no live listener holds.
///
/// The bind is a plain socket operation, so it needs no reactor and is
/// callable from any context; the accept loop converts the listener where the
/// runtime is.
fn bind_enrollment_endpoint(path: &Path, owner: (u32, u32)) -> std::io::Result<StdUnixListener> {
    replace_stale_socket(path)?;
    let listener = StdUnixListener::bind(path)?;
    set_socket_owner(path, owner);
    Ok(listener)
}

/// Remove one stale endpoint socket, refusing while a live listener holds it
/// or while the path is not a socket at all.
///
/// A daemon restart leaves the previous socket file behind. It is replaced
/// only after proving nothing answers on it, so a restarted daemon recovers
/// its endpoint and a running one is never displaced.
fn replace_stale_socket(path: &Path) -> std::io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_socket() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "enrollment endpoint path is not a socket",
        ));
    }
    if std::os::unix::net::UnixStream::connect(path).is_ok() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "enrollment endpoint is already served",
        ));
    }
    std::fs::remove_file(path)
}

/// Hand one bound endpoint to the owner the hypervisor connects as.
///
/// The endpoint lives beside the guest's own vsock socket, so the identity
/// that bridges it is the state root's owner. A failure here is reported and
/// not fatal: the endpoint stays bound, and the refusal a connection then hits
/// is the kernel's permission check, not a silently widened one.
fn set_socket_owner(path: &Path, owner: (u32, u32)) {
    if let Err(error) =
        std::fs::set_permissions(path, PermissionsExt::from_mode(ZONE_ENROLLMENT_SOCKET_MODE))
    {
        tracing::warn!(endpoint = %path.display(), error = %error, "enrollment endpoint mode refused");
    }
    if let Err(error) = std::os::unix::fs::chown(path, Some(owner.0), Some(owner.1)) {
        tracing::warn!(endpoint = %path.display(), error = %error, "enrollment endpoint ownership refused");
    }
}

/// The host-side endpoint path of one guest's vsock socket family.
///
/// Cloud Hypervisor bridges a guest-to-host connection on port `P` to
/// `<sock>_P`, which is the shape every in-tree guest-to-host relay declares.
fn vsock_endpoint_path(vsock_host_socket: &Path) -> PathBuf {
    let mut path = vsock_host_socket.as_os_str().to_owned();
    path.push(format!("_{ZONE_ENROLLMENT_PORT}"));
    PathBuf::from(path)
}

/// The digest of one guest's enrolled public key.
///
/// This is the fingerprint the allocator pinned and the agent presents; the
/// host derives it from the key material it published for that guest.
fn peer_key_fingerprint(guest_public: [u8; 32]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(guest_public);
    digest.finalize().into()
}

/// The allocator's opaque digest of the placement it committed.
///
/// The binding is never rendered and never compared against anything the wire
/// states; it is the allocator's own record that one enrollment was
/// authorized by one committed placement, and it must stay stable across a
/// daemon restart, so it is a digest of the committed facts rather than a
/// random or counter-derived value.
fn allocator_binding(
    edge: &ZoneTreeEdge,
    controller_generation: &ZoneLinkControllerGeneration,
    identity: &GuestIdentity,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b-guest-enrollment-allocator-binding-v3\0");
    digest.update(edge.parent().to_storage_string().as_bytes());
    digest.update(edge.child().to_storage_string().as_bytes());
    digest.update(controller_generation.as_str().as_bytes());
    digest.update(identity.guest_ref().to_canonical_string().as_bytes());
    digest.update(identity.guest_uid().as_str().as_bytes());
    digest.update(identity.schema_fingerprint().as_str().as_bytes());
    digest.update(identity.reconnect_generation().get().to_be_bytes());
    digest.update(identity.provider_generation().to_be_bytes());
    digest.update(identity.controller_generation().to_be_bytes());
    digest.update(identity.assignment_epoch().to_be_bytes());
    digest.finalize().into()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::MetadataExt;

    use d2b_contracts_resource::v3::ResourceRef;
    use d2b_contracts_resource::v3::identity::{
        ReconnectGeneration, SchemaFingerprint, SessionPurpose,
    };
    use d2b_contracts_zone_session::v3::zone_routing::ZoneLabelId;
    use d2b_contracts_zone_session::v3::zone_session::{
        ZoneBootstrapCall, ZoneBootstrapReply, ZoneEnrollCall, ZoneEnrollReply,
    };
    use d2b_session::{OwnedTransport, TransportPacket};

    use super::*;
    use d2bd_runtime::guest_mode::{BootIdentity, GUEST_COMPONENT_SESSION_PURPOSE};

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
        ZoneTreeEdge::new(zone_path(&["k0"]), zone_path(&["k1", "k0"]))
            .expect("a direct child edge")
    }

    fn identity() -> GuestIdentity {
        GuestIdentity::new(
            ResourceRef::parse("Guest/sk-vm").expect("a valid guest ref"),
            ResourceUid::parse("11111111-1111-4111-8111-111111111111").expect("a valid UID"),
            ZoneId::parse("zone-k1").expect("a valid zone"),
            BootIdentity::from_digest(
                "sha256:2222222222222222222222222222222222222222222222222222222222222222",
            )
            .expect("a valid boot identity"),
            SessionPurpose::parse(GUEST_COMPONENT_SESSION_PURPOSE).expect("a valid purpose"),
            SchemaFingerprint::parse(
                "sha256:1111111111111111111111111111111111111111111111111111111111111111",
            )
            .expect("a valid schema fingerprint"),
            ReconnectGeneration::new(7).expect("a valid generation"),
            3,
            4,
            5,
        )
        .expect("the guest identity's own fields are valid")
    }

    fn endpoint() -> GuestEnrollmentEndpoint {
        GuestEnrollmentEndpoint::new(
            ZoneId::parse("zone-k1").expect("a valid zone"),
            ResourceUid::parse("33333333-3333-4333-8333-333333333333").expect("a valid UID"),
            edge(),
            ZoneLinkControllerGeneration::parse("controller-1").expect("a valid generation"),
            PathBuf::from("/var/lib/d2b/vms/sk-vm/vsock.sock"),
            &identity(),
            [0x33; 32],
            (1000, 1000),
        )
        .expect("the contract's enrolled guest session profile")
    }

    fn named_identity(endpoint: &GuestEnrollmentEndpoint) -> ZoneEnrollmentIdentity {
        ZoneEnrollmentIdentity {
            zone_link_uid: endpoint.expectation.zone_link_uid().clone(),
            edge: endpoint.expectation.edge().clone(),
            controller_generation: endpoint.expectation.controller_generation().clone(),
            reconnect_generation: ReconnectGeneration::new(
                endpoint.expectation.session_policy().reconnect_generation,
            )
            .expect("a valid generation"),
            schema_fingerprint: endpoint.expectation.session_policy().schema_fingerprint,
        }
    }

    #[test]
    fn the_endpoint_path_is_the_guest_socket_family_plus_the_declared_port() {
        let endpoint = endpoint();
        assert_eq!(
            endpoint.socket_path(),
            PathBuf::from("/var/lib/d2b/vms/sk-vm/vsock.sock_14320")
        );
    }

    #[test]
    fn an_endpoint_admits_exactly_its_own_link_and_nothing_else() {
        let endpoint = endpoint();
        assert!(endpoint.admits(&named_identity(&endpoint)));
        let mut substituted = named_identity(&endpoint);
        substituted.zone_link_uid =
            ResourceUid::parse("44444444-4444-4444-8444-444444444444").expect("a valid UID");
        assert!(!endpoint.admits(&substituted));
        let mut other_edge = named_identity(&endpoint);
        other_edge.edge = ZoneTreeEdge::new(zone_path(&["k0"]), zone_path(&["k2", "k0"]))
            .expect("a direct child edge");
        assert!(!endpoint.admits(&other_edge));
    }

    #[test]
    fn a_vsock_link_row_declares_the_guest_its_enrollment_places() {
        let link = enrollment_link_row(serde_json::json!({
            "guestRef": "Guest/sk-vm",
            "portClass": "d2b-link",
            "connectTimeoutSeconds": 30,
        }));
        let facts =
            declared_enrollment_link(&link, &ZoneId::parse("zone-k1").expect("a valid zone"))
                .expect("a declared enrollment link");
        assert_eq!(facts.guest_name, "sk-vm");
        assert_eq!(
            facts.link_uid.as_str(),
            "33333333-3333-4333-8333-333333333333"
        );
        assert_eq!(
            facts.controller_generation.as_str(),
            "zonelink-33333333333343338333333333333333"
        );
    }

    #[test]
    fn a_row_of_another_zone_or_without_a_guest_is_not_an_enrollment_link() {
        let zone = ZoneId::parse("zone-k1").expect("a valid zone");
        let mut link = enrollment_link_row(serde_json::json!({ "guestRef": "Guest/sk-vm" }));
        assert!(
            declared_enrollment_link(&link, &ZoneId::parse("zone-k2").expect("a valid zone"))
                .is_none(),
            "a row of another Zone is never this Zone's enrollment"
        );
        assert!(declared_enrollment_link(&link, &zone).is_some());

        // A relay link's settings name no Guest, so it is not served here.
        link["spec"]["transportSettings"] = serde_json::json!({
            "relayNamespaceId": "ns",
            "relayEntityId": "link",
        });
        assert!(declared_enrollment_link(&link, &zone).is_none());

        // A settings object with no Guest at all is not an enrollment link.
        link["spec"]["transportSettings"] = serde_json::json!({ "portClass": "d2b-link" });
        assert!(declared_enrollment_link(&link, &zone).is_none());
    }

    /// One committed enrollment-link row with the given transport settings.
    fn enrollment_link_row(transport_settings: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "metadata": {
                "name": "sk-link",
                "zone": "zone-k1",
                "uid": "33333333-3333-4333-8333-333333333333",
            },
            "spec": {
                "childZoneName": "zone-k1",
                "transportProviderRef": "Provider/transport-vsock",
                "transportSettings": transport_settings,
                "transportCredentials": [],
                "disabled": false,
            },
        })
    }

    /// The clock the served runtime timestamps with, and the issuance the
    /// client presents: an issuance inside its lifetime at that clock.
    const NOW_UNIX_MS: u64 = 1_700_000_000_500;
    const ISSUED_AT_UNIX_MS: u64 = 1_700_000_000_000;
    const FRAME_LIMIT_BYTES: usize = 64 * 1024;

    #[tokio::test]
    async fn a_guest_enrolls_over_the_bound_endpoint() {
        let root =
            std::env::temp_dir().join(format!("d2b-zone-enrollment-serve-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary root");
        let endpoint = GuestEnrollmentEndpoint::new(
            ZoneId::parse("zone-k1").expect("a valid zone"),
            ResourceUid::parse("33333333-3333-4333-8333-333333333333").expect("a valid UID"),
            edge(),
            ZoneLinkControllerGeneration::parse("controller-1").expect("a valid generation"),
            root.join("vsock.sock"),
            &identity(),
            [0x33; 32],
            state_root_owner(&root),
        )
        .expect("the contract's enrolled guest session profile");
        assert_eq!(
            serve_guest_enrollments(
                zone_path(&["k0"]),
                vec![edge()],
                vec![endpoint.clone()],
                Arc::new(|| NOW_UNIX_MS),
            ),
            1,
            "one declared endpoint is bound"
        );

        // The peer side is the guest agent's own wire: one bounded bootstrap
        // call, then one bounded enroll call, both decoded by the runtime.
        let mut client = FramedVsockTransport::new(
            tokio::net::UnixStream::connect(endpoint.socket_path())
                .await
                .expect("the bound endpoint accepts"),
        );
        let bootstrap =
            ZoneBootstrapCall::new(named_identity(&endpoint), 1, 300_000, ISSUED_AT_UNIX_MS)
                .encode()
                .expect("an encodable bootstrap call");
        client
            .send(TransportPacket::new(bootstrap))
            .await
            .expect("the bootstrap call crosses");
        let reply = client
            .receive(FRAME_LIMIT_BYTES)
            .await
            .expect("a bootstrap reply");
        assert_eq!(
            ZoneBootstrapReply::decode(reply.as_bytes()).expect("a bootstrap reply"),
            ZoneBootstrapReply::Admitted {
                expires_at_unix_ms: ISSUED_AT_UNIX_MS + 300_000
            }
        );

        let enroll = ZoneEnrollCall::new(
            named_identity(&endpoint),
            peer_key_fingerprint([0x33; 32]),
            NOW_UNIX_MS,
        )
        .encode()
        .expect("an encodable enroll call");
        client
            .send(TransportPacket::new(enroll))
            .await
            .expect("the enroll call crosses");
        let reply = client
            .receive(FRAME_LIMIT_BYTES)
            .await
            .expect("an enroll reply");
        assert_eq!(
            ZoneEnrollReply::decode(reply.as_bytes()).expect("an enroll reply"),
            ZoneEnrollReply::Enrolled {
                zone: ZoneId::parse("zone-k1").expect("a valid zone"),
                generation: 1,
            }
        );

        let _ = std::fs::remove_file(endpoint.socket_path());
        let _ = std::fs::remove_dir(&root);
    }

    #[test]
    fn a_stale_endpoint_socket_is_replaced_and_a_live_one_is_left_alone() {
        let root = std::env::temp_dir().join(format!("d2b-zone-enrollment-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary root");
        let owner = state_root_owner(&root);
        let path = root.join("vsock.sock_14320");
        let _ = std::fs::remove_file(&path);

        let live = bind_enrollment_endpoint(&path, owner).expect("the first bind succeeds");
        let refused = bind_enrollment_endpoint(&path, owner);
        assert!(refused.is_err(), "a live endpoint is never displaced");

        drop(live);
        bind_enrollment_endpoint(&path, owner)
            .expect("the stale endpoint is replaced after the owner goes away");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&root);
    }

    #[test]
    fn a_non_socket_endpoint_path_is_refused_rather_than_removed() {
        let root =
            std::env::temp_dir().join(format!("d2b-zone-enrollment-file-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("a temporary root");
        let path = root.join("vsock.sock_14320");
        std::fs::write(&path, b"not a socket").expect("a file at the endpoint path");

        assert!(bind_enrollment_endpoint(&path, state_root_owner(&root)).is_err());
        assert!(path.exists(), "a non-socket path is never removed");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&root);
    }

    /// The owner a bound endpoint is handed to: the state root's own owner,
    /// which for a test root is this process.
    fn state_root_owner(root: &Path) -> (u32, u32) {
        let metadata = std::fs::metadata(root).expect("a temporary root");
        (metadata.uid(), metadata.gid())
    }
}
