//! Hermetic test doubles and fixtures for `system-core`.
//!
//! Every double here is scripted. Nothing in this module resolves a local
//! account, reads an account database, or touches the machine the tests run
//! on, so the reconcilers can be exercised on any host as any user.

use std::future::Future;
use std::pin::pin;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::user::{OsUsername, UserSpec};

use crate::error::SystemCoreError;
use crate::user::{DiscoveredUser, UserBinding, UserDiscoveryEffectPort, UserIdentityDigest};

/// Drive a future to completion on the calling thread.
///
/// The suite is hermetic and never waits on I/O or wall time, so a
/// single-threaded driver is sufficient and keeps this crate free of an
/// async runtime dependency.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::hint::spin_loop(),
        }
    }
}

/// The canonical scripted identity digest.
pub const SCRIPTED_IDENTITY: UserIdentityDigest = UserIdentityDigest::from_bytes([0x22; 32]);

/// A scripted, recording [`UserDiscoveryEffectPort`].
#[derive(Debug)]
pub struct ScriptedDiscoveryPort {
    result: Option<DiscoveredUser>,
    error: Option<SystemCoreError>,
    calls: Mutex<u32>,
}

impl ScriptedDiscoveryPort {
    /// Script a port that resolves the User, verifying `verified`.
    pub fn resolving(verified: impl IntoIterator<Item = UserBinding>) -> Self {
        Self {
            result: Some(DiscoveredUser {
                identity: SCRIPTED_IDENTITY,
                observed: crate::user::UserObservation::from_verified(verified),
            }),
            error: None,
            calls: Mutex::new(0),
        }
    }

    /// Script a port that resolves nothing.
    pub fn absent() -> Self {
        Self {
            result: None,
            error: None,
            calls: Mutex::new(0),
        }
    }

    /// Script a port whose discovery fails.
    pub const fn failing(error: SystemCoreError) -> Self {
        Self {
            result: None,
            error: Some(error),
            calls: Mutex::new(0),
        }
    }

    /// How many times discovery was called.
    pub fn call_count(&self) -> u32 {
        self.calls.lock().map(|calls| *calls).unwrap_or_default()
    }
}

impl UserDiscoveryEffectPort for ScriptedDiscoveryPort {
    async fn discover(
        &self,
        _user_ref: &ResourceRef,
        _spec: &UserSpec,
    ) -> Result<Option<DiscoveredUser>, SystemCoreError> {
        if let Ok(mut calls) = self.calls.lock() {
            *calls += 1;
        }
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(self.result.clone())
    }
}

/// Canonical fixtures for the hermetic suite.
pub mod fixtures {
    use super::*;
    use d2b_contracts::v3::execution_policy::BoundedText;
    use d2b_contracts::v3::host::{HostSpec, IsolationPosture};
    use d2b_contracts::v3::user::OsGroupName;

    /// The canonical Host reference.
    pub fn host_ref() -> ResourceRef {
        ResourceRef::parse("Host/host-system").expect("valid fixture reference")
    }

    /// The canonical user-only Host reference.
    pub fn user_only_host_ref() -> ResourceRef {
        ResourceRef::parse("Host/personal").expect("valid fixture reference")
    }

    /// The canonical User reference.
    pub fn user_ref() -> ResourceRef {
        ResourceRef::parse("User/alice").expect("valid fixture reference")
    }

    /// The `Provider/system-core` reference every Host must declare.
    pub fn system_core_provider_ref() -> ResourceRef {
        ResourceRef::parse(crate::PROVIDER_REF).expect("the frozen provider reference is valid")
    }

    /// The minimal system Host spec.
    pub fn system_host_spec() -> HostSpec {
        HostSpec::system_default()
    }

    /// The user-only Host spec, carrying the explicit no-isolation posture.
    pub fn user_only_host_spec() -> HostSpec {
        let spec = HostSpec::user_only(user_ref()).expect("the user-only fixture spec is valid");
        debug_assert_eq!(
            spec.isolation_posture(),
            Some(IsolationPosture::NoIsolation),
            "the user-only fixture must carry the explicit posture"
        );
        spec
    }

    /// The declared OS username.
    ///
    /// It is deliberately different from the `User/alice` resource name, so
    /// a redaction assertion can tell the two apart. The split is the one
    /// the User primitive contract describes: `metadata.name` is the
    /// Zone-local key and `spec.osUsername` is what NSS resolves.
    pub const OS_USERNAME: &str = "alice_admin";

    /// A User spec declaring no additional groups.
    pub fn user_spec() -> UserSpec {
        UserSpec::minimal(OsUsername::parse(OS_USERNAME).expect("valid fixture username"))
    }

    /// A User spec declaring one additional group membership.
    pub fn user_spec_with_groups() -> UserSpec {
        UserSpec::new(
            OsUsername::parse(OS_USERNAME).expect("valid fixture username"),
            BoundedText::parse("").expect("empty text is always valid"),
            vec![OsGroupName::parse("wheel").expect("valid fixture group")],
        )
        .expect("the grouped fixture spec is valid")
    }
}
