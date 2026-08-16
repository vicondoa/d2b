//! Authenticated notification Provider runtime composition.

use d2b_contracts::v3::ResourceRef;
use d2b_provider_toolkit::{AuthenticatedComponentSession, AuthenticatedSessionRouteBinding};

use crate::{
    ActionNonceError, DesktopNotificationPort, DisplayDependencyEvidence, GuestSource,
    NotificationController, NotificationError, NotificationProviderConfig, NotificationRequest,
    NotificationResult, NotificationSink, SessionEvidence, SourceProcessEffectPort,
    SourceReconcileResult,
};

/// Daemon-owned notification effect boundary.
pub trait NotificationProcessEffectPort: SourceProcessEffectPort {
    /// Release the authenticated ComponentSession authority after drain.
    fn release_authority(&mut self) -> Result<(), &'static str>;
}

/// Stable failures from notification runtime admission and reconciliation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationRuntimeError {
    /// A ComponentSession route did not authenticate for this Provider.
    SessionUnauthenticated,
    /// A source or observer session was not admitted.
    SessionAdmissionFailed,
    /// The display dependency was absent or invalid.
    DisplayDependencyUnavailable,
    /// The controller or daemon effect port rejected reconciliation.
    ReconciliationFailed,
}

impl core::fmt::Display for NotificationRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::SessionUnauthenticated => "notification-runtime-session-unauthenticated",
            Self::SessionAdmissionFailed => "notification-runtime-session-admission-failed",
            Self::DisplayDependencyUnavailable => "notification-runtime-display-unavailable",
            Self::ReconciliationFailed => "notification-runtime-reconciliation-failed",
        })
    }
}

impl std::error::Error for NotificationRuntimeError {}

/// Finalization evidence for one notification Provider instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationFinalizationReport {
    /// Number of source endpoints drained.
    pub drained_sources: usize,
    /// Whether the host sink was included in the drain plan.
    pub drained_host_sink: bool,
    /// Whether ComponentSession authority release completed.
    pub authority_released: bool,
}

/// Long-lived authenticated notification runtime.
pub struct NotificationRuntime<E> {
    controller: NotificationController,
    config: NotificationProviderConfig,
    sink: NotificationSink,
    effects: E,
    final_report: Option<NotificationFinalizationReport>,
}

impl<E: NotificationProcessEffectPort> NotificationRuntime<E> {
    /// Construct a runtime for one fixed notification Provider instance.
    pub fn new(
        config: NotificationProviderConfig,
        effects: E,
    ) -> Result<Self, NotificationRuntimeError> {
        Ok(Self {
            controller: NotificationController::new(crate::PROVIDER_REF)
                .map_err(|_| NotificationRuntimeError::ReconciliationFailed)?,
            sink: NotificationSink::from_config(&config),
            config,
            effects,
            final_report: None,
        })
    }

    /// Borrow the authenticated placement controller.
    pub const fn controller(&self) -> &NotificationController {
        &self.controller
    }

    /// Borrow the bounded host sink state.
    pub const fn sink(&self) -> &NotificationSink {
        &self.sink
    }

    /// Deliver one request through the authenticated source and observer
    /// sessions, using only the configured source category set.
    pub fn deliver<P: DesktopNotificationPort + ?Sized, C>(
        &mut self,
        port: &mut P,
        source_session: &AuthenticatedComponentSession<C>,
        observer_session: &AuthenticatedComponentSession<C>,
        request: NotificationRequest,
        now_secs: u64,
    ) -> Result<NotificationResult, NotificationError> {
        let source = self
            .source_evidence(source_session)
            .map_err(|_| NotificationError::InvalidOpaqueKey)?;
        let observer = self
            .source_evidence(observer_session)
            .map_err(|_| NotificationError::InvalidOpaqueKey)?;
        let config = self
            .config
            .guest_sources()
            .iter()
            .find(|configured| configured.source_ref() == source.subject_ref())
            .ok_or(NotificationError::InvalidOpaqueKey)?;
        let guest_source = GuestSource::from_config_at_generation(config, source.generation())
            .map_err(|_| NotificationError::InvalidOpaqueKey)?;
        self.sink.deliver_from_guest_source(
            port,
            &guest_source,
            &source,
            &observer,
            request,
            now_secs,
        )
    }

    /// Deliver using route projections already authenticated and retained by
    /// the daemon.  This is the production dispatcher entry point.
    pub fn deliver_evidence<P: DesktopNotificationPort + ?Sized>(
        &mut self,
        port: &mut P,
        source_session: &SessionEvidence,
        observer_session: &SessionEvidence,
        request: NotificationRequest,
        now_secs: u64,
    ) -> Result<NotificationResult, NotificationError> {
        let config = self
            .config
            .guest_sources()
            .iter()
            .find(|configured| configured.source_ref() == source_session.subject_ref())
            .ok_or(NotificationError::InvalidOpaqueKey)?;
        let guest_source =
            GuestSource::from_config_at_generation(config, source_session.generation())
                .map_err(|_| NotificationError::InvalidOpaqueKey)?;
        self.sink.deliver_from_guest_source(
            port,
            &guest_source,
            source_session,
            observer_session,
            request,
            now_secs,
        )
    }

    /// Deliver using an authenticated Provider transport and one Guest
    /// selected from committed daemon state.
    pub fn deliver_evidence_for_guest<P: DesktopNotificationPort + ?Sized>(
        &mut self,
        port: &mut P,
        source_route: AuthenticatedSessionRouteBinding,
        guest_ref: ResourceRef,
        observer_session: &SessionEvidence,
        request: NotificationRequest,
        now_secs: u64,
    ) -> Result<NotificationResult, NotificationError> {
        let source_session = SessionEvidence::from_daemon_route_for_guest(source_route, guest_ref)
            .map_err(|_| NotificationError::InvalidOpaqueKey)?;
        self.deliver_evidence(port, &source_session, observer_session, request, now_secs)
    }

    /// Consume one action capability using an authenticated observer session.
    pub fn invoke_action<C>(
        &mut self,
        action_key: &str,
        observer_session: &AuthenticatedComponentSession<C>,
        now_secs: u64,
    ) -> Result<String, ActionNonceError> {
        let observer = self
            .source_evidence(observer_session)
            .map_err(|_| ActionNonceError::SessionMismatch)?;
        self.sink.invoke_action(action_key, &observer, now_secs)
    }

    /// Consume one action capability using daemon-retained observer evidence.
    pub fn invoke_action_evidence(
        &mut self,
        action_key: &str,
        observer: &SessionEvidence,
        now_secs: u64,
    ) -> Result<String, ActionNonceError> {
        self.sink.invoke_action(action_key, observer, now_secs)
    }

    /// Close all projections owned by one authenticated observer session.
    pub fn close_observer<C>(
        &mut self,
        observer_session: &AuthenticatedComponentSession<C>,
    ) -> Result<(), NotificationRuntimeError> {
        let observer = self.source_evidence(observer_session)?;
        observer
            .admit_observer()
            .map_err(|_| NotificationRuntimeError::SessionAdmissionFailed)?;
        self.sink.close_session(&observer);
        Ok(())
    }

    /// Close all projections owned by daemon-retained observer evidence.
    pub fn close_observer_evidence(
        &mut self,
        observer: &SessionEvidence,
    ) -> Result<(), NotificationRuntimeError> {
        observer
            .admit_observer()
            .map_err(|_| NotificationRuntimeError::SessionAdmissionFailed)?;
        self.sink.close_session(observer);
        Ok(())
    }

    /// Admit a display route from the sealed ComponentSession authority.
    pub fn display_route<C>(
        &self,
        session: &AuthenticatedComponentSession<C>,
    ) -> Result<AuthenticatedSessionRouteBinding, NotificationRuntimeError> {
        let route = session.route_binding();
        DisplayDependencyEvidence::from_authenticated_route(route.clone())
            .map_err(|_| NotificationRuntimeError::DisplayDependencyUnavailable)?;
        Ok(route)
    }

    /// Admit one Guest source or local observer ComponentSession.
    pub fn source_evidence<C>(
        &self,
        session: &AuthenticatedComponentSession<C>,
    ) -> Result<SessionEvidence, NotificationRuntimeError> {
        SessionEvidence::from_component_session(session)
            .map_err(|_| NotificationRuntimeError::SessionAdmissionFailed)
    }

    /// Project one route retained by the daemon after authenticated bus
    /// registration into notification source evidence.
    pub fn source_route_evidence(
        &self,
        route: AuthenticatedSessionRouteBinding,
    ) -> Result<SessionEvidence, NotificationRuntimeError> {
        SessionEvidence::from_authenticated_route(route)
            .map_err(|_| NotificationRuntimeError::SessionAdmissionFailed)
    }

    /// Project daemon-local Guest source routes into typed source evidence.
    pub fn daemon_source_route_evidence(
        &self,
        route: AuthenticatedSessionRouteBinding,
    ) -> Result<SessionEvidence, NotificationRuntimeError> {
        SessionEvidence::from_daemon_route(route)
            .map_err(|_| NotificationRuntimeError::SessionAdmissionFailed)
    }

    /// Reconcile all configured source sessions against the authenticated
    /// display route. Missing or stale evidence drains existing ownership.
    pub fn reconcile<C>(
        &mut self,
        display: Option<&AuthenticatedComponentSession<C>>,
        source_sessions: &[&AuthenticatedComponentSession<C>],
    ) -> Result<SourceReconcileResult, NotificationRuntimeError> {
        let display_route = display.map(|session| session.route_binding());
        let source_evidence = source_sessions
            .iter()
            .map(|session| self.source_evidence(session))
            .collect::<Result<Vec<_>, _>>()?;
        self.controller
            .reconcile_authenticated_display_with_effects(
                display_route,
                &self.config,
                &source_evidence,
                &mut self.effects,
            )
            .map_err(|_| NotificationRuntimeError::ReconciliationFailed)
    }

    /// Reconcile source routes retained by the daemon after registration.
    pub fn reconcile_routes(
        &mut self,
        display: Option<AuthenticatedSessionRouteBinding>,
        source_routes: &[AuthenticatedSessionRouteBinding],
    ) -> Result<SourceReconcileResult, NotificationRuntimeError> {
        let source_evidence = source_routes
            .iter()
            .cloned()
            .map(|route| self.source_route_evidence(route))
            .collect::<Result<Vec<_>, _>>()?;
        self.controller
            .reconcile_authenticated_display_with_effects(
                display,
                &self.config,
                &source_evidence,
                &mut self.effects,
            )
            .map_err(|_| NotificationRuntimeError::ReconciliationFailed)
    }

    /// Reconcile daemon-local Guest source routes admitted through the
    /// authenticated ComponentSession listener.
    pub fn reconcile_daemon_routes(
        &mut self,
        display: Option<AuthenticatedSessionRouteBinding>,
        source_routes: &[AuthenticatedSessionRouteBinding],
    ) -> Result<SourceReconcileResult, NotificationRuntimeError> {
        let source_evidence = source_routes
            .iter()
            .cloned()
            .map(|route| self.daemon_source_route_evidence(route))
            .collect::<Result<Vec<_>, _>>()?;
        self.controller
            .reconcile_daemon_display_with_effects(
                display,
                &self.config,
                &source_evidence,
                &mut self.effects,
            )
            .map_err(|_| NotificationRuntimeError::ReconciliationFailed)
    }

    /// Reconcile configured Guest source projections over one authenticated
    /// daemon-owned Provider transport.
    pub fn reconcile_daemon_routes_for_guests(
        &mut self,
        display: Option<AuthenticatedSessionRouteBinding>,
        source_routes: &[AuthenticatedSessionRouteBinding],
        guest_refs: &[ResourceRef],
    ) -> Result<SourceReconcileResult, NotificationRuntimeError> {
        let source_evidence = source_routes
            .iter()
            .flat_map(|route| {
                guest_refs.iter().map(|guest_ref| {
                    SessionEvidence::from_daemon_route_for_guest(route.clone(), guest_ref.clone())
                        .map_err(|_| NotificationRuntimeError::SessionAdmissionFailed)
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.controller
            .reconcile_daemon_display_with_effects(
                display,
                &self.config,
                &source_evidence,
                &mut self.effects,
            )
            .map_err(|_| NotificationRuntimeError::ReconciliationFailed)
    }

    /// Drain source processes and the bounded host projection without
    /// releasing the authenticated ComponentSession authority.
    pub fn drain(&mut self) -> Result<SourceReconcileResult, NotificationRuntimeError> {
        let plan = self
            .controller
            .reconcile_authenticated_display_with_effects(
                None,
                &self.config,
                &[],
                &mut self.effects,
            )
            .map_err(|_| NotificationRuntimeError::ReconciliationFailed)?;
        self.sink.drain();
        Ok(plan)
    }

    /// Drain every source and sink before releasing the authenticated
    /// ComponentSession authority.
    pub fn finalize(&mut self) -> Result<NotificationFinalizationReport, NotificationRuntimeError> {
        if let Some(report) = self.final_report {
            return Ok(report);
        }
        let plan = self
            .controller
            .reconcile_authenticated_display_with_effects(
                None,
                &self.config,
                &[],
                &mut self.effects,
            )
            .map_err(|_| NotificationRuntimeError::ReconciliationFailed)?;
        let drained_sources = plan.stop_endpoints.len();
        let drained_host_sink = plan.stop_host_sink;
        self.sink.drain();
        self.effects
            .release_authority()
            .map_err(|_| NotificationRuntimeError::ReconciliationFailed)?;
        let report = NotificationFinalizationReport {
            drained_sources,
            drained_host_sink,
            authority_released: true,
        };
        self.final_report = Some(report);
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Category, DesktopNotificationPort, GuestSourceConfig, NotificationRequest,
        NotificationResult, SinkError,
        admission::{test_observer, test_source},
    };
    use d2b_contracts::v3::{ResourceRef, ZoneId};

    #[derive(Default)]
    struct Effects {
        plans: usize,
        authority_releases: usize,
    }

    impl SourceProcessEffectPort for Effects {
        fn apply(
            &mut self,
            plan: &SourceReconcileResult,
            _lifecycle: &crate::NotificationLifecyclePlan,
        ) -> Result<crate::SourceProcessEffectReceipt, &'static str> {
            self.plans += 1;
            Ok(crate::SourceProcessEffectReceipt::complete(plan))
        }
    }

    impl NotificationProcessEffectPort for Effects {
        fn release_authority(&mut self) -> Result<(), &'static str> {
            self.authority_releases += 1;
            Ok(())
        }
    }

    #[derive(Default)]
    struct Port {
        calls: usize,
    }

    impl DesktopNotificationPort for Port {
        fn activate(&mut self) -> Result<(), SinkError> {
            Ok(())
        }

        fn deactivate(&mut self) -> Result<(), SinkError> {
            Ok(())
        }

        fn notify(
            &mut self,
            notification: &crate::SanitizedNotification,
        ) -> Result<u32, SinkError> {
            self.calls += 1;
            assert!(!notification.body().is_empty());
            Ok(self.calls as u32)
        }
    }

    #[test]
    fn finalization_drains_the_effect_plan_before_releasing_authority() {
        let config = NotificationProviderConfig::new(Vec::new()).unwrap();
        let mut runtime = NotificationRuntime::new(config, Effects::default()).unwrap();
        let report = runtime.finalize().expect("finalization succeeds");
        assert_eq!(
            report,
            NotificationFinalizationReport {
                drained_sources: 0,
                drained_host_sink: false,
                authority_released: true
            }
        );
        assert_eq!(runtime.effects.plans, 0);
        assert_eq!(runtime.effects.authority_releases, 1);
        assert_eq!(runtime.finalize().unwrap(), report);
    }

    #[test]
    fn authenticated_evidence_delivery_redacts_and_issues_bounded_action_keys() {
        let source_ref = ResourceRef::parse("Guest/guest").unwrap();
        let source = GuestSourceConfig::new(
            source_ref,
            ZoneId::parse("work").unwrap(),
            [Category::SystemInfo],
        )
        .unwrap();
        let config = NotificationProviderConfig::new(vec![source]).unwrap();
        let mut runtime = NotificationRuntime::new(config, Effects::default()).unwrap();
        let mut port = Port::default();
        let request = NotificationRequest::new("summary", "body", Category::SystemInfo).unwrap();
        let result = runtime
            .deliver_evidence(
                &mut port,
                &test_source("guest"),
                &test_observer("alice"),
                request,
                1,
            )
            .unwrap();
        assert!(matches!(
            result,
            NotificationResult::Accepted {
                notification_id: 1,
                ..
            }
        ));
        assert_eq!(port.calls, 1);
    }
}
