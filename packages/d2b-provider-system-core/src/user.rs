//! Local User discovery and status.
//!
//! `Provider/system-core` owns local User discovery and status
//! (`ADR-046-provider-model-and-packaging`, section "system-core
//! bootstrap"). A `User` resource names an identity; discovery answers
//! whether the local machine actually resolves it, and status says so
//! without ever restating what was resolved.
//!
//! The Provider does not call NSS. Resolution is a host effect, so it goes
//! through the injected [`UserDiscoveryEffectPort`], exactly as the Process
//! Providers reach a launch only through their own effect port. The port
//! returns opaque evidence: a digest standing for the resolved identity and
//! the closed set of properties it actually verified. No uid, gid, home
//! directory, shell, or resolved username crosses back into Provider code,
//! so none of it can reach status, a log, or an audit record.
//!
//! Verification is the same shape the process paths use, and for the same
//! reason. A partially verified identity is ambiguity, and ambiguity is
//! reported as drift or as unverified rather than being treated as the
//! declared User. Adapted from the daemon supervisor's
//! `(pid, start_time_ticks)` recheck in
//! `packages/d2bd/src/supervisor/pidfd_table.rs`, which never accepts a
//! single matching property as identity.

use std::collections::BTreeSet;
use std::fmt;

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::resource_status::ResourcePhase;
use d2b_contracts::v3::user::{USER_RESOURCE_TYPE, UserSpec};
use serde::{Serialize, Serializer};

use crate::error::SystemCoreError;
use crate::ownership;

/// One property a discovery adapter can verify about a local User.
///
/// The set is closed. Nothing here is a value: a binding records only that
/// the adapter checked a property and found it consistent with the spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum UserBinding {
    /// The declared OS username resolves to exactly one local record.
    NssRecord,
    /// That record's primary group resolves.
    PrimaryGroup,
    /// Every additional group the spec declares is a verified membership.
    GroupMemberships,
    /// A per-user service manager exists for the resolved identity.
    UserManager,
}

/// The opaque stable identity of one discovered local User.
///
/// It is derived by the discovery adapter from the immutable identity
/// material it resolved. This digest is the only User identity that is ever
/// public status.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserIdentityDigest([u8; 32]);

impl UserIdentityDigest {
    /// Wrap 32 opaque digest bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Render the digest as lowercase hex.
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
            out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
        }
        out
    }
}

impl fmt::Debug for UserIdentityDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UserIdentityDigest(<redacted>)")
    }
}

impl Serialize for UserIdentityDigest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

/// The exact set of properties a discovery adapter actually verified.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserObservation {
    verified: BTreeSet<UserBinding>,
}

impl UserObservation {
    /// Build an observation from the bindings that were verified.
    pub fn from_verified(verified: impl IntoIterator<Item = UserBinding>) -> Self {
        Self {
            verified: verified.into_iter().collect(),
        }
    }

    /// Whether every required binding was verified.
    pub fn covers(&self, required: &BTreeSet<UserBinding>) -> bool {
        required.is_subset(&self.verified)
    }

    /// Borrow the verified bindings.
    pub const fn verified(&self) -> &BTreeSet<UserBinding> {
        &self.verified
    }
}

/// One locally resolved User, as reported by the discovery adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredUser {
    /// The opaque identity the adapter derived.
    pub identity: UserIdentityDigest,
    /// What the adapter verified while deriving it.
    pub observed: UserObservation,
}

/// The injected seam through which local User discovery reaches the host.
///
/// The fixed core effect adapter is the sole implementor. A Provider never
/// implements this itself, never opens an NSS handle, and never reads a
/// local account database.
pub trait UserDiscoveryEffectPort {
    /// Resolve one declared User locally.
    ///
    /// `Ok(None)` means the local machine resolves no such identity, which
    /// is an ordinary state rather than a failure.
    fn discover(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> impl Future<Output = Result<Option<DiscoveredUser>, SystemCoreError>>;
}

/// How discovery resolved a declared User.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum UserDiscoveryCondition {
    /// Every required property was verified.
    Discovered,
    /// The local machine resolves no such identity.
    Absent,
    /// The identity resolves, but a declared property did not verify.
    Drifted,
    /// The identity resolves without the properties required to call it
    /// this User at all.
    Unverified,
}

/// The public User status this Provider computes.
///
/// The declared OS username, its groups, and every numeric identity are
/// deliberately absent. A reader learns which User resource this is, and
/// whether the local machine agrees it exists.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserStatusReport {
    /// The User this status describes.
    pub user_ref: ResourceRef,
    /// The reconciling Provider, always `system-core`.
    pub provider: &'static str,
    /// The universal resource phase.
    pub phase: ResourcePhase,
    /// How discovery resolved the User.
    pub discovery: UserDiscoveryCondition,
    /// The opaque identity, present only once something resolved.
    pub identity: Option<UserIdentityDigest>,
}

impl fmt::Debug for UserStatusReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("UserStatusReport(<redacted>)")
    }
}

/// The `system-core` local User reconciler.
#[derive(Debug)]
pub struct UserReconciler<P: UserDiscoveryEffectPort> {
    port: P,
}

impl<P: UserDiscoveryEffectPort> UserReconciler<P> {
    /// Build the reconciler over an injected discovery port.
    pub const fn new(port: P) -> Self {
        Self { port }
    }

    /// Borrow the injected discovery port.
    pub const fn port(&self) -> &P {
        &self.port
    }

    /// The properties a User must verify before it is reported discovered.
    ///
    /// The record and its primary group are always required. Group
    /// memberships are required exactly when the spec declares any, so a
    /// User that declares none is not held to a check with nothing to
    /// check, and a User that declares some cannot be called discovered
    /// while they are unverified.
    pub fn required_bindings(spec: &UserSpec) -> BTreeSet<UserBinding> {
        let mut required = BTreeSet::from([UserBinding::NssRecord, UserBinding::PrimaryGroup]);
        if !spec.groups().is_empty() {
            required.insert(UserBinding::GroupMemberships);
        }
        required
    }

    /// Discover one declared User and compute its public status.
    pub async fn reconcile(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> Result<UserStatusReport, SystemCoreError> {
        ownership::require_resource_type(user_ref, USER_RESOURCE_TYPE)?;
        let Some(discovered) = self.port.discover(user_ref, spec).await? else {
            return Ok(self.report(
                user_ref,
                ResourcePhase::Pending,
                UserDiscoveryCondition::Absent,
                None,
            ));
        };
        let required = Self::required_bindings(spec);
        if discovered.observed.covers(&required) {
            return Ok(self.report(
                user_ref,
                ResourcePhase::Ready,
                UserDiscoveryCondition::Discovered,
                Some(discovered.identity),
            ));
        }
        // The identity resolved but not completely. Distinguish the two
        // incomplete cases rather than collapsing them: a User whose record
        // and primary group verified is the declared identity with drifted
        // group state, while one missing either of those was never
        // established as this User at all.
        let established = BTreeSet::from([UserBinding::NssRecord, UserBinding::PrimaryGroup]);
        let (phase, condition) = if discovered.observed.covers(&established) {
            (ResourcePhase::Degraded, UserDiscoveryCondition::Drifted)
        } else {
            (ResourcePhase::Unknown, UserDiscoveryCondition::Unverified)
        };
        Ok(self.report(user_ref, phase, condition, Some(discovered.identity)))
    }

    fn report(
        &self,
        user_ref: &ResourceRef,
        phase: ResourcePhase,
        discovery: UserDiscoveryCondition,
        identity: Option<UserIdentityDigest>,
    ) -> UserStatusReport {
        UserStatusReport {
            user_ref: user_ref.clone(),
            provider: crate::PROVIDER_NAME,
            phase,
            discovery,
            identity,
        }
    }
}
