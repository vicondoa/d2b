//! Broker operation audit and mutation flags.
//!
//! The existing [`super::privileges::OperationAuthzRow`] table covers the
//! established broker variants. This module provides the closed
//! [`W3BrokerOperation`] inventory and [`W3OperationFlags`] helper used to
//! audit each row's `audit`, `destructive`, and `secret_access` posture.
//!
//! Established broker variants (`DelegateCgroupV2`, `OpenCgroupDir`,
//! `OpenKvm`, `OpenVhostNet`, `OpenFuse`, `OpenDevice`, `CreateTapFd`,
//! `CreatePersistentTap`, `SetBridgePortFlags`, `ApplyNftables`,
//! `ApplyRoute`, `ApplySysctl`, `ApplyNmUnmanaged`, `UpdateHostsFile`,
//! `BindUnixSocket`, `SetSocketAcl`, `ModprobeIfAllowed`,
//! `PrepareStateDir`, `PrepareRuntimeDir`) already have rows in
//! [`super::privileges::BROKER_OPERATION_AUTHZ`]. Their audit fields are
//! documented in `docs/reference/privileges.md` and enforced by the broker
//! dispatcher.
//!
//! Wire discriminants retain the established PascalCase convention, such as
//! `DelegateCgroupV2`.

use serde::{Deserialize, Serialize};

/// Closed broker operation inventory used to enumerate
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
    DeletePersistentTap,
    CreateBridge,
    DeleteBridge,
    SetBridgePortFlags,
    ApplyNftables,
    ApplyNftablesProjection,
    ApplyRoute,
    ApplySysctl,
    ApplyNmUnmanaged,
    UpdateHostsFile,
    BindUnixSocket,
    SetSocketAcl,
    ModprobeIfAllowed,
    UsbipBindFirewallRule,
    MigrateLegacySwtpmState,
    LaunchCutoverRunner,
    CutoverAudit,
    CutoverEffect,
    /// Open the FIDO/CTAP hidraw node for the broker-configured device
    /// selector. Typed stub until the live host-broker handler is implemented.
    SecurityKeyOpenDevice,
    /// Apply udev group grants for configured FIDO hidraw nodes.
    /// Typed stub until the live host-broker handler is implemented.
    SecurityKeyApplyUdevRules,
}

impl W3BrokerOperation {
    /// Returns the on-wire enum tag (matching the
    /// `d2b_contracts_broker::broker_wire::BrokerRequest` discriminant for
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
            Self::DeletePersistentTap => "DeletePersistentTap",
            Self::CreateBridge => "CreateBridge",
            Self::DeleteBridge => "DeleteBridge",
            Self::SetBridgePortFlags => "SetBridgePortFlags",
            Self::ApplyNftables => "ApplyNftables",
            Self::ApplyNftablesProjection => "ApplyNftablesProjection",
            Self::ApplyRoute => "ApplyRoute",
            Self::ApplySysctl => "ApplySysctl",
            Self::ApplyNmUnmanaged => "ApplyNmUnmanaged",
            Self::UpdateHostsFile => "UpdateHostsFile",
            Self::BindUnixSocket => "BindUnixSocket",
            Self::SetSocketAcl => "SetSocketAcl",
            Self::ModprobeIfAllowed => "ModprobeIfAllowed",
            Self::UsbipBindFirewallRule => "UsbipBindFirewallRule",
            Self::MigrateLegacySwtpmState => "MigrateLegacySwtpmState",
            Self::LaunchCutoverRunner => "LaunchCutoverRunner",
            Self::CutoverAudit => "CutoverAudit",
            Self::CutoverEffect => "CutoverEffect",
            Self::SecurityKeyOpenDevice => "SecurityKeyOpenDevice",
            Self::SecurityKeyApplyUdevRules => "SecurityKeyApplyUdevRules",
        }
    }

    /// Returns every broker operation in stable order. Consumed by
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
            Self::DeletePersistentTap,
            Self::CreateBridge,
            Self::DeleteBridge,
            Self::SetBridgePortFlags,
            Self::ApplyNftables,
            Self::ApplyNftablesProjection,
            Self::ApplyRoute,
            Self::ApplySysctl,
            Self::ApplyNmUnmanaged,
            Self::UpdateHostsFile,
            Self::BindUnixSocket,
            Self::SetSocketAcl,
            Self::ModprobeIfAllowed,
            Self::UsbipBindFirewallRule,
            Self::MigrateLegacySwtpmState,
            Self::LaunchCutoverRunner,
            Self::CutoverAudit,
            Self::CutoverEffect,
            Self::SecurityKeyOpenDevice,
            Self::SecurityKeyApplyUdevRules,
        ]
    }

    /// Returns the audit, mutation, and secret-access posture for the row.
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
            | Self::DeletePersistentTap
            | Self::CreateBridge
            | Self::DeleteBridge
            | Self::SetBridgePortFlags
            | Self::ApplyNftables
            | Self::ApplyNftablesProjection
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
            Self::MigrateLegacySwtpmState => W3OperationFlags {
                audit: true,
                destructive: true,
                secret_access: false,
            },
            Self::LaunchCutoverRunner | Self::CutoverEffect => W3OperationFlags {
                audit: true,
                destructive: true,
                secret_access: false,
            },
            Self::CutoverAudit => W3OperationFlags {
                audit: true,
                destructive: false,
                secret_access: false,
            },
            // SecurityKeyOpenDevice: opens a single FIDO hidraw fd; read-only
            // from the broker's perspective (no state mutation, no secret data).
            Self::SecurityKeyOpenDevice => W3OperationFlags {
                audit: true,
                destructive: false,
                secret_access: false,
            },
            // SecurityKeyApplyUdevRules: writes udev rules (host mutation).
            Self::SecurityKeyApplyUdevRules => W3OperationFlags {
                audit: true,
                destructive: true,
                secret_access: false,
            },
        }
    }
}

/// Audit, mutation, and secret-access flags for one broker operation.
/// `default_for_unknown` is always `Deny`, so it is not stored here.
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
    fn every_operation_has_audit_set() {
        for op in W3BrokerOperation::all() {
            assert!(op.flags().audit, "broker operation {op:?} must be audited");
        }
    }

    #[test]
    fn destructive_flags_match_contract_table() {
        assert!(!W3BrokerOperation::DelegateCgroupV2.flags().destructive);
        assert!(W3BrokerOperation::PrepareStateDir.flags().destructive);
        assert!(W3BrokerOperation::ApplyNftables.flags().destructive);
        assert!(
            W3BrokerOperation::ApplyNftablesProjection
                .flags()
                .destructive
        );
        assert!(W3BrokerOperation::CreateBridge.flags().destructive);
        assert!(W3BrokerOperation::DeleteBridge.flags().destructive);
        assert!(W3BrokerOperation::DeletePersistentTap.flags().destructive);
        assert!(!W3BrokerOperation::UsbipBindFirewallRule.flags().destructive);
        assert!(W3BrokerOperation::LaunchCutoverRunner.flags().destructive);
        assert!(!W3BrokerOperation::CutoverAudit.flags().destructive);
        assert!(W3BrokerOperation::CutoverEffect.flags().destructive);
    }

    #[test]
    fn no_remaining_operation_grants_secret_access() {
        for op in W3BrokerOperation::all() {
            assert_eq!(
                op.flags().secret_access,
                false,
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
