//! The declared facets the provider-owned `DeviceBinding` effects service
//! reaches the host's device mediation through.
//!
//! A `DeviceBinding` row is the *row side* of one admitted device
//! relationship: the Device source decided the claim, and this row realizes
//! it. The realization is a mediation over the host's device inventory, so
//! everything that touches the host crosses the provider boundary as a
//! declared facet rather than as a daemon handle: the claim-and-attach drive,
//! the two release steps of the teardown, and the two observations (is the
//! attachment realized now, does the consumer still hold it).
//!
//! The traits here are the crate's own declaration surface. The production
//! implementation is supplied by the composition root over the Device
//! family's trusted inventory and realizations; test doubles implement the
//! same seam, so the crate holds no host state and no device node path.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    DeviceClaimRequest, DeviceFunction, ResourceGeneration, ResourceRef, ResourceUid, ZoneRevision,
    device_binding::DeviceBindingSpec, execution_policy::BoundedToken,
};
use d2b_resource_runtime::identity::ResourceKey;

/// The uid / generation / revision fence one binding's realization is pinned
/// to.
///
/// Every effect this family drives is observed under this fence, and every
/// readiness report the row publishes carries it: the uid pins reassignment
/// (a row deleted and re-created under the same name is a different
/// relationship), the generation pins a spec change, and the revision bounds
/// how old the evidence may be. The manager has no separate Zone revision -
/// its wire revision *is* the row generation - so the fence names the row
/// revision the manager publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceBindingFence {
    uid: ResourceUid,
    generation: ResourceGeneration,
    revision: ZoneRevision,
}

impl DeviceBindingFence {
    /// Bind one row's durable identity as the fence its effects are observed
    /// under.
    pub const fn new(
        uid: ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
    ) -> Self {
        Self {
            uid,
            generation,
            revision,
        }
    }

    /// The row identity the evidence was observed under.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// The spec generation the evidence was observed under.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    /// The row revision the evidence was observed under.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Whether this fence still matches the row's own identity.
    ///
    /// Readiness is only current under a matching uid and generation, and the
    /// fence revision may only precede the stored revision: the status write
    /// carrying the report itself advances the store past the observed
    /// commit, so an exact-revision rule could never latch.
    pub fn matches(
        &self,
        uid: &ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
    ) -> bool {
        self.uid == *uid && self.generation == generation && self.revision <= revision
    }
}

/// The identity one realized `DeviceBinding` attachment is driven under.
///
/// The value is derived once per pass from the row's committed spec and the
/// row's own durable identity, so every effect the driver asks for names the
/// same relationship: the exact Device, the exact admitted consumer, the
/// stable consumer slot, the named function, and the requested claim. No
/// device node path, serial, bus id, numeric principal, or host permission bit
/// is reachable from it - the physical authority key is the trusted
/// inventory's, never this row's.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceAttachment {
    binding: ResourceKey,
    provider: Option<ResourceRef>,
    spec: DeviceBindingSpec,
    fence: DeviceBindingFence,
}

impl DeviceAttachment {
    /// Bind one committed row spec to the row that carries it and to the
    /// fence its effects are observed under.
    pub fn new(
        binding: ResourceKey,
        provider: Option<ResourceRef>,
        spec: DeviceBindingSpec,
        fence: DeviceBindingFence,
    ) -> Self {
        Self {
            binding,
            provider,
            spec,
            fence,
        }
    }

    /// The binding row this attachment realizes.
    pub const fn binding(&self) -> &ResourceKey {
        &self.binding
    }

    /// The Provider the row's envelope names, when it carries one.
    ///
    /// The Device source mints the row from the canonical request, so the
    /// envelope provider reference is optional: a row that names one tells
    /// the mediation which device component serves the function, and a row
    /// that names none is resolved from the trusted inventory the same way.
    pub const fn provider(&self) -> Option<&ResourceRef> {
        self.provider.as_ref()
    }

    /// The exact canonical spec this row commits.
    pub const fn spec(&self) -> &DeviceBindingSpec {
        &self.spec
    }

    /// The exact source Device this relationship claims from.
    pub const fn device(&self) -> &ResourceRef {
        self.spec.device_ref()
    }

    /// The exact admitted consumer the attachment reaches.
    pub const fn consumer(&self) -> &ResourceRef {
        self.spec.execution_ref()
    }

    /// The stable consumer slot this claim occupies.
    pub const fn slot(&self) -> &BoundedToken {
        self.spec.slot()
    }

    /// The named device function this claim covers.
    pub const fn function(&self) -> &DeviceFunction {
        self.spec.function()
    }

    /// The requested claim mode.
    pub const fn claim(&self) -> DeviceClaimRequest {
        *self.spec.claim()
    }

    /// The fence this attachment's evidence is observed under.
    pub const fn fence(&self) -> &DeviceBindingFence {
        &self.fence
    }
}

impl core::fmt::Debug for DeviceAttachment {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("DeviceAttachment")
            .field("binding", &self.binding)
            .field("function", &self.function())
            .field("claim", &self.claim())
            .field("fence", &self.fence)
            .finish_non_exhaustive()
    }
}

/// What one drive of the physical authority did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceEstablishOutcome {
    /// This pass realized the claim and its attachment; the row's work was
    /// done and the next pass has nothing to change.
    Realized,
    /// The exact realization this row declares was already in place, so
    /// nothing was claimed twice and no consumer saw a second attachment.
    AlreadyRealized,
}

impl DeviceEstablishOutcome {
    /// Whether this outcome changed the host's realization.
    pub const fn mutated(self) -> bool {
        matches!(self, Self::Realized)
    }
}

/// Why the trusted device mediation refused one attachment.
///
/// Every variant is the row side of one source-side refusal: the two
/// terminal variants are the halves of the Device source's own admission
/// refusal (a function the trusted inventory never resolved, and a capability
/// it no longer backs), and the two retryable ones are states that a later
/// pass can still converge on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceRefusal {
    /// The trusted inventory does not admit this named function for this
    /// Device: the attachment is unauthorized and no retry can make it
    /// admitted. Terminal.
    Unauthorized {
        /// The trusted adapter's own bounded reason.
        detail: String,
    },
    /// The capability this row was admitted against is no longer current:
    /// the claim is stale and re-deriving it would take a second claim.
    /// Terminal.
    Stale {
        /// The trusted adapter's own bounded reason.
        detail: String,
    },
    /// Another live relationship holds the capability exclusively. The
    /// source's release evidence frees it, so the pass retries.
    Conflicted {
        /// The trusted adapter's own bounded reason.
        detail: String,
    },
    /// The mediation adapter could not complete the drive. Operational; the
    /// pass retries.
    MediationFailed {
        /// The trusted adapter's own bounded reason.
        detail: String,
    },
}

impl DeviceRefusal {
    /// The stable provider reason this refusal reports.
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::Unauthorized { .. } => "device-function-unauthorized",
            Self::Stale { .. } => "device-attachment-stale",
            Self::Conflicted { .. } => "device-claim-conflict",
            Self::MediationFailed { .. } => "device-mediation-failed",
        }
    }

    /// Whether no retry of this row can converge.
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Unauthorized { .. } | Self::Stale { .. })
    }

    /// The adapter's own bounded detail, for the failure note.
    pub fn detail(&self) -> &str {
        match self {
            Self::Unauthorized { detail }
            | Self::Stale { detail }
            | Self::Conflicted { detail }
            | Self::MediationFailed { detail } => detail,
        }
    }
}

/// The daemon-supplied facet set the provider-owned `DeviceBinding` effects
/// are built from.
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type (R2).
#[derive(Clone)]
pub struct DeviceBindingEffectFacets {
    /// The mutating half: the claim-and-attach drive and the two release
    /// steps the teardown runs.
    pub mediation: Arc<dyn AttachmentMediation>,
    /// The observing half: whether the attachment is realized now, and
    /// whether the consumer still holds it.
    pub observation: Arc<dyn AttachmentObservation>,
}

/// The daemon-supplied device mediation: the half of the family that takes
/// and gives back the physical authority.
///
/// The claim is taken once per admitted relationship, by the source that
/// arbitrated it; a helper that realizes the parent's binding takes an
/// explicitly bound leg of that reservation rather than a second claim, so
/// this facet never arbitrates: it realizes what was admitted and refuses
/// anything else.
#[async_trait]
pub trait AttachmentMediation: Send + Sync + 'static {
    /// Realize the claim and the attachment this row declares, or report the
    /// exact realization is already in place.
    ///
    /// The call is idempotent under retry and under restart: an attachment
    /// that is already realized for this exact relationship answers
    /// [`DeviceEstablishOutcome::AlreadyRealized`] and takes nothing twice.
    ///
    /// # Errors
    ///
    /// Returns the [`DeviceRefusal`] the trusted adapter refused with: an
    /// unauthorized or stale capability (terminal), a claim conflict, or an
    /// operational mediation failure.
    async fn establish(
        &self,
        attachment: &DeviceAttachment,
    ) -> Result<DeviceEstablishOutcome, DeviceRefusal>;

    /// Remove the realized attachment the consumer was holding.
    ///
    /// This is the attachment-first half of the teardown: the attachment
    /// goes before the claim on the physical authority is given back, so a
    /// second consumer can never receive the capability while a stale
    /// attachment still exists.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon adapter could not remove the
    /// attachment. An attachment that was never realized, or is already gone,
    /// answers `Ok(())` - the release is idempotent under retry.
    async fn release_attachment(&self, attachment: &DeviceAttachment) -> Result<(), String>;

    /// Release the consumer's device slot: hand the claim back to the source
    /// that arbitrated it, so the capability becomes assignable again.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon adapter could not release the slot. A
    /// slot that is already free answers `Ok(())` - the release is
    /// idempotent under retry.
    async fn release_slot(&self, attachment: &DeviceAttachment) -> Result<(), String>;
}

/// The daemon-supplied observation over one realized attachment.
#[async_trait]
pub trait AttachmentObservation: Send + Sync + 'static {
    /// Whether the realized attachment reaches its consumer now.
    ///
    /// This is the serving evidence the row's fenced readiness projection
    /// reports: a binding whose attachment the mediation cannot observe is
    /// not ready, and reports itself so rather than claiming a capability it
    /// could not prove.
    async fn attachment_ready(&self, attachment: &DeviceAttachment) -> bool;

    /// Whether the consumer still holds the realized attachment.
    ///
    /// This is the drain gate the teardown reads before anything is
    /// released. The evidence is the target layer's, never a second channel:
    /// only a realization the consumer's own runtime reports for this exact
    /// attachment answers `true`. A consumer that cannot be reached, and an
    /// attachment that was never realized, both answer `false`.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon adapter could not complete the
    /// observation; the observation itself fails closed with `Ok(false)`.
    async fn attachment_held(&self, attachment: &DeviceAttachment) -> Result<bool, String>;
}