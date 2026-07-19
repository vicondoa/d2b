//! The former realm facade was removed.
//!
//! Realm controller requests are served by
//! [`d2b_realm_router::service_v2::RealmServiceProcess`] over an authenticated
//! ComponentSession. Production code must not route through a local executor or
//! negotiated codec facade.
