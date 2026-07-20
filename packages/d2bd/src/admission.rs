use d2b_contracts::public_wire;
use d2b_realm_core::PrincipalId;
use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use socket2::Socket;
use uzers::{get_user_by_uid, get_user_groups};

use crate::{ServerState, io_wrap, typed_error::TypedError};

#[derive(Debug, Clone)]
pub(crate) struct PeerIdentity {
    pub(crate) role: PeerRole,
    pub(crate) uid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PeerRole {
    Launcher,
    Admin,
    /// Scoped authority for the guarded `ExecStop` host-shutdown hook
    /// (`d2b host shutdown-hook --apply`), which runs as uid 0 under
    /// systemd `ExecStop = "+..."`. Permits only `vmStop` during host
    /// shutdown; all other admin-only operations (exec, USB attach, key
    /// rotation, host prepare, audit export …) are explicitly denied.
    /// The kernel's `SO_PEERCRED` provides the uid=0 identity — no other
    /// per-connection credential is evaluated for this role.
    HostShutdown,
}

#[cfg_attr(test, derive(Clone))]
pub(crate) struct PeerOverride {
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) username: Option<String>,
    pub(crate) groups: Option<Vec<String>>,
}

pub(crate) fn authorize_peer(
    stream: &Socket,
    state: &ServerState,
) -> Result<PeerIdentity, TypedError> {
    // Unit tests may inject identity through a cfg(test)-only slot. Every
    // production binary, including binaries exercised by integration tests,
    // derives peer identity exclusively from SO_PEERCRED.
    let peer_override = match peer_override_injected() {
        Some(peer) => peer,
        None => {
            let peer = getsockopt(stream, PeerCredentials).map_err(io_wrap("read SO_PEERCRED"))?;
            PeerOverride {
                uid: peer.uid() as u32,
                gid: peer.gid() as u32,
                username: None,
                groups: None,
            }
        }
    };
    let uid = peer_override.uid;
    let _gid = peer_override.gid;
    let username = peer_override
        .username
        .or_else(|| get_user_by_uid(uid).map(|user| user.name().to_string_lossy().into_owned()));
    let _supplementary_groups = if let Some(groups) = peer_override.groups {
        groups
    } else if let Some(user) = get_user_by_uid(uid) {
        get_user_groups(user.name(), user.primary_group_id())
            .into_iter()
            .flatten()
            .map(|group| group.name().to_string_lossy().into_owned())
            .collect()
    } else {
        Vec::new()
    };

    if uid == state.daemon_uid {
        return Err(TypedError::AuthzNotALauncher { peer_uid: uid });
    }

    // uid=0 is the host-shutdown hook running under systemd
    // `ExecStop = "+..."`.  It receives the narrow `HostShutdown` role
    // which is only permitted to issue `vmStop` during host shutdown
    // teardown.  Any other admin-only operation is denied at dispatch.
    if uid == 0 {
        return Ok(PeerIdentity {
            role: PeerRole::HostShutdown,
            uid,
        });
    }

    let is_launcher = username
        .as_ref()
        .map(|name| {
            state
                .config
                .launcher_users
                .iter()
                .any(|launcher| launcher == name)
        })
        .unwrap_or(false);
    if !is_launcher {
        return Err(TypedError::AuthzNotALauncher { peer_uid: uid });
    }

    let role = if username
        .as_ref()
        .map(|name| state.config.admin_users.iter().any(|admin| admin == name))
        .unwrap_or(false)
    {
        PeerRole::Admin
    } else {
        PeerRole::Launcher
    };

    Ok(PeerIdentity { role, uid })
}

pub(crate) fn verb_requires_admin(verb: &str) -> bool {
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
            | "readGuestConfig"
            | "observabilityExportInspect"
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
pub(crate) fn verb_allowed_for_host_shutdown(verb: &str) -> bool {
    matches!(verb, "vmStop")
}

pub(crate) fn gateway_display_op_requires_admin(op: &public_wire::GatewayDisplayOp) -> bool {
    matches!(
        op,
        public_wire::GatewayDisplayOp::Start(_) | public_wire::GatewayDisplayOp::Stop(_)
    )
}

pub(crate) fn gateway_display_peer_principal(peer: &PeerIdentity) -> PrincipalId {
    PrincipalId::parse(format!("uid-{}", peer.uid))
        .expect("trusted display principal derived from numeric uid is valid")
}

pub(crate) fn gateway_display_peer_principal_string(peer: &PeerIdentity) -> String {
    gateway_display_peer_principal(peer).to_string()
}

/// Test-only peer-credential injection. The accept path
/// ([`authorize_peer`]) reads the connecting peer's identity from
/// `SO_PEERCRED`; the accept-loop tests need to drive `handle_connection`
/// over an in-process socketpair while pretending the peer is a specific
/// launcher/admin uid. Rather than mutate process-global env (which is
/// `unsafe` under edition 2024) this is injected through a `#[cfg(test)]`
/// `Mutex`. In non-test builds it is compiled out and always `None`, so the
/// production accept path has no test backdoor at all.
#[cfg(test)]
pub(crate) static TEST_PEER_OVERRIDE: std::sync::Mutex<Option<PeerOverride>> =
    std::sync::Mutex::new(None);

/// Serializes the accept-loop tests that inject a [`PeerOverride`] so two of
/// them cannot interleave on the process-global injection slot.
#[cfg(any())]
pub(crate) static TEST_PEER_OVERRIDE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
fn peer_override_injected() -> Option<PeerOverride> {
    TEST_PEER_OVERRIDE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

#[cfg(not(test))]
fn peer_override_injected() -> Option<PeerOverride> {
    None
}
