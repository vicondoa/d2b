//! Resource-API watch ownership over the concrete store stream.
//!
//! The wire service returns a stream name and snapshot revision, while the
//! authenticated bus owns the actual delivery task.  This module keeps that
//! handoff explicit: registration and replay are performed by the store
//! actor, and acknowledgements return to that same actor.

use std::sync::Arc;

use d2b_contracts::v3::ZoneRevision;
use d2b_resource_store::{StoreError, StoreWatchReceipt, StoreWatchRequest};
use d2b_resource_store_redb::{
    RedbResourceStore, SharedChangeBatch, WatchRegistrationId, WatchSignals, WatchStream,
};

/// One authenticated resource watch with an owned delivery stream.
pub struct ResourceWatch {
    store: Arc<RedbResourceStore>,
    receipt: StoreWatchReceipt,
    stream: WatchStream,
}

impl core::fmt::Debug for ResourceWatch {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ResourceWatch")
            .field("registration", &self.stream.id())
            .field("receipt", &self.receipt)
            .finish()
    }
}

impl ResourceWatch {
    fn new(store: Arc<RedbResourceStore>, receipt: StoreWatchReceipt, stream: WatchStream) -> Self {
        Self {
            store,
            receipt,
            stream,
        }
    }

    /// The receipt returned to the authenticated stream owner.
    pub const fn receipt(&self) -> &StoreWatchReceipt {
        &self.receipt
    }

    /// The opaque registration id used for acknowledgements.
    pub const fn id(&self) -> WatchRegistrationId {
        self.stream.id()
    }

    /// Receive the next shared immutable change batch.
    pub async fn recv(&mut self) -> Option<SharedChangeBatch> {
        self.stream.recv().await
    }

    /// Acknowledge all batches through `revision`.
    pub async fn acknowledge(&self, revision: ZoneRevision) -> Result<(), StoreError> {
        self.store.acknowledge_watch(self.id(), revision).await
    }

    /// Explicitly close the watch and release its global budget.
    pub async fn close(self) -> Result<Option<ZoneRevision>, StoreError> {
        let id = self.id();
        self.store.unregister_watch(id).await
    }
}

impl Drop for ResourceWatch {
    fn drop(&mut self) {
        self.store.unregister_watch_now(self.id());
    }
}

/// Resource-API watch adapter for one already-authorized Zone store.
pub struct WatchService {
    store: Arc<RedbResourceStore>,
}

impl core::fmt::Debug for WatchService {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("WatchService(<redacted>)")
    }
}

impl WatchService {
    /// Bind the adapter to the one store selected by the authenticated Zone.
    pub const fn new(store: Arc<RedbResourceStore>) -> Self {
        Self { store }
    }

    /// Register, replay, and return an owned stream without a replay/live gap.
    pub async fn open(&self, request: StoreWatchRequest) -> Result<ResourceWatch, StoreError> {
        let (receipt, stream) = self.store.watch_stream(request).await?;
        Ok(ResourceWatch::new(Arc::clone(&self.store), receipt, stream))
    }

    /// Return the fixed-cardinality store watch saturation snapshot.
    pub fn signals(&self) -> Result<WatchSignals, StoreError> {
        self.store.watch_signals()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn watch_adapter_has_no_public_selector_or_path_surface() {
        let source = include_str!("watch.rs");
        assert!(!source.contains("host_path"));
        assert!(!source.contains("path_template"));
        assert!(source.contains("acknowledge"));
        assert!(source.contains("unregister_watch_now"));
    }
}
