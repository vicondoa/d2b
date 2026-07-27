use d2b_resource_api::service::TrustedRequest;
use d2b_resource_api::{
    AdmissionVerifier, AdmittedMutation, AuthenticatedSubjectContext, StoreIdentity,
};

fn verifier(value: &AdmissionVerifier) {
    let _ = &value.authority;
}

fn store_identity(value: &StoreIdentity) {
    let _ = &value.authority;
}

fn admission(value: &AdmittedMutation) {
    let _ = &value.mutations;
}

fn subject(value: &AuthenticatedSubjectContext) {
    let _ = &value.claims;
}

fn trusted<T>(value: &TrustedRequest<T>) {
    let _ = &value.subject;
}
