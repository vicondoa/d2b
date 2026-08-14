//! Common long-lived Provider entrypoint lifecycle.
//!
//! Provider binaries are supervised children.  They must publish readiness
//! only after their local service registration has completed, remain alive
//! while the supervisor owns them, and stop admitting work before they drain.

use std::{
    fmt,
    io::{self, Write},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use d2b_contracts::v3::ResourceRef;
use d2b_session::{AuthenticatedComponentSession, AuthenticatedSessionRouteBinding};

const STARTING: u8 = 0;
const READY: u8 = 1;
const DRAINING: u8 = 2;
const STOPPED: u8 = 3;

/// A bounded Provider lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderLifecycle {
    /// Local service registration is still in progress.
    Starting,
    /// The service has completed registration and accepts work.
    Ready,
    /// New work is refused while admitted work drains.
    Draining,
    /// The process has completed its drain.
    Stopped,
}

impl ProviderLifecycle {
    const fn from_u8(value: u8) -> Self {
        match value {
            READY => Self::Ready,
            DRAINING => Self::Draining,
            STOPPED => Self::Stopped,
            _ => Self::Starting,
        }
    }
}

/// Stable failures encountered while bootstrapping a Provider process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderRuntimeError {
    /// The Provider name is empty or exceeds the wire bound.
    InvalidName,
    /// Readiness was requested after the process began draining.
    NotAccepting,
    /// The readiness announcement could not be written.
    ReadinessIo,
    /// The Provider has no authenticated ComponentSession route.
    SessionUnauthenticated,
    /// The generated Provider service loop failed after readiness.
    SessionLoopFailed,
}

impl fmt::Display for ProviderRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidName => "provider-runtime-name-invalid",
            Self::NotAccepting => "provider-runtime-not-accepting",
            Self::ReadinessIo => "provider-runtime-readiness-io",
            Self::SessionUnauthenticated => "provider-runtime-session-unauthenticated",
            Self::SessionLoopFailed => "provider-runtime-session-loop-failed",
        })
    }
}

impl std::error::Error for ProviderRuntimeError {}

struct RuntimeState {
    admitted: usize,
}

/// A non-cloneable process lifecycle owner.
///
/// The owner is deliberately small and transport-neutral.  Generated
/// ComponentSession servers own request admission; this type owns only the
/// process boundary and readiness/drain handshake around them.
pub struct ProviderEntrypoint {
    name: &'static str,
    provider_ref: Option<ResourceRef>,
    service: Option<&'static str>,
    lifecycle: AtomicU8,
    state: Arc<(Mutex<RuntimeState>, Condvar)>,
}

/// A non-authorizing admission proof derived from one authenticated
/// ComponentSession route.
///
/// This proof carries only redacted routing metadata and is consumed when the
/// entrypoint publishes readiness. It cannot be constructed from a subject,
/// Provider name, or Zone string.
pub struct ProviderSessionAdmission {
    route: AuthenticatedSessionRouteBinding,
}

impl fmt::Debug for ProviderSessionAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderSessionAdmission(REDACTED)")
    }
}

impl fmt::Debug for ProviderEntrypoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderEntrypoint")
            .field("name", &self.name)
            .field(
                "provider_ref",
                &self.provider_ref.as_ref().map(|_| "<redacted>"),
            )
            .field("service", &self.service.map(|_| "<redacted>"))
            .field("lifecycle", &self.lifecycle())
            .finish()
    }
}

impl ProviderEntrypoint {
    /// Construct a process lifecycle owner for one fixed Provider binary.
    pub fn new(name: &'static str) -> Result<Self, ProviderRuntimeError> {
        if name.is_empty() || name.len() > 128 || !name.is_ascii() {
            return Err(ProviderRuntimeError::InvalidName);
        }
        Ok(Self {
            name,
            provider_ref: None,
            service: None,
            lifecycle: AtomicU8::new(STARTING),
            state: Arc::new((Mutex::new(RuntimeState { admitted: 0 }), Condvar::new())),
        })
    }

    /// Construct a lifecycle owner bound to one compiled Provider identity
    /// and service package.
    pub fn with_provider(
        name: &'static str,
        provider_ref: ResourceRef,
        service: &'static str,
    ) -> Result<Self, ProviderRuntimeError> {
        let mut runtime = Self::new(name)?;
        if provider_ref.resource_type().as_str() != "Provider" || service.is_empty() {
            return Err(ProviderRuntimeError::InvalidName);
        }
        runtime.provider_ref = Some(provider_ref);
        runtime.service = Some(service);
        Ok(runtime)
    }

    /// Return the current process lifecycle.
    pub fn lifecycle(&self) -> ProviderLifecycle {
        ProviderLifecycle::from_u8(self.lifecycle.load(Ordering::Acquire))
    }

    /// Admit one local service registration.
    pub fn admit(&self) -> Result<ProviderAdmission, ProviderRuntimeError> {
        let (lock, _) = &*self.state;
        let mut state = lock
            .lock()
            .map_err(|_| ProviderRuntimeError::NotAccepting)?;
        // Drain takes the lifecycle transition before it waits on this lock.
        // Checking only before locking would let a registration slip into a
        // draining process after the supervisor had fenced new work.
        if self.lifecycle() != ProviderLifecycle::Starting {
            return Err(ProviderRuntimeError::NotAccepting);
        }
        state.admitted = state.admitted.saturating_add(1);
        Ok(ProviderAdmission {
            state: Arc::clone(&self.state),
        })
    }

    /// Publish process readiness after all local registrations have completed.
    ///
    /// This is kept private to the lifecycle module so an embedded Provider
    /// cannot bypass authenticated ComponentSession admission. Production
    /// callers must use [`Self::publish_authenticated_ready`].
    #[cfg(test)]
    fn publish_ready(&self) -> Result<(), ProviderRuntimeError> {
        let (lock, _) = &*self.state;
        let state = lock
            .lock()
            .map_err(|_| ProviderRuntimeError::NotAccepting)?;
        if state.admitted == 0 {
            return Err(ProviderRuntimeError::NotAccepting);
        }
        self.transition_ready()?;
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "D2B_PROVIDER_READY {}", self.name)
            .and_then(|()| stdout.flush())
            .map_err(|_| ProviderRuntimeError::ReadinessIo)
    }

    /// Derive a route-bound session admission from an authenticated session.
    pub fn admit_authenticated<C>(
        &self,
        session: &AuthenticatedComponentSession<C>,
    ) -> Result<ProviderSessionAdmission, ProviderRuntimeError> {
        let Some(expected_provider) = &self.provider_ref else {
            return Err(ProviderRuntimeError::SessionUnauthenticated);
        };
        let Some(expected_service) = self.service else {
            return Err(ProviderRuntimeError::SessionUnauthenticated);
        };
        let route = session.route_binding();
        if route.provider_ref() != Some(expected_provider)
            || route.service().as_str() != expected_service
            || route.provider_generation().is_none()
            || route.reconnect_generation().get() == 0
        {
            return Err(ProviderRuntimeError::SessionUnauthenticated);
        }
        Ok(ProviderSessionAdmission { route })
    }

    /// Publish readiness only after both local registration and authenticated
    /// ComponentSession route admission have succeeded.
    pub fn publish_authenticated_ready<C>(
        &self,
        registration: &ProviderAdmission,
        session: ProviderSessionAdmission,
        live_session: &AuthenticatedComponentSession<C>,
    ) -> Result<(), ProviderRuntimeError> {
        let live_route = live_session.route_binding();
        self.validate_authenticated_ready(registration, &session, &live_route)?;
        drop(session);
        self.transition_ready()?;
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "D2B_PROVIDER_READY {}", self.name)
            .and_then(|()| stdout.flush())
            .map_err(|_| ProviderRuntimeError::ReadinessIo)
    }

    fn validate_authenticated_ready(
        &self,
        registration: &ProviderAdmission,
        session: &ProviderSessionAdmission,
        live_route: &AuthenticatedSessionRouteBinding,
    ) -> Result<(), ProviderRuntimeError> {
        let (lock, _) = &*self.state;
        let state = lock
            .lock()
            .map_err(|_| ProviderRuntimeError::NotAccepting)?;
        if !Arc::ptr_eq(&registration.state, &self.state)
            || state.admitted == 0
            || session.route.reconnect_generation().get() == 0
            || session.route != *live_route
        {
            return Err(ProviderRuntimeError::SessionUnauthenticated);
        }
        Ok(())
    }

    fn transition_ready(&self) -> Result<(), ProviderRuntimeError> {
        if self
            .lifecycle
            .compare_exchange(STARTING, READY, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ProviderRuntimeError::NotAccepting);
        }
        Ok(())
    }

    /// Stop accepting registrations and wait for local registrations to drain.
    pub fn drain(&self, timeout: Duration) -> bool {
        let prior = self.lifecycle.swap(DRAINING, Ordering::AcqRel);
        if prior == STOPPED {
            return true;
        }
        let (lock, idle) = &*self.state;
        let guard = lock.lock();
        let Ok(mut state) = guard else {
            return false;
        };
        let result = idle
            .wait_timeout_while(state, timeout, |state| state.admitted != 0)
            .ok();
        let Some((new_state, wait)) = result else {
            return false;
        };
        state = new_state;
        let drained = state.admitted == 0 && !wait.timed_out();
        if drained {
            self.lifecycle.store(STOPPED, Ordering::Release);
        }
        drained
    }
}

/// One local registration held until its service is fully drained.
pub struct ProviderAdmission {
    state: Arc<(Mutex<RuntimeState>, Condvar)>,
}

impl fmt::Debug for ProviderAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderAdmission(REDACTED)")
    }
}

impl Drop for ProviderAdmission {
    fn drop(&mut self) {
        let (lock, idle) = &*self.state;
        if let Ok(mut state) = lock.lock() {
            state.admitted = state.admitted.saturating_sub(1);
            if state.admitted == 0 {
                idle.notify_all();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::ResourceRef;

    #[test]
    fn readiness_is_not_published_before_registration() {
        let runtime = ProviderEntrypoint::new("Provider/test").unwrap();
        assert_eq!(runtime.lifecycle(), ProviderLifecycle::Starting);
        let admission = runtime.admit().unwrap();
        assert!(runtime.publish_ready().is_ok());
        assert_eq!(runtime.lifecycle(), ProviderLifecycle::Ready);
        drop(admission);
        assert!(runtime.drain(Duration::from_millis(10)));
        assert_eq!(runtime.lifecycle(), ProviderLifecycle::Stopped);
    }

    #[test]
    fn draining_refuses_new_registration() {
        let runtime = ProviderEntrypoint::new("Provider/test").unwrap();
        let admission = runtime.admit().unwrap();
        runtime.publish_ready().unwrap();
        assert!(!runtime.drain(Duration::from_millis(10)));
        assert_eq!(
            runtime.admit().unwrap_err().to_string(),
            "provider-runtime-not-accepting"
        );
        drop(admission);
        assert!(runtime.drain(Duration::from_millis(10)));
    }

    #[test]
    fn authenticated_readiness_requires_the_live_route_and_registration() {
        let runtime = ProviderEntrypoint::with_provider(
            "Provider/test",
            ResourceRef::parse("Provider/test").unwrap(),
            "d2b.provider.v3",
        )
        .unwrap();
        let registration = runtime.admit().unwrap();
        let route = AuthenticatedSessionRouteBinding::for_test(
            Some(ResourceRef::parse("Provider/test").unwrap()),
            "d2b.provider.v3",
            1,
            Some(1),
            Some(1),
        );
        let admission = ProviderSessionAdmission {
            route: route.clone(),
        };
        assert!(
            runtime
                .validate_authenticated_ready(&registration, &admission, &route)
                .is_ok()
        );

        let mismatched = AuthenticatedSessionRouteBinding::for_test(
            Some(ResourceRef::parse("Provider/test").unwrap()),
            "d2b.other.v3",
            1,
            Some(1),
            Some(1),
        );
        assert_eq!(
            runtime.validate_authenticated_ready(&registration, &admission, &mismatched,),
            Err(ProviderRuntimeError::SessionUnauthenticated)
        );
        let other_runtime = ProviderEntrypoint::new("Provider/other").unwrap();
        let other_registration = other_runtime.admit().unwrap();
        assert_eq!(
            runtime.validate_authenticated_ready(&other_registration, &admission, &route),
            Err(ProviderRuntimeError::SessionUnauthenticated)
        );
    }
}
