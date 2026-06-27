//! `UsbipBindFirewallRule` broker op.
//!
//! Adds the trusted-bundle USBIP nft carve-out rule before broad
//! allow/drop rules in `inet d2b`. The live path inserts the
//! resolved env TCP/3240 expression into the `input` chain for the
//! host-side proxy listener. The ordering invariant is enforced by
//! [`d2b_host::nftables::NftBatch::assert_carveout_ordering`].
//!
//! The full `UsbipBind`/`UsbipUnbind`/`UsbipProxyReconcile` UX (live
//! device routing) is handled separately; [`refuse_w6_operation`] is the
//! explicit fail-closed handler used by the broker dispatch table when
//! one of those live-routing variants is invoked before support.

use d2b_host::nftables::{BusId, ChainHook, NftBatch, NftError, Sha256};
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
pub fn bind_firewall_rule(
    mut batch: NftBatch,
    bus_id: &BusId,
    rule_expr: &str,
) -> Result<UsbipBindFirewallRuleDecision, NftError> {
    batch.add_usbip_carveout_expr(ChainHook::Input, bus_id, rule_expr)?;
    batch.assert_carveout_ordering()?;
    let rule_hash = batch.canonical_hash();
    Ok(UsbipBindFirewallRuleDecision {
        batch,
        audit: UsbipBindFirewallRuleAudit {
            busid: bus_id.0.clone(),
            rule_hash,
        },
    })
}

/// USBIP operations explicitly outside this firewall-rule skeleton.
/// These are refused with the `unknown-operation` kebab-case
/// discriminant + audit (`defaultForUnknown: deny`).
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

/// Refusal-audit payload emitted when a USBIP UX operation is
/// dispatched before support.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefusedW6Audit {
    pub operation: W6UsbipOperation,
    pub reason: &'static str,
}

/// Fail-closed handler for USBIP live-routing variants. Returns the
/// audit payload the broker runtime writes; the wire-level response is
/// `broker-unimplemented` per the legacy broker enum disposition
/// contract.
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
        let decision = bind_firewall_rule(
            d2b_host::nftables::build_inet_d2b_chains(),
            &BusId::new("1-1.4"),
            "iifname \"br-work-up\" tcp dport 3240 accept",
        )
        .unwrap();
        assert_eq!(decision.audit.busid, "1-1.4");
        // Rule hash matches the rendered batch canonical hash.
        assert_eq!(decision.audit.rule_hash, decision.batch.canonical_hash());
        let script = decision.batch.render_nft_script();
        assert!(script.contains("chain input"));
        assert!(script.contains("iifname \"br-work-up\" tcp dport 3240 accept"));
        assert!(!script.contains("usbip-1-1.4\" accept"));
    }

    #[test]
    fn carveout_ordering_invariant_via_op() {
        let decision = bind_firewall_rule(
            d2b_host::nftables::build_inet_d2b_chains(),
            &BusId::new("2-3.1"),
            "iifname \"br-work-up\" tcp dport 3240 accept",
        )
        .unwrap();
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
