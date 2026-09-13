#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;

use d2b_contracts::types::VmId;
use d2b_contracts_broker::broker_wire::{
    BrokerRequest, BrokerResponse, OpenHidrawSecurityKeyRequest,
};
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
use d2b_provider_device_security_key::{
    PhysicalAuthorityLease, PhysicalUsbBackingClaim, PhysicalUsbBackingToken, RelayLaunchTicket,
    SecurityKeyEffectError, SecurityKeyEffectPort, SecurityKeyOpenIntent,
};
use sha2::{Digest, Sha256};

use crate::ServerState;

struct RelayTarget {
    socket_path: PathBuf,
    uid: u32,
    gid: u32,
}

struct LiveSecurityKeyEffectPort<'a> {
    state: &'a ServerState,
    vm_id: VmId,
    selector_id: String,
    device_ref: ResourceRef,
    zone_ref: ResourceRef,
    device_uid: ResourceUid,
    holder_ref: ResourceRef,
    target: RelayTarget,
    caller_role: d2b_contracts_broker::broker_wire::BrokerCallerRole,
    claimed_backing: Option<PhysicalUsbBackingToken>,
}

impl SecurityKeyEffectPort for LiveSecurityKeyEffectPort<'_> {
    fn claim_physical_backing(
        &mut self,
        claim: &PhysicalUsbBackingClaim,
    ) -> Result<PhysicalAuthorityLease, SecurityKeyEffectError> {
        if claim.device_uid() != Some(&self.device_uid)
            || claim.zone_ref() != Some(&self.zone_ref)
            || claim.holder_ref() != Some(&self.holder_ref)
        {
            return Err(SecurityKeyEffectError::AuthorizationDenied);
        }
        let backing = claim.token().clone();
        if !self
            .state
            .security_key_sessions
            .lock()
            .claim_backing(self.vm_id.as_str(), backing.clone())
        {
            return Err(SecurityKeyEffectError::PhysicalUsbBackingConflict);
        }
        self.claimed_backing = Some(backing);
        Ok(PhysicalAuthorityLease::from_core(ticket_bytes(
            b"d2b:security-key-lease/v1",
            self.selector_id.as_bytes(),
        )))
    }

    fn open_hidraw(
        &mut self,
        intent: &SecurityKeyOpenIntent,
    ) -> Result<RelayLaunchTicket, SecurityKeyEffectError> {
        if intent.device_uid() != &self.device_uid
            || intent.backing().device_uid() != Some(&self.device_uid)
            || intent.backing().zone_ref() != Some(&self.zone_ref)
            || intent.backing().holder_ref() != Some(&self.holder_ref)
        {
            return Err(SecurityKeyEffectError::AuthorizationDenied);
        }
        let (response, fds) = crate::dispatch_broker_request_with_fds_timeout_as(
            self.state,
            BrokerRequest::OpenHidrawSecurityKey(OpenHidrawSecurityKeyRequest {
                vm_id: self.vm_id.clone(),
                selector_id: self.selector_id.clone(),
                device_ref: self.device_ref.clone(),
                authority_key: d2b_contracts_broker::broker_wire::security_key_authority_binding(
                    &self.device_ref,
                    &self.selector_id,
                ),
                tracing_span_id: None,
            }),
            self.caller_role.clone(),
            std::time::Duration::from_secs(30),
        )
        .map_err(|_| SecurityKeyEffectError::BrokerInaccessible)?;
        if !matches!(response, BrokerResponse::OpenHidrawSecurityKey(_)) || fds.len() != 1 {
            crate::close_received_fds(&fds);
            return Err(SecurityKeyEffectError::EffectRejected);
        }
        let fd = match crate::duplicate_received_fd(&fds, 0, "security-key hidraw") {
            Ok(fd) => fd,
            Err(_) => {
                crate::close_received_fds(&fds);
                return Err(SecurityKeyEffectError::EffectRejected);
            }
        };
        crate::close_received_fds(&fds);
        let listener = d2b_provider_device_security_key::bind_accept_socket(&self.target.socket_path)
            .map_err(|_| SecurityKeyEffectError::Transient)?;
        let mut relay_state = d2b_provider_device_security_key::SecurityKeyState::new(self.selector_id.clone());
        relay_state.enable_vm(self.vm_id.as_str());
        let state = Arc::new(parking_lot::Mutex::new(relay_state));
        let abort = d2b_provider_device_security_key::spawn_accept_loop(
            listener,
            self.vm_id.as_str().to_owned(),
            self.target.uid,
            self.target.gid,
            Arc::clone(&state),
            d2b_provider_device_security_key::HidrawDevice::from_owned_fd(fd),
        )
        .map_err(|_| SecurityKeyEffectError::Transient)?;
        self.state.security_key_sessions.lock().register(
            self.vm_id.as_str().to_owned(),
            d2b_provider_device_security_key::SkAcceptHandle { state, abort },
        );
        Ok(RelayLaunchTicket::from_core(ticket_bytes(
            b"d2b:security-key-relay/v1",
            intent.device_uid().as_str().as_bytes(),
        )))
    }

    fn release_physical_backing(
        &mut self,
        _lease: PhysicalAuthorityLease,
    ) -> Result<(), SecurityKeyEffectError> {
        if let Some(backing) = self.claimed_backing.take() {
            self.state
                .security_key_sessions
                .lock()
                .release_backing(self.vm_id.as_str(), &backing);
        }
        Ok(())
    }
}

fn ticket_bytes(domain: &[u8], value: &[u8]) -> [u8; 16] {
    let digest = Sha256::new()
        .chain_update(domain)
        .chain_update([0])
        .chain_update(value)
        .finalize();
    let mut out = [0; 16];
    out.copy_from_slice(&digest[..16]);
    out
}
