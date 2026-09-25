//! Test-support doubles for the User family's two seams.
//!
//! - [`RecordingEffects`], the scripted double for the driver's typed
//!   [`UserDriverEffects`] seam: records every discovery call
//!   order-preservingly and can script the reported phase or a probe
//!   refusal.
//! - [`ScriptedProbe`], the scripted double for the
//!   [`UserDiscoveryEffectPort`] seam the crate's production probe
//!   implements: resolves every declared identity as discovered, deriving
//!   the opaque identity digest from the declared identity material the
//!   way the production probe does, records the requested usernames, and
//!   can script an absent account or a discovery refusal.
//!
//! [`recording_facets`] builds the declared facet set over a scripted
//! probe, exactly as the production composition root builds it over the
//! crate's own probe, so the plane tests script the same boundary
//! production composes.
//!
//! Gated behind the `test-support` Cargo feature (available automatically
//! under `cargo test`), so production consumers never pull it in. The plane
//! tests in `d2bd` reach it through the same public surface.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use d2b_contracts_resource::v3::{ResourcePhase, ResourceRef};
use d2b_contracts_resource::v3::user::{OsUsername, UserSpec};
use d2b_provider_system_core::{
    DiscoveredUser, SystemCoreError, UserBinding, UserDiscoveryCondition, UserDiscoveryEffectPort,
    UserIdentityDigest, UserObservation, UserStatusReport,
};

use crate::driver::UserDriverEffects;
use crate::facets::UserEffectFacets;

// -- the driver seam ---------------------------------------------------------

/// Scripted discovery port: records every call order-preservingly and can
/// fail discovery.
pub struct RecordingEffects {
    calls: parking_lot::Mutex<Vec<String>>,
    phase: parking_lot::Mutex<ResourcePhase>,
    /// Script whether the next discovery refuses.
    pub fail: AtomicBool,
}

impl RecordingEffects {
    /// Construct a fresh double with the default Ready phase and no recorded
    /// calls.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: parking_lot::Mutex::new(Vec::new()),
            phase: parking_lot::Mutex::new(ResourcePhase::Ready),
            fail: AtomicBool::new(false),
        })
    }

    /// The observed call labels in arrival order.
    pub fn call_order(&self) -> Vec<String> {
        self.calls.lock().clone()
    }

    /// Script the phase the next discovery reports.
    pub fn set_phase(&self, phase: ResourcePhase) {
        *self.phase.lock() = phase;
    }
}

#[async_trait::async_trait]
impl UserDriverEffects for RecordingEffects {
    async fn observe_user(
        &self,
        user_ref: &ResourceRef,
        _spec: &UserSpec,
    ) -> Result<UserStatusReport, String> {
        self.calls.lock().push("observe-user".to_owned()); // async-gate-allow: test-support recorder lock
        if self.fail.load(Ordering::Relaxed) {
            return Err("the scripted discovery refused".to_owned());
        }
        Ok(UserStatusReport {
            user_ref: user_ref.clone(),
            provider: "system-core",
            phase: *self.phase.lock(), // async-gate-allow: test-support recorder lock
            discovery: UserDiscoveryCondition::Discovered,
            identity: None,
        })
    }
}

// -- the discovery-port seam -------------------------------------------------

/// Scripted [`UserDiscoveryEffectPort`]: resolves every declared identity
/// as discovered with the opaque identity digest derived from the declared
/// identity material the way the production probe does - the fixed base
/// digest for a group-free identity, extended with every declared group -
/// and verifies the record and primary group but never a declared group
/// membership, so a spec that declares groups classifies as drifted, like
/// a machine whose declared memberships do not verify. Records every
/// requested username order-preservingly, and can script an absent account
/// or a discovery refusal. The scripted state lives behind an `Arc`, so a
/// test can keep a handle and script the service-held probe.
pub struct ScriptedProbe {
    core: Arc<ScriptedCore>,
}

struct ScriptedCore {
    /// The usernames discovery was requested for, in arrival order.
    calls: parking_lot::Mutex<Vec<OsUsername>>,
    /// Whether the next discovery resolves no local record.
    absent: AtomicBool,
    /// Whether the next discovery refuses.
    failing: AtomicBool,
}

impl ScriptedProbe {
    /// Construct a fresh double that resolves every declared identity as
    /// discovered.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            core: Arc::new(ScriptedCore {
                calls: parking_lot::Mutex::new(Vec::new()),
                absent: AtomicBool::new(false),
                failing: AtomicBool::new(false),
            }),
        })
    }

    /// The usernames discovery was requested for, in arrival order.
    pub fn discovered_names(&self) -> Vec<OsUsername> {
        self.core.calls.lock().clone()
    }

    /// Script whether the next discovery resolves no local record.
    pub fn set_absent(&self, absent: bool) {
        self.core.absent.store(absent, Ordering::Relaxed);
    }

    /// Script whether the next discovery refuses.
    pub fn set_failing(&self, failing: bool) {
        self.core.failing.store(failing, Ordering::Relaxed);
    }
}

#[async_trait::async_trait]
impl UserDiscoveryEffectPort for ScriptedProbe {
    async fn discover(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> Result<Option<DiscoveredUser>, SystemCoreError> {
        self.core.calls.lock().push(spec.os_username().clone()); // async-gate-allow: test-support recorder lock
        if self.core.failing.load(Ordering::Relaxed) {
            return Err(SystemCoreError::DiscoveryUnavailable);
        }
        if self.core.absent.load(Ordering::Relaxed) {
            return Ok(None);
        }
        Ok(Some(DiscoveredUser {
            identity: scripted_identity(user_ref, spec),
            observed: UserObservation::from_verified([
                UserBinding::NssRecord,
                UserBinding::PrimaryGroup,
            ]),
        }))
    }
}

/// The scripted identity for one declared spec: the fixed base digest for
/// a group-free identity, extended with every declared group under the
/// scripted domain - the declared group names are identity material the
/// production probe folds into its digest, so the scripted digest must
/// track them too, or a group-declaring identity would share a group-free
/// identity's digest and the group half of the declared identity could
/// never be observed.
fn scripted_identity(user_ref: &ResourceRef, spec: &UserSpec) -> UserIdentityDigest {
    if spec.groups().is_empty() {
        return UserIdentityDigest::from_bytes([0x5a; 32]);
    }
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"scripted-user-v1");
    hasher.update(user_ref.name().as_str().as_bytes());
    hasher.update([0]);
    hasher.update(spec.os_username().as_str().as_bytes());
    for group in spec.groups() {
        hasher.update([0]);
        hasher.update(group.as_str().as_bytes());
    }
    UserIdentityDigest::from_bytes(hasher.finalize().into())
}

// -- the declared facet seam -------------------------------------------------

/// Build the user family's declared facet set over one scripted probe,
/// exactly as the production composition root builds it over the crate's
/// own probe: composition and scripting cross the same
/// [`UserDiscoveryEffectPort`] boundary.
pub fn recording_facets(probe: Arc<ScriptedProbe>) -> UserEffectFacets {
    UserEffectFacets { probe }
}