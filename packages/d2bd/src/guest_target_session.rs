//! Daemon-side session adapter for the Guest family's target-control channel.
//!
//! The family crate declares [`GuestTargetSession`] - a liveness answer and
//! one request/answer round trip - and owns the channel's policy: the live
//! generation fence, the bounded deadline, and the frozen service method.
//! This module is the other half of that seam: the daemon's authenticated
//! `GuestComponentSession` client, which is the carrier the channel rides.
//! The session runtime stays here because it belongs to the daemon, not to
//! the family.
//!
//! ## Where the common contract lives
//!
//! The common Guest target/session contract is
//! [`d2b_provider_guest::GuestTargetContract`], and its evidence type is
//! `d2bd_runtime::target_runtime::GuestParentSessionEvidence`. Both are
//! provider-neutral: the declared Provider is carried as graph data and never
//! matched, so the local VM, media, and remote-cloud Guest providers all ride
//! the same contract.
//!
//! This adapter is the carrier, not the contract: the family fences the
//! channel on the graph evidence, and this carrier only answers whether the
//! session underneath it is still live. U34 switches the daemon's two
//! composition call sites from
//! [`d2b_provider_guest::session_target_control`] to
//! [`d2b_provider_guest::graph_target_control`], which runs the same
//! carrier's frames through the contract first.

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
