use std::sync::Arc;

use d2b_resource_api::{AdmissionVerifier, StoreIdentity};

struct Backend {
    verifier: Arc<AdmissionVerifier>,
    identity: Arc<StoreIdentity>,
}

fn share_both_tokens(
    verifier: AdmissionVerifier,
    identity: StoreIdentity,
) -> (Backend, Backend) {
    let verifier = Arc::new(verifier);
    let identity = Arc::new(identity);
    (
        Backend {
            verifier: Arc::clone(&verifier),
            identity: Arc::clone(&identity),
        },
        Backend { verifier, identity },
    )
}

fn main() {}
