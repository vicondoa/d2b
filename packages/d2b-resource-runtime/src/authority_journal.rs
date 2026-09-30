//! Durable desired revisions and recoverable publication transactions
//! (plan unit U5, KTD5-KTD6).
//!
//! # Why a journal and not a second generation counter
//!
//! The spec generation answers "did the rendered spec change?". Authority
//! does not move with the spec: an ownership change, a source-view change, a
//! provider-assignment change, or an execution-policy change can leave the
//! rendered bytes byte-identical while invalidating every effect authority
//! an earlier commit granted (R35, AE16). So every committed desired
//! mutation - spec, owner, metadata, or deletion - advances a durable
//! per-row [`DesiredRevision`], and an identical ensure advances nothing.
//! Runtime status is not a desired mutation at all and has no write path in
//! this store, so it cannot advance anything.
//!
//! The Zone-wide [`ZoneDesiredSequence`] orders those mutations within one
//! store incarnation and is what a broker fences its accepted projection
//! against. Both counters fail closed at their ceiling rather than wrapping
//! (see [`crate::revision`]).
//!
//! # The freeze / commit / publish / acknowledge transaction
//!
//! One mutation travels through four durable states, and the record that
//! carries it is the recovery identity between the store's own transactions:
//!
//! 1. **staged** - [`stage_mutation`] persists the exact candidate and
//!    reserves its Zone sequence. Nothing desired has moved.
//! 2. **prepared** - [`record_prepared`] records the broker's prepared
//!    transaction identity, which means the broker has durably frozen the
//!    Zone's new-effect admission for this candidate.
//! 3. **committed** - [`commit_mutation`] applies the staged candidate to
//!    the desired rows in *one* SQLite transaction together with the
//!    per-row revisions, the audit record, the publication outbox entry, and
//!    the Zone sequence. There is no code path that writes a desired row
//!    outside this transaction, so no open transaction, store lock, or
//!    reservation lock can survive into the caller's broker I/O.
//! 4. **accepted** - [`acknowledge`] records the broker's acknowledged
//!    revision, drops the outbox entry, and moves the accepted cursor.
//!
//! A staged candidate is stored rather than re-supplied, so commit is a pure
//! state transition: replaying it after any failure boundary re-derives the
//! same bytes and never applies a mutation twice. [`zone_recovery`] maps
//! every outstanding transaction onto the plan's Mutation recovery table so
//! a restart decides explicitly rather than guessing.
//!
//! # Counters, not credentials
//!
//! Desired digests, revisions, sequences, and transaction identities are
//! non-secret freshness data: they identify which bytes were committed and
//! in which store generation, never the right to act on them. Nothing here
//! is a bearer credential, and the store never hands the broker authority -
//! it hands it the exact committed projection to validate.

use std::sync::atomic::{AtomicU64, Ordering};

use d2b_contracts_resource::v3::authority::{
    DESIRED_ROW_DIGEST_DOMAIN_TAG, DesiredDigest, DesiredRevision, StoreIncarnation,
    ZoneDesiredSequence,
};
use d2b_contracts_resource::v3::resource_schema::canonical_digest;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use crate::identity::TransactionId;
use crate::revision::{
    decode_persisted_counter, desired_revision_from_counter, encode_persisted_counter,
    next_desired_revision, next_zone_sequence, zone_sequence_from_counter,
};
use crate::schema::SchemaError;
use crate::spec_store::{
    AuditWrite, ResourceKey, ResourceProvenance, SpecSelector, SpecStoreError,
    StoredDesiredResource, decode_desired_row, desired_row_tuple, insert_audit, now,
};

/// The module declared name.
pub const MODULE_NAME: &str = "authority_journal";

/// Domain tag binding a derived transaction identity to this protocol, so
/// the same bytes can never be re-read as a different kind of identity.
const TRANSACTION_ID_DOMAIN_TAG: &str = "d2b:v3:authority-transaction";

/// Domain tag separating a minted store incarnation from every other digest
/// this tree computes.
const STORE_INCARNATION_DOMAIN_TAG: &str = "d2b:v3:store-incarnation";

/// Largest candidate or payload the store will read back. A blob larger than
/// this cannot be a spec store row, so decoding refuses it instead of
/// allocating on a corrupt length prefix.
const MAX_JOURNAL_BLOB_BYTES: usize = 64 * 1024 * 1024;

/// The non-secret digest of canonical bytes.
///
/// The shared row-digest domain tag is used for both the candidate digest and
/// the committed-row digest, so every digest in the journal is framed the
/// same way and neither can be confused with the other. The digest names
/// which bytes were committed; it is never the right to act on them.
fn digest_of(canonical_bytes: &[u8]) -> DesiredDigest {
    DesiredDigest::parse(canonical_digest(DESIRED_ROW_DIGEST_DOMAIN_TAG, canonical_bytes))
        .expect("canonical_digest emits the canonical digest spelling")
}

// ---------------------------------------------------------------------------
// The staged candidate
// ---------------------------------------------------------------------------

/// One desired mutation, staged durably before it can be committed.
///
/// Every desired mutation uses this path, including ones that change nothing
/// the rendered spec shows: the plan's conservative rule is that a mutation
/// is classified only after it has been validated, never by guessing that it
/// is harmless.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesiredMutation {
    /// Create or update one desired row. Spec bytes, ownership, metadata,
    /// and provenance all arrive here; whether they differ from the
    /// committed row decides whether the revision advances.
    Ensure(StoredDesiredResource),
    /// Set the row's terminal deleting mark.
    MarkDeleting(ResourceKey),
    /// Retire the row after its cleanup finished.
    Remove(ResourceKey),
}

impl DesiredMutation {
    /// The Zone this mutation belongs to.
    pub fn zone(&self) -> &str {
        match self {
            Self::Ensure(row) => &row.key.zone,
            Self::MarkDeleting(key) | Self::Remove(key) => &key.zone,
        }
    }

    /// The exact row this mutation addresses.
    pub fn key(&self) -> &ResourceKey {
        match self {
            Self::Ensure(row) => &row.key,
            Self::MarkDeleting(key) | Self::Remove(key) => key,
        }
    }

    fn tag(&self) -> u8 {
        match self {
            Self::Ensure(_) => 0,
            Self::MarkDeleting(_) => 1,
            Self::Remove(_) => 2,
        }
    }

    /// Canonical bytes naming this candidate.
    ///
    /// The digest of these bytes is the transaction's candidate digest, so
    /// two mutations that name the same row change produce the same digest
    /// and a mutation that changes any committed field does not.
    fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Canonical::new();
        out.u8(self.tag());
        out.text(&self.key().zone);
        out.text(&self.key().type_name);
        out.text(&self.key().name);
        if let Self::Ensure(row) = self {
            out.bytes(row.uid.as_slice());
            out.u8(provenance_tag(row.provenance));
            out.optional_bytes(row.owner_uid.as_ref().map(|uid| uid.as_slice()));
            out.bytes(&row.spec);
            out.bytes(&row.metadata);
        }
        out.finish()
    }

    /// The non-secret digest naming this candidate.
    pub fn candidate_digest(&self) -> DesiredDigest {
        digest_of(&self.canonical_bytes())
    }

    fn encode(&self) -> Vec<u8> {
        self.canonical_bytes()
    }

    fn decode(bytes: &[u8]) -> Result<Self, SpecStoreError> {
        let mut reader = CanonicalReader::new(bytes);
        let tag = reader.u8()?;
        let zone = reader.text()?;
        let type_name = reader.text()?;
        let name = reader.text()?;
        let key = ResourceKey::new(zone.clone(), type_name, name);
        match tag {
            0 => {
                let uid = reader.fixed::<16>()?;
                let provenance = provenance_from_tag(reader.u8()?);
                let owner_uid = reader.optional_fixed::<16>()?;
                let spec = reader.bytes()?.to_vec();
                let metadata = reader.bytes()?.to_vec();
                Ok(Self::Ensure(StoredDesiredResource {
                    key,
                    uid,
                    // The store assigns both: they are outputs of the
                    // committed mutation, not inputs to it.
                    generation: 0,
                    owner_uid,
                    provenance,
                    deleting: false,
                    spec,
                    metadata,
                    created_at: 0,
                }))
            }
            1 => Ok(Self::MarkDeleting(key)),
            2 => Ok(Self::Remove(key)),
            _ => Err(SpecStoreError::CorruptJournalPayload { detail: "unknown mutation tag" }),
        }
    }
}

fn provenance_tag(provenance: ResourceProvenance) -> u8 {
    match provenance {
        ResourceProvenance::Nix => 0,
        ResourceProvenance::Api => 1,
        ResourceProvenance::Resource => 2,
    }
}

fn provenance_from_tag(tag: u8) -> ResourceProvenance {
    match tag {
        0 => ResourceProvenance::Nix,
        2 => ResourceProvenance::Resource,
        _ => ResourceProvenance::Api,
    }
}

// ---------------------------------------------------------------------------
// Committed rows
// ---------------------------------------------------------------------------

/// One committed desired row, with the revision and digest it committed at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredRow {
    pub row: StoredDesiredResource,
    pub revision: DesiredRevision,
    pub digest: DesiredDigest,
}

impl DesiredRow {
    /// Canonical bytes naming exactly these committed bytes.
    fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = Canonical::new();
        out.text(&self.row.key.zone);
        out.text(&self.row.key.type_name);
        out.text(&self.row.key.name);
        out.bytes(self.row.uid.as_slice());
        out.u64(self.row.generation);
        out.u64(self.revision.get());
        out.optional_bytes(self.row.owner_uid.as_ref().map(|uid| uid.as_slice()));
        out.u8(provenance_tag(self.row.provenance));
        out.flag(self.row.deleting);
        out.bytes(&self.row.spec);
        out.bytes(&self.row.metadata);
        out.u64(self.row.created_at.cast_unsigned());
        out.finish()
    }
}

/// The canonical bytes naming one committed row at one revision.
///
/// Free-standing so the digest can be recomputed from a decoded row without
/// constructing a [`DesiredRow`] first.
fn canonical_row_bytes(row: &StoredDesiredResource, revision: DesiredRevision) -> Vec<u8> {
    DesiredRow { row: row.clone(), revision, digest: digest_of(&[]) }.canonical_bytes()
}

// ---------------------------------------------------------------------------
// The durable transaction record
// ---------------------------------------------------------------------------

/// Where one publication transaction stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationState {
    /// The candidate is durable; the broker has not acknowledged a fence.
    Staged,
    /// The broker durably froze this Zone's new-effect admission for this
    /// candidate and returned its prepared transaction identity.
    Prepared,
    /// The desired rows, revisions, audit, and outbox entry are committed;
    /// the broker has not acknowledged the accepted revision yet.
    Committed,
    /// The broker acknowledged this exact accepted revision.
    Accepted,
    /// Abandoned before any desired row or outbox entry existed.
    Cancelled,
}

impl PublicationState {
    /// The database spelling this state persists as.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Staged => "staged",
            Self::Prepared => "prepared",
            Self::Committed => "committed",
            Self::Accepted => "accepted",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "staged" => Some(Self::Staged),
            "prepared" => Some(Self::Prepared),
            "committed" => Some(Self::Committed),
            "accepted" => Some(Self::Accepted),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// Whether the transaction still owes the store an outcome, i.e. whether
    /// it blocks the next authority mutation for its Zone.
    pub const fn is_outstanding(self) -> bool {
        matches!(self, Self::Staged | Self::Prepared | Self::Committed)
    }
}

/// One durable publication transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationTransaction {
    pub transaction: TransactionId,
    pub zone: String,
    pub incarnation: StoreIncarnation,
    /// The reserved Zone sequence this transaction commits at.
    pub sequence: ZoneDesiredSequence,
    pub candidate: DesiredDigest,
    pub state: PublicationState,
    /// The broker's prepared transaction identity, once it exists.
    pub prepared: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The identity of one staged candidate, returned before any broker I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedMutation {
    pub transaction: TransactionId,
    pub zone: String,
    pub incarnation: StoreIncarnation,
    pub sequence: ZoneDesiredSequence,
    pub candidate: DesiredDigest,
    pub staged_at: i64,
}

/// One pending publication: the exact committed bytes the broker validates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEntry {
    pub transaction: TransactionId,
    pub zone: String,
    pub incarnation: StoreIncarnation,
    pub sequence: ZoneDesiredSequence,
    pub candidate: DesiredDigest,
    /// Rows the mutation committed or rewrote, at their committed revisions.
    pub rows: Vec<DesiredRow>,
    /// Rows the mutation retired.
    pub removed: Vec<ResourceKey>,
}

/// The committed mutation the broker must publish, or replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedPublication {
    pub transaction: TransactionId,
    pub zone: String,
    pub incarnation: StoreIncarnation,
    pub sequence: ZoneDesiredSequence,
    pub candidate: DesiredDigest,
    pub publication: OutboxEntry,
}

/// What committing one staged transaction did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitOutcome {
    /// The desired state advanced: the row moved to a new revision and the
    /// outbox now carries the exact committed bytes.
    Committed(CommittedPublication),
    /// The exact candidate is already committed at this revision. The
    /// recorded bytes are returned unchanged: the mutation is not applied a
    /// second time.
    AlreadyCommitted(CommittedPublication),
    /// The candidate is byte-identical to the committed desired state, so
    /// nothing advanced. There is no revision to publish and no outbox
    /// entry, which settles the transaction instead of leaving the Zone
    /// waiting for a publication that would say nothing.
    Unchanged {
        transaction: TransactionId,
        sequence: ZoneDesiredSequence,
        row: DesiredRow,
    },
}

/// The broker's acknowledgment of one accepted revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedPublication {
    pub transaction: TransactionId,
    pub zone: String,
    pub incarnation: StoreIncarnation,
    pub sequence: ZoneDesiredSequence,
    pub candidate: DesiredDigest,
}

/// The last revision the broker durably accepted for one Zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedCursor {
    pub zone: String,
    pub incarnation: StoreIncarnation,
    pub sequence: ZoneDesiredSequence,
    pub digest: DesiredDigest,
    pub accepted_at: i64,
}

/// The explicit recovery decision for one outstanding transaction.
///
/// The variants are the manager-side rows of the plan's Mutation recovery
/// table, keyed on what is durable at the failure boundary. Each one is
/// actionable and idempotent: the caller resolves the broker side and then
/// calls exactly one of
/// [`SpecStore::commit_mutation`](crate::spec_store::SpecStore::commit_mutation),
/// [`SpecStore::cancel_transaction`](crate::spec_store::SpecStore::cancel_transaction), or
/// [`SpecStore::acknowledge`](crate::spec_store::SpecStore::acknowledge).
///
/// The table's remaining two rows have no durable manager-side fact to key
/// on and are answered by refusal rather than by a variant:
///
/// - an unknown or mismatched acknowledgment carries a transaction,
///   sequence, digest, or incarnation this store never committed, so
///   [`SpecStoreError::PublicationMismatch`] refuses it and the Zone stays
///   fenced until the exact transaction state is reconciled;
/// - a broker restart that leaves the accepted cursor behind the store's
///   committed sequence is exactly a [`Self::ReplayCommit`] entry here, and a
///   broker restart with nothing outstanding owes nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionRecovery {
    /// Before the broker's PrepareChange acknowledgment: the staged
    /// candidate may exist and the desired rows are unchanged. Check the
    /// broker's transaction state, then commit or cancel.
    ResumeOrDiscard { transaction: PublicationTransaction },
    /// After the fence and before the desired commit: the broker holds a
    /// prepared identity and this store a staged candidate. Replay the exact
    /// candidate or cancel it after proving no desired or outbox commit
    /// exists.
    ReplayOrCancel { transaction: PublicationTransaction },
    /// After the desired commit and before the broker's acknowledgment: the
    /// desired rows and the outbox entry are committed and the fence
    /// remains. Replay the exact CommitChange; no provider effect may use
    /// the unaccepted revision.
    ReplayCommit { transaction: PublicationTransaction, publication: OutboxEntry },
}

impl TransactionRecovery {
    /// The transaction this decision is about.
    pub fn transaction(&self) -> &PublicationTransaction {
        match self {
            Self::ResumeOrDiscard { transaction }
            | Self::ReplayOrCancel { transaction }
            | Self::ReplayCommit { transaction, .. } => transaction,
        }
    }
}

/// Everything one Zone owes after a restart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneRecovery {
    pub zone: String,
    pub incarnation: StoreIncarnation,
    /// The last revision the broker accepted, if any.
    pub accepted: Option<AcceptedCursor>,
    /// Every transaction this store still owes an outcome for, with its
    /// explicit decision.
    pub transactions: Vec<(TransactionId, TransactionRecovery)>,
}

impl ZoneRecovery {
    /// Whether the Zone has any transaction that must be resolved before
    /// another authority mutation may be staged.
    pub fn has_outstanding(&self) -> bool {
        self.transactions.iter().any(|(_, recovery)| recovery.transaction().state.is_outstanding())
    }
}

// ---------------------------------------------------------------------------
// Canonical encoding
// ---------------------------------------------------------------------------

/// Length-prefixed canonical byte encoding.
///
/// Every digest and every durable candidate or payload blob uses this one
/// framing, so a value can be hashed, stored, and read back without a second
/// spelling of the same bytes.
struct Canonical(Vec<u8>);

impl Canonical {
    fn new() -> Self {
        Self(Vec::new())
    }

    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn flag(&mut self, value: bool) {
        self.0.push(u8::from(value));
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_be_bytes());
    }

    fn bytes(&mut self, value: &[u8]) {
        self.u64(value.len() as u64);
        self.0.extend_from_slice(value);
    }

    fn optional_bytes(&mut self, value: Option<&[u8]>) {
        match value {
            None => self.flag(false),
            Some(value) => {
                self.flag(true);
                self.bytes(value);
            }
        }
    }

    fn text(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    fn finish(self) -> Vec<u8> {
        self.0
    }
}

/// The reader half of [`Canonical`], refusing anything it cannot account for.
struct CanonicalReader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> CanonicalReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], SpecStoreError> {
        let end = self.at.checked_add(len).ok_or(SpecStoreError::CorruptJournalPayload {
            detail: "length overflows the payload",
        })?;
        let slice = self.data.get(self.at..end).ok_or(SpecStoreError::CorruptJournalPayload {
            detail: "payload ended before the encoded value did",
        })?;
        self.at = end;
        Ok(slice)
    }

    /// One fixed-size identity field.
    ///
    /// It keeps the same length prefix as every other value, so the encoding
    /// stays self-describing, and the decoder refuses a length that does not
    /// name exactly the fixed size instead of reading a neighbouring field's
    /// bytes as the identity.
    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], SpecStoreError> {
        let bytes = self.bytes()?;
        <[u8; N]>::try_from(bytes)
            .map_err(|_| SpecStoreError::CorruptJournalPayload {
                detail: "a fixed-size identity field names another length",
            })
    }

    fn u8(&mut self) -> Result<u8, SpecStoreError> {
        Ok(self.take(1)?[0])
    }

    fn flag(&mut self) -> Result<bool, SpecStoreError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(SpecStoreError::CorruptJournalPayload { detail: "invalid boolean tag" }),
        }
    }

    fn u64(&mut self) -> Result<u64, SpecStoreError> {
        let bytes = self.take(8)?;
        let mut raw = [0u8; 8];
        raw.copy_from_slice(bytes);
        Ok(u64::from_be_bytes(raw))
    }

    fn bytes(&mut self) -> Result<&'a [u8], SpecStoreError> {
        let len = self.u64()?;
        if len > MAX_JOURNAL_BLOB_BYTES as u64 {
            return Err(SpecStoreError::CorruptJournalPayload {
                detail: "encoded value exceeds the journal blob ceiling",
            });
        }
        self.take(len as usize)
    }

    /// One optional fixed-size identity field, framed like every other value.
    fn optional_fixed<const N: usize>(&mut self) -> Result<Option<[u8; N]>, SpecStoreError> {
        if self.flag()? { self.fixed::<N>().map(Some) } else { Ok(None) }
    }

    fn text(&mut self) -> Result<String, SpecStoreError> {
        let bytes = self.bytes()?;
        String::from_utf8(bytes.to_vec()).map_err(|_| SpecStoreError::CorruptJournalPayload {
            detail: "encoded text is not valid UTF-8",
        })
    }
}

// ---------------------------------------------------------------------------
// Store incarnation and transaction identity
// ---------------------------------------------------------------------------

/// Mint the store incarnation of a freshly created authority-journal store.
///
/// It is minted once, when the schema is created, and only ever read back
/// afterwards: a different incarnation is a different store, never a newer
/// one, so ordinary acceptance can never install it and only the
/// ownership-bounded reset can. The derivation mixes wall-clock nanoseconds
/// with the creating process and a per-process counter so two stores created
/// in the same nanosecond still differ; the value is non-secret identity, not
/// a credential.
pub fn mint_store_incarnation() -> Result<StoreIncarnation, SchemaError> {
    static MINTED: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or(0);
    let ordinal = MINTED.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(STORE_INCARNATION_DOMAIN_TAG.as_bytes());
    hasher.update(b"\0");
    hasher.update(nanos.to_be_bytes());
    hasher.update(std::process::id().to_be_bytes());
    hasher.update(ordinal.to_be_bytes());
    let digest = hasher.finalize();
    let suffix = digest[..8].iter().fold(String::with_capacity(16), |mut text, byte| {
        use std::fmt::Write;
        let _ = write!(text, "{byte:02x}");
        text
    });
    StoreIncarnation::parse(format!("store-{suffix}")).map_err(|_| SchemaError::Incarnation)
}

/// Derive the durable identity of one staged candidate.
///
/// The identity is a function of durable facts - the store incarnation, the
/// Zone, the reserved sequence, and the candidate digest - rather than of a
/// random draw. Re-staging the same candidate at the same reserved sequence
/// therefore names the same transaction, so recovery after a lost reply
/// cannot mint a second identity for one mutation.
fn derive_transaction_id(
    incarnation: &StoreIncarnation,
    zone: &str,
    sequence: ZoneDesiredSequence,
    candidate: &DesiredDigest,
) -> TransactionId {
    let mut hasher = Sha256::new();
    hasher.update(TRANSACTION_ID_DOMAIN_TAG.as_bytes());
    hasher.update(b"\0");
    hasher.update(incarnation.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(zone.as_bytes());
    hasher.update(b"\0");
    hasher.update(sequence.get().to_be_bytes());
    hasher.update(b"\0");
    hasher.update(candidate.as_str().as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    TransactionId::from_bytes(bytes)
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

pub(crate) fn store_incarnation(
    conn: &Connection,
) -> Result<StoreIncarnation, SpecStoreError> {
    let stored: Option<String> = conn
        .query_row("SELECT value FROM store_meta WHERE key = 'store_incarnation'", [], |row| {
            row.get(0)
        })
        .optional()?;
    let stored = stored.ok_or(SpecStoreError::JournalCorrupt {
        transaction: TransactionId::from_bytes([0; 16]),
        detail: "the store carries no incarnation",
    })?;
    StoreIncarnation::parse(stored).map_err(|_| SpecStoreError::JournalCorrupt {
        transaction: TransactionId::from_bytes([0; 16]),
        detail: "the stored store incarnation is not canonical",
    })
}

pub(crate) fn zone_sequence(
    conn: &Connection,
    zone: &str,
) -> Result<ZoneDesiredSequence, SpecStoreError> {
    let stored: Option<i64> = conn
        .query_row("SELECT sequence FROM zone_desired_sequence WHERE zone = ?1", params![zone], |row| {
            row.get(0)
        })
        .optional()?;
    match stored {
        None => Ok(ZoneDesiredSequence::INITIAL),
        Some(raw) => decode_persisted_counter(raw)
            .and_then(zone_sequence_from_counter)
            .ok_or_else(|| SpecStoreError::CorruptCounter { zone: zone.to_owned() }),
    }
}

pub(crate) fn accepted_cursor(
    conn: &Connection,
    zone: &str,
) -> Result<Option<AcceptedCursor>, SpecStoreError> {
    let row: Option<(String, String, i64, String, i64)> = conn
        .query_row(
            "SELECT zone, incarnation, accepted_sequence, accepted_digest, updated_at \
             FROM accepted_cursor WHERE zone = ?1",
            params![zone],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()?;
    let Some((zone, incarnation, sequence, digest, accepted_at)) = row else {
        return Ok(None);
    };
    let incarnation = StoreIncarnation::parse(incarnation).map_err(|_| SpecStoreError::JournalCorrupt {
        transaction: TransactionId::from_bytes([0; 16]),
        detail: "the accepted cursor names a non-canonical incarnation",
    })?;
    let sequence = decode_persisted_counter(sequence)
        .and_then(zone_sequence_from_counter)
        .ok_or_else(|| SpecStoreError::CorruptCounter { zone: zone.clone() })?;
    let digest = DesiredDigest::parse(digest).map_err(|_| SpecStoreError::JournalCorrupt {
        transaction: TransactionId::from_bytes([0; 16]),
        detail: "the accepted cursor names a non-canonical digest",
    })?;
    Ok(Some(AcceptedCursor { zone, incarnation, sequence, digest, accepted_at }))
}

/// One transaction record read as plain SQLite types, for the same reason
/// [`crate::spec_store::DesiredRowTuple`] exists: reading the columns and
/// decoding them are separate steps so a decode failure keeps its typed
/// error.
type TransactionTuple = (
    Vec<u8>,
    String,
    String,
    i64,
    String,
    Vec<u8>,
    String,
    Option<String>,
    i64,
    i64,
);

fn transaction_tuple(row: &rusqlite::Row<'_>) -> rusqlite::Result<TransactionTuple> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
    ))
}

fn decode_transaction(
    values: TransactionTuple,
) -> Result<PublicationTransaction, SpecStoreError> {
    let (id, zone, incarnation, sequence, candidate, _staged_candidate, state, prepared, created_at, updated_at) = values;
    let corrupt = |detail: &'static str| SpecStoreError::JournalCorrupt {
        transaction: TransactionId::from_bytes([0; 16]),
        detail,
    };
    let incarnation =
        StoreIncarnation::parse(incarnation).map_err(|_| corrupt("non-canonical incarnation"))?;
    let sequence = decode_persisted_counter(sequence)
        .and_then(zone_sequence_from_counter)
        .ok_or_else(|| SpecStoreError::CorruptCounter { zone: zone.clone() })?;
    let candidate =
        DesiredDigest::parse(candidate).map_err(|_| corrupt("non-canonical candidate digest"))?;
    let state =
        PublicationState::parse(&state).ok_or_else(|| corrupt("unknown transaction state"))?;
    let transaction = id.as_slice().try_into().map(TransactionId::from_bytes).map_err(|_| {
        corrupt("transaction identity is not 16 bytes")
    })?;
    Ok(PublicationTransaction {
        transaction,
        zone,
        incarnation,
        sequence,
        candidate,
        state,
        prepared,
        created_at,
        updated_at,
    })
}

const TRANSACTION_COLUMNS: &str = "transaction_id, zone, incarnation, sequence, \
     candidate_digest, candidate, state, prepared_id, created_at, updated_at";

fn load_transaction(
    conn: &Connection,
    transaction: TransactionId,
) -> Result<PublicationTransaction, SpecStoreError> {
    let row = conn
        .query_row(
            &format!("SELECT {TRANSACTION_COLUMNS} FROM authority_transaction \
                      WHERE transaction_id = ?1"),
            params![transaction.as_bytes().as_slice()],
            transaction_tuple,
        )
        .optional()?
        .map(decode_transaction)
        .transpose()?;
    row.ok_or(SpecStoreError::TransactionNotFound { transaction })
}

fn load_candidate(conn: &Connection, transaction: TransactionId) -> Result<Vec<u8>, SpecStoreError> {
    conn.query_row(
        "SELECT candidate FROM authority_transaction WHERE transaction_id = ?1",
        params![transaction.as_bytes().as_slice()],
        |row| row.get::<_, Vec<u8>>(0),
    )
    .optional()?
    .ok_or(SpecStoreError::TransactionNotFound { transaction })
}

fn load_outbox_entry(
    conn: &Connection,
    transaction: TransactionId,
) -> Result<OutboxEntry, SpecStoreError> {
    let row: Option<(String, String, i64, String, Vec<u8>)> = conn
        .query_row(
            "SELECT zone, incarnation, sequence, candidate_digest, payload FROM publication_outbox \
             WHERE transaction_id = ?1",
            params![transaction.as_bytes().as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()?;
    let Some((zone, incarnation, sequence, candidate, payload)) = row else {
        return Err(SpecStoreError::JournalCorrupt {
            transaction,
            detail: "a committed transaction has no outbox entry",
        });
    };
    let incarnation = StoreIncarnation::parse(incarnation).map_err(|_| SpecStoreError::JournalCorrupt {
        transaction,
        detail: "the outbox entry names a non-canonical incarnation",
    })?;
    let sequence = decode_persisted_counter(sequence)
        .and_then(zone_sequence_from_counter)
        .ok_or_else(|| SpecStoreError::CorruptCounter { zone: zone.clone() })?;
    let candidate =
        DesiredDigest::parse(candidate).map_err(|_| SpecStoreError::JournalCorrupt {
            transaction,
            detail: "the outbox entry names a non-canonical digest",
        })?;
    let payload = decode_payload(&payload)?;
    Ok(OutboxEntry {
        transaction,
        zone,
        incarnation,
        sequence,
        candidate,
        rows: payload.rows,
        removed: payload.removed,
    })
}

/// The committed rows and retired keys one outbox payload carries.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CommittedPayload {
    rows: Vec<DesiredRow>,
    removed: Vec<ResourceKey>,
}

fn encode_payload(rows: &[DesiredRow], removed: &[ResourceKey]) -> Vec<u8> {
    let mut out = Canonical::new();
    out.u64(rows.len() as u64);
    for row in rows {
        let bytes = row.canonical_bytes();
        out.bytes(&bytes);
    }
    out.u64(removed.len() as u64);
    for key in removed {
        out.text(&key.zone);
        out.text(&key.type_name);
        out.text(&key.name);
    }
    out.finish()
}

fn decode_payload(payload: &[u8]) -> Result<CommittedPayload, SpecStoreError> {
    let mut reader = CanonicalReader::new(payload);
    let row_count = reader.u64()?;
    let mut rows = Vec::with_capacity(usize::try_from(row_count).unwrap_or(0));
    for _ in 0..row_count {
        let encoded = reader.bytes()?;
        rows.push(decode_row(&mut CanonicalReader::new(encoded))?);
    }
    let removed_count = reader.u64()?;
    let mut removed = Vec::with_capacity(usize::try_from(removed_count).unwrap_or(0));
    for _ in 0..removed_count {
        removed.push(ResourceKey::new(reader.text()?, reader.text()?, reader.text()?));
    }
    Ok(CommittedPayload { rows, removed })
}

fn decode_row(reader: &mut CanonicalReader<'_>) -> Result<DesiredRow, SpecStoreError> {
    let zone = reader.text()?;
    let type_name = reader.text()?;
    let name = reader.text()?;
    let uid = reader.fixed::<16>()?;
    let generation = reader.u64()?;
    let revision = reader.u64()?;
    let owner_uid = reader.optional_fixed::<16>()?;
    let provenance = provenance_from_tag(reader.u8()?);
    let deleting = reader.flag()?;
    let spec = reader.bytes()?.to_vec();
    let metadata = reader.bytes()?.to_vec();
    let created_at = reader.u64()?;
    let row = StoredDesiredResource {
        key: ResourceKey::new(zone, type_name, name),
        uid,
        generation,
        owner_uid,
        provenance,
        deleting,
        spec,
        metadata,
        created_at: created_at as i64,
    };
    let revision = desired_revision_from_counter(revision).ok_or(
        SpecStoreError::CorruptJournalPayload { detail: "encoded revision is not a revision" },
    )?;
    let digest = digest_of(&canonical_row_bytes(&row, revision));
    Ok(DesiredRow { row, revision, digest })
}

// ---------------------------------------------------------------------------
// Desired-row reads (authority-journal format)
// ---------------------------------------------------------------------------

/// One committed desired row with its revision.
pub fn desired_row(conn: &Connection, key: &ResourceKey) -> Result<DesiredRow, SpecStoreError> {
    let sql = format!(
        "SELECT {}, desired_revision FROM resources WHERE zone = ?1 AND type = ?2 AND name = ?3",
        crate::spec_store::AUTHORITY_ROW_COLUMNS
    );
    let row = conn
        .query_row(&sql, params![key.zone, key.type_name, key.name], desired_row_tuple)
        .optional()?;
    let values = row.ok_or_else(|| SpecStoreError::NotFound {
        zone: key.zone.clone(),
        type_name: key.type_name.clone(),
        name: key.name.clone(),
    })?;
    let (row, revision) = decode_desired_row(values)?;
    let raw = revision.ok_or_else(|| SpecStoreError::JournalCorrupt {
        transaction: TransactionId::from_bytes([0; 16]),
        detail: "a desired row carries no persisted revision",
    })?;
    finish_desired_row(row, raw)
}

/// Decode one persisted desired revision, refusing a column that is not a
/// usable counter.
fn finish_desired_row(row: StoredDesiredResource, raw: i64) -> Result<DesiredRow, SpecStoreError> {
    let revision = decode_persisted_counter(raw)
        .and_then(desired_revision_from_counter)
        .ok_or_else(|| SpecStoreError::CorruptCounter { zone: row.key.zone.clone() })?;
    let digest = digest_of(&canonical_row_bytes(&row, revision));
    Ok(DesiredRow { row, revision, digest })
}

/// Every committed desired row the selector matches, with its revision.
pub fn desired_rows(
    conn: &Connection,
    selector: &SpecSelector,
) -> Result<Vec<DesiredRow>, SpecStoreError> {
    let sql = format!(
        "SELECT {}, desired_revision FROM resources \
         WHERE (?1 IS NULL OR zone = ?1) \
           AND (?2 IS NULL OR type = ?2) \
           AND (?3 IS NULL OR owner_uid = ?3) \
         ORDER BY zone, type, name",
        crate::spec_store::AUTHORITY_ROW_COLUMNS
    );
    let zone = selector.zone.as_deref();
    let type_name = selector.type_name.as_deref();
    let owner = selector.owner_uid.map(|uid| uid.to_vec());
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query(params![zone, type_name, owner])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let (row, revision) = decode_desired_row(desired_row_tuple(row)?)?;
        let raw = revision.ok_or_else(|| SpecStoreError::JournalCorrupt {
            transaction: TransactionId::from_bytes([0; 16]),
            detail: "a desired row carries no persisted revision",
        })?;
        out.push(finish_desired_row(row, raw)?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Mutations
// ---------------------------------------------------------------------------

/// Stage one candidate: persist it and reserve its Zone sequence.
///
/// Nothing desired has moved, but the sequence is consumed: it is the value
/// the broker will fence this candidate against, so it must never be handed
/// out twice even if the candidate turns out to change nothing. The
/// reservation is exactly one past the Zone's last reserved sequence, because
/// at most one transaction is outstanding per Zone - other authority
/// mutations queue behind it.
pub fn stage_mutation(
    conn: &mut Connection,
    mutation: DesiredMutation,
) -> Result<StagedMutation, SpecStoreError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let zone = mutation.zone().to_owned();
    let incarnation = store_incarnation(&tx)?;
    if let Some(outstanding) = outstanding_transaction(&tx, &zone)? {
        return Err(SpecStoreError::ZoneTransactionOutstanding {
            zone,
            transaction: outstanding,
        });
    }
    let current = zone_sequence(&tx, &zone)?;
    let sequence = next_zone_sequence(current).map_err(|_| SpecStoreError::ZoneSequenceExhausted {
        zone: zone.clone(),
    })?;
    let candidate = mutation.candidate_digest();
    let transaction = derive_transaction_id(&incarnation, &zone, sequence, &candidate);
    // The reservation is durable with the candidate, so a transaction that
    // settles without publishing still leaves the Zone sequence monotonic:
    // no later transaction can reuse a sequence a broker may already have
    // seen.
    advance_zone_sequence(&tx, &zone, sequence)?;
    let staged_at = now();
    let payload = mutation.encode();
    tx.execute(
        "INSERT INTO authority_transaction (transaction_id, zone, incarnation, sequence, \
         candidate_digest, candidate, state, prepared_id, committed_at, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, NULL, ?8, ?8)",
        params![
            transaction.as_bytes().as_slice(),
            zone,
            incarnation.as_str(),
            encode_persisted_counter(sequence.get()),
            candidate.as_str(),
            payload,
            PublicationState::Staged.as_str(),
            staged_at,
        ],
    )?;
    tx.commit()?;
    Ok(StagedMutation {
        transaction,
        zone,
        incarnation,
        sequence,
        candidate,
        staged_at,
    })
}

/// Record the broker's prepared transaction identity for a staged candidate.
///
/// This is the point at which the Zone's new-effect admission is durably
/// frozen for this candidate. Recording the same identity twice is
/// idempotent; a different identity, or a transaction that already committed,
/// is refused rather than silently accepted.
pub fn record_prepared(
    conn: &mut Connection,
    transaction: TransactionId,
    prepared: &str,
) -> Result<PublicationTransaction, SpecStoreError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = load_transaction(&tx, transaction)?;
    let updated = match existing.state {
        PublicationState::Staged => true,
        PublicationState::Prepared if existing.prepared.as_deref() == Some(prepared) => false,
        PublicationState::Prepared => {
            return Err(SpecStoreError::TransactionStateConflict {
                transaction,
                state: PublicationState::Prepared.as_str(),
                transition: "recording a different prepared identity",
            });
        }
        state => {
            return Err(SpecStoreError::TransactionStateConflict {
                transaction,
                state: state.as_str(),
                transition: "recording a prepared identity",
            });
        }
    };
    if updated {
        tx.execute(
            "UPDATE authority_transaction SET state = ?2, prepared_id = ?3, updated_at = ?4 \
             WHERE transaction_id = ?1",
            params![
                transaction.as_bytes().as_slice(),
                PublicationState::Prepared.as_str(),
                prepared,
                now(),
            ],
        )?;
    }
    tx.commit()?;
    load_transaction(conn, transaction)
}

/// Apply the staged candidate: desired rows, revisions, audit, outbox entry,
/// and the Zone sequence, in one SQLite transaction.
///
/// The candidate comes from the durable staged record rather than from the
/// caller, so a replay after any failure boundary re-derives the same bytes.
/// A transaction that is already committed returns its recorded publication
/// instead of applying anything a second time.
pub fn commit_mutation(
    conn: &mut Connection,
    transaction: TransactionId,
) -> Result<CommitOutcome, SpecStoreError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = load_transaction(&tx, transaction)?;
    if existing.state == PublicationState::Committed {
        let publication = committed_publication(&tx, &existing)?;
        tx.commit()?;
        return Ok(CommitOutcome::AlreadyCommitted(publication));
    }
    if !matches!(existing.state, PublicationState::Staged | PublicationState::Prepared) {
        return Err(SpecStoreError::TransactionStateConflict {
            transaction,
            state: existing.state.as_str(),
            transition: "committing the desired mutation",
        });
    }
    let candidate = DesiredMutation::decode(&load_candidate(&tx, transaction)?)?;
    let zone = existing.zone.clone();
    let sequence = existing.sequence;
    let (rows, removed, audit) = match &candidate {
        DesiredMutation::Ensure(row) => match apply_ensure(&tx, row.clone())? {
            EnsureApply::Changed { row: desired, generation_before, generation_after } => (
                vec![desired],
                Vec::new(),
                audit_for(&candidate, row.provenance, generation_before, Some(generation_after)),
            ),
            EnsureApply::Unchanged(unchanged) => {
                settle_without_publication(&tx, &existing)?;
                tx.commit()?;
                return Ok(CommitOutcome::Unchanged {
                    transaction,
                    sequence,
                    row: unchanged,
                });
            }
        },
        DesiredMutation::MarkDeleting(key) => match apply_mark_deleting(&tx, key)? {
            DeleteApply::Changed { row: desired, provenance, generation } => (
                vec![desired],
                Vec::new(),
                audit_for(&candidate, provenance, Some(generation), Some(generation)),
            ),
            DeleteApply::Unchanged(unchanged) => {
                settle_without_publication(&tx, &existing)?;
                tx.commit()?;
                return Ok(CommitOutcome::Unchanged {
                    transaction,
                    sequence,
                    row: unchanged,
                });
            }
        },
        DesiredMutation::Remove(key) => {
            let (provenance, generation) = apply_remove(&tx, key)?;
            (
                Vec::new(),
                vec![key.clone()],
                audit_for(&candidate, provenance, Some(generation), None),
            )
        }
    };
    let payload = encode_payload(&rows, &removed);
    insert_audit(&tx, audit)?;
    tx.execute(
        "INSERT INTO publication_outbox (transaction_id, zone, incarnation, sequence, \
         candidate_digest, payload, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            transaction.as_bytes().as_slice(),
            zone,
            existing.incarnation.as_str(),
            encode_persisted_counter(sequence.get()),
            existing.candidate.as_str(),
            payload,
            now(),
        ],
    )?;
    let committed_at = now();
    tx.execute(
        "UPDATE authority_transaction SET state = ?2, committed_at = ?3, updated_at = ?3 \
         WHERE transaction_id = ?1",
        params![
            transaction.as_bytes().as_slice(),
            PublicationState::Committed.as_str(),
            committed_at,
        ],
    )?;
    let publication = OutboxEntry {
        transaction,
        zone: zone.clone(),
        incarnation: existing.incarnation.clone(),
        sequence,
        candidate: existing.candidate.clone(),
        rows,
        removed,
    };
    tx.commit()?;
    Ok(CommitOutcome::Committed(CommittedPublication {
        transaction,
        zone,
        incarnation: existing.incarnation,
        sequence,
        candidate: existing.candidate,
        publication,
    }))
}

/// Record the broker's accepted revision: settle the transaction, drop its
/// outbox entry, and move the accepted cursor.
///
/// The acknowledgment must name this store's exact transaction, sequence,
/// digest, and store incarnation. A mismatch is refused rather than
/// accepted, because publishing visibility for a revision this store never
/// committed is precisely the fabricated success the recovery table forbids.
pub fn acknowledge(
    conn: &mut Connection,
    accepted: AcceptedPublication,
) -> Result<AcceptedCursor, SpecStoreError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = load_transaction(&tx, accepted.transaction)?;
    if existing.zone != accepted.zone
        || existing.sequence != accepted.sequence
        || existing.candidate != accepted.candidate
        || existing.incarnation != accepted.incarnation
    {
        return Err(SpecStoreError::PublicationMismatch { transaction: accepted.transaction });
    }
    let cursor = match existing.state {
        PublicationState::Committed => {
            let accepted_at = now();
            tx.execute(
                "DELETE FROM publication_outbox WHERE transaction_id = ?1",
                params![accepted.transaction.as_bytes().as_slice()],
            )?;
            tx.execute(
                "UPDATE authority_transaction SET state = ?2, updated_at = ?3 \
                 WHERE transaction_id = ?1",
                params![
                    accepted.transaction.as_bytes().as_slice(),
                    PublicationState::Accepted.as_str(),
                    accepted_at,
                ],
            )?;
            write_cursor(&tx, &existing.zone, &existing.incarnation, existing.sequence, &existing.candidate, accepted_at)?;
            AcceptedCursor {
                zone: existing.zone.clone(),
                incarnation: existing.incarnation.clone(),
                sequence: existing.sequence,
                digest: existing.candidate.clone(),
                accepted_at,
            }
        }
        // A repeated acknowledgment of a transaction this store already
        // settled returns the recorded outcome: the accepted answer is
        // idempotent, and a lost response must not fail the retry.
        PublicationState::Accepted => accepted_cursor(&tx, &existing.zone)?.ok_or(
            SpecStoreError::JournalCorrupt {
                transaction: accepted.transaction,
                detail: "an accepted transaction has no accepted cursor",
            },
        )?,
        state => {
            return Err(SpecStoreError::TransactionStateConflict {
                transaction: accepted.transaction,
                state: state.as_str(),
                transition: "acknowledging the publication",
            });
        }
    };
    tx.commit()?;
    Ok(cursor)
}

/// Abandon a staged or prepared transaction that has committed no desired row.
///
/// Cancelling after the desired commit is refused: aborting cannot silently
/// roll desired state back, so recovery must finish publication or commit an
/// explicitly authorized compensating mutation.
pub fn cancel_transaction(
    conn: &mut Connection,
    transaction: TransactionId,
) -> Result<PublicationTransaction, SpecStoreError> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing = load_transaction(&tx, transaction)?;
    match existing.state {
        PublicationState::Staged | PublicationState::Prepared => {
            let cancelled_at = now();
            tx.execute(
                "UPDATE authority_transaction SET state = ?2, updated_at = ?3 \
                 WHERE transaction_id = ?1",
                params![
                    transaction.as_bytes().as_slice(),
                    PublicationState::Cancelled.as_str(),
                    cancelled_at,
                ],
            )?;
        }
        state => {
            return Err(SpecStoreError::TransactionStateConflict {
                transaction,
                state: state.as_str(),
                transition: "cancelling the transaction",
            });
        }
    }
    tx.commit()?;
    load_transaction(conn, transaction)
}

// ---------------------------------------------------------------------------
// Recovery
// ---------------------------------------------------------------------------

/// Everything one Zone owes after a restart, mapped onto the Mutation
/// recovery table.
pub fn zone_recovery(conn: &Connection, zone: &str) -> Result<ZoneRecovery, SpecStoreError> {
    let incarnation = store_incarnation(conn)?;
    let accepted = accepted_cursor(conn, zone)?;
    let mut stmt = conn.prepare(&format!(
        "SELECT {TRANSACTION_COLUMNS} FROM authority_transaction WHERE zone = ?1 \
         ORDER BY sequence"
    ))?;
    let mut transactions = Vec::new();
    let mut rows = stmt.query(params![zone])?;
    while let Some(row) = rows.next()? {
        let transaction = decode_transaction(transaction_tuple(row)?)?;
        let identity = transaction.transaction;
        let recovery = match transaction.state {
            PublicationState::Staged => TransactionRecovery::ResumeOrDiscard { transaction },
            PublicationState::Prepared => TransactionRecovery::ReplayOrCancel { transaction },
            PublicationState::Committed => TransactionRecovery::ReplayCommit {
                publication: load_outbox_entry(conn, identity)?,
                transaction,
            },
            // A terminal state owes nothing, so recovery does not list it.
            PublicationState::Accepted | PublicationState::Cancelled => continue,
        };
        transactions.push((identity, recovery));
    }
    Ok(ZoneRecovery { zone: zone.to_owned(), incarnation, accepted, transactions })
}

/// The one outstanding transaction of `zone`, if any.
fn outstanding_transaction(
    conn: &Connection,
    zone: &str,
) -> Result<Option<TransactionId>, SpecStoreError> {
    let id: Option<Vec<u8>> = conn
        .query_row(
            "SELECT transaction_id FROM authority_transaction \
             WHERE zone = ?1 AND state IN ('staged', 'prepared', 'committed') ORDER BY sequence",
            params![zone],
            |row| row.get(0),
        )
        .optional()?;
    match id {
        None => Ok(None),
        Some(bytes) => TransactionId::from_column(&bytes).map(Some).ok_or(
            SpecStoreError::JournalCorrupt {
                transaction: TransactionId::from_bytes([0; 16]),
                detail: "a transaction identity is not 16 bytes",
            },
        ),
    }
}

// ---------------------------------------------------------------------------
// Transaction helpers
// ---------------------------------------------------------------------------

fn committed_publication(
    conn: &Connection,
    transaction: &PublicationTransaction,
) -> Result<CommittedPublication, SpecStoreError> {
    let publication = load_outbox_entry(conn, transaction.transaction)?;
    Ok(CommittedPublication {
        transaction: transaction.transaction,
        zone: transaction.zone.clone(),
        incarnation: transaction.incarnation.clone(),
        sequence: transaction.sequence,
        candidate: transaction.candidate.clone(),
        publication,
    })
}

/// Settle a transaction whose candidate changed nothing.
///
/// The transaction becomes terminal so the Zone is not left fenced against a
/// publication that carries no change, and its reserved sequence stays
/// consumed. No outbox entry and no accepted-cursor move: the broker
/// accepted no revision here, so there is nothing to record as accepted and
/// nothing to replay.
fn settle_without_publication(
    conn: &Connection,
    transaction: &PublicationTransaction,
) -> Result<(), SpecStoreError> {
    let settled_at = now();
    conn.execute(
        "UPDATE authority_transaction SET state = ?2, updated_at = ?3 WHERE transaction_id = ?1",
        params![
            transaction.transaction.as_bytes().as_slice(),
            PublicationState::Accepted.as_str(),
            settled_at,
        ],
    )?;
    Ok(())
}

fn write_cursor(
    conn: &Connection,
    zone: &str,
    incarnation: &StoreIncarnation,
    sequence: ZoneDesiredSequence,
    digest: &DesiredDigest,
    accepted_at: i64,
) -> Result<(), SpecStoreError> {
    if let Some(previous) = accepted_cursor(conn, zone)?
        && previous.sequence >= sequence
    {
        return Err(SpecStoreError::AcceptedSequenceConflict {
            transaction: TransactionId::from_bytes([0; 16]),
            committed: sequence.get(),
            acknowledged: previous.sequence.get().to_string(),
        });
    }
    conn.execute(
        "INSERT INTO accepted_cursor (zone, incarnation, accepted_sequence, accepted_digest, \
         updated_at) VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT (zone) DO UPDATE SET incarnation = ?2, accepted_sequence = ?3, \
         accepted_digest = ?4, updated_at = ?5",
        params![
            zone,
            incarnation.as_str(),
            encode_persisted_counter(sequence.get()),
            digest.as_str(),
            accepted_at,
        ],
    )?;
    Ok(())
}

fn advance_zone_sequence(
    conn: &Connection,
    zone: &str,
    sequence: ZoneDesiredSequence,
) -> Result<(), SpecStoreError> {
    let stored = encode_persisted_counter(sequence.get())
        .ok_or_else(|| SpecStoreError::ZoneSequenceExhausted { zone: zone.to_owned() })?;
    conn.execute(
        "INSERT INTO zone_desired_sequence (zone, sequence) VALUES (?1, ?2) \
         ON CONFLICT (zone) DO UPDATE SET sequence = ?2",
        params![zone, stored],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Desired-row writes
// ---------------------------------------------------------------------------

enum EnsureApply {
    Changed { row: DesiredRow, generation_before: Option<u64>, generation_after: u64 },
    Unchanged(DesiredRow),
}

enum DeleteApply {
    Changed { row: DesiredRow, provenance: ResourceProvenance, generation: u64 },
    Unchanged(DesiredRow),
}

/// The committed row at `key`, or `None` when the row is absent.
///
/// A row that exists but cannot be decoded is an error, never an absence: a
/// mutation must not be able to read an unreadable row as "create".
fn lookup_desired_row(
    conn: &Connection,
    key: &ResourceKey,
) -> Result<Option<DesiredRow>, SpecStoreError> {
    let sql = format!(
        "SELECT {}, desired_revision FROM resources WHERE zone = ?1 AND type = ?2 AND name = ?3",
        crate::spec_store::AUTHORITY_ROW_COLUMNS
    );
    let found = conn
        .query_row(&sql, params![key.zone, key.type_name, key.name], desired_row_tuple)
        .optional()?;
    match found {
        None => Ok(None),
        Some(values) => {
            let (row, raw) = decode_desired_row(values)?;
            finish_desired_row(
                row,
                raw.ok_or(SpecStoreError::JournalCorrupt {
                    transaction: TransactionId::from_bytes([0; 16]),
                    detail: "a desired row carries no persisted revision",
                })?,
            )
            .map(Some)
        }
    }
}

/// Apply one desired row.
///
/// The revision advances on every branch that changes a committed column, and
/// the branches differ in *which* column moved, never in whether the row is
/// still authoritative:
///
/// - absent: created at generation 1 and revision 1;
/// - spec bytes differ: generation and revision both advance;
/// - spec bytes identical but owner, metadata, or provenance moved: the
///   generation stays, because identity and generation are the spec's
///   contract, while the revision advances, because an ownership or metadata
///   change invalidates earlier effect authority exactly like a spec change;
/// - nothing differs: no write, no revision, no publication.
fn apply_ensure(
    conn: &Connection,
    row: StoredDesiredResource,
) -> Result<EnsureApply, SpecStoreError> {
    let Some(existing) = lookup_desired_row(conn, &row.key)? else {
        let revision = next_desired_revision(DesiredRevision::INITIAL).map_err(|_| {
            SpecStoreError::RowRevisionExhausted {
                zone: row.key.zone.clone(),
                type_name: row.key.type_name.clone(),
                name: row.key.name.clone(),
            }
        })?;
        insert_row(conn, row.clone(), revision)?;
        return Ok(EnsureApply::Changed {
            row: desired_row(conn, &row.key)?,
            generation_before: None,
            generation_after: 1,
        });
    };
    if existing.row.deleting {
        return Err(SpecStoreError::ResourceDeleting {
            zone: row.key.zone.clone(),
            type_name: row.key.type_name.clone(),
            name: row.key.name.clone(),
        });
    }
    let incoming_owner = row.owner_uid.map(|uid| uid.to_vec());
    let spec_changed = existing.row.spec != row.spec;
    if !spec_changed
        && existing.row.metadata == row.metadata
        && existing.row.owner_uid.map(|uid| uid.to_vec()) == incoming_owner
        && existing.row.provenance == row.provenance
    {
        return Ok(EnsureApply::Unchanged(existing));
    }
    let revision = next_desired_revision(existing.revision).map_err(|_| {
        SpecStoreError::RowRevisionExhausted {
            zone: row.key.zone.clone(),
            type_name: row.key.type_name.clone(),
            name: row.key.name.clone(),
        }
    })?;
    if spec_changed {
        let next_generation = existing.row.generation.checked_add(1).ok_or_else(|| {
            SpecStoreError::GenerationExhausted {
                zone: row.key.zone.clone(),
                type_name: row.key.type_name.clone(),
                name: row.key.name.clone(),
            }
        })?;
        conn.execute(
            "UPDATE resources SET generation = ?4, uid = ?5, owner_uid = ?6, provenance = ?7, \
             spec = ?8, metadata = ?9 WHERE zone = ?1 AND type = ?2 AND name = ?3",
            params![
                row.key.zone,
                row.key.type_name,
                row.key.name,
                encode_persisted_counter(next_generation),
                row.uid.as_slice(),
                incoming_owner,
                row.provenance.as_str(),
                row.spec,
                row.metadata,
            ],
        )?;
    } else {
        conn.execute(
            "UPDATE resources SET owner_uid = ?4, provenance = ?5, metadata = ?6 \
             WHERE zone = ?1 AND type = ?2 AND name = ?3",
            params![
                row.key.zone,
                row.key.type_name,
                row.key.name,
                incoming_owner,
                row.provenance.as_str(),
                row.metadata,
            ],
        )?;
    }
    set_row_revision(conn, &row.key, revision)?;
    let committed = desired_row(conn, &row.key)?;
    Ok(EnsureApply::Changed {
        generation_after: committed.row.generation,
        generation_before: Some(existing.row.generation),
        row: committed,
    })
}

fn insert_row(
    conn: &Connection,
    row: StoredDesiredResource,
    revision: DesiredRevision,
) -> Result<(), SpecStoreError> {
    conn.execute(
        "INSERT INTO resources (zone, type, name, uid, generation, desired_revision, owner_uid, \
         provenance, deleting, spec, metadata, created_at) \
         VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, 0, ?8, ?9, ?10)",
        params![
            row.key.zone,
            row.key.type_name,
            row.key.name,
            row.uid.as_slice(),
            encode_persisted_counter(revision.get()),
            row.owner_uid.map(|uid| uid.to_vec()),
            row.provenance.as_str(),
            row.spec,
            row.metadata,
            now(),
        ],
    )?;
    Ok(())
}

fn set_row_revision(
    conn: &Connection,
    key: &ResourceKey,
    revision: DesiredRevision,
) -> Result<(), SpecStoreError> {
    conn.execute(
        "UPDATE resources SET desired_revision = ?4 WHERE zone = ?1 AND type = ?2 AND name = ?3",
        params![
            key.zone,
            key.type_name,
            key.name,
            encode_persisted_counter(revision.get()),
        ],
    )?;
    Ok(())
}

/// Set the terminal deleting mark, advancing the revision exactly once.
fn apply_mark_deleting(
    conn: &Connection,
    key: &ResourceKey,
) -> Result<DeleteApply, SpecStoreError> {
    let existing = desired_row(conn, key)?;
    if existing.row.deleting {
        return Ok(DeleteApply::Unchanged(existing));
    }
    let provenance = existing.row.provenance;
    let generation = existing.row.generation;
    let revision = next_desired_revision(existing.revision).map_err(|_| {
        SpecStoreError::RowRevisionExhausted {
            zone: key.zone.clone(),
            type_name: key.type_name.clone(),
            name: key.name.clone(),
        }
    })?;
    conn.execute(
        "UPDATE resources SET deleting = 1, desired_revision = ?4 \
         WHERE zone = ?1 AND type = ?2 AND name = ?3",
        params![
            key.zone,
            key.type_name,
            key.name,
            encode_persisted_counter(revision.get()),
        ],
    )?;
    Ok(DeleteApply::Changed { row: desired_row(conn, key)?, provenance, generation })
}

/// Retire the row. The revision dies with the row; the Zone sequence and the
/// outbox entry are what tell the broker the relationship is gone.
fn apply_remove(
    conn: &Connection,
    key: &ResourceKey,
) -> Result<(ResourceProvenance, u64), SpecStoreError> {
    let existing = desired_row(conn, key)?;
    conn.execute(
        "DELETE FROM resources WHERE zone = ?1 AND type = ?2 AND name = ?3",
        params![key.zone, key.type_name, key.name],
    )?;
    Ok((existing.row.provenance, existing.row.generation))
}

/// The audit record committed with the mutation, in the same transaction.
///
/// It is built from the committed facts rather than from the candidate: the
/// provenance and generation an operator reads are the ones that were
/// actually written.
fn audit_for<'a>(
    mutation: &'a DesiredMutation,
    provenance: ResourceProvenance,
    generation_before: Option<u64>,
    generation_after: Option<u64>,
) -> AuditWrite<'a> {
    let (subject, operation) = match mutation {
        DesiredMutation::Ensure(_) => ("resource.ensure", "authority.ensure"),
        DesiredMutation::MarkDeleting(_) => ("resource.deletion", "authority.deletion.mark"),
        DesiredMutation::Remove(_) => ("resource.deletion", "authority.deletion.removed"),
    };
    AuditWrite {
        ts: now(),
        subject,
        provenance: provenance.as_str(),
        key: Some(mutation.key()),
        operation,
        generation_before: generation_before.map(|value| value as i64),
        generation_after: generation_after.map(|value| value as i64),
    }
}