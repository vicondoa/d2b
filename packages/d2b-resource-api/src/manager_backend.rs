//! Manager-backed store backend: the Resource API rewired onto the per-Zone
//! `ResourceManager` (U8, KTD6/KTD8; R23-R25, R28).
//!
//! [`ManagerBackend`] implements [`ResourceStoreBackend`] over the
//! [`ResourceManagerClient`], so the [`crate::ResourceService`] keeps its
//! public operation shapes and wire envelopes while the backend and dispatch
//! change to the single-writer manager:
//!
//! - reads (`get`/`list`/`resolve_ref`) resolve manager runtime views, and
//!   LIST pages over the manager's deterministic `(zone, type, name)` row
//!   order with an opaque keyset cursor: `truncated` is true only when a
//!   remainder exists, and a cursor that cannot be honoured is refused with
//!   a typed error rather than ignored;
//! - the closed list filters keep their durable-plane semantics, including
//!   ownership (`owner.resourceUid` matches the row's resolved owner uid,
//!   `owner.resourceRef` the owner reference the row renders);
//! - mutations admit at the manager boundary under the caller's real
//!   authenticated subject ([`api_subject`]) and persist through
//!   `Ensure`/`Remove` with commit-before-return (F1/AE1);
//! - resource **generation** maps onto the existing Exact-revision wire
//!   semantics (KTD8): a stored resource's wire revision is its generation,
//!   so an `Exact(rev)` precondition rejects a stale generation with the
//!   existing resource-conflict wire error carrying the current generation;
//! - external WATCH is not served in this phase: the manager-plane handoff
//!   has no delivery pump in the composition (nothing takes the registered
//!   stream and writes it to the opened component stream), so [`watch`]
//!   refuses with `UnsupportedCapability` instead of returning a stream name
//!   no producer fills - a client that followed such a receipt would wait
//!   forever. LIST is the enumeration surface until the composition wires
//!   the pump;
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
use d2b_resource_runtime::{
    error::ResourceError,
    identity::ResourceKey as RuntimeResourceKey,
};
use d2b_contracts_resource::v3::operations::seal::MutationSealAcceptor;
use d2b_contracts_resource::v3::{
    AdmittedAuthorization, ExpectedRevision, MutationSealBody, ResourceMutationKind,
    SealedMutation, StoreCommitResult, StoreError, StoreErrorKind, StoreGetRequest,
    StoreInspectSchemaRequest, StoreListRequest, StoreListResult, StoreMutation, StoreProjection,
    StoreResolveRequest, StoreResolvedIdentity, StoreWatchReceipt, StoreWatchRequest,
    StoredResource, StoredSchema,
};

use crate::ResourceStoreBackend;

/// The runtime-revision to wire-revision mapping (U5 budget): the wire
/// carries `(epoch_seconds << 32) | sequence`, where `epoch_seconds =
/// epoch_nanos / 1_000_000_000` and the sequence occupies the low 32 bits.
/// Order-preserving within an epoch.
pub fn wire_revision(revision: RuntimeRevision) -> u64 {
    (revision.epoch / 1_000_000_000) << 32 | (revision.sequence & 0xffff_ffff)
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

/// The owner reference the row renders, when one is rendered: the authored
/// `metadata.ownerRef` wins (exactly as [`render_envelope`] resolves it), and
/// a row without one renders the resolved owner key. Evaluating the filter
/// here keeps it on the same value the row's readers see.
fn rendered_owner_ref(view: &ResourceView) -> Option<String> {
    let authored = serde_json::from_slice::<serde_json::Value>(&view.metadata)
        .ok()
        .and_then(|metadata| metadata.get("ownerRef").cloned())
        .filter(|value| !value.is_null())
        .and_then(|value| value.as_str().map(str::to_owned));
    authored.or_else(|| {
        view.owner_key
            .as_ref()
            .map(|owner| format!("{}/{}", owner.type_name, owner.name))
    })
}

/// Whether one manager view is in the request's matching set: the type/name
/// selectors first, then the closed filter set evaluated against the view's
/// own attributes (including the row's real ownership - an owner-scoped LIST
/// must match its children, not return an empty page).
fn list_view_matches(request: &StoreListRequest, view: &ResourceView) -> bool {
    if !request.resource_types.is_empty()
        && !request
            .resource_types
            .iter()
            .any(|resource_type| resource_type.as_str() == view.key.type_name)
    {
        return false;
    }
    if !request.resource_names.is_empty()
        && !request.resource_names.iter().any(|name| name.as_str() == view.key.name)
    {
        return false;
    }
    request.filters.iter().all(|filter| match filter.field.as_str() {
        "metadata.name" => filter.values.iter().any(|value| value == &view.key.name),
        "type" => filter.values.iter().any(|value| value == &view.key.type_name),
        "assignment.resourceUid" => {
            let uid = row_uid(&view.uid);
            filter.values.iter().any(|value| value == uid.as_str())
        }
        "owner.resourceUid" => owner_wire_uid(view)
            .as_ref()
            .is_some_and(|owner| filter.values.iter().any(|value| value == owner.as_str())),
        "owner.resourceRef" => {
            let owner = rendered_owner_ref(view);
            filter
                .values
                .iter()
                .any(|value| owner.as_deref() == Some(value.as_str()))
        }
        _ => false,
    })
}

/// The LIST order key: `(zone, type, name)`, the same order the durable plane
/// walked, so cursor positions are reproducible within a page sequence.
fn key_order(key: &RuntimeResourceKey) -> (&str, &str, &str) {
    (key.zone.as_str(), key.type_name.as_str(), key.name.as_str())
}

/// The selector binding of a LIST cursor: the request fields a page sequence
/// depends on. A cursor replayed under different selectors addresses a
/// different sequence, so it is refused instead of silently landing
/// elsewhere.
fn list_selector_digest(request: &StoreListRequest) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(request.zone.as_str().as_bytes());
    digest.update([request.projection as u8]);
    for resource_type in &request.resource_types {
        digest.update(resource_type.as_str().as_bytes());
        digest.update([0]);
    }
    for name in &request.resource_names {
        digest.update(name.as_str().as_bytes());
        digest.update([0]);
    }
    for filter in &request.filters {
        digest.update(filter.field.as_bytes());
        digest.update([0]);
        for value in &filter.values {
            digest.update(value.as_bytes());
            digest.update([0]);
        }
    }
    let hex = |bytes: &[u8]| {
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            out.push(char::from_digit((byte >> 4) as u32, 16).expect("nibble"));
            out.push(char::from_digit((byte & 0x0f) as u32, 16).expect("nibble"));
        }
        out
    };
    hex(&digest.finalize())
}

fn hex_decode(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    let nibble = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    };
    value
        .as_bytes()
        .chunks(2)
        .map(|pair| Some((nibble(pair[0])? << 4) | nibble(pair[1])?))
        .collect()
}

fn list_cursor_error(reason: &'static str) -> StoreError {
    error(StoreErrorKind::ResourceSchemaInvalid, None, RetryClass::Never, reason)
}

/// Encode the continuation cursor: `v1.<revision>.<selector>.<hex of the
/// last returned key>`. Opaque to clients; the next page resumes strictly
/// after the key in [`key_order`]. The snapshot revision is carried for
/// observability only: pages resume by key, so a concurrent change cannot
/// make a live row unreachable mid-sequence.
fn encode_list_cursor(
    revision: u64,
    request: &StoreListRequest,
    after: &RuntimeResourceKey,
) -> String {
    let mut key = Vec::new();
    for part in [after.zone.as_str(), after.type_name.as_str(), after.name.as_str()] {
        key.extend_from_slice(part.as_bytes());
        key.push(0);
    }
    let hex = key.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    format!("v1.{revision}.{}.{hex}", list_selector_digest(request))
}

/// Decode and validate a continuation cursor against the request it is
/// replayed with. Malformed cursors and cursors from a different selector
/// are refused with a typed error - never ignored, which would silently
/// restart the sequence.
fn decode_list_cursor(
    cursor: &str,
    request: &StoreListRequest,
) -> Result<RuntimeResourceKey, StoreError> {
    let mut parts = cursor.split('.');
    if parts.next() != Some("v1") {
        return Err(list_cursor_error("list-cursor-invalid"));
    }
    parts
        .next()
        .filter(|revision| !revision.is_empty() && revision.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| list_cursor_error("list-cursor-invalid"))?;
    let selector = parts
        .next()
        .filter(|digest| digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| list_cursor_error("list-cursor-invalid"))?;
    if selector != list_selector_digest(request) {
        return Err(list_cursor_error("list-cursor-selector-mismatch"));
    }
    let key = hex_decode(parts.next().ok_or_else(|| list_cursor_error("list-cursor-invalid"))?)
        .ok_or_else(|| list_cursor_error("list-cursor-invalid"))?;
    if parts.next().is_some() {
        return Err(list_cursor_error("list-cursor-invalid"));
    }
    // The encoded key is `zone\0type\0name\0`: drop the terminator before
    // splitting, so the three components are exactly the fields present.
    let key = key
        .strip_suffix(&[0u8])
        .ok_or_else(|| list_cursor_error("list-cursor-invalid"))?;
    let mut fields = key.split(|byte| *byte == 0).map(|part| {
        std::str::from_utf8(part)
            .map(str::to_owned)
            .map_err(|_| list_cursor_error("list-cursor-invalid"))
    });
    let zone = fields.next().transpose()?.ok_or_else(|| list_cursor_error("list-cursor-invalid"))?;
    let type_name =
        fields.next().transpose()?.ok_or_else(|| list_cursor_error("list-cursor-invalid"))?;
    let name = fields.next().transpose()?.ok_or_else(|| list_cursor_error("list-cursor-invalid"))?;
    if zone.is_empty() || type_name.is_empty() || name.is_empty() || fields.next().is_some() {
        return Err(list_cursor_error("list-cursor-invalid"));
    }
    Ok(RuntimeResourceKey::new(zone, type_name, name))
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
/// **This is the one wire-view producer for converted types** (issue #515):
/// envelope, metadata, spec, and status are rendered here and nowhere else,
/// so no reader - public API, internal scan, dependency probe - can serve a
/// row whose shape differs from what any other reader serves for the same
/// manager state. The universal status layer is
/// [`ResourceView::wire_status`]'s output, the row's identity and desired
/// state come from the view, and the rendered bytes are always a complete
/// envelope the strict readers (`ResourceEnvelope::from_json`) decode.
///
/// Public because it is also the canonical manager-row projection for the
/// daemon's reader bridges (G5, U12): they merge manager rows into the old
/// plane's store-shaped readers through exactly this rendering, so a bridged
/// row is identical to the row the manager-backed API serves.
pub fn manager_row_stored(view: &ResourceView) -> Result<StoredResource, StoreError> {
    stored_from_view(view)
}

/// The wire uid of the owner a manager view is bound to, when it is an owned
/// child.
///
/// The manager links ownership by the owner's stable uid (KTD2) and resolves
/// the owner *key* through its uid index ([`ResourceView::owner_key`]). Every
/// manager row's uid is the deterministic derivation of its own key
/// (`d2b_resource_runtime::manager::deterministic_uid`, which
/// [`ManagerBackend::commit_verified`] cross-checks on every API commit), so
/// the owner's wire uid follows from the resolved key without a second RPC -
/// and the row carries the same ownership a durable row would.
fn owner_wire_uid(view: &ResourceView) -> Option<ResourceUid> {
    view.owner_key.as_ref().map(|owner| row_uid(&manager_uid(owner)))
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
        &view.wire_status(),
    )?;
    let mut stored = stored_of(
        &view.key.zone,
        &view.key.type_name,
        &view.key.name,
        &view.uid,
        owner_wire_uid(view),
        view.generation,
        &canonical,
    );
    if view.deleting {
        stamp_deletion_request(&mut stored)?;
        // The stamp edits the rendered envelope after `stored_of` derived the
        // digest; refresh it so the strict readers' recomputed envelope digest
        // keeps matching the row.
        reseal_envelope(&mut stored)?;
    }
    Ok(stored)
}

/// The canonical manager state of a row immediately after its removal
/// commit: marked deleting at its own generation, its status the closed
/// deleting classification, and the status projection dropped (it was
/// published for a live row and is obsolete once the removal commits).
///
/// The resolved owner key is the one every read resolves, so the deleting
/// rendering carries the same `ownerRef` a GET of the same row carries.
///
/// Feeding the row through the one projection ([`manager_row_stored`]) is
/// what makes a delete confirmation identical to any concurrent read of the
/// same manager state - the confirmations do not assemble a second shape.
fn deleting_view(row: &StoredDesiredResource, owner_key: Option<&RuntimeResourceKey>) -> ResourceView {
    ResourceView {
        key: row.key.clone(),
        uid: row.uid,
        generation: row.generation,
        deleting: true,
        provenance: row.provenance,
        spec: row.spec.clone(),
        metadata: row.metadata.clone(),
        owner_key: owner_key.cloned(),
        status: Some(ResourceStatus::Deleting),
        status_generation: Some(row.generation),
        status_projection: None,
    }
}

/// The wire envelope of one manager row.
///
/// API-created rows persist the canonical envelope as authored (its `status`
/// member is never observed state on the manager plane, R11); Nix-materialized
/// rows persist the compiled spec with its authored metadata beside it. Reads
/// always answer with the envelope shape - the public surface stays uniform
/// across both provenances - so a spec-shaped row is rendered against its
/// metadata and the row's authoritative identity, the supplied canonical
/// status replaces whatever `status` the row persisted, and the rendered bytes
/// are always a complete envelope the strict readers
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
    status: &serde_json::Value,
) -> Result<Vec<u8>, StoreError> {
    let spec_value = CanonicalJsonValue::parse(spec).map_err(|_| envelope_invalid())?;
    if spec_value.as_object().is_some_and(|root| {
        root.contains_key("apiVersion") && root.contains_key("spec")
    }) {
        // Envelope-shaped rows are served from their persisted bytes - the
        // stored envelope is authoritative for identity and desired state -
        // but the `status` object is never persisted on the manager plane
        // (R11): the served status is the canonical projection of the row's
        // live state, exactly as it is for a spec-shaped row.
        let mut envelope: serde_json::Value =
            serde_json::from_slice(spec).map_err(|_| envelope_invalid())?;
        let Some(root) = envelope.as_object_mut() else {
            return Err(envelope_invalid());
        };
        root.insert("status".to_owned(), status.clone());
        let canonical = CanonicalJsonValue::parse(
            &serde_json::to_vec(&envelope).map_err(|_| envelope_invalid())?,
        )
        .map_err(|_| envelope_invalid())?;
        let envelope = ResourceEnvelope::from_json(&canonical.to_canonical_bytes())
            .map_err(|_| envelope_invalid())?;
        return envelope.canonical_bytes().map_err(|_| envelope_invalid());
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
    // Always a complete status: the canonical projection of the row's live
    // classification and its driver-published layer
    // ([`ResourceView::wire_status`], the one status producer) - `Pending`
    // at the row's own generation for a row that published nothing, the
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
        "status": status,
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
        Self { manager, hub, acceptor }
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
                // keeps the row and generation unchanged, so the response is
                // the same canonical wire view a read of that row serves -
                // never a second rendering of the desired bytes alone.
                if next == row.spec {
                    return Ok(Some(self.committed(&key).await?));
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
                // Resolve the owner the way every read does: the deleting
                // view carries the same owner reference a GET of the same
                // row resolves, so the confirmation cannot differ from the
                // canonical projection of the post-commit state.
                let owner_key = self.owner_key_for(&row).await?;
                self.manager
                    .remove(subject.clone(), key.clone())
                    .await
                    .map_err(map_manager_error)?;
                // The removal commit marked the row deleting at its own
                // generation; the confirmation is the canonical projection
                // of exactly that state (identity, desired bytes, and the
                // deletion the caller just requested), never a second shape
                // assembled for the response.
                Ok(Some(stored_from_view(&deleting_view(&row, owner_key.as_ref()))?))
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
        self.owner_key_for(row).await
    }

    /// The owning resource's manager key for one desired row, resolved
    /// through the manager's own uid index (the same lookup
    /// [`ResourceManagerState::view`](d2b_resource_runtime::manager::ResourceView)
    /// performs): `None` for a root, and for a row whose owner is not in
    /// this manager.
    async fn owner_key_for(
        &self,
        row: &StoredDesiredResource,
    ) -> Result<Option<RuntimeResourceKey>, StoreError> {
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
        let mut views = self.manager.list(selector).await.map_err(map_manager_error)?;
        // The manager's row map is unordered; paging needs one stable order.
        // `(zone, type, name)` is the durable plane's key order, so a cursor
        // is a position in the same sequence a durable list would walk.
        views.sort_by(|left, right| key_order(&left.key).cmp(&key_order(&right.key)));
        let after = match request.cursor.as_deref() {
            Some(cursor) => Some(decode_list_cursor(cursor, &request)?),
            None => None,
        };
        let matched: Vec<&ResourceView> =
            views.iter().filter(|view| list_view_matches(&request, view)).collect();
        let start = match &after {
            Some(after) => matched.partition_point(|view| key_order(&view.key) <= key_order(after)),
            None => 0,
        };
        let page_size = request.page_size.max(1) as usize;
        let end = matched.len().min(start.saturating_add(page_size));
        let page = &matched[start..end];
        let truncated = end < matched.len();
        let mut resources = Vec::with_capacity(page.len());
        for view in page {
            let mut stored = stored_from_view(view)?;
            project_resource(&mut stored, request.projection)?;
            resources.push(stored);
        }
        let next_cursor = truncated.then(|| {
            let last = page.last().expect("a non-empty truncated page has a last row");
            encode_list_cursor(wire_revision(self.hub.snapshot_revision()), &request, &last.key)
        });
        Ok(StoreListResult {
            resources,
            // The LIST snapshot anchors WATCH resume (F4/R23): the wire
            // revision of the hub's snapshot at listing time.
            snapshot_revision: ZoneRevision::new(wire_revision(self.hub.snapshot_revision())),
            next_cursor,
            truncated,
        })
    }

    async fn watch(&self, _request: StoreWatchRequest) -> Result<StoreWatchReceipt, StoreError> {
        // External WATCH is not served in this phase. The manager plane has
        // the producer (`ResourceManagerClient::watch` registers replay +
        // live delivery with the hub) but the composition has no consumer: no
        // delivery task takes the registered stream and writes it to the
        // component stream the bus opened, so a receipt would name a stream
        // nothing fills and a client would wait on it forever. Refuse with
        // the typed capability error instead of issuing that receipt; a
        // client relists. Wiring the pump (take each registered stream from
        // the manager backend and pump it into the opened component stream)
        // is the composition's change, not this backend's.
        Err(error(
            StoreErrorKind::UnsupportedCapability,
            None,
            RetryClass::Never,
            "watch-not-wired",
        ))
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
        // The schema catalog has no manager-plane table: the manager persists
        // desired rows only, and the per-type spec decoders are compile-time
        // contracts. Nothing on this plane can answer a schema read, so the
        // refusal is typed and terminal (Phase A scope, KTD4).
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
