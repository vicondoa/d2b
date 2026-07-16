use std::{error::Error, fmt};

use d2b_contracts::{
    v2_identity::ProviderType,
    v2_provider::{
        CgroupAuthority, DeviceMediationPosture, ImplementationId, NetworkPosture,
        PersistentIdentityPosture, ProcessAuthority, ProviderContractError, ProviderFactoryKey,
        RuntimeAuthorityPosture, UserNamespacePosture,
    },
};

pub const CLOUD_HYPERVISOR_IMPLEMENTATION_ID: &str = "cloud-hypervisor";
pub const QEMU_MEDIA_IMPLEMENTATION_ID: &str = "qemu-media";
pub const SYSTEMD_USER_IMPLEMENTATION_ID: &str = "systemd-user";
pub const MAX_RUNTIME_OPAQUE_ID_BYTES: usize = 128;

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

    pub const fn uses_user_agent(self) -> bool {
        matches!(self, Self::SystemdUser)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalRuntimeConfigurationError {
    InvalidOpaqueIdentifier,
}

impl fmt::Display for LocalRuntimeConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("local runtime opaque identifier is invalid")
    }
}

impl Error for LocalRuntimeConfigurationError {}

macro_rules! opaque_runtime_id {
    ($name:ident, $debug_name:literal) => {
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn parse(value: impl Into<String>) -> Result<Self, LocalRuntimeConfigurationError> {
                let value = value.into();
                if opaque_id_is_valid(&value) {
                    Ok(Self(value))
                } else {
                    Err(LocalRuntimeConfigurationError::InvalidOpaqueIdentifier)
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!($debug_name, "(<redacted>)"))
            }
        }
    };
}

opaque_runtime_id!(RuntimeBundleIntentId, "RuntimeBundleIntentId");
opaque_runtime_id!(RuntimeRunnerId, "RuntimeRunnerId");

fn opaque_id_is_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_RUNTIME_OPAQUE_ID_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b":_-.".contains(&byte)
        })
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
}

#[derive(Clone, PartialEq, Eq)]
pub struct RuntimeIntentBinding {
    bundle_intent_id: RuntimeBundleIntentId,
    runner_id: RuntimeRunnerId,
}

impl fmt::Debug for RuntimeIntentBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeIntentBinding")
            .finish_non_exhaustive()
    }
}

impl RuntimeIntentBinding {
    pub fn new(bundle_intent_id: RuntimeBundleIntentId, runner_id: RuntimeRunnerId) -> Self {
        Self {
            bundle_intent_id,
            runner_id,
        }
    }

    pub fn bundle_intent_id(&self) -> &RuntimeBundleIntentId {
        &self.bundle_intent_id
    }

    pub fn runner_id(&self) -> &RuntimeRunnerId {
        &self.runner_id
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum LocalRuntimeConfiguration {
    CloudHypervisor(RuntimeIntentBinding),
    QemuMedia(RuntimeIntentBinding),
    SystemdUser(RuntimeIntentBinding),
}

impl fmt::Debug for LocalRuntimeConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalRuntimeConfiguration")
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}

impl LocalRuntimeConfiguration {
    pub fn cloud_hypervisor(
        bundle_intent_id: RuntimeBundleIntentId,
        runner_id: RuntimeRunnerId,
    ) -> Self {
        Self::CloudHypervisor(RuntimeIntentBinding::new(bundle_intent_id, runner_id))
    }

    pub fn qemu_media(bundle_intent_id: RuntimeBundleIntentId, runner_id: RuntimeRunnerId) -> Self {
        Self::QemuMedia(RuntimeIntentBinding::new(bundle_intent_id, runner_id))
    }

    pub fn systemd_user(
        bundle_intent_id: RuntimeBundleIntentId,
        runner_id: RuntimeRunnerId,
    ) -> Self {
        Self::SystemdUser(RuntimeIntentBinding::new(bundle_intent_id, runner_id))
    }

    pub const fn kind(&self) -> LocalRuntimeKind {
        match self {
            Self::CloudHypervisor(_) => LocalRuntimeKind::CloudHypervisor,
            Self::QemuMedia(_) => LocalRuntimeKind::QemuMedia,
            Self::SystemdUser(_) => LocalRuntimeKind::SystemdUser,
        }
    }

    pub fn intent_binding(&self) -> &RuntimeIntentBinding {
        match self {
            Self::CloudHypervisor(binding)
            | Self::QemuMedia(binding)
            | Self::SystemdUser(binding) => binding,
        }
    }
}
