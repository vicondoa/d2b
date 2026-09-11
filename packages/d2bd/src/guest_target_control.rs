//! Host-side target-control channel over the authenticated Guest
//! ComponentSession (U13, spec section 23.3).
//!
//! One frame per call on the frozen target-control service. The channel does
//! not re-implement session bookkeeping: the frame names the session
//! generation, the guest refuses any other one, and a session that is gone
//! answers `SessionUnavailable`.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use d2b_contracts_resource::v3::ResourceRef;
use d2b_resource_runtime::guest_target::{
    GuestTargetControl, GuestTargetError, TargetControlChannel, TargetControlClient,
    TARGET_CONTROL_METHOD, TARGET_CONTROL_SERVICE,
};
use d2b_resource_runtime::target::TargetRef;
use d2bd_runtime::guest_component_session::GuestComponentSessionClient;

/// Bounded deadline for one target-control round trip.
const TARGET_CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

/// One target-control channel over one authenticated Guest session.
#[derive(Debug)]
pub struct SessionTargetControlChannel {
    session: Arc<GuestComponentSessionClient>,
}

impl SessionTargetControlChannel {
    /// Wrap one authenticated Guest session.
    pub fn new(session: Arc<GuestComponentSessionClient>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl TargetControlChannel for SessionTargetControlChannel {
    async fn call(&self, frame: Vec<u8>) -> Result<Vec<u8>, GuestTargetError> {
        // A session that is no longer live cannot carry a target-control
        // request: the answer is the closed `SessionUnavailable`, never a
        // retry on another generation (R21, R28).
        if !self.session.route_binding().liveness().is_live() {
            return Err(GuestTargetError::SessionUnavailable);
        }
        let request = ttrpc::Request {
            service: TARGET_CONTROL_SERVICE.to_owned(),
            method: TARGET_CONTROL_METHOD.to_owned(),
            payload: frame,
            timeout_nano: TARGET_CONTROL_TIMEOUT.as_nanos() as i64,
            ..ttrpc::Request::default()
        };
        let response = self
            .session
            .client()
            .request(request)
            .await
            .map_err(|_| GuestTargetError::SessionUnavailable)?;
        Ok(response.payload)
    }
}

/// The target-control handle of one live Guest session generation.
///
/// The returned handle is generation-bound: a request naming another
/// generation is refused host-side, and the guest refuses it again on the
/// wire.
pub fn session_target_control(
    session: Arc<GuestComponentSessionClient>,
    session_generation: u64,
) -> Result<Arc<dyn GuestTargetControl>, GuestTargetError> {
    let client =
        TargetControlClient::new(SessionTargetControlChannel::new(session), session_generation)?;
    Ok(Arc::new(client))
}

/// The directory target reference of an authenticated Guest session.
pub fn guest_target_ref(guest: &ResourceRef) -> Option<TargetRef> {
    if guest.resource_type().as_str() != "Guest" {
        return None;
    }
    TargetRef::guest(guest.name().as_str()).ok()
}
