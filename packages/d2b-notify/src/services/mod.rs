//! Composition boundary for the authenticated desktop services.
//!
//! Endpoint discovery and ComponentSession establishment are owned by the
//! caller. This module accepts only evidence from an already-established
//! pre-authorized session, starts observer and action handling transactionally,
//! and drops both bounded service stores as soon as that session is lost.

pub mod actions;
pub mod observer;

use actions::{ActionService, ActionSession};
use observer::{ObserverService, ObserverSession};

/// Session evidence supplied by the ComponentSession endpoint adapter.
///
/// Every value must come from negotiated session state, never from a request
/// envelope or a presentation projection.
pub trait EstablishedDesktopSession {
    fn service_package(&self) -> &str;
    fn endpoint_purpose(&self) -> &str;
    fn endpoint_role(&self) -> &str;
    fn is_established(&self) -> bool;
    fn is_authenticated(&self) -> bool;
    fn uses_pre_authorized_transport(&self) -> bool;
}

struct SessionEvidence<'a, S>(&'a S);

impl<S: EstablishedDesktopSession> actions::EstablishedComponentSession for SessionEvidence<'_, S> {
    fn service_package(&self) -> &str {
        self.0.service_package()
    }

    fn endpoint_purpose(&self) -> &str {
        self.0.endpoint_purpose()
    }

    fn endpoint_role(&self) -> &str {
        self.0.endpoint_role()
    }

    fn is_authenticated(&self) -> bool {
        self.0.is_authenticated()
    }

    fn uses_pre_authorized_transport(&self) -> bool {
        self.0.uses_pre_authorized_transport()
    }
}

impl<S: EstablishedDesktopSession> observer::EstablishedComponentSession
    for SessionEvidence<'_, S>
{
    fn service_package(&self) -> &str {
        self.0.service_package()
    }

    fn endpoint_purpose(&self) -> &str {
        self.0.endpoint_purpose()
    }

    fn endpoint_role(&self) -> &str {
        self.0.endpoint_role()
    }

    fn is_authenticated(&self) -> bool {
        self.0.is_authenticated()
    }

    fn uses_pre_authorized_transport(&self) -> bool {
        self.0.uses_pre_authorized_transport()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopServicePhase {
    Active,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopServiceCloseReason {
    SessionUnavailable,
    Requested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopStartupError {
    SessionNotEstablished,
    Unauthenticated,
    UntrustedTransport,
    ContractMismatch,
}

impl std::fmt::Display for DesktopStartupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = match self {
            Self::SessionNotEstablished => "desktop-session-not-established",
            Self::Unauthenticated => "desktop-session-unauthenticated",
            Self::UntrustedTransport => "desktop-transport-untrusted",
            Self::ContractMismatch => "desktop-session-contract-mismatch",
        };
        formatter.write_str(code)
    }
}

impl std::error::Error for DesktopStartupError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopServicesUnavailable;

impl std::fmt::Display for DesktopServicesUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("desktop-services-unavailable")
    }
}

impl std::error::Error for DesktopServicesUnavailable {}

struct ActiveServices {
    observer: ObserverService,
    actions: ActionService,
}

/// Observer and action handling bound to one authenticated ComponentSession.
///
/// Construction is transactional: neither service is exposed unless both
/// admissions succeed. Closing the session drops the observer queue, action
/// capabilities, and replay cache together. Reopening requires a fresh
/// authenticated session and a new `DesktopServices` value.
pub struct DesktopServices {
    active: Option<ActiveServices>,
    close_reason: Option<DesktopServiceCloseReason>,
}

impl std::fmt::Debug for DesktopServices {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DesktopServices")
            .field("phase", &self.phase())
            .field("close_reason", &self.close_reason)
            .finish()
    }
}

impl DesktopServices {
    pub fn start<S: EstablishedDesktopSession>(
        session: &S,
        now_secs: u64,
    ) -> Result<Self, DesktopStartupError> {
        if !session.is_established() {
            return Err(DesktopStartupError::SessionNotEstablished);
        }
        if actions::SERVICE_PACKAGE != observer::SERVICE_PACKAGE
            || actions::ENDPOINT_PURPOSE != observer::ENDPOINT_PURPOSE
            || actions::ENDPOINT_ROLE != observer::ENDPOINT_ROLE
        {
            return Err(DesktopStartupError::ContractMismatch);
        }

        let evidence = SessionEvidence(session);
        let observer = ObserverSession::admit(&evidence).map_err(map_observer_admission)?;
        let actions = ActionSession::admit(&evidence).map_err(map_action_admission)?;

        Ok(Self {
            active: Some(ActiveServices {
                observer: ObserverService::new(observer, now_secs),
                actions: ActionService::new(actions),
            }),
            close_reason: None,
        })
    }

    pub fn phase(&self) -> DesktopServicePhase {
        if self.active.is_some() {
            DesktopServicePhase::Active
        } else {
            DesktopServicePhase::Closed
        }
    }

    pub fn close_reason(&self) -> Option<DesktopServiceCloseReason> {
        self.close_reason
    }

    pub fn observer(&self) -> Result<&ObserverService, DesktopServicesUnavailable> {
        self.active
            .as_ref()
            .map(|services| &services.observer)
            .ok_or(DesktopServicesUnavailable)
    }

    pub fn observer_mut(&mut self) -> Result<&mut ObserverService, DesktopServicesUnavailable> {
        self.active
            .as_mut()
            .map(|services| &mut services.observer)
            .ok_or(DesktopServicesUnavailable)
    }

    pub fn actions(&self) -> Result<&ActionService, DesktopServicesUnavailable> {
        self.active
            .as_ref()
            .map(|services| &services.actions)
            .ok_or(DesktopServicesUnavailable)
    }

    pub fn actions_mut(&mut self) -> Result<&mut ActionService, DesktopServicesUnavailable> {
        self.active
            .as_mut()
            .map(|services| &mut services.actions)
            .ok_or(DesktopServicesUnavailable)
    }

    pub fn session_unavailable(&mut self) {
        self.close(DesktopServiceCloseReason::SessionUnavailable);
    }

    pub fn shutdown(&mut self) {
        self.close(DesktopServiceCloseReason::Requested);
    }

    fn close(&mut self, reason: DesktopServiceCloseReason) {
        if self.active.take().is_some() {
            self.close_reason = Some(reason);
        }
    }
}

fn map_observer_admission(error: observer::SessionAdmissionError) -> DesktopStartupError {
    match error {
        observer::SessionAdmissionError::Unauthenticated => DesktopStartupError::Unauthenticated,
        observer::SessionAdmissionError::UntrustedTransport => {
            DesktopStartupError::UntrustedTransport
        }
        observer::SessionAdmissionError::ContractMismatch => DesktopStartupError::ContractMismatch,
    }
}

fn map_action_admission(error: actions::SessionAdmissionError) -> DesktopStartupError {
    match error {
        actions::SessionAdmissionError::Unauthenticated => DesktopStartupError::Unauthenticated,
        actions::SessionAdmissionError::UntrustedTransport => {
            DesktopStartupError::UntrustedTransport
        }
        actions::SessionAdmissionError::ContractMismatch => DesktopStartupError::ContractMismatch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::SecurityKeyEvent;

    #[derive(Clone)]
    struct Session {
        package: &'static str,
        purpose: &'static str,
        role: &'static str,
        established: bool,
        authenticated: bool,
        pre_authorized: bool,
    }

    impl Default for Session {
        fn default() -> Self {
            Self {
                package: observer::SERVICE_PACKAGE,
                purpose: observer::ENDPOINT_PURPOSE,
                role: observer::ENDPOINT_ROLE,
                established: true,
                authenticated: true,
                pre_authorized: true,
            }
        }
    }

    impl EstablishedDesktopSession for Session {
        fn service_package(&self) -> &str {
            self.package
        }

        fn endpoint_purpose(&self) -> &str {
            self.purpose
        }

        fn endpoint_role(&self) -> &str {
            self.role
        }

        fn is_established(&self) -> bool {
            self.established
        }

        fn is_authenticated(&self) -> bool {
            self.authenticated
        }

        fn uses_pre_authorized_transport(&self) -> bool {
            self.pre_authorized
        }
    }

    #[test]
    fn starts_both_services_on_one_authenticated_boundary() {
        let mut services = DesktopServices::start(&Session::default(), 1_750_000_000).unwrap();

        services
            .observer_mut()
            .unwrap()
            .observe(
                SecurityKeyEvent::TouchNeeded {
                    session_id: "session-1".to_owned(),
                    vm_name: "personal".to_owned(),
                },
                1_750_000_000_000,
            )
            .unwrap();
        let offer = services
            .actions_mut()
            .unwrap()
            .offer_cancel("session-1", 1_750_000_000)
            .unwrap();

        assert_eq!(services.phase(), DesktopServicePhase::Active);
        assert_eq!(services.observer().unwrap().projection().active.len(), 1);
        assert_eq!(offer.kind, actions::ActionKind::CancelSecurityKeyCeremony);
    }

    #[test]
    fn startup_rejects_every_untrusted_session_shape() {
        let cases = [
            (
                Session {
                    established: false,
                    ..Session::default()
                },
                DesktopStartupError::SessionNotEstablished,
            ),
            (
                Session {
                    authenticated: false,
                    ..Session::default()
                },
                DesktopStartupError::Unauthenticated,
            ),
            (
                Session {
                    pre_authorized: false,
                    ..Session::default()
                },
                DesktopStartupError::UntrustedTransport,
            ),
            (
                Session {
                    package: "d2b.notify.v1",
                    ..Session::default()
                },
                DesktopStartupError::ContractMismatch,
            ),
            (
                Session {
                    purpose: "daemon-local",
                    ..Session::default()
                },
                DesktopStartupError::ContractMismatch,
            ),
            (
                Session {
                    role: "command-client",
                    ..Session::default()
                },
                DesktopStartupError::ContractMismatch,
            ),
        ];

        for (session, expected) in cases {
            assert_eq!(
                DesktopServices::start(&session, 1_750_000_000).unwrap_err(),
                expected
            );
        }
    }

    #[test]
    fn session_loss_drops_both_bounded_service_stores() {
        let mut services = DesktopServices::start(&Session::default(), 1_750_000_000).unwrap();
        services
            .actions_mut()
            .unwrap()
            .offer_cancel("session-1", 1_750_000_000)
            .unwrap();

        services.session_unavailable();

        assert_eq!(services.phase(), DesktopServicePhase::Closed);
        assert_eq!(
            services.close_reason(),
            Some(DesktopServiceCloseReason::SessionUnavailable)
        );
        assert_eq!(services.observer().unwrap_err(), DesktopServicesUnavailable);
        assert_eq!(services.actions().unwrap_err(), DesktopServicesUnavailable);

        services.shutdown();
        assert_eq!(
            services.close_reason(),
            Some(DesktopServiceCloseReason::SessionUnavailable)
        );
    }

    #[test]
    fn debug_output_contains_only_closed_lifecycle_state() {
        let mut services = DesktopServices::start(&Session::default(), 1_750_000_000).unwrap();
        let active = format!("{services:?}");
        assert_eq!(
            active,
            "DesktopServices { phase: Active, close_reason: None }"
        );

        services.shutdown();
        let closed = format!("{services:?}");
        assert_eq!(
            closed,
            "DesktopServices { phase: Closed, close_reason: Some(Requested) }"
        );
    }
}
