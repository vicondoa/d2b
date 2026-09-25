//! Daemon-side session adapter for the Guest family's target-control channel.
//!
//! The family crate declares [`GuestTargetSession`] - a liveness answer and
//! one request/answer round trip - and owns the channel's policy: the live
//! generation fence, the bounded deadline, and the frozen service method.
//! This module is the other half of that seam: the daemon's authenticated
//! `GuestComponentSession` client, which is the carrier the channel rides.
//! The session runtime stays here because it belongs to the daemon, not to
//! the family.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_guest::GuestTargetSession;
use d2b_resource_runtime::guest_target::GuestTargetError;
use d2bd_runtime::guest_component_session::GuestComponentSessionClient;

/// The daemon's authenticated Guest session, offered to the family's channel.
#[derive(Debug)]
pub(crate) struct DaemonGuestTargetSession {
    session: Arc<GuestComponentSessionClient>,
}

impl DaemonGuestTargetSession {
    /// Wrap one accepted Guest session.
    pub(crate) fn new(session: Arc<GuestComponentSessionClient>) -> Self {
        Self { session }
    }
}

#[async_trait]
impl GuestTargetSession for DaemonGuestTargetSession {
    fn is_live(&self) -> bool {
        self.session.route_binding().liveness().is_live()
    }

    /// Forward one target-control request through the live session client.
    ///
    /// # Errors
    ///
    /// Returns [`GuestTargetError::SessionUnavailable`] when the session is
    /// no longer live or the request fails.

    async fn request(
        &self,
        request: ttrpc::Request,
    ) -> Result<ttrpc::Response, GuestTargetError> {
        // A session that is no longer live cannot carry a target-control
        // request: the answer is the closed `SessionUnavailable`, never a
        // retry on another generation (R21, R28).
        self.session
            .client()
            .request(request)
            .await
            .map_err(|_| GuestTargetError::SessionUnavailable)
    }
}
