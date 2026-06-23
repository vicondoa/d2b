use crate::processes::{
    NodeId, ProcessNode, ProcessRole, ReadinessPredicate, RoleProfile, SpawnRunnerPlanOp,
};
use std::fmt;
use std::path::{Path, PathBuf};

/// Pure builder for the existing serialized [`ProcessNode`] shape.
#[derive(Debug, Clone)]
pub struct ProcessNodeBuilder {
    id: Option<NodeId>,
    role: ProcessRole,
    unit: Option<String>,
    binary_path: Option<String>,
    argv: Vec<String>,
    env: Vec<String>,
    plan_ops: Vec<SpawnRunnerPlanOp>,
    profile: RoleProfile,
    readiness: Vec<ReadinessPredicate>,
}

impl ProcessNodeBuilder {
    pub fn new(role: ProcessRole, profile: RoleProfile) -> Self {
        Self {
            id: None,
            role,
            unit: None,
            binary_path: None,
            argv: Vec::new(),
            env: Vec::new(),
            plan_ops: Vec::new(),
            profile,
            readiness: Vec::new(),
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(NodeId(id.into()));
        self
    }

    pub fn with_node_id(mut self, id: NodeId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    pub fn with_binary_path(mut self, binary_path: impl Into<String>) -> Self {
        self.binary_path = Some(binary_path.into());
        self
    }

    pub fn with_argv(mut self, argv: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.argv = argv.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.argv.push(arg.into());
        self
    }

    pub fn with_env(mut self, env: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.env = env.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_env_entry(mut self, entry: impl Into<String>) -> Self {
        self.env.push(entry.into());
        self
    }

    pub fn with_plan_ops(mut self, plan_ops: impl IntoIterator<Item = SpawnRunnerPlanOp>) -> Self {
        self.plan_ops = plan_ops.into_iter().collect();
        self
    }

    pub fn with_plan_op(mut self, plan_op: SpawnRunnerPlanOp) -> Self {
        self.plan_ops.push(plan_op);
        self
    }

    pub fn with_readiness(
        mut self,
        readiness: impl IntoIterator<Item = ReadinessPredicate>,
    ) -> Self {
        self.readiness = readiness.into_iter().collect();
        self
    }

    pub fn with_readiness_predicate(mut self, readiness: ReadinessPredicate) -> Self {
        self.readiness.push(readiness);
        self
    }

    pub fn build(self) -> Result<ProcessNode, ProcessNodeBuildError> {
        let id = self
            .id
            .ok_or_else(|| ProcessNodeBuildError::MissingNodeId {
                role: self.role.clone(),
            })?;
        if id.0.is_empty() {
            return Err(ProcessNodeBuildError::EmptyNodeId {
                role: self.role.clone(),
            });
        }
        if self.binary_path.is_some() && self.argv.is_empty() {
            return Err(ProcessNodeBuildError::RunnerBinaryWithoutArgv {
                node_id: id.0,
                role: self.role,
            });
        }
        if is_spawnable_runner_role(&self.role)
            && self
                .binary_path
                .as_deref()
                .is_some_and(|binary_path| !Path::new(binary_path).is_absolute())
        {
            return Err(ProcessNodeBuildError::RelativeRunnerBinaryPath {
                node_id: id.0,
                role: self.role,
            });
        }
        for (idx, readiness) in self.readiness.iter().enumerate() {
            if self.readiness[..idx].contains(readiness) {
                return Err(ProcessNodeBuildError::DuplicateReadiness {
                    node_id: id.0,
                    role: self.role,
                    readiness_kind: readiness_kind(readiness),
                });
            }
        }
        Ok(ProcessNode {
            id,
            role: self.role,
            unit: self.unit,
            binary_path: self.binary_path,
            argv: self.argv,
            env: self.env,
            plan_ops: self.plan_ops,
            profile: self.profile,
            readiness: self.readiness,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessNodeBuildError {
    MissingNodeId {
        role: ProcessRole,
    },
    EmptyNodeId {
        role: ProcessRole,
    },
    RunnerBinaryWithoutArgv {
        node_id: String,
        role: ProcessRole,
    },
    RelativeRunnerBinaryPath {
        node_id: String,
        role: ProcessRole,
    },
    DuplicateReadiness {
        node_id: String,
        role: ProcessRole,
        readiness_kind: &'static str,
    },
}

impl fmt::Display for ProcessNodeBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingNodeId { role } => {
                write!(f, "process node for role {role:?} is missing an id")
            }
            Self::EmptyNodeId { role } => write!(f, "process node for role {role:?} has empty id"),
            Self::RunnerBinaryWithoutArgv { node_id, role } => write!(
                f,
                "process node {node_id} for role {role:?} declares a binary without argv"
            ),
            Self::RelativeRunnerBinaryPath { node_id, role } => write!(
                f,
                "process node {node_id} for role {role:?} declares a non-absolute runner binary"
            ),
            Self::DuplicateReadiness {
                node_id,
                role,
                readiness_kind,
            } => write!(
                f,
                "process node {node_id} for role {role:?} has duplicate {readiness_kind} readiness"
            ),
        }
    }
}

impl std::error::Error for ProcessNodeBuildError {}

pub fn readiness_api_socket_info(value: impl Into<String>) -> ReadinessPredicate {
    ReadinessPredicate::ApiSocketInfo(value.into())
}

pub fn readiness_vsock_notify(value: impl Into<String>) -> ReadinessPredicate {
    ReadinessPredicate::VsockNotify(value.into())
}

pub fn readiness_unix_socket_exists(path: impl Into<String>) -> ReadinessPredicate {
    ReadinessPredicate::UnixSocketExists(path.into())
}

pub fn readiness_unix_socket_listening(path: impl Into<String>) -> ReadinessPredicate {
    ReadinessPredicate::UnixSocketListening(path.into())
}

pub fn readiness_tcp_port(host: impl Into<String>, port: u16) -> ReadinessPredicate {
    ReadinessPredicate::TcpPort {
        host: host.into(),
        port,
    }
}

pub fn readiness_command(argv: impl IntoIterator<Item = impl Into<String>>) -> ReadinessPredicate {
    ReadinessPredicate::Command(argv.into_iter().map(Into::into).collect())
}

pub fn readiness_component_specific(value: impl Into<String>) -> ReadinessPredicate {
    ReadinessPredicate::ComponentSpecific(value.into())
}

pub fn readiness_guest_control_health(vm: impl Into<String>) -> ReadinessPredicate {
    ReadinessPredicate::GuestControlHealth { vm: vm.into() }
}

pub fn disk_init_plan_op(
    target_path: impl Into<PathBuf>,
    size_bytes: u64,
    mode: u32,
    owner_uid: u32,
    owner_gid: u32,
    if_absent: bool,
) -> SpawnRunnerPlanOp {
    SpawnRunnerPlanOp::DiskInit {
        target_path: target_path.into(),
        size_bytes,
        mode,
        owner_uid,
        owner_gid,
        if_absent,
    }
}

fn is_spawnable_runner_role(role: &ProcessRole) -> bool {
    !matches!(
        role,
        ProcessRole::HostReconcile
            | ProcessRole::StoreVirtiofsPreflight
            | ProcessRole::GuestSshReadiness
            | ProcessRole::GuestControlHealth
    )
}

fn readiness_kind(readiness: &ReadinessPredicate) -> &'static str {
    match readiness {
        ReadinessPredicate::ApiSocketInfo(_) => "api-socket-info",
        ReadinessPredicate::VsockNotify(_) => "vsock-notify",
        ReadinessPredicate::UnixSocketExists(_) => "unix-socket-exists",
        ReadinessPredicate::UnixSocketListening(_) => "unix-socket-listening",
        ReadinessPredicate::TcpPort { .. } => "tcp-port",
        ReadinessPredicate::Command(_) => "command",
        ReadinessPredicate::ComponentSpecific(_) => "component-specific",
        ReadinessPredicate::GuestControlHealth { .. } => "guest-control-health",
    }
}
