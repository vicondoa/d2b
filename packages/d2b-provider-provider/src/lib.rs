//! The Provider provider crate: the `Provider` resource type's driver, its
//! spec decoder, its effect port, and its driver declaration.
//!
//! The crate owns the `Provider` type's complete resource knowledge: the
//! readiness observation the driver applies to a Provider's owned controller
//! `Process` rows and state `Volume` rows, the phases the pure provider policy
//! projects, the fixed provider identities the two internally hosted providers
//! observe, and the [`DriverDescriptor`](d2b_resource_types::DriverDescriptor)
//! the plane registers the type by.
//!
//! Everything the driver needs from outside arrives through the driver effect
//! port ([`ProviderDriverEffects`]): the live controller-session evidence the
//! manager cannot serve, because a converted row's status is in-memory only.
//! The production implementation lives in the daemon behind that port, so this
//! crate depends on no daemon type.


mod driver;
pub mod providers;

pub use driver::{
    FailClosedProviderDriverEffects, PROVIDER_TYPE_NAME, ProviderDriver, ProviderDriverArgs,
    ProviderDriverEffects, ProviderDriverFactory, ProviderDriverStatus, SYSTEM_CORE_HOST_REF,
    SYSTEM_CORE_PROVIDER_REF, SYSTEM_MINIJAIL_PROVIDER_REF, provider_descriptor,
    provider_spec_decoder,
};
