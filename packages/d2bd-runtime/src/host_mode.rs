//! Provider-neutral Host target runtime boundary.
//!
//! Host mode keeps the existing d2bd composition responsible for the Zone
//! store, public operator socket, realm routing, and Host-target Provider
//! assignments. The type is intentionally small: the existing Host startup
//! path remains the authoritative implementation, while this module gives
//! shared code an explicit mode-bound surface contract.

use std::path::PathBuf;

use crate::target_runtime::{AdmissionLimits, DaemonMode, ModeSurfaces, ProviderDeployment};

/// Host-only startup inputs. Guest mode has no conversion from this type.
#[derive(Debug, Clone)]
pub struct HostRuntimeConfig {
    pub public_socket: PathBuf,
    pub broker_socket: PathBuf,
    pub state_dir: PathBuf,
    pub realm_identity: PathBuf,
}

/// Provider-neutral Host runtime descriptor used by static composition.
#[derive(Debug, Clone)]
pub struct HostRuntime {
    deployment: ProviderDeployment,
    config: HostRuntimeConfig,
}

impl HostRuntime {
    pub fn new(
        config: HostRuntimeConfig,
        limits: AdmissionLimits,
    ) -> Result<Self, crate::target_runtime::AdmissionError> {
        Ok(Self {
            deployment: ProviderDeployment::new(DaemonMode::Host, limits)?,
            config,
        })
    }

    pub const fn mode(&self) -> DaemonMode {
        DaemonMode::Host
    }

    pub const fn surfaces(&self) -> ModeSurfaces {
        DaemonMode::Host.surfaces()
    }

    pub fn deployment(&self) -> &ProviderDeployment {
        &self.deployment
    }

    pub fn config(&self) -> &HostRuntimeConfig {
        &self.config
    }
}
