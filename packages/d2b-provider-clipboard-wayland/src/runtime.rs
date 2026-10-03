//! Authenticated clipboard Provider runtime composition.

use std::collections::BTreeMap;

use d2b_contracts_resource::v3::ResourceRef;
use d2b_provider_toolkit::AuthenticatedSessionRouteBinding;

use crate::{
    AuthenticatedPasteRoute, ClipboardAuditSink, ClipboardServiceError, DisplayDependencyEvidence,
    GuestSelectionEvent, PickerReceipt, PickerRequest, PickerResult, Policy,
    service::{
        AdmittedClipboardEndpoint, AuthenticatedClipboardSession, ClipboardEndpointFence,
        ClipboardEndpointGrant, ClipboardEndpointRefusal, ClipboardEndpointRole,
        ClipboardServiceRole, ClipdHost,
    },
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
    /// No admitted endpoint relationship carries the delivery channel this
    /// operation needs.
    ///
    /// A missing or revoked relationship stops delivery here. It is never a
    /// reason to reach another host channel: the alternative display routes
    /// below are consulted only after an admitted relationship exists.
    EndpointUnavailable(ClipboardEndpointRefusal),
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
            Self::EndpointUnavailable(refusal) => formatter.write_str(refusal.code()),
        }
    }
}

impl std::error::Error for ClipboardRuntimeError {}

impl From<ClipboardEndpointRefusal> for ClipboardRuntimeError {
    fn from(refusal: ClipboardEndpointRefusal) -> Self {
        Self::EndpointUnavailable(refusal)
    }
}

/// Finalization evidence for one clipboard Provider instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardFinalizationReport {
    /// Whether daemon-owned worker drain completed.
    pub drained: bool,
    /// Whether ComponentSession authority release completed.
    pub authority_released: bool,
}

/// Long-lived authenticated clipboard runtime.
///
/// The runtime holds the admitted `EndpointBinding` relationship per delivery
/// channel ([`ClipboardEndpointRole`]). A channel with no admitted
/// relationship, or one whose relationship the graph has revoked or is
/// draining, cannot be delivered through: the gate runs before any host route
/// is constructed, so a withdrawn endpoint stops the selection instead of
/// redirecting it onto another host channel.
pub struct ClipboardRuntime<E> {
    host: ClipdHost,
    effects: E,
    endpoints: BTreeMap<ClipboardEndpointRole, ClipboardEndpointGrant>,
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
            endpoints: BTreeMap::new(),
            finalized: false,
        })
    }

    /// Reconcile the admitted endpoint relationship of one delivery channel.
    ///
    /// A `None` relationship is a fail-closed revocation of that channel: it
    /// drops the retained relationship, and the channel stops delivering.
    pub fn reconcile_endpoint(
        &mut self,
        role: ClipboardEndpointRole,
        admitted: Option<AdmittedClipboardEndpoint>,
    ) -> Result<(), ClipboardRuntimeError> {
        match admitted {
            Some(admitted) if admitted.role() != role => {
                tracing::debug!(?role, "endpoint reconcile refused: relationship belongs to another channel");
                Err(ClipboardRuntimeError::EndpointUnavailable(
                    ClipboardEndpointRefusal::channel_mismatch(),
                ))
            }
            Some(admitted) => {
                self.endpoints
                    .insert(role, ClipboardEndpointGrant::new(admitted));
                Ok(())
            }
            None => {
                self.endpoints.remove(&role);
                Ok(())
            }
        }
    }

    /// Borrow the retained grant of one delivery channel, when one is held.
    pub fn endpoint(&self, role: ClipboardEndpointRole) -> Option<&ClipboardEndpointGrant> {
        self.endpoints.get(&role)
    }

    /// Re-fence one retained relationship at newer committed state.
    ///
    /// The graph moving on is the ordinary case, not a re-admission: the
    /// relationship keeps its identity while its committed fence advances, is
    /// raised to a new reconnect generation, or is revoked. A revoked or
    /// superseded grant stops carrying deliveries from the next call onward.
    ///
    /// # Errors
    ///
    /// Returns [`ClipboardRuntimeError::EndpointUnavailable`] when the channel
    /// holds no retained relationship to re-fence.
    pub fn refence_endpoint(
        &mut self,
        role: ClipboardEndpointRole,
        fence: ClipboardEndpointFence,
    ) -> Result<(), ClipboardRuntimeError> {
        let grant = self.endpoints.get(&role).cloned().ok_or(
            ClipboardEndpointRefusal::relationship_absent(),
        )?;
        if grant.role() != role {
            return Err(ClipboardEndpointRefusal::channel_mismatch().into());
        }
        self.endpoints.insert(role, grant.refenced(fence));
        Ok(())
    }

    /// The one gate every endpoint-gated delivery runs before it constructs a
    /// host route.
    ///
    /// A channel with no retained relationship, or one whose relationship no
    /// longer admits delivery, is refused here. Nothing after this point can
    /// substitute another host channel for the withdrawn one.
    fn admitted_endpoint(
        &self,
        role: ClipboardEndpointRole,
    ) -> Result<&AdmittedClipboardEndpoint, ClipboardRuntimeError> {
        let grant = self
            .endpoints
            .get(&role)
            .ok_or(ClipboardEndpointRefusal::relationship_absent())?;
        if let Some(refusal) = grant.delivery_refusal() {
            tracing::debug!(?role, refusal = refusal.code(), "delivery refused: endpoint relationship withdrawn");
            return Err(refusal.into());
        }
        Ok(grant.admitted())
    }

    /// Capture one Guest selection through the admitted Guest-transfer
    /// endpoint relationship.
    ///
    /// The endpoint gate runs first, so a withdrawn transfer relationship
    /// stops the capture before the route is even authenticated; the
    /// direction, role, and history checks below then run unchanged. Content
    /// never participates in the gate: `mime` and `bytes` are consumed by the
    /// policy after admission and cannot influence the relationship.
    ///
    /// # Errors
    ///
    /// Returns [`ClipboardRuntimeError::EndpointUnavailable`] when the
    /// Guest-transfer channel carries no admitted relationship, or when its
    /// relationship no longer admits delivery.
    pub fn capture_guest_route_over_endpoint(
        &mut self,
        route: AuthenticatedSessionRouteBinding,
        mime: &str,
        bytes: &[u8],
        now_secs: u64,
    ) -> Result<String, ClipboardRuntimeError> {
        self.admitted_endpoint(ClipboardEndpointRole::GuestTransfer)?;
        self.capture_guest_route(route, mime, bytes, now_secs)
    }

    /// Capture one host selection through the admitted host-selection-read
    /// endpoint relationship.
    ///
    /// The endpoint gate runs before the host route is constructed, so a
    /// missing or withdrawn read relationship stops the capture instead of
    /// letting the display observer or display dependency route stand in for
    /// it. The policy and direction checks the Provider has always applied -
    /// display dependency readiness, host/bridge role, dependency user match,
    /// host-capture policy, echo suppression, item byte ceiling, and the
    /// fail-closed audit queue - then run unchanged on the admitted channel.
    ///
    /// # Errors
    ///
    /// Returns [`ClipboardRuntimeError::EndpointUnavailable`] when the
    /// host-selection-read channel carries no admitted relationship, or when
    /// its relationship no longer admits delivery.
    pub fn capture_host_route_over_endpoint(
        &mut self,
        route: AuthenticatedSessionRouteBinding,
        mime: &str,
        bytes: &[u8],
        source_event: Option<GuestSelectionEvent>,
        observer_user: Option<&ResourceRef>,
        now_secs: u64,
    ) -> Result<String, ClipboardRuntimeError> {
        self.admitted_endpoint(ClipboardEndpointRole::HostSelectionRead)?;
        self.capture_host_route(route, mime, bytes, source_event, observer_user, now_secs)
    }

    /// Borrow the service state for authenticated request dispatch.
    pub const fn host(&self) -> &ClipdHost {
        &self.host
    }

    /// Mutably borrow the service state for authenticated request dispatch.
    pub const fn host_mut(&mut self) -> &mut ClipdHost {
        &mut self.host
    }

    /// Admit a route retained by the daemon after bus registration.
    ///
    /// # Errors
    ///
    /// Returns [`ClipboardRuntimeError::SessionUnauthenticated`] when the
    /// retained route is not authenticated for this Provider.
    pub fn admit_route(
        &self,
        route: AuthenticatedSessionRouteBinding,
    ) -> Result<AuthenticatedClipboardSession, ClipboardRuntimeError> {
        AuthenticatedClipboardSession::from_authenticated_route(route)
            .map_err(|e| {
                tracing::debug!(error = %e, "admission refused: retained route not authenticated for clipboard");
                ClipboardRuntimeError::SessionUnauthenticated
            })
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
            tracing::debug!("admission refused: route is not a clipboard bridge session");
            return Err(ClipboardRuntimeError::SessionRoleInvalid);
        }
        self.host
            .capture_guest(&authenticated, mime, bytes, now_secs)
            .map_err(ClipboardRuntimeError::Service)
    }

    /// Capture one Guest selection through an authenticated Provider
    /// transport and a daemon-validated committed Guest projection.
    pub fn capture_guest_route_for_guest(
        &mut self,
        route: AuthenticatedSessionRouteBinding,
        guest_ref: ResourceRef,
        mime: &str,
        bytes: &[u8],
        now_secs: u64,
    ) -> Result<String, ClipboardRuntimeError> {
        let authenticated =
            AuthenticatedClipboardSession::from_authenticated_route_for_guest(route, guest_ref)
                .map_err(|e| {
                    tracing::debug!(error = %e, "admission refused: guest route not authenticated for clipboard");
                    ClipboardRuntimeError::SessionUnauthenticated
                })?;
        if authenticated.role() != ClipboardServiceRole::Bridge {
            tracing::debug!("admission refused: guest route is not a clipboard bridge session");
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

    /// Issue an echo-suppression event for a daemon-validated committed Guest
    /// projection over an authenticated Provider transport.
    pub fn guest_selection_event_route_for_guest(
        &mut self,
        route: AuthenticatedSessionRouteBinding,
        guest_ref: ResourceRef,
        entry_digest: &str,
        now_secs: u64,
    ) -> Result<GuestSelectionEvent, ClipboardRuntimeError> {
        let authenticated =
            AuthenticatedClipboardSession::from_authenticated_route_for_guest(route, guest_ref)
                .map_err(|e| {
                    tracing::debug!(error = %e, "admission refused: guest route not authenticated for clipboard");
                    ClipboardRuntimeError::SessionUnauthenticated
                })?;
        self.host
            .guest_selection_event(&authenticated, entry_digest, now_secs)
            .map_err(ClipboardRuntimeError::Service)
    }

    /// Capture a host selection through the daemon-retained route.
    pub fn capture_host_route(
        &mut self,
        route: AuthenticatedSessionRouteBinding,
        mime: &str,
        bytes: &[u8],
        source_event: Option<GuestSelectionEvent>,
        observer_user: Option<&ResourceRef>,
        now_secs: u64,
    ) -> Result<String, ClipboardRuntimeError> {
        let authenticated = AuthenticatedClipboardSession::from_authenticated_route(route.clone())
            .or_else(|_| AuthenticatedClipboardSession::from_display_observer_route(route.clone()))
            .or_else(|_| {
                observer_user
                    .cloned()
                    .ok_or(ClipboardServiceError::HostSessionInvalid)
                    .and_then(|user_ref| {
                        AuthenticatedClipboardSession::from_display_dependency_route(
                            route, user_ref,
                        )
                    })
            })
            .map_err(|e| {
                tracing::debug!(error = %e, "admission refused: host route not authenticated for clipboard capture");
                ClipboardRuntimeError::SessionUnauthenticated
            })?;
        if authenticated.role() != ClipboardServiceRole::Bridge {
            tracing::debug!("admission refused: host route is not a clipboard bridge session");
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
        receipt: &crate::PickerReceipt,
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
            tracing::debug!(
                source_role = ?source.role(),
                destination_role = ?destination.role(),
                "picker completion rejected: session roles do not match picker/bridge contract"
            );
            return Err(ClipboardRuntimeError::SessionRoleInvalid);
        }
        self.host
            .complete_picker(source, destination, request, result, entry_digest, now_secs)
            .map_err(|e| {
                tracing::warn!(
                    error = %e,
                    "picker completion failed; collapsing failure to PickerReceiptInvalid"
                );
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
            .map_err(|_e| {
                tracing::warn!("audit flush through daemon-owned sink failed");
                ClipboardRuntimeError::Service(ClipboardServiceError::AuditUnavailable)
            })
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
            .map_err(|e| {
                tracing::warn!(error = %e, "worker drain effect failed during clipboard finalization");
                ClipboardRuntimeError::Service(e)
            })?;
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
            .map_err(|e| {
                tracing::warn!(error = %e, "display dependency revocation failed during clipboard finalization");
                ClipboardRuntimeError::Service(e)
            })?;
        self.effects
            .release_authority()
            .map_err(|e| {
                tracing::error!(error = %e, "session authority release failed during clipboard finalization");
                ClipboardRuntimeError::Service(e)
            })?;
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
            .map_err(|_e| {
                tracing::warn!("audit flush through daemon-owned effect sink failed");
                ClipboardRuntimeError::Service(ClipboardServiceError::AuditUnavailable)
            })
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
