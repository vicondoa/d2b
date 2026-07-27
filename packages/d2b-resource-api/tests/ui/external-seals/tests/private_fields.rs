use d2b_resource_api::service::TrustedRequest;
use d2b_resource_api::{AdmittedMutation, AuthenticatedSubjectContext};

fn admission(value: &AdmittedMutation) {
    let _ = &value.mutations;
}

fn subject(value: &AuthenticatedSubjectContext) {
    let _ = &value.claims;
}

fn trusted<T>(value: &TrustedRequest<T>) {
    let _ = &value.subject;
}
