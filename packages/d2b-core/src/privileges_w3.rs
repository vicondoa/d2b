//! W3 broker operation matrix extensions.
//!
//! Wire-stable contract authored by the W3 integrator API/contract prep
//! commit. The existing W1/W2 [`super::privileges::OperationAuthzRow`]
//! table covers every W2 broker variant; this module adds rows for
//! genuinely new W3 broker variants (currently `UsbipBindFirewallRule`)
//! plus the [`W3BrokerOperation`] enum and the [`W3OperationFlags`]
//! helper that audits each row's `audit`/`destructive`/`secret_access`
//! bits per plan.md "W3 broker variant additions".
//!
//! Pre-existing W2 broker variants (`DelegateCgroupV2`, `OpenCgroupDir`,
//! `OpenKvm`, `OpenVhostNet`, `OpenFuse`, `OpenDevice`, `CreateTapFd`,
//! `CreatePersistentTap`, `SetBridgePortFlags`, `ApplyNftables`,
//! `ApplyRoute`, `ApplySysctl`, `ApplyNmUnmanaged`, `UpdateHostsFile`,
//! `BindUnixSocket`, `SetSocketAcl`, `ModprobeIfAllowed`,
//! `PrepareStateDir`, `PrepareRuntimeDir`) already have rows in
//! [`super::privileges::BROKER_OPERATION_AUTHZ`]. The W3 plan
//! re-anchors them under the W3 audit-field schema; the audit fields
//! themselves are documented in `docs/reference/privileges.md` and are
//! enforced by the broker dispatcher.
//!
//! Spec correction (per AGENTS.md "Existing code is canon"): the W3
//! plan example shows kebab-case wire discriminants (e.g.
//! `delegate-cgroup-v2`); the existing W2 broker enum uses PascalCase
//! variants (e.g. `DelegateCgroupV2`). W2 panel signoff froze the
//! PascalCase convention. This module preserves the existing wire
//! convention.

use serde::{Deserialize, Serialize};

/// Closed enum of every W3 broker operation, used by the wire-skew
/// gate (plan.md §"W3 wire-compat / version-skew gate") to enumerate
/// `Capabilities::broker_operations` without re-typing strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum W3BrokerOperation {
    DelegateCgroupV2,
    OpenCgroupDir,
    PrepareStateDir,
    PrepareRuntimeDir,
    OpenKvm,
    OpenVhostNet,
    OpenFuse,
    OpenDevice,
    CreateTapFd,
    CreatePersistentTap,
    SetBridgePortFlags,
    ApplyNftables,
    ApplyRoute,
    ApplySysctl,
    ApplyNmUnmanaged,
    UpdateHostsFile,
    BindUnixSocket,
    SetSocketAcl,
    GuestControlSign,
    ModprobeIfAllowed,
    UsbipBindFirewallRule,
}

impl W3BrokerOperation {
    /// Returns the on-wire enum tag (matching the
    /// [`d2b_contracts::broker_wire::BrokerRequest`] discriminant) for
    /// this operation.
    pub const fn wire_tag(self) -> &'static str {
        match self {
            Self::DelegateCgroupV2 => "DelegateCgroupV2",
            Self::OpenCgroupDir => "OpenCgroupDir",
            Self::PrepareStateDir => "PrepareStateDir",
            Self::PrepareRuntimeDir => "PrepareRuntimeDir",
            Self::OpenKvm => "OpenKvm",
            Self::OpenVhostNet => "OpenVhostNet",
            Self::OpenFuse => "OpenFuse",
            Self::OpenDevice => "OpenDevice",
            Self::CreateTapFd => "CreateTapFd",
            Self::CreatePersistentTap => "CreatePersistentTap",
            Self::SetBridgePortFlags => "SetBridgePortFlags",
            Self::ApplyNftables => "ApplyNftables",
            Self::ApplyRoute => "ApplyRoute",
            Self::ApplySysctl => "ApplySysctl",
            Self::ApplyNmUnmanaged => "ApplyNmUnmanaged",
            Self::UpdateHostsFile => "UpdateHostsFile",
            Self::BindUnixSocket => "BindUnixSocket",
            Self::SetSocketAcl => "SetSocketAcl",
            Self::GuestControlSign => "GuestControlSign",
            Self::ModprobeIfAllowed => "ModprobeIfAllowed",
            Self::UsbipBindFirewallRule => "UsbipBindFirewallRule",
        }
    }

    /// Returns every W3 broker operation in stable order. Consumed by
    /// the `Capabilities::broker_operations` advertisement in
    /// `d2b-contracts` and by the broker-enum-disposition gate.
    pub const fn all() -> &'static [W3BrokerOperation] {
        &[
            Self::DelegateCgroupV2,
            Self::OpenCgroupDir,
            Self::PrepareStateDir,
            Self::PrepareRuntimeDir,
            Self::OpenKvm,
            Self::OpenVhostNet,
            Self::OpenFuse,
            Self::OpenDevice,
            Self::CreateTapFd,
            Self::CreatePersistentTap,
            Self::SetBridgePortFlags,
            Self::ApplyNftables,
            Self::ApplyRoute,
            Self::ApplySysctl,
            Self::ApplyNmUnmanaged,
            Self::UpdateHostsFile,
            Self::BindUnixSocket,
            Self::SetSocketAcl,
            Self::GuestControlSign,
            Self::ModprobeIfAllowed,
            Self::UsbipBindFirewallRule,
        ]
    }

    /// Audit/destructive/secret flags for the row per plan.md "W3
    /// broker variant additions". Consumed by the privileges drift
    /// gate.
    pub const fn flags(self) -> W3OperationFlags {
        match self {
            Self::DelegateCgroupV2 => W3OperationFlags {
                audit: true,
                destructive: false,
                secret_access: false,
            },
            Self::OpenCgroupDir => W3OperationFlags {
                audit: true,
                destructive: false,
                secret_access: false,
            },
            Self::PrepareStateDir | Self::PrepareRuntimeDir => W3OperationFlags {
                audit: true,
                destructive: true,
                secret_access: false,
            },
            Self::OpenKvm | Self::OpenVhostNet | Self::OpenFuse | Self::OpenDevice => {
                W3OperationFlags {
                    audit: true,
                    destructive: false,
                    secret_access: false,
                }
            }
            Self::CreateTapFd
            | Self::CreatePersistentTap
            | Self::SetBridgePortFlags
            | Self::ApplyNftables
            | Self::ApplyRoute
            | Self::ApplySysctl
            | Self::ApplyNmUnmanaged
            | Self::UpdateHostsFile => W3OperationFlags {
                audit: true,
                destructive: true,
                secret_access: false,
            },
            Self::BindUnixSocket | Self::SetSocketAcl => W3OperationFlags {
                audit: true,
                destructive: true,
                secret_access: false,
            },
            Self::GuestControlSign => W3OperationFlags {
                audit: true,
                destructive: false,
                secret_access: true,
            },
            Self::ModprobeIfAllowed => W3OperationFlags {
                audit: true,
                destructive: true,
                secret_access: false,
            },
            Self::UsbipBindFirewallRule => W3OperationFlags {
                audit: true,
                destructive: false,
                secret_access: false,
            },
        }
    }
}

/// Audit/destructive/secret flags from plan.md "W3 broker variant
/// additions". `default_for_unknown` is always `Deny` per the W3
/// fail-closed posture, so it is not stored here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct W3OperationFlags {
    /// Whether a successful operation must emit a broker audit event.
    pub audit: bool,
    /// Whether the operation mutates host state.
    pub destructive: bool,
    /// Whether the operation can read or modify secret material.
    pub secret_access: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_w3_operation_has_audit_set() {
        for op in W3BrokerOperation::all() {
            assert!(op.flags().audit, "W3 operation {op:?} must be audited");
        }
    }

    #[test]
    fn destructive_flags_match_plan_table() {
        assert!(!W3BrokerOperation::DelegateCgroupV2.flags().destructive);
        assert!(W3BrokerOperation::PrepareStateDir.flags().destructive);
        assert!(W3BrokerOperation::ApplyNftables.flags().destructive);
        assert!(!W3BrokerOperation::UsbipBindFirewallRule.flags().destructive);
    }

    #[test]
    fn only_guest_control_sign_grants_secret_access() {
        for op in W3BrokerOperation::all() {
            assert_eq!(
                op.flags().secret_access,
                *op == W3BrokerOperation::GuestControlSign,
                "unexpected secret_access flag for {op:?}"
            );
        }
    }

    #[test]
    fn wire_tags_are_unique_pascalcase() {
        let mut tags: Vec<_> = W3BrokerOperation::all()
            .iter()
            .map(|op| op.wire_tag())
            .collect();
        tags.sort();
        let len_before = tags.len();
        tags.dedup();
        assert_eq!(tags.len(), len_before, "duplicate W3 wire tag");
    }
}
