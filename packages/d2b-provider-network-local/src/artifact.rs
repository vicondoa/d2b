//! Net-VM artifact catalog resolution and generic-system identity projection.

use d2b_contracts::v3::{execution_policy::BoundedToken, network::NetworkSpec};

/// Artifact kinds accepted by the network-local resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactKind {
    /// A Provider package.
    Provider,
    /// A bootable NixOS system closure.
    NixosSystem,
}

/// One private artifact catalog row.
#[derive(Clone, PartialEq, Eq)]
pub struct ArtifactCatalogEntry {
    artifact_id: BoundedToken,
    kind: ArtifactKind,
}

impl ArtifactCatalogEntry {
    /// Construct one catalog row without accepting a store path.
    pub const fn new(artifact_id: BoundedToken, kind: ArtifactKind) -> Self {
        Self { artifact_id, kind }
    }

    /// Borrow the plain artifact ID.
    pub const fn artifact_id(&self) -> &BoundedToken {
        &self.artifact_id
    }
}

impl core::fmt::Debug for ArtifactCatalogEntry {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ArtifactCatalogEntry(<redacted>)")
    }
}

/// Value-free artifact resolution failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactResolutionError {
    /// The required ID has no catalog row.
    Missing,
    /// The row is not a NixOS system.
    TypeMismatch,
}

impl ArtifactResolutionError {
    /// Return the stable build or reconcile reason.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Missing => "net-vm-artifact-missing",
            Self::TypeMismatch => "artifact-type-mismatch",
        }
    }
}

impl core::fmt::Display for ArtifactResolutionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ArtifactResolutionError {}

/// Resolve the required Network artifact ID to a NixOS system catalog row.
pub fn resolve_net_vm_system_artifact(
    spec: &NetworkSpec,
    catalog: &[ArtifactCatalogEntry],
) -> Result<BoundedToken, ArtifactResolutionError> {
    let requested = spec.net_vm_system_artifact_id();
    let entry = catalog
        .iter()
        .find(|entry| entry.artifact_id() == requested)
        .ok_or(ArtifactResolutionError::Missing)?;
    if entry.kind != ArtifactKind::NixosSystem {
        return Err(ArtifactResolutionError::TypeMismatch);
    }
    Ok(entry.artifact_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::network::Ipv4Cidr;

    fn spec() -> NetworkSpec {
        NetworkSpec::minimal(
            Ipv4Cidr::parse("10.20.0.0/24").unwrap(),
            Ipv4Cidr::parse("192.0.2.0/30").unwrap(),
            BoundedToken::parse("net-vm-base").unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn requires_declared_nixos_system() {
        assert_eq!(
            resolve_net_vm_system_artifact(&spec(), &[]),
            Err(ArtifactResolutionError::Missing)
        );
        assert_eq!(
            resolve_net_vm_system_artifact(
                &spec(),
                &[ArtifactCatalogEntry::new(
                    BoundedToken::parse("net-vm-base").unwrap(),
                    ArtifactKind::Provider,
                )],
            ),
            Err(ArtifactResolutionError::TypeMismatch)
        );
    }
}
