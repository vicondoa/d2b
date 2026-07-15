use std::{collections::BTreeSet, error::Error, fmt};

use d2b_contracts::{
    v2_identity::ProviderType,
    v2_provider::{
        CgroupAuthority, ConfiguredItemId, DeviceMediationPosture, ImplementationId,
        NetworkPosture, PersistentIdentityPosture, ProcessAuthority, ProviderContractError,
        ProviderFactoryKey, RuntimeAuthorityPosture, UserNamespacePosture,
    },
};
use d2b_host::{
    ch_argv::{ChArgvInput, generate_ch_argv},
    qemu_media_argv::{QemuMediaArgvInput, generate_qemu_media_argv},
};

pub const CLOUD_HYPERVISOR_IMPLEMENTATION_ID: &str = "cloud-hypervisor";
pub const QEMU_MEDIA_IMPLEMENTATION_ID: &str = "qemu-media";
pub const SYSTEMD_USER_IMPLEMENTATION_ID: &str = "systemd-user";
pub const MAX_CONFIGURED_RUNTIME_ITEMS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalRuntimeKind {
    CloudHypervisor,
    QemuMedia,
    SystemdUser,
}

impl LocalRuntimeKind {
    pub const fn implementation_id(self) -> &'static str {
        match self {
            Self::CloudHypervisor => CLOUD_HYPERVISOR_IMPLEMENTATION_ID,
            Self::QemuMedia => QEMU_MEDIA_IMPLEMENTATION_ID,
            Self::SystemdUser => SYSTEMD_USER_IMPLEMENTATION_ID,
        }
    }

    pub fn canonical_implementation_id(self) -> Result<ImplementationId, ProviderContractError> {
        ImplementationId::parse(self.implementation_id())
    }

    pub fn factory_key(self) -> Result<ProviderFactoryKey, ProviderContractError> {
        Ok(ProviderFactoryKey {
            provider_type: ProviderType::Runtime,
            implementation_id: self.canonical_implementation_id()?,
        })
    }

    pub fn authority_posture(self) -> RuntimeAuthorityPosture {
        match self {
            Self::CloudHypervisor => RuntimeAuthorityPosture {
                process: ProcessAuthority::ProviderOwnedPidfd,
                cgroup: CgroupAuthority::RealmDelegatedLeaf,
                network: NetworkPosture::IsolatedNamespace,
                user_namespace: UserNamespacePosture::BrokerPreestablished,
                persistent_identity: PersistentIdentityPosture::FileBackedCloneable,
                device_mediation: DeviceMediationPosture::BrokerDelegatedTyped,
            },
            Self::QemuMedia => RuntimeAuthorityPosture {
                process: ProcessAuthority::ProviderOwnedPidfd,
                cgroup: CgroupAuthority::RealmDelegatedLeaf,
                network: NetworkPosture::IsolatedNamespace,
                user_namespace: UserNamespacePosture::None,
                persistent_identity: PersistentIdentityPosture::None,
                device_mediation: DeviceMediationPosture::BrokerDelegatedTyped,
            },
            Self::SystemdUser => RuntimeAuthorityPosture {
                process: ProcessAuthority::VerifiedSystemdUserScope,
                cgroup: CgroupAuthority::VerifiedSystemdUserScope,
                network: NetworkPosture::HostShared,
                user_namespace: UserNamespacePosture::None,
                persistent_identity: PersistentIdentityPosture::None,
                device_mediation: DeviceMediationPosture::None,
            },
        }
    }

    pub const fn requires_provider_agent(self) -> bool {
        matches!(self, Self::SystemdUser)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalRuntimeConfigurationError {
    BackendConfigurationInvalid,
    ConfiguredItemBoundExceeded,
    DuplicateConfiguredItem,
}

impl fmt::Display for LocalRuntimeConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BackendConfigurationInvalid => "local runtime backend configuration is invalid",
            Self::ConfiguredItemBoundExceeded => "configured runtime item bound exceeded",
            Self::DuplicateConfiguredItem => "duplicate configured runtime item",
        })
    }
}

impl Error for LocalRuntimeConfigurationError {}

#[derive(Clone)]
pub enum LocalRuntimeConfiguration {
    CloudHypervisor(Box<CloudHypervisorConfiguration>),
    QemuMedia(Box<QemuMediaConfiguration>),
    SystemdUser(SystemdUserConfiguration),
}

impl fmt::Debug for LocalRuntimeConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("LocalRuntimeConfiguration");
        debug.field("kind", &self.kind());
        if let Self::SystemdUser(configuration) = self {
            debug.field(
                "configured_item_count",
                &configuration.configured_items.len(),
            );
        }
        debug.finish_non_exhaustive()
    }
}

impl LocalRuntimeConfiguration {
    pub fn cloud_hypervisor(input: ChArgvInput) -> Result<Self, LocalRuntimeConfigurationError> {
        let configuration = CloudHypervisorConfiguration { input };
        configuration.validate()?;
        Ok(Self::CloudHypervisor(Box::new(configuration)))
    }

    pub fn qemu_media(input: QemuMediaArgvInput) -> Result<Self, LocalRuntimeConfigurationError> {
        let configuration = QemuMediaConfiguration { input };
        configuration.validate()?;
        Ok(Self::QemuMedia(Box::new(configuration)))
    }

    pub fn systemd_user(
        configured_items: Vec<ConfiguredItemId>,
    ) -> Result<Self, LocalRuntimeConfigurationError> {
        Ok(Self::SystemdUser(SystemdUserConfiguration::new(
            configured_items,
        )?))
    }

    pub const fn kind(&self) -> LocalRuntimeKind {
        match self {
            Self::CloudHypervisor(_) => LocalRuntimeKind::CloudHypervisor,
            Self::QemuMedia(_) => LocalRuntimeKind::QemuMedia,
            Self::SystemdUser(_) => LocalRuntimeKind::SystemdUser,
        }
    }

    pub fn validates_configured_item(&self, item: &ConfiguredItemId) -> bool {
        match self {
            Self::SystemdUser(configuration) => configuration.configured_items.contains(item),
            Self::CloudHypervisor(_) | Self::QemuMedia(_) => false,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), LocalRuntimeConfigurationError> {
        match self {
            Self::CloudHypervisor(configuration) => configuration.validate(),
            Self::QemuMedia(configuration) => configuration.validate(),
            Self::SystemdUser(configuration) => configuration.validate(),
        }
    }
}

#[derive(Clone)]
pub struct CloudHypervisorConfiguration {
    input: ChArgvInput,
}

impl fmt::Debug for CloudHypervisorConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CloudHypervisorConfiguration")
            .finish_non_exhaustive()
    }
}

impl CloudHypervisorConfiguration {
    fn validate(&self) -> Result<(), LocalRuntimeConfigurationError> {
        generate_ch_argv(&self.input)
            .map(|_| ())
            .map_err(|_| LocalRuntimeConfigurationError::BackendConfigurationInvalid)
    }
}

#[derive(Clone)]
pub struct QemuMediaConfiguration {
    input: QemuMediaArgvInput,
}

impl fmt::Debug for QemuMediaConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QemuMediaConfiguration")
            .finish_non_exhaustive()
    }
}

impl QemuMediaConfiguration {
    fn validate(&self) -> Result<(), LocalRuntimeConfigurationError> {
        generate_qemu_media_argv(&self.input)
            .map(|_| ())
            .map_err(|_| LocalRuntimeConfigurationError::BackendConfigurationInvalid)
    }
}

#[derive(Clone)]
pub struct SystemdUserConfiguration {
    configured_items: BTreeSet<ConfiguredItemId>,
}

impl fmt::Debug for SystemdUserConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SystemdUserConfiguration")
            .field("configured_item_count", &self.configured_items.len())
            .finish_non_exhaustive()
    }
}

impl SystemdUserConfiguration {
    fn new(
        configured_items: Vec<ConfiguredItemId>,
    ) -> Result<Self, LocalRuntimeConfigurationError> {
        if configured_items.len() > MAX_CONFIGURED_RUNTIME_ITEMS {
            return Err(LocalRuntimeConfigurationError::ConfiguredItemBoundExceeded);
        }
        let configured_item_count = configured_items.len();
        let configured_items = configured_items.into_iter().collect::<BTreeSet<_>>();
        if configured_items.len() != configured_item_count {
            return Err(LocalRuntimeConfigurationError::DuplicateConfiguredItem);
        }
        Ok(Self { configured_items })
    }

    fn validate(&self) -> Result<(), LocalRuntimeConfigurationError> {
        if self.configured_items.len() > MAX_CONFIGURED_RUNTIME_ITEMS {
            Err(LocalRuntimeConfigurationError::ConfiguredItemBoundExceeded)
        } else {
            Ok(())
        }
    }
}
