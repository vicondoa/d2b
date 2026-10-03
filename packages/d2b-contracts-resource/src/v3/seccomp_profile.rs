//! The `SeccompProfile` ResourceType: syscall-filter policy and nothing else.
//!
//! A profile row states which syscalls a workload may make and what happens
//! to the rest. Namespace isolation, cgroup confinement, device-node access,
//! and mount authority are not syscall-filter concerns: they belong to the
//! execution contract that requires them and to the typed binding that
//! admits the resource. A profile that also carried them would grant access
//! through a name a provider or role happens to select, which is exactly the
//! indirection this contract removes.
//!
//! The wire mirror denies unknown fields, so a row that still carries a
//! namespace set, a cgroup set, or a device-node bind is rejected outright
//! rather than being silently ignored. Ignoring a mandatory control is never
//! a success: a deployment that cannot honor a declared restriction is
//! refused at admission, and a row that cannot express its intent does not
//! decode.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::execution_policy::{BoundedToken, PrimitiveSpecError, ensure_unique, redacted_debug};
use d2b_contracts::wire_deserialize;

/// Canonical `SeccompProfile` ResourceType name.
pub const SECCOMP_PROFILE_RESOURCE_TYPE: &str = "SeccompProfile";
/// Maximum syscalls in one allowlist.
pub const MAX_SECCOMP_SYSCALLS: usize = 1024;

/// One invalid `SeccompProfile` declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeccompContractError {
    /// The syscall allowlist exceeded its frozen ceiling.
    TooManySyscalls,
    /// The syscall allowlist carried a duplicate entry.
    DuplicateSyscall,
    /// A syscall name was not a bounded lower-kebab token.
    InvalidSyscallName,
}

impl core::fmt::Display for SeccompContractError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooManySyscalls => f.write_str("syscall allowlist exceeds its frozen bound"),
            Self::DuplicateSyscall => f.write_str("syscall allowlist entries must be unique"),
            Self::InvalidSyscallName => f.write_str("syscall name is invalid"),
        }
    }
}

impl std::error::Error for SeccompContractError {}

/// What the kernel does with a syscall outside the allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SyscallDefaultAction {
    /// Terminate the workload.
    KillProcess,
    /// Terminate the workload and record the core.
    KillThread,
    /// Raise `SIGSYS`.
    Trap,
    /// Return `EPERM` without terminating.
    Errno,
}

impl SyscallDefaultAction {
    /// The action's canonical wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::KillProcess => "kill-process",
            Self::KillThread => "kill-thread",
            Self::Trap => "trap",
            Self::Errno => "errno",
        }
    }
}

/// The syscall filter one profile declares.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyscallFilter {
    default_action: SyscallDefaultAction,
    allowed: Vec<BoundedToken>,
}

impl SyscallFilter {
    /// Construct a filter after checking its bound and uniqueness.
    pub fn new(
        default_action: SyscallDefaultAction,
        allowed: Vec<BoundedToken>,
    ) -> Result<Self, SeccompContractError> {
        if allowed.len() > MAX_SECCOMP_SYSCALLS {
            return Err(SeccompContractError::TooManySyscalls);
        }
        ensure_unique(&allowed).map_err(|error| match error {
            PrimitiveSpecError::DuplicateEntry => SeccompContractError::DuplicateSyscall,
            _ => SeccompContractError::InvalidSyscallName,
        })?;
        Ok(Self {
            default_action,
            allowed,
        })
    }

    /// What happens to a syscall outside the allowlist.
    pub const fn default_action(&self) -> SyscallDefaultAction {
        self.default_action
    }

    /// The allowed syscalls.
    pub fn allowed(&self) -> &[BoundedToken] {
        &self.allowed
    }

    /// Whether the filter admits `syscall`.
    pub fn allows(&self, syscall: &BoundedToken) -> bool {
        self.allowed.contains(syscall)
    }
}

redacted_debug!(SyscallFilter);

/// The `SeccompProfile` desired spec.
///
/// The spec has exactly one field. There is no namespace set, no cgroup set,
/// no device-node bind, no mount, and no host path here, and none can be
/// added without changing this contract's meaning.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeccompProfileSpec {
    filter: SyscallFilter,
}

impl SeccompProfileSpec {
    /// Construct a profile from its syscall filter.
    pub const fn new(filter: SyscallFilter) -> Self {
        Self { filter }
    }

    /// Borrow the declared syscall filter.
    pub const fn filter(&self) -> &SyscallFilter {
        &self.filter
    }
}

redacted_debug!(SeccompProfileSpec);

wire_deserialize!(
    SeccompProfileSpec,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        filter: SyscallFilter,
    },
    wire,
    Ok(SeccompProfileSpec::new(wire.filter))
);

#[cfg(test)]
mod tests {
    use super::*;

    fn token(value: &str) -> BoundedToken {
        BoundedToken::parse(value).expect("token")
    }

    #[test]
    fn a_profile_declares_only_its_syscall_filter() {
        let json = String::from_utf8(
            crate::v3::resource_schema::canonical_json_bytes(
                &SeccompProfileSpec::new(
                    SyscallFilter::new(SyscallDefaultAction::Errno, vec![token("read")])
                        .expect("filter"),
                ),
            )
            .expect("canonical"),
        )
        .expect("utf8");
        assert!(json.contains("\"filter\""));
        for forbidden in ["namespace", "cgroup", "deviceBind", "mount", "path"] {
            assert!(!json.contains(forbidden), "{forbidden} is not a profile field");
        }
    }

    #[test]
    fn a_row_carrying_the_old_authority_fields_is_rejected() {
        for row in [
            r#"{"filter":{"defaultAction":"errno","allowed":["read"]},"namespaces":{"mount":true}}"#,
            r#"{"filter":{"defaultAction":"errno","allowed":["read"]},"cgroups":{}}"#,
            r#"{"filter":{"defaultAction":"errno","allowed":["read"]},"deviceBinds":[]}"#,
        ] {
            assert!(
                serde_json::from_str::<SeccompProfileSpec>(row).is_err(),
                "a profile that still carries non-syscall authority must be rejected"
            );
        }
    }

    #[test]
    fn the_allowlist_is_bounded_and_unique() {
        let allowed = (0..=MAX_SECCOMP_SYSCALLS as u32)
            .map(|index| token(&format!("call-{index}")))
            .collect();
        assert_eq!(
            SyscallFilter::new(SyscallDefaultAction::Trap, allowed),
            Err(SeccompContractError::TooManySyscalls)
        );
        assert_eq!(
            SyscallFilter::new(
                SyscallDefaultAction::Trap,
                vec![token("read"), token("read")]
            ),
            Err(SeccompContractError::DuplicateSyscall)
        );
    }

    #[test]
    fn the_filter_admits_exactly_its_listed_syscalls() {
        let filter =
            SyscallFilter::new(SyscallDefaultAction::KillProcess, vec![token("read"), token("write")])
                .expect("filter");
        assert!(filter.allows(&token("read")));
        assert!(!filter.allows(&token("mount")));
        assert_eq!(filter.default_action(), SyscallDefaultAction::KillProcess);
    }
}
