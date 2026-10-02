//! An in-process authority publisher, for composition that has no broker.
//!
//! # What this is and is not
//!
//! It is a [`AuthorityPublisher`] that accepts every candidate, names a
//! prepared identity derived from the transaction, and answers each commit
//! with the revision it was given. It is a stand-in for the *broker's*
//! answers, not a second authority: nothing here decides whether a mutation
//! is admissible, and every fact it hands back is the one the store already
//! projected. A composition that published through it in production would be
//! recording a fence that no broker holds, so production binds the daemon's
//! origination leg instead and this stays behind a feature.
//!
//! It is also the surface a test uses to assert the protocol itself: the
//! fences it recorded are the durable identities a restart has to adopt, and
//! the refusals a test arms are the ones a Zone must stay fenced under.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::ZoneDesiredSequence;
use tokio::sync::Mutex;

use crate::authority_journal::{CommitOutcome, CommittedPublication};
use crate::identity::TransactionId;
use crate::authority_publish::{
    AcceptedRevision, AuthorityPublisher, FencedTransaction, PublicationCandidate,
    PublicationRefusal, ZoneProjection,
};

/// The module declared name.
pub const MODULE_NAME: &str = "test_support";

/// One recorded publication round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recorded {
    /// A candidate this publisher fenced.
    Prepared {
        transaction: TransactionId,
        committed: ZoneDesiredSequence,
    },
    /// A committed state this publisher accepted.
    Accepted {
        transaction: TransactionId,
        sequence: ZoneDesiredSequence,
    },
    /// One reconciliation this publisher was shown.
    ///
    /// A stand-in holds no cached authority of its own to prove, so what it
    /// records is the projection the manager presented: the Zone it described,
    /// the accepted cursor it restated, how many committed rows it carried, and
    /// the transaction it carried forward.
    Resynchronized {
        zone: String,
        sequence: ZoneDesiredSequence,
        rows: usize,
        outstanding: Option<TransactionId>,
    },
}

/// An accepting authority publisher that records what it was asked to do.
#[derive(Debug, Default)]
pub struct RecordingPublisher {
    recorded: Mutex<Vec<Recorded>>,
}

impl RecordingPublisher {
    /// A publisher that accepts every candidate.
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Every round trip this publisher has answered, in order.
    pub async fn recorded(&self) -> Vec<Recorded> {
        self.recorded.lock().await.clone()
    }

    /// The answer this publisher gives one committed state. A test composes
    /// the store's half around it exactly as the manager does.
    pub async fn accept(&self, publication: &CommittedPublication) -> AcceptedRevision {
        <Self as AuthorityPublisher>::commit(self, publication)
            .await
            .expect("the recording publisher accepts every commit")
    }

    async fn record(&self, entry: Recorded) {
        self.recorded.lock().await.push(entry);
    }
}

#[async_trait]
impl AuthorityPublisher for RecordingPublisher {
    async fn prepare(
        &self,
        candidate: &PublicationCandidate,
    ) -> Result<FencedTransaction, PublicationRefusal> {
        self.record(Recorded::Prepared {
            transaction: candidate.transaction,
            committed: candidate.committed,
        })
        .await;
        Ok(FencedTransaction {
            transaction: candidate.transaction,
            prepared: format!("prepared-{}", candidate.transaction),
            committed: candidate.committed,
        })
    }

    async fn commit(
        &self,
        publication: &CommittedPublication,
    ) -> Result<AcceptedRevision, PublicationRefusal> {
        self.record(Recorded::Accepted {
            transaction: publication.transaction,
            sequence: publication.sequence,
        })
        .await;
        Ok(AcceptedRevision {
            transaction: publication.transaction,
            sequence: publication.sequence,
            candidate: publication.candidate.clone(),
        })
    }

    async fn accepted(&self) -> Result<ZoneDesiredSequence, PublicationRefusal> {
        Ok(ZoneDesiredSequence::INITIAL)
    }

    async fn resynchronize(&self, projection: &ZoneProjection) -> Result<(), PublicationRefusal> {
        self.record(Recorded::Resynchronized {
            zone: projection.zone.clone(),
            sequence: projection.accepted.sequence,
            rows: projection.rows.len(),
            outstanding: projection.outstanding,
        })
        .await;
        Ok(())
    }
}

/// An authority publisher that refuses every fence.
///
/// A Zone with no reachable broker is exactly this posture: new effects stay
/// refused, and nothing is committed against a fence that does not exist.
#[derive(Debug, Default)]
pub struct RefusingPublisher {
    detail: String,
}

impl RefusingPublisher {
    /// A publisher that refuses with `detail`.
    pub fn new(detail: impl Into<String>) -> Arc<Self> {
        Arc::new(Self { detail: detail.into() })
    }
}

#[async_trait]
impl AuthorityPublisher for RefusingPublisher {
    async fn prepare(
        &self,
        _candidate: &PublicationCandidate,
    ) -> Result<FencedTransaction, PublicationRefusal> {
        Err(PublicationRefusal::Refused(self.detail.clone()))
    }

    async fn commit(
        &self,
        _publication: &CommittedPublication,
    ) -> Result<AcceptedRevision, PublicationRefusal> {
        Err(PublicationRefusal::Refused(self.detail.clone()))
    }

    async fn accepted(&self) -> Result<ZoneDesiredSequence, PublicationRefusal> {
        Err(PublicationRefusal::Refused(self.detail.clone()))
    }

    async fn resynchronize(&self, _projection: &ZoneProjection) -> Result<(), PublicationRefusal> {
        Err(PublicationRefusal::Refused(self.detail.clone()))
    }
}

/// The committed rows one outcome installed, for a test asserting what a
/// publication carried.
pub fn committed_rows(outcome: &CommitOutcome) -> &[crate::authority_journal::DesiredRow] {
    match outcome {
        CommitOutcome::Committed(committed) | CommitOutcome::AlreadyCommitted(committed) => {
            &committed.publication.rows
        }
        CommitOutcome::Unchanged { .. } => &[],
    }
}