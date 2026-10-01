//! The CredentialBinding provider crate: the `CredentialBinding` resource
//! type's driver, its spec decoder, its delivery effect port, the
//! provider-owned implementation of that port, and the read-side helpers
//! over one stored binding row.
//!
//! The crate owns the row side of one admitted credential delivery. The
//! source side - who may use the credential, for what, and for how long - is
//! the `Credential` provider's own realization and is not restated here; what
//! is here is everything a committed `CredentialBinding` row needs in order to
//! be served: the identity one delivery is admitted under, the effects that
//! establish, observe, and retire it, and the fenced projection a reader
//! accepts as evidence that a delivery is live.
//!
//! The driver owns the whole relationship: `validate_spec` decodes the row
//! strictly and checks its owner fence, reconcile delivers to the exact
//! destination and publishes the fenced readiness projection, `observe`
//! adopts a delivery that is already live instead of minting a second one,
//! and the drain and teardown steps revoke the delivery idempotently.
//!
//! Credential material never enters this crate. The delivery request holds
//! identity, vocabulary, counters, and bounds, and it is deliberately not
//! serializable; the material is minted and kept inside the admitted
//! delivery session on the far side of the effect port, which is the only
//! component that ever sees it.
//!
//! The family's driver effects are implemented by this crate itself
//! ([`crate::effects_service`]) over the daemon-supplied declared facets
//! ([`crate::facets`]): the delivery, the observation of a live delivery, the
//! revocation, and the clock. The daemon hosts the family's declared effects
//! service ([`crate::effects_service::CREDENTIAL_BINDING_EFFECTS_SERVICE`])
//! per zone from the family's registered factory, and no externally built
//! port appears at any construction site.

#![deny(missing_docs)]

#[cfg(any(test, feature = "test-support"))]
/// Recording test doubles shared with this crate's and downstream crates'
/// unit tests, gated behind the `test-support` Cargo feature so production
/// consumers never pull them in.
pub mod test_support;

mod driver;
mod effects_service;
mod facets;
mod row_readers;

pub use driver::{
    CREDENTIAL_BINDING_CREATIONS, CREDENTIAL_BINDING_EXECUTION_DOMAINS,
    CREDENTIAL_BINDING_PROVIDER_REF, CREDENTIAL_BINDING_READS, CREDENTIAL_BINDING_TYPE_NAME,
    CredentialBindingDriverArgs, CredentialBindingDriverEffects, CredentialBindingDriverStatus,
    CredentialBindingReadinessFence, CredentialBindingStatusResource, CredentialDelivery,
    CredentialRevocation, DeliveredSession, binding_descriptor, binding_spec_decoder,
};
pub use effects_service::{
    CREDENTIAL_BINDING_EFFECTS_SERVICE, CredentialBindingEffectsService,
    CredentialBindingEffectsServiceFactory,
};
pub use facets::{
    CredentialBindingEffectFacets, CredentialClock, CredentialDeliverySource,
    CredentialRevocationSource,
};
pub use row_readers::{binding_readiness_current, parsed_binding_spec, stored_binding_status};
