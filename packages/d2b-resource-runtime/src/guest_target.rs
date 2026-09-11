//! Guest-side target runtime (spec sections 23.2-23.4; R18, R19, R21).
//!
//! A Host-zone resource that targets a Guest is realized by target-local
//! effect code inside that Guest. The Guest-side bookkeeping for those
//! realizations lives here:
//!
//! - [`TargetResourceInstance`] is the spec section 23.2 target-local object.
//!   It is an implementation detail of the target runtime: it has no desired
//!   spec of its own, is never exposed through the Guest's Resource API, and
//!   never enters the Guest resource namespace as a second authoritative
//!   resource.
//! - [`GuestTargetControl`] is the target-control port the Host side reaches
//!   through the authenticated ComponentSession (spec section 23.3). The
//!   Host-side implementation of this port is the composition's
//!   session-backed client; the Guest-side implementation is
//!   [`SessionBoundGuestTargetControl`].
//! - [`SessionBoundGuestTargetControl`] is one capability bound to one
//!   authenticated ComponentSession generation. Every request it carries is
//!   fenced by that generation, so a session that reconnected at a newer
//!   generation (or an old capability retained across a reconnect) can never
//!   realize, observe, or delete on behalf of its successor.

pub const MODULE_NAME: &str = "guest_target";

use std::{collections::HashMap, fmt, sync::Arc};

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::identity::ResourceKey;
use crate::target::TargetRef;

/// Target-local realization phase of one Host-zone resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetInstanceState {
    /// The target-local effect is still converging on the desired state.
    Realizing,
    /// The target-local effect is realized and serving.
    Ready,
}

impl TargetInstanceState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Realizing => "realizing",
            Self::Ready => "ready",
        }
    }
}

/// One target-local realization (spec section 23.2).
///
/// Keyed by the owning Host-zone resource identity: [`Self::source`] is the
/// durable key of the resource in its authority Zone, never a guest-local
/// name, so a Guest never learns a second identity for the resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetResourceInstance {
    source: ResourceKey,
    source_uid: [u8; 16],
    assignment_generation: u64,
    session_generation: u64,
    local_handle: String,
    state: TargetInstanceState,
}

impl TargetResourceInstance {
    /// Borrow the owning Host-zone resource key.
    pub const fn source(&self) -> &ResourceKey {
        &self.source
    }

    /// The owning resource's stable uid.
    pub const fn source_uid(&self) -> &[u8; 16] {
        &self.source_uid
    }

    /// The desired generation this realization was minted for.
    pub const fn assignment_generation(&self) -> u64 {
        self.assignment_generation
    }

    /// The session generation that created or adopted this realization.
    pub const fn session_generation(&self) -> u64 {
        self.session_generation
    }

    /// The opaque target-local handle (socket path, pid, mount identity)
    /// minted by target-local effect code. It is never a Host path.
    pub fn local_handle(&self) -> &str {
        &self.local_handle
    }

    /// Current target-local phase.
    pub const fn state(&self) -> TargetInstanceState {
        self.state
    }
}

/// Outcome of target-local discovery for one source (F2, F5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestAdoption {
    /// A target-local realization was present and is re-adopted under the
    /// current session generation. It is the same realization: adoption never
    /// mints a second one.
    Adopted(TargetResourceInstance),
    /// Nothing is realized on the target; the Host-owned actor realizes it.
    Missing,
}

impl GuestAdoption {
    /// The adopted realization, when discovery found one.
    pub const fn instance(&self) -> Option<&TargetResourceInstance> {
        match self {
            Self::Adopted(instance) => Some(instance),
            Self::Missing => None,
        }
    }
}

/// One realize request across the ComponentSession target-control boundary.
///
/// The request names the owning Host-zone resource and the desired generation
/// it belongs to; the target-local handle is chosen by target-local effect
/// code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestRealizeRequest {
    source: ResourceKey,
    source_uid: [u8; 16],
    assignment_generation: u64,
    local_handle: String,
}

impl GuestRealizeRequest {
    /// Construct one realize request.
    pub fn new(
        source: ResourceKey,
        source_uid: [u8; 16],
        assignment_generation: u64,
        local_handle: impl Into<String>,
    ) -> Self {
        Self { source, source_uid, assignment_generation, local_handle: local_handle.into() }
    }

    /// Borrow the owning Host-zone resource key.
    pub const fn source(&self) -> &ResourceKey {
        &self.source
    }

    /// The owning resource's stable uid.
    pub const fn source_uid(&self) -> &[u8; 16] {
        &self.source_uid
    }

    /// The desired generation this realization is minted for.
    pub const fn assignment_generation(&self) -> u64 {
        self.assignment_generation
    }

    /// Borrow the target-local handle.
    pub fn local_handle(&self) -> &str {
        &self.local_handle
    }
}

/// The Guest-side target-control port (spec section 23.3).
///
/// One implementation serves one authenticated ComponentSession generation.
/// A stale generation is refused by the implementation, never by the caller,
/// and callers never see a target-local handle they did not send.
#[async_trait]
pub trait GuestTargetControl: Send + Sync + fmt::Debug + 'static {
    /// Realize (create or update) the target-local instance for one resource.
    /// Realization is idempotent per source: a second request for the same
    /// source updates the same instance and never mints a second one.
    async fn realize(
        &self,
        request: GuestRealizeRequest,
    ) -> Result<TargetResourceInstance, GuestTargetError>;

    /// Observe the target-local instance for one resource, when present.
    async fn observe(
        &self,
        source: &ResourceKey,
    ) -> Result<Option<TargetResourceInstance>, GuestTargetError>;

    /// Delete the target-local instance for one resource. Reports whether an
    /// instance was present.
    async fn delete(&self, source: &ResourceKey) -> Result<bool, GuestTargetError>;

    /// Target-local discovery and adoption for the named sources (F5). This
    /// is the only operation accepted across a generation change: it re-binds
    /// existing realizations to the current session instead of inheriting an
    /// older session's authority.
    async fn adopt(&self, sources: &[ResourceKey]) -> Result<Vec<GuestAdoption>, GuestTargetError>;
}

#[derive(Debug)]
struct GuestTargetState {
    reference: TargetRef,
    session_generation: Option<u64>,
    instances: HashMap<ResourceKey, TargetResourceInstance>,
}

/// The Guest-side target runtime.
///
/// One runtime per Guest. It owns the target-local realizations of resources
/// whose authority stays in the parent Host Zone; it owns no desired spec, no
/// store, and no resource namespace.
#[derive(Debug, Clone)]
pub struct GuestTargetRuntime {
    inner: Arc<Mutex<GuestTargetState>>,
}

impl GuestTargetRuntime {
    pub fn new(reference: TargetRef) -> Self {
        Self {
            inner: Arc::new(Mutex::new(GuestTargetState {
                reference,
                session_generation: None,
                instances: HashMap::new(),
            })),
        }
    }

    /// The Guest this runtime realizes for.
    pub fn reference(&self) -> TargetRef {
        self.inner.lock().reference.clone()
    }

    /// The authenticated ComponentSession generation currently bound.
    pub fn session_generation(&self) -> Option<u64> {
        self.inner.lock().session_generation
    }

    /// Bind the authenticated session generation of the parent
    /// ComponentSession. A generation older than the live one is refused: a
    /// reconnect can never inherit an older session's authority, and binding
    /// the same generation again is a no-op.
    pub fn bind_session(&self, session_generation: u64) -> Result<(), GuestTargetError> {
        if session_generation == 0 {
            return Err(GuestTargetError::SessionUnavailable);
        }
        let mut state = self.inner.lock();
        if let Some(live) = state.session_generation {
            if session_generation < live {
                return Err(GuestTargetError::SessionGenerationRegression);
            }
            if session_generation == live {
                return Ok(());
            }
        }
        state.session_generation = Some(session_generation);
        Ok(())
    }

    /// Every target-local realization this runtime holds, in identity order.
    pub fn instances(&self) -> Vec<TargetResourceInstance> {
        let mut instances: Vec<TargetResourceInstance> =
            self.inner.lock().instances.values().cloned().collect();
        instances.sort_by(|left, right| identity_order(&left.source).cmp(&identity_order(&right.source)));
        instances
    }

    /// The target-local realization for one source, when present.
    pub fn instance(&self, source: &ResourceKey) -> Option<TargetResourceInstance> {
        self.inner.lock().instances.get(source).cloned()
    }

    /// A target-control capability bound to one authenticated generation.
    ///
    /// Fails closed: the generation must be the live one, so an old session
    /// cannot mint a capability for a newer one.
    pub fn control(
        self: &Arc<Self>,
        session_generation: u64,
    ) -> Result<Arc<dyn GuestTargetControl>, GuestTargetError> {
        SessionBoundGuestTargetControl::new(Arc::clone(self), session_generation)
    }

    fn require_generation(&self, session_generation: u64) -> Result<(), GuestTargetError> {
        match self.inner.lock().session_generation {
            Some(live) if live == session_generation => Ok(()),
            Some(_) => Err(GuestTargetError::StaleSessionGeneration),
            None => Err(GuestTargetError::SessionUnavailable),
        }
    }

    fn realize(
        &self,
        session_generation: u64,
        request: GuestRealizeRequest,
    ) -> Result<TargetResourceInstance, GuestTargetError> {
        self.require_generation(session_generation)?;
        let mut state = self.inner.lock();
        let instance = TargetResourceInstance {
            source: request.source.clone(),
            source_uid: request.source_uid,
            assignment_generation: request.assignment_generation,
            session_generation,
            local_handle: request.local_handle,
            state: TargetInstanceState::Realizing,
        };
        state.instances.insert(request.source, instance.clone());
        Ok(instance)
    }

    fn observe(
        &self,
        session_generation: u64,
        source: &ResourceKey,
    ) -> Result<Option<TargetResourceInstance>, GuestTargetError> {
        self.require_generation(session_generation)?;
        Ok(self.inner.lock().instances.get(source).cloned())
    }

    fn delete(
        &self,
        session_generation: u64,
        source: &ResourceKey,
    ) -> Result<bool, GuestTargetError> {
        self.require_generation(session_generation)?;
        Ok(self.inner.lock().instances.remove(source).is_some())
    }

    fn adopt(
        &self,
        session_generation: u64,
        sources: &[ResourceKey],
    ) -> Result<Vec<GuestAdoption>, GuestTargetError> {
        self.require_generation(session_generation)?;
        let mut state = self.inner.lock();
        let mut adopted = Vec::with_capacity(sources.len());
        for source in sources {
            match state.instances.get_mut(source) {
                Some(instance) => {
                    instance.session_generation = session_generation;
                    adopted.push(GuestAdoption::Adopted(instance.clone()));
                }
                None => adopted.push(GuestAdoption::Missing),
            }
        }
        Ok(adopted)
    }

    /// Advance one target-local realization to ready (the target-local effect
    /// code calls this when its effect is serving).
    pub fn mark_ready(&self, source: &ResourceKey) -> Option<TargetResourceInstance> {
        let mut state = self.inner.lock();
        let instance = state.instances.get_mut(source)?;
        instance.state = TargetInstanceState::Ready;
        Some(instance.clone())
    }
}

/// One ComponentSession generation's capability over a [`GuestTargetRuntime`].
///
/// The generation is captured when the capability is minted and validated on
/// every operation, so a capability retained across a reconnect stops working
/// instead of acting for the new session.
#[derive(Debug)]
pub struct SessionBoundGuestTargetControl {
    runtime: Arc<GuestTargetRuntime>,
    session_generation: u64,
}

impl SessionBoundGuestTargetControl {
    /// Mint a capability for one authenticated session generation.
    pub fn new(
        runtime: Arc<GuestTargetRuntime>,
        session_generation: u64,
    ) -> Result<Arc<dyn GuestTargetControl>, GuestTargetError> {
        runtime.require_generation(session_generation)?;
        Ok(Arc::new(Self { runtime, session_generation }))
    }

    /// The session generation this capability is bound to.
    pub const fn session_generation(&self) -> u64 {
        self.session_generation
    }
}

#[async_trait]
impl GuestTargetControl for SessionBoundGuestTargetControl {
    async fn realize(
        &self,
        request: GuestRealizeRequest,
    ) -> Result<TargetResourceInstance, GuestTargetError> {
        self.runtime.realize(self.session_generation, request)
    }

    async fn observe(
        &self,
        source: &ResourceKey,
    ) -> Result<Option<TargetResourceInstance>, GuestTargetError> {
        self.runtime.observe(self.session_generation, source)
    }

    async fn delete(&self, source: &ResourceKey) -> Result<bool, GuestTargetError> {
        self.runtime.delete(self.session_generation, source)
    }

    async fn adopt(&self, sources: &[ResourceKey]) -> Result<Vec<GuestAdoption>, GuestTargetError> {
        self.runtime.adopt(self.session_generation, sources)
    }
}

/// Closed Guest target-control failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestTargetError {
    /// No authenticated session generation is bound.
    SessionUnavailable,
    /// The request belongs to a generation that is not the live one.
    StaleSessionGeneration,
    /// A reconnect carried a generation older than the live one.
    SessionGenerationRegression,
}

impl fmt::Display for GuestTargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SessionUnavailable => "guest-target-session-unavailable",
            Self::StaleSessionGeneration => "guest-target-stale-session-generation",
            Self::SessionGenerationRegression => "guest-target-session-generation-regression",
        })
    }
}

impl std::error::Error for GuestTargetError {}

/// Stable ordering for resource identities inside a target.
fn identity_order(key: &ResourceKey) -> (&str, &str, &str) {
    (&key.zone, &key.type_name, &key.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guest() -> TargetRef {
        TargetRef::guest("work-vm").expect("guest ref")
    }

    fn source(name: &str) -> ResourceKey {
        ResourceKey::new("work", "Process", name)
    }

    #[tokio::test]
    async fn realization_is_idempotent_per_source_and_never_a_second_resource() {
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        runtime.bind_session(1).expect("bind session");
        let control = runtime.control(1).expect("control");

        let first = control
            .realize(GuestRealizeRequest::new(source("foo"), [7; 16], 3, "/run/d2b/foo.sock"))
            .await
            .expect("realize");
        let second = control
            .realize(GuestRealizeRequest::new(source("foo"), [7; 16], 4, "/run/d2b/foo.sock"))
            .await
            .expect("realize again");

        assert_eq!(runtime.instances().len(), 1, "one realization per host-owned resource");
        assert_eq!(first.source(), second.source());
        assert_eq!(second.assignment_generation(), 4, "the same instance tracks the new desired generation");
        assert_eq!(second.source().zone, "work", "the guest keeps the authority-zone identity");
    }

    #[tokio::test]
    async fn a_session_bound_capability_stops_working_after_reconnect() {
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        runtime.bind_session(1).expect("bind session");
        let old = runtime.control(1).expect("control");
        old.realize(GuestRealizeRequest::new(source("foo"), [7; 16], 1, "/run/d2b/foo.sock"))
            .await
            .expect("realize");

        runtime.bind_session(2).expect("reconnect generation");
        assert_eq!(
            old.observe(&source("foo")).await.err(),
            Some(GuestTargetError::StaleSessionGeneration),
            "a capability retained across a reconnect cannot act for the new session"
        );
        let current = runtime.control(2).expect("control");
        assert!(current.observe(&source("foo")).await.expect("observe").is_some());
    }

    #[tokio::test]
    async fn adoption_rebinds_present_realizations_and_reports_missing_sources() {
        let runtime = Arc::new(GuestTargetRuntime::new(guest()));
        runtime.bind_session(1).expect("bind session");
        runtime
            .control(1)
            .expect("control")
            .realize(GuestRealizeRequest::new(source("foo"), [7; 16], 1, "/run/d2b/foo.sock"))
            .await
            .expect("realize");
        runtime.bind_session(2).expect("reconnect generation");

        let adopted = runtime.control(2).expect("control").adopt(&[source("foo"), source("gone")]).await.expect("adopt");

        match &adopted[0] {
            GuestAdoption::Adopted(instance) => {
                assert_eq!(instance.session_generation(), 2);
                assert_eq!(instance.local_handle(), "/run/d2b/foo.sock");
            }
            GuestAdoption::Missing => panic!("a present realization is adopted, not recreated"),
        }
        assert_eq!(adopted[1], GuestAdoption::Missing);
        assert_eq!(runtime.instances().len(), 1);
    }

    #[test]
    fn session_binding_refuses_a_generation_regression() {
        let runtime = GuestTargetRuntime::new(guest());
        runtime.bind_session(2).expect("bind session");
        assert_eq!(runtime.bind_session(2), Ok(()), "rebinding the live generation is a no-op");
        assert_eq!(
            runtime.bind_session(1),
            Err(GuestTargetError::SessionGenerationRegression)
        );
        assert_eq!(runtime.session_generation(), Some(2));
    }
}
