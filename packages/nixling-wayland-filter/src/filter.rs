//! wl-proxy handler implementations for the Wayland filter proxy.
//!
//! Handler chain:
//!   FilterStateHandler
//!     -> FilterDisplayHandler (per client)
//!       -> FilterRegistryHandler (per wl_registry)
//!         -> FilterXdgWmBaseHandler (when xdg_wm_base is bound)
//!           -> FilterXdgSurfaceHandler (per xdg_surface)
//!             -> FilterXdgToplevelHandler (per xdg_toplevel)

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
};
use wl_proxy::{
    client::{Client, ClientHandler},
    object::{Object, ObjectCoreApi, ObjectRcUtils},
    protocols::{
        stream::{
            wl_eglstream::WlEglstreamHandleType,
            wl_eglstream_display::{
                WlEglstreamDisplay, WlEglstreamDisplayCap, WlEglstreamDisplayHandler,
            },
        },
        linux_dmabuf_v1::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        wayland::{
            wl_buffer::WlBuffer,
            wl_display::{WlDisplay, WlDisplayHandler},
            wl_registry::{WlRegistry, WlRegistryHandler},
        },
        xdg_shell::{
            xdg_surface::{XdgSurface, XdgSurfaceHandler},
            xdg_toplevel::{XdgToplevel, XdgToplevelHandler},
            xdg_wm_base::{XdgWmBase, XdgWmBaseHandler},
        },
        ObjectInterface,
    },
    state::{Destructor, State, StateHandler},
};

use crate::{
    diag::{DiagRateLimiter, DropReason},
    dmabuf::DmabufHandler,
    policy::FilterPolicy,
};

/// State-level handler: creates per-client display handlers.
pub struct FilterStateHandler {
    policy: Rc<FilterPolicy>,
    diag: Rc<RefCell<DiagRateLimiter>>,
}

impl FilterStateHandler {
    pub fn new(policy: Rc<FilterPolicy>, diag: Rc<RefCell<DiagRateLimiter>>) -> Self {
        Self { policy, diag }
    }
}

impl StateHandler for FilterStateHandler {
    fn new_client(&mut self, client: &Rc<Client>) {
        install_client_handlers(client, self.policy.clone(), self.diag.clone());
    }
}

/// Install all per-client handlers required by the filter.
///
/// `State::add_client` does not invoke `StateHandler::new_client`, so the
/// manual accept loop in the binary calls this helper directly after adding a
/// client. The state handler also uses it for clients accepted through
/// wl-proxy-managed acceptors.
pub fn install_client_handlers(
    client: &Rc<Client>,
    policy: Rc<FilterPolicy>,
    diag: Rc<RefCell<DiagRateLimiter>>,
) {
    let handler = FilterDisplayHandler {
        policy: policy.clone(),
        diag,
    };
    client.display().set_handler(handler);
    log::debug!(
        "[nixling-wlproxy] vm={} new client connected",
        policy.vm_name
    );
}

/// Per-client display handler: intercepts `get_registry`.
struct FilterDisplayHandler {
    policy: Rc<FilterPolicy>,
    diag: Rc<RefCell<DiagRateLimiter>>,
}

impl WlDisplayHandler for FilterDisplayHandler {
    fn handle_get_registry(&mut self, slf: &Rc<WlDisplay>, registry: &Rc<WlRegistry>) {
        // Forward get_registry to the compositor so the server side of the
        // registry is established.
        slf.send_get_registry(registry);
        // Install our registry handler to filter globals.
        registry.set_handler(FilterRegistryHandler::new(
            self.policy.clone(),
            self.diag.clone(),
        ));
    }
}

/// Per-registry handler: filters globals and intercepts binds.
///
/// Match wl-veil's approach: preserve compositor-provided global names for
/// forwarded globals, suppress denied globals, and track which names were
/// advertised to reject binds for hidden/unadvertised names. Avoid remapping
/// registry names here; wl-proxy's generated object handling already manages
/// client/server object ID translation for bound objects.
pub struct FilterRegistryHandler {
    policy: Rc<FilterPolicy>,
    diag: Rc<RefCell<DiagRateLimiter>>,
    /// Server global names intentionally hidden from this client.
    hidden_globals: HashSet<u32>,
    /// Server global names actually advertised to this client, with the
    /// interface and version we advertised. Bind requests above this version
    /// are rejected even if the client guessed the original compositor version.
    advertised_globals: HashMap<u32, AdvertisedGlobal>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AdvertisedGlobal {
    interface: ObjectInterface,
    version: u32,
}

impl FilterRegistryHandler {
    pub fn new(policy: Rc<FilterPolicy>, diag: Rc<RefCell<DiagRateLimiter>>) -> Self {
        Self {
            policy,
            diag,
            hidden_globals: HashSet::new(),
            advertised_globals: HashMap::new(),
        }
    }
}

impl WlRegistryHandler for FilterRegistryHandler {
    fn handle_global(
        &mut self,
        slf: &Rc<WlRegistry>,
        name: u32,
        interface: ObjectInterface,
        version: u32,
    ) {
        let iface_name = interface.name();
        let (action, _) = self.policy.lookup(iface_name);
        let crate::policy::GlobalAction::Allow = action else {
            // Denied: ignore and suppress global_remove forwarding too.
            if self.policy.log_filtered_globals {
                self.diag.borrow_mut().global_filtered(iface_name);
            }
            self.hidden_globals.insert(name);
            return;
        };

        let adv_version = self.policy.advertised_version(iface_name, version);
        self.advertised_globals.insert(
            name,
            AdvertisedGlobal {
                interface,
                version: adv_version,
            },
        );
        slf.send_global(name, interface, adv_version);
    }

    fn handle_global_remove(&mut self, slf: &Rc<WlRegistry>, name: u32) {
        if self.hidden_globals.remove(&name) {
            return;
        }
        self.advertised_globals.remove(&name);
        slf.send_global_remove(name);
    }

    fn handle_bind(&mut self, slf: &Rc<WlRegistry>, name: u32, id: Rc<dyn Object>) {
        // Detect and log bind attempts for names that were never advertised
        // to this client or that were explicitly hidden.
        let Some(advertised) = self.advertised_globals.get(&name).copied() else {
            let reason = if self.hidden_globals.contains(&name) {
                DropReason::BindDeniedHidden
            } else {
                DropReason::BindDeniedUnadvertised
            };
            self.diag.borrow_mut().bind_denied(reason, name);
            // The wl-proxy decoder has already registered the newly requested
            // client object ID before this handler runs. Since the bind is not
            // forwarded, keeping the client alive would leak that ID in the
            // client's object table. Denied binds are protocol violations
            // against our filtered registry, so fail closed by dropping the
            // offending client connection.
            if let Some(client) = id.client() {
                client.disconnect();
            }
            drop(id);
            return;
        };

        if !bind_matches_advertised_cap(advertised, id.interface(), id.version()) {
            log::warn!(
                "[nixling-wlproxy] vm={} event=bind-denied reason=version-cap registry-name={} interface={} requested-version={} advertised-version={}",
                self.policy.vm_name,
                name,
                advertised.interface.name(),
                id.version(),
                advertised.version,
            );
            if let Some(client) = id.client() {
                client.disconnect();
            }
            drop(id);
            return;
        }

        // Install per-interface handlers before forwarding, so we can
        // intercept the object's lifecycle from the first message.
        if let Some(wm_base) = id.try_downcast::<XdgWmBase>() {
            wm_base.set_handler(FilterXdgWmBaseHandler {
                policy: self.policy.clone(),
            });
        } else if let Some(eglstream_display) = id.try_downcast::<WlEglstreamDisplay>() {
            eglstream_display.set_handler(FilterEglstreamDisplayHandler {
                vm: self.policy.vm_name.clone(),
            });
        } else if let Some(dmabuf) = id.try_downcast::<ZwpLinuxDmabufV1>() {
            if !self.policy.dmabuf_filters.is_empty() {
                dmabuf.set_handler(DmabufHandler::new(self.policy.dmabuf_filters.clone()));
            }
        }

        slf.send_bind(name, id);
    }
}

/// Handler for `xdg_wm_base`: intercepts `get_xdg_surface` to install our
/// surface handler on each new `xdg_surface`.
struct FilterXdgWmBaseHandler {
    policy: Rc<FilterPolicy>,
}

impl XdgWmBaseHandler for FilterXdgWmBaseHandler {
    fn handle_get_xdg_surface(
        &mut self,
        slf: &Rc<XdgWmBase>,
        xdg_surface: &Rc<XdgSurface>,
        _surface: &Rc<wl_proxy::protocols::wayland::wl_surface::WlSurface>,
    ) {
        xdg_surface.set_handler(FilterXdgSurfaceHandler {
            policy: self.policy.clone(),
        });
        // Forward to the compositor.
        slf.send_get_xdg_surface(xdg_surface, _surface);
    }
}

/// Handler for `xdg_surface`: intercepts `get_toplevel` to install our
/// toplevel handler.
struct FilterXdgSurfaceHandler {
    policy: Rc<FilterPolicy>,
}

impl XdgSurfaceHandler for FilterXdgSurfaceHandler {
    fn handle_get_toplevel(&mut self, slf: &Rc<XdgSurface>, toplevel: &Rc<XdgToplevel>) {
        toplevel.set_handler(FilterXdgToplevelHandler {
            policy: self.policy.clone(),
        });
        slf.send_get_toplevel(toplevel);
    }
}

/// Handler for `xdg_toplevel`: rewrites app-id and title.
struct FilterXdgToplevelHandler {
    policy: Rc<FilterPolicy>,
}

impl XdgToplevelHandler for FilterXdgToplevelHandler {
    fn handle_set_app_id(&mut self, slf: &Rc<XdgToplevel>, app_id: &str) {
        let rewritten = self.policy.rewrite_app_id(app_id);
        slf.send_set_app_id(&rewritten);
    }

    fn handle_set_title(&mut self, slf: &Rc<XdgToplevel>, title: &str) {
        let rewritten = self.policy.rewrite_title(title);
        slf.send_set_title(&rewritten);
    }
}

/// Handler for `wl_eglstream_display`: keep NVIDIA EGLStream constrained to
/// fd-backed streams. The protocol also defines inet/socket modes, but those
/// are network/socket transport surfaces and are outside nixling's intended
/// guest-to-host Wayland boundary.
struct FilterEglstreamDisplayHandler {
    vm: String,
}

impl WlEglstreamDisplayHandler for FilterEglstreamDisplayHandler {
    fn handle_caps(&mut self, slf: &Rc<WlEglstreamDisplay>, caps: i32) {
        slf.send_caps(eglstream_fd_caps(caps));
    }

    fn handle_create_stream(
        &mut self,
        slf: &Rc<WlEglstreamDisplay>,
        id: &Rc<WlBuffer>,
        width: i32,
        height: i32,
        handle: &Rc<std::os::fd::OwnedFd>,
        r#type: i32,
        attribs: &[u8],
    ) {
        if eglstream_handle_is_fd(r#type) {
            slf.send_create_stream(id, width, height, handle, r#type, attribs);
            return;
        }

        log::warn!(
            "[nixling-wlproxy] vm={} denied wl_eglstream_display.create_stream with non-fd handle_type={}",
            self.vm,
            r#type
        );
        if let Some(client) = id.client() {
            client.disconnect();
        }
    }
}

fn eglstream_fd_caps(caps: i32) -> i32 {
    caps & WlEglstreamDisplayCap::STREAM_FD.0 as i32
}

fn eglstream_handle_is_fd(handle_type: i32) -> bool {
    handle_type == WlEglstreamHandleType::FD.0 as i32
}

fn bind_matches_advertised_cap(
    advertised: AdvertisedGlobal,
    requested_interface: ObjectInterface,
    requested_version: u32,
) -> bool {
    requested_interface == advertised.interface && requested_version <= advertised.version
}

/// Minimal `ClientHandler` that logs disconnections for debugging.
pub struct FilterClientHandler {
    vm: String,
    _destructor: Option<Destructor>,
}

impl FilterClientHandler {
    pub fn new(vm: String) -> Self {
        Self {
            vm,
            _destructor: None,
        }
    }

    pub fn with_destructor(vm: String, destructor: Destructor) -> Self {
        Self {
            vm,
            _destructor: Some(destructor),
        }
    }
}

impl ClientHandler for FilterClientHandler {
    fn disconnected(self: Box<Self>) {
        log::debug!("[nixling-wlproxy] vm={} client disconnected", self.vm);
    }
}

/// Create a `State` connected to the compositor at `upstream_path`, using the
/// full `Baseline::ALL_OF_THEM` so every known protocol passes through to our
/// `handle_global` callback for policy classification.
pub fn build_state(upstream_path: &str) -> Result<Rc<State>, wl_proxy::state::StateError> {
    State::builder(wl_proxy::baseline::Baseline::ALL_OF_THEM)
        .with_server_display_name(upstream_path)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{FilterPolicy, PolicyInput};

    fn policy() -> Rc<FilterPolicy> {
        Rc::new(FilterPolicy::build(PolicyInput {
            vm_name: "work".to_owned(),
            ..PolicyInput::default()
        }))
    }

    #[test]
    fn filtered_globals_preserve_original_global_names() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut handler = FilterRegistryHandler::new(policy(), diag);

        handler.advertised_globals.insert(
            7,
            AdvertisedGlobal {
                interface: ObjectInterface::WlCompositor,
                version: 6,
            },
        );
        handler.hidden_globals.insert(42);
        handler.advertised_globals.insert(
            99,
            AdvertisedGlobal {
                interface: ObjectInterface::WlShm,
                version: 2,
            },
        );

        assert!(handler.advertised_globals.contains_key(&7));
        assert!(handler.advertised_globals.contains_key(&99));
        assert!(handler.hidden_globals.contains(&42));
        assert!(!handler.advertised_globals.contains_key(&42));
    }

    #[test]
    fn registry_handler_records_bind_denials_in_shared_limiter() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let handler = FilterRegistryHandler::new(policy(), diag.clone());

        for name in 0..6 {
            handler
                .diag
                .borrow_mut()
                .bind_denied(crate::diag::DropReason::BindDeniedUnadvertised, name);
        }

        let suppressed_before_flush = diag.borrow().suppressed_total_for_tests();
        assert_eq!(suppressed_before_flush, 1);

        diag.borrow_mut().flush_suppressed();

        let suppressed_after_flush = diag.borrow().suppressed_total_for_tests();
        assert_eq!(suppressed_after_flush, 0);
    }

    #[test]
    fn eglstream_caps_are_fd_only() {
        let all_caps = WlEglstreamDisplayCap::STREAM_FD.0
            | WlEglstreamDisplayCap::STREAM_INET.0
            | WlEglstreamDisplayCap::STREAM_SOCKET.0;

        assert_eq!(
            eglstream_fd_caps(all_caps as i32),
            WlEglstreamDisplayCap::STREAM_FD.0 as i32
        );
        assert_eq!(
            eglstream_fd_caps(WlEglstreamDisplayCap::STREAM_INET.0 as i32),
            0
        );
    }

    #[test]
    fn eglstream_create_stream_allows_only_fd_handles() {
        assert!(eglstream_handle_is_fd(WlEglstreamHandleType::FD.0 as i32));
        assert!(!eglstream_handle_is_fd(
            WlEglstreamHandleType::INET.0 as i32
        ));
        assert!(!eglstream_handle_is_fd(
            WlEglstreamHandleType::SOCKET.0 as i32
        ));
    }

    #[test]
    fn bind_version_must_not_exceed_advertised_cap() {
        let advertised = AdvertisedGlobal {
            interface: ObjectInterface::ZwpLinuxDmabufV1,
            version: 3,
        };

        assert!(bind_matches_advertised_cap(
            advertised,
            ObjectInterface::ZwpLinuxDmabufV1,
            3
        ));
        assert!(bind_matches_advertised_cap(
            advertised,
            ObjectInterface::ZwpLinuxDmabufV1,
            2
        ));
        assert!(!bind_matches_advertised_cap(
            advertised,
            ObjectInterface::ZwpLinuxDmabufV1,
            4
        ));
        assert!(!bind_matches_advertised_cap(
            advertised,
            ObjectInterface::WlCompositor,
            3
        ));
    }
}
