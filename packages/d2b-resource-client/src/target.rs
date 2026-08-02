//! Zone-addressed targets, the v3 service inventory, and local route lookup.
//!
//! Addressing is by [`ZonePath`] throughout: a target names the Zone that owns
//! the service, never a socket, a descriptor, a uid, or a store path. Resolution
//! is exact and fail-closed - an owner with no record refuses with
//! [`ClientError::RouteUnavailable`], an owner reachable only over a different
//! carriage refuses with [`ClientError::TransportPolicyMismatch`], and an
//! ambiguous table refuses with [`ClientError::InvalidTarget`].

use core::fmt;

use d2b_contracts::v3::{ResourceName, ResourceRef, ResourceTypeName, zone_routing::ZonePath};

use crate::ClientError;

/// The v3 service inventory reachable through the Zone bus.
///
/// This is the closed v3 service inventory. A service that is not listed here
/// is not addressable by this client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZoneServiceKind {
    /// The Zone resource plane.
    Resource,
    /// The Zone topology, enrollment, and route-resolution service.
    Zone,
    /// The ZoneLink uplink control service.
    ZoneLink,
    /// A Provider service installed in the addressed Zone.
    Provider,
    /// A controller service.
    Controller,
    /// The audit service.
    Audit,
    /// The support-bundle service.
    Support,
    /// The credential service.
    Credential,
}

impl ZoneServiceKind {
    /// Exhaustive stable variant order.
    pub const fn all() -> &'static [Self; 8] {
        &[
            Self::Resource,
            Self::Zone,
            Self::ZoneLink,
            Self::Provider,
            Self::Controller,
            Self::Audit,
            Self::Support,
            Self::Credential,
        ]
    }

    /// The canonical v3 package name.
    pub const fn package(self) -> &'static str {
        match self {
            Self::Resource => "d2b.resource.v3",
            Self::Zone => "d2b.zone.v3",
            Self::ZoneLink => "d2b.zonelink.v3",
            Self::Provider => "d2b.provider.v3",
            Self::Controller => "d2b.controller.v3",
            Self::Audit => "d2b.audit.v3",
            Self::Support => "d2b.support.v3",
            Self::Credential => "d2b.credential.v3",
        }
    }

    /// The stable kebab-case label for a diagnostic or metric label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Resource => "resource",
            Self::Zone => "zone",
            Self::ZoneLink => "zone-link",
            Self::Provider => "provider",
            Self::Controller => "controller",
            Self::Audit => "audit",
            Self::Support => "support",
            Self::Credential => "credential",
        }
    }
}

/// The Zone-scoped principal that owns the addressed service.
///
/// Every variant names exactly one Zone, which is the routing key.
#[derive(Clone, PartialEq, Eq)]
pub enum ServiceOwner {
    /// A service in the caller's own local Zone, reached without a ZoneLink.
    ZoneLocal(ZonePath),
    /// A Zone-owned service, possibly a different Zone in the tree.
    Zone(ZonePath),
    /// A Guest-owned service inside the named Zone.
    Guest {
        /// The owning Zone.
        zone: ZonePath,
        /// The Guest resource name inside that Zone.
        guest: ResourceName,
    },
    /// A Provider-owned service inside the named Zone.
    Provider {
        /// The owning Zone.
        zone: ZonePath,
        /// The Provider resource name inside that Zone.
        provider: ResourceName,
    },
    /// A Host resource inside a named Zone.
    Host {
        /// The owning Zone.
        zone: ZonePath,
        /// The Host resource name.
        host: ResourceName,
    },
    /// An arbitrary ResourceRef inside a named Zone.
    Resource {
        /// The owning Zone.
        zone: ZonePath,
        /// The canonical same-Zone resource reference.
        resource: ResourceRef,
    },
}

impl ServiceOwner {
    /// Borrow the Zone this owner routes to.
    pub const fn zone(&self) -> &ZonePath {
        match self {
            Self::ZoneLocal(zone) | Self::Zone(zone) => zone,
            Self::Guest { zone, .. }
            | Self::Provider { zone, .. }
            | Self::Host { zone, .. }
            | Self::Resource { zone, .. } => zone,
        }
    }

    /// The stable kebab-case owner class, carrying no identity.
    pub const fn class(&self) -> &'static str {
        match self {
            Self::ZoneLocal(_) => "zone-local",
            Self::Zone(_) => "zone",
            Self::Guest { .. } => "guest",
            Self::Provider { .. } => "provider",
            Self::Host { .. } => "host",
            Self::Resource { .. } => "resource",
        }
    }

    /// Borrow the resource reference when this owner addresses one.
    pub fn resource_ref(&self) -> Option<ResourceRef> {
        match self {
            Self::Guest { guest, .. } => Some(ResourceRef::new(
                ResourceTypeName::parse("Guest").expect("closed ResourceType"),
                guest.clone(),
            )),
            Self::Provider { provider, .. } => Some(ResourceRef::new(
                ResourceTypeName::parse("Provider").expect("closed ResourceType"),
                provider.clone(),
            )),
            Self::Host { host, .. } => Some(ResourceRef::new(
                ResourceTypeName::parse("Host").expect("closed ResourceType"),
                host.clone(),
            )),
            Self::Resource { resource, .. } => Some(resource.clone()),
            Self::ZoneLocal(_) | Self::Zone(_) => None,
        }
    }
}

impl fmt::Debug for ServiceOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceOwner")
            .field("class", &self.class())
            .finish_non_exhaustive()
    }
}

/// A caller-supplied target, before route resolution.
#[derive(Clone, PartialEq, Eq)]
pub enum TargetInput {
    /// The caller's own local Zone.
    ZoneLocal(ZonePath),
    /// A named Zone.
    Zone(ZonePath),
    /// A Guest inside a named Zone.
    Guest {
        /// The owning Zone.
        zone: ZonePath,
        /// The Guest resource name.
        guest: ResourceName,
    },
    /// A Provider inside a named Zone.
    Provider {
        /// The owning Zone.
        zone: ZonePath,
        /// The Provider resource name.
        provider: ResourceName,
    },
    /// A Host inside a named Zone.
    Host {
        /// The owning Zone.
        zone: ZonePath,
        /// The Host resource name.
        host: ResourceName,
    },
    /// An arbitrary canonical ResourceRef inside a named Zone.
    Resource {
        /// The owning Zone.
        zone: ZonePath,
        /// The canonical same-Zone resource reference.
        resource: ResourceRef,
    },
    /// An owner plus the exact service the caller intends to reach.
    Service {
        /// The addressed owner.
        owner: ServiceOwner,
        /// The declared service.
        service: ZoneServiceKind,
    },
    /// A cross-Zone service target: an exact Zone plus an exact service.
    ZoneService(ZonePath, ZoneServiceKind),
}

impl TargetInput {
    /// The owner this target addresses.
    pub fn owner(&self) -> ServiceOwner {
        match self {
            Self::ZoneLocal(zone) => ServiceOwner::ZoneLocal(zone.clone()),
            Self::Zone(zone) | Self::ZoneService(zone, _) => ServiceOwner::Zone(zone.clone()),
            Self::Guest { zone, guest } => ServiceOwner::Guest {
                zone: zone.clone(),
                guest: guest.clone(),
            },
            Self::Provider { zone, provider } => ServiceOwner::Provider {
                zone: zone.clone(),
                provider: provider.clone(),
            },
            Self::Host { zone, host } => ServiceOwner::Host {
                zone: zone.clone(),
                host: host.clone(),
            },
            Self::Resource { zone, resource } => ServiceOwner::Resource {
                zone: zone.clone(),
                resource: resource.clone(),
            },
            Self::Service { owner, .. } => owner.clone(),
        }
    }

    /// The service the caller declared, when the target names one.
    pub const fn declared_service(&self) -> Option<ZoneServiceKind> {
        match self {
            Self::Service { service, .. } | Self::ZoneService(_, service) => Some(*service),
            _ => None,
        }
    }

    /// The stable kebab-case target class, carrying no identity.
    pub const fn class(&self) -> &'static str {
        match self {
            Self::ZoneLocal(_) => "zone-local",
            Self::Zone(_) => "zone",
            Self::Guest { .. } => "guest",
            Self::Provider { .. } => "provider",
            Self::Host { .. } => "host",
            Self::Resource { .. } => "resource",
            Self::Service { .. } => "service",
            Self::ZoneService(..) => "zone-service",
        }
    }

    /// Borrow the canonical resource target when one was supplied.
    pub fn resource_ref(&self) -> Option<ResourceRef> {
        self.owner().resource_ref()
    }
}

impl fmt::Debug for TargetInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TargetInput")
            .field("class", &self.class())
            .finish_non_exhaustive()
    }
}

/// The carriage class a route record admits.
///
/// A variant names a class of carriage, never a concrete endpoint: no socket
/// path, descriptor number, address, or store path appears here or anywhere
/// else in this crate's surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TransportKind {
    /// A descriptor issued by the Zone allocator and inherited by the caller.
    AllocatorIssuedDescriptor,
    /// A local Unix ComponentSession to the local Zone bus.
    LocalUnix,
    /// Carriage over the Zone's local ZoneLink uplink.
    ZoneLink,
    /// Carriage supplied by an installed transport Provider.
    Provider,
}

impl TransportKind {
    /// The stable kebab-case carriage label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::AllocatorIssuedDescriptor => "allocator-issued-descriptor",
            Self::LocalUnix => "local-unix",
            Self::ZoneLink => "zone-link",
            Self::Provider => "provider",
        }
    }
}

/// The caller's carriage requirement for one resolution.
///
/// [`TransportSelection::unspecified`] is a refusal, not a default: resolution
/// fails closed rather than picking a carriage on the caller's behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportSelection {
    exact: Option<TransportKind>,
}

impl TransportSelection {
    /// Require exactly one carriage class.
    pub const fn exact(kind: TransportKind) -> Self {
        Self { exact: Some(kind) }
    }

    /// Decline to name a carriage class.
    pub const fn unspecified() -> Self {
        Self { exact: None }
    }

    /// The required carriage class, when one was named.
    pub const fn kind(self) -> Option<TransportKind> {
        self.exact
    }
}

/// One local route record: an exact owner reachable over an exact carriage.
#[derive(Clone, PartialEq, Eq)]
pub struct RouteRecord {
    owner: ServiceOwner,
    transport: TransportKind,
}

impl RouteRecord {
    /// Declare one route.
    pub const fn new(owner: ServiceOwner, transport: TransportKind) -> Self {
        Self { owner, transport }
    }

    /// Borrow the owner this record admits.
    pub const fn owner(&self) -> &ServiceOwner {
        &self.owner
    }

    /// The carriage class this record admits.
    pub const fn transport(&self) -> TransportKind {
        self.transport
    }
}

impl fmt::Debug for RouteRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RouteRecord")
            .field("owner", &self.owner)
            .field("transport", &self.transport)
            .finish()
    }
}

/// An exactly resolved target: one owner, one carriage class, one service.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    owner: ServiceOwner,
    transport: TransportKind,
    service: ZoneServiceKind,
}

impl ResolvedTarget {
    /// Borrow the resolved owner.
    pub const fn owner(&self) -> &ServiceOwner {
        &self.owner
    }

    /// The resolved carriage class.
    pub const fn transport(&self) -> TransportKind {
        self.transport
    }

    /// The resolved service.
    pub const fn service(&self) -> ZoneServiceKind {
        self.service
    }

    /// Return the addressed resource reference, when this target names one.
    pub fn resource_ref(&self) -> Option<ResourceRef> {
        self.owner.resource_ref()
    }
}

impl fmt::Debug for ResolvedTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedTarget")
            .field("owner", &self.owner)
            .field("transport", &self.transport)
            .field("service", &self.service)
            .finish()
    }
}

/// Resolves a caller target to an exact route.
pub trait TargetResolver: Send + Sync {
    /// Resolve one target, or refuse with a typed reason.
    fn resolve(
        &self,
        target: &TargetInput,
        service: ZoneServiceKind,
        selection: TransportSelection,
    ) -> Result<ResolvedTarget, ClientError>;
}

/// A local, static table of admitted routes, keyed by Zone-scoped owner.
#[derive(Debug, Clone, Default)]
pub struct RouteTable {
    records: Vec<RouteRecord>,
}

impl RouteTable {
    /// Build a table from an exact record list.
    pub const fn new(records: Vec<RouteRecord>) -> Self {
        Self { records }
    }

    /// Borrow the declared records.
    pub fn records(&self) -> &[RouteRecord] {
        &self.records
    }
}

impl TargetResolver for RouteTable {
    fn resolve(
        &self,
        target: &TargetInput,
        service: ZoneServiceKind,
        selection: TransportSelection,
    ) -> Result<ResolvedTarget, ClientError> {
        if target
            .declared_service()
            .is_some_and(|declared| declared != service)
        {
            return Err(ClientError::InvalidService);
        }
        let owner = target.owner();
        let selected = selection
            .kind()
            .ok_or(ClientError::TransportPolicyMismatch)?;
        let mut candidates = self
            .records
            .iter()
            .filter(|record| record.owner == owner && record.transport == selected);
        let Some(record) = candidates.next() else {
            return Err(if self.records.iter().any(|record| record.owner == owner) {
                ClientError::TransportPolicyMismatch
            } else {
                ClientError::RouteUnavailable
            });
        };
        if candidates.next().is_some() {
            return Err(ClientError::InvalidTarget);
        }
        Ok(ResolvedTarget {
            owner,
            transport: record.transport,
            service,
        })
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use d2b_contracts::v3::zone_routing::ZoneLabelId;

    /// Build a Zone path from most-specific-first labels.
    pub(crate) fn zone(labels: &[&str]) -> ZonePath {
        ZonePath::new(
            labels
                .iter()
                .map(|label| ZoneLabelId::parse(*label).expect("valid label"))
                .collect(),
        )
        .expect("valid zone path")
    }

    pub(crate) fn name(value: &str) -> ResourceName {
        ResourceName::parse(value).expect("valid resource name")
    }
}

#[cfg(test)]
mod tests {
    use super::{fixtures::*, *};

    #[test]
    fn service_and_transport_labels_are_unique_and_stable() {
        let mut services = ZoneServiceKind::all()
            .iter()
            .map(|kind| kind.label())
            .collect::<Vec<_>>();
        assert_eq!(
            services,
            vec![
                "resource",
                "zone",
                "zone-link",
                "provider",
                "controller",
                "audit",
                "support",
                "credential"
            ]
        );
        services.sort_unstable();
        services.dedup();
        assert_eq!(services.len(), ZoneServiceKind::all().len());

        let transports = [
            TransportKind::AllocatorIssuedDescriptor,
            TransportKind::LocalUnix,
            TransportKind::ZoneLink,
            TransportKind::Provider,
        ];
        let mut labels = transports
            .iter()
            .map(|kind| kind.label())
            .collect::<Vec<_>>();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), transports.len());
    }

    #[test]
    fn every_target_names_exactly_one_routing_zone() {
        let k0 = zone(&["k0"]);
        let k1 = zone(&["k1", "k0"]);
        let cases = [
            (TargetInput::ZoneLocal(k0.clone()), &k0, "zone-local"),
            (TargetInput::Zone(k1.clone()), &k1, "zone"),
            (
                TargetInput::Guest {
                    zone: k1.clone(),
                    guest: name("builder"),
                },
                &k1,
                "guest",
            ),
            (
                TargetInput::Provider {
                    zone: k1.clone(),
                    provider: name("volume-local"),
                },
                &k1,
                "provider",
            ),
            (
                TargetInput::Host {
                    zone: k1.clone(),
                    host: name("host-system"),
                },
                &k1,
                "host",
            ),
            (
                TargetInput::Resource {
                    zone: k1.clone(),
                    resource: ResourceRef::parse("Process/worker").unwrap(),
                },
                &k1,
                "resource",
            ),
            (
                TargetInput::ZoneService(k1.clone(), ZoneServiceKind::Resource),
                &k1,
                "zone-service",
            ),
            (
                TargetInput::Service {
                    owner: ServiceOwner::ZoneLocal(k0.clone()),
                    service: ZoneServiceKind::Zone,
                },
                &k0,
                "service",
            ),
        ];
        for (target, expected_zone, class) in cases {
            assert_eq!(target.class(), class);
            assert_eq!(target.owner().zone(), expected_zone);
        }

        assert_eq!(
            TargetInput::ZoneService(k1.clone(), ZoneServiceKind::Resource).declared_service(),
            Some(ZoneServiceKind::Resource)
        );
        assert_eq!(TargetInput::Zone(k1).declared_service(), None);
        // A cross-Zone service target resolves to a Zone owner, never a local one.
        assert_eq!(
            TargetInput::ZoneService(k0.clone(), ZoneServiceKind::Resource)
                .owner()
                .class(),
            "zone"
        );
        assert_eq!(TargetInput::ZoneLocal(k0).owner().class(), "zone-local");
    }

    #[test]
    fn resource_targets_preserve_the_exact_canonical_reference() {
        let zone = zone(&["k1", "k0"]);
        let target = TargetInput::Resource {
            zone: zone.clone(),
            resource: ResourceRef::parse("acme.d2bus.org.Widget/item").unwrap(),
        };
        assert_eq!(
            target.resource_ref().unwrap().to_canonical_string(),
            "acme.d2bus.org.Widget/item"
        );
        assert_eq!(target.owner().zone(), &zone);
    }

    #[test]
    fn route_lookup_is_keyed_on_the_exact_zone_path() {
        let k0 = zone(&["k0"]);
        let k1 = zone(&["k1", "k0"]);
        let k2 = zone(&["k2", "k1", "k0"]);
        let table = RouteTable::new(vec![
            RouteRecord::new(
                ServiceOwner::ZoneLocal(k0.clone()),
                TransportKind::LocalUnix,
            ),
            RouteRecord::new(ServiceOwner::Zone(k1.clone()), TransportKind::ZoneLink),
        ]);

        let local = table
            .resolve(
                &TargetInput::ZoneLocal(k0.clone()),
                ZoneServiceKind::Resource,
                TransportSelection::exact(TransportKind::LocalUnix),
            )
            .expect("local Zone route");
        assert_eq!(local.transport(), TransportKind::LocalUnix);
        assert_eq!(local.owner().zone(), &k0);

        let child = table
            .resolve(
                &TargetInput::ZoneService(k1.clone(), ZoneServiceKind::Resource),
                ZoneServiceKind::Resource,
                TransportSelection::exact(TransportKind::ZoneLink),
            )
            .expect("child Zone route");
        assert_eq!(child.transport(), TransportKind::ZoneLink);
        assert_eq!(child.owner().zone(), &k1);
        assert_eq!(child.service(), ZoneServiceKind::Resource);

        // A Zone with no record refuses, even though a sibling Zone has one.
        assert_eq!(
            table
                .resolve(
                    &TargetInput::Zone(k2),
                    ZoneServiceKind::Resource,
                    TransportSelection::exact(TransportKind::ZoneLink),
                )
                .unwrap_err(),
            ClientError::RouteUnavailable
        );
        // The same Zone path under a different owner class is a different key.
        assert_eq!(
            table
                .resolve(
                    &TargetInput::Zone(k0.clone()),
                    ZoneServiceKind::Resource,
                    TransportSelection::exact(TransportKind::LocalUnix),
                )
                .unwrap_err(),
            ClientError::RouteUnavailable
        );
        // A known owner over an unadmitted carriage is a policy mismatch.
        assert_eq!(
            table
                .resolve(
                    &TargetInput::Zone(k1.clone()),
                    ZoneServiceKind::Resource,
                    TransportSelection::exact(TransportKind::Provider),
                )
                .unwrap_err(),
            ClientError::TransportPolicyMismatch
        );
    }

    #[test]
    fn resolution_fails_closed_on_service_carriage_and_ambiguity() {
        let k1 = zone(&["k1", "k0"]);
        let table = RouteTable::new(vec![RouteRecord::new(
            ServiceOwner::Zone(k1.clone()),
            TransportKind::ZoneLink,
        )]);

        // A declared service that contradicts the requested one is refused.
        assert_eq!(
            table
                .resolve(
                    &TargetInput::ZoneService(k1.clone(), ZoneServiceKind::Zone),
                    ZoneServiceKind::Resource,
                    TransportSelection::exact(TransportKind::ZoneLink),
                )
                .unwrap_err(),
            ClientError::InvalidService
        );
        // An unspecified carriage is never defaulted.
        assert_eq!(
            table
                .resolve(
                    &TargetInput::Zone(k1.clone()),
                    ZoneServiceKind::Resource,
                    TransportSelection::unspecified(),
                )
                .unwrap_err(),
            ClientError::TransportPolicyMismatch
        );
        // A duplicated record is ambiguous rather than first-wins.
        let ambiguous = RouteTable::new(vec![
            RouteRecord::new(ServiceOwner::Zone(k1.clone()), TransportKind::ZoneLink),
            RouteRecord::new(ServiceOwner::Zone(k1.clone()), TransportKind::ZoneLink),
        ]);
        assert_eq!(
            ambiguous
                .resolve(
                    &TargetInput::Zone(k1),
                    ZoneServiceKind::Resource,
                    TransportSelection::exact(TransportKind::ZoneLink),
                )
                .unwrap_err(),
            ClientError::InvalidTarget
        );
        assert!(RouteTable::default().records().is_empty());
    }

    #[test]
    fn debug_renderings_never_echo_a_zone_label_or_a_resource_name() {
        let marker = format!("marker{:x}", std::process::id());
        let target = TargetInput::Guest {
            zone: zone(&[marker.as_str()]),
            guest: name(&marker),
        };
        let owner = target.owner();
        let record = RouteRecord::new(owner.clone(), TransportKind::ZoneLink);
        let resolved = ResolvedTarget {
            owner: owner.clone(),
            transport: TransportKind::ZoneLink,
            service: ZoneServiceKind::Resource,
        };
        let table = RouteTable::new(vec![record.clone()]);
        for rendered in [
            format!("{target:?}"),
            format!("{owner:?}"),
            format!("{record:?}"),
            format!("{resolved:?}"),
            format!("{table:?}"),
        ] {
            assert!(!rendered.contains(&marker), "{rendered}");
            assert!(!rendered.contains('/'), "{rendered}");
        }
    }
}
