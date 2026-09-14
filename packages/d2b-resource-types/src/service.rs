//! Zone/session service metadata a provider declares.

/// One service a provider serves.
///
/// The declaration is the service metadata source the session layer and the
/// CLI read: both resolve a service to its declaring driver from here instead
/// of keeping per-type service tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceDecl {
    /// The service identity the session layer addresses.
    pub id: &'static str,
    /// The request methods the service answers.
    pub methods: &'static [&'static str],
    /// The attachment kinds the service accepts.
    pub attach_kinds: &'static [&'static str],
    /// The stream kinds the service exposes.
    pub streams: &'static [&'static str],
    /// The endpoint policy the session layer enforces for the service, when
    /// the service declares one rather than the layer's default.
    pub endpoint_policy: Option<&'static str>,
}
