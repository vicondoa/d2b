//! Manager-backed store backend: the Resource API rewired onto the per-Zone
//! `ResourceManager` (U8, KTD6/KTD8; R23-R25, R28).
//!
//! [`ManagerBackend`] implements [`ResourceStoreBackend`] over the
//! [`ResourceManagerClient`], so the [`crate::ResourceService`] keeps its
//! public operation shapes and wire envelopes while the backend and dispatch
//! change to the single-writer manager:
//!
//! - reads (`get`/`list`/`resolve_ref`) resolve manager runtime views;
//! - mutations admit at the manager boundary under the caller's real
//!   authenticated subject ([`api_subject`]) and persist through
//!   `Ensure`/`Remove` with commit-before-return (F1/AE1);
//! - resource **generation** maps onto the existing Exact-revision wire
//!   semantics (KTD8): a stored resource's wire revision is its generation,
//!   so an `Exact(rev)` precondition rejects a stale generation with the
//!   existing resource-conflict wire error carrying the current generation;
//! - external WATCH rides the existing named-stream handoff with the runtime
//!   revision (epoch + sequence) mapped onto the wire revision
//!   ([`wire_revision`]) and `RevisionExpired` for pre-epoch cursors (R24);
//! - status-shaped writes are rejected: the manager has no durable status
//!   write path (R11/AE6), and this module must never grow one.
//!
//! # Phase A scope notes
//!
//! - The manager has no atomic multi-resource batch: a committed batch
//!   applies its mutations sequentially and rejects a repeated target inside
//!   one batch. Cross-resource batch atomicity is a consolidated-review item
//!   for the composition cutover (U9).
//! - Precondition checks read the manager's authoritative row before the
//!   mutation RPC; the manager mailbox serializes the mutation itself. The
//!   residual check-then-act window narrows to concurrent writers of the
//!   same key inside one daemon epoch.

pub const MODULE_NAME: &str = "manager_backend";

use std::collections::BTreeSet;
use std::sync::Arc;

use d2b_contracts_resource::v3::{
    CanonicalJsonValue, FinalizerId, ResourceEnvelope, ResourceGeneration, ResourceRef,
    ResourceUid, RetryClass, ZoneId, ZoneRevision, canonical_digest,
    RESOURCE_ENVELOPE_DOMAIN_TAG,
};
use d2b_resource_runtime::manager::{
    DesiredResource, MutationSubject, ResourceManagerClient, ResourceSelector, ResourceView,
};
use d2b_resource_runtime::revision::RuntimeRevision;
use d2b_resource_runtime::resource::ResourceStatus;
use d2b_resource_runtime::spec_store::{ResourceProvenance, StoredDesiredResource};
use d2b_resource_runtime::watch::{
    WatchRegistration as RuntimeWatchRegistration, WatchSelector as RuntimeWatchSelector,
};
use d2b_resource_runtime::{
    error::{DriverFailure, ResourceError},
    identity::ResourceKey as RuntimeResourceKey,
};
use d2b_resource_store::mutation_seal::MutationSealAcceptor;
use d2b_resource_store::{
    AdmittedAuthorization, ExpectedRevision, MutationSealBody, ResourceMutationKind,
    SealedMutation, StoreCommitResult, StoreError, StoreErrorKind, StoreFilter, StoreGetRequest,
    StoreInspectSchemaRequest, StoreListRequest, StoreListResult, StoreMutation, StoreProjection,
    StoreResolveRequest, StoreResolvedIdentity, StoreWatchReceipt, StoreWatchRequest,
    StoredResource, StoredSchema,
};

use crate::ResourceStoreBackend;

/// The runtime-revision to wire-revision mapping (U5 budget): the wire
/// carries `(epoch_seconds << 32) | sequence`, where `epoch_seconds =
/// epoch_nanos / 1_000_000_000` and the sequence occupies the low 32 bits.
/// Order-preserving within an epoch; cursors from other epochs never decode
/// (see [`decode_wire_revision`]).
pub fn wire_revision(revision: RuntimeRevision) -> u64 {
    (revision.epoch / 1_000_000_000) << 32 | (revision.sequence & 0xffff_ffff)
}

/// Decode a wire watch cursor against the daemon's current epoch.
///
/// The wire carries only the epoch's second count, so a cursor is
/// reconstructible only inside the current daemon epoch. A cursor whose
/// epoch seconds differ from the live epoch - a previous daemon lifetime, a
/// pre-cutover durable-store revision (whose numeric range has no epoch
/// bits), or a future cursor - returns `None` and the registration fails
/// with `RevisionExpired` (R24/AE4); the client relists.
pub(crate) fn decode_wire_revision(wire: u64, live_epoch_nanos: u64) -> Option<RuntimeRevision> {
    let seconds = wire >> 32;
    if seconds == 0 || seconds != live_epoch_nanos / 1_000_000_000 {
        return None;
    }
    Some(RuntimeRevision::new(live_epoch_nanos, wire & 0xffff_ffff))
}

/// Deterministic stable uid for a manager key. This replicates the runtime's
/// deterministic uid derivation byte for byte (R8: identities reconstruct
/// identically after restart); [`ManagerBackend::commit_verified`]
/// cross-checks every committed handle against it, so a derivation change in
/// the runtime fails closed here instead of persisting a wrong identity.
fn manager_uid(key: &RuntimeResourceKey) -> [u8; 16] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"d2b-resource-uid/v1\x00");
    hasher.update(key.zone.as_bytes());
    hasher.update([0u8]);
    hasher.update(key.type_name.as_bytes());
    hasher.update([0u8]);
    hasher.update(key.name.as_bytes());
    let digest = hasher.finalize();
    let mut uid = [0u8; 16];
    uid.copy_from_slice(&digest[..16]);
    uid
}

/// Render a manager row uid as the wire UUIDv4 identity: the shape bits are
/// forced onto the stable digest so the identity survives the closed
/// [`ResourceUid`] contract unchanged across restarts.
pub(crate) fn row_uid(uid: &[u8; 16]) -> ResourceUid {
    let mut bytes = *uid;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let text = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    );
    ResourceUid::parse(text).expect("shaped row uids satisfy the UUIDv4 contract")
}

fn error(
    kind: StoreErrorKind,
    current: Option<u64>,
    retry: RetryClass,
    reason: &'static str,
) -> StoreError {
    let retry_after_ms = (retry == RetryClass::AfterDelay).then_some(1_000);
    StoreError::new(kind, current.map(ZoneRevision::new), retry_after_ms, retry, reason)
}

fn not_found() -> StoreError {
    error(StoreErrorKind::ResourceNotFound, None, RetryClass::Never, "resource-not-found")
}

fn conflict(current_generation: u64, reason: &'static str) -> StoreError {
    error(
        StoreErrorKind::ResourceConflict,
        Some(current_generation),
        RetryClass::Reauthorize,
        reason,
    )
}

fn already_exists(current_generation: u64) -> StoreError {
    error(
        StoreErrorKind::ResourceAlreadyExists,
        Some(current_generation),
        RetryClass::Reauthorize,
        "resource-already-exists",
    )
}

/// API status updates have no durable write path (R11/AE6): status exists
/// only in memory at the authoritative actor and on the actual target.
fn status_write_rejected() -> StoreError {
    error(
        StoreErrorKind::ResourceStatusOwnerMismatch,
        None,
        RetryClass::Never,
        "resource-status-owner-mismatch",
    )
}

fn envelope_invalid() -> StoreError {
    error(
        StoreErrorKind::ResourceSchemaInvalid,
        None,
        RetryClass::Never,
        "resource-envelope-invalid",
    )
}

/// Map a manager error onto the closed store error classification.
fn map_manager_error(failure: ResourceError) -> StoreError {
    match failure {
        ResourceError::DeletingConflict { .. } => {
            error(
                StoreErrorKind::ResourceConflict,
                None,
                RetryClass::Reauthorize,
                "resource-deleting",
            )
        }
        ResourceError::Provider { .. } | ResourceError::Driver(_) => {
            // The spec row stays durable (F1); the resource recovers on the
            // next Ensure or manager restart.
            error(
                StoreErrorKind::ResourceProviderUnavailable,
                None,
                RetryClass::AfterDelay,
                "resource-provider-unavailable",
            )
        }
        ResourceError::AdmissionDenied { .. } => {
            error(
                StoreErrorKind::AuthorizationDenied,
                None,
                RetryClass::Reauthorize,
                "admission-denied",
            )
        }
        other => {
            tracing::warn!(failure = %other, "manager-backed store call failed");
            error(
                StoreErrorKind::ResourcePlaneUnavailable,
                None,
                RetryClass::AfterDelay,
                "manager-unavailable",
            )
        }
    }
}

/// The API caller subject (R28): API operations admit at the manager
/// boundary under the exact subject the authorization evaluation captured,
/// with API provenance.
pub fn api_subject(authorization: &AdmittedAuthorization) -> MutationSubject {
    MutationSubject {
        principal: authorization.subject_ref.to_canonical_string(),
        origin: ResourceProvenance::Api,
    }
}

/// The Nix bundle materialization subject (U10 wires it): ingestion admits
/// under the zone-authority/bundle identity, never under an API caller.
pub fn nix_bundle_subject(bundle_identity: &str) -> MutationSubject {
    MutationSubject {
        principal: format!("nix:{bundle_identity}"),
        origin: ResourceProvenance::Nix,
    }
}

/// The parent-actor subject for owned-child ensures (U9/U10 wire it): a
/// parent actor's child mutation admits under the owning resource's
/// identity, mirroring the manager's internal cascade subject.
pub fn resource_owner_subject(owner: &RuntimeResourceKey) -> MutationSubject {
    MutationSubject {
        principal: owner.to_string(),
        origin: ResourceProvenance::Resource,
    }
}

/// Stamp store-authoritative identity onto a canonical resource envelope at
/// the JSON level: the manager row is authoritative for `uid`, the
/// generation, and the wire revision (mapped to the generation per KTD8), so
/// every read reconstructs an envelope consistent with its row.
fn stamp_envelope(
    canonical: &[u8],
    uid: &str,
    generation: u64,
) -> Result<(Vec<u8>, String), StoreError> {
    let mut value =
        CanonicalJsonValue::parse(canonical).map_err(|_| envelope_invalid())?;
    let CanonicalJsonValue::Object(root) = &mut value else {
        return Err(envelope_invalid());
    };
    let Some(CanonicalJsonValue::Object(metadata)) = root.get_mut("metadata") else {
        return Err(envelope_invalid());
    };
    metadata.insert("uid".to_owned(), CanonicalJsonValue::String(uid.to_owned()));
    metadata
        .insert("generation".to_owned(), CanonicalJsonValue::Integer(generation as i64));
    metadata.insert("revision".to_owned(), CanonicalJsonValue::Integer(generation as i64));
    let stamped = value.to_canonical_bytes();
    let digest = canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &stamped);
    Ok((stamped, digest))
}

/// Extract the metadata sub-envelope of a canonical resource envelope: the
/// manager persists `spec` and `metadata` as opaque envelopes, and the full
/// canonical envelope inside `spec` stays the change-detection source.
fn extract_metadata(canonical: &[u8]) -> Result<Vec<u8>, StoreError> {
    let value = CanonicalJsonValue::parse(canonical).map_err(|_| envelope_invalid())?;
    let metadata = value
        .as_object()
        .and_then(|root| root.get("metadata"))
        .ok_or_else(envelope_invalid)?;
    Ok(metadata.to_canonical_bytes())
}

/// Apply finalizer add/remove to a canonical envelope's metadata at the JSON
/// level (the typed metadata has no public finalizer setter), re-rendering
/// the exact canonical profile.
fn apply_finalizers(
    canonical: &[u8],
    add: &[FinalizerId],
    remove: &[FinalizerId],
) -> Result<Vec<u8>, StoreError> {
    let mut value = CanonicalJsonValue::parse(canonical).map_err(|_| envelope_invalid())?;
    let CanonicalJsonValue::Object(root) = &mut value else {
        return Err(envelope_invalid());
    };
    let Some(CanonicalJsonValue::Object(metadata)) = root.get_mut("metadata") else {
        return Err(envelope_invalid());
    };
    let existing = match metadata.get("finalizers") {
        Some(CanonicalJsonValue::Array(values)) => values
            .iter()
            .map(|value| match value {
                CanonicalJsonValue::String(value) => Ok(value.clone()),
                _ => Err(envelope_invalid()),
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(envelope_invalid()),
        None => Vec::new(),
    };
    let mut finalizers: BTreeSet<String> = existing.into_iter().collect();
    for id in add {
        finalizers.insert(id.as_str().to_owned());
    }
    for id in remove {
        finalizers.remove(id.as_str());
    }
    metadata.insert(
        "finalizers".to_owned(),
        CanonicalJsonValue::Array(finalizers.into_iter().map(CanonicalJsonValue::String).collect()),
    );
    Ok(value.to_canonical_bytes())
}

/// Enforce the wire precondition against the authoritative manager row:
/// `Exact(rev)` compares the resource generation (KTD8 mapping), and the
/// expected uid binds the exact stable identity.
fn check_precondition(mutation: &StoreMutation, row: &StoredDesiredResource) -> Result<(), StoreError> {
    match mutation.expected {
        ExpectedRevision::Exact(revision) if revision.get() == row.generation => {}
        ExpectedRevision::Exact(_) => return Err(conflict(row.generation, "resource-conflict")),
        ExpectedRevision::CreateAbsent => {
            return Err(conflict(row.generation, "resource-conflict"));
        }
    }
    if let Some(expected) = &mutation.expected_uid
        && expected != &row_uid(&row.uid)
    {
        return Err(conflict(row.generation, "resource-uid-mismatch"));
    }
    Ok(())
}

/// The owner reference string of a stored envelope, when one is present.
fn owner_ref_of(spec: &[u8]) -> Option<String> {
    let value = CanonicalJsonValue::parse(spec).ok()?;
    value
        .as_object()?
        .get("metadata")?
        .as_object()?
        .get("ownerRef")?
        .as_object()
        .map(|_| ())?;
    None
}

/// The closed list-filter projection, evaluated against the authoritative
/// row attributes (same closed filter set as the durable store plane).
fn filters_match(filters: &[StoreFilter], stored: &StoredResource, spec: &[u8]) -> bool {
    filters.iter().all(|filter| match filter.field.as_str() {
        "metadata.name" => {
            filter.values.iter().any(|value| value == stored.resource_ref.name().as_str())
        }
        "type" => filter
            .values
            .iter()
            .any(|value| value == stored.resource_ref.resource_type().as_str()),
        "assignment.resourceUid" => filter.values.iter().any(|value| value == stored.uid.as_str()),
        "owner.resourceUid" => stored.owner_uid.as_ref().is_some_and(|owner| {
            filter.values.iter().any(|value| value == owner.as_str())
        }),
        "owner.resourceRef" => {
            let owner = owner_ref_of(spec);
            filter
                .values
                .iter()
                .any(|value| owner.as_ref().is_some_and(|candidate| candidate == value))
        }
        _ => false,
    })
}

/// Mirror the durable store's read projections.
fn project_resource(
    resource: &mut StoredResource,
    projection: StoreProjection,
) -> Result<(), StoreError> {
    if projection == StoreProjection::Full {
        return Ok(());
    }
    let mut value =
        CanonicalJsonValue::parse(&resource.canonical_json).map_err(|_| envelope_invalid())?;
    let CanonicalJsonValue::Object(root) = &mut value else {
        return Err(envelope_invalid());
    };
    match projection {
        StoreProjection::Full => unreachable!("full projection returned above"),
        StoreProjection::BaseOnly => {
            for layer in ["spec", "status"] {
                if let Some(CanonicalJsonValue::Object(section)) = root.get_mut(layer) {
                    section.remove("provider");
                }
            }
        }
        StoreProjection::MetadataOnly => {
            root.retain(|key, _| matches!(key.as_str(), "apiVersion" | "metadata" | "type"));
        }
    }
    resource.canonical_json = value.to_canonical_bytes();
    Ok(())
}

fn stored_of(
    zone: &str,
    type_name: &str,
    name: &str,
    uid: &[u8; 16],
    owner_uid: Option<ResourceUid>,
    generation: u64,
    spec: &[u8],
) -> StoredResource {
    // Rows not created through this backend (e.g. Nix-materialized rows whose
    // spec the compiler owns) keep their bytes verbatim; the digest is
    // recomputed only when the spec is a complete resource envelope.
    let payload_digest = CanonicalJsonValue::parse(spec)
        .ok()
        .map(|canonical| canonical_digest(RESOURCE_ENVELOPE_DOMAIN_TAG, &canonical.to_canonical_bytes()))
        .unwrap_or_default();
    StoredResource {
        resource_ref: ResourceRef::parse(&format!("{type}/{name}", type = type_name, name = name))
            .expect("manager keys carry validated type and name components"),
        zone: ZoneId::parse(zone).expect("manager keys carry validated Zones"),
        uid: row_uid(uid),
        owner_uid,
        owner_generation: None,
        generation: ResourceGeneration::new(generation.max(1))
            .expect("manager generations are nonzero"),
        revision: ZoneRevision::new(generation.max(1)),
        canonical_json: spec.to_vec(),
        payload_digest,
    }
}

/// Map one manager runtime view onto the stored-resource contract, carrying
/// the actor's in-memory status onto the read (R11): the durable row has no
/// status, the live actor does.
///
/// Public because it is the canonical manager-row projection: the daemon's
/// reader bridges (G5, U12) merge manager rows into the old plane's
/// store-shaped readers through exactly this rendering, so a bridged row is
/// identical to the row the manager-backed API serves.
pub fn manager_row_stored(view: &ResourceView) -> Result<StoredResource, StoreError> {
    stored_from_view(view)
}

fn stored_from_view(view: &ResourceView) -> Result<StoredResource, StoreError> {
    let canonical = render_envelope(
        &view.key,
        &view.uid,
        view.generation,
        &view.metadata,
        &view.spec,
        view.owner_key.as_ref(),
        view.provenance,
    )?;
    let mut stored = stored_of(
        &view.key.zone,
        &view.key.type_name,
        &view.key.name,
        &view.uid,
        None,
        view.generation,
        &canonical,
    );
    let mut stamped = false;
    if let Some(status) = view.status
        && view.status_generation == Some(view.generation)
    {
        // Only a status the actor published for this exact row generation is
        // observed state of it; a status carried over from an older
        // generation is not (see `ResourceView::status_generation`).
        stamp_status(
            &mut stored,
            status,
            view.generation,
            view.observed_status_projection(),
        )?;
        stamped = true;
    }
    if view.deleting {
        stamp_deletion_request(&mut stored)?;
        stamped = true;
    }
    if stamped {
        // The stamps above edit the rendered envelope after `stored_of`
        // derived the digest; refresh it so the strict readers' recomputed
        // envelope digest keeps matching the row.
        reseal_envelope(&mut stored)?;
    }
    Ok(stored)
}

/// The wire envelope of one manager row.
///
/// API-created rows persist the full canonical envelope minus status (KTD2);
/// Nix-materialized rows persist the compiled spec with its authored metadata
/// beside it. Reads always answer with the envelope shape - the public
/// surface stays uniform across both provenances - so a spec-shaped row is
/// rendered against its metadata and the row's authoritative identity, and
/// the rendered bytes are always a complete envelope the strict readers
/// (`ResourceEnvelope::from_json`) decode: a rendered row that misses a
/// required member is a row every store-shaped consumer refuses.
fn render_envelope(
    key: &RuntimeResourceKey,
    uid: &[u8; 16],
    generation: u64,
    metadata: &[u8],
    spec: &[u8],
    resolved_owner: Option<&RuntimeResourceKey>,
    provenance: ResourceProvenance,
) -> Result<Vec<u8>, StoreError> {
    let spec_value = CanonicalJsonValue::parse(spec).map_err(|_| envelope_invalid())?;
    if spec_value.as_object().is_some_and(|root| {
        root.contains_key("apiVersion") && root.contains_key("spec")
    }) {
        return Ok(spec.to_vec());
    }
    let authored: serde_json::Value =
        serde_json::from_slice(metadata).unwrap_or(serde_json::Value::Null);
    let field = |name: &str, fallback: serde_json::Value| {
        authored
            .get(name)
            .cloned()
            .unwrap_or_else(|| fallback.clone())
    };
    // The stable identity is data, not log material: `ResourceUid`'s
    // `Display` is a redaction stub, and rendering it here would publish
    // `ResourceUid(<redacted>)` as the row's uid to every API client (the
    // public delete precondition resolves the exact uid from this field).
    let row_identity = row_uid(uid);
    let generation = generation.max(1);
    // The strict metadata contract requires a management owner. API-created
    // rows author one; a spec-shaped row (the Nix bundle ingestion persists
    // no `managedBy`) derives it from the row's provenance, the same closed
    // vocabulary the durable plane assigns (configuration for materialized
    // rows, controller for owned children, api for API writes).
    let managed_by = authored
        .get("managedBy")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| {
            serde_json::Value::String(
                match provenance {
                    ResourceProvenance::Nix => "configuration",
                    ResourceProvenance::Api => "api",
                    ResourceProvenance::Resource => "controller",
                }
                .to_owned(),
            )
        });
    // `managedBy: configuration` requires an owning configuration
    // generation (strict metadata construction refuses configuration
    // ownership without one). A manager row never persists the bundle
    // ordinal - the new plane's configuration authority is the row's
    // provenance - so the row's own generation is the only ordinal the
    // rendering can carry honestly.
    let configuration_generation = match authored
        .get("configurationGeneration")
        .filter(|value| !value.is_null())
    {
        Some(value) => value.clone(),
        None if managed_by.as_str() == Some("configuration") => serde_json::json!(generation),
        None => serde_json::Value::Null,
    };
    // Always a complete status: the actor's classification when one is
    // stamped for this generation (see `stamp_status`, which replaces this
    // fallback), `Pending` at the row's own generation otherwise - the
    // honest "nothing published yet" phase, unlike a fabricated `Ready`.
    let envelope = serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "metadata": {
            "annotations": field("annotations", serde_json::json!({})),
            "configurationGeneration": configuration_generation,
            "createdAt": field("createdAt", serde_json::json!("1970-01-01T00:00:00.000Z")),
            "deletionRequestedAt": field("deletionRequestedAt", serde_json::Value::Null),
            "finalizers": field("finalizers", serde_json::json!([])),
            "generation": generation,
            "labels": field("labels", serde_json::json!({})),
            "managedBy": managed_by,
            "name": key.name,
            "ownerRef": authored
                .get("ownerRef")
                .cloned()
                .filter(|value| !value.is_null())
                .unwrap_or_else(|| match resolved_owner {
                    Some(owner) => serde_json::Value::String(format!(
                        "{}/{}",
                        owner.type_name, owner.name
                    )),
                    None => serde_json::Value::Null,
                }),
            "revision": generation,
            "uid": row_identity.as_str(),
            "updatedAt": field("updatedAt", serde_json::json!("1970-01-01T00:00:00.000Z")),
            "zone": key.zone,
        },
        "spec": serde_json::to_value(&spec_value).map_err(|_| envelope_invalid())?,
        "status": manager_status_value("Pending", None, generation, None),
        "type": key.type_name,
    });
    let canonical =
        CanonicalJsonValue::parse(&serde_json::to_vec(&envelope).map_err(|_| envelope_invalid())?)
            .map_err(|_| envelope_invalid())?;
    // Fail closed: the rendered row must decode as a complete envelope, and
    // rendering through the decoded form keeps the row bytes byte-identical
    // to `ResourceEnvelope::canonical_bytes`, so the digest this crate
    // stores for the row is the digest the strict readers recompute.
    let envelope = ResourceEnvelope::from_json(&canonical.to_canonical_bytes())
        .map_err(|_| envelope_invalid())?;
    envelope.canonical_bytes().map_err(|_| envelope_invalid())
}

/// The manager plane's wire status for one row: the actor classification
/// projected onto the contract's closed status shape (the shape the durable
/// plane materializes and the strict readers decode) at the row's own
/// generation.
///
/// The manager runs no update assessment, so the currency object reports
/// `Unknown` with empty owned/dependency sets. The free-form `resource` layer
/// carries the row's driver-published projection when it has one; otherwise a
/// failed actor's closed failure classification rides there (the status
/// object itself is closed to unknown fields).
fn manager_status_value(
    phase: &str,
    failure: Option<&DriverFailure>,
    generation: u64,
    projection: Option<&serde_json::Value>,
) -> serde_json::Value {
    let resource = match projection {
        // The driver published this row's own `status.resource` layer (the
        // Cloud Hypervisor Guest runtime status): it is what the type's
        // consumers read, so it stands in for the default empty layer.
        Some(projection) => projection.clone(),
        None => match failure {
            Some(failure) => serde_json::json!({
                "driverFailure": {
                    "operation": format!("{:?}", failure.op()),
                    "retryable": failure.class()
                        == d2b_resource_runtime::error::FailureClass::Retryable,
                },
            }),
            None => serde_json::json!({}),
        },
    };
    serde_json::json!({
        "completedAt": serde_json::Value::Null,
        "conditions": [],
        "lastReconciledAt": serde_json::Value::Null,
        "observedGeneration": generation,
        "outcome": serde_json::Value::Null,
        "phase": phase,
        "resource": resource,
        "startedAt": serde_json::Value::Null,
        "update": {
            "dependencies": {"count": 0, "refs": []},
            "disruption": "None",
            "lastAssessedAt": serde_json::Value::Null,
            "observedGeneration": generation,
            "operationId": serde_json::Value::Null,
            "owned": {"count": 0, "refs": []},
            "preserveState": true,
            "reasons": [],
            "state": "Unknown",
            "targetGeneration": generation,
        },
    })
}

/// Project the closed runtime classification onto the wire status.
///
/// The manager's view carries the runtime classification and nothing else,
/// so the projection is the phase and the row generation the status is
/// current for; a failed actor's closed failure classification rides under
/// the free-form status `resource` layer (`status` itself is closed). A
/// Ready row reports `observedGeneration == metadata.generation`, the
/// manager-view form of the old plane's converged status.
fn stamp_status(
    stored: &mut StoredResource,
    status: ResourceStatus,
    generation: u64,
    projection: Option<&serde_json::Value>,
) -> Result<(), StoreError> {
    let (phase, failure) = match status {
        ResourceStatus::Pending | ResourceStatus::Recovering | ResourceStatus::Reconciling => {
            ("Pending", None)
        }
        ResourceStatus::Ready => ("Ready", None),
        ResourceStatus::Failed(failure) => ("Failed", Some(failure)),
        // The closed wire vocabulary has no `Deleting` phase; `Deleted` is
        // the tombstone projection the other manager-row planners
        // (`interaction_effects::view_phase`,
        // `shared_provider_effects::live_phase`) already apply.
        ResourceStatus::Deleting => ("Deleted", None),
    };
    let projected = manager_status_value(phase, failure.as_ref(), generation, projection);
    let mut value =
        CanonicalJsonValue::parse(&stored.canonical_json).map_err(|_| envelope_invalid())?;
    let CanonicalJsonValue::Object(root) = &mut value else {
        return Err(envelope_invalid());
    };
    let status = CanonicalJsonValue::parse(
        &serde_json::to_vec(&projected).map_err(|_| envelope_invalid())?,
    )
    .map_err(|_| envelope_invalid())?;
    root.insert("status".to_owned(), status);
    stored.canonical_json = value.to_canonical_bytes();
    Ok(())
}

/// Re-seal a rendered row after a post-render stamp: the stamped bytes are
/// re-decoded as a complete envelope and the row digest is refreshed from the
/// canonical form the strict readers recompute, so `payload_digest` keeps
/// agreeing with the envelope it belongs to.
fn reseal_envelope(stored: &mut StoredResource) -> Result<(), StoreError> {
    let envelope =
        ResourceEnvelope::from_json(&stored.canonical_json).map_err(|_| envelope_invalid())?;
    stored.canonical_json = envelope.canonical_bytes().map_err(|_| envelope_invalid())?;
    stored.payload_digest = envelope.digest().map_err(|_| envelope_invalid())?;
    Ok(())
}

/// Project the durable deletion mark onto the wire envelope.
///
/// The manager row records the deletion *request* as its `deleting` mark; the
/// instant itself stays in the store's audit stream in Phase A. The wire
/// contract's `deletionRequestedAt` is what consumers read as "deletion
/// requested" (the old plane stamped it at the delete transition), so a row
/// that carries no instant is stamped at its latest known change: the epoch
/// fallback for a spec-shaped row, `max(createdAt, updatedAt)` for an
/// envelope-shaped one - never an instant the strict metadata contract
/// rejects as preceding `createdAt`. The value is constant per row, so
/// repeated reads and diffs stay stable.
fn stamp_deletion_request(stored: &mut StoredResource) -> Result<(), StoreError> {
    const DELETION_FALLBACK: &str = "1970-01-01T00:00:00.000Z";
    let mut value =
        CanonicalJsonValue::parse(&stored.canonical_json).map_err(|_| envelope_invalid())?;
    let CanonicalJsonValue::Object(root) = &mut value else {
        return Err(envelope_invalid());
    };
    let Some(CanonicalJsonValue::Object(metadata)) = root.get_mut("metadata") else {
        return Err(envelope_invalid());
    };
    if matches!(
        metadata.get("deletionRequestedAt"),
        Some(CanonicalJsonValue::String(_))
    ) {
        // The row already carries the instant; it wins.
        return Ok(());
    }
    let timestamp = |key: &str| match metadata.get(key) {
        Some(CanonicalJsonValue::String(value)) => Some(value.clone()),
        _ => None,
    };
    let stamped = match (timestamp("createdAt"), timestamp("updatedAt")) {
        (Some(created), Some(updated)) => created.max(updated),
        _ => DELETION_FALLBACK.to_owned(),
    };
    metadata.insert(
        "deletionRequestedAt".to_owned(),
        CanonicalJsonValue::String(stamped),
    );
    stored.canonical_json = value.to_canonical_bytes();
    Ok(())
}

fn stored_from_row(row: &StoredDesiredResource) -> StoredResource {
    let canonical = render_envelope(
        &row.key,
        &row.uid,
        row.generation,
        &row.metadata,
        &row.spec,
        None,
        row.provenance,
    )
    .unwrap_or_else(|_| row.spec.clone());
    stored_of(
        &row.key.zone,
        &row.key.type_name,
        &row.key.name,
        &row.uid,
        row.owner_uid.map(|uid| row_uid(&uid)),
        row.generation,
        &canonical,
    )
}

/// Watch selectors narrow by the closed metadata.name and type filters; the
/// remaining durable-plane filters have no key-level equivalent on the hub
/// (Phase A minimal external WATCH).
fn watch_selector(request: &StoreWatchRequest) -> Result<RuntimeWatchSelector, StoreError> {
    let mut types: BTreeSet<String> = request
        .resource_types
        .iter()
        .map(|resource_type| resource_type.as_str().to_owned())
        .collect();
    let mut names: BTreeSet<String> = request
        .resource_names
        .iter()
        .map(|name| name.as_str().to_owned())
        .collect();
    for filter in &request.filters {
        match filter.field.as_str() {
            "metadata.name" => names.extend(filter.values.iter().cloned()),
            "type" => types.extend(filter.values.iter().cloned()),
            _ => {
                return Err(error(
                    StoreErrorKind::ResourceSchemaInvalid,
                    None,
                    RetryClass::Never,
                    "watch-filter-unavailable",
                ));
            }
        }
    }
    let zone = request.zone.as_str().to_owned();
    Ok(RuntimeWatchSelector::with_predicate(move |key| {
        key.zone == zone
            && (types.is_empty() || types.contains(key.type_name.as_str()))
            && (names.is_empty() || names.contains(key.name.as_str()))
    }))
}

/// The manager-backed resource-store backend (U8): one per Zone manager.
///
/// The composition (U9) constructs it against the Zone's manager actor:
///
/// ```text
/// let authorizer = Arc::new(NativeAuthorizer::new(catalog, policy)?);
/// let acceptor = authorizer.take_store_seal(manager_seal_identity())?;
/// let backend = ManagerBackend::new(client, hub, acceptor);
/// let service = ResourceService::new_with_zone_uid(Arc::new(backend), authorizer, zone_uid)?;
/// ```
pub struct ManagerBackend {
    manager: ResourceManagerClient,
    hub: Arc<d2b_resource_runtime::watch::WatchHub>,
    streams: crate::watch::ManagerWatchStreams,
    acceptor: MutationSealAcceptor,
}

impl core::fmt::Debug for ManagerBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ManagerBackend(<redacted>)")
    }
}

impl ManagerBackend {
    /// Bind the backend to one Zone manager, its watch hub, and the paired
    /// mutation-seal acceptor taken from the same authorizer that seals the
    /// mutations.
    pub fn new(
        manager: ResourceManagerClient,
        hub: Arc<d2b_resource_runtime::watch::WatchHub>,
        acceptor: MutationSealAcceptor,
    ) -> Self {
        Self {
            manager,
            hub,
            streams: crate::watch::ManagerWatchStreams::default(),
            acceptor,
        }
    }

    /// The named-stream handoff: the authenticated bus adapter takes the
    /// replay/live delivery for a registered watch by its receipt stream
    /// name and pumps it through its sink.
    pub fn watch_streams(&self) -> &crate::watch::ManagerWatchStreams {
        &self.streams
    }

    fn runtime_key(zone: &ZoneId, target: &ResourceRef) -> RuntimeResourceKey {
        RuntimeResourceKey::new(
            zone.as_str(),
            target.resource_type().as_str(),
            target.name().as_str(),
        )
    }

    async fn row(
        &self,
        key: &RuntimeResourceKey,
    ) -> Result<Option<StoredDesiredResource>, StoreError> {
        self.manager.get_row(key.clone()).await.map_err(map_manager_error)
    }

    /// The authoritative committed view after a mutation returned.
    async fn committed(&self, key: &RuntimeResourceKey) -> Result<StoredResource, StoreError> {
        match self.manager.get(key.clone()).await.map_err(map_manager_error)? {
            Some(view) => stored_from_view(&view),
            None => Err(not_found()),
        }
    }

    async fn commit_mutation(
        &self,
        subject: &MutationSubject,
        mutation: &StoreMutation,
    ) -> Result<Option<StoredResource>, StoreError> {
        let key = Self::runtime_key(&mutation.zone, &mutation.target);
        match mutation.kind {
            // R11/AE6: the API has no status write path. Status is published
            // by the authoritative resource actor only; this arm must never
            // reach the manager.
            ResourceMutationKind::UpdateStatus => Err(status_write_rejected()),
            ResourceMutationKind::Create => {
                if let Some(row) = self.row(&key).await? {
                    return Err(already_exists(row.generation));
                }
                let canonical = mutation
                    .canonical_resource
                    .as_deref()
                    .ok_or_else(envelope_invalid)?;
                let (stamped, _) =
                    stamp_envelope(canonical, row_uid(&manager_uid(&key)).as_str(), 1)?;
                let metadata = extract_metadata(&stamped)?;
                let owner = mutation
                    .owner
                    .as_ref()
                    .map(|owner| Self::runtime_key(&mutation.zone, owner));
                let handle = self
                    .manager
                    .ensure(subject.clone(), owner, DesiredResource {
                        key: key.clone(),
                        spec: stamped,
                        metadata,
                        provenance: ResourceProvenance::Api,
                    })
                    .await
                    .map_err(map_manager_error)?;
                if handle.uid != manager_uid(&key) {
                    return Err(error(
                        StoreErrorKind::InternalIntegrityFailure,
                        None,
                        RetryClass::Never,
                        "row-identity-drift",
                    ));
                }
                Ok(Some(self.committed(&key).await?))
            }
            kind @ (ResourceMutationKind::UpdateSpec
            | ResourceMutationKind::UpdateMetadata
            | ResourceMutationKind::UpdateFinalizers) => {
                let row = self.row(&key).await?.ok_or_else(not_found)?;
                check_precondition(mutation, &row)?;
                let next = match kind {
                    ResourceMutationKind::UpdateFinalizers => apply_finalizers(
                        &row.spec,
                        &mutation.add_finalizers,
                        &mutation.remove_finalizers,
                    )?,
                    _ => mutation.canonical_resource.clone().ok_or_else(envelope_invalid)?,
                };
                // A byte-identical desired envelope is a no-op: the manager
                // keeps the row and generation unchanged.
                if next == row.spec {
                    return Ok(Some(stored_from_row(&row)));
                }
                let (stamped, _) =
                    stamp_envelope(&next, row_uid(&row.uid).as_str(), row.generation + 1)?;
                let metadata = extract_metadata(&stamped)?;
                let owner = self.owner_for_update(&row, mutation).await?;
                let handle = self
                    .manager
                    .ensure(subject.clone(), owner, DesiredResource {
                        key: key.clone(),
                        spec: stamped,
                        metadata,
                        provenance: ResourceProvenance::Api,
                    })
                    .await
                    .map_err(map_manager_error)?;
                if handle.uid != manager_uid(&key) {
                    return Err(error(
                        StoreErrorKind::InternalIntegrityFailure,
                        None,
                        RetryClass::Never,
                        "row-identity-drift",
                    ));
                }
                Ok(Some(self.committed(&key).await?))
            }
            ResourceMutationKind::Delete => {
                let row = self.row(&key).await?.ok_or_else(not_found)?;
                check_precondition(mutation, &row)?;
                self.manager
                    .remove(subject.clone(), key.clone())
                    .await
                    .map_err(map_manager_error)?;
                // The identity of the deleted resource is what the wire
                // delete response carries.
                Ok(Some(stored_from_row(&row)))
            }
        }
    }

    /// Preserve the owned-child graph across updates: the manager recomputes
    /// the owner binding from the `owner` argument on every Ensure, so an
    /// update of an owned child must name its owner. An explicit metadata
    /// owner change (Create or UpdateMetadata) carries the new owner.
    async fn owner_for_update(
        &self,
        row: &StoredDesiredResource,
        mutation: &StoreMutation,
    ) -> Result<Option<RuntimeResourceKey>, StoreError> {
        if let Some(owner) = &mutation.owner {
            return Ok(Some(Self::runtime_key(&mutation.zone, owner)));
        }
        let Some(owner_uid) = row.owner_uid else {
            return Ok(None);
        };
        let views = self
            .manager
            .list(ResourceSelector::default())
            .await
            .map_err(map_manager_error)?;
        Ok(views
            .iter()
            .find(|view| view.uid == owner_uid)
            .map(|view| {
                RuntimeResourceKey::new(
                    view.key.zone.clone(),
                    view.key.type_name.clone(),
                    view.key.name.clone(),
                )
            }))
    }
}

impl ResourceStoreBackend for ManagerBackend {
    async fn get(&self, request: StoreGetRequest) -> Result<StoredResource, StoreError> {
        let key = Self::runtime_key(&request.zone, &request.target);
        let Some(view) = self.manager.get(key).await.map_err(map_manager_error)? else {
            return Err(not_found());
        };
        if let Some(expected) = &request.expected_uid
            && expected != &row_uid(&view.uid)
        {
            return Err(conflict(view.generation, "resource-uid-mismatch"));
        }
        let mut stored = stored_from_view(&view)?;
        project_resource(&mut stored, request.projection)?;
        Ok(stored)
    }

    async fn list(&self, request: StoreListRequest) -> Result<StoreListResult, StoreError> {
        let mut selector = ResourceSelector::default();
        selector.zone = Some(request.zone.as_str().to_owned());
        if request.resource_types.len() == 1 {
            selector.type_name = Some(request.resource_types[0].as_str().to_owned());
        }
        let views = self.manager.list(selector).await.map_err(map_manager_error)?;
        let page_size = request.page_size.max(1) as usize;
        let mut resources = Vec::new();
        for view in &views {
            if !request.resource_types.is_empty()
                && !request
                    .resource_types
                    .iter()
                    .any(|resource_type| resource_type.as_str() == view.key.type_name)
            {
                continue;
            }
            if !request.resource_names.is_empty()
                && !request.resource_names.iter().any(|name| name.as_str() == view.key.name)
            {
                continue;
            }
            let mut stored = stored_from_view(view)?;
            if !filters_match(&request.filters, &stored, &view.spec) {
                continue;
            }
            project_resource(&mut stored, request.projection)?;
            resources.push(stored);
            if resources.len() == page_size {
                break;
            }
        }
        let truncated = resources.len() == page_size;
        Ok(StoreListResult {
            resources,
            // The LIST snapshot anchors WATCH resume (F4/R23): the wire
            // revision of the hub's snapshot at listing time.
            snapshot_revision: ZoneRevision::new(wire_revision(self.hub.snapshot_revision())),
            next_cursor: None,
            truncated,
        })
    }

    async fn watch(&self, request: StoreWatchRequest) -> Result<StoreWatchReceipt, StoreError> {
        let selector = watch_selector(&request)?;
        let live = self.hub.snapshot_revision();
        let after = if request.after_revision.get() == 0 {
            // The beginning of the epoch: replay everything still retained.
            Some(RuntimeRevision::new(live.epoch, 0))
        } else {
            Some(decode_wire_revision(request.after_revision.get(), live.epoch).ok_or_else(|| {
                error(
                    StoreErrorKind::RevisionExpired,
                    Some(wire_revision(live)),
                    RetryClass::AfterDelay,
                    "revision-expired",
                )
            })?)
        };
        match self.manager.watch(selector, after).await.map_err(map_manager_error)? {
            RuntimeWatchRegistration::Live { snapshot, replay, stream } => {
                let stream_name = self.streams.insert(replay, stream);
                Ok(StoreWatchReceipt {
                    stream_name,
                    snapshot_revision: ZoneRevision::new(wire_revision(snapshot)),
                })
            }
            RuntimeWatchRegistration::Expired(expired) => Err(error(
                StoreErrorKind::RevisionExpired,
                Some(wire_revision(expired.snapshot)),
                RetryClass::AfterDelay,
                "revision-expired",
            )),
        }
    }

    async fn resolve_ref(
        &self,
        request: StoreResolveRequest,
    ) -> Result<StoreResolvedIdentity, StoreError> {
        let key = Self::runtime_key(&request.zone, &request.target);
        let Some(view) = self.manager.get(key).await.map_err(map_manager_error)? else {
            return Err(not_found());
        };
        if let Some(expected) = &request.expected_uid
            && expected != &row_uid(&view.uid)
        {
            return Err(conflict(view.generation, "resource-uid-mismatch"));
        }
        let generation = ResourceGeneration::new(view.generation.max(1))
            .expect("manager generations are nonzero");
        Ok(StoreResolvedIdentity {
            zone: request.zone,
            resource_ref: request.target,
            uid: row_uid(&view.uid),
            generation,
            revision: ZoneRevision::new(view.generation.max(1)),
        })
    }

    async fn inspect_schema(
        &self,
        _request: StoreInspectSchemaRequest,
    ) -> Result<StoredSchema, StoreError> {
        // The schema catalog stays attached to the redb plane in Phase A
        // (KTD4): the manager-backed plane carries no schema table.
        Err(error(
            StoreErrorKind::UnsupportedCapability,
            None,
            RetryClass::Never,
            "schema-catalog-not-wired",
        ))
    }

    async fn commit_verified(
        &self,
        sealed: SealedMutation,
    ) -> Result<StoreCommitResult, StoreError> {
        let body: MutationSealBody = self.acceptor.open(sealed)?.into_body();
        let subject = api_subject(&body.authorization);
        let mut seen = BTreeSet::new();
        for prepared in &body.mutations {
            let mutation = prepared.mutation();
            if !seen.insert((
                mutation.zone.to_canonical_string(),
                mutation.target.to_canonical_string(),
            )) {
                return Err(conflict(0, "same-batch-target-repeated"));
            }
        }
        let mut resources = Vec::new();
        let mut revision = 0u64;
        for prepared in &body.mutations {
            let mutation = prepared.mutation();
            if let Some(stored) = self.commit_mutation(&subject, mutation).await? {
                revision = revision.max(stored.revision.get());
                resources.push(stored);
            }
        }
        Ok(StoreCommitResult {
            resources,
            revision: ZoneRevision::new(revision),
        })
    }
}

#[cfg(test)]
mod tests;
