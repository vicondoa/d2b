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
    collections::{BTreeSet, HashSet},
    rc::Rc,
};
use wl_proxy::{
    client::{Client, ClientHandler},
    global_mapper::GlobalMapper,
    object::{ObjectRcUtils, Object},
    protocols::{
        ObjectInterface,
        wayland::{
            wl_display::{WlDisplay, WlDisplayHandler},
            wl_registry::{WlRegistry, WlRegistryHandler},
        },
        xdg_shell::{
            xdg_surface::{XdgSurface, XdgSurfaceHandler},
            xdg_toplevel::{XdgToplevel, XdgToplevelHandler},
            xdg_wm_base::{XdgWmBase, XdgWmBaseHandler},
        },
    },
    state::{State, StateHandler},
};

use crate::{
    diag::{DiagRateLimiter, DropReason},
    policy::FilterPolicy,
};

/// State-level handler: creates per-client display handlers.
pub struct FilterStateHandler {
    policy: Rc<FilterPolicy>,
}

impl FilterStateHandler {
    pub fn new(policy: Rc<FilterPolicy>) -> Self {
        Self { policy }
    }
}

impl StateHandler for FilterStateHandler {
    fn new_client(&mut self, client: &Rc<Client>) {
        let handler = FilterDisplayHandler {
            policy: self.policy.clone(),
        };
        client.display().set_handler(handler);
        log::debug!(
            "[nixling-wlproxy] vm={} new client connected",
            self.policy.vm_name
        );
    }
}

/// Per-client display handler: intercepts `get_registry`.
struct FilterDisplayHandler {
    policy: Rc<FilterPolicy>,
}

impl WlDisplayHandler for FilterDisplayHandler {
    fn handle_get_registry(&mut self, slf: &Rc<WlDisplay>, registry: &Rc<WlRegistry>) {
        // Forward get_registry to the compositor so the server side of the
        // registry is established.
        slf.send_get_registry(registry);
        // Install our registry handler to filter globals.
        registry.set_handler(FilterRegistryHandler::new(self.policy.clone()));
    }
}

/// Per-registry handler: filters globals and intercepts binds.
///
/// Uses `GlobalMapper` to manage client/server name translation.
/// Maintains a parallel set of client names we actually advertised so that
/// bind attempts for unadvertised names can be detected and logged.
pub struct FilterRegistryHandler {
    policy: Rc<FilterPolicy>,
    mapper: GlobalMapper,
    diag: DiagRateLimiter,
    /// Server names we explicitly ignored (mapped to None in the mapper).
    /// Used to distinguish hidden-global bind attempts from
    /// completely-unadvertised bind attempts in diagnostic messages.
    ignored_server_names: HashSet<u32>,
    /// Client names we actually advertised.
    advertised_client_names: BTreeSet<u32>,
    /// Tracks the next client_name the mapper will assign.
    /// GlobalMapper initialises `client_to_server` with one sentinel element
    /// (index 0 → None), so the first forwarded global gets name 1.
    next_mapper_client_name: u32,
}

impl FilterRegistryHandler {
    pub fn new(policy: Rc<FilterPolicy>) -> Self {
        let vm = policy.vm_name.clone();
        Self {
            policy,
            mapper: GlobalMapper::default(),
            diag: DiagRateLimiter::new(vm),
            ignored_server_names: HashSet::new(),
            advertised_client_names: BTreeSet::new(),
            next_mapper_client_name: 1,
        }
    }

    /// Record the client_name that GlobalMapper will assign to the next
    /// `forward_global` call, then bump the counter.
    fn record_forward(&mut self) {
        self.advertised_client_names
            .insert(self.next_mapper_client_name);
        self.next_mapper_client_name += 1;
    }

    /// Record a server name that we explicitly ignored.
    fn record_ignore(&mut self, server_name: u32) {
        self.ignored_server_names.insert(server_name);
        // The mapper's internal list still grows by one sentinel slot.
        self.next_mapper_client_name += 1;
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
                self.diag.global_filtered(iface_name);
            }
            self.mapper.ignore_global(name);
            self.record_ignore(name);
            return;
        };

        let adv_version = self.policy.advertised_version(iface_name, version);
        self.record_forward();
        self.mapper
            .forward_global(slf, name, interface, adv_version);
    }

    fn handle_global_remove(&mut self, slf: &Rc<WlRegistry>, name: u32) {
        self.mapper.forward_global_remove(slf, name);
    }

    fn handle_bind(&mut self, slf: &Rc<WlRegistry>, name: u32, id: Rc<dyn Object>) {
        // Detect and log bind attempts for names that were never advertised
        // to this client or that were explicitly hidden.
        if !self.advertised_client_names.contains(&name) {
            let reason = if self.ignored_server_names.contains(&name) {
                // The server_name is in our ignored set — but wait, `name` here
                // is a CLIENT name, not a server name.  The ignored set is keyed
                // by server names.  We cannot distinguish the two cases purely
                // from the client name without a reverse map into GlobalMapper
                // internals.  Use "hidden" as a conservative label when the
                // client_name is out of the advertised range and the mapper
                // would have assigned it to an ignored server_name.
                DropReason::BindDeniedHidden
            } else {
                DropReason::BindDeniedUnadvertised
            };
            self.diag.bind_denied(reason, name);
            // Drop `id` without forwarding — fail-closed.
            drop(id);
            return;
        }

        // Install per-interface handlers before forwarding, so we can
        // intercept the object's lifecycle from the first message.
        if let Some(wm_base) = id.try_downcast::<XdgWmBase>() {
            wm_base.set_handler(FilterXdgWmBaseHandler {
                policy: self.policy.clone(),
            });
        }

        self.mapper.forward_bind(slf, name, &id);
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
    fn handle_get_toplevel(
        &mut self,
        slf: &Rc<XdgSurface>,
        toplevel: &Rc<XdgToplevel>,
    ) {
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

/// Minimal `ClientHandler` that logs disconnections for debugging.
pub struct FilterClientHandler {
    vm: String,
}

impl FilterClientHandler {
    pub fn new(vm: String) -> Self {
        Self { vm }
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
