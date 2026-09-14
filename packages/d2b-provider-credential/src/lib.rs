//! The Credential provider crate: the Credential resource type's driver, its
//! spec decoder, its driver declaration, and the revocations and sessions the
//! type's teardown binds.
//!
//! The crate owns the Credential type's complete resource knowledge: the three
//! Credential Providers the plane serves (secret-service, entra, and
//! managed-identity), the admission rules that classify a stored spec onto
//! them, the managed-identity agent Process child the driver mints, and the
//! driver's validate, recover, reconcile, finalize, and delete verbs.
//!
//! Everything the driver needs from outside arrives through its ports:
//! [`CredentialDriverEffects`] for the Provider facts, the lease facts, and
//! the agent probe, and [`CredentialSession`] for the authenticated Provider
//! revocation call. The production implementations stay in the daemon behind
//! those ports, so this crate depends on no daemon runtime. The three
//! Credential realizers stay separate crates
//! (`d2b-provider-credential-secret-service`, `-entra`,
//! `-managed-identity`); the family consumes their exported Provider
//! identities so its admission set cannot drift from the Providers that
//! declare them, and the agent child it mints is declared as a
//! [`ChildCreation`](d2b_resource_types::ChildCreation) under the minijail
//! Process Provider's own exported reference.

#![deny(missing_docs)]

mod driver;
mod session;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use driver::{
    CONTROLLER_PROVIDER_GENERATION_ANNOTATION, CONTROLLER_PROVIDER_REF_ANNOTATION,
    CONTROLLER_PROVIDER_UID_ANNOTATION, CREDENTIAL_TYPE_NAME, CredentialDependencyFacts,
    CredentialDriver, CredentialDriverArgs, CredentialDriverEffects, CredentialDriverError,
    CredentialDriverFactory, CredentialDriverStatus, CredentialLeaseFacts, credential_descriptor,
    credential_spec_decoder,
};
pub use session::{
    CredentialResourceRuntimeError, CredentialRevocationEvidence, CredentialRevocationInputs,
    CredentialRevocationOutcome, CredentialRevocationRequest, CredentialSession,
    credential_provider_kind, is_credential_provider_ref,
};
