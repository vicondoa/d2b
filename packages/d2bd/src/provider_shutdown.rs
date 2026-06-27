//! Provider-aware graceful VM shutdown seam for daemon lifecycle code.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;

use crate::ch_api;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    CloudHypervisor,
    QemuMedia,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CloudHypervisor => "cloud_hypervisor",
            Self::QemuMedia => "qemu_media",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderShutdownTarget {
    pub vm: String,
    pub kind: ProviderKind,
    pub api_socket: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRequestOutcome {
    Requested,
    Unavailable { reason: &'static str },
    Rejected { reason: &'static str },
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderGuestState {
    Running,
    GuestStopped,
    Unknown { reason: &'static str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderVmmExitOutcome {
    Requested,
    NotSupported,
    Unavailable { reason: &'static str },
}

#[async_trait]
pub trait GracefulVmShutdown: Send + Sync {
    async fn request_shutdown(&self, target: &ProviderShutdownTarget) -> ProviderRequestOutcome;
    async fn poll_state(&self, target: &ProviderShutdownTarget) -> ProviderGuestState;
    async fn request_vmm_exit(&self, target: &ProviderShutdownTarget) -> ProviderVmmExitOutcome;
}

#[derive(Debug, Clone, Copy)]
pub struct CloudHypervisorShutdown {
    pub io_timeout: Duration,
}

impl Default for CloudHypervisorShutdown {
    fn default() -> Self {
        Self {
            io_timeout: ch_api::DEFAULT_TIMEOUT,
        }
    }
}

#[async_trait]
impl GracefulVmShutdown for CloudHypervisorShutdown {
    async fn request_shutdown(&self, target: &ProviderShutdownTarget) -> ProviderRequestOutcome {
        let Some(socket) = target.api_socket.as_deref() else {
            return ProviderRequestOutcome::Unavailable {
                reason: "api_unavailable",
            };
        };
        match ch_api::shutdown_vm(socket, self.io_timeout).await {
            Ok(()) => ProviderRequestOutcome::Requested,
            Err(error) => match error {
                ch_api::ChApiError::Rejected(_) => ProviderRequestOutcome::Rejected {
                    reason: error.bounded_label(),
                },
                _ => ProviderRequestOutcome::Unavailable {
                    reason: error.bounded_label(),
                },
            },
        }
    }

    async fn poll_state(&self, target: &ProviderShutdownTarget) -> ProviderGuestState {
        let Some(socket) = target.api_socket.as_deref() else {
            return ProviderGuestState::Unknown {
                reason: "api_unavailable",
            };
        };
        match ch_api::get_vm_info(socket, self.io_timeout).await {
            Ok(info) => match info.state.as_deref() {
                Some("Created" | "Shutdown") => ProviderGuestState::GuestStopped,
                Some("Running" | "Paused") => ProviderGuestState::Running,
                Some(_) | None => ProviderGuestState::Unknown {
                    reason: "unknown_state",
                },
            },
            Err(error) => ProviderGuestState::Unknown {
                reason: error.bounded_label(),
            },
        }
    }

    async fn request_vmm_exit(&self, _target: &ProviderShutdownTarget) -> ProviderVmmExitOutcome {
        ProviderVmmExitOutcome::NotSupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cloud_hypervisor_provider_fails_closed_without_api_socket() {
        let provider = CloudHypervisorShutdown::default();
        let target = ProviderShutdownTarget {
            vm: "work".to_owned(),
            kind: ProviderKind::CloudHypervisor,
            api_socket: None,
        };

        assert_eq!(
            provider.request_shutdown(&target).await,
            ProviderRequestOutcome::Unavailable {
                reason: "api_unavailable"
            }
        );
        assert_eq!(
            provider.poll_state(&target).await,
            ProviderGuestState::Unknown {
                reason: "api_unavailable"
            }
        );
        assert_eq!(
            provider.request_vmm_exit(&target).await,
            ProviderVmmExitOutcome::NotSupported
        );
    }
}
