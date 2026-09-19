//! Core-owned production adapter for `Provider/network-local`.
//!
//! The provider receives no broker socket or raw host intent. This module
//! resolves the provider's opaque context into the U12 broker-generic
//! network kernels (the privileged cores the retired typed network-family
//! wire arms served) and maps only closed broker outcomes back to the
//! provider.
//!
//! The daemon drives the network effects directly: each method resolves the
//! trusted bundle intents the retired arms resolved broker-side and invokes
//! the matching kernel as a direct envelope call over the origination
//! socket (the U10 legacy-leg pattern), so the broker never grows family
//! code and the daemon never re-implements a privileged host effect.

use std::time::Duration;

use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_contracts_broker::kernel_client::{
    KernelInvocation, KernelInvokeError, envelope_invoke_kernel,
};
use d2b_provider_network_local::{
    KERNEL_APPLY_NFTABLES_PROJECTION, KERNEL_SEED_DNSMASQ_LEASE,
    broker::{
        BrokerNetworkEffectPort, FirewallProjectionAction, NetworkBroker, NetworkBrokerError,
        NetworkEffectContext,
    },
    controller::FirewallDigest,
};

use crate::ServerState;

/// The broker kernel IO budget one direct invocation may take.
const KERNEL_IO_TIMEOUT: Duration = Duration::from_secs(10);

/// A Core adapter that sends one kernel invocation through the
/// authenticated daemon-to-broker transport.
pub(crate) struct DaemonNetworkBroker<'a> {
    state: &'a ServerState,
    caller_role: BrokerCallerRole,
}

impl<'a> DaemonNetworkBroker<'a> {
    /// Bind the adapter to the current daemon request and caller role.
    pub(crate) const fn new(state: &'a ServerState, caller_role: BrokerCallerRole) -> Self {
        Self { state, caller_role }
    }

    /// Invoke one broker-generic network kernel over the origination
    /// socket. The payload is the resolved values the retired typed arm
    /// derived broker-side; the kernel runs the same ops-module core.
    fn invoke_kernel(
        &self,
        operation: &str,
        zone: &str,
        payload: serde_json::Value,
    ) -> Result<(), NetworkBrokerError> {
        match envelope_invoke_kernel(
            &crate::broker_socket_path(self.state),
            KERNEL_IO_TIMEOUT,
            self.caller_role.clone(),
            KernelInvocation {
                operation,
                zone,
                payload,
                fds: &[],
                chain_root_invocation_id: None,
                chain_identities: None,
            },
        ) {
            Ok(_) => Ok(()),
            Err(KernelInvokeError::Refused { code, detail }) => {
                let message = detail.unwrap_or_default();
                tracing::warn!(
                    broker_kind = %code,
                    broker_operation = operation,
                    "Network broker refused a kernel effect request"
                );
                Err(map_broker_error(&code, &message))
            }
            Err(error) => {
                tracing::warn!(
                    broker_operation = operation,
                    error = %error,
                    "Network broker kernel invocation failed"
                );
                Err(NetworkBrokerError::Transport)
            }
        }
    }

    /// The Zone one network effect runs in: the admitted Network identity's
    /// Zone uid, the same scope the retired arms' audit join keyed on.
    fn zone_for(&self, context: &NetworkEffectContext) -> Result<String, NetworkBrokerError> {
        Ok(context.provenance()?.zone_uid().as_str().to_owned())
    }
}

/// The production provider effect-port type.
pub(crate) type DaemonNetworkEffectPort<'a> = BrokerNetworkEffectPort<DaemonNetworkBroker<'a>>;

/// Construct a production Network effect port for one Core-resolved context.
pub(crate) fn production_port<'a>(
    state: &'a ServerState,
    caller_role: BrokerCallerRole,
    context: NetworkEffectContext,
) -> DaemonNetworkEffectPort<'a> {
    BrokerNetworkEffectPort::new(DaemonNetworkBroker::new(state, caller_role), context)
}

impl NetworkBroker for DaemonNetworkBroker<'_> {
    fn create_bridge(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        let provenance = context.provenance()?;
        let zone = self.zone_for(context)?;
        let resolver = crate::load_bundle_resolver(self.state).map_err(|_| {
            NetworkBrokerError::NetworkAdmissionMismatch
        })?;
        for intent_ref in context.bridge_intent_refs() {
            let intent = resolver
                .resolve_network_bridge_intent(intent_ref.as_str(), &provenance)
                .ok_or(NetworkBrokerError::NetworkAdmissionMismatch)?;
            self.invoke_kernel(
                "create-bridge",
                &zone,
                resolved_bridge_payload(&intent),
            )?;
        }
        Ok(())
    }

    fn delete_bridge(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        let provenance = context.provenance()?;
        let zone = self.zone_for(context)?;
        let resolver = crate::load_bundle_resolver(self.state).map_err(|_| {
            NetworkBrokerError::NetworkAdmissionMismatch
        })?;
        for intent_ref in context.bridge_intent_refs() {
            let intent = resolver
                .resolve_network_bridge_intent(intent_ref.as_str(), &provenance)
                .ok_or(NetworkBrokerError::NetworkAdmissionMismatch)?;
            self.invoke_kernel(
                "delete-bridge",
                &zone,
                resolved_bridge_payload(&intent),
            )?;
        }
        Ok(())
    }

    fn apply_projection(
        &self,
        context: &NetworkEffectContext,
        action: FirewallProjectionAction,
    ) -> Result<FirewallDigest, NetworkBrokerError> {
        let provenance = context.provenance()?;
        let zone = self.zone_for(context)?;
        let resolver = crate::load_bundle_resolver(self.state).map_err(|_| {
            NetworkBrokerError::NetworkAdmissionMismatch
        })?;
        let intent = resolver
            .resolve_network_projection_intent(
                context.projection_intent_ref().as_str(),
                &provenance,
            )
            .ok_or(NetworkBrokerError::NetworkAdmissionMismatch)?;
        let marker = resolver
            .resolve_network_marker_intent(&intent.ownership_marker_intent_ref, &provenance)
            .ok_or(NetworkBrokerError::NetworkAdmissionMismatch)?;
        let installed = resolver
            .installed_generation_identity()
            .ok_or(NetworkBrokerError::StaleGeneration)?;
        self.invoke_kernel(
            KERNEL_APPLY_NFTABLES_PROJECTION,
            &zone,
            serde_json::json!({
                "scriptBody": intent.script_body,
                "marker": marker.marker,
                "trustedHash": intent.desired_hash,
                "callerHash": serde_json::Value::Null,
                "expectedGenerationId": context.expected_generation_id().as_str(),
                "installedGenerationId": installed.as_str(),
                "action": match action {
                    FirewallProjectionAction::Apply => "apply",
                    FirewallProjectionAction::Remove => "remove",
                },
            }),
        )?;
        Ok(FirewallDigest::new(context.projection_digest()))
    }

    fn apply_nm_unmanaged(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        let zone = context
            .scope_id()
            .as_str()
            .to_owned();
        let resolver = crate::load_bundle_resolver(self.state).map_err(|_| {
            NetworkBrokerError::NetworkAdmissionMismatch
        })?;
        let intent = resolver
            .find_nm_unmanaged_intent(context.nm_intent_ref().as_str())
            .ok_or(NetworkBrokerError::NetworkAdmissionMismatch)?
            .clone();
        self.invoke_kernel(
            "apply-nm-unmanaged",
            &zone,
            serde_json::json!({
                "intentId": intent.intent_id,
                "filePath": intent.file_path.display().to_string(),
                "contents": intent.contents,
                "mode": intent.mode,
                "owner": intent.owner,
                "group": intent.group,
                "reloadBehavior": intent.reload_behavior,
                "destroy": false,
            }),
        )
    }

    fn apply_routes(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        let provenance = context.provenance()?;
        let zone = self.zone_for(context)?;
        let resolver = crate::load_bundle_resolver(self.state).map_err(|_| {
            NetworkBrokerError::NetworkAdmissionMismatch
        })?;
        for intent_ref in context.route_intent_refs() {
            let intent = resolver
                .resolve_network_route_intent(intent_ref.as_str(), &provenance)
                .ok_or(NetworkBrokerError::NetworkAdmissionMismatch)?;
            self.invoke_kernel(
                "apply-route",
                &zone,
                resolved_route_payload(&intent, &provenance, false),
            )?;
        }
        Ok(())
    }

    fn remove_routes(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        let provenance = context.provenance()?;
        let zone = self.zone_for(context)?;
        let resolver = crate::load_bundle_resolver(self.state).map_err(|_| {
            NetworkBrokerError::NetworkAdmissionMismatch
        })?;
        for intent_ref in context.route_intent_refs() {
            let intent = resolver
                .resolve_network_route_intent(intent_ref.as_str(), &provenance)
                .ok_or(NetworkBrokerError::NetworkAdmissionMismatch)?;
            self.invoke_kernel(
                "apply-route",
                &zone,
                resolved_route_payload(&intent, &provenance, true),
            )?;
        }
        Ok(())
    }

    fn apply_sysctls(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        let provenance = context.provenance()?;
        let zone = self.zone_for(context)?;
        let resolver = crate::load_bundle_resolver(self.state).map_err(|_| {
            NetworkBrokerError::NetworkAdmissionMismatch
        })?;
        for intent_ref in context.sysctl_intent_refs() {
            let intent = resolver
                .resolve_network_sysctl_intent(intent_ref.as_str(), &provenance)
                .ok_or(NetworkBrokerError::NetworkAdmissionMismatch)?;
            self.invoke_kernel(
                "apply-sysctl",
                &zone,
                serde_json::json!({
                    "key": intent.key,
                    "value": intent.value,
                    "destroy": false,
                }),
            )?;
        }
        Ok(())
    }

    fn update_hosts(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        let resolver = crate::load_bundle_resolver(self.state).map_err(|_| {
            NetworkBrokerError::NetworkAdmissionMismatch
        })?;
        let zone = context
            .scope_id()
            .as_str()
            .to_owned();
        let (intent, provenance) = if context
            .hosts_intent_ref()
            .as_str()
            .starts_with("network-hosts:")
        {
            let provenance = context.provenance()?;
            let intent = resolver
                .resolve_network_hosts_intent(context.hosts_intent_ref().as_str(), &provenance)
                .ok_or(NetworkBrokerError::NetworkAdmissionMismatch)?;
            (intent, Some(provenance))
        } else {
            let intent = resolver
                .find_hosts_intent(context.hosts_intent_ref().as_str())
                .ok_or(NetworkBrokerError::NetworkAdmissionMismatch)?
                .clone();
            (intent, None)
        };
        self.invoke_kernel(
            "update-hosts-file",
            &zone,
            serde_json::json!({
                "intentId": intent.intent_id,
                "path": intent.path.display().to_string(),
                "managedBlock": intent.managed_block,
                "startMarker": intent.start_marker,
                "endMarker": intent.end_marker,
                "mode": intent.mode,
                "provenance": provenance.as_ref().map(serde_json::to_value).transpose().map_err(|_| NetworkBrokerError::Rejected)?,
                "ownershipMarker": intent.ownership_marker,
                "destroy": false,
            }),
        )
    }

    fn seed_dhcp(&self, context: &NetworkEffectContext) -> Result<(), NetworkBrokerError> {
        let provenance = context.provenance()?;
        let zone = self.zone_for(context)?;
        self.invoke_kernel(
            KERNEL_SEED_DNSMASQ_LEASE,
            &zone,
            serde_json::json!({
                "vmId": context.dhcp_vm_id().as_str(),
                "scopeId": context.scope_id().as_str(),
                "zoneUid": provenance.zone_uid().as_str(),
                "networkUid": provenance.network_uid().as_str(),
                "networkGeneration": provenance.network_generation().get(),
                "attachmentGeneration": provenance.attachment_generation().get(),
                "bundleGeneration": provenance.bundle_generation().as_str(),
            }),
        )
    }

    fn delete_persistent_tap(
        &self,
        context: &NetworkEffectContext,
        handle: &d2b_contracts_resource::v3::network::AttachmentHandle,
        fence: &d2b_contracts_resource::v3::network::AttachmentGenerationFence,
    ) -> Result<(), NetworkBrokerError> {
        let proof = context
            .network_admission()
            .ok_or(NetworkBrokerError::NetworkAdmissionRequired)?;
        if handle.opaque_id() != fence.attachment_uid() {
            return Err(NetworkBrokerError::NetworkAdmissionMismatch);
        }
        let zone = self.zone_for(context)?;
        self.invoke_kernel(
            "delete-persistent-tap",
            &zone,
            serde_json::json!({
                "attachmentId": handle.opaque_id().as_str(),
                "expectedZoneUid": proof.key().zone_uid().as_str(),
                "expectedNetworkUid": proof.key().network_uid().as_str(),
                "expectedNetworkGeneration": fence.network_generation().get(),
                "expectedAttachmentGeneration": fence.attachment_generation().get(),
                "expectedBundleGeneration": proof.key().bundle_generation().as_str(),
            }),
        )
    }
}

/// The resolved bridge intent payload one bridge kernel invocation carries.
fn resolved_bridge_payload(
    intent: &d2b_core::bundle_resolver::ResolvedBridgeIntent,
) -> serde_json::Value {
    serde_json::json!({
        "intentId": intent.intent_id,
        "scopeLabel": intent.scope_label,
        "bridgeIfname": intent.bridge_ifname.as_str(),
        "mtu": intent.mtu,
        "stpDisabled": intent.stp_disabled,
        "multicastSnoopingDisabled": intent.multicast_snooping_disabled,
        "ipv6Suppressed": intent.ipv6_suppressed,
        "provenance": intent.provenance.as_ref().map(serde_json::to_value).transpose().ok().flatten(),
        "ownershipMarker": intent.ownership_marker,
    })
}

/// The resolved route intent payload one apply-route kernel invocation
/// carries.
fn resolved_route_payload(
    intent: &d2b_core::bundle_resolver::ResolvedRouteIntent,
    provenance: &d2b_contracts_resource::v3::NetworkProvenance,
    destroy: bool,
) -> serde_json::Value {
    serde_json::json!({
        "intentId": intent.intent_id,
        "routeSpec": intent.route_spec,
        "destination": intent.destination,
        "via": intent.via,
        "device": intent.device,
        "table": intent.table,
        "owned": intent.owned,
        "routeName": intent.route_name,
        "provenance": serde_json::to_value(provenance).ok(),
        "ownershipMarker": intent.ownership_marker,
        "destroy": destroy,
    })
}

fn map_broker_error(kind: &str, message: &str) -> NetworkBrokerError {
    if message.contains("nm-managed-foreign-conflict")
        || message.contains("foreign route")
        || message.contains("foreign-bridge-ownership-marker")
        || message.contains("foreign-tap-ownership-marker")
        || message.contains("foreign-nft-ownership")
        || message.contains("foreign ownership marker")
    {
        return NetworkBrokerError::ForeignOwnership;
    }
    let reason = message
        .split_once("failed: ")
        .map_or(message, |(_, reason)| reason);
    if reason.contains("stale-bundle-generation") {
        return NetworkBrokerError::StaleGeneration;
    }
    if reason.contains("network-zone-unknown") {
        return NetworkBrokerError::NetworkAdmissionMismatch;
    }
    match (kind, reason) {
        ("Broker.NftablesDriftDetected", _)
        | ("Broker.StaleProjectionGeneration", _)
        | ("Broker.RequestValidation", "stale-projection-generation")
        | ("Broker.RequestValidation", "stale-network-generation") => {
            NetworkBrokerError::StaleGeneration
        }
        ("Broker.ForeignOwnership", _)
        | ("Broker.RequestValidation", "foreign-nft-rule-preserved")
        | ("Broker.RequestValidation", "nm-managed-foreign-conflict")
        | ("Broker.RequestValidation", "attachment-ownership-conflict") => {
            NetworkBrokerError::ForeignOwnership
        }
        ("Broker.RequestValidation", "stale-attachment-generation") => {
            NetworkBrokerError::StaleAttachmentGeneration
        }
        ("Broker.RequestValidation", "network-admission-required") => {
            NetworkBrokerError::NetworkAdmissionRequired
        }
        ("Broker.RequestValidation", "network-admission-mismatch")
        | ("Broker.RequestValidation", "network-scope-invalid")
        | ("Broker.RequestValidation", "network-scope-required")
        | ("Broker.RequestValidation", "network-scope-mismatch") => {
            NetworkBrokerError::NetworkAdmissionMismatch
        }
        ("Broker.RequestValidation", "network-interface-collision") => {
            NetworkBrokerError::NetworkInterfaceCollision
        }
        ("Broker.RequestValidation", "network-route-collision") => {
            NetworkBrokerError::NetworkRouteCollision
        }
        ("Broker.RequestValidation", "network-admission-conflict")
        | ("Broker.RequestValidation", "legacy-network-authority") => {
            NetworkBrokerError::NetworkAdmissionConflict
        }
        ("Broker.RequestValidation", "attachment-delete-failed")
        | ("Broker.RequestValidation", "network-broker-transient") => NetworkBrokerError::Transient,
        ("Broker.Transient", _) | (_, "network-effect-transient") => NetworkBrokerError::Transient,
        (_, "east-west-host-opt-in-required") => NetworkBrokerError::EastWestHostOptInRequired,
        _ => NetworkBrokerError::Rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_validation_reasons_keep_provider_retry_and_block_states() {
        assert_eq!(
            map_broker_error(
                "Broker.RequestValidation",
                "broker request validation failed: stale-projection-generation",
            ),
            NetworkBrokerError::StaleGeneration
        );
        assert_eq!(
            map_broker_error(
                "Broker.RequestValidation",
                "broker request validation failed: stale-attachment-generation",
            ),
            NetworkBrokerError::StaleAttachmentGeneration
        );
        assert_eq!(
            map_broker_error(
                "Broker.RequestValidation",
                "broker request validation failed: attachment-ownership-conflict",
            ),
            NetworkBrokerError::ForeignOwnership
        );
        assert_eq!(
            map_broker_error(
                "Broker.LiveHandler",
                "broker live handler failed: nm-managed-foreign-conflict",
            ),
            NetworkBrokerError::ForeignOwnership
        );
        assert_eq!(
            map_broker_error(
                "Broker.RequestValidation",
                "broker request validation failed: east-west-host-opt-in-required",
            ),
            NetworkBrokerError::EastWestHostOptInRequired
        );
    }
}