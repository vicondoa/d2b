//! The display session's durable child derivation and its endpoint authority
//! vocabulary (U12, U26).
//!
//! One admitted `WaylandSession` owns two worker Process rows - the host
//! proxy and the guest frontend - and each worker's private Endpoint. The
//! derivation of those durable child rows is display vocabulary: the worker
//! templates, the private endpoint shapes, and the restart-generation
//! annotation all belong to this crate. The derivation was authored on the
//! daemon side of the family's child-intent port; it moved here with the
//! family's effects, so the session driver (through its own crate's child
//! source) and the family's effects service both read the same derivation.
//!
//! Every host connection the workers make is a typed `EndpointBinding`
//! relationship over an exact `Endpoint` row, never a name, an environment
//! variable, or a directory. The host proxy reaches the host compositor
//! through the session's compositor Endpoint and no other socket; the guest
//! frontend reaches the proxy through the proxy's own Endpoint. Those two
//! relationships are the ONLY ones the display graph has, and they exist
//! because each `Endpoint` row's own publication intent names its consumer -
//! the `Endpoint` driver derives the canonical `EndpointBinding` row from
//! that intent, and a session gates on the delivery those rows publish. The
//! display graph keeps no second description of them: a slot table here would
//! be a second authority that could name a different endpoint, consumer, or
//! generation than the committed rows do.
//!
//! The derivation is pure: it builds payloads and references from the
//! session's row identity and spec, and never touches host state.
use std::collections::BTreeMap;
use std::sync::RwLock;

use d2b_contracts_resource::v3::{
    CanonicalJsonValue, RESOURCE_ENVELOPE_DOMAIN_TAG, ResourceRef, ResourceUid, ZoneId,
    canonical_digest,
    execution_policy::{BoundedText, BoundedToken, BudgetSpec},
    process::{EnvironmentClass, ExecutionSpec, ProcessClass, ProcessSpec, SandboxSpec, TelemetrySpec},
};
use d2b_core_controller::OwnedChildIntent;
use d2b_provider_endpoint::endpoint::{
    EndpointAttachmentPolicy, EndpointBindingPublication, EndpointClass, EndpointConsumerPolicy,
    EndpointLifecyclePolicy, EndpointLocality, EndpointOperation, EndpointSpec, EndpointTransport,
    EndpointVisibility,
};
use d2b_provider_endpoint::{CommittedEndpointShape, EndpointPurposeVocabulary};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{DisplayProcessRole, WaylandSessionResourceStatus, WaylandSessionSpec, WorkerEffectError};

/// The restart-generation annotation the display session's worker rows
/// carry: the durable generation the display supervisor restarts a worker
/// from.
pub const DISPLAY_RESTART_ANNOTATION: &str = "d2b.d2bus.org/restart-generation";

/// The execution-policy annotation the display session's worker rows carry:
/// the authorized `ExecutionPolicy` the helper's privileges were admitted
/// under, if the session names one.
pub const DISPLAY_EXECUTION_POLICY_ANNOTATION: &str = "d2b.d2bus.org/execution-policy";

/// The bounded purpose of the admitted compositor connection when the session
/// names no display of its own.
pub const COMPOSITOR_BINDING_PURPOSE: &str = "display-compositor";

/// The bounded purpose of the admitted proxy attachment: the same
/// cross-domain carriage purpose the proxy's Endpoint row declares, so the
/// admitted relationship and the source it consumes name one purpose.
pub const PROXY_BINDING_PURPOSE: &str = "wayland-cross-domain";

/// The source fingerprint base of the session's host compositor Endpoint.
pub const COMPOSITOR_ENDPOINT_FINGERPRINT: &str = "display-wayland-compositor";

/// The source fingerprint base of the host proxy's private Endpoint.
pub const PROXY_ENDPOINT_FINGERPRINT: &str = "display-wayland-data-v3";

/// The source fingerprint base of the guest frontend's private Endpoint.
pub const FRONTEND_ENDPOINT_FINGERPRINT: &str = "guest-frontend-v3";

/// The sandbox every display worker runs under.
///
/// A display helper asks for no namespace of its own, no capability, no
/// mount, no device, and no inherited session environment: the whole of its
/// reach beyond that is the endpoint relationship its admitted binding
/// covers.
fn display_worker_sandbox() -> Result<SandboxSpec, WorkerEffectError> {
    SandboxSpec::new(
        Vec::new(),
        Vec::new(),
        BoundedToken::parse("strict").map_err(|_| WorkerEffectError::LaunchRejected)?,
        true,
        false,
        EnvironmentClass::Minimal,
        true,
        None,
        0,
        None,
    )
    .map_err(|_| WorkerEffectError::LaunchRejected)
}

/// The Process and Endpoint intents one session owns, in the family's
/// preserved order (host proxy, guest frontend; each with its endpoint, and
/// the proxy's own compositor source).
///
/// The intent bodies are the full resource envelopes the display Provider
/// synthesized; the manager owns the child identity, so only the spec and the
/// authored metadata (ownerRef, labels, annotations - the restart-generation
/// and execution-policy annotations the display status reads) are carried.
pub fn display_owned_child_intents(
    zone: &ZoneId,
    session_ref: &ResourceRef,
    session_uid: &ResourceUid,
    spec: &WaylandSessionSpec,
    process_generation: u64,
) -> Result<Vec<OwnedChildIntent>, WorkerEffectError> {
    let mut intents = Vec::with_capacity(7);
    for role in [
        DisplayProcessRole::HostProxy,
        DisplayProcessRole::GuestFrontend,
    ] {
        let process_ref = durable_process_ref(session_uid, role)?;
        let process = durable_process_payload_for_generation(
            zone,
            session_ref,
            session_uid,
            spec,
            role,
            process_generation,
        )?;
        let process_digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &process);
        intents.push(
            OwnedChildIntent::new(process_ref.clone(), process, process_digest)
                .map_err(|_| WorkerEffectError::LaunchRejected)?
                .with_dependencies([session_ref.clone()])
                .map_err(|_| WorkerEffectError::LaunchRejected)?,
        );
        if role == DisplayProcessRole::HostProxy {
            // The compositor socket the proxy may reach is a declared Endpoint
            // row owned by this session, so its locator is resolved from the
            // admitted relationship instead of an inherited environment name.
            let compositor_ref = durable_compositor_endpoint_ref(session_uid)?;
            let compositor = durable_compositor_endpoint_payload(
                zone,
                session_ref,
                session_uid,
                spec,
                process_generation,
            )?;
            let compositor_digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &compositor);
            intents.push(
                OwnedChildIntent::new(compositor_ref, compositor, compositor_digest)
                    .map_err(|_| WorkerEffectError::LaunchRejected)?
                    .with_dependencies([process_ref.clone()])
                    .map_err(|_| WorkerEffectError::LaunchRejected)?,
            );
        }
        let endpoint_ref = durable_endpoint_ref(session_uid, role)?;
        let endpoint = durable_endpoint_payload(
            zone,
            session_ref,
            session_uid,
            spec,
            role,
            &process_ref,
            process_generation,
        )?;
        let endpoint_digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &endpoint);
        intents.push(
            OwnedChildIntent::new(endpoint_ref, endpoint, endpoint_digest)
                .map_err(|_| WorkerEffectError::LaunchRejected)?
                .with_dependencies([process_ref])
                .map_err(|_| WorkerEffectError::LaunchRejected)?,
        );
    }
    Ok(intents)
}

/// The bounded purpose the compositor connection is admitted for.
///
/// A session may name its own display, which is one socket component: an
/// absolute or nested value never reaches here, so it can neither be the
/// purpose of an admitted relationship nor redirect one.
fn compositor_purpose(spec: &WaylandSessionSpec) -> &str {
    spec.compositor_display()
        .map_or(COMPOSITOR_BINDING_PURPOSE, BoundedToken::as_str)
}

/// The endpoint source fingerprint one role's Endpoint row must carry.
///
/// The fingerprint binds the endpoint to this session's committed reconnect
/// generation, so endpoint access admitted for an earlier generation cannot
/// be reused by a later one.
fn endpoint_fingerprint(base: &'static str, reconnect_generation: u64) -> String {
    format!("{base}-r{reconnect_generation}")
}

/// The durable Process reference of one worker role: the session uid's
/// bounded suffix makes the name stable across restarts and unique per
/// session.
fn durable_process_ref(
    session_uid: &ResourceUid,
    role: DisplayProcessRole,
) -> Result<ResourceRef, WorkerEffectError> {
    let name = match role {
        DisplayProcessRole::HostProxy => "display-host-proxy",
        DisplayProcessRole::GuestFrontend => "display-guest-frontend",
    };
    let rendered = format!(
        "Process/{name}-{}",
        durable_display_suffix(session_uid, role)
    );
    ResourceRef::parse(&rendered).map_err(|_| WorkerEffectError::LaunchRejected)
}

/// The durable Endpoint reference of one worker role.
fn durable_endpoint_ref(
    session_uid: &ResourceUid,
    role: DisplayProcessRole,
) -> Result<ResourceRef, WorkerEffectError> {
    let rendered = format!(
        "Endpoint/display-endpoint-{}",
        durable_display_suffix(session_uid, role)
    );
    ResourceRef::parse(&rendered).map_err(|_| WorkerEffectError::LaunchRejected)
}

/// The durable Process reference of the session's host proxy worker.
///
/// The host proxy is the worker the session's other worker is ordered behind,
/// so a reader that has to name it - the display aggregation, or a test that
/// asserts which child stands where - derives the same name here rather than
/// picking a child row out of a list.
pub fn durable_host_proxy_process_ref(
    session_uid: &ResourceUid,
) -> Result<ResourceRef, WorkerEffectError> {
    durable_process_ref(session_uid, DisplayProcessRole::HostProxy)
}

/// The durable Process reference of the session's guest frontend worker.
pub fn durable_guest_frontend_process_ref(
    session_uid: &ResourceUid,
) -> Result<ResourceRef, WorkerEffectError> {
    durable_process_ref(session_uid, DisplayProcessRole::GuestFrontend)
}

/// The durable Endpoint reference the session projects as its Wayland
/// endpoint (R23).
///
/// This is the guest frontend's OWN cross-domain transport, not the first
/// `Endpoint` child a list happens to yield and not the host proxy's private
/// data carriage: it is the endpoint the frontend produces, it is the one the
/// session's readiness is gated on, and it publishes no in-Zone relationship
/// (R20). The name is derived from the session's own row identity, so the
/// projection names the same row across restarts and across a reconnect.
pub fn durable_wayland_endpoint_ref(
    session_uid: &ResourceUid,
) -> Result<ResourceRef, WorkerEffectError> {
    durable_endpoint_ref(session_uid, DisplayProcessRole::GuestFrontend)
}

/// The durable Endpoint reference of the host proxy's private cross-domain
/// carriage: the source the guest frontend's own attachment consumes.
pub fn durable_host_proxy_endpoint_ref(
    session_uid: &ResourceUid,
) -> Result<ResourceRef, WorkerEffectError> {
    durable_endpoint_ref(session_uid, DisplayProcessRole::HostProxy)
}

/// The durable Endpoint reference of the session's host compositor socket.
///
/// The compositor socket is a declared Endpoint of the session, so the only
/// way to reach it is the admitted relationship this reference names.
pub fn durable_compositor_endpoint_ref(
    session_uid: &ResourceUid,
) -> Result<ResourceRef, WorkerEffectError> {
    let rendered = format!(
        "Endpoint/wayland-compositor-{}",
        durable_display_suffix(session_uid, DisplayProcessRole::HostProxy)
    );
    ResourceRef::parse(&rendered).map_err(|_| WorkerEffectError::LaunchRejected)
}

/// The source fingerprint the session's compositor Endpoint must carry.
fn expected_compositor_fingerprint(spec: &WaylandSessionSpec) -> String {
    endpoint_fingerprint(COMPOSITOR_ENDPOINT_FINGERPRINT, spec.reconnect_generation())
}

/// The source fingerprint the proxy's Endpoint must carry for the guest
/// frontend's attachment.
fn expected_proxy_fingerprint(spec: &WaylandSessionSpec) -> String {
    endpoint_fingerprint(PROXY_ENDPOINT_FINGERPRINT, spec.reconnect_generation())
}

/// The source fingerprint the guest frontend's own Endpoint must carry.
fn expected_frontend_fingerprint(spec: &WaylandSessionSpec) -> String {
    endpoint_fingerprint(FRONTEND_ENDPOINT_FINGERPRINT, spec.reconnect_generation())
}

/// The durable Endpoint payload of one worker role: the private
/// cross-domain endpoint the worker's wayland traffic rides.
///
/// The proxy's endpoint is consumed by exactly one subject - the session's
/// guest frontend row - and admits exactly the attach operation that
/// relationship needs. Every other consumer of that socket, and every other
/// operation on it, stays outside the admitted relationship.
fn durable_endpoint_payload(
    zone: &ZoneId,
    session_ref: &ResourceRef,
    session_uid: &ResourceUid,
    spec: &WaylandSessionSpec,
    role: DisplayProcessRole,
    producer_ref: &ResourceRef,
    generation: u64,
) -> Result<Vec<u8>, WorkerEffectError> {
    endpoint_envelope(
        zone,
        session_ref,
        &durable_endpoint_ref(session_uid, role)?,
        worker_endpoint_spec(session_uid, spec, role, producer_ref)?,
        generation,
    )
}

/// The committed shape of one worker role's private Endpoint row.
///
/// This is the ONE derivation of that shape: the durable payload the session
/// commits and the shape this Provider admits into the Endpoint plane are
/// the same value, so the two can never drift (KTD5).
fn worker_endpoint_spec(
    session_uid: &ResourceUid,
    spec: &WaylandSessionSpec,
    role: DisplayProcessRole,
    producer_ref: &ResourceRef,
) -> Result<EndpointSpec, WorkerEffectError> {
    let (endpoint_class, transport, purpose, fingerprint) = match role {
        DisplayProcessRole::HostProxy => (
            EndpointClass::Data,
            EndpointTransport::FdAttachment,
            "wayland-cross-domain",
            expected_proxy_fingerprint(spec),
        ),
        DisplayProcessRole::GuestFrontend => (
            EndpointClass::Transport,
            EndpointTransport::Vsock,
            "guest-cross-domain",
            expected_frontend_fingerprint(spec),
        ),
    };
    // Publication intent and authorization are separate axes (KTD4). The proxy's
    // endpoint is PUBLISHED to exactly one subject - the session's guest
    // frontend row - and that is the only relationship derived from it. The
    // frontend's own endpoint publishes NOTHING: it gates the session's
    // aggregate readiness, and there is no in-Zone consumer for it to deliver
    // to (R20), so naming a subject here would invent one.
    let published_subjects = match role {
        DisplayProcessRole::HostProxy => vec![durable_process_ref(
            session_uid,
            DisplayProcessRole::GuestFrontend,
        )?],
        DisplayProcessRole::GuestFrontend => Vec::new(),
    };
    let allowed_subjects = published_subjects.clone();
    let allowed_operations = match role {
        DisplayProcessRole::HostProxy => {
            vec![EndpointOperation::Attach, EndpointOperation::Resolve]
        }
        DisplayProcessRole::GuestFrontend => vec![EndpointOperation::Resolve],
    };
    Ok(
        EndpointSpec::new(
            display_provider_ref()?,
            producer_ref.clone(),
            endpoint_class,
            transport,
            BoundedToken::parse(purpose).map_err(|_| WorkerEffectError::LaunchRejected)?,
            Some(BoundedText::parse(fingerprint).map_err(|_| WorkerEffectError::LaunchRejected)?),
            EndpointLocality::CrossDomain,
            EndpointVisibility::Owner,
            EndpointAttachmentPolicy::new(
                matches!(role, DisplayProcessRole::HostProxy),
                u16::from(matches!(role, DisplayProcessRole::HostProxy)),
            )
            .map_err(|_| WorkerEffectError::LaunchRejected)?,
            EndpointConsumerPolicy::new(allowed_subjects, Vec::new(), allowed_operations)
                .map_err(|_| WorkerEffectError::LaunchRejected)?,
            EndpointLifecyclePolicy::RecycleWithProducer,
        )
        .map_err(|_| WorkerEffectError::LaunchRejected)?
        // An empty subject list is the `none` class, not an empty `named` list:
        // "publishes nothing" and "publishes to nobody I listed" are the same
        // commitment here, and spelling it as `none` keeps the guest frontend's
        // endpoint honest about having no in-Zone relationship at all (R20).
        .with_binding_publication(if published_subjects.is_empty() {
            EndpointBindingPublication::None
        } else {
            EndpointBindingPublication::named(published_subjects)
                .map_err(|_| WorkerEffectError::LaunchRejected)?
        }),
    )
}

/// The durable payload of the session's host compositor Endpoint: the exact
/// socket the proxy is admitted to connect to, produced by the session's Host
/// execution target.
fn durable_compositor_endpoint_payload(
    zone: &ZoneId,
    session_ref: &ResourceRef,
    session_uid: &ResourceUid,
    spec: &WaylandSessionSpec,
    generation: u64,
) -> Result<Vec<u8>, WorkerEffectError> {
    endpoint_envelope(
        zone,
        session_ref,
        &durable_compositor_endpoint_ref(session_uid)?,
        compositor_endpoint_spec(session_uid, spec)?,
        generation,
    )
}

/// The committed shape of the session's host compositor Endpoint row.
///
/// This is the ONE derivation of that shape: the durable payload the session
/// commits and the shape this Provider admits into the Endpoint plane are
/// the same value, so the two can never drift (KTD5).
fn compositor_endpoint_spec(
    session_uid: &ResourceUid,
    spec: &WaylandSessionSpec,
) -> Result<EndpointSpec, WorkerEffectError> {
    let proxy_ref = durable_process_ref(session_uid, DisplayProcessRole::HostProxy)?;
    Ok(
        EndpointSpec::new(
            display_provider_ref()?,
            spec.host_ref().clone(),
            EndpointClass::Transport,
            EndpointTransport::Unix,
            BoundedToken::parse(compositor_purpose(spec))
                .map_err(|_| WorkerEffectError::LaunchRejected)?,
            Some(
                BoundedText::parse(expected_compositor_fingerprint(spec))
                    .map_err(|_| WorkerEffectError::LaunchRejected)?,
            ),
            EndpointLocality::CrossDomain,
            EndpointVisibility::Owner,
            EndpointAttachmentPolicy::new(false, 0)
                .map_err(|_| WorkerEffectError::LaunchRejected)?,
            EndpointConsumerPolicy::new(
                vec![proxy_ref.clone()],
                Vec::new(),
                vec![EndpointOperation::Resolve],
            )
            .map_err(|_| WorkerEffectError::LaunchRejected)?,
            EndpointLifecyclePolicy::RecycleWithProducer,
        )
        .map_err(|_| WorkerEffectError::LaunchRejected)?
        // The compositor socket is published to exactly one consumer: this
        // session's host proxy row. Nothing else reaches it.
        .with_binding_publication(EndpointBindingPublication::named(vec![proxy_ref]).map_err(
            |_| WorkerEffectError::LaunchRejected,
        )?),
    )
}

/// The endpoint shapes this Provider commits for one display session.
///
/// The roles are the Provider's own vocabulary. The Endpoint plane never names
/// them: it asks which shape a committed row is, and this Provider answers
/// with its own exact match (KTD5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayEndpointRole {
    /// The session's host compositor socket: a transport the session's Host
    /// execution target resolves privately, reached by exactly one consumer.
    Compositor,
    /// The host proxy's private data carriage: the cross-domain attachment the
    /// session's guest frontend consumes.
    HostProxy,
    /// The guest frontend's own cross-domain transport, published to nobody.
    GuestFrontend,
}

impl DisplayEndpointRole {
    /// The realization the Endpoint plane serves this shape behind.
    ///
    /// Published because the mapping is this Provider's own vocabulary and a
    /// reader of a committed row needs to know which evidence stands behind
    /// it; the admission answers with the same value.
    pub const fn realization(self) -> d2b_provider_endpoint::EndpointRealization {
        use d2b_provider_endpoint::EndpointRealization as Realization;
        match self {
            // The compositor socket is realized behind the daemon's private
            // observation of it; both worker shapes are realized behind the
            // live row of the worker this Provider launched.
            Self::Compositor => Realization::HostSocketTransport,
            Self::HostProxy => Realization::WorkerDataAttachment,
            Self::GuestFrontend => Realization::WorkerCrossDomainTransport,
        }
    }
}


/// The endpoint shapes this Provider has committed, keyed by the producer
/// each one is realized behind (U5, KTD5).
///
/// The vocabulary is a registry rather than a single session's answer because
/// the Endpoint driver asks about one committed row at a time and a Zone
/// serves more than one display session. Each entry holds the shape the
/// Provider committed in full, so the admission is an exact comparison: a
/// committed row that differs from the committed shape on the provider
/// reference, the producer, the class, the transport, the purpose, the
/// locality, the visibility, the lifecycle, the reconnect fingerprint, the
/// consumer policy, the attachment posture, or the publication intent is
/// simply not a shape this Provider commits, and the Endpoint driver refuses
/// it terminally.
///
/// The committed shapes are the SAME values the durable child rows are built
/// from, so the row this Provider commits and the shape this Provider admits
/// cannot drift apart.
///
/// # Wiring
///
/// This vocabulary is the display half of the Endpoint family's provider
/// seam: the display Provider owns the shapes it commits, so its own
/// vocabulary is what admits them, and nothing else decides whether a
/// committed display `Endpoint` row is one this Provider recognizes. One
/// session's answer is [`DisplayEndpointVocabulary`]; a Zone's whole answer is
/// [`SharedDisplayEndpointVocabulary`].
#[derive(Debug, Clone, Default)]
pub struct DisplayEndpointVocabulary {
    committed: BTreeMap<String, (EndpointSpec, CommittedEndpointShape)>,
}

impl DisplayEndpointVocabulary {
    /// Commit the three endpoint shapes of one admitted session.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerEffectError::LaunchRejected`] when the session's own
    /// durable derivation refuses, which is the same answer the durable child
    /// rows would have given.
    pub fn for_session(
        session_uid: &ResourceUid,
        spec: &WaylandSessionSpec,
    ) -> Result<Self, WorkerEffectError> {
        Self::default().with_session(session_uid, spec)
    }

    /// Commit another session's shapes into this vocabulary.
    ///
    /// # Errors
    ///
    /// The refusals [`Self::for_session`] reports.
    pub fn with_session(
        mut self,
        session_uid: &ResourceUid,
        spec: &WaylandSessionSpec,
    ) -> Result<Self, WorkerEffectError> {
        let reconnect_generation = spec.reconnect_generation();
        let proxy_ref = durable_process_ref(session_uid, DisplayProcessRole::HostProxy)?;
        let frontend_ref = durable_process_ref(session_uid, DisplayProcessRole::GuestFrontend)?;
        let committed = [
            (
                DisplayEndpointRole::Compositor,
                compositor_endpoint_spec(session_uid, spec)?,
            ),
            (
                DisplayEndpointRole::HostProxy,
                worker_endpoint_spec(
                    session_uid,
                    spec,
                    DisplayProcessRole::HostProxy,
                    &proxy_ref,
                )?,
            ),
            (
                DisplayEndpointRole::GuestFrontend,
                worker_endpoint_spec(
                    session_uid,
                    spec,
                    DisplayProcessRole::GuestFrontend,
                    &frontend_ref,
                )?,
            ),
        ];
        for (role, endpoint) in committed {
            // The reconnect generation travels with the shape: the shape is
            // admitted only while its fingerprint is the one this session's
            // currently authenticated generation mints, and the Endpoint
            // driver's incarnation derivation is bounded by that same number.
            self.committed.insert(
                endpoint.producer_ref().to_canonical_string(),
                (
                    endpoint,
                    CommittedEndpointShape::new(role.realization(), reconnect_generation),
                ),
            );
        }
        Ok(self)
    }

    /// The shape this Provider commits for `spec`, matched in full.
    fn committed_shape(&self, spec: &EndpointSpec) -> Option<CommittedEndpointShape> {
        self.committed
            .get(&spec.producer_ref().to_canonical_string())
            .filter(|(committed, _)| committed == spec)
            .map(|(_, shape)| *shape)
    }
}

impl EndpointPurposeVocabulary for DisplayEndpointVocabulary {
    fn committed_endpoint_shape(&self, spec: &EndpointSpec) -> Option<CommittedEndpointShape> {
        self.committed_shape(spec)
    }
}

/// The Zone-wide vocabulary the Endpoint family's provider seam reads in the
/// production composition (KTD5).
///
/// A Zone serves more than one display session and the Endpoint driver asks
/// about one committed row at a time, so the object a composition installs is
/// a registry: every admitted session contributes the shapes this Provider
/// committed for it, and a row is admitted only when one of them matches it in
/// full. The comparison, the constants, and the shapes are all this crate's
/// own - the seam carries no display type into the Endpoint family, which only
/// asks the question and takes the one exact verdict.
///
/// # Re-commit replaces a session's shapes
///
/// A session's entry is keyed by its own row uid, so re-admitting a session -
/// after its reconnect generation moved, or after anything else in its spec
/// changed - REPLACES the shapes it committed rather than adding to them. A
/// row still carrying an earlier generation's fingerprint is therefore not
/// admitted once the session moved on, which is what keeps a replaced
/// session's shape out of the admission set (R15).
///
/// # Concurrency
///
/// The registry is written on the session admission path and read on the
/// Endpoint driver's synchronous admission path, so it is guarded by a
/// read-write lock. Both sides are short critical sections over an in-memory
/// map, and neither holds the guard across a suspension point - there is no
/// await inside either - so no executor worker is parked on it.
///
/// # Retention
///
/// A session's shapes stay until the same uid commits again. A removed
/// session's rows are removed with it, so nothing can ask about those shapes;
/// what the registry retains for them is three specs and no answer.
#[derive(Debug, Default)]
pub struct SharedDisplayEndpointVocabulary {
    sessions: RwLock<BTreeMap<String, DisplayEndpointVocabulary>>,
}

impl SharedDisplayEndpointVocabulary {
    /// An empty registry: no session's shapes are committed yet, so nothing is
    /// admitted.
    pub fn new() -> Self {
        Self::default()
    }

    /// Commit one admitted session's three endpoint shapes, replacing whatever
    /// that same session uid committed before.
    ///
    /// # Errors
    ///
    /// Returns [`WorkerEffectError::LaunchRejected`] when the session's own
    /// durable derivation refuses, which is the same answer the durable child
    /// rows would have given - and the registry is left untouched, so a session
    /// that cannot derive its shapes never displaces the ones it did commit.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    pub fn commit_session(
        &self,
        session_uid: &ResourceUid,
        spec: &WaylandSessionSpec,
    ) -> Result<(), WorkerEffectError> {
        // Derived before the lock is taken: a session that cannot derive its
        // own shapes must not be able to displace what it committed before.
        let committed = DisplayEndpointVocabulary::for_session(session_uid, spec)?;
        let mut sessions = self
            .sessions
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.insert(session_uid.as_str().to_owned(), committed);
        Ok(())
    }
}

impl EndpointPurposeVocabulary for SharedDisplayEndpointVocabulary {
    /// The one exact shape any admitted session committed for `spec`.
    ///
    /// The guard is held across the map walk and released before this returns;
    /// there is no await in this path, so nothing parks on the lock.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    fn committed_endpoint_shape(&self, spec: &EndpointSpec) -> Option<CommittedEndpointShape> {
        let sessions = self
            .sessions
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions
            .values()
            .find_map(|committed| committed.committed_shape(spec))
    }
}

/// The display Provider's answer on the Endpoint family's provider seam
/// (KTD5).
///
/// The Endpoint family never names this type: it holds the trait object, asks
/// which shape a committed row is, and takes this Provider's one exact
/// verdict. The match itself is [`DisplayEndpointVocabulary`]'s, over the
/// shapes this Provider derived for the sessions committed here.
impl d2b_provider_endpoint::CommittedEndpointShapeSource for SharedDisplayEndpointVocabulary {
    fn committed_endpoint_shape(&self, spec: &EndpointSpec) -> Option<CommittedEndpointShape> {
        EndpointPurposeVocabulary::committed_endpoint_shape(self, spec)
    }
}

/// The canonical `EndpointBinding` rows one committed display `Endpoint` row
/// publishes (KTD4, R16, R20).
///
/// This asks the Endpoint family what a committed display endpoint row
/// publishes rather than describing the relationship here again: publication
/// intent names the consumer, the row's own operation allowlist and
/// attachment capacity decide how that consumer reaches it, and the delivery
/// slot is a function of the endpoint's own row identity. The `Endpoint`
/// driver mints the committed rows from this same derivation, so the row a
/// reader gates on and the row the actor committed are the same name by
/// construction - a slot table kept beside them could only ever disagree.
///
/// The guest frontend's own endpoint publishes nothing, so it derives no row
/// here: it gates the session's aggregate readiness instead, and inventing a
/// consumer for it would commit a relationship no row backs.
///
/// # Errors
///
/// Returns [`WorkerEffectError::LaunchRejected`] when the committed row
/// declares a delivery the Endpoint family refuses to derive a row for, or
/// when the derived row name is not a canonical reference.
pub fn display_canonical_bindings(
    zone: &ZoneId,
    endpoint_ref: &ResourceRef,
    spec: &EndpointSpec,
) -> Result<Vec<ResourceRef>, WorkerEffectError> {
    let deliveries = d2b_provider_endpoint::declared_endpoint_bindings(zone, spec, endpoint_ref)
        .map_err(|_| WorkerEffectError::LaunchRejected)?;
    d2b_provider_endpoint::canonical_binding_rows(zone, spec, endpoint_ref, &deliveries)
        .map_err(|_| WorkerEffectError::LaunchRejected)?
        .iter()
        .map(|row| {
            ResourceRef::parse(&format!(
                "{}/{name}",
                d2b_provider_endpoint::ENDPOINT_BINDING_TYPE_NAME,
                name = row.name().as_str(),
            ))
            .map_err(|_| WorkerEffectError::LaunchRejected)
        })
        .collect()
}

/// Whether one published `EndpointBinding` layer proves a DELIVERED
/// relationship for a row at `generation` (R20).
///
/// The published layer is the relationship actor's own redacted evidence, so
/// it is read through the Endpoint family's own projection parser rather
/// than matched as loose JSON here. Only the closed `delivered` state at the
/// row's own current generation proves anything: a replaced endpoint, an
/// undelivered row, a draining row, and a layer this reader cannot parse are
/// all the same answer - the relationship is not standing - because a session
/// that cannot prove its delivery is not usable.
pub fn display_binding_delivered(layer: Option<&Value>, generation: u64) -> bool {
    layer
        .and_then(d2b_provider_endpoint::BindingDeliveryProjection::from_projection)
        .is_some_and(
            |evidence| {
                matches!(
                    evidence,
                    d2b_provider_endpoint::BindingDeliveryProjection::Delivered {
                        generation: delivered,
                        ..
                    } if delivered == generation
                )
            },
        )
}

/// The display Provider reference the session's endpoint rows declare.
fn display_provider_ref() -> Result<ResourceRef, WorkerEffectError> {
    ResourceRef::parse("Provider/display-wayland").map_err(|_| WorkerEffectError::LaunchRejected)
}

/// The canonical durable envelope one derived endpoint row is committed as.
fn endpoint_envelope(
    zone: &ZoneId,
    owner_ref: &ResourceRef,
    endpoint_ref: &ResourceRef,
    endpoint_spec: EndpointSpec,
    generation: u64,
) -> Result<Vec<u8>, WorkerEffectError> {
    let payload = serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": "Endpoint",
        "metadata": {
            "name": endpoint_ref.name().as_str(),
            "zone": zone.as_str(),
            "ownerRef": owner_ref.to_canonical_string(),
            "finalizers": [],
            "deletionRequestedAt": null,
            "createdAt": "1970-01-01T00:00:00.000Z",
            "updatedAt": "1970-01-01T00:00:00.000Z",
            "managedBy": "controller",
            "generation": generation.max(1),
            "revision": 1
        },
        "spec": endpoint_spec,
        "status": {
            "completedAt": null,
            "conditions": [],
            "lastReconciledAt": null,
            "observedGeneration": 0,
            "outcome": null,
            "phase": "Pending",
            "resource": {
                "readiness": "Pending",
                "observedProducerGeneration": 0,
                "observedResourceGeneration": generation.max(1),
                "endpointGeneration": 0,
                "connectionAvailability": "unavailable",
                "leaseAvailability": "lease-required"
            },
            "startedAt": null,
            "update": {
                "dependencies": {"count": 0, "refs": []},
                "disruption": "None",
                "lastAssessedAt": null,
                "observedGeneration": 0,
                "operationId": null,
                "owned": {"count": 0, "refs": []},
                "preserveState": true,
                "reasons": [],
                "state": "Unknown",
                "targetGeneration": generation.max(1)
            }
        }
    });
    let bytes = serde_json::to_vec(&payload).map_err(|_| WorkerEffectError::LaunchRejected)?;
    Ok(CanonicalJsonValue::parse(&bytes)
        .map_err(|_| WorkerEffectError::LaunchRejected)?
        .to_canonical_bytes())
}

/// The durable Process payload of one worker role: the worker template, its
/// execution target, the admitted User the helper runs as, and the restart-
/// generation and execution-policy annotations.
///
/// The row asks for no capability, no mount, no device, and a minimal
/// environment: a helper's reach beyond its sandbox is exactly the endpoint
/// relationship its `EndpointBinding` admits, never a template name or an
/// inherited session environment.
fn durable_process_payload_for_generation(
    zone: &ZoneId,
    session_ref: &ResourceRef,
    session_uid: &ResourceUid,
    spec: &WaylandSessionSpec,
    role: DisplayProcessRole,
    process_generation: u64,
) -> Result<Vec<u8>, WorkerEffectError> {
    let execution_ref = match role {
        DisplayProcessRole::HostProxy => spec.host_ref(),
        DisplayProcessRole::GuestFrontend => spec.guest_ref(),
    }
    .clone();
    let template = match role {
        DisplayProcessRole::HostProxy => "wayland-proxy-worker",
        DisplayProcessRole::GuestFrontend => "wayland-frontend-worker",
    };
    let provider = match role {
        DisplayProcessRole::HostProxy => "Provider/system-minijail",
        DisplayProcessRole::GuestFrontend => "Provider/system-systemd",
    };
    let process = ProcessSpec::minimal(
        ExecutionSpec::new(
            execution_ref,
            None,
            Some(spec.user_ref().clone()),
            ProcessClass::Worker,
            BoundedToken::parse(template).map_err(|_| WorkerEffectError::LaunchRejected)?,
            None,
            Vec::new(),
            Vec::new(),
            display_worker_sandbox()?,
            BudgetSpec::default(),
            None,
            Vec::new(),
            TelemetrySpec::default(),
        )
        .map_err(|_| WorkerEffectError::LaunchRejected)?,
    );
    let mut spec_value =
        serde_json::to_value(process).map_err(|_| WorkerEffectError::LaunchRejected)?;
    let spec_object = spec_value
        .as_object_mut()
        .ok_or(WorkerEffectError::LaunchRejected)?;
    spec_object.insert("providerRef".to_owned(), serde_json::json!(provider));
    spec_object.insert(
        "updatePolicy".to_owned(),
        serde_json::json!({
            "disruptive": "manual",
            "nonDisruptive": "automatic"
        }),
    );
    let process_ref = durable_process_ref(session_uid, role)?;
    let owner_ref = session_ref;
    let generation = process_generation.max(1);
    // A session that names an `ExecutionPolicy` records which authorized
    // policy its helper privileges were admitted under; the row itself asks
    // for nothing beyond the declared sandbox.
    let mut annotations = serde_json::Map::from_iter([(
        DISPLAY_RESTART_ANNOTATION.to_owned(),
        Value::String(generation.to_string()),
    )]);
    if let Some(policy) = spec.execution_policy_ref() {
        annotations.insert(
            DISPLAY_EXECUTION_POLICY_ANNOTATION.to_owned(),
            Value::String(policy.to_canonical_string()),
        );
    }
    let annotations = Value::Object(annotations);
    let payload = serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": "Process",
        "metadata": {
            "name": process_ref.name().as_str(),
            "zone": zone.as_str(),
            "ownerRef": owner_ref.to_canonical_string(),
            "annotations": annotations,
            "finalizers": [],
            "deletionRequestedAt": null,
            "createdAt": "1970-01-01T00:00:00.000Z",
            "updatedAt": "1970-01-01T00:00:00.000Z",
            "managedBy": "controller",
            "generation": generation,
            "revision": 1
        },
        "spec": spec_value,
        "status": {
            "completedAt": null,
            "conditions": [],
            "lastReconciledAt": null,
            "observedGeneration": 0,
            "outcome": null,
            "phase": "Pending",
            "resource": {},
            "startedAt": null,
            "update": {
                "dependencies": {"count": 0, "refs": []},
                "disruption": "None",
                "lastAssessedAt": null,
                "observedGeneration": 0,
                "operationId": null,
                "owned": {"count": 0, "refs": []},
                "preserveState": true,
                "reasons": [],
                "state": "Unknown",
                "targetGeneration": generation
            }
        }
    });
    let bytes = serde_json::to_vec(&payload).map_err(|_| WorkerEffectError::LaunchRejected)?;
    CanonicalJsonValue::parse(&bytes)
        .map(|value| value.to_canonical_bytes())
        .map_err(|_| WorkerEffectError::LaunchRejected)
}

/// Lowercase hex digits for two-digit-per-byte encoding.
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// The bounded session-uid suffix of one worker role's durable names.
fn durable_display_suffix(session_uid: &ResourceUid, role: DisplayProcessRole) -> String {
    let mut digest = Sha256::new();
    digest.update(b"d2bd-durable-display-process-v1");
    digest.update(session_uid.as_str().as_bytes());
    digest.update([role as u8]);
    let digest = digest.finalize();
    let mut suffix = String::with_capacity(40);
    for byte in digest.iter().take(20) {
        suffix.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        suffix.push(HEX_DIGITS[(byte & 0x0f) as usize] as char);
    }
    suffix
}

/// The `status.resource` projection for one display session: the two worker
/// Process references and the private Endpoint with its committed
/// generation.
pub fn wayland_session_resource_projection(
    resource: &WaylandSessionResourceStatus,
) -> Value {
    let mut projection = serde_json::Map::new();
    if let Some(reference) = resource.proxy_process_ref.as_ref() {
        projection.insert(
            "proxyProcessRef".to_owned(),
            serde_json::Value::String(reference.to_canonical_string()),
        );
    }
    if let Some(reference) = resource.guest_frontend_process_ref.as_ref() {
        projection.insert(
            "guestFrontendProcessRef".to_owned(),
            serde_json::Value::String(reference.to_canonical_string()),
        );
    }
    if let Some(reference) = resource.wayland_endpoint_ref.as_ref() {
        projection.insert(
            "waylandEndpointRef".to_owned(),
            serde_json::Value::String(reference.to_canonical_string()),
        );
    }
    if let Some(generation) = resource.wayland_endpoint_generation {
        projection.insert(
            "waylandEndpointGeneration".to_owned(),
            serde_json::Value::Number(generation.into()),
        );
    }
    if !resource.policy_digest.is_empty() {
        projection.insert(
            "policyDigest".to_owned(),
            serde_json::Value::String(resource.policy_digest.clone()),
        );
    }
    serde_json::Value::Object(projection)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone() -> ZoneId {
        ZoneId::parse("work").expect("zone")
    }

    fn session_ref() -> ResourceRef {
        ResourceRef::parse("display-wayland.d2bus.org.WaylandSession/display-wayland")
            .expect("session ref")
    }

    fn session_uid() -> ResourceUid {
        ResourceUid::parse("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").expect("session uid")
    }

    fn session_spec() -> WaylandSessionSpec {
        WaylandSessionSpec::new(
            ResourceRef::parse("Guest/work").expect("guest"),
            ResourceRef::parse("Host/host-system").expect("host"),
            ResourceRef::parse("User/alice").expect("user"),
            ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/default")
                .expect("policy"),
            crate::DisplayIdentity::new("work", "#112233", "#223344", "#334455")
                .expect("identity"),
            true,
        )
        .expect("session spec")
    }

    /// The session's durable child set: two Process rows and two Endpoint
    /// rows, owned by the session, carrying no host socket vocabulary.
    #[test]
    fn display_owned_child_intents_are_durable_and_owned() {
        let zone = ZoneId::parse("work").expect("zone");
        let session_ref =
            ResourceRef::parse("display-wayland.d2bus.org.WaylandSession/display-wayland")
                .expect("session ref");
        let session_uid =
            ResourceUid::parse("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").expect("session uid");
        let spec = WaylandSessionSpec::new(
            ResourceRef::parse("Guest/work").expect("guest"),
            ResourceRef::parse("Host/host-system").expect("host"),
            ResourceRef::parse("User/alice").expect("user"),
            ResourceRef::parse("display-wayland.d2bus.org.WaylandPolicy/default")
                .expect("policy"),
            crate::DisplayIdentity::new(
                "work",
                "#112233",
                "#223344",
                "#334455",
            )
            .expect("identity"),
            true,
        )
        .expect("session spec");
        let intents = display_owned_child_intents(&zone, &session_ref, &session_uid, &spec, 4);
        let intents = intents.expect("display child intents");
        assert_eq!(intents.len(), 5);
        assert_eq!(
            intents
                .iter()
                .filter(|intent| intent.target().resource_type().as_str() == "Process")
                .count(),
            2
        );
        assert_eq!(
            intents
                .iter()
                .filter(|intent| intent.target().resource_type().as_str() == "Endpoint")
                .count(),
            3,
            "the proxy's own cross-domain Endpoint, the host compositor \
             Endpoint, and the frontend's Endpoint"
        );
        for intent in intents {
            let value: serde_json::Value =
                serde_json::from_slice(intent.canonical_resource()).expect("child resource");
            assert_eq!(
                value["metadata"]["ownerRef"],
                session_ref.to_canonical_string()
            );
            assert!(
                !value.to_string().contains("WAYLAND_DISPLAY")
                    && !value.to_string().contains("NIRI_SOCKET")
            );
        }
    }

    /// The display graph still derives its two canonical `EndpointBinding`
    /// rows from PUBLICATION INTENT alone (KTD4, R16).
    ///
    /// This is the regression guard for the cutover: publication intent
    /// defaults to `none`, so an emitter that forgot to declare it would
    /// silently derive no relationship at all. The two relationships this
    /// session really has - the host proxy consuming the compositor socket,
    /// and the guest frontend consuming the proxy's cross-domain endpoint -
    /// are derived here through the source's own derivation, and the guest
    /// frontend's own endpoint derives none because it gates aggregate
    /// readiness without an in-Zone consumer (R20).
    #[test]
    fn display_publication_intent_derives_exactly_the_two_real_relationships() {
        let zone = zone();
        let session_uid = session_uid();
        let spec = session_spec();
        let intents =
            display_owned_child_intents(&zone, &session_ref(), &session_uid, &spec, 4)
                .expect("display child intents");

        let mut derived: Vec<(String, String)> = Vec::new();
        for intent in intents {
            if intent.target().resource_type().as_str() != "Endpoint" {
                continue;
            }
            let value: serde_json::Value =
                serde_json::from_slice(intent.canonical_resource()).expect("child resource");
            let endpoint_ref = intent.target().clone();
            let decoded: EndpointSpec =
                serde_json::from_value(value["spec"].clone()).expect("endpoint spec");
            let deliveries = d2b_provider_endpoint::declared_endpoint_bindings(
                &zone,
                &decoded,
                &endpoint_ref,
            )
            .expect("the source derives its relationships");
            for delivery in deliveries {
                derived.push((
                    endpoint_ref.name().as_str().to_owned(),
                    delivery.consumer().as_ref().to_canonical_string(),
                ));
            }
        }
        derived.sort();

        let compositor = durable_compositor_endpoint_ref(&session_uid)
            .expect("compositor endpoint reference");
        let proxy_endpoint = durable_endpoint_ref(&session_uid, DisplayProcessRole::HostProxy)
            .expect("proxy endpoint reference");
        let frontend = durable_process_ref(&session_uid, DisplayProcessRole::GuestFrontend)
            .expect("frontend process reference");
        let proxy = durable_process_ref(&session_uid, DisplayProcessRole::HostProxy)
            .expect("proxy process reference");
        let mut expected = vec![
            (compositor.name().as_str().to_owned(), proxy.to_canonical_string()),
            (proxy_endpoint.name().as_str().to_owned(), frontend.to_canonical_string()),
        ];
        expected.sort();
        derived.sort();
        assert_eq!(
            derived, expected,
            "the session derives exactly the host proxy's compositor relationship \
             and the guest frontend's proxy relationship, from publication intent"
        );
    }

    /// The guest frontend's own endpoint publishes nothing.
    ///
    /// It gates the session's aggregate readiness and has no in-Zone consumer,
    /// so an endpoint that named one would invent a relationship no row
    /// backs (R20).
    #[test]
    fn the_frontend_endpoint_publishes_no_relationship() {
        let zone = zone();
        let uid = session_uid();
        let spec = session_spec();
        let endpoint_ref = durable_endpoint_ref(&uid, DisplayProcessRole::GuestFrontend)
            .expect("frontend endpoint reference");
        let payload = durable_endpoint_payload(
            &zone,
            &session_ref(),
            &uid,
            &spec,
            DisplayProcessRole::GuestFrontend,
            &durable_process_ref(&uid, DisplayProcessRole::GuestFrontend)
                .expect("frontend process reference"),
            4,
        )
        .expect("frontend endpoint payload");
        let value: serde_json::Value =
            serde_json::from_slice(&payload).expect("endpoint envelope");
        let decoded: EndpointSpec =
            serde_json::from_value(value["spec"].clone()).expect("endpoint spec");
        assert!(
            decoded.binding_publication().is_none(),
            "the guest frontend's endpoint publishes nothing, which is distinct \
             from an unconstrained consumer policy"
        );
        assert!(
            d2b_provider_endpoint::declared_endpoint_bindings(&zone, &decoded, &endpoint_ref)
                .expect("the source derives its relationships")
                .is_empty(),
            "and therefore derives no binding row"
        );
    }
}
