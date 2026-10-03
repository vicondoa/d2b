//! The provider-facing typed effects seam the Process driver needs (U1).
//!
//! The family's own implementation ([`crate::effects_service`]) serves the
//! seam over the daemon-supplied declared facets
//! ([`crate::facets::ProcessEffectFacets`]); test doubles implement the same
//! seam (R4: the conversion is mechanical, the provider effects are
//! preserved). The classification types below are the closed results that
//! seam reports.

use std::time::Duration;

use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::process::{EphemeralProcessSpec, ProcessSpec};
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, ZoneId};
use d2b_process_conformance::{
    AdoptionCandidate, BindingPreparation, ProcessIdentityDigest, ProcessStatusReport,
    ProcessSubject, ResolvedProcessPlan,
};
use d2b_resource_runtime::context::ResourceContext;

use crate::identity::{ProcessFamilySpec, ProcessResourceIdentity};
use crate::worker_launch::DeviceWorkerLaunch;

/// The provider-facing effect surface the Process driver needs. The
/// family's implementation ([`crate::effects_service::ProcessEffectsService`])
/// runs over the daemon-supplied facets; test doubles implement the same
/// seam (R4: the conversion is mechanical, the provider effects are
/// preserved).
///
/// Object-erased on purpose: the driver holds the surface as
/// `Arc<dyn ProcessDriverEffects>` so one factory serves every Process row.
#[async_trait::async_trait]
pub trait ProcessDriverEffects: Send + Sync + 'static {
    /// Resolve the one plan this row launches against.
    ///
    /// Both Process lifetimes call this before any launch or adoption, and
    /// both receive the same answer: `Ok(None)` means the effect owner has not
    /// resolved a plan for this row and the pre-plan ticket path applies, which
    /// U34 removes with the rest of the ticket authority. `Ok(Some(plan))` is a
    /// plan the broker admitted from its own accepted graph, and the driver
    /// launches against it instead of the row's own posture.
    ///
    /// The default is `Ok(None)` rather than a refusal so an effect owner that
    /// has not been converted keeps serving the rows it already serves; an
    /// owner that resolves plans returns them, and an owner that cannot is
    /// visible in the result rather than hidden behind a fabricated plan.
    async fn prepare(
        &self,
        _identity: &ProcessResourceIdentity,
        _subject: &ProcessSubject,
    ) -> Result<Option<ResolvedProcessPlan>, String> {
        Ok(None)
    }

    /// Launch through the signed provider-ticket path.
    async fn launch(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        timeout: Duration,
    ) -> Result<ProcessIdentityDigest, String>;

    /// Launch one one-shot process through the preserved ephemeral ticket
    /// (old `launch_ephemeral_resource`; `start_deadline` is the timeout).
    async fn launch_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
        timeout: Duration,
    ) -> Result<ProcessIdentityDigest, String>;

    /// Probe-and-adopt over pidfd/proc evidence with the preserved
    /// Adopt/Stale/Quarantined classification.
    async fn adopt(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
    ) -> Result<ProviderAdoption, String>;

    /// Probe one already-started durable process (old `probe_record`): the
    /// Alive/Exited/Unknown liveness classification drives the steady-state
    /// observation of a process this actor adopted or launched, and the
    /// provider clears its exact local authority when the process is gone.
    async fn probe(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
    ) -> Result<ProviderLiveness, String>;

    /// Probe-and-adopt one one-shot process (old
    /// `adopt_ephemeral_resource`): `Absent` is also the observed exit of a
    /// process this driver launched, because the provider clears its local
    /// authority for the missing identity.
    async fn adopt_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderAdoption, String>;

    /// Probe one already-started one-shot identity (old
    /// `probe_ephemeral_resource`): the Alive/Exited/Unknown liveness
    /// classification drives the steady-state observation, and the provider
    /// clears its local authority when the exact process is gone.
    async fn probe_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderLiveness, String>;

    /// Preserved term-then-kill escalation with pidfd retry; `Ok(killed)`
    /// reports whether the kill stage ran.
    async fn stop(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String>;

    /// Derive the typed launch parameters of one declared Device-owned worker
    /// row (`U17` gap closure). The daemon host resolves the
    /// Device-family-specific inputs behind the runtime facet (it may name
    /// the device families), and this crate receives the already-resolved
    /// typed parameters; a row no Device worker template declares yields
    /// `Ok(None)`, and a declared template whose trusted inputs cannot be
    /// resolved yields the named refusal code.
    async fn device_worker_launch(
        &self,
        _ctx: &mut ResourceContext,
        _identity: &ProcessResourceIdentity,
        _spec: &ProcessFamilySpec,
    ) -> Result<Option<DeviceWorkerLaunch>, &'static str> {
        Ok(None)
    }

    /// Stop one exact one-shot identity (old `stop_ephemeral_resource`).
    async fn stop_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String>;

    /// Stop one exactly-identified stale candidate before a fresh launch.
    async fn stop_stale(
        &self,
        provider_ref: &ResourceRef,
        candidate: &AdoptionCandidate,
    ) -> Result<(), String>;

    /// Remove the provider's exact local authority after a terminal exit.
    async fn finalize(&self, identity: &ProcessResourceIdentity) -> Result<(), String>;

    /// Whether this zone retains a verified identity for the resource.
    fn has_active(
        &self,
        zone: &ZoneId,
        zone_uid: Option<&ResourceUid>,
        resource_ref: &ResourceRef,
    ) -> bool;
}

/// Result of a Provider-backed adoption attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAdoption {
    /// No process matching the trusted ticket is running.
    Absent,
    /// The exact process was adopted.
    Adopted(ProcessStatusReport),
    /// A static Provider controller was found without its exact bootstrap
    /// endpoint retained by this daemon.
    ControllerBootstrapMissing,
    /// A uniquely identified stale process is available for exact replacement.
    Stale {
        /// Opaque effect-owner evidence for the exact stale process.
        candidate: AdoptionCandidate,
    },
    /// A candidate was present but identity was ambiguous and quarantined.
    Quarantined(ProcessStatusReport),
}

/// Provider-backed liveness result used by the daemon readiness loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderLiveness {
    /// The exact process is still present.
    Alive,
    /// The exact process is absent.
    Exited,
    /// Identity could not be established safely.
    Unknown,
}

// ---------------------------------------------------------------------------
// Launch binding gate (U4, KTD6, R18)
// ---------------------------------------------------------------------------

/// One expected canonical `EndpointBinding` row a Process launch requires.
///
/// The expectation is derived from the CURRENT publication intent of the
/// `Endpoint` rows that name this exact Process identity - not from a
/// consumer-local slot table, and not from the rows that happen to exist
/// (R18). Everything a reader needs to prove the delivery it observed belongs
/// to THIS launch is a field here: the relationship row's identity, the
/// endpoint and row generations it was derived at, the consumer it is for,
/// the canonical slot, the authorization digest and dependency revision it was
/// derived under, and the opaque realization-incarnation token the endpoint
/// published.
///
/// Nothing here is host-shaped. The slot is the derived bounded token and the
/// incarnation is a digest, so a host path, a `(dev, ino)` pair, and a raw
/// host error cannot enter an expectation and therefore cannot be compared,
/// logged, or leaked through one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedBindingRow {
    binding_ref: ResourceRef,
    endpoint_ref: ResourceRef,
    endpoint_generation: u64,
    binding_generation: u64,
    consumer_ref: ResourceRef,
    slot: String,
    authorization_digest: String,
    dependency_revision: String,
    incarnation: String,
    preparation: BindingPreparation,
}

impl ExpectedBindingRow {
    /// Record one expected relationship.
    ///
    /// # Errors
    ///
    /// Returns [`BindingGateError::Malformed`] when any identity field is
    /// empty, a generation is zero, or the slot is not a bounded token. An
    /// expectation assembled from an incomplete fact would compare against
    /// evidence that can never match it, which reads as a permanent deferral
    /// rather than as the malformed expectation it is.
    #[allow(clippy::too_many_arguments, reason = "one closed expectation per argument")]
    pub fn new(
        binding_ref: ResourceRef,
        endpoint_ref: ResourceRef,
        endpoint_generation: u64,
        binding_generation: u64,
        consumer_ref: ResourceRef,
        slot: String,
        authorization_digest: String,
        dependency_revision: String,
        incarnation: String,
        preparation: BindingPreparation,
    ) -> Result<Self, BindingGateError> {
        if endpoint_generation == 0
            || binding_generation == 0
            || slot.is_empty()
            || authorization_digest.is_empty()
            || dependency_revision.is_empty()
            || incarnation.is_empty()
        {
            return Err(BindingGateError::Malformed);
        }
        BoundedToken::parse(slot.as_str()).map_err(|_| BindingGateError::Malformed)?;
        Ok(Self {
            binding_ref,
            endpoint_ref,
            endpoint_generation,
            binding_generation,
            consumer_ref,
            slot,
            authorization_digest,
            dependency_revision,
            incarnation,
            preparation,
        })
    }

    /// The canonical relationship row this expectation names.
    pub const fn binding_ref(&self) -> &ResourceRef {
        &self.binding_ref
    }

    /// The `Endpoint` row the relationship is published by.
    pub const fn endpoint_ref(&self) -> &ResourceRef {
        &self.endpoint_ref
    }

    /// The endpoint row generation the expectation was derived at.
    pub const fn endpoint_generation(&self) -> u64 {
        self.endpoint_generation
    }

    /// The relationship row generation the expectation was derived at.
    pub const fn binding_generation(&self) -> u64 {
        self.binding_generation
    }

    /// The exact consumer this relationship is for.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// The canonical consumer slot.
    pub fn slot(&self) -> &str {
        &self.slot
    }

    /// The digest of the authorization this relationship was derived under.
    pub fn authorization_digest(&self) -> &str {
        &self.authorization_digest
    }

    /// The revision of the dependencies the derivation was read at.
    pub fn dependency_revision(&self) -> &str {
        &self.dependency_revision
    }

    /// The opaque realization-incarnation token the endpoint published.
    pub fn incarnation(&self) -> &str {
        &self.incarnation
    }

    /// The per-relationship source-side preparation this expectation carries.
    ///
    /// This is the launch gate's composition with the conformance crate's own
    /// marker rather than a second one: where `Prepared` says the source side
    /// is established and `Incomplete` says it is still being established, the
    /// gate below reports `Pending` for the incomplete half and then demands
    /// DELIVERY evidence for the complete one (R39, R40, R18).
    pub const fn preparation(&self) -> BindingPreparation {
        self.preparation
    }
}

/// Why a launch binding gate could not answer (R18).
///
/// Every variant names a condition, never a material: a refusal carries no
/// socket name, no host path, and no `(dev, ino)` pair, so it reads the same
/// in a status, an audit record, and a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingGateError {
    /// The expected set was assembled from an incomplete or unparseable fact.
    Malformed,
    /// Evidence was published for a relationship this launch does not expect,
    /// or for one whose authority facts do not match the expectation.
    Foreign,
    /// A projection was published and could not be read.
    EvidenceUnreadable,
    /// A sealed lease was revoked between preparation and the effect.
    LeaseRevoked,
}

impl BindingGateError {
    /// The closed, host-free slug this refusal reports under.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Malformed => "process-binding-gate-malformed",
            Self::Foreign => "process-binding-gate-foreign-evidence",
            Self::EvidenceUnreadable => "process-binding-gate-evidence-unreadable",
            Self::LeaseRevoked => "process-binding-gate-lease-revoked",
        }
    }
}

impl core::fmt::Display for BindingGateError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for BindingGateError {}

/// Why a published delivery projection could not be read (R18).
///
/// The two are not the same answer. An absent projection is an ordinary
/// not-yet - the relationship's own actor has published nothing for the
/// current row generation - while a projection that IS present and does not
/// parse is evidence this launch cannot interpret. Reading an unreadable
/// projection as "not delivered" would defer forever; reading it as delivered
/// would launch over evidence nothing proved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingEvidenceFault {
    /// The relationship published no projection for its current row
    /// generation.
    Absent,
    /// A projection was published and could not be read.
    Unreadable,
}

/// The redacted delivery evidence one relationship's own actor published.
///
/// These are the four states the `EndpointBinding` contract publishes, and
/// only the first proves a delivery. `EndpointReplaced` in particular is NOT
/// a delivery: the grant was re-applied against a new realization, so a
/// consumer holding the old one has to re-derive rather than read the
/// replacement as its own evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingDeliveryEvidence {
    /// The exact endpoint is delivered at one incarnation.
    Delivered {
        /// The relationship row generation the evidence was published for.
        generation: u64,
        /// The same opaque realization-incarnation token the endpoint
        /// published.
        incarnation: String,
    },
    /// The pinned endpoint changed under a prepared relationship.
    EndpointReplaced,
    /// No host effect is standing for this relationship.
    Undelivered,
    /// The relationship is fenced and draining.
    Draining,
}

impl BindingDeliveryEvidence {
    /// Read one published binding projection.
    ///
    /// This is the TPM typed-projection gate's shape applied to the binding
    /// contract: the reader names the pointer it trusts, and anything it
    /// cannot read there is a fault rather than a value.
    pub fn from_projection(
        value: Option<&serde_json::Value>,
    ) -> Result<Self, BindingEvidenceFault> {
        let layer = value
            .and_then(|projection| projection.pointer("/binding"))
            .ok_or(BindingEvidenceFault::Absent)?;
        match layer.pointer("/state").and_then(serde_json::Value::as_str) {
            Some("delivered") => Ok(Self::Delivered {
                generation: layer
                    .pointer("/generation")
                    .and_then(serde_json::Value::as_u64)
                    .filter(|generation| *generation > 0)
                    .ok_or(BindingEvidenceFault::Unreadable)?,
                incarnation: layer
                    .pointer("/incarnation")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .ok_or(BindingEvidenceFault::Unreadable)?,
            }),
            Some("endpoint-replaced") => Ok(Self::EndpointReplaced),
            Some("undelivered") => Ok(Self::Undelivered),
            Some("draining") => Ok(Self::Draining),
            _ => Err(BindingEvidenceFault::Unreadable),
        }
    }

    /// Whether this evidence proves a delivery at `incarnation`.
    pub fn proves_delivery_at(&self, incarnation: &str) -> bool {
        matches!(
            self,
            Self::Delivered { incarnation: published, .. } if published == incarnation
        )
    }

    /// The closed state slug this evidence publishes.
    pub const fn state_slug(&self) -> &'static str {
        match self {
            Self::Delivered { .. } => "delivered",
            Self::EndpointReplaced => "endpoint-replaced",
            Self::Undelivered => "undelivered",
            Self::Draining => "draining",
        }
    }
}

/// What the manager currently reports for one expected relationship.
///
/// The authority facts are carried separately from the delivery state on
/// purpose: a delivery published for the right row but derived from a
/// different authorization, a different dependency revision, or a different
/// canonical slot is FOREIGN evidence, and the only way to see that is to
/// compare each fact rather than to trust the state slug (R18).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedBinding {
    binding_ref: ResourceRef,
    binding_uid: String,
    binding_generation: u64,
    consumer_ref: ResourceRef,
    slot: String,
    authorization_digest: String,
    dependency_revision: String,
    endpoint_ready: bool,
    endpoint_incarnation: Option<String>,
    delivery: Result<BindingDeliveryEvidence, BindingEvidenceFault>,
}

impl ObservedBinding {
    /// Record one observed relationship.
    #[allow(clippy::too_many_arguments, reason = "one closed observation per argument")]
    pub const fn new(
        binding_ref: ResourceRef,
        binding_uid: String,
        binding_generation: u64,
        consumer_ref: ResourceRef,
        slot: String,
        authorization_digest: String,
        dependency_revision: String,
        endpoint_ready: bool,
        endpoint_incarnation: Option<String>,
        delivery: Result<BindingDeliveryEvidence, BindingEvidenceFault>,
    ) -> Self {
        Self {
            binding_ref,
            binding_uid,
            binding_generation,
            consumer_ref,
            slot,
            authorization_digest,
            dependency_revision,
            endpoint_ready,
            endpoint_incarnation,
            delivery,
        }
    }

    /// The relationship row this observation describes.
    pub const fn binding_ref(&self) -> &ResourceRef {
        &self.binding_ref
    }
}

/// One sealed relationship inside a [`BindingAuthorityLease`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingLeaseRow {
    expectation: ExpectedBindingRow,
    binding_uid: String,
    binding_generation: u64,
}

impl BindingLeaseRow {
    /// The expectation this lease sealed.
    pub const fn expectation(&self) -> &ExpectedBindingRow {
        &self.expectation
    }

    /// The relationship row identity observed when the lease was sealed.
    pub fn binding_uid(&self) -> &str {
        &self.binding_uid
    }

    /// The relationship row generation observed when the lease was sealed.
    pub const fn binding_generation(&self) -> u64 {
        self.binding_generation
    }

    /// Whether one freshly observed relationship still matches what was
    /// sealed: the same row identity, the same row generation, the same
    /// authority facts, and a delivery that still proves the SAME realization.
    fn matches(&self, observed: &ObservedBinding) -> bool {
        if observed.binding_uid != self.binding_uid
            || observed.binding_generation != self.binding_generation
            || observed.consumer_ref != self.expectation.consumer_ref
            || observed.slot != self.expectation.slot
            || observed.authorization_digest != self.expectation.authorization_digest
            || observed.dependency_revision != self.expectation.dependency_revision
            || !observed.endpoint_ready
        {
            return false;
        }
        match &observed.delivery {
            Ok(evidence) => evidence.proves_delivery_at(&self.expectation.incarnation),
            Err(_) => false,
        }
    }
}

/// The closed outcome of one Process preparation against its expected binding
/// set (KTD6).
///
/// This is the launch-level aggregate over the per-relationship
/// [`BindingPreparation`] marker: where that value says whether ONE
/// relationship's source side is established, this one says whether the whole
/// expected set is delivered at the realization the launch is gated on. It is
/// the only shape a launch or an adoption reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessBindingPreparation {
    /// This launch requires no `EndpointBinding` at all.
    ///
    /// The answer comes from the expected set being genuinely empty, which is
    /// the answer for every existing non-display Process. It is a real answer
    /// and not a fallback: a row that REQUIRES a relationship and cannot see
    /// its evidence defers instead of reporting this.
    NotRequired,
    /// Every expected relationship is delivered at the expected realization,
    /// and the sealed lease may be carried into the launch.
    Ready(BindingAuthorityLease),
    /// The set is not delivered yet.
    ///
    /// A missing row, an endpoint that is not ready, an undelivered, replaced
    /// or draining relationship, and a relationship whose source side is still
    /// incomplete all land here. The launch requeues through the runtime's
    /// retryable path and issues NO effect.
    Pending,
    /// The evidence is malformed or foreign, and retrying cannot fix it.
    ///
    /// Terminal for this launch: an observation naming a relationship this
    /// launch does not expect, one whose authority facts do not match, and a
    /// projection that was published but cannot be read are all refused rather
    /// than deferred.
    Refused(BindingGateError),
}

impl ProcessBindingPreparation {
    /// Whether this preparation produced a sealed lease.
    pub fn lease(&self) -> Option<&BindingAuthorityLease> {
        match self {
            Self::Ready(lease) => Some(lease),
            Self::NotRequired | Self::Pending | Self::Refused(_) => None,
        }
    }

    /// Whether this launch requires no relationship.
    pub const fn is_not_required(&self) -> bool {
        matches!(self, Self::NotRequired)
    }

    /// Whether this launch must wait for delivery evidence.
    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Pending)
    }

    /// The refusal this preparation ended in, if it refused.
    pub const fn refusal(&self) -> Option<BindingGateError> {
        match self {
            Self::Refused(error) => Some(*error),
            Self::NotRequired | Self::Ready(_) | Self::Pending => None,
        }
    }
}

/// A revocable snapshot of the authority a `Ready` preparation proved.
///
/// The lease is what makes the gate a fence rather than a reading: it carries
/// the exact expectation and the exact relationship identity and generation
/// observed when preparation concluded, so the same comparison can be run
/// again IMMEDIATELY BEFORE the effect, inside the same serialized boundary
/// the preparation ran under. Anything that moved in between - a re-derived
/// row, a re-issued grant, a withdrawn authorization, a re-realized endpoint -
/// makes the revalidation fail closed, and no effect is issued (KTD6, R18).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingAuthorityLease {
    consumer: ResourceUid,
    rows: Vec<BindingLeaseRow>,
}

impl BindingAuthorityLease {
    /// Seal one `Ready` preparation.
    ///
    /// # Errors
    ///
    /// Returns [`BindingGateError::Foreign`] when the observation set names a
    /// relationship the expected set does not, or one that is not for this
    /// launch's committed consumer. A lease that sealed a mismatched set would
    /// carry that mismatch into the effect.
    pub fn seal(
        consumer: ResourceUid,
        expected: &[ExpectedBindingRow],
        observed: &[ObservedBinding],
    ) -> Result<Self, BindingGateError> {
        let mut rows = Vec::with_capacity(expected.len());
        for row in expected {
            let found = observed
                .iter()
                .find(|candidate| candidate.binding_ref() == row.binding_ref())
                .ok_or(BindingGateError::Foreign)?;
            if found.consumer_ref != row.consumer_ref {
                return Err(BindingGateError::Foreign);
            }
            rows.push(BindingLeaseRow {
                expectation: row.clone(),
                binding_uid: found.binding_uid.clone(),
                binding_generation: found.binding_generation,
            });
        }
        Ok(Self { consumer, rows })
    }

    /// The committed consumer identity this lease was sealed for.
    pub const fn consumer(&self) -> &ResourceUid {
        &self.consumer
    }

    /// The relationships this lease covers.
    pub fn rows(&self) -> &[BindingLeaseRow] {
        &self.rows
    }

    /// Revalidate the lease against freshly observed evidence.
    ///
    /// This is the call that belongs immediately before the launch or
    /// adoption effect, not before the preparation: a lease that was correct
    /// when it was sealed says nothing about the moment the process starts.
    ///
    /// # Errors
    ///
    /// [`BindingGateError::LeaseRevoked`] when the authority this lease sealed
    /// no longer holds: a relationship row was replaced, re-issued, or moved
    /// generation, its delivery was withdrawn, its realization changed, or its
    /// authorization facts moved.
    pub fn revalidate(&self, observed: &[ObservedBinding]) -> Result<(), BindingGateError> {
        if observed.len() != self.rows.len() {
            return Err(BindingGateError::LeaseRevoked);
        }
        for row in &self.rows {
            let Some(found) = observed
                .iter()
                .find(|candidate| candidate.binding_ref() == row.expectation.binding_ref())
            else {
                return Err(BindingGateError::LeaseRevoked);
            };
            if !row.matches(found) {
                return Err(BindingGateError::LeaseRevoked);
            }
        }
        Ok(())
    }
}

/// Resolve one Process preparation against its expected binding set.
///
/// The outcomes are closed and the mapping is total (KTD6):
///
/// - an empty expected set is [`ProcessBindingPreparation::NotRequired`];
/// - a missing row, an endpoint that is not `Ready`, an undelivered, replaced
///   or draining relationship, an absent projection, and a relationship whose
///   source side is still incomplete are
///   [`ProcessBindingPreparation::Pending`] - the launch defers and issues no
///   effect;
/// - an observation naming a relationship this launch does not expect, one
///   whose authority facts do not match, and a projection that was published
///   but cannot be read are [`ProcessBindingPreparation::Refused`] - terminal,
///   because retrying the same evidence cannot change either answer;
/// - everything matching is [`ProcessBindingPreparation::Ready`] carrying the
///   sealed lease.
pub fn resolve_process_binding_preparation(
    consumer: ResourceUid,
    expected: &[ExpectedBindingRow],
    observed: &[ObservedBinding],
) -> ProcessBindingPreparation {
    if expected.is_empty() {
        return ProcessBindingPreparation::NotRequired;
    }
    // Evidence for a relationship this launch does not expect is not evidence
    // about any of the ones it does: it is another launch's, or a stale one.
    // It is refused before the expected set is compared, so a foreign row can
    // never be satisfied by a matching one.
    if observed.iter().any(|candidate| {
        !expected
            .iter()
            .any(|row| row.binding_ref() == candidate.binding_ref())
    }) {
        return ProcessBindingPreparation::Refused(BindingGateError::Foreign);
    }
    for row in expected {
        if !matches!(row.preparation(), BindingPreparation::Prepared) {
            // The source side is still being established. A consumer that
            // started now would be waiting on access that does not exist,
            // which is exactly the startup cycle R40 removes.
            return ProcessBindingPreparation::Pending;
        }
        let Some(found) = observed
            .iter()
            .find(|candidate| candidate.binding_ref() == row.binding_ref())
        else {
            // A row this launch requires that the manager cannot answer for is
            // not delivered. Deferring is the honest answer and keeps the
            // launch out of the effect path entirely.
            return ProcessBindingPreparation::Pending;
        };
        if found.consumer_ref != row.consumer_ref
            || found.slot != row.slot
            || found.authorization_digest != row.authorization_digest
            || found.dependency_revision != row.dependency_revision
            || found.binding_uid.is_empty()
            || found.binding_generation != row.binding_generation
        {
            return ProcessBindingPreparation::Refused(BindingGateError::Foreign);
        }
        if !found.endpoint_ready {
            return ProcessBindingPreparation::Pending;
        }
        match found.endpoint_incarnation.as_deref() {
            Some(incarnation) if incarnation == row.incarnation => {}
            // An endpoint that published a DIFFERENT realization is evidence
            // about another incarnation, not a stale reading of this one.
            Some(_) => return ProcessBindingPreparation::Refused(BindingGateError::Foreign),
            None => return ProcessBindingPreparation::Pending,
        }
        match &found.delivery {
            Err(BindingEvidenceFault::Unreadable) => {
                return ProcessBindingPreparation::Refused(BindingGateError::EvidenceUnreadable);
            }
            Err(BindingEvidenceFault::Absent) | Ok(BindingDeliveryEvidence::Undelivered) => {
                return ProcessBindingPreparation::Pending;
            }
            Ok(BindingDeliveryEvidence::EndpointReplaced) | Ok(BindingDeliveryEvidence::Draining) => {
                // A replacement is a re-derived delivery at a realization the
                // consumer has not observed, and draining is a fence: both
                // WAIT rather than refuse, because the relationship's own actor
                // resolves them by publishing fresh evidence.
                return ProcessBindingPreparation::Pending;
            }
            Ok(evidence) => {
                if !evidence.proves_delivery_at(&row.incarnation) {
                    return ProcessBindingPreparation::Refused(BindingGateError::Foreign);
                }
            }
        }
    }
    match BindingAuthorityLease::seal(consumer, expected, observed) {
        Ok(lease) => ProcessBindingPreparation::Ready(lease),
        Err(error) => ProcessBindingPreparation::Refused(error),
    }
}
