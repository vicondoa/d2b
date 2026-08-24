//! One-shot detached Process resource routing.
//!
//! Production detached create/list/logs/status/kill uses
//! [`ResourceDetachedClient`] and the authenticated Resource API. No
//! feature-specific component-session adapter is retained.

use async_trait::async_trait;
use d2b_contracts_control::public_wire::{
    ExecDetachedCreateResult, ExecDetachedKillResult, ExecDetachedListResult,
    ExecDetachedLogsResult, ExecDetachedStatusResult,
};
#[cfg(test)]
use d2b_contracts_control::public_wire::{ExecDetachedKillOutcome, ExecState};
use d2b_contracts_resource::v3::ResourceRef;

use crate::exec_session::{ExecOpError, ExecStartSpec};

/// Resource API seam for detached EphemeralProcess management.
///
/// Implementations perform the authenticated Resource API calls and named
/// stream/log attachment. No broker role, guest socket, process identifier, or
/// transport locator crosses this boundary.
#[async_trait]
pub trait DetachedProcessResourcePort: Send + Sync {
    /// Create one detached EphemeralProcess.
    async fn create_ephemeral_process(
        &self,
        execution_ref: &ResourceRef,
        spec: &ExecStartSpec,
    ) -> Result<ExecDetachedCreateResult, ExecOpError>;

    /// List detached EphemeralProcess resources under one execution target.
    async fn list_ephemeral_processes(
        &self,
        execution_ref: &ResourceRef,
    ) -> Result<ExecDetachedListResult, ExecOpError>;

    /// Read one EphemeralProcess status.
    async fn status_ephemeral_process(
        &self,
        process_ref: &ResourceRef,
    ) -> Result<ExecDetachedStatusResult, ExecOpError>;

    /// Read retained EphemeralProcess output.
    async fn logs_ephemeral_process(
        &self,
        process_ref: &ResourceRef,
        stdout_offset: Option<u64>,
        stderr_offset: Option<u64>,
        max_len: Option<u64>,
    ) -> Result<ExecDetachedLogsResult, ExecOpError>;

    /// Cancel one EphemeralProcess.
    async fn kill_ephemeral_process(
        &self,
        process_ref: &ResourceRef,
    ) -> Result<ExecDetachedKillResult, ExecOpError>;
}

/// Detached execution facade over the authenticated Resource API.
pub struct ResourceDetachedClient<P> {
    port: P,
    execution_ref: ResourceRef,
}

impl<P> std::fmt::Debug for ResourceDetachedClient<P> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ResourceDetachedClient(<authenticated-resource-port>)")
    }
}

impl<P> ResourceDetachedClient<P> {
    /// Bind detached operations to one exact Host or Guest execution target.
    pub fn new(port: P, execution_ref: ResourceRef) -> Result<Self, ExecOpError> {
        if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
            return Err(ExecOpError::Protocol);
        }
        Ok(Self {
            port,
            execution_ref,
        })
    }

    /// Borrow the target bound before resource admission.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }
}

impl<P> ResourceDetachedClient<P>
where
    P: DetachedProcessResourcePort,
{
    /// Create one detached EphemeralProcess resource.
    pub async fn create(
        &self,
        spec: &ExecStartSpec,
    ) -> Result<ExecDetachedCreateResult, ExecOpError> {
        if !spec.detached {
            return Err(ExecOpError::Protocol);
        }
        self.port
            .create_ephemeral_process(&self.execution_ref, spec)
            .await
    }

    /// List detached EphemeralProcess resources.
    pub async fn list(&self) -> Result<ExecDetachedListResult, ExecOpError> {
        self.port
            .list_ephemeral_processes(&self.execution_ref)
            .await
    }

    /// Read one detached EphemeralProcess status.
    pub async fn status(
        &self,
        process_ref: &ResourceRef,
    ) -> Result<ExecDetachedStatusResult, ExecOpError> {
        validate_ephemeral_process_ref(process_ref)?;
        self.port.status_ephemeral_process(process_ref).await
    }

    /// Read retained detached output.
    pub async fn logs(
        &self,
        process_ref: &ResourceRef,
        stdout_offset: Option<u64>,
        stderr_offset: Option<u64>,
        max_len: Option<u64>,
    ) -> Result<ExecDetachedLogsResult, ExecOpError> {
        validate_ephemeral_process_ref(process_ref)?;
        self.port
            .logs_ephemeral_process(process_ref, stdout_offset, stderr_offset, max_len)
            .await
    }

    /// Cancel one detached EphemeralProcess.
    pub async fn kill(
        &self,
        process_ref: &ResourceRef,
    ) -> Result<ExecDetachedKillResult, ExecOpError> {
        validate_ephemeral_process_ref(process_ref)?;
        self.port.kill_ephemeral_process(process_ref).await
    }
}

fn validate_ephemeral_process_ref(process_ref: &ResourceRef) -> Result<(), ExecOpError> {
    if process_ref.resource_type().as_str() == "EphemeralProcess" {
        Ok(())
    } else {
        Err(ExecOpError::Protocol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingResourcePort {
        calls: std::sync::Mutex<Vec<&'static str>>,
    }

    #[async_trait]
    impl DetachedProcessResourcePort for RecordingResourcePort {
        async fn create_ephemeral_process(
            &self,
            execution_ref: &ResourceRef,
            _spec: &ExecStartSpec,
        ) -> Result<ExecDetachedCreateResult, ExecOpError> {
            assert_eq!(execution_ref.resource_type().as_str(), "Guest");
            self.calls.lock().unwrap().push("create");
            Ok(ExecDetachedCreateResult {
                exec_id: "resource-id".to_owned(),
                state: ExecState::Created,
            })
        }

        async fn list_ephemeral_processes(
            &self,
            _execution_ref: &ResourceRef,
        ) -> Result<ExecDetachedListResult, ExecOpError> {
            self.calls.lock().unwrap().push("list");
            Ok(ExecDetachedListResult { execs: Vec::new() })
        }

        async fn status_ephemeral_process(
            &self,
            process_ref: &ResourceRef,
        ) -> Result<ExecDetachedStatusResult, ExecOpError> {
            assert_eq!(process_ref.resource_type().as_str(), "EphemeralProcess");
            self.calls.lock().unwrap().push("status");
            Ok(ExecDetachedStatusResult {
                exec_id: process_ref.name().as_str().to_owned(),
                state: ExecState::Running,
                reason: None,
                exit_code: None,
                signal: None,
                start_offset: 0,
                end_offset: 0,
                dropped_bytes: 0,
                truncated: false,
            })
        }

        async fn logs_ephemeral_process(
            &self,
            _process_ref: &ResourceRef,
            _stdout_offset: Option<u64>,
            _stderr_offset: Option<u64>,
            _max_len: Option<u64>,
        ) -> Result<ExecDetachedLogsResult, ExecOpError> {
            self.calls.lock().unwrap().push("logs");
            Ok(ExecDetachedLogsResult {
                exec_id: "resource-id".to_owned(),
                stdout_base64: String::new(),
                stderr_base64: String::new(),
                start_offset: 0,
                end_offset: 0,
                dropped_bytes: 0,
                truncated: false,
                stdout_start_offset: 0,
                stdout_end_offset: 0,
                stdout_next_offset: 0,
                stdout_eof: true,
                stdout_dropped_bytes: 0,
                stdout_truncated: false,
                stderr_start_offset: 0,
                stderr_end_offset: 0,
                stderr_next_offset: 0,
                stderr_eof: true,
                stderr_dropped_bytes: 0,
                stderr_truncated: false,
            })
        }

        async fn kill_ephemeral_process(
            &self,
            _process_ref: &ResourceRef,
        ) -> Result<ExecDetachedKillResult, ExecOpError> {
            self.calls.lock().unwrap().push("kill");
            Ok(ExecDetachedKillResult {
                exec_id: "resource-id".to_owned(),
                result: ExecDetachedKillOutcome::Cancelling,
                state: ExecState::Running,
            })
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resource_detached_client_routes_lifecycle_by_resource_ref() {
        let port = RecordingResourcePort::default();
        let client =
            ResourceDetachedClient::new(port, ResourceRef::parse("Guest/work").unwrap()).unwrap();
        let spec = ExecStartSpec {
            vm: "work".to_owned(),
            request_id: None,
            argv: vec!["true".to_owned()],
            tty: false,
            detached: true,
            env: Vec::new(),
            cwd: None,
            term_size: None,
        };
        assert_eq!(client.create(&spec).await.unwrap().exec_id, "resource-id");
        assert!(
            client
                .status(&ResourceRef::parse("Process/wrong").unwrap())
                .await
                .is_err()
        );
        assert_eq!(client.execution_ref().to_canonical_string(), "Guest/work");
    }
}
