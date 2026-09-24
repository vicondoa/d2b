//! Broker operation audit and mutation flags.
//!
//! The existing [`super::privileges::OperationAuthzRow`] table covers the
//! established broker variants. This module provides the closed
//! [`W3BrokerOperation`] inventory used to enumerate broker operations
//! without re-typing strings. Each row's `audit`, `destructive`, and
//! `secret_access` posture lives in the broker's generated
//! `broker_operation_catalog` rows.
//!
//! Established broker variants (`DelegateCgroupV2`, `OpenCgroupDir`,
//! `OpenKvm`, `OpenVhostNet`, `OpenFuse`, `OpenDevice`, `CreateTapFd`,
//! `CreatePersistentTap`, `SetBridgePortFlags`, `ApplyNftables`,
//! `ApplyRoute`, `ApplySysctl`, `ApplyNmUnmanaged`, `UpdateHostsFile`,
//! `ModprobeIfAllowed`, `PrepareStateDir`, `PrepareRuntimeDir`) already have
//! rows in
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
    ModprobeIfAllowed,
    UsbipBindFirewallRule,
    MigrateLegacySwtpmState,
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
            Self::ModprobeIfAllowed => "ModprobeIfAllowed",
            Self::UsbipBindFirewallRule => "UsbipBindFirewallRule",
            Self::MigrateLegacySwtpmState => "MigrateLegacySwtpmState",
            Self::SecurityKeyOpenDevice => "SecurityKeyOpenDevice",
            Self::SecurityKeyApplyUdevRules => "SecurityKeyApplyUdevRules",
        }
    }

    /// Returns every broker operation in stable order. Consumed by
    /// the `Capabilities::broker_operations` advertisement in
    /// `d2b-contracts` and by the broker-enum-disposition gate.
    pub const fn all() -> &'static [W3BrokerOperation] {
        include!("generated/w3_broker_operations.rs")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
