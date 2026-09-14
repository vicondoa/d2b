//! Production activation driver effects: the preserved broker boundary the
//! activation family drives through its port.
//!
//! The driver, its spec decoder, its factory, and its declaration live in
//! `d2b-provider-activation-nixos`. This module is the daemon half: it
//! dispatches the preserved `ApplyHostGenerationHandoff` request - caller
//! role `Lifecycle` on the typed request, admin-uid daemon caller on the
//! dispatch - and reduces the response to the closed result the driver's
//! outcome mapping reads.

use std::sync::Arc;

use d2b_contracts_broker::broker_wire::{
    ApplyHostGenerationHandoffResponse, BrokerCallerRole, BrokerRequest, BrokerResponse,
};
use d2b_contracts_broker::host_generation::{
    ApplyHostGenerationHandoff, HandoffCallerRole, HandoffState, HostGenerationHandoffIntent,
};
use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_activation_nixos::{ActivationDriverEffects, HostHandoffResult};

use crate::{ServerState, dispatch_broker_request_as};

/// Production effects over the preserved broker boundary (old
/// `execute_host_handoff` dispatch).
pub(crate) struct ProductionActivationDriverEffects {
    state: Arc<ServerState>,
}

impl ProductionActivationDriverEffects {
    pub(crate) fn new(state: Arc<ServerState>) -> Self {
        Self { state }
    }
}

impl core::fmt::Debug for ProductionActivationDriverEffects {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ProductionActivationDriverEffects")
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl ActivationDriverEffects for ProductionActivationDriverEffects {
    async fn apply_host_generation_handoff(
        &self,
        target: ResourceRef,
        intent: HostGenerationHandoffIntent,
    ) -> HostHandoffResult {
        let request = BrokerRequest::ApplyHostGenerationHandoff(ApplyHostGenerationHandoff {
            caller_role: HandoffCallerRole::Lifecycle,
            target,
            intent,
        });
        match dispatch_broker_request_as(
            &self.state,
            request,
            BrokerCallerRole::AdminUid {
                uid: self.state.daemon_uid,
            },
        ) {
            Ok(BrokerResponse::ApplyHostGenerationHandoff(response)) => {
                host_handoff_result(&response)
            }
            Ok(BrokerResponse::Error(_)) | Ok(_) | Err(_) => HostHandoffResult::Incomplete,
        }
    }
}

fn host_handoff_result(response: &ApplyHostGenerationHandoffResponse) -> HostHandoffResult {
    match response.state {
        HandoffState::Completed => HostHandoffResult::Completed {
            source_generation: response.source_generation,
            target_generation: response.target_generation,
        },
        HandoffState::Refused => HostHandoffResult::Refused,
        HandoffState::RolledBack => HostHandoffResult::RolledBack,
        _ => HostHandoffResult::Incomplete,
    }
}
