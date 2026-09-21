//! The bounded local-account probe (U5): the NSS discovery the User
//! reconciler reads the machine through.
//!
//! The probe moved wholesale into this crate with the family's daemon-side
//! effects: every call that touches the account database - the bounded
//! `getpwnam` / `getgrnam` record reads and the group-membership checks -
//! lives here, so this crate reaches host state through no daemon runtime.

use d2b_contracts_resource::v3::{ResourceRef, user::UserSpec};
use d2b_provider_system_core::{
    DiscoveredUser, SystemCoreError, UserBinding, UserDiscoveryEffectPort, UserIdentityDigest,
    UserObservation,
};

/// The bounded local-account probe: resolves one declared User through the
/// fixed NSS surface and derives the opaque identity digest and the verified
/// bindings, exactly as the daemon-side adapter did (U5).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct UserProbe;

#[async_trait::async_trait]
impl UserDiscoveryEffectPort for UserProbe {
    async fn discover(
        &self,
        user_ref: &ResourceRef,
        spec: &UserSpec,
    ) -> Result<Option<DiscoveredUser>, SystemCoreError> {
        discover_local_user(user_ref, spec).await
    }
}

/// Resolve one declared User locally.
///
/// `Ok(None)` means the local machine resolves no such identity, which is
/// an ordinary state rather than a failure; an NSS lookup that cannot
/// complete reports [`SystemCoreError::DiscoveryUnavailable`]. The digest is
/// derived from the immutable identity material: the declared reference and
/// username, the resolved numeric ids, and every declared group, in the
/// fixed `d2b-system-core-user-v1` domain.
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
