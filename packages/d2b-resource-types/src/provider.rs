//! Provider identity and the zone-level declarations a provider makes.

/// The declaration one provider makes about itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderDeclaration {
    /// The provider identity every declaration row references.
    pub provider_ref: &'static str,
    /// The role bindings scoped to the provider itself.
    ///
    /// A self-binding is structurally scoped: its subject must be the
    /// declaring provider and its role must be one the provider declares for
    /// itself. A binding that escapes that scope fails the manifest check.
    pub self_bindings: &'static [SelfBinding],
    /// Whether the zone requires the provider to be present.
    pub required: bool,
    /// How many instances of the provider a zone may run.
    pub cardinality: Cardinality,
    /// The isolation posture the provider runs under.
    pub isolation_posture: IsolationPosture,
    /// The plane adapters the provider requires.
    pub plane_adapters: &'static [PlaneAdapter],
    /// The principals the provider uses, declared by name only.
    pub principals: &'static [PrincipalName],
    /// The storage roots the provider declares.
    pub storage_roots: &'static [StorageRoot],
}

/// One structurally scoped self-binding of a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelfBinding {
    /// The binding subject, which must be the declaring provider.
    pub subject_ref: &'static str,
    /// The role the subject is bound to, declared by the provider itself.
    pub role_ref: &'static str,
}

/// How many instances of a provider a zone may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Cardinality {
    /// At most one instance per zone.
    AtMostOne,
    /// Any number of instances per zone.
    Many,
}

/// The isolation posture a provider runs under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IsolationPosture {
    /// Isolated execution behind the standard sandbox.
    Standard,
    /// Local execution without the standard sandbox; a declaration has to opt
    /// in explicitly.
    UnsafeLocal,
}

/// One plane adapter a provider requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaneAdapter {
    /// The adapter identity.
    pub id: &'static str,
    /// The ids of the adapters this adapter must run after.
    ///
    /// The composition root derives attach and drain order from these edges:
    /// an adapter attaches only after every declared dependency has attached,
    /// and drains before it.
    pub depends_on: &'static [&'static str],
}

/// One principal a provider declares, by name only.
///
/// Numeric identity is not declared: the generator allocates uid/gid for each
/// name and persists the allocation in the committed catalog, so a
/// declaration cannot pin a colliding or host-specific id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrincipalName(pub &'static str);

/// One storage root a provider declares.
///
/// Invariant: every declared path stays inside the provider's declared
/// subtree. The generator refuses roots that overlap another provider's
/// subtree or escape through path syntax, so a provider never names storage
/// it does not own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StorageRoot {
    /// The declared subtree path.
    pub path: &'static str,
    /// Whether the provider owns the root and manages its contents, rather
    /// than consuming a root provisioned for it.
    pub provider_owned: bool,
}
