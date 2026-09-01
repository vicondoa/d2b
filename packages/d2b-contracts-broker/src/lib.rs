#![doc = "Canonical private broker IPC wire contracts for d2b."]

pub mod broker_wire;
pub mod host_generation;

pub use broker_wire::BrokerRequest;
pub use d2b_contracts::privileges_w3::W3BrokerOperation;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Broker operation-catalogue protocol version.
pub const PROTOCOL_VERSION: u32 = 6;

/// Broker operation capability snapshot associated with the protocol version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrokerCapabilities {
    pub protocol_version: u32,
    pub broker_operations: Vec<String>,
}

impl BrokerCapabilities {
    pub fn w3() -> Self {
        let mut operations: Vec<String> = W3BrokerOperation::all()
            .iter()
            .map(|op| op.wire_tag().to_owned())
            .collect();
        operations.extend(
            [
                "Hello",
                "ValidateBundle",
                "ExportBrokerAudit",
                "CreateOrReconcileUsersGroups",
                "SetupMountNamespace",
                "PrepareStoreView",
                "LaunchMinijailChild",
                "ReadSecretById",
                "InjectSecretById",
                "RotateSecretById",
                "UsbipBind",
                "UsbipUnbind",
                "UsbipProxyReconcile",
                "PauseBroker",
                "ResumeBroker",
            ]
            .into_iter()
            .map(str::to_owned),
        );
        operations.sort();
        operations.dedup();
        Self {
            protocol_version: PROTOCOL_VERSION,
            broker_operations: operations,
        }
    }
}
