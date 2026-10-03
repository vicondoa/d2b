//! The declared facets the provider-owned Volume effects service reaches
//! daemon state through (U7).
//!
//! The Volume family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]). The daemon state that
//! implementation holds - the trusted root resolver that anchors one
//! Volume's layout root through the daemon's bundle and state roots, and
//! the durable layout probe recover reads - crosses the provider boundary
//! as declared facets rather than as a daemon handle: every facet here is a
//! type the provider crate declares, an implementation of it is supplied by
//! the daemon host through the composition root (never derived from caller
//! input), and the family crate holds no daemon state type.
//!
//! One daemon-supplied runtime value backs both surfaces:
//!
//! - [`VolumeRuntime`] is the orchestration facade the driver effects
//!   delegate to: the daemon implements the reconcile and cleanup machinery
//!   over the anchored adapters ([`d2b_provider_volume_local::adapter`])
//!   and its own trusted root resolver, and the durable layout probe
//!   (`has_layout`) the driver's recover reads.
//!
//! - [`VolumeRuntime::admit_bindings`] is the second seam: the canonical
//!   binding admission (U14, KTD2/KTD3) needs authorization evidence and a
//!   freshness fence that only the daemon's authority path holds, so the
//!   runtime supplies the admitted set and the driver commits it. The
//!   default refuses rather than minting either fact.
//!
//! - the anchored-fd filesystem implementation itself lives in
//!   `d2b-provider-volume-local` beside the effect ports it implements, so
//!   the daemon holds no volume-local mutation code.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ResourceGeneration, ResourceRef, ResourceUid, ZoneId, volume::VolumeSpec,
    volume_binding::VolumeBindingSpec,
};
use d2b_provider_volume_local::AdmittedVolumeBinding;
use d2b_provider_volume_virtiofs::{
    BindingPhase, MountObservation, StoredBinding, VirtiofsBindingError, VirtiofsServingDispatch,
    VirtiofsServingError,
};
use d2b_resource_runtime::spec_store::StoredDesiredResource;

/// The daemon-supplied facet set the provider-owned Volume effects are
/// built from (U7).
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type (R2).

#[derive(Clone)]
pub struct VolumeEffectFacets {
    /// The daemon's Volume runtime: the orchestration the driver effects
    /// delegate to, over the daemon's own trusted root resolver and durable
    /// layout state.
    pub runtime: Arc<dyn VolumeRuntime>,
}

/// Why one committed relationship is not delivered.
///
/// The two halves are distinct facts and are never collapsed: the serving
/// pass's own closed refusal is this family's vocabulary, while a dispatch
/// class is the privileged leg's and says the request never reached the
/// host at all. Neither carries a socket path, a shared directory, argv,
/// or a numerical identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingDeliveryReason {
    /// The serving pass refused this row itself.
    Serving(VirtiofsBindingError),
    /// The privileged leg refused the request, or never answered it.
    Dispatch(VirtiofsServingError),
}

impl BindingDeliveryReason {
    /// The closed, path-free code this refusal reports under.
    pub fn code(&self) -> &str {
        match self {
            Self::Serving(reason) => reason.code(),
            Self::Dispatch(error) => error.code(),
        }
    }
}

impl core::fmt::Display for BindingDeliveryReason {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for BindingDeliveryReason {}

/// One committed relationship's virtiofs delivery verdict (U15).
///
/// Delivery is a real observation: the consumer's own mount reaching it.
/// `phase` reaches `Ready` only when the privileged leg reports the mount
/// present, and a consumer that reports the source serving while its own
/// mount is absent is `Degraded`, never delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingDelivery {
    name: String,
    phase: BindingPhase,
    source_prepared: bool,
    consumer_mount: MountObservation,
    reason: Option<BindingDeliveryReason>,
}

impl BindingDelivery {
    /// Bind one verdict to the committed row it describes.
    pub fn new(
        name: impl Into<String>,
        phase: BindingPhase,
        source_prepared: bool,
        consumer_mount: MountObservation,
        reason: Option<BindingDeliveryReason>,
    ) -> Self {
        Self {
            name: name.into(),
            phase,
            source_prepared,
            consumer_mount,
            reason,
        }
    }

    /// The committed row this verdict is about.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The phase the serving pass computed.
    pub const fn phase(&self) -> BindingPhase {
        self.phase
    }

    /// Whether the source is prepared: the consumer's pre-start condition.
    pub const fn source_prepared(&self) -> bool {
        self.source_prepared
    }

    /// What the consumer currently observes at its mount point.
    pub const fn consumer_mount(&self) -> MountObservation {
        self.consumer_mount
    }

    /// Why the relationship is not delivered.
    pub const fn reason(&self) -> Option<&BindingDeliveryReason> {
        self.reason.as_ref()
    }

    /// Whether the consumer's mount is observed present.
    pub fn is_delivered(&self) -> bool {
        self.phase == BindingPhase::Ready
    }
}

/// One committed canonical `VolumeBinding` row this pass wrote, read back
/// as the serving identity the virtiofs family delivers (U15).
///
/// The row is the manager's own committed handle: its durable uid, its row
/// generation, and the exact bytes it was committed under. Nothing here is
/// re-derived from the attachment list, so a delivery can only ever be
/// about a row that exists in the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedBinding {
    name: String,
    stored: StoredBinding,
}

impl CommittedBinding {
    /// Bind one committed row's name and serving identity.
    pub const fn new(name: String, stored: StoredBinding) -> Self {
        Self { name, stored }
    }

    /// Read one committed manager row as a serving identity.
    ///
    /// # Errors
    ///
    /// Refuses committed bytes that are not a strictly neutral
    /// `VolumeBinding` spec (KTD1), and an identity the readiness fence
    /// cannot be built from.
    pub fn from_row(row: &StoredDesiredResource) -> Result<Self, VirtiofsBindingError> {
        let invalid = VirtiofsBindingError::InvalidBinding;
        let mut base: serde_json::Map<String, serde_json::Value> =
            serde_json::from_slice::<serde_json::Value>(&row.spec)
                .map_err(|_| invalid)?
                .as_object()
                .cloned()
                .ok_or(invalid)?;
        // The committed bytes carry the serving Provider reference beside
        // the neutral spec; the standard catalog admits no provider
        // extension path for the type, so only the reference itself is
        // dropped before the strict spec decode.
        base.remove("providerRef");
        let spec: VolumeBindingSpec =
            serde_json::from_value(serde_json::Value::Object(base)).map_err(|_| invalid)?;
        let uid = ResourceUid::from_bytes(&row.uid).map_err(|_| invalid)?;
        let generation = ResourceGeneration::new(row.generation).map_err(|_| invalid)?;
        Ok(Self {
            name: row.key.name.clone(),
            stored: StoredBinding::from_committed_row(spec, uid, generation),
        })
    }

    /// The committed row's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The serving identity the committed row stands for.
    pub const fn stored(&self) -> &StoredBinding {
        &self.stored
    }
}

/// The privileged virtiofs serving delivery one Zone composes (U15).
///
/// Everything here is a host fact the provider crate cannot derive for
/// itself: the privileged dispatch only the daemon can reach, the
/// broker-owned directory the private serving socket is bound under, and
/// the consumer Guest's vcpu count the worker plan's thread pool is tuned
/// from. The composition root supplies the value; the family crate holds no
/// daemon state type.
#[derive(Clone)]
pub struct VolumeServingComposition {
    /// The privileged virtiofs serving dispatch.
    pub dispatch: Arc<dyn VirtiofsServingDispatch>,
    /// The broker-owned runtime root the private serving socket is derived
    /// under.
    pub runtime_root: PathBuf,
    /// The consumer Guest's vcpu count, supplied by the daemon as the Zone's
    /// own authority fact rather than read from a Guest row.
    pub vcpu_count: u32,
}

/// One Volume as the canonical binding admission reads it (U14, KTD2/KTD3).
///
/// This is the whole of what the source owns about itself: the Zone the
/// relationships belong to, the exact source reference and store identity
/// the KTD3 key is derived from, and the declared spec the source's own
/// policy - declared views, granted rights, one writer - is decided
/// against.
#[derive(Clone)]
pub struct VolumeBindingAdmission<'a> {
    zone: ZoneId,
    volume_ref: ResourceRef,
    volume_uid: ResourceUid,
    spec: &'a VolumeSpec,
}

impl<'a> VolumeBindingAdmission<'a> {
    /// Bind one Volume's Zone, identity, and declared spec.
    pub const fn new(
        zone: ZoneId,
        volume_ref: ResourceRef,
        volume_uid: ResourceUid,
        spec: &'a VolumeSpec,
    ) -> Self {
        Self { zone, volume_ref, volume_uid, spec }
    }

    /// The Zone the relationships belong to.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// The exact source reference.
    pub const fn volume_ref(&self) -> &ResourceRef {
        &self.volume_ref
    }

    /// The source's store-assigned identity.
    pub const fn volume_uid(&self) -> &ResourceUid {
        &self.volume_uid
    }

    /// The declared spec the source's own admission policy reads.
    pub const fn spec(&self) -> &'a VolumeSpec {
        self.spec
    }
}

impl core::fmt::Debug for VolumeBindingAdmission<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("VolumeBindingAdmission")
            .field("zone", &self.zone)
            .field("volume_ref", &self.volume_ref)
            .finish_non_exhaustive()
    }
}

/// One admission fact the canonical binding path needs and the Volume
/// driver's effect seam does not carry.
///
/// Both facts come from the daemon's authority path, not from the provider:
/// a driver holds the row it reconciles, never the accepted graph or the
/// broker's observed state, so neither is derivable here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingAdmissionEvidence {
    /// The grant the Role/RoleBinding evaluation produced for the
    /// relationship's own row, read from the prior accepted graph. A
    /// well-formed request is not authorization, and a candidate carrying
    /// its own grant authorizes nothing.
    Authorization,
    /// The store incarnation plus the dependency revisions and digests the
    /// admission is fenced against. A `FreshnessTuple` cannot be assembled
    /// without them: the driver observes its own row, not the observed
    /// revisions of the source and the consumer.
    FreshnessFence,
}

impl BindingAdmissionEvidence {
    /// The stable label the refusal renders.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authorization => "the role authorization the accepted graph carries",
            Self::FreshnessFence => {
                "the store incarnation and dependency revisions the admission is fenced against"
            }
        }
    }
}

impl core::fmt::Display for BindingAdmissionEvidence {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Why a pass admitted no canonical relationship (U14).
///
/// The named facts are absent, which is not a pending approval: the source
/// may not commit a row it cannot fence, so the pass commits nothing and
/// retires nothing. An absent grant is never evidence that a relationship
/// ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingEvidenceAbsent {
    missing: Vec<BindingAdmissionEvidence>,
}

impl BindingEvidenceAbsent {
    /// Name the facts the seam did not carry.
    pub fn new(missing: Vec<BindingAdmissionEvidence>) -> Self {
        Self { missing }
    }

    /// Every fact this seam was asked for and did not carry.
    pub fn missing(&self) -> &[BindingAdmissionEvidence] {
        &self.missing
    }
}

impl core::fmt::Display for BindingEvidenceAbsent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("no binding admission evidence")?;
        for evidence in &self.missing {
            write!(formatter, "; missing {evidence}")?;
        }
        Ok(())
    }
}

impl std::error::Error for BindingEvidenceAbsent {}

/// The daemon-hosted Volume runtime one zone's effects run over (U7).
///
/// The daemon implements this trait in its composition root (the same
/// controller construction the retired daemon adapter served), supplying
/// the trusted root resolver and the durable layout probe. The family
/// crate's effects service delegates the driver seam to it; the anchored
/// filesystem mutations themselves run through
/// [`d2b_provider_volume_local::adapter::AnchoredVolumeEffectAdapter`],
/// which the runtime constructs over its resolver.
#[async_trait]
pub trait VolumeRuntime: Send + Sync + 'static {
    /// Run the preserved volume-local layout reconcile and report whether
    /// the layout phase reached `Ready`.
    async fn reconcile_volume(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
        provider: Option<&serde_json::Value>,
        owner_ref: Option<&ResourceRef>,
    ) -> Result<bool, String>;

    /// Remove the Volume's own layout state (drain finalizer preserved);
    /// idempotent under retry (R10).
    async fn cleanup_volume(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
    ) -> Result<(), String>;

    /// Discover existing volume-local layout state for this exact uid
    /// (recover probe).
    fn has_layout(&self, volume_uid: &ResourceUid) -> bool;

    /// Admit the canonical `VolumeBindingRequest` relationships declared
    /// against this Volume and report exactly the relationships the source
    /// may commit a row for (U14, KTD2/KTD3).
    ///
    /// The admitted set is the output of the one source-side admission path
    /// ([`d2b_provider_volume_local::VolumeLocalController::admit_bindings`]),
    /// so the source's own policy - the declared view, the rights it grants,
    /// the single writer, the realization's declared support - is decided
    /// once, in the crate that owns it, and the driver commits exactly what
    /// that path admitted.
    ///
    /// The default refuses. A runtime that was not given the authority path
    /// cannot derive [`BindingAdmissionEvidence::Authorization`] or
    /// [`BindingAdmissionEvidence::FreshnessFence`], and it must not
    /// substitute a grant of its own: a row committed without them is a
    /// relationship nothing fenced. Refusing keeps the producing half
    /// closed rather than quietly un-fenced - the same shape
    /// [`d2b_resource_runtime::context::ManagerEndpoint::ensure_source_owned_binding`]
    /// takes for an endpoint that carries no source-controller authority.
    async fn admit_bindings(
        &self,
        _source: &VolumeBindingAdmission<'_>,
    ) -> Result<Vec<AdmittedVolumeBinding>, BindingEvidenceAbsent> {
        Err(BindingEvidenceAbsent::new(vec![
            BindingAdmissionEvidence::Authorization,
            BindingAdmissionEvidence::FreshnessFence,
        ]))
    }

    /// The privileged virtiofs serving delivery this Zone composes (U15).
    ///
    /// A committed `VolumeBinding` row is realized by the virtiofs
    /// family's own serving pass, and that pass crosses the privileged
    /// boundary: only the daemon holds the broker socket, the broker-owned
    /// runtime root the private serving socket is bound under, and the
    /// Zone's vcpu authority. The composition root supplies those three
    /// facts together; the family crate derives the plan, the socket
    /// identity, and the fence itself and reconciles what comes back.
    ///
    /// The default is `None`: the leg is daemon-only, so a runtime that
    /// was not given one composes no delivery, and every committed
    /// relationship is reported undelivered by name rather than being
    /// described as served by a leg nobody holds.
    fn virtiofs_serving(&self) -> Option<VolumeServingComposition> {
        None
    }
}