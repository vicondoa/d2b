use std::fmt;
use std::os::fd::{AsFd, OwnedFd};

use d2b_contracts::v2_services::{
    MethodSpec, ServiceSpec, broker, validate_spawn_response_for_request,
};
use d2b_host::realm_children::{RealmChildLaunchRecord, RealmChildRole};

use crate::realm_child_supervisor::{RealmChildHandle, RealmChildPair, pidfd_kernel_identity};

pub(super) fn owns(service: &ServiceSpec, method: &MethodSpec) -> bool {
    service.package == "d2b.broker.v2" && matches!(method.name, "Allocate" | "Spawn")
}

/// Consume a successful Spawn response and its SCM_RIGHTS table. Correlation
/// is checked before either pidfd enters the supervisor.
pub fn consume_spawn_handoff(
    request: &broker::SpawnRealmChildrenRequest,
    response: &broker::SpawnRealmChildrenResponse,
    attachments: Vec<OwnedFd>,
    record: &RealmChildLaunchRecord,
) -> Result<RealmChildPair, AllocatorControlError> {
    validate_spawn_response_for_request(request, response)
        .map_err(|_| AllocatorControlError::ResponseCorrelation)?;
    if attachments.len() != 2 || response.children.len() != 2 {
        return Err(AllocatorControlError::AttachmentCorrelation);
    }
    record
        .validate_for_request(
            &request.realm_id,
            &request.controller_generation_id,
            &request.controller_process_id,
            &request.broker_process_id,
            &request.launch_record_digest,
        )
        .map_err(|_| AllocatorControlError::LaunchRecord)?;

    let mut slots = attachments
        .into_iter()
        .map(Some)
        .collect::<Vec<Option<OwnedFd>>>();
    let mut controller = None;
    let mut child_broker = None;
    for child in &response.children {
        let role = match child.role.value() {
            1 => RealmChildRole::Controller,
            2 => RealmChildRole::Broker,
            _ => return Err(AllocatorControlError::ResponseCorrelation),
        };
        let expected = match role {
            RealmChildRole::Controller => &record.controller,
            RealmChildRole::Broker => &record.broker,
        };
        if child.executable_digest.as_slice() != expected.executable_digest {
            return Err(AllocatorControlError::LaunchRecord);
        }
        let pidfd = slots
            .get_mut(child.pidfd_attachment_index as usize)
            .and_then(Option::take)
            .ok_or(AllocatorControlError::AttachmentCorrelation)?;
        let pidfd_identity = pidfd_kernel_identity(pidfd.as_fd())
            .map_err(|_| AllocatorControlError::AttachmentCorrelation)?;
        let handle = RealmChildHandle {
            role,
            process_id: child.process_id.clone(),
            pid: child.pid,
            pidfd,
            pidfd_identity,
            executable: expected.executable.clone(),
            executable_digest: expected.executable_digest,
            controller_generation_id: request.controller_generation_id.clone(),
            cgroup_leaf: record.cgroup_leaf(role),
        };
        match role {
            RealmChildRole::Controller => {
                if controller.replace(handle).is_some() {
                    return Err(AllocatorControlError::ResponseCorrelation);
                }
            }
            RealmChildRole::Broker => {
                if child_broker.replace(handle).is_some() {
                    return Err(AllocatorControlError::ResponseCorrelation);
                }
            }
        }
    }
    if slots.iter().any(Option::is_some) {
        return Err(AllocatorControlError::AttachmentCorrelation);
    }
    Ok(RealmChildPair {
        realm_id: request.realm_id.clone(),
        controller: controller.ok_or(AllocatorControlError::ResponseCorrelation)?,
        broker: child_broker.ok_or(AllocatorControlError::ResponseCorrelation)?,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocatorControlError {
    ResponseCorrelation,
    AttachmentCorrelation,
    LaunchRecord,
}

impl fmt::Display for AllocatorControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ResponseCorrelation => "Spawn response does not match request",
            Self::AttachmentCorrelation => "Spawn pidfd attachment correlation failed",
            Self::LaunchRecord => "Spawn response does not match the trusted launch record",
        })
    }
}

impl std::error::Error for AllocatorControlError {}
