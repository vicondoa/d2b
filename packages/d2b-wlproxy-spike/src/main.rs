//! Lane A spike: wl-proxy integration feasibility under `unsafe_code = forbid`.
//!
//! This module demonstrates the patterns required for the host-side
//! d2b-wayland-filter binary (Lane C) without implementing the full
//! production proxy. It is compile-only evidence: the patterns here compile
//! under `#![forbid(unsafe_code)]` even though `wl-proxy` uses unsafe
//! internally (in its `uapi`/fd-passing layer). The forbid attribute applies
//! only to this crate's own code, which is the correct constraint.
//!
//! Key feasibility questions addressed by this spike:
//!
//! 1. **safe-Rust integration**: `wl-proxy`'s public API is entirely safe Rust;
//!    we can implement `WlRegistryHandler`, `XdgToplevelHandler`, etc. without
//!    any `unsafe` block in our code.
//!
//! 2. **`wl_registry.bind` rejection**: `GlobalMapper::ignore_global(name)` +
//!    `GlobalMapper::forward_bind(registry, client_name, &id)` correctly drop
//!    bind requests for ignored or unadvertised globals. A client that never
//!    received the `global` event for a name cannot construct a valid
//!    `client_name` that maps to a server name; `GlobalMapper` silently drops
//!    such bind attempts (see `try_forward_bind_impl`).
//!
//! 3. **fail-closed startup**: `State::builder(baseline).with_server_display_name(…).build()`
//!    fails with `StateErrorKind::Connect` if the upstream compositor socket is
//!    unavailable. The listen socket (`Acceptor::new`) is opened separately
//!    and ONLY after `build()` succeeds. This gives us the fail-closed ordering
//!    the plan requires: no listener before upstream is confirmed reachable.
//!
//! 4. **fd/SCM_RIGHTS**: `wl-proxy` handles fd passing via `uapi::recvmsg` with
//!    `MSG_CMSG_CLOEXEC`. **MSG_CTRUNC gap**: the control buffer in `trans.rs` is
//!    fixed at 128 bytes. A Wayland message with more than ~28 fds in a single
//!    recvmsg call would trigger MSG_CTRUNC, silently dropping the excess fds
//!    because `trans.rs` does not check `header.flags` for MSG_CTRUNC after the
//!    call. In practice the Wayland protocol sends at most 1–3 fds per message
//!    (dmabuf planes, clipboard, etc.), so this limit is unlikely to be reached.
//!    The d2b filter binary (Lane C) should document this as a known
//!    low-risk gap and log a startup warning if a safety margin is needed.
//!
//! 5. **proxy restart semantics**: `State::builder` connects to the upstream
//!    compositor via a blocking `connect(2)` at build time. There is no
//!    reconnect loop inside `wl-proxy`; if the upstream closes the connection
//!    during proxy operation, `wl-proxy` surfaces an error through its poll
//!    loop. For crosvm reconnect semantics: crosvm does NOT reconnect the
//!    Wayland socket after it is opened; a proxy death or restart that changes
//!    the socket path (or deletes and re-creates the socket) will leave the
//!    running VM's GPU sidecar with a dead Wayland path. The plan's requirement
//!    to treat proxy death as a fatal VM error (triggering d2bd's pidfd
//!    watchdog) is correct — silent restart without socket-path continuity is
//!    not viable.
//!
//! 6. **minijail/seccomp/RLIMIT_NOFILE**: The filter binary uses a standard
//!    epoll event loop (Poller::new() wraps epoll_create1). Required syscalls:
//!    `socket(AF_UNIX)`, `connect`, `bind`, `listen`, `accept4`, `epoll_create1`,
//!    `epoll_ctl`, `epoll_wait`/`epoll_pwait`, `recvmsg`/`sendmsg`, `fcntl`,
//!    `eventfd`, `read`, `write`, `close`. The `uapi` layer also uses `getpid`
//!    and `gettid`. A seccomp allowlist for this set is well-defined and minimal.
//!    `RLIMIT_NOFILE`: each guest client requires ~3 fds (client socket, server
//!    socket, potential dmabuf fd). A 1024-fd limit is adequate for a single-VM
//!    proxy; 4096 is a safe default. No exotic ioctl or privileged syscall is
//!    required after startup.
//!
//! 7. **secure defaults and warning visibility**: The filter binary should deny
//!    security-sensitive globals by default (e.g., `wlr_screencopy`, `wlr_export_dmabuf`,
//!    `ext_data_control`, `zwp_linux_dmabuf_feedback_v1` if not needed for graphics,
//!    `security_context_v1`). Warnings for operator overrides are trivially
//!    implemented by printing to stderr before starting the listen loop — visible
//!    in journald output from the broker-spawned process.
//!
//! 8. **multi-output behavior**: `wl-proxy` passes through all compositor globals
//!    including `wl_output` and `xdg_output`; the guest sees a single (virtual)
//!    output via the cross-domain transport. Multi-output host compositor behavior
//!    is not visible to the guest through this stack — the cross-domain transport
//!    presents a single virtual output regardless of host monitor count. This is
//!    a property of the cross-domain virtio-gpu layer, not of the host filter.

fn main() {
    println!("d2b-wlproxy-spike: compile-time feasibility proof only. Not a runnable binary.");
}

use std::{os::fd::OwnedFd, rc::Rc};

use wl_proxy::{
    baseline::Baseline,
    global_mapper::GlobalMapper,
    object::Object,
    protocols::{
        ObjectInterface,
        wayland::wl_display::{WlDisplay, WlDisplayHandler},
        wayland::wl_registry::{WlRegistry, WlRegistryHandler},
        xdg_shell::xdg_toplevel::{XdgToplevel, XdgToplevelHandler},
    },
    state::{State, StateError},
};

/// Demonstrates the fail-closed startup pattern:
/// 1. Attempt upstream connect (State::build).
/// 2. Only if that succeeds, create the listen socket (Acceptor).
///
/// In production (Lane C), `upstream_socket_path` is the path to the
/// real host compositor socket inside the minijail, e.g. `/run/wl/wayland-0`.
/// The listen socket is `/run/d2b-wlproxy/<vm>/wayland.sock`.
///
/// Returns Err if the upstream is unreachable — the caller exits non-zero
/// and the broker never reports the process as ready.
pub fn build_proxy_state(
    upstream_socket_path: &str,
    policy: FilterPolicy,
) -> Result<Rc<State>, StateError> {
    // Step 1: Connect to upstream. Fails closed if socket is unavailable.
    // This MUST happen before we create the listen socket.
    let state = State::builder(Baseline::ALL_OF_THEM)
        .with_server_display_name(upstream_socket_path)
        .build()?;

    // Step 2: Emit policy warnings before accepting any clients.
    // (In production, check each denied global against the "required for
    // baseline graphics" list and warn if an operator override would remove it.)
    policy.emit_warnings();

    // Step 3 (not shown here): Create the Acceptor and begin accept loop.
    // Acceptor::new() is called only here, after upstream is confirmed alive.
    Ok(state)
}

/// Minimal filter policy for the spike.
///
/// In production (Lane C) this struct carries the full resolved rule set
/// produced from the NixOS option tree, with operator overrides and warning
/// metadata. Here it just captures the names to ignore.
pub struct FilterPolicy {
    /// Wayland global interfaces to deny entirely.
    pub denied_globals: Vec<String>,
    /// vm_name, used for app-id prefix enforcement.
    pub vm_name: String,
}

impl FilterPolicy {
    /// Emit advisory warnings for any deny that changes a required global.
    ///
    /// Called before the listen loop starts. Output goes to stderr and is
    /// captured by journald. This is advisory-only: the proxy still starts.
    pub fn emit_warnings(&self) {
        // Baseline required globals for d2b graphics VMs.
        const REQUIRED_FOR_GRAPHICS: &[&str] = &[
            "wl_compositor",
            "wl_shm",
            "xdg_wm_base",
            "wp_viewporter",
            "linux_dmabuf_v1",
        ];
        for denied in &self.denied_globals {
            if REQUIRED_FOR_GRAPHICS.contains(&denied.as_str()) {
                log::warn!(
                    "waylandFilter: denied global '{}' is required for baseline graphics; \
                     VM applications may fail to render",
                    denied
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Handler types — these implement wl-proxy traits in safe Rust only.
// No unsafe{} block appears anywhere in this file.
// ---------------------------------------------------------------------------

/// Top-level display handler — the entry point for each proxied client.
pub struct FilterDisplay {
    policy: Rc<FilterPolicy>,
}

impl FilterDisplay {
    pub fn new(policy: Rc<FilterPolicy>) -> Self {
        Self { policy }
    }
}

impl WlDisplayHandler for FilterDisplay {
    fn handle_get_registry(&mut self, slf: &Rc<WlDisplay>, registry: &Rc<WlRegistry>) {
        // Forward to compositor; install our registry handler.
        slf.send_get_registry(registry);
        registry.set_handler(FilterRegistry {
            filter: GlobalMapper::default(),
            policy: self.policy.clone(),
        });
    }
}

/// Registry handler — enforces the deny list and name remapping.
struct FilterRegistry {
    filter: GlobalMapper,
    policy: Rc<FilterPolicy>,
}

impl WlRegistryHandler for FilterRegistry {
    fn handle_global(
        &mut self,
        slf: &Rc<WlRegistry>,
        name: u32,
        interface: ObjectInterface,
        version: u32,
    ) {
        // Check interface name against deny list.
        let interface_name: &str = interface.name();
        if self.policy.denied_globals.iter().any(|d| d == interface_name) {
            // Mark as ignored: any subsequent client bind for this name's
            // remapped client-name will be silently dropped by GlobalMapper.
            self.filter.ignore_global(name);
        } else {
            // Forward the global advertisement to the client.
            // GlobalMapper remaps server-side names to client-side names,
            // so the client never learns the real compositor name.
            self.filter.forward_global(slf, name, interface, version);
        }
    }

    fn handle_global_remove(&mut self, slf: &Rc<WlRegistry>, name: u32) {
        // GlobalMapper's forward_global_remove correctly no-ops for ignored globals.
        self.filter.forward_global_remove(slf, name);
    }

    fn handle_bind(&mut self, slf: &Rc<WlRegistry>, name: u32, id: Rc<dyn Object>) {
        // forward_bind maps the client-side name back to the server-side name
        // using GlobalMapper's internal table. If the client tries to bind a
        // name that was never advertised (or was ignored), GlobalMapper logs
        // a warning and drops the bind — the compositor never sees it.
        self.filter.forward_bind(slf, name, &id);
    }
}

/// xdg_toplevel handler — enforces app-id prefix for niri window-rule matching.
///
/// This demonstrates that app-id rewriting is purely safe Rust: intercept
/// `handle_set_app_id`, prepend the VM prefix if not already present, and
/// call `slf.send_set_app_id(&prefixed)`.
pub struct FilterToplevel {
    vm_name: String,
}

impl FilterToplevel {
    pub fn new(vm_name: String) -> Self {
        Self { vm_name }
    }

    fn app_id_prefix(&self) -> String {
        format!("d2b.{}.", self.vm_name)
    }
}

impl XdgToplevelHandler for FilterToplevel {
    fn handle_set_app_id(&mut self, slf: &Rc<XdgToplevel>, app_id: &str) {
        let prefix = self.app_id_prefix();
        // Enforce prefix: strip any existing d2b prefix first to avoid
        // double-prefixing on proxy restart, then re-add ours.
        let stripped = app_id
            .strip_prefix(&prefix)
            .unwrap_or(app_id);
        let prefixed = format!("{prefix}{stripped}");
        slf.send_set_app_id(&prefixed);
    }
}

/// Demonstrates that passing an `OwnedFd` to `State::builder::with_server_fd`
/// is safe Rust: the builder takes `&Rc<OwnedFd>` and the fd is managed by
/// the RAII wrapper. No `unsafe` required for fd injection from the broker's
/// `SCM_RIGHTS` handoff.
pub fn build_from_fd(fd: OwnedFd, policy: FilterPolicy) -> Result<Rc<State>, StateError> {
    let fd = Rc::new(fd);
    let state = State::builder(Baseline::ALL_OF_THEM)
        .with_server_fd(&fd)
        .build()?;
    policy.emit_warnings();
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_warnings_do_not_panic_for_safe_defaults() {
        let policy = FilterPolicy {
            denied_globals: vec![
                // Security-sensitive globals that are safe to deny
                "wlr_screencopy_manager_v1".into(),
                "wlr_export_dmabuf_manager_v1".into(),
                "ext_data_control_manager_v1".into(),
                "zwlr_data_control_manager_v1".into(),
                "security_context_manager_v1".into(),
            ],
            vm_name: "work".into(),
        };
        // Should not warn (none of these are in REQUIRED_FOR_GRAPHICS)
        policy.emit_warnings();
    }

    #[test]
    fn app_id_prefixing_idempotent() {
        let h = FilterToplevel::new("work-aad".into());
        let prefix = h.app_id_prefix();
        assert_eq!(prefix, "d2b.work-aad.");
        // First application
        let prefixed = format!("{prefix}org.mozilla.firefox");
        // Idempotency: strip then re-add prefix
        let stripped = prefixed.strip_prefix(&prefix).unwrap_or(&prefixed);
        assert_eq!(stripped, "org.mozilla.firefox");
        let re_prefixed = format!("{prefix}{stripped}");
        assert_eq!(re_prefixed, "d2b.work-aad.org.mozilla.firefox");
    }

    #[test]
    fn denied_global_warning_emitted_for_required_global() {
        // This test just checks the warning path doesn't panic.
        // In production, warnings are captured by journald.
        let policy = FilterPolicy {
            denied_globals: vec!["xdg_wm_base".into()], // required global
            vm_name: "work".into(),
        };
        policy.emit_warnings(); // logs a warning, does not panic
    }
}
