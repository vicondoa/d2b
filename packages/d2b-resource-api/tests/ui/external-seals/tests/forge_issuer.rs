use d2b_resource_api::{AdmissionIssuer, VerifiedMutation};

fn probe() {
    let _ = core::mem::size_of::<AdmissionIssuer>();
}

fn forge_verified_mutation() {
    let _ = VerifiedMutation {};
}
