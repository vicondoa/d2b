//! Authenticated clipboard Provider runtime composition.

use d2b_provider_toolkit::{AuthenticatedComponentSession, AuthenticatedSessionRouteBinding};

use crate::{
    AuthenticatedPasteRoute, ClipboardAuditSink, ClipboardServiceError, DisplayDependencyEvidence,
    GuestSelectionEvent, PickerReceipt, PickerRequest, PickerResult, Policy,
    service::{AuthenticatedClipboardSession, ClipboardServiceRole, ClipdHost},
};

/// Daemon-owned effects needed to drain clipboard workers and authority.
pub trait ClipboardProcessEffectPort {
    /// Drain the controller, bridge, and picker workers.
    fn drain(&mut self) -> Result<(), ClipboardServiceError>;
    /// Release the authenticated ComponentSession authority.
    fn release_authority(&mut self) -> Result<(), ClipboardServiceError>;
}

/// Stable failures at the authenticated clipboard runtime boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardRuntimeError {
    /// The supplied ComponentSession is not admitted for this Provider.
    SessionUnauthenticated,
    /// The session is not a clipboard bridge session.
    SessionRoleInvalid,
    /// The display dependency could not be authenticated.
    DisplayDependencyUnavailable,
    /// A clipboard service operation failed.
    Service(ClipboardServiceError),
}

impl core::fmt::Display for ClipboardRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Service(error) => error.fmt(formatter),
            Self::SessionUnauthenticated => {
                formatter.write_str("clipboard-runtime-session-unauthenticated")
            }
            Self::SessionRoleInvalid => {
                formatter.write_str("clipboard-runtime-session-role-invalid")
            }
            Self::DisplayDependencyUnavailable => {
                formatter.write_str("clipboard-runtime-display-unavailable")
            }
        }
    }
}

impl std::error::Error for ClipboardRuntimeError {}

/// Finalization evidence for one clipboard Provider instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardFinalizationReport {
    /// Whether daemon-owned worker drain completed.
    pub drained: bool,
    /// Whether ComponentSession authority release completed.
    pub authority_released: bool,
}

/// Long-lived authenticated clipboard runtime.
pub struct ClipboardRuntime<E> {
    host: ClipdHost,
    effects: E,
    finalized: bool,
}

impl<E: ClipboardProcessEffectPort> ClipboardRuntime<E> {
    /// Construct the runtime with the configured clipboard policy.
    pub fn new(
        policy: Policy,
        audit_capacity: usize,
        display: Option<DisplayDependencyEvidence>,
        effects: E,
    ) -> Result<Self, ClipboardRuntimeError> {
        Ok(Self {
            host: ClipdHost::new(policy, audit_capacity, display)
                .map_err(ClipboardRuntimeError::Service)?,
            effects,
            finalized: false,
        })
    }

    /// Borrow the service state for authenticated request dispatch.
    pub const fn host(&self) -> &ClipdHost {
        &self.host
    }

    /// Mutably borrow the service state for authenticated request dispatch.
    pub const fn host_mut(&mut self) -> &mut ClipdHost {
        &mut self.host
    }

    /// Admit one ComponentSession and project its clipboard identity.
    pub fn admit_session<C>(
        &self,
        session: &AuthenticatedComponentSession<C>,
    ) -> Result<AuthenticatedClipboardSession, ClipboardRuntimeError> {
        let authenticated = AuthenticatedClipboardSession::from_component_session(session)
            .map_err(|_| ClipboardRuntimeError::SessionUnauthenticated)?;
        Ok(authenticated)
    }

    /// Admit a route retained by the daemon after bus registration.
    pub fn admit_route(
        &self,
        route: AuthenticatedSessionRouteBinding,
    ) -> Result<AuthenticatedClipboardSession, ClipboardRuntimeError> {
        AuthenticatedClipboardSession::from_authenticated_route(route)
            .map_err(|_| ClipboardRuntimeError::SessionUnauthenticated)
    }

    /// Admit a bridge session for host/Guest selection mediation.
    pub fn admit_bridge_session<C>(
        &self,
        session: &AuthenticatedComponentSession<C>,
    ) -> Result<AuthenticatedClipboardSession, ClipboardRuntimeError> {
        self.admit_role(session, ClipboardServiceRole::Bridge)
    }

    /// Admit a picker coordination session.
    pub fn admit_picker_session<C>(
        &self,
        session: &AuthenticatedComponentSession<C>,
    ) -> Result<AuthenticatedClipboardSession, ClipboardRuntimeError> {
        self.admit_role(session, ClipboardServiceRole::Picker)
    }

    /// Admit a management session for lifecycle and policy operations.
    pub fn admit_management_session<C>(
        &self,
        session: &AuthenticatedComponentSession<C>,
    ) -> Result<AuthenticatedClipboardSession, ClipboardRuntimeError> {
        self.admit_role(session, ClipboardServiceRole::Management)
    }

    fn admit_role<C>(
        &self,
        session: &AuthenticatedComponentSession<C>,
        role: ClipboardServiceRole,
    ) -> Result<AuthenticatedClipboardSession, ClipboardRuntimeError> {
        let authenticated = self.admit_session(session)?;
        if authenticated.role() != role {
            return Err(ClipboardRuntimeError::SessionRoleInvalid);
        }
        Ok(authenticated)
    }

    /// Reconcile the authenticated display dependency.
    pub fn reconcile_display(
        &mut self,
        display: Option<DisplayDependencyEvidence>,
    ) -> Result<(), ClipboardRuntimeError> {
        let absent = display.is_none();
        let result = self.host.reconcile_display_dependency(display);
        if absent {
            self.effects
                .drain()
                .map_err(ClipboardRuntimeError::Service)?;
        }
        result.map(|_| ()).map_err(ClipboardRuntimeError::Service)
    }

    /// Capture one Guest selection after authenticating the bridge session.
    pub fn capture_guest<C>(
        &mut self,
        session: &AuthenticatedComponentSession<C>,
        mime: &str,
        bytes: &[u8],
        now_secs: u64,
    ) -> Result<String, ClipboardRuntimeError> {
        let authenticated = self.admit_bridge_session(session)?;
        self.host
            .capture_guest(&authenticated, mime, bytes, now_secs)
            .map_err(ClipboardRuntimeError::Service)
    }

    /// Capture a Guest selection through the daemon-retained route.
    pub fn capture_guest_route(
        &mut self,
        route: AuthenticatedSessionRouteBinding,
        mime: &str,
        bytes: &[u8],
        now_secs: u64,
    ) -> Result<String, ClipboardRuntimeError> {
        let authenticated = self.admit_route(route)?;
        if authenticated.role() != ClipboardServiceRole::Bridge {
            return Err(ClipboardRuntimeError::SessionRoleInvalid);
        }
        self.host
            .capture_guest(&authenticated, mime, bytes, now_secs)
            .map_err(ClipboardRuntimeError::Service)
    }

    /// Issue an authenticated, opaque echo-suppression event for one live
    /// Guest selection.
    pub fn guest_selection_event_route(
        &mut self,
        route: AuthenticatedSessionRouteBinding,
        entry_digest: &str,
        now_secs: u64,
    ) -> Result<GuestSelectionEvent, ClipboardRuntimeError> {
        let authenticated = self.admit_route(route)?;
        self.host
            .guest_selection_event(&authenticated, entry_digest, now_secs)
            .map_err(ClipboardRuntimeError::Service)
    }

    /// Capture one host selection after authenticating the bound User bridge.
    pub fn capture_host<C>(
        &mut self,
        session: &AuthenticatedComponentSession<C>,
        mime: &str,
        bytes: &[u8],
        source_event: Option<GuestSelectionEvent>,
        now_secs: u64,
    ) -> Result<String, ClipboardRuntimeError> {
        let authenticated = self.admit_bridge_session(session)?;
        self.host
            .capture_host(&authenticated, mime, bytes, source_event, now_secs)
            .map_err(ClipboardRuntimeError::Service)
    }

    /// Capture a host selection through the daemon-retained route.
    pub fn capture_host_route(
        &mut self,
        route: AuthenticatedSessionRouteBinding,
        mime: &str,
        bytes: &[u8],
        source_event: Option<GuestSelectionEvent>,
        now_secs: u64,
    ) -> Result<String, ClipboardRuntimeError> {
        let authenticated = self.admit_route(route)?;
        if authenticated.role() != ClipboardServiceRole::Bridge {
            return Err(ClipboardRuntimeError::SessionRoleInvalid);
        }
        self.host
            .capture_host(&authenticated, mime, bytes, source_event, now_secs)
            .map_err(ClipboardRuntimeError::Service)
    }

    /// Authorize a paste route after the authenticated picker completed.
    pub fn authorize_paste_after_picker(
        &self,
        route: &AuthenticatedPasteRoute,
        receipt: crate::PickerReceipt,
        entry_digest: &str,
        now_secs: u64,
    ) -> Result<(), ClipboardRuntimeError> {
        self.host
            .authorize_paste_after_picker(route, receipt, entry_digest, now_secs)
            .map_err(ClipboardRuntimeError::Service)
    }

    /// Consume a picker receipt and materialize the selected bounded payload.
    pub fn materialize_after_picker(
        &mut self,
        route: &AuthenticatedPasteRoute,
        receipt: crate::PickerReceipt,
        entry_digest: &str,
        now_secs: u64,
    ) -> Result<Vec<u8>, ClipboardRuntimeError> {
        self.host
            .materialize_after_picker(route, receipt, entry_digest, now_secs)
            .map_err(ClipboardRuntimeError::Service)
    }

    /// Complete one picker operation using two authenticated clipboard
    /// projections.  The returned receipt is one-use and is minted only after
    /// the history claim succeeds.
    pub fn complete_picker(
        &mut self,
        source: &AuthenticatedClipboardSession,
        destination: &AuthenticatedClipboardSession,
        request: &PickerRequest,
        result: PickerResult,
        entry_digest: impl Into<String>,
        now_secs: u64,
    ) -> Result<PickerReceipt, ClipboardRuntimeError> {
        if source.role() != ClipboardServiceRole::Picker
            || destination.role() != ClipboardServiceRole::Bridge
        {
            return Err(ClipboardRuntimeError::SessionRoleInvalid);
        }
        self.host
            .complete_picker(source, destination, request, result, entry_digest, now_secs)
            .map_err(|_| {
                ClipboardRuntimeError::Service(ClipboardServiceError::PickerReceiptInvalid)
            })
    }

    /// Flush bounded audit records through the daemon-owned sink.
    pub fn flush_audit_to<S: ClipboardAuditSink>(
        &mut self,
        sink: &mut S,
        limit: usize,
    ) -> Result<usize, ClipboardRuntimeError> {
        self.host
            .flush_audit(sink, limit)
            .map_err(|_| ClipboardRuntimeError::Service(ClipboardServiceError::AuditUnavailable))
    }

    /// Drain daemon-owned workers without releasing the authenticated
    /// ComponentSession authority.
    pub fn drain(&mut self) -> Result<(), ClipboardRuntimeError> {
        self.effects.drain().map_err(ClipboardRuntimeError::Service)
    }

    /// Finalize in order: stop workers, purge retained history, then release
    /// the authenticated authority.
    pub fn finalize(
        &mut self,
        guests: impl IntoIterator<Item = String>,
    ) -> Result<ClipboardFinalizationReport, ClipboardRuntimeError> {
        if self.finalized {
            return Ok(ClipboardFinalizationReport {
                drained: true,
                authority_released: true,
            });
        }
        self.effects
            .drain()
            .map_err(ClipboardRuntimeError::Service)?;
        let mut had_guest = false;
        for guest in guests {
            had_guest = true;
            self.host.purge_guest(&guest);
        }
        if !had_guest {
            self.host.purge_all();
        }
        self.host
            .reconcile_display_dependency(None)
            .map_err(ClipboardRuntimeError::Service)?;
        self.effects
            .release_authority()
            .map_err(ClipboardRuntimeError::Service)?;
        self.finalized = true;
        Ok(ClipboardFinalizationReport {
            drained: true,
            authority_released: true,
        })
    }
}

impl<E: ClipboardProcessEffectPort + ClipboardAuditSink> ClipboardRuntime<E> {
    /// Flush queued redacted audit events through the daemon-owned effect
    /// sink, retaining the head when the sink cannot durably accept it.
    pub fn flush_audit(&mut self, limit: usize) -> Result<usize, ClipboardRuntimeError> {
        self.host
            .flush_audit(&mut self.effects, limit)
            .map_err(|_| ClipboardRuntimeError::Service(ClipboardServiceError::AuditUnavailable))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Effects {
        calls: Vec<&'static str>,
    }

    impl ClipboardProcessEffectPort for Effects {
        fn drain(&mut self) -> Result<(), ClipboardServiceError> {
            self.calls.push("drain");
            Ok(())
        }

        fn release_authority(&mut self) -> Result<(), ClipboardServiceError> {
            self.calls.push("authority");
            Ok(())
        }
    }

    #[test]
    fn finalization_drains_workers_revokes_dependency_and_releases_authority() {
        let mut runtime =
            ClipboardRuntime::new(Policy::default(), 4, None, Effects::default()).unwrap();
        let report = runtime
            .finalize([String::from("Guest/work")])
            .expect("finalization succeeds");
        assert_eq!(
            report,
            ClipboardFinalizationReport {
                drained: true,
                authority_released: true
            }
        );
        assert!(runtime.host().dependency().evidence.is_none());
        assert_eq!(runtime.effects.calls, ["drain", "authority"]);
        assert_eq!(runtime.finalize([]).unwrap(), report);
    }
}
