//! The declared facets the provider-owned User effects service reaches
//! daemon state through (U5).
//!
//! The User family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]) over the bounded local
//! account probe this crate owns (see [`crate::probe`]). The family reaches
//! only host state: the probe reads the local NSS account database directly
//! through `nix`'s bounded `getpwnam`/`getgrnam` surface. There is no
//! daemon-structural read today, so the declared facet set is empty - it
//! stays the declared boundary the construction sites take, so a
//! daemon-held read rides the same seam rather than a daemon call (R2).

/// The daemon-supplied facet set the provider-owned User effects are built
/// from (U5).
///
/// The composition root supplies the set; the driver never holds a daemon
/// state type (R2). Every probe input is host state the crate reads itself,
/// so the set is empty; it mirrors the Host family's facet-set shape as the
/// declared facet seam.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UserEffectFacets {}