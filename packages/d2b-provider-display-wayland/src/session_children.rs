//! The display session's durable child derivation (U12).
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
//! The derivation is pure: it builds payloads and references from the
//! session's row identity and spec, and never touches host state.

use d2b_contracts_resource::v3::{
    CanonicalJsonValue, RESOURCE_ENVELOPE_DOMAIN_TAG, ResourceRef, ResourceUid, ZoneId,
    canonical_digest, execution_policy::{BoundedText, BoundedToken},
    process::{ExecutionSpec, ProcessClass, ProcessSpec},
};
use d2b_core_controller::OwnedChildIntent;
use d2b_provider_endpoint::endpoint::{
    EndpointAttachmentPolicy, EndpointClass, EndpointConsumerPolicy, EndpointLifecyclePolicy,
    EndpointLocality, EndpointOperation, EndpointSpec, EndpointTransport, EndpointVisibility,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{DisplayProcessRole, WaylandSessionResourceStatus, WaylandSessionSpec, WorkerEffectError};

/// The restart-generation annotation the display session's worker rows
/// carry: the durable generation the display supervisor restarts a worker
/// from.
pub const DISPLAY_RESTART_ANNOTATION: &str = "d2b.d2bus.org/restart-generation";

/// The Process and Endpoint intents one session owns, in the family's
/// preserved order (host proxy, guest frontend; each with its endpoint).
///
/// The intent bodies are the full resource envelopes the display Provider
/// synthesized; the manager owns the child identity, so only the spec and
/// the authored metadata (ownerRef, labels, annotations - the
/// restart-generation annotation the display status reads) are carried.
pub fn display_owned_child_intents(
    zone: &ZoneId,
    session_ref: &ResourceRef,
    session_uid: &ResourceUid,
    spec: &WaylandSessionSpec,
    process_generation: u64,
) -> Result<Vec<OwnedChildIntent>, WorkerEffectError> {
    let mut intents = Vec::with_capacity(4);
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
        let endpoint_ref = durable_endpoint_ref(session_uid, role)?;
        let endpoint =
            durable_endpoint_payload(zone, session_ref, session_uid, role, &process_ref, process_generation)?;
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

/// The durable Endpoint payload of one worker role: the private
/// cross-domain endpoint the worker's wayland traffic rides.
fn durable_endpoint_payload(
    zone: &ZoneId,
    session_ref: &ResourceRef,
    session_uid: &ResourceUid,
    role: DisplayProcessRole,
    producer_ref: &ResourceRef,
    generation: u64,
) -> Result<Vec<u8>, WorkerEffectError> {
    let owner_ref = session_ref;
    let provider_ref = ResourceRef::parse("Provider/display-wayland")
        .map_err(|_| WorkerEffectError::LaunchRejected)?;
    let (endpoint_class, transport, purpose, fingerprint) = match role {
        DisplayProcessRole::HostProxy => (
            EndpointClass::Data,
            EndpointTransport::FdAttachment,
            "wayland-cross-domain",
            "display-wayland-data-v3",
        ),
        DisplayProcessRole::GuestFrontend => (
            EndpointClass::Transport,
            EndpointTransport::Vsock,
            "guest-cross-domain",
            "guest-frontend-v3",
        ),
    };
    let endpoint_spec = EndpointSpec::new(
        provider_ref,
        producer_ref.clone(),
        endpoint_class,
        transport,
        BoundedToken::parse(purpose).map_err(|_| WorkerEffectError::LaunchRejected)?,
        Some(BoundedText::parse(fingerprint).map_err(|_| WorkerEffectError::LaunchRejected)?),
        EndpointLocality::CrossDomain,
        EndpointVisibility::Zone,
        EndpointAttachmentPolicy::new(
            matches!(role, DisplayProcessRole::HostProxy),
            u16::from(matches!(role, DisplayProcessRole::HostProxy)),
        )
        .map_err(|_| WorkerEffectError::LaunchRejected)?,
        EndpointConsumerPolicy::new(Vec::new(), Vec::new(), vec![EndpointOperation::Resolve])
            .map_err(|_| WorkerEffectError::LaunchRejected)?,
        EndpointLifecyclePolicy::RecycleWithProducer,
    )
    .map_err(|_| WorkerEffectError::LaunchRejected)?;
    let endpoint_ref = durable_endpoint_ref(session_uid, role)?;
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
/// execution target, and the restart-generation annotation.
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
        ExecutionSpec::minimal(
            execution_ref,
            ProcessClass::Worker,
            BoundedToken::parse(template).map_err(|_| WorkerEffectError::LaunchRejected)?,
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
    let payload = serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": "Process",
        "metadata": {
            "name": process_ref.name().as_str(),
            "zone": zone.as_str(),
            "ownerRef": owner_ref.to_canonical_string(),
            "annotations": {
                DISPLAY_RESTART_ANNOTATION: generation.to_string()
            },
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
        assert_eq!(intents.len(), 4);
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
            2
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
}