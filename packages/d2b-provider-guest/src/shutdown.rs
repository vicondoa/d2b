//! Provider-aware graceful VM shutdown seam (U7).
//!
//! The guest runtime vocabulary and the Cloud Hypervisor graceful-shutdown
//! adapter live with the crate that owns the guest runtime families; the
//! daemon's lifecycle code reads them from here and implements the
//! qemu-media leg (which rides the broker's QMP surface) behind the same
//! seam.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;

use d2bd_runtime::ch_api;

/// The local-VM runtime kinds the daemon's shutdown path dispatches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    /// The Cloud Hypervisor runtime (`runtime-cloud-hypervisor`).
    CloudHypervisor,
    /// The qemu-media runtime (`runtime-qemu-media`).
    QemuMedia,
}

impl ProviderKind {
    /// The stable metric-label spelling of this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CloudHypervisor => "cloud_hypervisor",
            Self::QemuMedia => "qemu_media",
        }
    }
}

/// One VM the daemon's shutdown path addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderShutdownTarget {
    /// VM name.
    pub vm: String,
    /// The runtime kind serving the VM.
    pub kind: ProviderKind,
    /// The runtime's API socket, when the kind exposes one.
    pub api_socket: Option<PathBuf>,
}

/// The outcome of a graceful-shutdown request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderRequestOutcome {
    /// The runtime accepted the shutdown request.
    Requested,
    /// The runtime is unavailable; `reason` is a bounded label.
    Unavailable {
        /// Bounded refusal label.
        reason: &'static str,
    },
    /// The runtime rejected the request; `reason` is a bounded label.
    Rejected {
        /// Bounded refusal label.
        reason: &'static str,
    },
    /// The runtime kind does not support the request.
    Unsupported,
}

/// The guest state one poll reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderGuestState {
    /// The guest is running.
    Running,
    /// The guest stopped.
    GuestStopped,
    /// State is unknown; `reason` is a bounded label.
    Unknown {
        /// Bounded state label.
        reason: &'static str,
    },
}

/// The outcome of a VMM-exit request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderVmmExitOutcome {
    /// The runtime accepted the exit request.
    Requested,
    /// The runtime kind does not support exit requests.
    NotSupported,
    /// The runtime is unavailable; `reason` is a bounded label.
    Unavailable {
        /// Bounded refusal label.
        reason: &'static str,
    },
}

/// The graceful-VM-shutdown port one runtime leg implements.
#[async_trait]
pub trait GracefulVmShutdown: Send + Sync {
    /// Ask the runtime to shut the guest down gracefully.
    async fn request_shutdown(&self, target: &ProviderShutdownTarget) -> ProviderRequestOutcome;
    /// Poll the guest state.
    async fn poll_state(&self, target: &ProviderShutdownTarget) -> ProviderGuestState;
    /// Ask the runtime to exit the VMM without a guest shutdown.
    async fn request_vmm_exit(&self, target: &ProviderShutdownTarget) -> ProviderVmmExitOutcome;
}

/// The Cloud Hypervisor graceful-shutdown adapter: speaks the runtime's
/// API socket directly. v3 Guest lifecycle uses the owned VMM `Process`
/// child and does not enter this direct CH API path for new launches; the
/// seam keeps the legacy VM-stop path working.
#[derive(Debug, Clone, Copy)]
pub struct CloudHypervisorShutdown {
    /// Per-call I/O timeout for the API-socket exchange.
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
    use std::io::{Read, Write};

    use super::*;

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    /// Remove one test serving directory, ignoring absence.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn remove_serving_dir(dir: &std::path::Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Serve one `vm.info` HTTP-over-unix exchange for the given wire state,
    /// and return the socket path the poll reads, plus the serving dir the
    /// caller removes when the exchange is complete.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn serve_vm_info(state: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "d2b-provider-guest-shutdown-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let socket = dir.join(format!("vm-{state}.sock"));
        let _ = std::fs::remove_file(&socket);
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("bind");
        let body = format!(r#"{{"state":"{state}"}}"#);
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = [0u8; 512];
            let _ = stream.read(&mut request).expect("read request");
            let response = format!(
                "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).expect("write response");
        });
        (socket, dir)
    }

    /// The wire-state classification of [`CloudHypervisorShutdown::poll_state`]:
    /// Created/Shutdown stop, Running/Paused run, anything else and every
    /// error answer `Unknown`.
    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn cloud_hypervisor_poll_state_classifies_the_wire_state() {
        let provider = CloudHypervisorShutdown::default();
        for (state, expected) in [
            ("Created", ProviderGuestState::GuestStopped),
            ("Shutdown", ProviderGuestState::GuestStopped),
            ("Running", ProviderGuestState::Running),
            ("Paused", ProviderGuestState::Running),
            (
                "Resuming",
                ProviderGuestState::Unknown {
                    reason: "unknown_state",
                },
            ),
        ] {
            let (socket, dir) = serve_vm_info(state);
            let target = ProviderShutdownTarget {
                vm: "work".to_owned(),
                kind: ProviderKind::CloudHypervisor,
                api_socket: Some(socket.clone()),
            };
            assert_eq!(provider.poll_state(&target).await, expected, "state {state}");
            remove_serving_dir(&dir);
        }

        // An unreachable socket is an error, and errors answer Unknown.
        let missing = std::env::temp_dir().join(format!(
            "d2b-provider-guest-shutdown-{}-missing.sock",
            std::process::id()
        ));
        let target = ProviderShutdownTarget {
            vm: "work".to_owned(),
            kind: ProviderKind::CloudHypervisor,
            api_socket: Some(missing),
        };
        assert_eq!(
            provider.poll_state(&target).await,
            ProviderGuestState::Unknown {
                reason: "api_unavailable"
            },
            "an unreachable API socket answers Unknown"
        );
    }
}
