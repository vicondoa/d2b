//! The nested-call evidence chain (KTD6) and the audit records its legs
//! write.
//!
//! A provider handler that invokes another provider's service never
//! re-presents as the daemon class: each call carries an ordered evidence
//! chain of identities - root first - so the receiving leg knows who
//! initiated the invocation and every hop between. The chain root owns the
//! invocation's audit record: the leg executing the root operation writes
//! exactly one root record per root invocation, and each nested leg writes
//! a correlation record keyed by the root invocation id and its own depth.
//! A mixed-leg chain therefore never produces two root records, and audit
//! consumers count one root record per invocation id.
//!
//! Loop depth is capped: a nested call whose chain would run past
//! [`MAX_NESTED_DEPTH`] is refused with the dedicated
//! [`NESTED_DEPTH_EXCEEDED`] code. The envelope re-declares that code in
//! its own closed refusal set, the same aliasing pattern the fd-leg and
//! stale-context codes use.

use std::io;

use serde::{Deserialize, Serialize};

/// The deepest leg a chain may run.
///
/// The root invocation is depth zero; a nested call presents a chain whose
/// depth is its own leg's depth. A chain past this cap is refused with
/// [`NESTED_DEPTH_EXCEEDED`] at the nested-call entry - both execution
/// legs enforce the same cap, so a call loop trips the dedicated refusal
/// code instead of growing its chain without bound.
pub const MAX_NESTED_DEPTH: usize = 8;

/// The closed loop-refusal code for a nested call whose chain exceeds the
/// depth cap.
///
/// One spelling shared by both execution legs: the broker envelope carries
/// it in its closed refusal set (`ENVELOPE_REFUSALS`), and the daemon-side
/// rendezvous refuses a nested call with it. A chain that hit the cap is a
/// loop or an unresolvably deep chain, never an authz or payload refusal.
pub const NESTED_DEPTH_EXCEEDED: &str = "nested-depth-exceeded";

/// One ordered evidence chain: the root invocation id plus the identities
/// of every hop, root first.
///
/// The chain is root-anchored: every leg shares the root invocation id, so
/// nested legs never mint their own identifiers and the audit records of
/// one invocation always key on the same id. The identities are the
/// invoking principals, ordered from the initiating principal at index
/// zero to the invoking handler of the leg itself at the end.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceChain {
    /// The invocation identifier the root call was minted under; every
    /// nested leg of the invocation carries the same id.
    root_invocation_id: String,
    /// The ordered identities, root first. Never empty: a chain always
    /// starts with the initiating principal.
    identities: Vec<String>,
}

impl EvidenceChain {
    /// The root chain of one invocation: the broker-minted invocation
    /// identifier and the identity the call was initiated under.
    pub fn root(
        root_invocation_id: impl Into<String>,
        initiating_identity: impl Into<String>,
    ) -> Self {
        Self {
            root_invocation_id: root_invocation_id.into(),
            identities: vec![initiating_identity.into()],
        }
    }

    /// The chain one nested leg presents: this chain with the invoking
    /// identity of the calling handler appended.
    ///
    /// Appending never fails: the depth cap is enforced where a chain is
    /// presented for execution (`call_nested` / `invoke_nested`), so a
    /// loop is refused with [`NESTED_DEPTH_EXCEEDED`] at the entry the
    /// chain reaches, never truncate or silently re-rooted here.
    pub fn nested(&self, invoking_identity: impl Into<String>) -> Self {
        let mut identities = self.identities.clone();
        identities.push(invoking_identity.into());
        Self {
            root_invocation_id: self.root_invocation_id.clone(),
            identities,
        }
    }

    /// The invocation identifier the root call was minted under.
    pub fn root_invocation_id(&self) -> &str {
        &self.root_invocation_id
    }

    /// The ordered identities, root first.
    pub fn identities(&self) -> &[String] {
        &self.identities
    }

    /// This leg's depth: zero for the root invocation, one plus the parent
    /// chain's depth for a nested call.
    pub fn depth(&self) -> usize {
        self.identities.len() - 1
    }

    /// Whether this chain presents a nested (non-root) leg.
    pub fn is_nested(&self) -> bool {
        self.depth() > 0
    }

    /// The initiating principal: the identity the root call was made under.
    ///
    /// The graft rule checks every nested call's grants against this
    /// identity, never the daemon class a handler's process re-presents.
    pub fn initiating_identity(&self) -> &str {
        &self.identities[0]
    }

    /// The identity of the handler whose leg presented this chain: the last
    /// hop, which is the invoking principal of the current leg.
    pub fn invoking_identity(&self) -> &str {
        self.identities.last().expect("a chain always carries identities")
    }
}

/// Which process class executed one leg of an invocation.
///
/// Forwarded ops audit daemon-side only; in-broker executions audit
/// broker-side only; never both. The leg tag lets an audit consumer tell
/// the two writers apart on the shared record shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainLeg {
    /// The leg ran inside the broker's own process.
    Broker,
    /// The leg ran in the daemon's process.
    Daemon,
}

/// What role one chain record plays in its invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainRecordClass {
    /// The leg executed the root operation; exactly one root record exists
    /// per root invocation, wherever in the chain that leg ran.
    Root,
    /// The leg executed a nested operation; correlation records key on the
    /// root invocation id and the leg's depth.
    Correlation,
}

/// How one executed leg ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainOutcome {
    /// The leg produced the operation's result.
    Succeeded,
    /// The leg refused the call; the refusal code names the reason.
    Refused,
}

/// The audit record one executing leg writes for its leg of an invocation.
///
/// The shape is shared by both writers: the broker writes records for
/// in-broker executions, the daemon writes records for forwarded and
/// daemon-side executions, and the two never both record one leg. An audit
/// consumer restores the full picture of one invocation by keying
/// correlation records on [`ChainRecord::correlation_key`] - the root
/// invocation id and the depth - and counts exactly one root record per
/// invocation id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub struct ChainRecord {
    /// The wall-clock millisecond timestamp the leg completed under.
    pub ts_ms: u64,
    /// Whether this leg executed the root operation or a nested one.
    pub record_class: ChainRecordClass,
    /// Which process class executed the leg.
    pub leg: ChainLeg,
    /// The root invocation id every leg of this invocation shares.
    pub invocation_id: String,
    /// This leg's depth in the chain.
    pub depth: u32,
    /// The initiating principal of the chain, as attested at the root.
    pub initiating_identity: String,
    /// The identity of the handler that invoked this leg (the leg's own
    /// invoking identity; the initiating identity for the root leg).
    pub invoking_identity: String,
    /// The committed operation name this leg executed.
    pub operation: String,
    /// The Zone the invocation ran in.
    pub zone: String,
    /// How the leg ended.
    pub outcome: ChainOutcome,
    /// The closed refusal code, when the leg refused the call.
    pub code: Option<String>,
}

impl ChainRecord {
    /// The correlation key of this record: the root invocation id plus the
    /// leg's depth.
    ///
    /// Consumers group correlation records by this key; the root record of
    /// the invocation shares the id at depth zero.
    pub fn correlation_key(&self) -> (&str, u32) {
        (&self.invocation_id, self.depth)
    }

    /// Whether this record is the root record of its invocation.
    pub fn is_root(&self) -> bool {
        self.record_class == ChainRecordClass::Root
    }
}

/// The sink one leg's audit writer appends chain records to.
///
/// Each crate wires its own durable sink: the broker appends to its daily
/// audit file, the daemon to its daemon audit log. Tests install an
/// in-memory sink and assert on the records.
pub trait ChainAuditSink: Send + Sync {
    /// Append one chain record.
    fn record(&self, record: &ChainRecord) -> io::Result<()>;
}

/// The number of root records a set of records carries for one invocation
/// id.
///
/// The KTD6 consumer invariant: exactly one root record per root
/// invocation, no matter how many legs the chain ran or which process
/// class executed them.
pub fn root_record_count<'a>(
    records: impl IntoIterator<Item = &'a ChainRecord>,
    invocation_id: &str,
) -> usize {
    records
        .into_iter()
        .filter(|record| record.is_root() && record.invocation_id == invocation_id)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_root_chain_starts_at_depth_zero_with_the_initiating_identity() {
        let chain = EvidenceChain::root("invocation-1", "provider-alpha");
        assert_eq!(chain.root_invocation_id(), "invocation-1");
        assert_eq!(chain.depth(), 0);
        assert!(!chain.is_nested());
        assert_eq!(chain.initiating_identity(), "provider-alpha");
        assert_eq!(chain.invoking_identity(), "provider-alpha");
        assert_eq!(chain.identities(), ["provider-alpha"]);
    }

    #[test]
    fn a_nested_call_appends_the_invoking_identity_and_never_re_roots() {
        let root = EvidenceChain::root("invocation-1", "provider-alpha");
        let nested = root.nested("provider-beta");
        assert_eq!(nested.root_invocation_id(), "invocation-1");
        assert_eq!(nested.depth(), 1);
        assert!(nested.is_nested());
        assert_eq!(
            nested.initiating_identity(),
            "provider-alpha",
            "the initiating principal of the chain never changes"
        );
        assert_eq!(nested.invoking_identity(), "provider-beta");
        assert_eq!(
            nested.identities(),
            ["provider-alpha", "provider-beta"]
        );
        let deeper = nested.nested("provider-gamma");
        assert_eq!(deeper.depth(), 2);
        assert_eq!(deeper.invoking_identity(), "provider-gamma");
        assert_eq!(deeper.initiating_identity(), "provider-alpha");
    }

    #[test]
    fn the_root_invocation_id_survives_every_hop() {
        let root = EvidenceChain::root("invocation-9", "daemon");
        let mut chain = root;
        for _ in 0..5 {
            chain = chain.nested("provider-alpha");
        }
        assert_eq!(chain.root_invocation_id(), "invocation-9");
        assert_eq!(chain.depth(), 5);
    }

    #[test]
    fn the_record_shape_serializes_to_stable_greppable_keys() {
        let record = ChainRecord {
            ts_ms: 42,
            record_class: ChainRecordClass::Correlation,
            leg: ChainLeg::Daemon,
            invocation_id: "invocation-1".to_owned(),
            depth: 1,
            initiating_identity: "provider-alpha".to_owned(),
            invoking_identity: "provider-beta".to_owned(),
            operation: "BetaService".to_owned(),
            zone: "zone-a".to_owned(),
            outcome: ChainOutcome::Refused,
            code: Some("nested-depth-exceeded".to_owned()),
        };
        let json = serde_json::to_value(&record).expect("the record serializes");
        assert_eq!(json["record_class"], "correlation");
        assert_eq!(json["leg"], "daemon");
        assert_eq!(json["invocation_id"], "invocation-1");
        assert_eq!(json["depth"], 1);
        assert_eq!(json["initiating_identity"], "provider-alpha");
        assert_eq!(json["invoking_identity"], "provider-beta");
        assert_eq!(json["operation"], "BetaService");
        assert_eq!(json["outcome"], "refused");
        assert_eq!(json["code"], "nested-depth-exceeded");
        // The record round-trips through the wire/JSONL shape.
        let parsed: ChainRecord =
            serde_json::from_value(json).expect("the record deserializes");
        assert_eq!(parsed, record);
        assert_eq!(parsed.correlation_key(), ("invocation-1", 1));
        assert!(!parsed.is_root());
    }

    #[test]
    fn root_records_are_counted_once_per_invocation_across_both_legs() {
        let root_broker = ChainRecord {
            ts_ms: 1,
            record_class: ChainRecordClass::Root,
            leg: ChainLeg::Broker,
            invocation_id: "invocation-1".to_owned(),
            depth: 0,
            initiating_identity: "provider-alpha".to_owned(),
            invoking_identity: "provider-alpha".to_owned(),
            operation: "AlphaService".to_owned(),
            zone: "zone-a".to_owned(),
            outcome: ChainOutcome::Succeeded,
            code: None,
        };
        let correlation_daemon = ChainRecord {
            record_class: ChainRecordClass::Correlation,
            leg: ChainLeg::Daemon,
            invocation_id: "invocation-1".to_owned(),
            depth: 1,
            operation: "BetaService".to_owned(),
            ..root_broker.clone()
        };
        let other_root = ChainRecord {
            record_class: ChainRecordClass::Root,
            invocation_id: "invocation-2".to_owned(),
            ..root_broker.clone()
        };
        let records = vec![
            root_broker,
            correlation_daemon,
            other_root.clone(),
            other_root,
        ];
        assert_eq!(root_record_count(&records, "invocation-1"), 1);
        assert_eq!(root_record_count(&records, "invocation-2"), 2);
        assert_eq!(root_record_count(&records, "invocation-3"), 0);
        // A nested leg never writes a root record for its own leg: the
        // correlation record keys on the id and depth, and the count stays
        // one for the invocation the id names.
        let only_nested = records
            .iter()
            .filter(|record| record.record_class == ChainRecordClass::Correlation)
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(root_record_count(&only_nested, "invocation-1"), 0);
    }
}