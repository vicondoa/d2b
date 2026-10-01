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
//! revocation call. The family's own effects implementation
//! ([`effects_service`], U8) serves those ports over the daemon-supplied
//! declared facets ([`facets`]) and is hosted per zone as the declared
//! `credential.d2bus.org/effects` service, so this crate depends on no daemon
//! runtime. The three Credential realizers stay separate crates
//! (`d2b-provider-credential-secret-service`, `-entra`,
//! `-managed-identity`); the family consumes their exported Provider
//! identities so its admission set cannot drift from the Providers that
//! declare them, and the agent child it mints is declared as a
//! [`ChildCreation`](d2b_resource_types::ChildCreation) under the minijail
//! Process Provider's own exported reference.
//!
//! The crate also owns the `CredentialBinding` realization
//! ([`binding`]): the source-side policy a `Credential` row admits delivery
//! under, and the typed private delivery authority that mints a delivery
//! session only while the admitted audience, operations, component
//! generation, dependency revisions, and lifetime still hold. No credential
//! material leaves that module: the graph spec, the generic binding status,
//! the audit record, and the publication snapshot carry identity, policy,
//! and state only.

#![deny(missing_docs)]

mod binding;
mod driver;
mod effects_service;
mod facets;
mod session;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;

pub use binding::{
    CREDENTIAL_DELIVERY_SLOT, CanonicalCredentialBinding, CredentialBindingAdmission,
    CredentialBindingStatus, CredentialDeliveryAuthority, CredentialDeliveryEvidence,
    CredentialDeliveryFence, CredentialDeliveryLeg, CredentialDeliveryRefusal,
    canonical_binding_rows, credential_binding_row_name, credential_binding_support,
    credential_source_decision, delivery_operation,
};
pub use driver::{
    CONTROLLER_PROVIDER_GENERATION_ANNOTATION, CONTROLLER_PROVIDER_REF_ANNOTATION,
    CONTROLLER_PROVIDER_UID_ANNOTATION, CREDENTIAL_TYPE_NAME, CredentialDependencyFacts,
    CredentialDriver, CredentialDriverArgs, CredentialDriverEffects, CredentialDriverError,
    CredentialDriverFactory, CredentialDriverStatus, CredentialLeaseFacts, CredentialSourcePolicy,
    credential_descriptor, credential_spec_decoder,
};
pub use effects_service::{
    CREDENTIAL_EFFECTS_SERVICE, CredentialEffectsService, CredentialEffectsServiceFactory,
};
pub use facets::{
    AgentReadyFuture, CredentialEffectFacets, CredentialRuntime, DependencyFactsFuture,
    LeaseFactsFuture,
};
pub use session::{
    CredentialResourceRuntimeError, CredentialRevocationEvidence, CredentialRevocationInputs,
    CredentialRevocationOutcome, CredentialRevocationReport, CredentialRevocationRequest,
    CredentialSession, credential_provider_kind, is_credential_provider_ref,
};

/// The wire backend identity of the Entra backend Provider, as declared by
/// `d2b-provider-credential-entra`.
pub const ENTRA_BACKEND_REF: &str = d2b_provider_credential_entra::BACKEND_REF;
/// The wire backend identity of the Managed Identity backend Provider, as
/// declared by `d2b-provider-credential-managed-identity`.
pub const MANAGED_IDENTITY_BACKEND_REF: &str = d2b_provider_credential_managed_identity::BACKEND_REF;
/// The wire backend identity of the Secret Service backend Provider, as
/// declared by `d2b-provider-credential-secret-service`.
pub const SECRET_SERVICE_BACKEND_REF: &str = d2b_provider_credential_secret_service::BACKEND_REF;
/// The canonical Provider reference of the Entra backend Provider.
pub const ENTRA_PROVIDER_REF: &str = d2b_provider_credential_entra::PROVIDER_REF;
/// The canonical Provider reference of the Managed Identity backend Provider.
pub const MANAGED_IDENTITY_PROVIDER_REF: &str = d2b_provider_credential_managed_identity::PROVIDER_REF;
/// The canonical Provider reference of the Secret Service backend Provider.
pub const SECRET_SERVICE_PROVIDER_REF: &str = d2b_provider_credential_secret_service::PROVIDER_REF;

/// The signed agent binary the managed-identity backend's co-located
/// client-holding agent Process runs, as declared by
/// `d2b-provider-credential-managed-identity`.
pub const CREDENTIAL_AGENT_BINARY: &str = d2b_provider_credential_managed_identity::AGENT_BINARY;
