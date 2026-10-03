//! Deterministic `VolumeBinding` intents produced from Volume attachments.
//!
//! volume-local owns this translation.  The shared runner mints one owned
//! binding child per intent, while the local Provider itself never imports
//! or calls the virtiofs Provider crate.  Attachments stay validated input
//! only: nothing besides the binding relationship is described here.

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::volume::{
    AttachmentAccess, AttachmentTransport, VolumeSpec,
};
use d2b_contracts_resource::v3::volume_binding::VolumeBindingSpec;
use d2b_contracts_resource::v3::{
    AdmissionStage, BindingAdmission, BindingArbitration, BindingAuthorization, BindingContractError,
    BindingKey, BindingRealizationFacet, BindingRealizationSupport, BindingRefusal,
    BindingSourceDecision, BindingSpecFingerprint, FreshnessTuple, RefusalReason, RequestedRights,
    ResourceUid, SourceAdmission, VolumeBindingRequest, ZoneId, admit_binding_request,
    canonical_json_bytes,
};
use sha2::{Digest, Sha256};

use crate::error::VolumeLocalError;
use crate::views::{
    admit_attachments, admit_consumer_kind, admit_view_request, destination_collides,
    view_requested_rights, view_subdirectory,
};

/// One desired Volume-side-created `VolumeBinding` resource.
#[derive(Clone, PartialEq, Eq)]
pub struct BindingIntent {
    name: BoundedToken,
    owner_ref: ResourceRef,
    volume_ref: ResourceRef,
    execution_ref: ResourceRef,
    view: BoundedToken,
    access: AttachmentAccess,
    mount_path: String,
}

impl BindingIntent {
    /// Borrow the deterministic binding name.
    pub const fn name(&self) -> &BoundedToken {
        &self.name
    }

    /// Borrow the Volume owner reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the referenced Volume.
    pub const fn volume_ref(&self) -> &ResourceRef {
        &self.volume_ref
    }

    /// Borrow the execution target.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the selected named View.
    pub const fn view(&self) -> &BoundedToken {
        &self.view
    }

    /// Return the admitted access class.
    pub const fn access(&self) -> AttachmentAccess {
        self.access
    }

    /// Borrow the guest-side mount path.
    pub fn mount_path(&self) -> &str {
        &self.mount_path
    }
}
impl core::fmt::Debug for BindingIntent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BindingIntent")
            .field("access", &self.access)
            .finish_non_exhaustive()
    }
}

/// Translate every virtiofs attachment into one deterministic binding.
///
/// The binding name derives from the Volume, execution target, named view,
/// and guest mount path -- never from the attachment index -- so reordering
/// declared attachments never churns identities.
pub fn desired_binding_intents(
    volume_ref: &ResourceRef,
    spec: &VolumeSpec,
    supports_shared_write: bool,
) -> Result<Vec<BindingIntent>, VolumeLocalError> {
    let admitted = admit_attachments(spec, supports_shared_write)?;
    let mut intents = Vec::with_capacity(admitted.len());
    for attachment in spec.attachments().iter() {
        if attachment.transport() != AttachmentTransport::Virtiofs {
            continue;
        }
        let name = derive_binding_name(volume_ref, attachment)?;
        intents.push(BindingIntent {
            name,
            owner_ref: volume_ref.clone(),
            volume_ref: volume_ref.clone(),
            execution_ref: attachment.execution_ref().clone(),
            view: attachment.view().clone(),
            access: attachment.access(),
            mount_path: attachment.mount_path().to_owned(),
        });
    }
    debug_assert_eq!(intents.len(), admitted.len());
    Ok(intents)
}

fn derive_binding_name(
    volume_ref: &ResourceRef,
    attachment: &d2b_contracts_resource::v3::volume::VolumeAttachment,
) -> Result<BoundedToken, VolumeLocalError> {
    let mut hasher = Sha256::new();
    hasher.update(b"d2b/volume-local/binding/v1");
    hasher.update([0]);
    hasher.update(volume_ref.to_canonical_string().as_bytes());
    hasher.update([0]);
    hasher.update(attachment.execution_ref().to_canonical_string().as_bytes());
    hasher.update([0]);
    hasher.update(attachment.view().as_str().as_bytes());
    hasher.update([0]);
    hasher.update(attachment.mount_path().as_bytes());
    let digest = hasher.finalize();
    let mut suffix = String::with_capacity(24);
    for byte in digest[..12].iter().copied() {
        suffix.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        suffix.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    BoundedToken::parse(format!("vol-binding-{suffix}")).map_err(|_| VolumeLocalError::InvalidSpec)
}

// ---------------------------------------------------------------------------
// The canonical source-side admission path.
//
// Everything above translates the old attachment list into binding intents.
// What follows admits the canonical relationship instead: the consumer's own
// `VolumeBindingRequest` is the one declaration site, the Volume source is
// the one admitter, and the committed `VolumeBinding` row is the source-owned
// child carrying that declaration plus the source's own accepted decision.
// There is no second relationship list here to keep in step with the first.
//
// The two derivations commit disjoint row names - the attachment derivation
// mints `vol-binding-` rows and the admitted one `vol-admitted-` rows - so
// each pass's retirement diff owns exactly the rows it mints, durably and
// across a restart, rather than depending on a wire shape that both share.
//
// The source owns writer arbitration across every realization of a view: a
// Process mount and a Guest attachment asking to write are arbitrated
// against each other here, so neither can take a writer the other already
// holds.  The durable claim itself stays with the one reservation owner that
// already arbitrates it; this decides, and never keeps a second record.
// ---------------------------------------------------------------------------

/// One consumer's store identity paired with its canonical request.
#[derive(Clone, PartialEq, Eq)]
pub struct VolumeConsumerRequest {
    consumer_uid: ResourceUid,
    request: VolumeBindingRequest,
}

impl VolumeConsumerRequest {
    /// Pair one consumer identity with the request it authored.
    pub const fn new(consumer_uid: ResourceUid, request: VolumeBindingRequest) -> Self {
        Self {
            consumer_uid,
            request,
        }
    }

    /// Borrow the consumer's store-assigned identity.
    pub const fn consumer_uid(&self) -> &ResourceUid {
        &self.consumer_uid
    }

    /// Borrow the canonical request this consumer authored.
    pub const fn request(&self) -> &VolumeBindingRequest {
        &self.request
    }
}

impl core::fmt::Debug for VolumeConsumerRequest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("VolumeConsumerRequest")
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}

/// The evidence every admission for one source is evaluated against.
///
/// These are the three inputs that are not this source's own decision: the
/// selected realization's declared support, the authorization evidence, and
/// the dependency revisions the admission is fenced against.  Omitting any
/// of them is not an unchecked admission - the shared evaluator refuses.
#[derive(Clone, Copy)]
pub struct VolumeAdmissionGrant<'a> {
    support: &'a BindingRealizationSupport,
    authorization: &'a BindingAuthorization,
    dependencies: &'a [FreshnessTuple],
}

impl<'a> VolumeAdmissionGrant<'a> {
    /// Carry the selected realization's support, the grant, and the fence.
    pub const fn new(
        support: &'a BindingRealizationSupport,
        authorization: &'a BindingAuthorization,
        dependencies: &'a [FreshnessTuple],
    ) -> Self {
        Self {
            support,
            authorization,
            dependencies,
        }
    }
}

impl core::fmt::Debug for VolumeAdmissionGrant<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.debug_struct("VolumeAdmissionGrant").finish_non_exhaustive()
    }
}

/// The source's own facts one admission is evaluated against.
#[derive(Clone, Copy)]
pub struct VolumeAdmissionSource<'a> {
    zone: &'a ZoneId,
    volume_ref: &'a ResourceRef,
    volume_uid: &'a ResourceUid,
    spec: &'a VolumeSpec,
    supports_shared_write: bool,
    grant: &'a VolumeAdmissionGrant<'a>,
}

impl<'a> VolumeAdmissionSource<'a> {
    /// Bind one Volume's identity, declared spec, and profile to the grant.
    pub const fn new(
        zone: &'a ZoneId,
        volume_ref: &'a ResourceRef,
        volume_uid: &'a ResourceUid,
        spec: &'a VolumeSpec,
        supports_shared_write: bool,
        grant: &'a VolumeAdmissionGrant<'a>,
    ) -> Self {
        Self {
            zone,
            volume_ref,
            volume_uid,
            spec,
            supports_shared_write,
            grant,
        }
    }

    /// Borrow the Zone the relationships belong to.
    pub const fn zone(&self) -> &'a ZoneId {
        self.zone
    }

    /// Borrow the exact source reference.
    pub const fn volume_ref(&self) -> &'a ResourceRef {
        self.volume_ref
    }

    /// Borrow the source's store-assigned identity.
    pub const fn volume_uid(&self) -> &'a ResourceUid {
        self.volume_uid
    }

    /// Borrow the source's declared spec.
    pub const fn spec(&self) -> &'a VolumeSpec {
        self.spec
    }

    /// Whether this source admits shared-writer relationships at all.
    pub const fn supports_shared_write(&self) -> bool {
        self.supports_shared_write
    }
}

impl core::fmt::Debug for VolumeAdmissionSource<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("VolumeAdmissionSource")
            .field("volume_ref", &self.volume_ref)
            .field("supports_shared_write", &self.supports_shared_write)
            .finish_non_exhaustive()
    }
}

/// One admitted Volume relationship, ready to become one owned row.
///
/// It carries the exact relationship key, the request that was admitted
/// unchanged, the admission the shared evaluator minted, the realization
/// facets that admission proved the selected backend can enforce, and the
/// declared view subdirectory.  The resolved host path, the serving socket
/// path, and every numeric identity stay out of it: those are resolved
/// privately from the admitted source at realization time.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmittedVolumeBinding {
    key: BindingKey,
    request: VolumeBindingRequest,
    admission: BindingAdmission,
    realized_facets: Vec<BindingRealizationFacet>,
    view_subdirectory: String,
}

impl AdmittedVolumeBinding {
    /// Borrow the KTD3 identity of this relationship.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// Borrow the canonical request, exactly as the consumer authored it.
    pub const fn request(&self) -> &VolumeBindingRequest {
        &self.request
    }

    /// Borrow the admission this relationship was granted.
    pub const fn admission(&self) -> &BindingAdmission {
        &self.admission
    }

    /// The realization facets this admission proved the selected backend can
    /// enforce.
    ///
    /// This is the source provider's own accepted decision, committed on the
    /// row: a boundary rebuilding the accepted graph from committed rows
    /// alone must be able to recover what the source admitted.
    pub fn realized_facets(&self) -> &[BindingRealizationFacet] {
        &self.realized_facets
    }

    /// The exact subdirectory the admitted view presents inside the Volume.
    ///
    /// This is what the relationship resolves to on the source side, so it
    /// is the view's declared path and never the Volume root standing in for
    /// a subtree the view did not declare.
    pub fn view_subdirectory(&self) -> &str {
        &self.view_subdirectory
    }

    /// Whether this relationship writes, and so arbitrates the writer slot.
    pub fn holds_writer(&self) -> bool {
        self.admission.rights() == RequestedRights::Mutate
    }

    /// The digest of the exact desired bytes this relationship commits.
    pub fn fingerprint(&self) -> BindingSpecFingerprint {
        self.request.fingerprint()
    }
}

impl core::fmt::Debug for AdmittedVolumeBinding {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AdmittedVolumeBinding")
            .field("request", &self.request)
            .field("rights", &self.admission.rights())
            .finish_non_exhaustive()
    }
}

/// One source-owned `VolumeBinding` row the source mints for one
/// admitted relationship.
///
/// The row is the committed form of the relationship: the consumer's
/// declaration plus the source provider's accepted decision, in the one
/// encoding the manager's relation index, the authority rebuild, and the
/// registered serving driver all read.
#[derive(Clone, PartialEq, Eq)]
pub struct BindingRow {
    name: BoundedToken,
    spec: Vec<u8>,
}

impl BindingRow {
    /// Borrow the deterministic row name.
    pub const fn name(&self) -> &BoundedToken {
        &self.name
    }

    /// Borrow the canonical desired bytes committed as the row's spec.
    pub fn spec(&self) -> &[u8] {
        &self.spec
    }
}

impl core::fmt::Debug for BindingRow {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BindingRow")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// Build one refusal of the shared contract.
const fn refuse(stage: AdmissionStage, reason: RefusalReason) -> BindingRefusal {
    BindingRefusal::new(stage, reason)
}

/// Admit one canonical consumer request against this source.
///
/// `held` is the set of relationships this source has already admitted, across
/// every consumer and every realization, so the writer and destination
/// decisions see their peers rather than only the request in hand.  Re-
/// admitting a relationship already in `held` is idempotent: the same key is
/// not a conflict with itself.
///
/// # Errors
///
/// Returns the first refusal of one source-side admission path: a consumer
/// the Volume binding kind does not deliver to, an undeclared view, an access
/// level the view's rights do not cover, shared write from a source that does
/// not declare it, a writer another live relationship already holds, a
/// destination the same consumer already claimed, a presentation the selected
/// realization cannot enforce, or a fence the shared evaluator rejects.
pub fn admit_consumer_request(
    source: &VolumeAdmissionSource<'_>,
    consumer_uid: &ResourceUid,
    request: &VolumeBindingRequest,
    held: &[AdmittedVolumeBinding],
) -> Result<AdmittedVolumeBinding, BindingRefusal> {
    if request.source_ref() != source.volume_ref {
        return Err(refuse(
            AdmissionStage::Admit,
            RefusalReason::SourcePolicyRefused,
        ));
    }
    // Per-kind consumer eligibility is the binding kind's own decision; it
    // is enforced here rather than left to each consumer's call site.
    admit_consumer_kind(request.consumer_ref())?;
    let view = admit_view_request(source.spec(), request)?;
    let requested = request.requested_rights();
    if requested == RequestedRights::Share && !source.supports_shared_write() {
        return Err(refuse(
            AdmissionStage::Admit,
            RefusalReason::SourcePolicyRefused,
        ));
    }
    let key = request
        .key(
            source.zone().clone(),
            source.volume_uid().clone(),
            consumer_uid.clone(),
        )
        .map_err(|_| {
            refuse(
                AdmissionStage::Admit,
                RefusalReason::SourcePolicyRefused,
            )
        })?;

    // Writer arbitration is the source's own single-writer decision, and it
    // spans consumers and realization backends: one live `Mutate`
    // relationship anywhere in this Volume is the one writer.  A reader
    // never competes for it, and re-admitting the relationship that already
    // holds it is not a conflict with itself.
    if requested == RequestedRights::Mutate
        && held
            .iter()
            .any(|admitted| admitted.holds_writer() && admitted.key() != &key)
    {
        return Err(refuse(
            AdmissionStage::Reserve,
            RefusalReason::ConflictingDeclaration,
        ));
    }
    if held
        .iter()
        .any(|admitted| destination_collides(admitted.request(), request))
    {
        return Err(refuse(
            AdmissionStage::Admit,
            RefusalReason::ConflictingDeclaration,
        ));
    }

    let mut admitted_rights = view_requested_rights(view);
    if !source.supports_shared_write() {
        admitted_rights.retain(|right| *right != RequestedRights::Share);
    }
    let arbitration = if requested == RequestedRights::Mutate {
        BindingArbitration::Exclusive
    } else {
        BindingArbitration::Shared
    };
    let decision = SourceAdmission::new(key.clone(), admitted_rights, arbitration).map_err(|_| {
        refuse(
            AdmissionStage::Admit,
            RefusalReason::SourcePolicyRefused,
        )
    })?;
    let admission = admit_binding_request(
        &key,
        requested,
        request.required_facets(),
        source.grant.authorization,
        &decision,
        source.grant.support,
        source.grant.dependencies,
    )?;
    // The source's accepted decision commits the facets the selected backend
    // was proven to realize, which is the admission's own evidence rather
    // than the request's wish: the request states what it needs, the
    // admission states what this source admitted it can enforce.
    let realized_facets = request
        .required_facets()
        .iter()
        .copied()
        .filter(|facet| source.grant.support.realizes(*facet))
        .collect();
    Ok(AdmittedVolumeBinding {
        key,
        request: request.clone(),
        admission,
        realized_facets,
        view_subdirectory: view_subdirectory(source.spec(), request.view())?.to_owned(),
    })
}

/// Admit a batch of canonical requests through the one source-side path.
///
/// Each admitted relationship joins the set the next one is judged against,
/// so writer arbitration and destination collisions are decided across the
/// whole batch in declaration order rather than per request.
///
/// # Errors
///
/// Returns the first refusal of the batch.  An admitted prefix is not
/// committed: the caller owns what it commits, and a partial batch is never
/// half-applied by this function.
pub fn admit_consumer_requests(
    source: &VolumeAdmissionSource<'_>,
    requests: &[VolumeConsumerRequest],
) -> Result<Vec<AdmittedVolumeBinding>, BindingRefusal> {
    let mut admitted: Vec<AdmittedVolumeBinding> = Vec::with_capacity(requests.len());
    for (index, input) in requests.iter().enumerate() {
        let next = admit_consumer_request(
            source,
            input.consumer_uid(),
            input.request(),
            &admitted[..index],
        )?;
        admitted.push(next);
    }
    Ok(admitted)
}

/// The name prefix the admitted-relationship derivation owns.
///
/// The attachment translation above mints `vol-binding-` names and this
/// derivation mints `vol-admitted-` names, so each pass's retirement diff
/// owns exactly the rows it mints.  The marker is durable and survives a
/// restart, which an in-memory set of this pass's rows would not.
pub const ADMITTED_BINDING_ROW_PREFIX: &str = "vol-admitted-";


/// Whether one row name belongs to the admitted-relationship derivation.
pub fn is_admitted_binding_row_name(name: &str) -> bool {
    name.starts_with(ADMITTED_BINDING_ROW_PREFIX)
}

/// The deterministic row name the source mints for one relationship.
///
/// The name derives from the relationship's committed identities - Zone,
/// source, consumer, kind, and the stable slot - never from a declaration
/// index or an attachment order, so reordering declarations never churns
/// identities and two relationships cannot collide by position.
pub fn binding_row_name(key: &BindingKey) -> Result<BoundedToken, BindingContractError> {
    let mut hasher = Sha256::new();
    hasher.update(b"d2b/volume/admitted-binding-row/v1");
    for part in [
        key.zone().to_canonical_string(),
        key.source_ref().to_canonical_string(),
        key.source_uid().to_canonical_string(),
        key.consumer_ref().to_canonical_string(),
        key.consumer_uid().to_canonical_string(),
        key.slot().as_str().to_owned(),
    ] {
        hasher.update([0]);
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    let mut suffix = String::with_capacity(24);
    for byte in digest[..12].iter().copied() {
        suffix.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        suffix.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    BoundedToken::parse(format!("{ADMITTED_BINDING_ROW_PREFIX}{suffix}"))
        .map_err(|_| BindingContractError::InvalidField)
}

/// The committed `VolumeBinding` row one admitted relationship mints.
///
/// The row's desired bytes are the committed form of the relationship: the
/// consumer's declaration plus the source provider's own accepted decision,
/// which is the one encoding the manager's relation index, the authority
/// rebuild, and the registered serving driver all read.  No host path, socket
/// path, or writable-path grant is added: the destination is consumer-side
/// and the source side is resolved privately from the admitted source and its
/// named view.  A block-device attachment commits its device slot rather
/// than a destination invented for it.
///
/// # Errors
///
/// Returns [`BindingContractError::InvalidField`] when the admitted decision
/// is not one the source contract admits, the derived row name is not a
/// bounded token, or the row does not render as canonical bytes.
pub fn canonical_binding_row(
    admitted: &AdmittedVolumeBinding,
) -> Result<BindingRow, BindingContractError> {
    let request = admitted.request();
    let decision = BindingSourceDecision::new(
        vec![admitted.admission().rights()],
        admitted.admission().arbitration(),
        admitted.realized_facets().to_vec(),
    )
    .map_err(|_| BindingContractError::InvalidField)?;
    let row = VolumeBindingSpec::new(
        request.source_ref().clone(),
        request.consumer_ref().clone(),
        request.view().as_str(),
        request.access(),
        request.presentation().clone(),
        admitted.key().slot().as_str(),
        decision,
    )
    .map_err(|_| BindingContractError::InvalidField)?;
    Ok(BindingRow {
        name: binding_row_name(admitted.key())?,
        spec: canonical_json_bytes(&row).map_err(|_| BindingContractError::InvalidField)?,
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fixtures;

    fn two_attachment_volume() -> VolumeSpec {
        serde_json::from_value(serde_json::json!({
            "source": {
                "executionRef": "Host/host-system",
                "settings": { "kind": "local-path", "sourcePolicyId": "state-root" },
            },
            "kind": "state",
            "layout": [],
            "views": {
                "controller": {
                    "path": "",
                    "rights": ["read", "write", "create", "delete", "traverse"],
                },
                "reader": { "path": "", "rights": ["read", "traverse"] },
            },
            "attachments": [
                {
                    "executionRef": "Guest/work-vm",
                    "transport": "virtiofs",
                    "view": "controller",
                    "access": "read-write",
                    "mountPath": "/state",
                },
                {
                    "executionRef": "Guest/other-vm",
                    "transport": "virtiofs",
                    "view": "reader",
                    "access": "read-only",
                    "mountPath": "/data",
                },
            ],
        }))
        .expect("conformant fixture Volume spec")
    }

    #[test]
    fn every_virtiofs_attachment_becomes_a_stable_owned_intent() {
        let volume = ResourceRef::parse("Volume/work-state").unwrap();
        let intents =
            desired_binding_intents(&volume, &fixtures::attached_state_volume(), false)
                .expect("intent");
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].owner_ref(), &volume);
        assert_eq!(intents[0].mount_path(), "/state");
        assert!(intents[0].name().as_str().starts_with("vol-binding-"));
        assert_eq!(
            intents[0].name(),
            desired_binding_intents(&volume, &fixtures::attached_state_volume(), false,).unwrap()[0]
                .name()
        );
    }

    #[test]
    fn reordering_attachments_never_churns_binding_names() {
        let volume = ResourceRef::parse("Volume/work-state").unwrap();
        let spec = two_attachment_volume();
        let forward = desired_binding_intents(&volume, &spec, false).expect("intents");
        let mut reordered = serde_json::to_value(&spec).unwrap();
        let attachments = reordered["attachments"].as_array().unwrap().clone();
        let swapped: Vec<_> = attachments.into_iter().rev().collect();
        reordered["attachments"] = serde_json::Value::Array(swapped);
        let backward_spec: VolumeSpec = serde_json::from_value(reordered).unwrap();
        let backward = desired_binding_intents(&volume, &backward_spec, false).expect("intents");

        assert_eq!(forward.len(), backward.len());
        for intent in &forward {
            assert!(backward.iter().any(|other| other.name() == intent.name()));
        }
    }

    #[test]
    fn the_named_view_is_part_of_the_binding_identity() {
        let volume = ResourceRef::parse("Volume/work-state").unwrap();
        let mut value = serde_json::to_value(fixtures::attached_state_volume()).unwrap();
        let spec: VolumeSpec = serde_json::from_value(value.clone()).unwrap();
        let controller = desired_binding_intents(&volume, &spec, false).unwrap();
        value["attachments"][0]["view"] = serde_json::json!("reader");
        value["attachments"][0]["access"] = serde_json::json!("read-only");
        let reader_spec: VolumeSpec = serde_json::from_value(value).unwrap();
        let reader = desired_binding_intents(&volume, &reader_spec, false).unwrap();

        assert_eq!(controller[0].execution_ref(), reader[0].execution_ref());
        assert_eq!(controller[0].mount_path(), reader[0].mount_path());
        assert_ne!(controller[0].name(), reader[0].name());
    }

    #[test]
    fn virtio_blk_attachments_do_not_create_filesystem_bindings() {
        let mut value = serde_json::to_value(fixtures::state_volume()).unwrap();
        value["source"]["settings"]["kind"] = serde_json::json!("block-image");
        value["source"]["settings"]["sourcePolicyId"] = serde_json::json!("disk-root");
        value["quota"] = serde_json::json!({ "maxBytes": 4096, "enforcement": "none" });
        value["attachments"] = serde_json::json!([{
            "executionRef": "Guest/work-vm",
            "transport": "virtio-blk",
            "view": "controller",
            "access": "read-only",
            "mountPath": "/disk"
        }]);
        let spec: VolumeSpec = serde_json::from_value(value).unwrap();
        assert!(
            desired_binding_intents(
                &ResourceRef::parse("Volume/work-state").unwrap(),
                &spec,
                false,
            )
            .unwrap()
            .is_empty()
        );
    }
}
