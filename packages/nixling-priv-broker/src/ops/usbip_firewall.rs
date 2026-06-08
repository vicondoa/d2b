//! `UsbipBindFirewallRule` broker op (W3 s3, skeleton-only).
//!
//! Adds a source-based nft carve-out rule before the broad allow/drop
//! rule in `inet nixling`'s `forward` chain. The ordering invariant is
//! enforced by [`nixling_host::nftables::NftBatch::assert_carveout_ordering`].
//!
//! The full `UsbipBind`/`UsbipUnbind`/`UsbipProxyReconcile` UX
//! (live device routing) is OUT of W3 scope and lives in W6;
//! [`refuse_w6_operation`] is the explicit fail-closed handler used by
//! the broker dispatch table when one of those W6 variants is invoked
//! at W3.

use nixling_host::nftables::{add_usbip_firewall_carveout, BusId, NftBatch, NftError, Sha256};
use serde::{Deserialize, Serialize};

/// Audit-event payload for `UsbipBindFirewallRule`. Combined with the
/// broker common header at write time. `rule_hash` is the
/// canonical-hash of the rendered nft batch *after* the carve-out has
/// been inserted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbipBindFirewallRuleAudit {
    pub busid: String,
    pub rule_hash: Sha256,
}

/// Decision returned by [`bind_firewall_rule`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbipBindFirewallRuleDecision {
    pub batch: NftBatch,
    pub audit: UsbipBindFirewallRuleAudit,
}

/// Insert the per-busid firewall carve-out rule. Returns the typed
/// decision (batch + audit row) for the broker runtime.
pub fn bind_firewall_rule(bus_id: &BusId) -> Result<UsbipBindFirewallRuleDecision, NftError> {
    let batch = add_usbip_firewall_carveout(bus_id)?;
    let rule_hash = batch.canonical_hash();
    Ok(UsbipBindFirewallRuleDecision {
        batch,
        audit: UsbipBindFirewallRuleAudit {
            busid: bus_id.0.clone(),
            rule_hash,
        },
    })
}

/// USBIP operations explicitly OUT of W3 scope. These are refused with
/// the `unknown-operation` kebab-case discriminant + audit per plan.md
/// §"W3 broker variant additions" (`defaultForUnknown: deny`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum W6UsbipOperation {
    UsbipBind,
    UsbipUnbind,
    UsbipProxyReconcile,
}

impl W6UsbipOperation {
    pub const fn as_kebab_case(&self) -> &'static str {
        match self {
            Self::UsbipBind => "usbip-bind",
            Self::UsbipUnbind => "usbip-unbind",
            Self::UsbipProxyReconcile => "usbip-proxy-reconcile",
        }
    }
}

/// Refusal-audit payload emitted when a W6 USBIP UX operation is
/// dispatched at W3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefusedW6Audit {
    pub operation: W6UsbipOperation,
    pub reason: &'static str,
}

/// Fail-closed handler for W6-scoped USBIP variants. Returns the audit
/// payload the broker runtime writes; the wire-level response is
/// `broker-unimplemented` per the W2 broker enum disposition contract.
pub fn refuse_w6_operation(op: W6UsbipOperation) -> RefusedW6Audit {
    RefusedW6Audit {
        operation: op,
        reason: "unknown-operation",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_firewall_rule_produces_audit_with_busid_and_hash() {
        let decision = bind_firewall_rule(&BusId::new("1-1.4")).unwrap();
        assert_eq!(decision.audit.busid, "1-1.4");
        // Rule hash matches the rendered batch canonical hash.
        assert_eq!(decision.audit.rule_hash, decision.batch.canonical_hash());
    }

    #[test]
    fn carveout_ordering_invariant_via_op() {
        let decision = bind_firewall_rule(&BusId::new("2-3.1")).unwrap();
        decision.batch.assert_carveout_ordering().unwrap();
    }

    #[test]
    fn w6_ops_refused_with_unknown_operation_audit() {
        for op in [
            W6UsbipOperation::UsbipBind,
            W6UsbipOperation::UsbipUnbind,
            W6UsbipOperation::UsbipProxyReconcile,
        ] {
            let audit = refuse_w6_operation(op);
            assert_eq!(audit.reason, "unknown-operation");
            assert_eq!(audit.operation, op);
        }
    }
}
