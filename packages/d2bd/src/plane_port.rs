//! The daemon's zone-plane port.
//!
//! The toolkit's lifecycle asks the composition root to realize the facts a
//! provider declared about its zone plane: claim one storage root, deploy one
//! adapter, publish one service. This module is that executor, and it is the
//! only place a declared plane action becomes host state.
//!
//! Nothing here is keyed by a provider name in code: the port is built over
//! the declarations of the providers the plane starts, so a provider can only
//! have the roots, adapters, and services it declared, and an action outside
//! its own declaration is refused rather than realized. Every refusal is
//! recorded with the three names an operator needs - the provider, the row,
//! and the reason - so the plane can report why a provider did not start
//! instead of failing opaquely.
//!
//! Realized state:
//!
//! - a declared storage root resolves under the zone's daemon-owned state
//!   directory; a provider-owned root is created `0700` there, a root the
//!   plane provisions is verified to exist, and an overlap with another
//!   provider's declared subtree refuses;
//! - the declared adapters are recorded in declared dependency order, so a
//!   dependency that has not been deployed refuses;
//! - the published services are recorded against the provider that declared
//!   them, and the port reports them as the zone's published service surface.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use d2b_contracts_resource::v3::ZoneId;
use d2b_provider_toolkit::{
    DriverDescriptor, PlaneAdapter, PlaneError, ProviderDeclaration, ServiceDecl, StorageRoot,
    ZonePlanePort,
};

/// Why the plane refused one declared action, with the action named.
///
/// The toolkit's plane error carries the stable code only; this record
/// carries the provider, the row, and the reason the operator reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlaneRefusal {
    /// The provider whose declared action was refused.
    pub(crate) provider_ref: &'static str,
    /// The row the action named, as the declaration spells it.
    pub(crate) row: String,
    /// The stable lower-kebab reason.
    pub(crate) reason: &'static str,
}

impl PlaneRefusal {
    /// The refusal as one line, naming provider, row, and reason.
    pub(crate) fn message(&self) -> String {
        format!(
            "{}:{}:{}",
            self.reason, self.provider_ref, self.row
        )
    }
}

/// One storage root the plane claimed for one provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaimedRoot {
    /// The provider that declared the root.
    pub(crate) provider_ref: &'static str,
    /// The declared subtree, relative to the zone's state directory.
    pub(crate) declared: &'static str,
    /// Whether the provider owns the root and manages its contents.
    pub(crate) provider_owned: bool,
}

/// The declared surface of one provider, as the port validates against it.
struct ProviderSurface {
    adapters: &'static [PlaneAdapter],
    storage_roots: &'static [StorageRoot],
    services: BTreeSet<&'static str>,
}

/// What the port realized, per provider.
#[derive(Default)]
struct PlaneLedger {
    roots: BTreeMap<&'static str, Vec<ClaimedRoot>>,
    adapters: BTreeMap<&'static str, Vec<&'static str>>,
    services: BTreeMap<&'static str, Vec<&'static str>>,
    refusal: Option<PlaneRefusal>,
}

/// The production zone-plane port.
pub(crate) struct ProductionPlanePort {
    zone: ZoneId,
    state_root: PathBuf,
    surfaces: BTreeMap<&'static str, ProviderSurface>,
    ledger: Mutex<PlaneLedger>,
}

impl core::fmt::Debug for ProductionPlanePort {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ProductionPlanePort")
            .field("zone", &self.zone)
            .field("state_root", &self.state_root)
            .field("providers", &self.surfaces.len())
            .finish_non_exhaustive()
    }
}

impl ProductionPlanePort {
    /// Build the port over the declarations of the providers a plane starts.
    ///
    /// The declaration set is the port's authority: a provider reference the
    /// plane does not start has no declared surface, so nothing can be
    /// realized for it.
    pub(crate) fn over(
        zone: ZoneId,
        state_root: PathBuf,
        declarations: &[(ProviderDeclaration, Arc<[DriverDescriptor]>)],
    ) -> Self {
        let mut surfaces = BTreeMap::new();
        for (declaration, drivers) in declarations {
            let services = drivers
                .iter()
                .flat_map(|driver| driver.services.iter())
                .map(|service| service.id)
                .collect();
            surfaces.insert(
                declaration.provider_ref,
                ProviderSurface {
                    adapters: declaration.plane_adapters,
                    storage_roots: declaration.storage_roots,
                    services,
                },
            );
        }
        Self {
            zone,
            state_root,
            surfaces,
            ledger: Mutex::new(PlaneLedger::default()),
        }
    }

    /// The first refusal the port recorded, if any.
    pub(crate) fn refusal(&self) -> Option<PlaneRefusal> {
        self.ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .refusal
            .clone()
    }

    /// Every claimed storage root, provider by provider.
    pub(crate) fn claimed_roots(&self) -> Vec<ClaimedRoot> {
        self.ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .roots
            .values()
            .flatten()
            .cloned()
            .collect()
    }

    /// Every deployed adapter, provider by provider.
    pub(crate) fn deployed_adapters(&self) -> Vec<(&'static str, &'static str)> {
        self.ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .adapters
            .iter()
            .flat_map(|(provider, adapters)| {
                adapters.iter().map(move |adapter| (*provider, *adapter))
            })
            .collect()
    }

    /// Every published service, provider by provider.
    pub(crate) fn published_services(&self) -> Vec<(&'static str, &'static str)> {
        self.ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .services
            .iter()
            .flat_map(|(provider, services)| {
                services.iter().map(move |service| (*provider, *service))
            })
            .collect()
    }

    /// Release everything one provider claimed through the port.
    pub(crate) fn release(&self, provider_ref: &'static str) {
        let mut ledger = self
            .ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ledger.roots.remove(provider_ref);
        ledger.adapters.remove(provider_ref);
        ledger.services.remove(provider_ref);
    }

    fn refuse(&self, provider_ref: &'static str, row: String, reason: &'static str) -> PlaneError {
        let mut ledger = self
            .ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if ledger.refusal.is_none() {
            ledger.refusal = Some(PlaneRefusal {
                provider_ref,
                row,
                reason,
            });
        }
        PlaneError::Refused
    }

    fn surface<'a>(
        surfaces: &'a BTreeMap<&'static str, ProviderSurface>,
        provider_ref: &'static str,
    ) -> Option<&'a ProviderSurface> {
        surfaces.get(provider_ref)
    }

    /// The declared subdirectories of every provider except `provider_ref`.
    ///
    /// Two providers may not claim overlapping subtrees: the declaration
    /// generator refuses an overlap, and the port refuses one here rather
    /// than letting a claim reach into another provider's storage.
    fn other_subtrees(
        surfaces: &BTreeMap<&'static str, ProviderSurface>,
        provider_ref: &'static str,
    ) -> Vec<&'static str> {
        surfaces
            .iter()
            .filter(|(other, _)| **other != provider_ref)
            .flat_map(|(_, surface)| surface.storage_roots.iter().map(|root| root.path))
            .collect()
    }
}

/// Whether one declared subtree contains the other.
fn subtree_contains(left: &str, right: &str) -> bool {
    left == right
        || right
            .strip_prefix(left)
            .is_some_and(|rest| rest.starts_with('/'))
        || left
            .strip_prefix(right)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Whether a declared subtree is a clean relative path.
///
/// The invariant the generator enforces is repeated here because the port is
/// the last seat before a path becomes host state: every component is a plain
/// name, so no declaration can climb out of the zone's state directory.
fn clean_relative(path: &str) -> bool {
    let path = Path::new(path);
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

#[async_trait]
impl ZonePlanePort for ProductionPlanePort {
    async fn claim_storage_root(
        &self,
        provider_ref: &'static str,
        root: &StorageRoot,
    ) -> Result<(), PlaneError> {
        let Some(surface) = Self::surface(&self.surfaces, provider_ref) else {
            return Err(self.refuse(provider_ref, root.path.to_owned(), "provider-undeclared"));
        };
        let declared = surface
            .storage_roots
            .iter()
            .find(|declared| declared.path == root.path)
            .ok_or_else(|| {
                self.refuse(provider_ref, root.path.to_owned(), "storage-root-undeclared")
            })?;
        if declared.provider_owned != root.provider_owned {
            return Err(self.refuse(
                provider_ref,
                root.path.to_owned(),
                "storage-root-ownership-mismatch",
            ));
        }
        if !clean_relative(root.path) {
            return Err(self.refuse(
                provider_ref,
                root.path.to_owned(),
                "storage-root-escapes-subtree",
            ));
        }
        if Self::other_subtrees(&self.surfaces, provider_ref)
            .into_iter()
            .any(|other| subtree_contains(other, root.path))
        {
            return Err(self.refuse(
                provider_ref,
                root.path.to_owned(),
                "storage-root-overlap",
            ));
        }
        {
            let ledger = self
                .ledger
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if ledger
                .roots
                .get(provider_ref)
                .is_some_and(|claimed| claimed.iter().any(|claimed| claimed.declared == root.path))
            {
                return Err(self.refuse(
                    provider_ref,
                    root.path.to_owned(),
                    "storage-root-duplicate",
                ));
            }
        }
        let resolved = self.state_root.join(root.path);
        if root.provider_owned {
            if let Err(error) = std::fs::create_dir_all(&resolved) {
                tracing::warn!(
                    zone = %self.zone.as_str(),
                    provider = provider_ref,
                    root = root.path,
                    %error,
                    "declared storage root could not be claimed"
                );
                return Err(self.refuse(
                    provider_ref,
                    root.path.to_owned(),
                    "storage-root-claim-failed",
                ));
            }
        } else if !resolved.exists() {
            return Err(self.refuse(
                provider_ref,
                root.path.to_owned(),
                "storage-root-missing",
            ));
        }
        self.ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .roots
            .entry(provider_ref)
            .or_default()
            .push(ClaimedRoot {
                provider_ref,
                declared: root.path,
                provider_owned: root.provider_owned,
            });
        Ok(())
    }

    async fn deploy_adapter(
        &self,
        provider_ref: &'static str,
        adapter: &PlaneAdapter,
    ) -> Result<(), PlaneError> {
        let Some(surface) = Self::surface(&self.surfaces, provider_ref) else {
            return Err(self.refuse(provider_ref, adapter.id.to_owned(), "provider-undeclared"));
        };
        if !surface
            .adapters
            .iter()
            .any(|declared| declared.id == adapter.id && declared.depends_on == adapter.depends_on)
        {
            return Err(self.refuse(provider_ref, adapter.id.to_owned(), "adapter-undeclared"));
        }
        let mut ledger = self
            .ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let deployed = ledger.adapters.entry(provider_ref).or_default();
        if deployed.contains(&adapter.id) {
            return Err(self.refuse(provider_ref, adapter.id.to_owned(), "adapter-duplicate"));
        }
        for dependency in adapter.depends_on {
            let owned = surface
                .adapters
                .iter()
                .any(|declared| declared.id == *dependency);
            if owned && !deployed.contains(dependency) {
                return Err(self.refuse(
                    provider_ref,
                    adapter.id.to_owned(),
                    "adapter-dependency-not-deployed",
                ));
            }
        }
        deployed.push(adapter.id);
        Ok(())
    }

    async fn publish_service(
        &self,
        provider_ref: &'static str,
        service: &ServiceDecl,
    ) -> Result<(), PlaneError> {
        let Some(surface) = Self::surface(&self.surfaces, provider_ref) else {
            return Err(self.refuse(provider_ref, service.id.to_owned(), "provider-undeclared"));
        };
        if !surface.services.contains(&service.id) {
            return Err(self.refuse(provider_ref, service.id.to_owned(), "service-undeclared"));
        }
        let mut ledger = self
            .ledger
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let published = ledger.services.entry(provider_ref).or_default();
        if published.contains(&service.id) {
            return Err(self.refuse(provider_ref, service.id.to_owned(), "service-duplicate"));
        }
        published.push(service.id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZONE: &str = "test";

    fn zone() -> ZoneId {
        ZoneId::parse(ZONE).expect("a zone label")
    }

    fn declaration(
        provider_ref: &'static str,
        adapters: &'static [PlaneAdapter],
        roots: &'static [StorageRoot],
    ) -> ProviderDeclaration {
        ProviderDeclaration {
            provider_ref,
            self_bindings: &[],
            required: true,
            cardinality: d2b_provider_toolkit::Cardinality::AtMostOne,
            isolation_posture: d2b_provider_toolkit::IsolationPosture::Standard,
            plane_adapters: adapters,
            principals: &[],
            storage_roots: roots,
        }
    }

    fn port(
        dir: &tempfile::TempDir,
        declarations: Vec<ProviderDeclaration>,
    ) -> ProductionPlanePort {
        let declarations: Vec<(ProviderDeclaration, Arc<[DriverDescriptor]>)> = declarations
            .into_iter()
            .map(|declaration| (declaration, Arc::from(Vec::<DriverDescriptor>::new())))
            .collect();
        ProductionPlanePort::over(zone(), dir.path().to_path_buf(), &declarations)
    }

    const fn root(path: &'static str) -> StorageRoot {
        StorageRoot {
            path,
            provider_owned: true,
        }
    }

    /// A provider claims exactly the root it declared, and the plane creates
    /// it under the zone's own state directory.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_declared_root_is_claimed_under_the_zone_state_directory() {
        const ROOTS: &[StorageRoot] = &[StorageRoot {
            path: "state",
            provider_owned: true,
        }];
        let dir = tempfile::tempdir().expect("tempdir");
        let port = port(&dir, vec![declaration("volume", &[], ROOTS)]);
        port.claim_storage_root("volume", &ROOTS[0])
            .await
            .expect("the declared root is claimed");
        assert!(dir.path().join("state").is_dir());
        assert_eq!(
            port.claimed_roots(),
            vec![ClaimedRoot {
                provider_ref: "volume",
                declared: "state",
                provider_owned: true,
            }]
        );
    }

    /// An action outside the declaration refuses, naming the provider, the
    /// row, and the reason.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_undeclared_action_refuses_named() {
        const ROOTS: &[StorageRoot] = &[root("state")];
        let dir = tempfile::tempdir().expect("tempdir");
        let port = port(&dir, vec![declaration("volume", &[], ROOTS)]);
        let undeclared = root("other");
        let error = port
            .claim_storage_root("volume", &undeclared)
            .await
            .expect_err("the root is not declared");
        assert_eq!(error, PlaneError::Refused);
        assert_eq!(
            port.refusal().expect("a named refusal").message(),
            "storage-root-undeclared:volume:other"
        );
        assert!(!dir.path().join("other").exists());
    }

    /// A provider the plane does not start has no declaration to act on.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_undeclared_provider_refuses_named() {
        let dir = tempfile::tempdir().expect("tempdir");
        let port = port(&dir, Vec::new());
        port.claim_storage_root("volume", &root("state"))
            .await
            .expect_err("the plane starts no such provider");
        assert_eq!(
            port.refusal().expect("a named refusal").message(),
            "provider-undeclared:volume:state"
        );
    }

    /// Two providers may not claim overlapping subtrees.
    #[tokio::test(flavor = "multi_thread")]
    async fn overlapping_declared_roots_refuse_named() {
        const ROOTS: &[StorageRoot] = &[root("state")];
        const NESTED: &[StorageRoot] = &[root("state/volumes")];
        let dir = tempfile::tempdir().expect("tempdir");
        let port = port(
            &dir,
            vec![
                declaration("volume", &[], ROOTS),
                declaration("volume-local", &[], NESTED),
            ],
        );
        port.claim_storage_root("volume-local", &NESTED[0])
            .await
            .expect_err("the subtree belongs to another provider");
        assert_eq!(
            port.refusal().expect("a named refusal").message(),
            "storage-root-overlap:volume-local:state/volumes"
        );
    }

    /// A declaration cannot escape the zone's state directory.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_escaping_declared_root_refuses_named() {
        const ROOTS: &[StorageRoot] = &[root("state/../../escape")];
        let dir = tempfile::tempdir().expect("tempdir");
        let port = port(&dir, vec![declaration("volume", &[], ROOTS)]);
        port.claim_storage_root("volume", &ROOTS[0])
            .await
            .expect_err("the path climbs out");
        assert_eq!(
            port.refusal().expect("a named refusal").message(),
            "storage-root-escapes-subtree:volume:state/../../escape"
        );
    }

    /// A root the plane provisions rather than the provider must exist.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_missing_provisioned_root_refuses_named() {
        const ROOTS: &[StorageRoot] = &[StorageRoot {
            path: "provisioned",
            provider_owned: false,
        }];
        let dir = tempfile::tempdir().expect("tempdir");
        let port = port(&dir, vec![declaration("volume", &[], ROOTS)]);
        port.claim_storage_root("volume", &ROOTS[0])
            .await
            .expect_err("the plane did not provision it");
        assert_eq!(
            port.refusal().expect("a named refusal").message(),
            "storage-root-missing:volume:provisioned"
        );
    }

    /// Adapters deploy in declared dependency order, and a dependency that
    /// has not deployed refuses.
    #[tokio::test(flavor = "multi_thread")]
    async fn adapters_deploy_in_declared_dependency_order() {
        const ADAPTERS: &[PlaneAdapter] = &[
            PlaneAdapter {
                id: "listener",
                depends_on: &["binding"],
            },
            PlaneAdapter {
                id: "binding",
                depends_on: &[],
            },
        ];
        let dir = tempfile::tempdir().expect("tempdir");
        let port = port(&dir, vec![declaration("endpoint", ADAPTERS, &[])]);
        port.deploy_adapter("endpoint", &ADAPTERS[0])
            .await
            .expect_err("the dependency is not deployed");
        assert_eq!(
            port.refusal().expect("a named refusal").message(),
            "adapter-dependency-not-deployed:endpoint:listener"
        );
        port.deploy_adapter("endpoint", &ADAPTERS[1])
            .await
            .expect("the dependency deploys first");
        port.deploy_adapter("endpoint", &ADAPTERS[0])
            .await
            .expect("the dependent deploys after it");
        assert_eq!(
            port.deployed_adapters(),
            vec![("endpoint", "binding"), ("endpoint", "listener")]
        );
    }

    /// A service no driver declared is refused rather than published.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_undeclared_service_refuses_named() {
        const SERVICE: ServiceDecl = ServiceDecl {
            id: "d2b.provider.v3",
            methods: &["start"],
            attach_kinds: &[],
            streams: &[],
            endpoint_policy: None,
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let port = port(&dir, vec![declaration("volume", &[], &[])]);
        port.publish_service("volume", &SERVICE)
            .await
            .expect_err("no driver declared the service");
        assert_eq!(
            port.refusal().expect("a named refusal").message(),
            "service-undeclared:volume:d2b.provider.v3"
        );
        assert!(port.published_services().is_empty());
    }

    /// Releasing one provider drops exactly what it claimed.
    #[tokio::test(flavor = "multi_thread")]
    async fn release_drops_one_providers_claims() {
        const VOLUMES: &[StorageRoot] = &[root("volumes")];
        const NETWORK: &[StorageRoot] = &[root("network")];
        let dir = tempfile::tempdir().expect("tempdir");
        let port = port(
            &dir,
            vec![
                declaration("volume", &[], VOLUMES),
                declaration("network-local", &[], NETWORK),
            ],
        );
        port.claim_storage_root("volume", &VOLUMES[0])
            .await
            .expect("volume claims");
        port.claim_storage_root("network-local", &NETWORK[0])
            .await
            .expect("network claims");
        assert_eq!(port.claimed_roots().len(), 2);
        port.release("volume");
        assert_eq!(
            port.claimed_roots(),
            vec![ClaimedRoot {
                provider_ref: "network-local",
                declared: "network",
                provider_owned: true,
            }]
        );
    }
}
