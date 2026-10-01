//! The CredentialBinding resource driver: the v3 `ResourceDriver` conversion
//! of the row side of one admitted credential delivery.
//!
//! A `CredentialBinding` row is the consumer's own canonical declaration,
//! read back out of committed bytes: it names one exact `Credential`, one
//! admitted consumer, one stable consumer slot, the operation classes that
//! consumer may perform, and the lifetime bound - and nothing else. The
//! driver owns the row side of that relationship.
//!
//! What is here, in the order a pass uses it:
//!
//! - the delivery vocabulary ([`CredentialDelivery`], [`DeliveredSession`],
//!   [`CredentialRevocation`]): the exact non-secret identity one admitted
//!   delivery is made under, the evidence a delivery answers with, and the
//!   idempotent teardown request. [`DeliveredSession::serves`] is the one
//!   predicate that decides whether a live session is the authority this
//!   row's committed spec asked for.
//! - the driver effect port ([`CredentialBindingDriverEffects`]): the
//!   delivery, the observation, the revocation, and the clock.
//! - the spec decoder and the one seam every row read goes through,
//!   [`binding_spec_decoder`] and [`CredentialBindingDriver::committed_row`]:
//!   the committed envelope is decoded, the provider binding is checked, and
//!   the base layer is parsed into the contract's own
//!   [`CredentialBindingSpec`](d2b_contracts_resource::v3::credential_binding::CredentialBindingSpec).
//!   No second copy of a binding facet lives here: a refusal this crate
//!   published for a row the contract admits, or admitted one the contract
//!   refuses, is exactly the drift this refactor exists to prevent, so the
//!   contract decides and this crate names the invariant it decided on.
//! - the driver verbs: validate resolves the source `Credential` row and its
//!   owner fence, reconcile delivers to the exact destination and publishes
//!   the fenced projection, recover adopts a live delivery instead of
//!   minting a second one, and pre-drain, finalize, and delete revoke the
//!   delivery idempotently.
//! - the fenced readiness projection ([`CredentialBindingStatusResource`])
//!   the row publishes and the read side accepts.
//!
//! Credential material never enters this module or any other in this crate.
//! The driver holds identity, vocabulary, counters, and bounds; the material
//! is minted and kept inside the admitted delivery session on the far side of
//! the effect port. Nothing the crate publishes - status, projection, failure
//! detail, trace - has a field a secret byte could occupy.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::credential_binding::CredentialBindingSpec;
use d2b_contracts_resource::v3::resource_status::StatusCode;
use d2b_contracts_resource::v3::{
    BindingConsumerKind, BindingKind, BindingLifecycleState, BindingRealizationFacet,
    CanonicalJsonObject, CREDENTIAL_BINDING_RESOURCE_TYPE, CredentialOperation,
    MAX_CREDENTIAL_LIFETIME_MS, MAX_CREDENTIAL_OPERATIONS, MIN_CREDENTIAL_LIFETIME_MS,
    RequestedRights, ResourceGeneration, ResourceRef, ResourceSpec, ResourceUid, ZoneId,
    ZoneRevision,
};
use d2b_resource_runtime::context::{
    ResourceContext, RowLookup, SpecDecoder, WatchCondition, typed_spec_decoder,
};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{
    DriverFailure, DriverOp, FailureClass, FailureComparison, FailureDetail, FailureKind,
    FailureKinds,
};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
use d2b_resource_types::{
    AllowedSources, CONVERTED_TYPE_VERBS, ChildCreation, DriverDescriptor, WellKnownType,
};

use crate::effects_service::{CREDENTIAL_BINDING_EFFECTS_SERVICE, CredentialBindingEffectsService};
use crate::facets::CredentialBindingEffectFacets;

/// The one resource type this crate serves.
///
/// Taken from the canonical contract constant rather than spelled again: the
/// crate's type name and the contract's ResourceType name are the same fact,
/// and two spellings could drift.
pub const CREDENTIAL_BINDING_TYPE_NAME: &str = CREDENTIAL_BINDING_RESOURCE_TYPE;

/// The serving Provider this crate owns.
///
/// A binding row names the provider that realizes the delivery leg, exactly as
/// a `VolumeBinding` row names the provider that serves its share rather than
/// the provider that owns the `Volume`.
pub const CREDENTIAL_BINDING_PROVIDER_REF: &str = "Provider/credential-binding";

/// The children this family's driver mints, in creation rank order.
///
/// The list is empty by construction and stays that way: a credential
/// delivery is realized inside an admitted session at an execution target
/// that already exists. Minting a child row per delivery would put the
/// relationship's existence in the graph a second time and would let the
/// delivery outlive the authority that admitted it, so the realization is the
/// effect port's delivery and nothing else.
pub const CREDENTIAL_BINDING_CREATIONS: &[ChildCreation] = &[];

/// The resource types this family's driver reads while reconciling.
///
/// The source `Credential` row carries the binding's owner fence and the
/// identity a delivery is fenced against; the consumer component rows are the
/// exact destinations a delivery may be made to, and their committed
/// generations are half of the delivery fence. `Host` is absent on purpose:
/// the family does not admit a Host consumer, so the contract refuses such a
/// row before this driver ever reads it.
pub const CREDENTIAL_BINDING_READS: &[WellKnownType] = &[
    WellKnownType::CREDENTIAL,
    WellKnownType::PROCESS,
    WellKnownType::EPHEMERAL_PROCESS,
    WellKnownType::GUEST,
];

/// The execution domains the CredentialBinding type can be reconciled in.
///
/// Derived from the placement contract: `CredentialBinding` names no
/// placement anchor, so a binding row never carries the canonical
/// `spec.executionRef` and the plane reconciles it on its containing Zone's
/// Host. The row's execution target is the destination of the delivery, never
/// where the binding row itself is reconciled.
pub const CREDENTIAL_BINDING_EXECUTION_DOMAINS: &[&str] = &["host"];

/// The exact, non-secret identity one admitted credential delivery is made
/// under.
///
/// Every field is an identity, a counter, a bound, or a vocabulary code. The
/// credential material itself is created behind the delivery call and is
/// never returned to the driver, so the driver has nothing to persist,
/// nothing to log, and nothing a future field could turn into a carrier: the
/// type has no serialization at all.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialDelivery {
    binding: ResourceKey,
    source: ResourceKey,
    source_uid: ResourceUid,
    source_generation: ResourceGeneration,
    destination: ResourceKey,
    destination_uid: ResourceUid,
    destination_generation: ResourceGeneration,
    slot: String,
    operations: Vec<&'static str>,
    expires_unix_ms: u64,
}

impl CredentialDelivery {
    /// Admit one delivery from the identities the driver resolved.
    ///
    /// The caller supplies everything the fence is made of: the binding row
    /// that owns the relationship, the exact source `Credential` row and the
    /// exact destination row in the same Zone, the stable consumer slot, the
    /// admitted operation classes, and the absolute instant the authority
    /// expires at. Nothing else is accepted, so a delivery cannot be minted
    /// for a destination, a source, or a window the caller did not name.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        binding: ResourceKey,
        source: ResourceKey,
        source_uid: ResourceUid,
        source_generation: ResourceGeneration,
        destination: ResourceKey,
        destination_uid: ResourceUid,
        destination_generation: ResourceGeneration,
        slot: impl Into<String>,
        operations: Vec<&'static str>,
        expires_unix_ms: u64,
    ) -> Self {
        Self {
            binding,
            source,
            source_uid,
            source_generation,
            destination,
            destination_uid,
            destination_generation,
            slot: slot.into(),
            operations,
            expires_unix_ms,
        }
    }

    /// The binding row this delivery belongs to.
    pub const fn binding(&self) -> &ResourceKey {
        &self.binding
    }

    /// The exact source `Credential` row this delivery draws from.
    pub const fn source(&self) -> &ResourceKey {
        &self.source
    }

    /// The source identity the delivery is fenced against.
    pub const fn source_uid(&self) -> &ResourceUid {
        &self.source_uid
    }

    /// The source generation the delivery is fenced against.
    pub const fn source_generation(&self) -> ResourceGeneration {
        self.source_generation
    }

    /// The exact destination the credential is delivered to.
    pub const fn destination(&self) -> &ResourceKey {
        &self.destination
    }

    /// The destination identity the delivery is fenced against.
    pub const fn destination_uid(&self) -> &ResourceUid {
        &self.destination_uid
    }

    /// The destination generation the delivery is fenced against.
    pub const fn destination_generation(&self) -> ResourceGeneration {
        self.destination_generation
    }

    /// The stable consumer slot this relationship occupies.
    pub fn slot(&self) -> &str {
        &self.slot
    }

    /// The operation classes this delivery admits.
    pub fn operations(&self) -> &[&'static str] {
        &self.operations
    }

    /// The instant after which this authority admits nothing.
    pub const fn expires_unix_ms(&self) -> u64 {
        self.expires_unix_ms
    }
}

impl core::fmt::Debug for CredentialDelivery {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // The slot and the two identities are redacted for the same reason
        // the family's source-side fence redacts them: a log line is not the
        // place a policy value or a durable identity belongs. The counters,
        // the vocabulary, and the bound are the diagnostic part and render.
        formatter
            .debug_struct("CredentialDelivery")
            .field("binding", &self.binding)
            .field("source_generation", &self.source_generation)
            .field("destination_generation", &self.destination_generation)
            .field("operations", &self.operations)
            .field("expires_unix_ms", &self.expires_unix_ms)
            .field("source", &"<redacted>")
            .field("source_uid", &"<redacted>")
            .field("destination", &"<redacted>")
            .field("destination_uid", &"<redacted>")
            .field("slot", &"<redacted>")
            .finish()
    }
}

/// The non-secret identity one established delivery answers with.
///
/// This is evidence, not authority: the caller still compares it against the
/// committed request through [`DeliveredSession::serves`], and a session that
/// does not serve that request is stale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveredSession {
    destination: ResourceKey,
    source_uid: ResourceUid,
    destination_uid: ResourceUid,
    source_generation: ResourceGeneration,
    destination_generation: ResourceGeneration,
    sequence: u64,
    expires_unix_ms: u64,
}

impl DeliveredSession {
    /// The identity one established delivery answers with.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        destination: ResourceKey,
        source_uid: ResourceUid,
        destination_uid: ResourceUid,
        source_generation: ResourceGeneration,
        destination_generation: ResourceGeneration,
        sequence: u64,
        expires_unix_ms: u64,
    ) -> Self {
        Self {
            destination,
            source_uid,
            destination_uid,
            source_generation,
            destination_generation,
            sequence,
            expires_unix_ms,
        }
    }

    /// The destination the credential was delivered to.
    pub const fn destination(&self) -> &ResourceKey {
        &self.destination
    }

    /// The source identity the session is fenced against.
    pub const fn source_uid(&self) -> &ResourceUid {
        &self.source_uid
    }

    /// The destination identity the session is fenced against.
    pub const fn destination_uid(&self) -> &ResourceUid {
        &self.destination_uid
    }

    /// The source generation the session is fenced against.
    pub const fn source_generation(&self) -> ResourceGeneration {
        self.source_generation
    }

    /// The destination generation the session is fenced against.
    pub const fn destination_generation(&self) -> ResourceGeneration {
        self.destination_generation
    }

    /// The monotonic replay sequence the session carries.
    ///
    /// A later delivery for the same relationship carries a strictly larger
    /// sequence, so a session replayed after a fresh admission is visible as
    /// stale even when every other identity matches.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// The instant after which this session admits nothing.
    pub const fn expires_unix_ms(&self) -> u64 {
        self.expires_unix_ms
    }

    /// Whether this live session is the authority the committed request asked
    /// for.
    ///
    /// The identity half is exact. A session at another destination, over
    /// another source or destination identity, or under another source or
    /// destination generation is a different authority: changing the
    /// component generation invalidates the earlier delivery instead of
    /// renewing it, so such a session is never reused. The bound half is
    /// one-sided: the port may deliver a window narrower than the one
    /// admitted, never a wider one, and any window whose expiry has passed
    /// authorizes nothing.
    pub fn serves(&self, delivery: &CredentialDelivery, now_unix_ms: u64) -> bool {
        self.destination == *delivery.destination()
            && self.source_uid == *delivery.source_uid()
            && self.destination_uid == *delivery.destination_uid()
            && self.source_generation == delivery.source_generation()
            && self.destination_generation == delivery.destination_generation()
            && self.expires_unix_ms <= delivery.expires_unix_ms()
            && now_unix_ms < self.expires_unix_ms
    }
}

/// One idempotent revocation request.
///
/// The binding row is always named, so a teardown can retire this row's
/// delivery even when its spec no longer decodes. The destination is named
/// whenever the row still decodes: revocation then retires exactly the
/// delivery this row's committed spec claims and never a destination the row
/// no longer claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRevocation {
    binding: ResourceKey,
    destination: Option<ResourceKey>,
}

impl CredentialRevocation {
    /// Revoke whatever delivery this binding row holds.
    pub fn for_row(binding: &ResourceKey) -> Self {
        Self {
            binding: binding.clone(),
            destination: None,
        }
    }

    /// Revoke the delivery this binding row holds at one exact destination.
    pub fn at_destination(binding: &ResourceKey, destination: &ResourceKey) -> Self {
        Self {
            binding: binding.clone(),
            destination: Some(destination.clone()),
        }
    }

    /// The binding row whose delivery is revoked.
    pub const fn binding(&self) -> &ResourceKey {
        &self.binding
    }

    /// The exact destination to revoke at, when the row still names one.
    pub fn destination(&self) -> Option<&ResourceKey> {
        self.destination.as_ref()
    }
}

/// The provider-facing delivery effect surface the family's driver needs.
///
/// The production implementation lives in this crate behind the declared
/// facets ([`crate::effects_service`]); test doubles implement the same seam.
/// Nothing here returns credential material: [`Self::deliver`] answers with
/// the non-secret identity of the session it established, and
/// [`Self::revoke`] answers with proof that the delivery is retired.
#[async_trait::async_trait]
pub trait CredentialBindingDriverEffects: Send + Sync + 'static {
    /// Deliver the credential to the delivery's exact destination and answer
    /// the non-secret identity of the session established.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the daemon-supplied adapter cannot establish the
    /// delivery. The driver classifies the failure retryable and re-runs the
    /// pass; a delivery that could not be established is never recorded as
    /// established.
    async fn deliver(&self, delivery: &CredentialDelivery) -> Result<DeliveredSession, String>;

    /// The live delivery at the delivery's exact destination, if there is one.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the adapter cannot answer. An unanswered question
    /// is never read as a revoked credential: only a destination the adapter
    /// genuinely holds nothing for answers `Ok(None)`.
    async fn observe(
        &self,
        delivery: &CredentialDelivery,
    ) -> Result<Option<DeliveredSession>, String>;

    /// Revoke the delivery this binding row holds.
    ///
    /// # Errors
    ///
    /// Returns `Err` when the revocation could not be proven. The driver
    /// defers retryably and keeps the row's durable deleting mark, so an
    /// unconfirmed revoke never publishes a release it could not prove.
    async fn revoke(&self, revocation: &CredentialRevocation) -> Result<(), String>;

    /// The current wall-clock instant in milliseconds since the Unix epoch.
    ///
    /// A delivery's authority ends at a deadline, so the window it is minted
    /// under must be computed against the same clock the session itself is
    /// bounded by.
    fn now_unix_ms(&self) -> u64;
}

// ---------------------------------------------------------------------------
// The fenced readiness projection
// ---------------------------------------------------------------------------

/// The UID, generation, and revision fence one delivery report is observed
/// under.
///
/// A report is only current when the UID and generation match the row's own
/// identity and the fence revision is at most the stored revision. The UID
/// pins reassignment and the generation pins a spec change; together they
/// bound how stale a report may be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialBindingReadinessFence {
    uid: ResourceUid,
    generation: ResourceGeneration,
    revision: ZoneRevision,
}

impl CredentialBindingReadinessFence {
    /// The fence one report observed this row under.
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

    /// The row generation the evidence was observed under.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    /// The Zone-store revision the evidence was observed under.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Whether this fence still matches the row's current identity.
    pub fn matches(&self, uid: &ResourceUid, generation: ResourceGeneration, revision: ZoneRevision) -> bool {
        self.uid == *uid && self.generation == generation && self.revision <= revision
    }
}

/// The CredentialBinding status projection a row publishes.
///
/// The field set is closed and deliberately minimal: the observed lifecycle
/// state, whether a delivery is live, the fence the observation was made
/// under, the replay sequence, the expiry bound, and a stable reason when one
/// applies. The audience, the credential identity, the route digest, the
/// lease handle, and the material itself are absent by construction, and
/// there is no field a credential byte could occupy.
///
/// [`Self::parse`] is the reader: it accepts exactly this field set, refuses
/// anything else, and is the only way a stored layer becomes this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialBindingStatusResource {
    state: &'static str,
    delivered: bool,
    fence: CredentialBindingReadinessFence,
    sequence: u64,
    expires_unix_ms: u64,
    reason: Option<StatusCode>,
}

impl CredentialBindingStatusResource {
    /// The field names the projection defines, in its rendered order.
    const REQUIRED_FIELDS: [&'static str; 5] = [
        "state",
        "delivered",
        "fence",
        "sequence",
        "expiresUnixMs",
    ];

    /// The field names a projection carries only when they apply.
    const OPTIONAL_FIELDS: [&'static str; 1] = ["reason"];

    /// The fence field names the projection defines.
    const FENCE_FIELDS: [&'static str; 3] = ["uid", "generation", "revision"];

    /// The closed lifecycle vocabulary this projection publishes.
    ///
    /// The states are the shared binding vocabulary, spelled once here so the
    /// projection never carries a state this crate invented.
    const STATES: [&'static str; 10] = [
        "requested",
        "admitted",
        "prepared",
        "active",
        "revoking",
        "draining",
        "released",
        "refused",
        "degraded",
        "unknown",
    ];

    /// Render one observation as the wire projection.
    ///
    /// The rendering is built through the canonical JSON object path, so a
    /// structural character in a committed value yields a correctly escaped
    /// report rather than an unparseable one.
    pub fn render(
        state: BindingLifecycleState,
        delivered: bool,
        fence: CredentialBindingReadinessFence,
        sequence: u64,
        expires_unix_ms: u64,
        reason: Option<StatusCode>,
    ) -> serde_json::Value {
        let mut projection = serde_json::json!({
            "state": state_code(state),
            "delivered": delivered,
            "fence": {
                "uid": fence.uid().to_canonical_string(),
                "generation": fence.generation().get(),
                "revision": fence.revision().get(),
            },
            "sequence": sequence,
            "expiresUnixMs": expires_unix_ms,
        });
        if let Some(reason) = reason {
            projection
                .as_object_mut()
                .expect("the projection is built as a JSON object")
                .insert("reason".to_owned(), serde_json::json!(reason.as_str()));
        }
        projection
    }

    /// Read one wire projection back, refusing anything this contract does not
    /// define.
    ///
    /// `None` means the stored layer is not this projection: a missing field,
    /// an extra field, a value of the wrong shape, an unknown lifecycle
    /// state, or a fence that is not a canonical uid, generation, and
    /// revision. Callers fail closed on `None` rather than reading a partial
    /// report as readiness.
    pub fn parse(value: &serde_json::Value) -> Option<Self> {
        let object = value.as_object()?;
        if !Self::REQUIRED_FIELDS.iter().all(|field| object.contains_key(*field))
            || object.keys().any(|key| {
                !Self::REQUIRED_FIELDS.contains(&key.as_str())
                    && !Self::OPTIONAL_FIELDS.contains(&key.as_str())
            })
        {
            return None;
        }
        let observed = object.get("state")?.as_str()?;
        let state = Self::STATES
            .iter()
            .copied()
            .find(|code| *code == observed)?;
        let delivered = object.get("delivered")?.as_bool()?;
        let fence = Self::parse_fence(object.get("fence")?)?;
        let sequence = object.get("sequence")?.as_u64()?;
        let expires_unix_ms = object.get("expiresUnixMs")?.as_u64()?;
        let reason = match object.get("reason") {
            Some(reason) => Some(StatusCode::parse(reason.as_str()?).ok()?),
            None => None,
        };
        Some(Self {
            state,
            delivered,
            fence,
            sequence,
            expires_unix_ms,
            reason,
        })
    }

    /// Read the fence half of a stored projection.
    fn parse_fence(value: &serde_json::Value) -> Option<CredentialBindingReadinessFence> {
        let object = value.as_object()?;
        if object.len() != Self::FENCE_FIELDS.len()
            || object
                .keys()
                .any(|key| !Self::FENCE_FIELDS.contains(&key.as_str()))
        {
            return None;
        }
        Some(CredentialBindingReadinessFence::new(
            ResourceUid::parse(object.get("uid")?.as_str()?).ok()?,
            ResourceGeneration::new(object.get("generation")?.as_u64()?).ok()?,
            ZoneRevision::new(object.get("revision")?.as_u64()?),
        ))
    }

    /// The observed lifecycle state code.
    pub const fn state(&self) -> &'static str {
        self.state
    }

    /// Whether a delivery was live when the observation was made.
    pub const fn delivered(&self) -> bool {
        self.delivered
    }

    /// The fence the observation was made under.
    pub const fn fence(&self) -> &CredentialBindingReadinessFence {
        &self.fence
    }

    /// The replay sequence the live session carried.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// The instant after which the observed authority admits nothing.
    pub const fn expires_unix_ms(&self) -> u64 {
        self.expires_unix_ms
    }

    /// The stable reason the delivery is not live, when one applies.
    pub fn reason(&self) -> Option<&StatusCode> {
        self.reason.as_ref()
    }

    /// Whether this projection reports a live delivery under the current
    /// fence.
    ///
    /// Readiness without a matching uid and generation is never current
    /// (fail closed). The fence revision only needs to precede the stored
    /// revision: the status write carrying the report advances the store past
    /// the observed commit.
    pub fn readiness_is_current(
        &self,
        uid: &ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
    ) -> bool {
        self.delivered && self.fence.matches(uid, generation, revision)
    }
}

/// The closed lifecycle vocabulary's wire code.
const fn state_code(state: BindingLifecycleState) -> &'static str {
    match state {
        BindingLifecycleState::Requested => "requested",
        BindingLifecycleState::Admitted => "admitted",
        BindingLifecycleState::Prepared => "prepared",
        BindingLifecycleState::Active => "active",
        BindingLifecycleState::Revoking => "revoking",
        BindingLifecycleState::Draining => "draining",
        BindingLifecycleState::Released => "released",
        BindingLifecycleState::Refused => "refused",
        BindingLifecycleState::Degraded => "degraded",
        BindingLifecycleState::Unknown => "unknown",
    }
}

// ---------------------------------------------------------------------------
// The committed row: the one seam every read goes through
// ---------------------------------------------------------------------------

/// The wire code one admitted operation class serializes as.
const ACQUIRE_TOKEN: &str = "acquire-token";
const REFRESH_TOKEN: &str = "refresh-token";
const SIGN_CHALLENGE: &str = "sign-challenge";

/// Every operation class that names a delivery session, in contract order.
///
/// The closed set a `CredentialBinding` row may declare. A row that names
/// anything else - including the Credential protocol operations, which have
/// no delivery session and therefore no binding at all - is refused rather
/// than approximated.
const DELIVERY_OPERATION_CODES: [&str; 3] = [ACQUIRE_TOKEN, REFRESH_TOKEN, SIGN_CHALLENGE];

/// The code one admitted operation class serializes as.
const fn operation_code(operation: CredentialOperation) -> &'static str {
    match operation {
        CredentialOperation::AcquireToken => ACQUIRE_TOKEN,
        CredentialOperation::RefreshToken => REFRESH_TOKEN,
        CredentialOperation::SignChallenge => SIGN_CHALLENGE,
    }
}

/// The spec-store envelope for one CredentialBinding row, exactly as
/// persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BindingSpecEnvelope {
    raw: Vec<u8>,
    provider_ref: Option<ResourceRef>,
    base: CanonicalJsonObject,
}

/// The manager-wired decode hook for CredentialBinding rows.
///
/// The envelope carries the provider binding; the base layer is the strict
/// row contract, which is the only authority on what a binding row may say.
pub fn binding_spec_decoder() -> Arc<dyn SpecDecoder> {
    typed_spec_decoder(|bytes| {
        serde_json::from_slice::<ResourceSpec>(bytes).map(|spec| BindingSpecEnvelope {
            raw: bytes.to_vec(),
            provider_ref: spec.provider_ref().cloned(),
            base: spec.base().clone(),
        })
    })
}

/// Which committed invariant a refused row violated.
///
/// The row contract is what refuses the row; these are the names this driver
/// publishes for that refusal, so an operator reads which invariant answered
/// instead of a generic decode error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpecRefusal {
    SelectorWildcard,
    SourceNotCredential,
    ConsumerUnsupported,
    OperationUndeclared,
    LifetimeUnbounded,
    Malformed,
    ProviderUnsupported,
}

impl SpecRefusal {
    const fn kind(self) -> CredentialDriverErrorKind {
        match self {
            Self::SelectorWildcard => CredentialDriverErrorKind::SelectorWildcard,
            Self::SourceNotCredential => CredentialDriverErrorKind::SourceNotCredential,
            Self::ConsumerUnsupported => CredentialDriverErrorKind::ConsumerUnsupported,
            Self::OperationUndeclared => CredentialDriverErrorKind::OperationUndeclared,
            Self::LifetimeUnbounded => CredentialDriverErrorKind::LifetimeUnbounded,
            Self::Malformed => CredentialDriverErrorKind::SpecMalformed,
            Self::ProviderUnsupported => CredentialDriverErrorKind::ProviderUnsupported,
        }
    }

    const fn stage(self) -> &'static str {
        match self {
            Self::SelectorWildcard => "spec/selector",
            Self::SourceNotCredential => "spec/source",
            Self::ConsumerUnsupported => "spec/consumer",
            Self::OperationUndeclared => "spec/operations",
            Self::LifetimeUnbounded => "spec/lifetime",
            Self::Malformed => "spec/decode",
            Self::ProviderUnsupported => "spec/provider",
        }
    }

    const fn field(self) -> &'static str {
        match self {
            Self::SelectorWildcard => "binding.selector",
            Self::SourceNotCredential => "spec.credentialRef",
            Self::ConsumerUnsupported => "spec.executionRef",
            Self::OperationUndeclared => "spec.operations",
            Self::LifetimeUnbounded => "spec.lifetimeMs",
            Self::Malformed => "spec.base",
            Self::ProviderUnsupported => "spec.providerRef",
        }
    }

    const fn expected(self) -> &'static str {
        match self {
            Self::SelectorWildcard => "one exact named row",
            Self::SourceNotCredential => "a Credential source reference",
            Self::ConsumerUnsupported => "an admitted consumer component",
            Self::OperationUndeclared => "distinct declared delivery classes",
            Self::LifetimeUnbounded => "a lifetime inside the family bounds",
            Self::Malformed => "a canonical credential binding row",
            Self::ProviderUnsupported => CREDENTIAL_BINDING_PROVIDER_REF,
        }
    }

    const fn observed(self) -> &'static str {
        match self {
            Self::SelectorWildcard => "a wildcard selector",
            Self::SourceNotCredential => "a source of another ResourceType",
            Self::ConsumerUnsupported => "a consumer this family refuses",
            Self::OperationUndeclared => "an undeclared or duplicated class",
            Self::LifetimeUnbounded => "a lifetime outside the family bounds",
            Self::Malformed => "bytes that do not decode",
            Self::ProviderUnsupported => "another or absent providerRef",
        }
    }

    /// The structured detail for the refusal.
    ///
    /// Both compared sides are fixed phrases: the committed value that
    /// refused is named by the field, never echoed, so no consumer name, slot
    /// token, or policy value can ride a failure line.
    fn detail(self) -> FailureDetail {
        FailureDetail::at(self.stage()).comparison(FailureComparison::new(
            self.field(),
            self.expected(),
            self.observed(),
        ))
    }

    /// Name the committed invariant that refused, in stable form.
    ///
    /// This reads the committed bytes, not the typed contract, so a row the
    /// contract refuses can be reported against the invariant it broke. It
    /// never admits anything: the contract's own `new()` is what decides, and
    /// a row whose bytes satisfy every check here and still fail to parse is
    /// reported as malformed.
    fn diagnose(base: &[u8]) -> Option<Self> {
        let Ok(raw) = serde_json::from_slice::<serde_json::Value>(base) else {
            return Some(Self::Malformed);
        };
        let text = |field: &str| raw.get(field).and_then(serde_json::Value::as_str);

        // A wildcard names no exact row. One admitted delivery is one exact
        // source, one exact consumer, and one exact slot; a `*` or `?` in any
        // of them would turn the row into a pattern over rows this driver
        // never resolved, so it is refused before anything is looked up.
        for field in ["credentialRef", "executionRef", "slot"] {
            if text(field).is_some_and(|value| value.contains(['*', '?'])) {
                return Some(Self::SelectorWildcard);
            }
        }
        if let Some(source) = text("credentialRef") {
            let kind = source.split_once('/').map(|(kind, _)| kind);
            if kind != Some(BindingKind::Credential.source_resource_type()) {
                return Some(Self::SourceNotCredential);
            }
        }
        if let Some(consumer) = text("executionRef") {
            let admitted = consumer
                .split_once('/')
                .map(|(kind, _)| kind)
                .and_then(BindingConsumerKind::from_resource_type)
                .is_some_and(|kind| BindingKind::Credential.admits_consumer(kind));
            if !admitted {
                return Some(Self::ConsumerUnsupported);
            }
        }
        match raw.get("operations") {
            Some(serde_json::Value::Array(entries)) => {
                if entries.is_empty() || entries.len() > MAX_CREDENTIAL_OPERATIONS {
                    return Some(Self::OperationUndeclared);
                }
                let mut declared: Vec<&str> = Vec::with_capacity(entries.len());
                for entry in entries {
                    let Some(code) = entry.as_str() else {
                        return Some(Self::OperationUndeclared);
                    };
                    if !DELIVERY_OPERATION_CODES.contains(&code) || declared.contains(&code) {
                        return Some(Self::OperationUndeclared);
                    }
                    declared.push(code);
                }
            }
            Some(_) => return Some(Self::Malformed),
            None => {}
        }
        match raw.get("lifetimeMs").and_then(serde_json::Value::as_u64) {
            Some(lifetime_ms)
                if (MIN_CREDENTIAL_LIFETIME_MS..=MAX_CREDENTIAL_LIFETIME_MS)
                    .contains(&lifetime_ms) => {}
            Some(_) => return Some(Self::LifetimeUnbounded),
            None if raw.get("lifetimeMs").is_some() => return Some(Self::Malformed),
            None => {}
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Driver error and status
// ---------------------------------------------------------------------------

/// The closed set of ways one pass refuses, defers, or fails.
///
/// Every variant names one decision the pass made; the classification is the
/// retry contract and the stable code is what an operator reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialDriverErrorKind {
    /// The committed row declares a selector that names no exact row.
    SelectorWildcard,
    /// The committed row's source is not a `Credential`.
    SourceNotCredential,
    /// The committed row's consumer is not one this family admits.
    ConsumerUnsupported,
    /// The committed row declares an operation class outside the closed
    /// delivery vocabulary, an empty set, or a duplicate.
    OperationUndeclared,
    /// The committed row's lifetime is outside the family's bounds.
    LifetimeUnbounded,
    /// The committed bytes are not a canonical binding row at all.
    SpecMalformed,
    /// The row's committed source decision does not admit the right this
    /// family asks.
    RightsNotAdmitted,
    /// The row's committed source decision does not declare the delivery
    /// facet this family's realization depends on.
    FacetUnsupported,
    /// The row selects a Provider this driver does not own.
    ProviderUnsupported,
    /// The source `Credential` row is present but its owner uid differs from
    /// this row's owner: the manager would silently re-parent.
    OwnerMismatch,
    /// The row's source target resolves in another Zone.
    TargetCrossZone,
    /// The source `Credential` row is not observable, or is already deleting.
    SourceUnavailable,
    /// The source `Credential` row is present but its stored spec does not
    /// decode.
    SourceSpecInvalid,
    /// The destination component is not observable, or is already deleting.
    DestinationUnavailable,
    /// The delivery fence could not be built from the committed identities.
    PlanDerivation,
    /// A delivery, observation, or revocation effect failed.
    DeliveryEffect,
}

impl CredentialDriverErrorKind {
    /// The retry contract: absence and an unanswerable plane defer, a decision
    /// against the committed row is terminal, and an effect failure is the
    /// driver's own operational class.
    const fn class(self) -> FailureClass {
        match self {
            Self::SourceUnavailable | Self::DestinationUnavailable | Self::DeliveryEffect => {
                FailureClass::Retryable
            }
            Self::SelectorWildcard
            | Self::SourceNotCredential
            | Self::ConsumerUnsupported
            | Self::OperationUndeclared
            | Self::LifetimeUnbounded
            | Self::SpecMalformed
            | Self::RightsNotAdmitted
            | Self::FacetUnsupported
            | Self::ProviderUnsupported
            | Self::OwnerMismatch
            | Self::TargetCrossZone
            | Self::SourceSpecInvalid
            | Self::PlanDerivation => FailureClass::Terminal,
        }
    }

    /// The registered failure kind this classification reports.
    const fn failure_kind(self) -> FailureKind {
        match self {
            Self::SelectorWildcard
            | Self::SourceNotCredential
            | Self::ConsumerUnsupported
            | Self::OperationUndeclared
            | Self::LifetimeUnbounded
            | Self::SpecMalformed
            | Self::RightsNotAdmitted
            | Self::FacetUnsupported
            | Self::TargetCrossZone => FailureKinds::BINDING_SPEC_INVALID,
            Self::ProviderUnsupported => FailureKinds::BINDING_PROVIDER_UNSUPPORTED,
            Self::OwnerMismatch => FailureKinds::BINDING_OWNER_MISMATCH,
            Self::SourceUnavailable | Self::DestinationUnavailable => {
                FailureKinds::BINDING_PARENT_UNAVAILABLE
            }
            Self::SourceSpecInvalid => FailureKinds::BINDING_PARENT_SPEC_INVALID,
            Self::PlanDerivation => FailureKinds::BINDING_PLAN_DERIVATION_INVALID,
            Self::DeliveryEffect => FailureKinds::BINDING_SERVING_EFFECT_FAILED,
        }
    }

    /// The stable provider code this decision reports.
    const fn code(self) -> &'static str {
        match self {
            Self::SelectorWildcard => "credential-selector-wildcard",
            Self::SourceNotCredential => "credential-source-not-credential",
            Self::ConsumerUnsupported => "credential-consumer-unsupported",
            Self::OperationUndeclared => "credential-operation-undeclared",
            Self::LifetimeUnbounded => "credential-lifetime-unbounded",
            Self::SpecMalformed => "credential-spec-malformed",
            Self::RightsNotAdmitted => "credential-rights-not-admitted",
            Self::FacetUnsupported => "credential-delivery-facet-unsupported",
            Self::ProviderUnsupported => "credential-binding-provider-unsupported",
            Self::OwnerMismatch => "credential-binding-owner-mismatch",
            Self::TargetCrossZone => "credential-target-cross-zone",
            Self::SourceUnavailable => "credential-source-unavailable",
            Self::SourceSpecInvalid => "credential-source-spec-invalid",
            Self::DestinationUnavailable => "credential-destination-unavailable",
            Self::PlanDerivation => "credential-delivery-plan-invalid",
            Self::DeliveryEffect => "credential-delivery-effect-failed",
        }
    }

}

/// Typed driver failure, mapped onto the structured failure surface at the
/// erased boundary through [`ResourceDriver::classify_error`].
#[derive(Debug, Clone)]
pub(crate) struct CredentialDriverError {
    kind: CredentialDriverErrorKind,
    op: DriverOp,
    detail: FailureDetail,
}

impl CredentialDriverError {
    fn new(kind: CredentialDriverErrorKind, op: DriverOp) -> Self {
        Self {
            kind,
            op,
            detail: FailureDetail::new(),
        }
    }

    fn with_detail(mut self, detail: FailureDetail) -> Self {
        self.detail = detail;
        self
    }
}

impl core::fmt::Display for CredentialDriverError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.kind.code())
    }
}

impl std::error::Error for CredentialDriverError {}

/// Typed in-memory status projection (never persisted).
///
/// Every field is an identity, a counter, or a state, so the value is safe to
/// render in a driver log. The delivery itself, the audience, and the
/// credential identity stay in the effect port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialBindingDriverStatus {
    /// One pass's delivery state: the exact destination, the replay sequence
    /// of the live session, whether that session is live, and whether this
    /// pass established it.
    Delivery {
        /// The exact destination this pass resolved for the committed row.
        destination: ResourceKey,
        /// The replay sequence of the live session.
        sequence: u64,
        /// Whether a delivery was live when the pass finished.
        delivered: bool,
        /// Whether this pass established it, as opposed to adopting one.
        established: bool,
    },
    /// Recover adopted a live delivery at the committed destination without
    /// minting a second one.
    Adopted {
        /// The exact destination the adopted delivery was made to.
        destination: ResourceKey,
        /// The replay sequence the adopted session carries.
        sequence: u64,
    },
    /// A terminal admission rejection: the stable provider reason stays
    /// visible while the actor publishes the Failed phase.
    Rejected {
        /// The stable provider reason the refusal published.
        reason: &'static str,
    },
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Everything the plane must construct to instantiate this family's driver
/// factory for one zone.
pub struct CredentialBindingDriverArgs {
    /// The zone this driver's rows live in.
    pub zone: ZoneId,
    /// The daemon-supplied facet set the family's effects are built from: the
    /// delivery, the observation, the revocation, and the clock. The family
    /// never receives a daemon-built effect port.
    pub facets: CredentialBindingEffectFacets,
}

/// [`ResourceDriverFactory`] for the `CredentialBinding` resource type.
/// Construction is infallible by contract.
pub(crate) struct CredentialBindingDriverFactory {
    types: [ResourceTypeName; 1],
    args: CredentialBindingDriverArgs,
}

impl CredentialBindingDriverFactory {
    pub(crate) fn new(args: CredentialBindingDriverArgs) -> Self {
        Self {
            types: [ResourceTypeName::new(CREDENTIAL_BINDING_TYPE_NAME)],
            args,
        }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for CredentialBindingDriverFactory {
    fn resource_types(&self) -> &[ResourceTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(CredentialBindingDriver::new(
            self.args.zone.clone(),
            // The driver builds its effects from the declared facets; no
            // externally built port appears at this construction site.
            Arc::new(CredentialBindingEffectsService::new(
                self.args.facets.clone(),
            )),
        ))
    }
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

/// One CredentialBinding resource's driver.
#[derive(Clone)]
pub(crate) struct CredentialBindingDriver {
    zone: ZoneId,
    effects: Arc<dyn CredentialBindingDriverEffects>,
    /// New use is fenced from the durable deleting mark onward: a pass that
    /// started before the mark must not establish a delivery after it.
    fenced: bool,
    /// Targets this driver already registered a dependency watch on. One
    /// registration per target keeps the dependency edge that wakes the actor
    /// on dependency death or readiness without accumulating manager watch
    /// entries.
    watched: Vec<ResourceKey>,
}

impl CredentialBindingDriver {
    pub(crate) fn new(zone: ZoneId, effects: Arc<dyn CredentialBindingDriverEffects>) -> Self {
        Self {
            zone,
            effects,
            fenced: false,
            watched: Vec::new(),
        }
    }

    fn error(&self, kind: CredentialDriverErrorKind, op: DriverOp) -> CredentialDriverError {
        CredentialDriverError::new(kind, op)
    }

    /// Record one terminal admission rejection in this pass's in-memory status
    /// and return the typed failure: the actor publishes the Failed phase and
    /// the stable provider reason stays visible instead of collapsing into a
    /// generic error.
    fn rejected(
        &self,
        ctx: &mut ResourceContext,
        kind: CredentialDriverErrorKind,
        op: DriverOp,
    ) -> CredentialDriverError {
        ctx.set_status(CredentialBindingDriverStatus::Rejected { reason: kind.code() });
        self.error(kind, op)
    }

    /// The one seam every row read goes through.
    ///
    /// The envelope must decode, must name the provider that serves this
    /// family, and its base layer must parse as the contract's own row spec.
    /// A refused row is reported against the invariant it broke; the contract
    /// decides whether it is served at all.
    fn committed_row(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<CredentialBindingSpec, CredentialDriverError> {
        let envelope = ctx
            .spec::<BindingSpecEnvelope>()
            .map_err(|_| self.error(CredentialDriverErrorKind::SpecMalformed, op))?;
        match envelope.provider_ref.as_ref() {
            Some(provider_ref)
                if provider_ref.to_canonical_string() == CREDENTIAL_BINDING_PROVIDER_REF => {}
            _ => {
                return Err(self
                    .error(CredentialDriverErrorKind::ProviderUnsupported, op)
                    .with_detail(SpecRefusal::ProviderUnsupported.detail()));
            }
        }
        let spec = serde_json::from_slice::<CredentialBindingSpec>(
            &envelope.base.to_canonical_bytes(),
        )
        .map_err(|_| {
            let refusal = SpecRefusal::diagnose(&envelope.base.to_canonical_bytes())
                .unwrap_or(SpecRefusal::Malformed);
            self.error(refusal.kind(), op).with_detail(refusal.detail())
        })?;
        self.admitted(&spec, op)?;
        Ok(spec)
    }

    /// The row's committed source decision must admit what this family asks
    /// and declare the facet the delivery is realized through.
    ///
    /// A binding row is the source's decision about the relationship, not only
    /// the relationship: a row whose decision admits another right, or whose
    /// realization never claimed credential delivery, is a row this driver
    /// must not deliver on. The check reads committed facts only.
    fn admitted(
        &self,
        spec: &CredentialBindingSpec,
        op: DriverOp,
    ) -> Result<(), CredentialDriverError> {
        let decision = spec.source();
        if !decision
            .admitted_rights()
            .contains(&RequestedRights::Consume)
        {
            return Err(self
                .error(CredentialDriverErrorKind::RightsNotAdmitted, op)
                .with_detail(FailureDetail::at("spec/source-decision").comparison(
                    FailureComparison::new(
                        "source.admittedRights",
                        "consume",
                        "another right set",
                    ),
                )));
        }
        if !decision
            .realized_facets()
            .contains(&BindingRealizationFacet::CredentialDelivery)
        {
            return Err(self
                .error(CredentialDriverErrorKind::FacetUnsupported, op)
                .with_detail(FailureDetail::at("spec/source-decision").comparison(
                    FailureComparison::new(
                        "source.realizedFacets",
                        "credential-delivery",
                        "another facet set",
                    ),
                )));
        }
        Ok(())
    }

    /// The source `Credential` row this binding is owned by, and the identity
    /// its delivery is fenced against.
    ///
    /// The binding's declared `Credential` must be the row the manager reports
    /// as this resource's owner: a binding cannot silently change owner, and
    /// its owner target must resolve in this Zone.
    async fn source_row(
        &self,
        ctx: &mut ResourceContext,
        spec: &CredentialBindingSpec,
        op: DriverOp,
    ) -> Result<SourceTarget, CredentialDriverError> {
        let key = ResourceKey::new(
            self.zone.as_str(),
            BindingKind::Credential.source_resource_type(),
            spec.credential_ref().name().as_str(),
        );
        let row = match ctx.lookup(&key).await {
            RowLookup::Present { row, .. } => row,
            lookup => {
                // A non-present read defers: the row may simply not be
                // committed yet, and an unusable payload is not terminal by
                // itself.
                let mut detail = FailureDetail::at("source/lookup");
                if let Some(comparison) = lookup.failure_comparison("source.credential", "present")
                {
                    detail = detail.comparison(comparison);
                }
                if let Some(error) = lookup.error_detail() {
                    detail = detail.with_note(error);
                }
                if let RowLookup::Error {
                    plane,
                    detail: row_detail,
                } = &lookup
                {
                    tracing::warn!(
                        plane = ?plane,
                        key = %key,
                        detail = %row_detail,
                        "credential binding source row read answered with an unreadable row",
                    );
                }
                return Err(self
                    .error(CredentialDriverErrorKind::SourceUnavailable, op)
                    .with_detail(detail));
            }
        };
        if row.deleting {
            return Err(self
                .error(CredentialDriverErrorKind::SourceUnavailable, op)
                .with_detail(FailureDetail::at("source/state").comparison(
                    FailureComparison::new("source.state", "live", "deleting"),
                )));
        }
        // The owner fence: a child cannot silently change owner, and the
        // owner the manager resolved must be the exact `Credential` this row
        // names, in this Zone.
        if let Some(owner) = ctx.owner()
            && owner != &row.uid
        {
            return Err(self
                .error(CredentialDriverErrorKind::OwnerMismatch, op)
                .with_detail(FailureDetail::at("source/owner").comparison(
                    FailureComparison::new("source.ownerUid", uid_hex(owner), uid_hex(&row.uid)),
                )));
        }
        if let Some(owner) = ctx.owner_key().cloned() {
            if owner.zone != self.zone.as_str() {
                return Err(self
                    .error(CredentialDriverErrorKind::TargetCrossZone, op)
                    .with_detail(FailureDetail::at("source/zone").comparison(
                        FailureComparison::new("source.zone", self.zone.as_str(), owner.zone),
                    )));
            }
            if owner.type_name != key.type_name || owner.name != key.name {
                return Err(self
                    .error(CredentialDriverErrorKind::SourceNotCredential, op)
                    .with_detail(FailureDetail::at("source/owner").comparison(
                        FailureComparison::new(
                            "source.ownerRef",
                            key.to_string(),
                            owner.to_string(),
                        ),
                    )));
            }
        }
        // The source row must be a readable resource: a present row whose
        // stored bytes do not decode cannot be the committed policy a
        // delivery is admitted against.
        serde_json::from_slice::<ResourceSpec>(&row.spec).map_err(|_| {
            self.rejected(ctx, CredentialDriverErrorKind::SourceSpecInvalid, op)
                .with_detail(FailureDetail::at("source/decode").comparison(
                    FailureComparison::new("source.spec", "a canonical Credential row", "decode failed"),
                ))
        })?;
        Ok(SourceTarget {
            key,
            uid: self.uid(row.uid, "source.uid", op)?,
            generation: self.generation(row.generation, "source.generation", op)?,
        })
    }

    /// The exact destination one delivery may be made to, and the identity
    /// the delivery is fenced against.
    async fn destination_row(
        &self,
        ctx: &mut ResourceContext,
        spec: &CredentialBindingSpec,
        op: DriverOp,
    ) -> Result<DestinationTarget, CredentialDriverError> {
        let key = ResourceKey::new(
            self.zone.as_str(),
            spec.execution_ref().resource_type().as_str(),
            spec.execution_ref().name().as_str(),
        );
        let row = match ctx.lookup(&key).await {
            RowLookup::Present { row, .. } => row,
            lookup => {
                let mut detail = FailureDetail::at("destination/lookup");
                if let Some(comparison) =
                    lookup.failure_comparison("destination.execution", "present")
                {
                    detail = detail.comparison(comparison);
                }
                if let Some(error) = lookup.error_detail() {
                    detail = detail.with_note(error);
                }
                return Err(self
                    .error(CredentialDriverErrorKind::DestinationUnavailable, op)
                    .with_detail(detail));
            }
        };
        if row.deleting {
            return Err(self
                .error(CredentialDriverErrorKind::DestinationUnavailable, op)
                .with_detail(FailureDetail::at("destination/state").comparison(
                    FailureComparison::new("destination.state", "live", "deleting"),
                )));
        }
        Ok(DestinationTarget {
            key,
            uid: self.uid(row.uid, "destination.uid", op)?,
            generation: self.generation(row.generation, "destination.generation", op)?,
        })
    }

    /// A durable row identity, or a refusal when the stored bytes are not one.
    ///
    /// The delivery fence pins the source and destination identities, so a row
    /// whose stored identity is not a canonical resource uid cannot anchor a
    /// fence and is refused instead of being admitted under a repaired one.
    fn uid(&self, bytes: [u8; 16], field: &'static str, op: DriverOp) -> Result<ResourceUid, CredentialDriverError> {
        ResourceUid::from_bytes(&bytes).map_err(|_| {
            self.error(CredentialDriverErrorKind::PlanDerivation, op).with_detail(
                FailureDetail::at("plan/identity").comparison(FailureComparison::new(
                    field,
                    "a canonical resource uid",
                    "an unrepresentable identity",
                )),
            )
        })
    }

    /// A committed generation, or a refusal when the row carries none.
    ///
    /// The delivery fence pins the source and destination generations, so a
    /// row whose generation is not a committed one cannot anchor a fence and
    /// is refused instead of being admitted under generation zero.
    fn generation(
        &self,
        value: u64,
        field: &'static str,
        op: DriverOp,
    ) -> Result<ResourceGeneration, CredentialDriverError> {
        ResourceGeneration::new(value).map_err(|_| {
            self.error(CredentialDriverErrorKind::PlanDerivation, op).with_detail(
                FailureDetail::at("plan/generation").comparison(FailureComparison::new(
                    field,
                    "a committed generation",
                    "generation zero",
                )),
            )
        })
    }

    /// Build the exact delivery this row's committed spec admits, bounded by
    /// the observed clock.
    fn plan(
        &self,
        ctx: &ResourceContext,
        spec: &CredentialBindingSpec,
        source: &SourceTarget,
        destination: &DestinationTarget,
        now_unix_ms: u64,
    ) -> Result<CredentialDelivery, CredentialDriverError> {
        Ok(CredentialDelivery::new(
            ctx.key().clone(),
            source.key.clone(),
            source.uid.clone(),
            source.generation,
            destination.key.clone(),
            destination.uid.clone(),
            destination.generation,
            spec.slot().as_str(),
            spec.operations().iter().copied().map(operation_code).collect(),
            now_unix_ms.saturating_add(spec.lifetime_ms()),
        ))
    }

    /// The fence one observation of this row is published under.
    ///
    /// The manager has no separate Zone revision: its wire revision is the
    /// row generation, which is what a reader compares against.
    fn fence(
        &self,
        ctx: &ResourceContext,
        op: DriverOp,
    ) -> Result<CredentialBindingReadinessFence, CredentialDriverError> {
        let generation = self.generation(ctx.generation(), "binding.generation", op)?;
        Ok(CredentialBindingReadinessFence::new(
            self.uid(*ctx.uid(), "binding.uid", op)?,
            generation,
            ZoneRevision::new(generation.get()),
        ))
    }

    /// Register one dependency watch exactly once per target.
    ///
    /// Best-effort by design: a dependency that is still an unconverted row
    /// has no actor to watch yet, and the next pass re-evaluates it.
    async fn watch_once(&mut self, ctx: &mut ResourceContext, target: ResourceKey) {
        if self.watched.contains(&target) {
            return;
        }
        if ctx.watch(target.clone(), WatchCondition::Ready).await.is_ok() {
            self.watched.push(target);
        }
    }

    /// Revoke whatever delivery this row holds, at the exact destination its
    /// committed spec names when the row still decodes.
    ///
    /// Idempotent: a row that never held a delivery, or whose delivery is
    /// already retired, converges without an effect failure.
    async fn revoke_row(
        &self,
        ctx: &mut ResourceContext,
        op: DriverOp,
    ) -> Result<(), CredentialDriverError> {
        let committed = self.committed_row(ctx, op).ok();
        let revocation = match committed {
            Some(spec) => CredentialRevocation::at_destination(
                ctx.key(),
                &ResourceKey::new(
                    self.zone.as_str(),
                    spec.execution_ref().resource_type().as_str(),
                    spec.execution_ref().name().as_str(),
                ),
            ),
            // A row whose spec no longer decodes still holds a delivery: the
            // row-scoped revocation retires it without this driver guessing
            // which destination it went to.
            None => CredentialRevocation::for_row(ctx.key()),
        };
        self.effects
            .revoke(&revocation)
            .await
            .map_err(|error| {
                self.error(CredentialDriverErrorKind::DeliveryEffect, op).with_detail(
                    FailureDetail::at("revoke/effect")
                        .comparison(FailureComparison::new(
                            "delivery.revocation",
                            "retired",
                            "unconfirmed",
                        ))
                        .with_note(error),
                )
            })
    }

    /// Publish the fenced readiness projection for one observation.
    #[allow(clippy::too_many_arguments)]
    fn publish(
        &self,
        ctx: &mut ResourceContext,
        fence: CredentialBindingReadinessFence,
        state: BindingLifecycleState,
        delivered: bool,
        sequence: u64,
        expires_unix_ms: u64,
        reason: Option<StatusCode>,
    ) {
        let projection = CredentialBindingStatusResource::render(
            state,
            delivered,
            fence,
            sequence,
            expires_unix_ms,
            reason,
        );
        ctx.set_status_projection(projection);
    }
}

/// The hex spelling one compared uid renders as.
fn uid_hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The resolved source row one delivery is admitted against.
struct SourceTarget {
    key: ResourceKey,
    uid: ResourceUid,
    generation: ResourceGeneration,
}

/// The resolved destination row one delivery is made to.
struct DestinationTarget {
    key: ResourceKey,
    uid: ResourceUid,
    generation: ResourceGeneration,
}

#[async_trait::async_trait]
impl ResourceDriver for CredentialBindingDriver {
    type Error = CredentialDriverError;

    fn classify_error(&self, error: &CredentialDriverError) -> DriverFailure {
        let failure = match error.kind {
            CredentialDriverErrorKind::SelectorWildcard
            | CredentialDriverErrorKind::SourceNotCredential
            | CredentialDriverErrorKind::ConsumerUnsupported
            | CredentialDriverErrorKind::OperationUndeclared
            | CredentialDriverErrorKind::LifetimeUnbounded
            | CredentialDriverErrorKind::SpecMalformed
            | CredentialDriverErrorKind::RightsNotAdmitted
            | CredentialDriverErrorKind::FacetUnsupported
            | CredentialDriverErrorKind::ProviderUnsupported
            | CredentialDriverErrorKind::OwnerMismatch
            | CredentialDriverErrorKind::TargetCrossZone
            | CredentialDriverErrorKind::SourceSpecInvalid
            | CredentialDriverErrorKind::PlanDerivation => {
                DriverFailure::refused(error.op, error.kind.failure_kind())
            }
            CredentialDriverErrorKind::SourceUnavailable
            | CredentialDriverErrorKind::DestinationUnavailable => {
                DriverFailure::not_yet(error.op, error.kind.failure_kind())
            }
            CredentialDriverErrorKind::DeliveryEffect => {
                DriverFailure::error(error.op, error.kind.failure_kind(), error.kind.class())
            }
        };
        failure.with_detail(error.detail.clone())
    }

    /// Spec decode, serving Provider check, and the owner fence: the binding's
    /// declared `Credential` must be the row the manager reports as this
    /// resource's owner, resolved in this Zone.
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        let op = DriverOp::Validate;
        let spec = self.committed_row(ctx, op)?;
        self.source_row(ctx, &spec, op).await?;
        Ok(())
    }

    /// Delivery adoption: the pre-restart incarnation is adopted only when the
    /// destination holds a live session that serves this row's committed
    /// request. Recovery creates nothing - a missing or stale delivery is
    /// `Missing`, and the next reconcile establishes it.
    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        let op = DriverOp::Recover;
        let spec = self.committed_row(ctx, op)?;
        let source = self.source_row(ctx, &spec, op).await?;
        let destination = self.destination_row(ctx, &spec, op).await?;
        let now_unix_ms = self.effects.now_unix_ms();
        let delivery = self.plan(ctx, &spec, &source, &destination, now_unix_ms)?;
        let observed = self
            .effects
            .observe(&delivery)
            .await
            .map_err(|error| self.effect_error(op, "recover/observe", error))?;
        match observed {
            Some(session) if session.serves(&delivery, now_unix_ms) => {
                ctx.set_status(CredentialBindingDriverStatus::Adopted {
                    destination: destination.key.clone(),
                    sequence: session.sequence(),
                });
                self.publish(
                    ctx,
                    self.fence(ctx, op)?,
                    BindingLifecycleState::Active,
                    true,
                    session.sequence(),
                    session.expires_unix_ms(),
                    None,
                );
                Ok(RecoveryOutcome::Adopted)
            }
            _ => {
                // Nothing serves this row's committed request at this
                // destination: the next reconcile establishes a fresh
                // delivery rather than reusing the earlier one.
                ctx.set_status(CredentialBindingDriverStatus::Delivery {
                    destination: destination.key.clone(),
                    sequence: 0,
                    delivered: false,
                    established: false,
                });
                self.publish(
                    ctx,
                    self.fence(ctx, op)?,
                    BindingLifecycleState::Requested,
                    false,
                    0,
                    delivery.expires_unix_ms(),
                    not_established(),
                );
                Ok(RecoveryOutcome::Missing)
            }
        }
    }

    /// One reconcile pass: resolve the source and the destination, register
    /// the dependency edges, then establish the delivery.
    ///
    /// A live session that serves the committed request is adopted as is -
    /// prior delivery authority is never re-minted. A session that does not
    /// serve it (a stale window, or a delivery at a destination this row no
    /// longer claims) is revoked before a fresh one is established, so a row
    /// never holds two deliveries at once and never re-delivers into a
    /// destination other than its committed one.
    async fn reconcile(&mut self, ctx: &mut ResourceContext) -> Result<ReconcileOutcome, Self::Error> {
        let op = DriverOp::Reconcile;
        if self.fenced {
            // The durable deleting mark is already committed: this row admits
            // no new use, so the pass establishes nothing and reaches no
            // effect at all.
            return Err(self.effect_error(
                op,
                "reconcile/fence",
                "new use is fenced by the committed deletion".to_owned(),
            ));
        }
        let spec = self.committed_row(ctx, op)?;
        let source = self.source_row(ctx, &spec, op).await?;
        let destination = self.destination_row(ctx, &spec, op).await?;
        self.watch_once(ctx, source.key.clone()).await;
        self.watch_once(ctx, destination.key.clone()).await;
        let now_unix_ms = self.effects.now_unix_ms();
        let delivery = self.plan(ctx, &spec, &source, &destination, now_unix_ms)?;
        let observed = self
            .effects
            .observe(&delivery)
            .await
            .map_err(|error| self.effect_error(op, "reconcile/observe", error))?;
        let (session, established) = match observed {
            Some(session) if session.serves(&delivery, now_unix_ms) => (session, false),
            stale => {
                if let Some(stale) = stale {
                    // The delivery this row actually holds is retired before
                    // anything else is established, at the destination it was
                    // made to: a row never serves two destinations at once,
                    // and it never re-delivers into a destination its
                    // committed spec no longer names.
                    self.effects
                        .revoke(&CredentialRevocation::at_destination(
                            ctx.key(),
                            stale.destination(),
                        ))
                        .await
                        .map_err(|error| self.effect_error(op, "reconcile/revoke", error))?;
                }
                let session = self
                    .effects
                    .deliver(&delivery)
                    .await
                    .map_err(|error| self.effect_error(op, "reconcile/deliver", error))?;
                (session, true)
            }
        };
        ctx.set_status(CredentialBindingDriverStatus::Delivery {
            destination: destination.key.clone(),
            sequence: session.sequence(),
            delivered: true,
            established,
        });
        self.publish(
            ctx,
            self.fence(ctx, op)?,
            BindingLifecycleState::Active,
            true,
            session.sequence(),
            session.expires_unix_ms(),
            None,
        );
        // The authority this row established ends at its expiry, so the row is
        // re-examined at that edge: the next pass observes what the destination
        // still holds and re-admits a fresh delivery when the window closed.
        // It never re-delivers on a shorter timer, because a delivery that
        // renews itself is exactly the prior authority this contract refuses.
        let remaining = session.expires_unix_ms().saturating_sub(now_unix_ms);
        if remaining > 0 {
            ctx.requeue_after(Duration::from_millis(remaining));
        }
        Ok(ReconcileOutcome::Satisfied)
    }

    /// Pre-drain: fence new use, then retire what this row holds.
    ///
    /// The durable deleting mark is already committed when this runs, so the
    /// pass first refuses any further establishment and then revokes the
    /// delivery. It is a lifecycle stage, not a wait: it returns as soon as it
    /// has recorded what it did.
    async fn pre_drain(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        self.fenced = true;
        self.revoke_row(ctx, DriverOp::Delete).await
    }

    /// Drain step: revoke the delivery and prove it is retired.
    ///
    /// The revocation is idempotent, so a requeued pass re-reads its own state
    /// and converges; an adapter that cannot prove the revocation fails the
    /// pass retryably and keeps the durable deleting mark.
    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        self.revoke_row(ctx, DriverOp::Delete).await
    }

    /// Teardown: retire the delivery this row holds, at the exact destination
    /// its committed spec names. Idempotent under retry: a row that holds
    /// nothing, or whose spec no longer decodes, converges on the row-scoped
    /// revocation.
    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        self.revoke_row(ctx, DriverOp::Delete).await
    }
}

/// The stable reason a projection publishes when no delivery is live.
fn not_established() -> Option<StatusCode> {
    Some(
        StatusCode::parse("credential-delivery-not-established")
            .expect("a kebab-case provider code is a valid status code"),
    )
}

impl CredentialBindingDriver {
    /// One delivery effect that failed, classified retryably.
    fn effect_error(&self, op: DriverOp, stage: &'static str, note: String) -> CredentialDriverError {
        self.error(CredentialDriverErrorKind::DeliveryEffect, op).with_detail(
            FailureDetail::at(stage)
                .comparison(FailureComparison::new(
                    "delivery.effect",
                    "established",
                    "failed",
                ))
                .with_note(note),
        )
    }
}

// ---------------------------------------------------------------------------
// Registration: the type's driver declaration
// ---------------------------------------------------------------------------

/// The CredentialBinding type's driver declaration.
///
/// `CredentialBinding` is `BUILTIN | STARTUP` (no RUNTIME bit): the plane
/// cannot serve the converted binding shapes without it, so it must be
/// registered before the plane opens. The type is not exportable:
/// `ResourceExport` admits only qualified `*.d2bus.org.*Service` types, so a
/// binding can never be an export subject. The driver serves no broker
/// operations and contributes no startup steps; a delivery mints no child row
/// at all ([`CREDENTIAL_BINDING_CREATIONS`]), and the declaration carries the
/// family's declared effects service
/// ([`CREDENTIAL_BINDING_EFFECTS_SERVICE`]), which the daemon hosts per zone
/// from the family's registered factory.
pub fn binding_descriptor(args: CredentialBindingDriverArgs) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::CREDENTIAL_BINDING,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: CONVERTED_TYPE_VERBS,
        execution: CREDENTIAL_BINDING_EXECUTION_DOMAINS,
        exportable: false,
        reads: CREDENTIAL_BINDING_READS,
        operations: &[],
        creations: CREDENTIAL_BINDING_CREATIONS,
        startup: &[],
        services: &[CREDENTIAL_BINDING_EFFECTS_SERVICE],
        decoder: binding_spec_decoder(),
        factory: Arc::new(CredentialBindingDriverFactory::new(args)),
    }
}

// ---------------------------------------------------------------------------
// Tests: driver unit tests over the scripted delivery port and a recording
// manager endpoint, with one shared ordered log so a pass reads as one
// sequence across manager calls and delivery effects.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use d2b_provider_toolkit::testing::fakes::{RecordingManagerEndpoint, RecordingRequeue};
    use d2b_resource_runtime::error::FailureKinds;
    use d2b_resource_runtime::identity::ResourceProvenance;
    use d2b_resource_runtime::spec_store::StoredDesiredResource;

    use super::*;
    use crate::test_support::{FakeDeliveryEffects, SCRIPTED_MATERIAL};

    const ZONE: &str = "work";
    const BINDING_NAME: &str = "delivery";
    /// The lifetime every fixture row requests.
    const LIFETIME_MS: u64 = 60_000;
    /// The clock every fixture double starts at.
    const NOW_UNIX_MS: u64 = 1_760_000_000_000;

    // -- fixtures ------------------------------------------------------------

    fn binding_key() -> ResourceKey {
        ResourceKey::new(ZONE, CREDENTIAL_BINDING_TYPE_NAME, BINDING_NAME)
    }

    fn uid_of(byte: u8) -> ResourceUid {
        ResourceUid::from_bytes(&[byte; 16]).expect("a uniform byte array is a canonical uid")
    }

    /// The valid committed binding row, with the caller's overrides applied.
    fn binding_spec_bytes(overrides: serde_json::Value) -> Vec<u8> {
        let mut spec = serde_json::json!({
            "providerRef": CREDENTIAL_BINDING_PROVIDER_REF,
            "credentialRef": "Credential/data",
            "executionRef": "Guest/work-vm",
            "operations": ["acquire-token"],
            "lifetimeMs": LIFETIME_MS,
            "slot": "operator",
            "source": {
                "admittedRights": ["consume"],
                "arbitration": "shared",
                "realizedFacets": ["credential-delivery"]
            }
        });
        if let (Some(base), Some(overrides)) = (spec.as_object_mut(), overrides.as_object()) {
            for (field, value) in overrides {
                base.insert(field.clone(), value.clone());
            }
        }
        serde_json::to_vec(&spec).expect("the binding row serializes")
    }

    /// The source `Credential` row the binding declares and is owned by.
    ///
    /// The row itself owns nothing; the binding is the owned child, and the
    /// owner uid the binding row carries is the fence the driver checks.
    fn source_row() -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new(ZONE, "Credential", "data"),
            uid: [0x42; 16],
            generation: 3,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: serde_json::json!({
                "providerRef": "Provider/credential",
                "audience": "azure-resource-manager",
                "allowedOperations": ["acquire-token"]
            })
            .to_string()
            .into_bytes(),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    /// The consumer component row a delivery is made to.
    fn destination_row() -> StoredDesiredResource {
        StoredDesiredResource {
            key: ResourceKey::new(ZONE, "Guest", "work-vm"),
            uid: [0x24; 16],
            generation: 2,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: b"{}".to_vec(),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    /// The binding row itself.
    fn binding_row(overrides: serde_json::Value) -> StoredDesiredResource {
        StoredDesiredResource {
            key: binding_key(),
            uid: [0x11; 16],
            generation: 4,
            owner_uid: Some([0x42; 16]),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: binding_spec_bytes(overrides),
            metadata: Vec::new(),
            created_at: 0,
        }
    }

    /// A manager holding the source and destination rows a fixture needs.
    fn plane() -> RecordingManagerEndpoint {
        RecordingManagerEndpoint::new()
            .with_row(source_row())
            .with_row(destination_row())
    }

    struct Fixture {
        ctx: ResourceContext,
        manager: RecordingManagerEndpoint,
        requeue: RecordingRequeue,
    }

    fn fixture(row: StoredDesiredResource, manager: RecordingManagerEndpoint) -> Fixture {
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let requeue = RecordingRequeue::default();
        let ctx = ResourceContext::new(
            row,
            binding_spec_decoder(),
            Arc::new(manager.clone()),
            Arc::new(requeue.clone()),
            effects_tx,
            notify_tx,
        );
        Fixture {
            ctx,
            manager,
            requeue,
        }
    }

    async fn driver(effects: &Arc<FakeDeliveryEffects>) -> Box<dyn DynResourceDriver> {
        CredentialBindingDriverFactory::new(CredentialBindingDriverArgs {
            zone: ZoneId::parse(ZONE).expect("zone"),
            facets: effects.facet_set(),
        })
        .create(&binding_key())
        .await
    }

    // -- validate ------------------------------------------------------------

    /// A lifetime above the family's ceiling is refused by name: the row
    /// contract bounds it, and this driver reports which invariant answered.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_a_lifetime_above_the_family_bound() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let mut f = fixture(
            binding_row(serde_json::json!({"lifetimeMs": MAX_CREDENTIAL_LIFETIME_MS + 1})),
            manager,
        );
        let mut d = driver(&effects).await;

        let failure = d.validate(&mut f.ctx).await.expect_err("over-bound lifetime");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SPEC_INVALID);
        assert_eq!(
            failure.comparisons().first().map(|comparison| comparison.field()),
            Some("spec.lifetimeMs"),
            "the refusal names the invariant that answered: {}",
            failure.log_line()
        );
        assert!(
            effects.call_order().is_empty(),
            "a refused row establishes nothing"
        );
    }

    /// A lifetime below the family's floor is refused the same way.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_a_lifetime_below_the_family_floor() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let mut f = fixture(
            binding_row(serde_json::json!({"lifetimeMs": MIN_CREDENTIAL_LIFETIME_MS - 1})),
            manager,
        );
        let mut d = driver(&effects).await;

        let failure = d.validate(&mut f.ctx).await.expect_err("under-bound lifetime");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(
            failure.comparisons().first().map(|comparison| comparison.field()),
            Some("spec.lifetimeMs"),
            "{}",
            failure.log_line()
        );
    }

    /// An operation class outside the closed delivery vocabulary - here a
    /// Credential protocol operation, which has no delivery session at all -
    /// is refused rather than delivered under an approximation.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_an_undeclared_operation() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let mut f = fixture(
            binding_row(serde_json::json!({"operations": ["revoke-token"]})),
            manager,
        );
        let mut d = driver(&effects).await;

        let failure = d.validate(&mut f.ctx).await.expect_err("undeclared class");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(
            failure.comparisons().first().map(|comparison| comparison.field()),
            Some("spec.operations"),
            "{}",
            failure.log_line()
        );
    }

    /// An empty operation set, a duplicated class, and an over-bound set are
    /// the same refusal: a delivery grants exactly what it names.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_an_empty_or_duplicated_operation_set() {
        for operations in [
            serde_json::json!([]),
            serde_json::json!(["acquire-token", "acquire-token"]),
            serde_json::json!([
                "acquire-token",
                "refresh-token",
                "sign-challenge",
                "acquire-token",
                "refresh-token",
                "sign-challenge",
                "acquire-token",
                "refresh-token",
                "sign-challenge"
            ]),
        ] {
            let manager = plane();
            let effects = FakeDeliveryEffects::new();
            let mut f = fixture(
                binding_row(serde_json::json!({ "operations": operations })),
                manager,
            );
            let mut d = driver(&effects).await;
            let failure = d
                .validate(&mut f.ctx)
                .await
                .expect_err("the operation set is refused");
            assert_eq!(failure.class(), FailureClass::Terminal, "{operations}");
            assert_eq!(
                failure.comparisons().first().map(|comparison| comparison.field()),
                Some("spec.operations"),
                "{operations}: {}",
                failure.log_line()
            );
        }
    }

    /// A wildcard selector names no exact row: one admitted delivery is one
    /// exact source, consumer, and slot.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_a_wildcard_credential_selector() {
        for wildcard in ["*", "operator-*", "work-?"] {
            let manager = plane();
            let effects = FakeDeliveryEffects::new();
            let mut f = fixture(
                binding_row(serde_json::json!({ "slot": wildcard })),
                manager,
            );
            let mut d = driver(&effects).await;
            let failure = d
                .validate(&mut f.ctx)
                .await
                .expect_err("a wildcard slot is refused");
            assert_eq!(failure.class(), FailureClass::Terminal, "{wildcard}");
            assert_eq!(
                failure.comparisons().first().map(|comparison| comparison.field()),
                Some("binding.selector"),
                "{wildcard}: {}",
                failure.log_line()
            );
        }
    }

    /// A source that is not a `Credential` and a consumer this family does not
    /// admit are both terminal refusals naming their own invariant.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_a_wrong_source_and_an_unadmitted_consumer() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let mut f = fixture(
            binding_row(serde_json::json!({"credentialRef": "Volume/data"})),
            manager,
        );
        let mut d = driver(&effects).await;
        let failure = d.validate(&mut f.ctx).await.expect_err("wrong source");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(
            failure.comparisons().first().map(|comparison| comparison.field()),
            Some("spec.credentialRef"),
            "{}",
            failure.log_line()
        );

        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let mut f = fixture(
            binding_row(serde_json::json!({"executionRef": "Host/host-system"})),
            manager,
        );
        let mut d = driver(&effects).await;
        let failure = d.validate(&mut f.ctx).await.expect_err("Host consumer");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(
            failure.comparisons().first().map(|comparison| comparison.field()),
            Some("spec.executionRef"),
            "{}",
            failure.log_line()
        );
    }

    /// The row's committed source decision must admit the right this family
    /// asks and declare the facet the delivery is realized through: a row
    /// whose decision says otherwise is not delivered on.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_a_source_decision_that_admits_nothing() {
        for (decision, field, expected) in [
            (
                serde_json::json!({"admittedRights": [], "arbitration": "shared", "realizedFacets": ["credential-delivery"]}),
                "source.admittedRights",
                "consume",
            ),
            (
                serde_json::json!({"admittedRights": ["consume"], "arbitration": "shared", "realizedFacets": []}),
                "source.realizedFacets",
                "credential-delivery",
            ),
            (
                serde_json::json!({"admittedRights": ["consume"], "arbitration": "shared", "realizedFacets": ["filesystem-presentation"]}),
                "source.realizedFacets",
                "credential-delivery",
            ),
        ] {
            let manager = plane();
            let effects = FakeDeliveryEffects::new();
            let mut f = fixture(
                binding_row(serde_json::json!({ "source": decision })),
                manager,
            );
            let mut d = driver(&effects).await;
            let failure = d
                .validate(&mut f.ctx)
                .await
                .expect_err("the decision is refused");
            assert_eq!(failure.class(), FailureClass::Terminal, "{field}");
            let comparison = failure
                .comparisons()
                .first()
                .expect("the refused decision is compared");
            assert_eq!(comparison.field(), field, "{}", failure.log_line());
            assert_eq!(comparison.expected().to_string(), expected, "{}", failure.log_line());
        }
    }

    /// The envelope must decode and must name the provider that serves this
    /// family; anything else fails closed.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn decoded_row_refuses_an_undecodable_spec_and_an_unsupported_provider() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let mut row = binding_row(serde_json::json!({}));
        row.spec = b"not a binding envelope".to_vec();
        let mut f = fixture(row, manager);
        let mut d = driver(&effects).await;
        let failure = d.validate(&mut f.ctx).await.expect_err("undecodable");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SPEC_INVALID);

        for provider in ["Provider/credential", "Provider/volume-binding"] {
            let manager = plane();
            let effects = FakeDeliveryEffects::new();
            let mut f = fixture(
                binding_row(serde_json::json!({ "providerRef": provider })),
                manager,
            );
            let mut d = driver(&effects).await;
            let failure = d
                .validate(&mut f.ctx)
                .await
                .expect_err("another provider is refused");
            assert_eq!(failure.kind(), FailureKinds::BINDING_PROVIDER_UNSUPPORTED, "{provider}");
        }

        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let mut spec: serde_json::Value =
            serde_json::from_slice(&binding_row(serde_json::json!({})).spec).expect("row spec");
        spec.as_object_mut()
            .expect("row spec object")
            .remove("providerRef");
        let mut row = binding_row(serde_json::json!({}));
        row.spec = serde_json::to_vec(&spec).expect("row spec bytes");
        let mut f = fixture(row, manager);
        let mut d = driver(&effects).await;
        let failure = d.validate(&mut f.ctx).await.expect_err("absent provider");
        assert_eq!(failure.kind(), FailureKinds::BINDING_PROVIDER_UNSUPPORTED);
    }

    /// The owner fence: a binding whose declared `Credential` is not the row
    /// that owns it is terminal, while a source that is simply not committed
    /// yet defers retryably.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn an_unobservable_source_defers_while_an_owner_mismatch_stays_terminal() {
        let effects = FakeDeliveryEffects::new();

        // Not committed yet: the manager holds no Credential row.
        let manager = RecordingManagerEndpoint::new().with_row(destination_row());
        let mut f = fixture(binding_row(serde_json::json!({})), manager.clone());
        let mut d = driver(&effects).await;
        let failure = d.validate(&mut f.ctx).await.expect_err("absent source");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::BINDING_PARENT_UNAVAILABLE);

        // The manager cannot answer: the same retryable defer, never reported
        // as an absent source.
        manager.set_fail_reads(true);
        let failure = d.validate(&mut f.ctx).await.expect_err("unanswerable plane");
        assert_eq!(failure.class(), FailureClass::Retryable);
        manager.set_fail_reads(false);

        // Present, but the binding claims another uid as its owner: the
        // manager would silently re-parent, so the row is refused.
        let manager = RecordingManagerEndpoint::new()
            .with_row(source_row())
            .with_row(destination_row());
        let mut row = binding_row(serde_json::json!({}));
        row.owner_uid = Some([0x99; 16]);
        let mut f = fixture(row, manager);
        let mut d = driver(&effects).await;
        let failure = d.validate(&mut f.ctx).await.expect_err("owner mismatch");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_OWNER_MISMATCH);
    }

    /// The owner target must resolve in this Zone: a delivery never crosses a
    /// Zone boundary, whatever the row names.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn validate_refuses_an_owner_target_in_another_zone() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
        let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut ctx = ResourceContext::new(
            binding_row(serde_json::json!({})),
            binding_spec_decoder(),
            Arc::new(manager),
            Arc::new(RecordingRequeue::default()),
            effects_tx,
            notify_tx,
        )
        .with_owner_key(Some(ResourceKey::new(
            "other-zone",
            "Credential",
            "data",
        )));
        let mut d = driver(&effects).await;

        let failure = d.validate(&mut ctx).await.expect_err("cross-zone owner");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(
            failure.comparisons().first().map(|comparison| comparison.field()),
            Some("source.zone"),
            "{}",
            failure.log_line()
        );
    }

    /// A source row whose stored bytes do not decode is terminal, and its
    /// stable reason stays visible in the typed status.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn an_undecodable_source_row_is_terminal_and_keeps_its_reason() {
        let mut source = source_row();
        source.spec = b"not a credential spec".to_vec();
        let manager = RecordingManagerEndpoint::new()
            .with_row(source)
            .with_row(destination_row());
        let effects = FakeDeliveryEffects::new();
        let mut f = fixture(binding_row(serde_json::json!({})), manager);
        let mut d = driver(&effects).await;

        let failure = d.validate(&mut f.ctx).await.expect_err("undecodable source");
        assert_eq!(failure.class(), FailureClass::Terminal);
        assert_eq!(failure.kind(), FailureKinds::BINDING_PARENT_SPEC_INVALID);
        assert!(matches!(
            f.ctx.status::<CredentialBindingDriverStatus>(),
            Some(CredentialBindingDriverStatus::Rejected {
                reason: "credential-source-spec-invalid"
            })
        ));
    }

    // -- reconcile -----------------------------------------------------------

    /// One reconcile pass delivers to the exact destination, publishes the
    /// fenced readiness projection, and re-examines the row at the edge of the
    /// authority it established.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_delivers_to_the_exact_destination_and_publishes_the_fence() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let mut f = fixture(binding_row(serde_json::json!({})), manager.clone());
        let mut d = driver(&effects).await;

        assert_eq!(
            d.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(
            effects.call_order(),
            vec![
                "observe:work/Guest/work-vm".to_owned(),
                "deliver:work/Guest/work-vm".to_owned(),
            ],
            "one observation, one delivery, at the committed destination"
        );
        assert_eq!(effects.deliveries(), 1);
        assert_eq!(
            f.manager
                .rows()
                .iter()
                .map(|row| format!("{}/{}", row.key.type_name, row.key.name))
                .collect::<Vec<_>>(),
            vec!["Credential/data".to_owned(), "Guest/work-vm".to_owned()],
            "a delivery mints no child row: the committed set is the two rows it read"
        );

        match f.ctx.status::<CredentialBindingDriverStatus>() {
            Some(CredentialBindingDriverStatus::Delivery {
                destination,
                sequence,
                delivered,
                established,
            }) => {
                assert_eq!(destination, &ResourceKey::new(ZONE, "Guest", "work-vm"));
                assert_eq!(*sequence, 1);
                assert!(*delivered && *established);
            }
            other => panic!("expected Delivery, got {other:?}"),
        }

        let projection = f
            .ctx
            .take_status_projection()
            .expect("the pass publishes the fenced projection");
        let status = CredentialBindingStatusResource::parse(&projection)
            .expect("the projection is this contract's");
        assert_eq!(status.state(), "active");
        assert!(status.delivered());
        assert_eq!(status.sequence(), 1);
        assert_eq!(status.expires_unix_ms(), NOW_UNIX_MS + LIFETIME_MS);
        assert!(status.reason().is_none());
        assert_eq!(
            status.fence().generation().get(),
            f.ctx.generation(),
            "the fence names the row's own generation"
        );
        assert!(status.readiness_is_current(
            &uid_of(0x11),
            ResourceGeneration::new(f.ctx.generation()).expect("generation"),
            ZoneRevision::new(f.ctx.generation())
        ));

        assert_eq!(
            f.requeue.scheduled(),
            vec![Duration::from_millis(LIFETIME_MS)],
            "the row is re-examined when the authority it established ends"
        );
    }

    /// Nothing the driver publishes carries credential material, and the row
    /// it committed no child carrying any either.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn no_published_surface_carries_credential_material() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let mut f = fixture(binding_row(serde_json::json!({})), manager.clone());
        let mut d = driver(&effects).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        d.delete(&mut f.ctx).await.expect("delete");

        let surfaces = [
            effects.call_order().join(" "),
            f.manager.call_order().join(" "),
            format!(
                "{:?}",
                f.ctx.status::<CredentialBindingDriverStatus>()
            ),
            f.ctx
                .take_status_projection()
                .map(|projection| projection.to_string())
                .unwrap_or_default(),
        ];
        for surface in surfaces {
            assert!(
                !surface.contains(effects.material()),
                "credential material reached a published surface: {surface}"
            );
            assert!(
                !surface.contains(SCRIPTED_MATERIAL),
                "credential material reached a published surface: {surface}"
            );
        }
        for row in f.manager.rows() {
            let rendered = String::from_utf8_lossy(&row.spec);
            assert!(
                !rendered.contains(SCRIPTED_MATERIAL),
                "credential material reached a committed row: {rendered}"
            );
        }
    }

    /// A delivery already live at the committed destination is adopted, not
    /// minted again: prior delivery authority is never re-issued.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_adopts_a_live_delivery_without_minting_a_second_one() {
        let session = DeliveredSession::new(
            ResourceKey::new(ZONE, "Guest", "work-vm"),
            uid_of(0x42),
            uid_of(0x24),
            ResourceGeneration::new(3).expect("generation"),
            ResourceGeneration::new(2).expect("generation"),
            7,
            NOW_UNIX_MS + LIFETIME_MS,
        );
        let manager = plane();
        let effects = FakeDeliveryEffects::holding(session.clone());
        let mut f = fixture(binding_row(serde_json::json!({})), manager);
        let mut d = driver(&effects).await;

        assert_eq!(
            d.recover(&mut f.ctx).await.expect("recover"),
            RecoveryOutcome::Adopted,
            "the pre-restart delivery serves this row's committed request"
        );
        assert_eq!(effects.deliveries(), 0, "adoption mints nothing");
        match f.ctx.status::<CredentialBindingDriverStatus>() {
            Some(CredentialBindingDriverStatus::Adopted {
                destination,
                sequence,
            }) => {
                assert_eq!(destination, &ResourceKey::new(ZONE, "Guest", "work-vm"));
                assert_eq!(*sequence, 7);
            }
            other => panic!("expected Adopted, got {other:?}"),
        }

        assert_eq!(
            d.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        assert_eq!(effects.deliveries(), 0, "a served delivery is reused");
        assert_eq!(effects.revocations(), 0, "a served delivery is not retired");
        assert_eq!(
            effects.call_order(),
            vec![
                "observe:work/Guest/work-vm".to_owned(),
                "observe:work/Guest/work-vm".to_owned(),
            ],
            "both passes observed; neither delivered"
        );
    }

    /// A delivery this row holds at a destination its committed spec no longer
    /// names is retired before the committed one is established: the row never
    /// serves two destinations at once.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_retires_a_delivery_at_another_destination_before_it_delivers() {
        let stale = DeliveredSession::new(
            ResourceKey::new(ZONE, "Guest", "other-vm"),
            uid_of(0x42),
            uid_of(0x25),
            ResourceGeneration::new(3).expect("generation"),
            ResourceGeneration::new(2).expect("generation"),
            3,
            NOW_UNIX_MS + LIFETIME_MS,
        );
        let manager = plane();
        let effects = FakeDeliveryEffects::holding(stale);
        let mut f = fixture(binding_row(serde_json::json!({})), manager);
        let mut d = driver(&effects).await;

        // Recover adopts nothing: the delivery in flight is not this row's
        // committed one.
        assert_eq!(
            d.recover(&mut f.ctx).await.expect("recover"),
            RecoveryOutcome::Missing
        );
        assert_eq!(effects.deliveries(), 0, "recover delivers nothing");
        assert!(matches!(
            f.ctx.status::<CredentialBindingDriverStatus>(),
            Some(CredentialBindingDriverStatus::Delivery {
                delivered: false,
                ..
            })
        ));

        assert_eq!(
            d.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        let order = effects.call_order();
        let revoke = order
            .iter()
            .position(|entry| entry == "revoke:work/Guest/other-vm")
            .unwrap_or_else(|| panic!("the stale delivery is retired: {order:?}"));
        let deliver = order
            .iter()
            .position(|entry| entry == "deliver:work/Guest/work-vm")
            .unwrap_or_else(|| panic!("the committed destination is served: {order:?}"));
        assert!(revoke < deliver, "retire before establishing: {order:?}");
    }

    /// A delivery whose window has closed is retired and re-admitted from
    /// current evidence: the earlier authority is never reused.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn reconcile_retires_an_expired_delivery_before_it_re_admits() {
        let expired = DeliveredSession::new(
            ResourceKey::new(ZONE, "Guest", "work-vm"),
            uid_of(0x42),
            uid_of(0x24),
            ResourceGeneration::new(3).expect("generation"),
            ResourceGeneration::new(2).expect("generation"),
            2,
            NOW_UNIX_MS - 1,
        );
        let manager = plane();
        let effects = FakeDeliveryEffects::holding(expired);
        let mut f = fixture(binding_row(serde_json::json!({})), manager);
        let mut d = driver(&effects).await;

        assert_eq!(
            d.recover(&mut f.ctx).await.expect("recover"),
            RecoveryOutcome::Missing,
            "an elapsed window admits nothing"
        );
        assert_eq!(
            d.reconcile(&mut f.ctx).await.expect("reconcile"),
            ReconcileOutcome::Satisfied
        );
        let order = effects.call_order();
        assert!(
            order.contains(&"revoke:work/Guest/work-vm".to_owned()),
            "{order:?}"
        );
        assert!(order.contains(&"deliver:work/Guest/work-vm".to_owned()), "{order:?}");
        assert_eq!(effects.deliveries(), 1, "exactly one fresh admission");
    }

    /// Recovery creates nothing: a destination that holds nothing is `Missing`
    /// and the pass that follows establishes the delivery.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn recover_reports_missing_when_the_destination_holds_nothing() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let mut f = fixture(binding_row(serde_json::json!({})), manager);
        let mut d = driver(&effects).await;

        assert_eq!(d.recover(&mut f.ctx).await.expect("recover"), RecoveryOutcome::Missing);
        assert_eq!(effects.deliveries(), 0, "recover delivers nothing");
        assert_eq!(
            effects.call_order(),
            vec!["observe:work/Guest/work-vm".to_owned()],
            "recovery only observes"
        );
        let projection = f
            .ctx
            .take_status_projection()
            .expect("recovery republishes the projection");
        let status = CredentialBindingStatusResource::parse(&projection).expect("typed");
        assert!(!status.delivered(), "nothing is delivered, and it says so");
        assert_eq!(
            status.reason().map(StatusCode::as_str),
            Some("credential-delivery-not-established")
        );
        assert!(!status.readiness_is_current(
            &uid_of(0x11),
            ResourceGeneration::new(f.ctx.generation()).expect("generation"),
            ZoneRevision::new(f.ctx.generation())
        ));
    }

    /// A destination that is not observable yet, or that is already deleting,
    /// defers retryably: the row waits rather than failing.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn an_unobservable_or_deleting_destination_defers_retryably() {
        let effects = FakeDeliveryEffects::new();

        // Not committed yet.
        let manager = RecordingManagerEndpoint::new().with_row(source_row());
        let mut f = fixture(binding_row(serde_json::json!({})), manager);
        let mut d = driver(&effects).await;
        let failure = d.reconcile(&mut f.ctx).await.expect_err("absent destination");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::BINDING_PARENT_UNAVAILABLE);
        assert_eq!(effects.deliveries(), 0, "nothing is delivered to nowhere");

        // Present, but on its way out.
        let mut deleting = destination_row();
        deleting.deleting = true;
        let manager = RecordingManagerEndpoint::new()
            .with_row(source_row())
            .with_row(deleting);
        let mut f = fixture(binding_row(serde_json::json!({})), manager);
        let mut d = driver(&effects).await;
        let failure = d.reconcile(&mut f.ctx).await.expect_err("deleting destination");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(effects.deliveries(), 0);
    }

    /// An effect the adapter could not complete defers retryably, and a row
    /// whose delivery failed is never published as delivered.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn a_failed_delivery_or_revocation_defers_retryably() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        effects.set_fail_deliver(true);
        let mut f = fixture(binding_row(serde_json::json!({})), manager);
        let mut d = driver(&effects).await;

        let failure = d.reconcile(&mut f.ctx).await.expect_err("delivery failed");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SERVING_EFFECT_FAILED);
        assert_eq!(effects.deliveries(), 0, "a failed delivery mints nothing");
        assert!(
            f.ctx.take_status_projection().is_none(),
            "a pass that established nothing publishes no readiness"
        );

        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        effects.set_fail_revoke(true);
        let mut f = fixture(binding_row(serde_json::json!({})), manager);
        let mut d = driver(&effects).await;
        let failure = d.delete(&mut f.ctx).await.expect_err("revocation failed");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SERVING_EFFECT_FAILED);
    }

    /// An unanswerable observation is never read as a revoked credential: the
    /// pass defers instead of establishing a second delivery.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn an_unanswerable_observation_defers_instead_of_re_delivering() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        effects.set_fail_observe(true);
        let mut f = fixture(binding_row(serde_json::json!({})), manager);
        let mut d = driver(&effects).await;

        let failure = d.reconcile(&mut f.ctx).await.expect_err("observation failed");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(failure.kind(), FailureKinds::BINDING_SERVING_EFFECT_FAILED);
        assert_eq!(
            effects.deliveries(),
            0,
            "an unanswered question never becomes a second delivery"
        );
    }

    // -- drain and teardown --------------------------------------------------

    /// Teardown revokes the delivery at the exact destination the row names,
    /// and a second pass converges without an effect failure.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn delete_revokes_idempotently() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let mut f = fixture(binding_row(serde_json::json!({})), manager);
        let mut d = driver(&effects).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");

        d.delete(&mut f.ctx).await.expect("first teardown");
        d.delete(&mut f.ctx).await.expect("second teardown");
        d.finalize(&mut f.ctx).await.expect("drain after teardown");

        let revocations = effects
            .call_order()
            .into_iter()
            .filter(|entry| entry.starts_with("revoke:"))
            .collect::<Vec<_>>();
        assert!(
            !revocations.is_empty(),
            "the teardown retired the delivery it held"
        );
        for entry in &revocations {
            assert_eq!(entry, "revoke:work/Guest/work-vm", "{revocations:?}");
        }
    }

    /// A row whose spec no longer decodes still holds a delivery: the teardown
    /// retires it against the row rather than guessing a destination.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn delete_revokes_a_row_whose_spec_no_longer_decodes() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let mut f = fixture(binding_row(serde_json::json!({})), manager);
        let mut d = driver(&effects).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile");
        let mut broken = binding_row(serde_json::json!({}));
        broken.spec = b"not a binding row".to_vec();
        f.ctx = fixture(broken, f.manager.clone()).ctx;

        d.delete(&mut f.ctx).await.expect("teardown");
        assert!(
            effects
                .call_order()
                .iter()
                .any(|entry| entry == "revoke-row:work/CredentialBinding/delivery"),
            "{:?}",
            effects.call_order()
        );
    }

    /// The pre-drain fences new use before anything is torn down: a pass that
    /// started before the durable deleting mark establishes nothing after it.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn pre_drain_fences_new_use_before_the_teardown() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let mut f = fixture(binding_row(serde_json::json!({})), manager);
        let mut d = driver(&effects).await;

        d.pre_drain(&mut f.ctx).await.expect("pre-drain");
        assert_eq!(effects.revocations(), 1, "the drain retired the delivery");
        let after_drain = effects.call_order().len();

        let failure = d
            .reconcile(&mut f.ctx)
            .await
            .expect_err("a deleting row admits no new use");
        assert_eq!(failure.class(), FailureClass::Retryable);
        assert_eq!(effects.deliveries(), 0, "nothing is delivered after the mark");
        assert_eq!(
            effects.call_order().len(),
            after_drain,
            "the fenced pass reaches no effect at all"
        );
    }

    // -- dependency edges ----------------------------------------------------

    /// The dependency edges are registered once per target: the source
    /// `Credential` row and the consumer component row.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn dependency_watches_are_registered_once_per_target() {
        let manager = plane();
        let effects = FakeDeliveryEffects::new();
        let mut f = fixture(binding_row(serde_json::json!({})), manager);
        let mut d = driver(&effects).await;
        d.reconcile(&mut f.ctx).await.expect("reconcile one");
        d.reconcile(&mut f.ctx).await.expect("reconcile two");

        let targets = f
            .manager
            .watch_targets()
            .into_iter()
            .map(|key| format!("{}/{}", key.type_name, key.name))
            .collect::<Vec<_>>();
        let registered = targets.len();
        let mut unique = targets.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(registered, unique.len(), "no watch is registered twice");
        assert!(targets.contains(&"Credential/data".to_owned()));
        assert!(targets.contains(&"Guest/work-vm".to_owned()));
    }

    /// The delivery vocabulary holds its own line: the request carries identity,
    /// vocabulary, counters, and bounds, and its rendering redacts the durable
    /// identities and the slot.
    #[test]
    fn the_delivery_request_renders_only_non_secret_fields() {
        let delivery = CredentialDelivery::new(
            binding_key(),
            ResourceKey::new(ZONE, "Credential", "data"),
            uid_of(0x42),
            ResourceGeneration::new(3).expect("generation"),
            ResourceKey::new(ZONE, "Guest", "work-vm"),
            uid_of(0x24),
            ResourceGeneration::new(2).expect("generation"),
            "operator",
            vec![ACQUIRE_TOKEN, REFRESH_TOKEN],
            NOW_UNIX_MS + LIFETIME_MS,
        );
        assert_eq!(delivery.operations(), [ACQUIRE_TOKEN, REFRESH_TOKEN]);
        assert_eq!(delivery.slot(), "operator");
        assert_eq!(delivery.expires_unix_ms(), NOW_UNIX_MS + LIFETIME_MS);

        let rendered = format!("{delivery:?}");
        for redacted in ["operator", "0x42", "Credential/data"] {
            assert!(
                !rendered.contains(redacted),
                "the request's rendering carries {redacted}: {rendered}"
            );
        }
        assert!(rendered.contains(ACQUIRE_TOKEN), "{rendered}");
    }

    /// The published reason codes are stable kebab-case codes the status layer
    /// accepts, not free text.
    #[test]
    fn every_provider_reason_is_a_valid_status_code() {
        for kind in [
            CredentialDriverErrorKind::SelectorWildcard,
            CredentialDriverErrorKind::SourceNotCredential,
            CredentialDriverErrorKind::ConsumerUnsupported,
            CredentialDriverErrorKind::OperationUndeclared,
            CredentialDriverErrorKind::LifetimeUnbounded,
            CredentialDriverErrorKind::SpecMalformed,
            CredentialDriverErrorKind::RightsNotAdmitted,
            CredentialDriverErrorKind::FacetUnsupported,
            CredentialDriverErrorKind::ProviderUnsupported,
            CredentialDriverErrorKind::OwnerMismatch,
            CredentialDriverErrorKind::TargetCrossZone,
            CredentialDriverErrorKind::SourceUnavailable,
            CredentialDriverErrorKind::SourceSpecInvalid,
            CredentialDriverErrorKind::DestinationUnavailable,
            CredentialDriverErrorKind::PlanDerivation,
            CredentialDriverErrorKind::DeliveryEffect,
        ] {
            assert!(
                StatusCode::parse(kind.code()).is_ok(),
                "{} is not a stable status code",
                kind.code()
            );
        }
    }
}
