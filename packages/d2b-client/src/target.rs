use std::fmt;

use d2b_contracts::v2_identity::{ProviderId, RealmId, WorkloadId};

use crate::{ClientError, ServiceKind};

#[derive(Clone, PartialEq, Eq)]
pub enum ServiceOwner {
    LocalRoot(RealmId),
    Realm(RealmId),
    Workload {
        realm: RealmId,
        workload: WorkloadId,
    },
    Provider {
        realm: RealmId,
        provider: ProviderId,
    },
}

impl fmt::Debug for ServiceOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LocalRoot(_) => "LocalRoot",
            Self::Realm(_) => "Realm",
            Self::Workload { .. } => "Workload",
            Self::Provider { .. } => "Provider",
        })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum TargetInput {
    LocalRoot(RealmId),
    Realm(RealmId),
    Workload {
        realm: RealmId,
        workload: WorkloadId,
    },
    Provider {
        realm: RealmId,
        provider: ProviderId,
    },
    Service {
        owner: ServiceOwner,
        service: ServiceKind,
    },
}

impl fmt::Debug for TargetInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LocalRoot(_) => "TargetInput::LocalRoot",
            Self::Realm(_) => "TargetInput::Realm",
            Self::Workload { .. } => "TargetInput::Workload",
            Self::Provider { .. } => "TargetInput::Provider",
            Self::Service { .. } => "TargetInput::Service",
        })
    }
}

impl TargetInput {
    pub fn owner(&self) -> ServiceOwner {
        match self {
            Self::LocalRoot(realm) => ServiceOwner::LocalRoot(realm.clone()),
            Self::Realm(realm) => ServiceOwner::Realm(realm.clone()),
            Self::Workload { realm, workload } => ServiceOwner::Workload {
                realm: realm.clone(),
                workload: workload.clone(),
            },
            Self::Provider { realm, provider } => ServiceOwner::Provider {
                realm: realm.clone(),
                provider: provider.clone(),
            },
            Self::Service { owner, .. } => owner.clone(),
        }
    }

    pub fn declared_service(&self) -> Option<ServiceKind> {
        match self {
            Self::Service { service, .. } => Some(*service),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    LocalUnix,
    InheritedSocket,
    NativeVsock,
    CloudHypervisorVsock,
    Provider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportSelection {
    exact: TransportKind,
}

impl TransportSelection {
    pub const fn exact(kind: TransportKind) -> Self {
        Self { exact: kind }
    }

    pub const fn kind(self) -> TransportKind {
        self.exact
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RouteRecord {
    pub owner: ServiceOwner,
    pub transport: TransportKind,
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

#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    owner: ServiceOwner,
    transport: TransportKind,
    service: ServiceKind,
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

impl ResolvedTarget {
    pub fn owner(&self) -> &ServiceOwner {
        &self.owner
    }

    pub const fn transport(&self) -> TransportKind {
        self.transport
    }

    pub const fn service(&self) -> ServiceKind {
        self.service
    }
}

pub trait TargetResolver: Send + Sync {
    fn resolve(
        &self,
        target: &TargetInput,
        service: ServiceKind,
        selection: TransportSelection,
    ) -> Result<ResolvedTarget, ClientError>;
}

#[derive(Debug, Clone, Default)]
pub struct RouteTable {
    records: Vec<RouteRecord>,
}

impl RouteTable {
    pub fn new(records: Vec<RouteRecord>) -> Self {
        Self { records }
    }
}

impl TargetResolver for RouteTable {
    fn resolve(
        &self,
        target: &TargetInput,
        service: ServiceKind,
        selection: TransportSelection,
    ) -> Result<ResolvedTarget, ClientError> {
        if target
            .declared_service()
            .is_some_and(|declared| declared != service)
        {
            return Err(ClientError::InvalidService);
        }
        let owner = target.owner();
        let mut candidates = self.records.iter().filter(|record| record.owner == owner);
        let record = candidates.next().ok_or(ClientError::RouteUnavailable)?;
        if candidates.next().is_some() {
            return Err(ClientError::InvalidTarget);
        }
        if record.transport != selection.kind() {
            return Err(ClientError::TransportPolicyMismatch);
        }
        Ok(ResolvedTarget {
            owner,
            transport: record.transport,
            service,
        })
    }
}
