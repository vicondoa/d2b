//! Daemon-side User family effects.
//!
//! The `d2b-provider-user` crate owns the driver and its effect port; this
//! module implements that port over the preserved system-core realization:
//! the local NSS discovery the `UserReconciler` drives. Every call that
//! touches the account database stays here, so the family crate reaches no
//! host state of its own. The Host family's probe moved into
//! `d2b-provider-host` with its declared service (U5).

use async_trait::async_trait;
use d2b_contracts_resource::v3::{ResourceRef, user::UserSpec};
use d2b_provider_system_core::{
    DiscoveredUser, SystemCoreError, UserBinding, UserDiscoveryEffectPort, UserIdentityDigest,
    UserObservation, UserReconciler, UserStatusReport,
};
use d2b_provider_user::UserDriverEffects;

// ---------------------------------------------------------------------------
// Production local User discovery (moved with the family's daemon-side
// effects)
// ---------------------------------------------------------------------------

/// Production effects over the local host for the `User` type: the NSS
/// discovery adapter the old core runner wired.
pub(crate) struct ProductionUserDriverEffects;

#[async_trait]
impl UserDriverEffects for ProductionUserDriverEffects {
    async fn observe_user(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> Result<UserStatusReport, String> {
        UserReconciler::new(LocalUserDiscovery)
            .reconcile(user_ref, spec)
            .await
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct LocalUserDiscovery;

impl UserDiscoveryEffectPort for LocalUserDiscovery {
    async fn discover(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> Result<Option<DiscoveredUser>, SystemCoreError> {
        discover_local_user(user_ref, spec).await
    }
}

async fn discover_local_user(
    user_ref: &ResourceRef,
    spec: &UserSpec,
) -> Result<Option<DiscoveredUser>, SystemCoreError> {
    use nix::unistd::{Group, User};
    use sha2::{Digest, Sha256};

    let username = spec.os_username().as_str();
    let user = User::from_name(username).map_err(|_| SystemCoreError::DiscoveryUnavailable)?;
    let Some(user) = user else {
        return Ok(None);
    };

    let mut digest = Sha256::new();
    digest.update(b"d2b-system-core-user-v1");
    digest.update(user_ref.name().as_str().as_bytes());
    digest.update([0]);
    digest.update(username.as_bytes());
    digest.update([0]);
    digest.update(user.uid.as_raw().to_le_bytes());
    digest.update(user.gid.as_raw().to_le_bytes());

    let mut verified = std::collections::BTreeSet::from([UserBinding::NssRecord]);
    if Group::from_gid(user.gid)
        .map_err(|_| SystemCoreError::DiscoveryUnavailable)?
        .is_some()
    {
        verified.insert(UserBinding::PrimaryGroup);
    }

    let mut groups_verified = true;
    for group in spec.groups() {
        let Some(group_record) = Group::from_name(group.as_str())
            .map_err(|_| SystemCoreError::DiscoveryUnavailable)?
        else {
            groups_verified = false;
            tracing::debug!(
                user = %username,
                group = %group.as_str(),
                "system-core user group record missing; membership unverified",
            );
            continue;
        };
        digest.update([0]);
        digest.update(group.as_str().as_bytes());
        if !group_record.mem.iter().any(|member| member == username) {
            groups_verified = false;
        }
    }
    if groups_verified && !spec.groups().is_empty() {
        verified.insert(UserBinding::GroupMemberships);
    }

    Ok(Some(DiscoveredUser {
        identity: UserIdentityDigest::from_bytes(digest.finalize().into()),
        observed: UserObservation::from_verified(verified),
    }))
}