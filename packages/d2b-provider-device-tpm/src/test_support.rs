//! Test-support doubles for the TPM effects facet set.
//!
//! Gated behind the `test-support` Cargo feature (or `cfg(test)`) so
//! production consumers never pull this in; `d2bd`'s plane tests build a
//! facet set through this module.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_core::bundle_resolver::BundleResolver;

use crate::facets::{TpmEffectFacets, TpmRuntime};
use crate::resource_effect::TpmResourceEffectError;

/// A fail-closed [`TpmRuntime`] double: every daemon-state read refuses
/// closed by name, so the plane tests never silently pass a port that would
/// have reached daemon state.
pub struct FailClosedRuntime;

#[async_trait::async_trait]
impl TpmRuntime for FailClosedRuntime {
    fn broker_socket_path(&self) -> &Path {
        Path::new("/run/d2b/d2b-broker.sock")
    }

    fn caller_role(&self) -> BrokerCallerRole {
        BrokerCallerRole::AdminUid { uid: 0 }
    }

    fn kernel_io_timeout(&self) -> Duration {
        Duration::from_secs(5)
    }

    async fn load_bundle(
        &self,
    ) -> Result<Arc<BundleResolver>, TpmResourceEffectError> {
        Err(TpmResourceEffectError::Transient)
    }

    async fn consume_lifecycle_lease(
        &self,
        _vm_id: &str,
        _operation_id: &str,
    ) -> Result<(), TpmResourceEffectError> {
        Err(TpmResourceEffectError::Transient)
    }
}

/// Build a TPM facet set from the fail-closed runtime double.
pub fn recording_facets() -> TpmEffectFacets {
    TpmEffectFacets {
        runtime: Arc::new(FailClosedRuntime),
    }
}