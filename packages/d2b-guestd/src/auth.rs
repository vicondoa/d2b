//! Guest authentication is the ComponentSession static-identity boundary.

pub use crate::service_v2::{
    GuestSessionAuthority, GuestSessionError, GuestSessionMaterial, GuestStaticIdentity,
    SealedIdentityStore, TpmSealedIdentityStore,
};
