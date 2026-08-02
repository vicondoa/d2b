//! Opaque process identity, verified-binding sets, and pidfd evidence.

use std::collections::BTreeSet;
use std::fmt;

use serde::Serialize;

/// Who calls `wait` and reaps the launched process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WaitReapOwner {
    /// d2b owns wait and reap; the process is a direct descendant of the
    /// local effect owner. This is the `system-minijail` posture.
    Local,
    /// The service manager owns wait and reap; d2b holds only a verified
    /// pidfd. This is the `system-systemd` posture.
    ServiceManager,
}

impl WaitReapOwner {
    /// Whether this owner is the privileged broker parent that must relay
    /// terminal status after wait/reap.
    pub const fn is_broker(self) -> bool {
        matches!(self, Self::Local)
    }
}

/// One stable identity property an effect adapter can verify before the
/// Provider treats a process as its own.
///
/// The set is closed. `system-minijail` verifies pid, process start time,
/// cgroup, executable, template, and generation; `system-systemd` binds the
/// unit InvocationID, cgroup, MainPID, process start time, and the
/// Provider, template, and generation triple. A unit name alone is never an
/// identity, so it is not a member here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum IdentityBinding {
    /// The observed process id.
    Pid,
    /// The observed process start time, which distinguishes a reused pid.
    ProcessStartTime,
    /// The exact cgroup leaf the process was born into.
    Cgroup,
    /// The verified executable behind the process.
    Executable,
    /// The owning Provider component template.
    Template,
    /// The resource generation the process was launched for.
    Generation,
    /// The transient unit or scope InvocationID.
    UnitInvocationId,
    /// The service manager's reported main process of the unit or scope.
    UnitMainPid,
}

/// The exact set of identity properties an effect adapter actually verified.
///
/// A Provider compares this against its profile's required set before it
/// requests a pidfd. Anything missing is ambiguity, and ambiguity
/// quarantines rather than adopting, signalling, or reusing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ObservedIdentity {
    verified: BTreeSet<IdentityBinding>,
}

impl ObservedIdentity {
    /// Build an observation from the bindings that were verified.
    pub fn from_verified(verified: impl IntoIterator<Item = IdentityBinding>) -> Self {
        Self {
            verified: verified.into_iter().collect(),
        }
    }

    /// Whether every required binding was verified.
    pub fn covers(&self, required: &BTreeSet<IdentityBinding>) -> bool {
        required.is_subset(&self.verified)
    }

    /// Borrow the verified bindings.
    pub const fn verified(&self) -> &BTreeSet<IdentityBinding> {
        &self.verified
    }
}

/// A digest standing in for one compiled configuration input.
///
/// The compiled sandbox, budget, mount, device, network, and endpoint plans
/// never travel as structured data through a Provider; only their digests
/// do, so no raw policy fragment, host path, or numeric identity can reach
/// Provider code.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigurationDigest([u8; 32]);

/// The opaque stable identity of one launched process.
///
/// It is derived by the effect adapter from immutable identity material.
/// The digest is the only process identity that is ever public status.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcessIdentityDigest([u8; 32]);

macro_rules! opaque_digest {
    ($name:ident, $label:literal) => {
        impl $name {
            /// Wrap 32 opaque digest bytes.
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            /// Render the digest as lowercase hex.
            pub fn to_hex(self) -> String {
                let mut out = String::with_capacity(64);
                for byte in self.0 {
                    out.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
                    out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap_or('0'));
                }
                out
            }

            /// Whether this digest is the forbidden all-zero identity.
            pub fn is_zero(self) -> bool {
                self.0 == [0; 32]
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!($label, "(<redacted>)"))
            }
        }

        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_hex())
            }
        }
    };
}

opaque_digest!(ConfigurationDigest, "ConfigurationDigest");
opaque_digest!(ProcessIdentityDigest, "ProcessIdentityDigest");

/// Proof that the effect adapter opened and verified a pidfd for exactly
/// this process, and that the Provider holds it locally.
///
/// The fd itself never reaches Provider code. This value is deliberately
/// not `Clone`, not `Copy`, not `Default`, not `Serialize`, and carries no
/// accessor: it is not persisted, is not public status, and never crosses
/// d2b-bus, a Zone boundary, or a Host or Guest transport. It is dropped
/// and re-derived across a controller restart, after identity is verified
/// again.
pub struct PidfdEvidence {
    _private: (),
}

impl PidfdEvidence {
    /// Record that a verified pidfd is held for this process.
    ///
    /// Only an effect adapter calls this, immediately after it verified the
    /// process identity and opened the descriptor it retains.
    pub const fn held() -> Self {
        Self { _private: () }
    }
}

impl fmt::Debug for PidfdEvidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PidfdEvidence(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digests_render_hex_but_redact_their_debug() {
        let digest = ProcessIdentityDigest::from_bytes([0xab; 32]);
        assert_eq!(digest.to_hex(), "ab".repeat(32));
        assert_eq!(format!("{digest:?}"), "ProcessIdentityDigest(<redacted>)");
        assert_eq!(
            format!("{:?}", ConfigurationDigest::from_bytes([0; 32])),
            "ConfigurationDigest(<redacted>)"
        );
    }

    #[test]
    fn pidfd_evidence_is_opaque_in_diagnostics() {
        assert_eq!(
            format!("{:?}", PidfdEvidence::held()),
            "PidfdEvidence(<redacted>)"
        );
    }

    #[test]
    fn observed_identity_covers_only_a_verified_superset() {
        let observed = ObservedIdentity::from_verified([
            IdentityBinding::Pid,
            IdentityBinding::ProcessStartTime,
            IdentityBinding::Cgroup,
        ]);
        let required = BTreeSet::from([IdentityBinding::Pid, IdentityBinding::Cgroup]);
        assert!(observed.covers(&required));
        let stricter = BTreeSet::from([IdentityBinding::Pid, IdentityBinding::Executable]);
        assert!(!observed.covers(&stricter));
    }
}
