//! Host-side target-control channel over the authenticated Guest
//! ComponentSession (U13, spec section 23.3).
//!
//! One frame per call on the frozen target-control service. The channel does
//! not re-implement session bookkeeping: the frame names the session
//! generation, the guest refuses any other one, and a session that is gone
//! answers `SessionUnavailable`.
//!
//! The session itself is the daemon's: the family declares the small port the
//! channel needs ([`GuestTargetSession`] - a liveness answer and one
//! request/answer round trip) and the daemon implements it over its
//! authenticated `GuestComponentSession` client, so this crate carries no
//! session runtime and no carrier of its own.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use d2b_contracts_resource::v3::ResourceRef;
use d2b_resource_runtime::guest_target::{
    GuestTargetControl, GuestTargetError, TargetControlChannel, TargetControlClient,
    TARGET_CONTROL_METHOD, TARGET_CONTROL_SERVICE,
};
use d2b_resource_runtime::target::TargetRef;

/// Bounded deadline for one target-control round trip.
const TARGET_CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

/// One authenticated Guest session a target-control channel rides.
///
/// The daemon implements this over the accepted session's own carrier: the
/// liveness answer is the session's live generation, and one request is one
/// round trip on the session's authenticated client. A session that is gone
/// answers [`GuestTargetError::SessionUnavailable`], never a retry on another
/// generation.
#[async_trait]
pub trait GuestTargetSession: Send + Sync + 'static {
    /// Whether this session is still live.
    fn is_live(&self) -> bool;

    /// Carry one target-control request over the session.
    async fn request(
        &self,
        request: ttrpc::Request,
    ) -> Result<ttrpc::Response, GuestTargetError>;
}

/// One target-control channel over one authenticated Guest session.
pub struct SessionTargetControlChannel<S: GuestTargetSession> {
    session: S,
}

/// The channel names the session it rides; the session's own shape is the
/// daemon's and is not rendered here.
impl<S: GuestTargetSession> std::fmt::Debug for SessionTargetControlChannel<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionTargetControlChannel")
            .finish_non_exhaustive()
    }
}

impl<S: GuestTargetSession> SessionTargetControlChannel<S> {
    /// Wrap one authenticated Guest session.
    pub fn new(session: S) -> Self {
        Self { session }
    }
}

#[async_trait]
impl<S: GuestTargetSession> TargetControlChannel for SessionTargetControlChannel<S> {
    async fn call(&self, frame: Vec<u8>) -> Result<Vec<u8>, GuestTargetError> {
        // A session that is no longer live cannot carry a target-control
        // request: the answer is the closed `SessionUnavailable`, never a
        // retry on another generation (R21, R28).
        if !self.session.is_live() {
            return Err(GuestTargetError::SessionUnavailable);
        }
        let request = ttrpc::Request {
            service: TARGET_CONTROL_SERVICE.to_owned(),
            method: TARGET_CONTROL_METHOD.to_owned(),
            payload: frame,
            timeout_nano: TARGET_CONTROL_TIMEOUT.as_nanos() as i64,
            ..ttrpc::Request::default()
        };
        let response = self.session.request(request).await?;
        Ok(response.payload)
    }
}

/// The target-control handle of one live Guest session generation.
///
/// The returned handle is generation-bound: a request naming another
/// generation is refused host-side, and the guest refuses it again on the
/// wire.
///
/// # Errors
///
/// Returns the [`GuestTargetError`] the client construction reports when
/// the session generation binding cannot be established.
pub fn session_target_control<S: GuestTargetSession>(
    session: S,
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
