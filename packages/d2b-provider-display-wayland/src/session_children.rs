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
//! relationship over an exact `Endpoint` row ([`DisplayEndpointBinding`]),
//! never a name, an environment variable, or a directory. The host proxy
//! reaches the host compositor through the session's compositor Endpoint and
//! no other socket; the guest frontend reaches the proxy through the proxy's
//! own Endpoint. [`admit_display_endpoint`] is the one admission those
//! relationships must pass before a helper may be used, and it re-derives the
//! expected relationship rather than trusting an observed row.
//!
//! The derivation is pure: it builds payloads and references from the
//! session's row identity and spec, and never touches host state.

use d2b_contracts_resource::v3::{
    BindingKey, BindingRealizationFacet, BindingRealizationSupport, BindingSlot, CanonicalJsonValue,
    EndpointAttachmentKind, EndpointBindingRequest, RESOURCE_ENVELOPE_DOMAIN_TAG, ResourceRef,
    ResourceUid, ZoneId, canonical_digest,
    execution_policy::{BoundedText, BoundedToken, BudgetSpec},
    process::{EnvironmentClass, ExecutionSpec, ProcessClass, ProcessSpec, SandboxSpec, TelemetrySpec},
};
use d2b_core_controller::OwnedChildIntent;
use d2b_provider_endpoint::endpoint::{
    EndpointAttachmentPolicy, EndpointBindingPublication, EndpointClass, EndpointConsumerPolicy,
    EndpointLifecyclePolicy, EndpointLocality, EndpointOperation, EndpointSpec, EndpointTransport,
    EndpointVisibility,
};
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

/// The stable consumer slot of the host proxy's admitted compositor
/// connection.
pub const COMPOSITOR_BINDING_SLOT: &str = "wayland-compositor";

/// The stable consumer slot of the guest frontend's admitted proxy
/// attachment.
pub const PROXY_BINDING_SLOT: &str = "wayland-proxy";

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

/// One admitted endpoint relationship of one display worker role.
///
/// The relationship is derived from the session's row identity and spec, so a
/// consumer can only ever reach the exact endpoint this value names, in the
/// declared attachment form, for the declared purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayEndpointBinding {
    role: DisplayProcessRole,
    source_ref: ResourceRef,
    consumer_ref: ResourceRef,
    request: EndpointBindingRequest,
}

impl DisplayEndpointBinding {
    /// Return the worker role this relationship belongs to.
    pub const fn role(&self) -> DisplayProcessRole {
        self.role
    }

    /// Borrow the exact source Endpoint.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// Borrow the exact consumer row.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// Borrow the typed binding request.
    pub const fn request(&self) -> &EndpointBindingRequest {
        &self.request
    }
}

/// Every endpoint relationship one session's workers require, in the family's
/// preserved role order.
pub fn display_endpoint_bindings(
    session_uid: &ResourceUid,
    spec: &WaylandSessionSpec,
) -> Result<Vec<DisplayEndpointBinding>, WorkerEffectError> {
    let mut bindings = Vec::with_capacity(2);
    for role in [
        DisplayProcessRole::HostProxy,
        DisplayProcessRole::GuestFrontend,
    ] {
        let consumer_ref = durable_process_ref(session_uid, role)?;
        let (source_ref, slot, attachment, purpose) = match role {
            DisplayProcessRole::HostProxy => (
                durable_compositor_endpoint_ref(session_uid)?,
                COMPOSITOR_BINDING_SLOT,
                EndpointAttachmentKind::Connect,
                compositor_purpose(spec),
            ),
            DisplayProcessRole::GuestFrontend => (
                durable_endpoint_ref(session_uid, DisplayProcessRole::HostProxy)?,
                PROXY_BINDING_SLOT,
                EndpointAttachmentKind::Attach,
                PROXY_BINDING_PURPOSE,
            ),
        };
        let slot = BindingSlot::parse(slot).map_err(|_| WorkerEffectError::LaunchRejected)?;
        let purpose =
            BoundedToken::parse(purpose).map_err(|_| WorkerEffectError::LaunchRejected)?;
        let request = EndpointBindingRequest::new(
            source_ref.clone(),
            consumer_ref.clone(),
            slot,
            attachment,
            purpose,
        )
        .map_err(|_| WorkerEffectError::LaunchRejected)?;
        bindings.push(DisplayEndpointBinding {
            role,
            source_ref,
            consumer_ref,
            request,
        });
    }
    Ok(bindings)
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

/// Decode one committed endpoint row's spec into the endpoint contract its
/// relationships are evaluated against.
///
/// The row is decoded here because this crate owns the endpoint vocabulary:
/// the consumer of a relationship must read the source's own contract, not
/// its own idea of it.
pub fn decode_endpoint_spec(
    spec: &Value,
) -> Result<EndpointSpec, WorkerEffectError> {
    serde_json::from_value(spec.clone()).map_err(|_| WorkerEffectError::LaunchRejected)
}

/// One committed endpoint observation a caller hands to
/// [`admit_display_endpoint`].
///
/// The observation is the committed `Endpoint` row plus the committed
/// consumer row; nothing here carries a socket path, because the endpoint's
/// locator stays private to its realization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayEndpointObservation<'a> {
    /// The committed `Endpoint` row the relationship would consume.
    pub spec: &'a EndpointSpec,
    /// The committed generation of that endpoint row.
    pub source_generation: u64,
    /// The committed generation of the consumer row.
    pub consumer_generation: u64,
    /// The admitted `User` the committed consumer row runs as.
    ///
    /// A worker row admitted for another identity cannot consume this
    /// session's endpoints: the relationship is bound to the session's own
    /// User, not to whatever the consumer row happens to request.
    pub consumer_user: Option<ResourceRef>,
}

/// Admit one observed endpoint relationship for one display worker role.
///
/// The relationship is re-derived from the session and compared with what was
/// observed: the exact source endpoint, the exact consumer row, the declared
/// purpose and source fingerprint bound to this session's reconnect
/// generation, the source's own subject and operation allowlists, and the
/// realization facets the transport supplies. A relationship over another
/// socket, under another purpose, for another generation, or one whose source
/// admits another subject, is refused rather than repaired.
pub fn admit_display_endpoint(
    zone: &ZoneId,
    spec: &WaylandSessionSpec,
    session_uid: &ResourceUid,
    binding: &DisplayEndpointBinding,
    observed: &DisplayEndpointObservation<'_>,
    source_uid: &ResourceUid,
    consumer_uid: &ResourceUid,
) -> Result<DisplayEndpointAdmission, WorkerEffectError> {
    let refused = || WorkerEffectError::LaunchRejected;
    let consumer_ref = durable_process_ref(session_uid, binding.role)?;
    if consumer_ref != *binding.consumer_ref()
        || observed.source_generation == 0
        || observed.source_generation != observed.consumer_generation
        || observed.consumer_user.as_ref() != Some(spec.user_ref())
    {
        return Err(refused());
    }
    let (expected_producer, expected_purpose, expected_fingerprint) = match binding.role {
        // The host compositor socket: its declared producer is the session's
        // own execution target, and its purpose and fingerprint are this
        // session's display name and reconnect generation.
        DisplayProcessRole::HostProxy => (
            spec.host_ref().clone(),
            compositor_purpose(spec),
            expected_compositor_fingerprint(spec),
        ),
        DisplayProcessRole::GuestFrontend => (
            durable_process_ref(session_uid, DisplayProcessRole::HostProxy)?,
            PROXY_BINDING_PURPOSE,
            expected_proxy_fingerprint(spec),
        ),
    };
    if observed.spec.producer_ref() != &expected_producer
        || observed.spec.purpose().as_str() != expected_purpose
        || observed.spec.service_fingerprint().map(BoundedText::as_str)
            != Some(expected_fingerprint.as_str())
        || observed.spec.lifecycle_policy() != EndpointLifecyclePolicy::RecycleWithProducer
        || observed.spec.visibility() != EndpointVisibility::Owner
    {
        return Err(refused());
    }
    let required_operation = match binding.role {
        DisplayProcessRole::HostProxy => EndpointOperation::Resolve,
        DisplayProcessRole::GuestFrontend => EndpointOperation::Attach,
    };
    if !observed
        .spec
        .consumer_policy()
        .admits_operation(required_operation)
        || !observed
            .spec
            .consumer_policy()
            .admits_subject(binding.consumer_ref())
    {
        return Err(refused());
    }
    // Target support: the attachment kind's realization facets must be ones the
    // observed transport actually supplies, or the relationship is refused
    // rather than approximated with a wider or narrower presentation.
    let support = display_realization_support(observed);
    if binding
        .request()
        .required_facets()
        .iter()
        .any(|facet| !support.realizes(*facet))
    {
        return Err(refused());
    }
    let key = binding
        .request()
        .key(zone.clone(), source_uid.clone(), consumer_uid.clone())
        .map_err(|_| refused())?;
    Ok(DisplayEndpointAdmission {
        binding: binding.clone(),
        key,
        generation: observed.source_generation,
    })
}

/// One admitted display endpoint relationship.
///
/// The admission is the exact identity of the relationship plus the row
/// generation it was proved against: later evidence cannot reuse it for a
/// different endpoint, consumer, purpose, or generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayEndpointAdmission {
    binding: DisplayEndpointBinding,
    key: BindingKey,
    generation: u64,
}

impl DisplayEndpointAdmission {
    /// Borrow the admitted relationship.
    pub const fn binding(&self) -> &DisplayEndpointBinding {
        &self.binding
    }

    /// Borrow the KTD3 identity of the admitted relationship.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Return the committed row generation this admission was proved against.
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

/// The realization facets the observed endpoint transport can supply.
fn display_realization_support(
    observed: &DisplayEndpointObservation<'_>,
) -> BindingRealizationSupport {
    BindingRealizationSupport::new(match observed.spec.transport() {
        EndpointTransport::Unix | EndpointTransport::FdAttachment => vec![
            BindingRealizationFacet::EndpointDescriptor,
            BindingRealizationFacet::EndpointPathname,
        ],
        _ => vec![BindingRealizationFacet::EndpointDescriptor],
    })
    .unwrap_or_default()
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
    let endpoint_spec = EndpointSpec::new(
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
    });
    endpoint_envelope(
        zone,
        session_ref,
        &durable_endpoint_ref(session_uid, role)?,
        endpoint_spec,
        generation,
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
    let proxy_ref = durable_process_ref(session_uid, DisplayProcessRole::HostProxy)?;
    let endpoint_spec = EndpointSpec::new(
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
        EndpointConsumerPolicy::new(vec![proxy_ref.clone()], Vec::new(), vec![EndpointOperation::Resolve])
            .map_err(|_| WorkerEffectError::LaunchRejected)?,
        EndpointLifecyclePolicy::RecycleWithProducer,
    )
    .map_err(|_| WorkerEffectError::LaunchRejected)?
    // The compositor socket is published to exactly one consumer: this
    // session's host proxy row. Nothing else reaches it.
    .with_binding_publication(EndpointBindingPublication::named(vec![proxy_ref]).map_err(
        |_| WorkerEffectError::LaunchRejected,
    )?);
    endpoint_envelope(
        zone,
        session_ref,
        &durable_compositor_endpoint_ref(session_uid)?,
        endpoint_spec,
        generation,
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
        let expected = vec![
            (compositor.name().as_str().to_owned(), proxy.to_canonical_string()),
            (proxy_endpoint.name().as_str().to_owned(), frontend.to_canonical_string()),
        ];
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