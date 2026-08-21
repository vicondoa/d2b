use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_contracts_control::public_wire;
use d2b_realm_core::PrincipalId;
use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use socket2::Socket;
use uzers::get_group_by_name;
use uzers::{get_user_by_uid, get_user_groups};

use crate::typed_error::TypedError;
use crate::unix_transport::io_wrap;

#[derive(Debug, Clone)]
pub struct AdmissionConfig {
    pub daemon_uid: u32,
    pub public_socket_group: String,
    pub launcher_users: Vec<String>,
    pub admin_users: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PeerIdentity {
    pub role: PeerRole,
    pub uid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerRole {
    Launcher,
    Admin,
    /// Scoped authority for the guarded `ExecStop` host-shutdown hook
    /// (`d2b host shutdown-hook --apply`), which runs as uid 0 under
    /// systemd `ExecStop = "+..."`. Permits only `vmStop` during host
    /// shutdown; all other admin-only operations (exec, USB attach, key
    /// rotation, host prepare, audit export …) are explicitly denied.
    /// The kernel's `SO_PEERCRED` provides the uid=0 identity - no other
    /// per-connection credential is evaluated for this role.
    HostShutdown,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
pub struct PeerOverride {
    pub uid: u32,
    pub gid: u32,
    pub username: Option<String>,
    pub groups: Option<Vec<String>>,
}

#[cfg(any(test, feature = "test-support"))]
// Compiled out unless test-support is enabled by d2bd's test targets; release
// binaries contain no peer-identity override path.
pub static TEST_PEER_OVERRIDE: std::sync::Mutex<Option<PeerOverride>> =
    std::sync::Mutex::new(None);

#[cfg(any(test, feature = "test-support"))]
pub static TEST_PEER_OVERRIDE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn authorize_peer(
    stream: &Socket,
    config: &AdmissionConfig,
) -> Result<PeerIdentity, TypedError> {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(peer) = peer_override_injected() {
        let _peer_gid = peer.gid;
        return classify_peer(peer.uid, peer.username, peer.groups, config, false);
    }

    let peer = getsockopt(stream, PeerCredentials).map_err(io_wrap("read SO_PEERCRED"))?;
    let _peer_pid = peer.pid();
    let _peer_gid = peer.gid();
    let uid = peer.uid() as u32;
    if uid == 0 {
        return classify_peer(uid, None, Some(Vec::new()), config, false);
    }
    let user = get_user_by_uid(uid);
    let username = user
        .as_ref()
        .map(|user| user.name().to_string_lossy().into_owned());
    let groups = user
        .as_ref()
        .and_then(|user| get_user_groups(user.name(), user.primary_group_id()))
        .map(|groups| {
            groups
                .into_iter()
                .map(|group| group.name().to_string_lossy().into_owned())
                .collect()
        });
    classify_peer(uid, username, groups, config, true)
}

#[cfg(any(test, feature = "test-support"))]
fn peer_override_injected() -> Option<PeerOverride> {
    TEST_PEER_OVERRIDE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

pub fn classify_peer(
    uid: u32,
    username: Option<String>,
    groups: Option<Vec<String>>,
    config: &AdmissionConfig,
    production_lookup: bool,
) -> Result<PeerIdentity, TypedError> {
    if uid == config.daemon_uid {
        return Err(TypedError::AuthzNotALauncher { peer_uid: uid });
    }

    // uid=0 is the host-shutdown hook running under systemd
    // `ExecStop = "+..."`. It receives the narrow `HostShutdown` role which
    // is only permitted to issue `vmStop` during host shutdown teardown.
    if uid == 0 {
        return Ok(PeerIdentity {
            role: PeerRole::HostShutdown,
            uid,
        });
    }

    // A failed local identity or group lookup is an authorization failure,
    // never an empty-group fallback. The configured socket group is the only
    // supplementary-group grant; only the exact configured group is used.
    let Some(groups) = groups else {
        return Err(TypedError::AuthzNotALauncher { peer_uid: uid });
    };
    let lifecycle_group = config.public_socket_group.as_str();
    let lifecycle_member = lifecycle_group_member(lifecycle_group, &groups);
    let configured_launcher = username.as_ref().is_some_and(|name| {
        config
            .launcher_users
            .iter()
            .any(|launcher| launcher == name)
    });
    if !configured_launcher && !lifecycle_member {
        return Err(TypedError::AuthzNotALauncher { peer_uid: uid });
    }

    // In production, an unknown configured lifecycle group is a failed
    // classifier lookup rather than an authority grant. Test injection keeps
    // the group list hermetic and therefore skips NSS group-name lookup.
    if production_lookup && get_group_by_name(lifecycle_group).is_none() {
        return Err(TypedError::AuthzNotALauncher { peer_uid: uid });
    }

    let role = if username
        .as_ref()
        .is_some_and(|name| config.admin_users.iter().any(|admin| admin == name))
    {
        PeerRole::Admin
    } else {
        PeerRole::Launcher
    };

    Ok(PeerIdentity { role, uid })
}

pub fn lifecycle_group_member(configured_group: &str, groups: &[String]) -> bool {
    !configured_group.is_empty() && groups.iter().any(|group| group == configured_group)
}

pub fn verb_requires_admin(verb: &str) -> bool {
    matches!(
        verb,
        "vmStart"
            | "vmStop"
            | "vmRestart"
            | "switch"
            | "boot"
            | "test"
            | "rollback"
            | "gc"
            | "keysRotate"
            | "trust"
            | "rotateKnownHost"
            | "usbipBind"
            | "usbipUnbind"
            | "storeVerify"
            | "migrate"
            | "hostPrepare"
            | "hostDestroy"
            | "hostInstall"
            | "hostReconcile"
            | "hostCutover"
            | "resourceReconcile"
            | "readGuestConfig"
            | "exec"
            | "shell"
    )
}

/// Returns `true` if the verb is permitted for the narrow [`PeerRole::HostShutdown`]
/// role. This is a strict positive allowlist: only `vmStop` is permitted.
/// All other admin-only operations (exec, USB attach, key rotation,
/// audit export, host prepare, …) are denied even though root could
/// normally perform them, because the shutdown hook only needs to stop
/// running VMs.
pub fn verb_allowed_for_host_shutdown(verb: &str) -> bool {
    matches!(verb, "vmStop")
}

pub fn gateway_display_op_requires_admin(op: &public_wire::GatewayDisplayOp) -> bool {
    matches!(
        op,
        public_wire::GatewayDisplayOp::Start(_) | public_wire::GatewayDisplayOp::Stop(_)
    )
}

pub fn gateway_display_peer_principal(peer: &PeerIdentity) -> PrincipalId {
    PrincipalId::parse(format!("uid-{}", peer.uid))
        .expect("trusted display principal derived from numeric uid is valid")
}

pub fn gateway_display_peer_principal_string(peer: &PeerIdentity) -> String {
    gateway_display_peer_principal(peer).to_string()
}

pub fn broker_caller_role_for_peer(peer: &PeerIdentity) -> BrokerCallerRole {
    match peer.role {
        PeerRole::Admin => BrokerCallerRole::AdminUid { uid: peer.uid },
        PeerRole::Launcher => BrokerCallerRole::LauncherUid { uid: peer.uid },
        PeerRole::HostShutdown => BrokerCallerRole::AdminUid { uid: peer.uid },
    }
}

#[cfg(test)]
mod tests {
    use super::lifecycle_group_member;

    #[test]
    fn only_the_configured_lifecycle_group_grants_group_authority() {
        let groups = vec!["wheel".to_owned(), "d2b".to_owned()];
        assert!(lifecycle_group_member("d2b", &groups));
        assert!(lifecycle_group_member("wheel", &["wheel".to_owned()]));
        assert!(!lifecycle_group_member("d2b", &["wheel".to_owned()]));
        assert!(!lifecycle_group_member("missing", &groups));
        assert!(!lifecycle_group_member("", &groups));
    }

    #[test]
    fn cutover_is_admin_only_and_host_shutdown_cannot_reach_it() {
        assert!(super::verb_requires_admin("hostCutover"));
        assert!(!super::verb_allowed_for_host_shutdown("hostCutover"));
    }
}
