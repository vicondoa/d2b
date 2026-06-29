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
    os::fd::OwnedFd,
    rc::{Rc, Weak},
};
use wl_proxy::{
    client::{Client, ClientHandler},
    object::{Object, ObjectCoreApi, ObjectRcUtils},
    protocols::{
        ObjectInterface,
        linux_dmabuf_v1::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        stream::{
            wl_eglstream::WlEglstreamHandleType,
            wl_eglstream_display::{
                WlEglstreamDisplay, WlEglstreamDisplayCap, WlEglstreamDisplayHandler,
            },
        },
        wayland::{
            wl_buffer::WlBuffer,
            wl_data_device::{WlDataDevice, WlDataDeviceHandler},
            wl_data_device_manager::{WlDataDeviceManager, WlDataDeviceManagerHandler},
            wl_data_offer::{WlDataOffer, WlDataOfferHandler},
            wl_data_source::{WlDataSource, WlDataSourceHandler},
            wl_display::{WlDisplay, WlDisplayHandler},
            wl_registry::{WlRegistry, WlRegistryHandler},
        },
        xdg_shell::{
            xdg_surface::{XdgSurface, XdgSurfaceHandler},
            xdg_toplevel::{XdgToplevel, XdgToplevelHandler},
            xdg_wm_base::{XdgWmBase, XdgWmBaseHandler},
        },
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
    clipboard: Rc<RefCell<VirtualClipboardState>>,
}

impl FilterStateHandler {
    pub fn new(
        policy: Rc<FilterPolicy>,
        diag: Rc<RefCell<DiagRateLimiter>>,
        clipboard: Rc<RefCell<VirtualClipboardState>>,
    ) -> Self {
        Self {
            policy,
            diag,
            clipboard,
        }
    }
}

impl StateHandler for FilterStateHandler {
    fn new_client(&mut self, client: &Rc<Client>) {
        install_client_handlers(
            client,
            self.policy.clone(),
            self.diag.clone(),
            self.clipboard.clone(),
        );
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
    clipboard: Rc<RefCell<VirtualClipboardState>>,
) {
    let handler = FilterDisplayHandler {
        policy: policy.clone(),
        diag,
        clipboard,
    };
    client.display().set_handler(handler);
    log::debug!("[d2b-wlproxy] vm={} new client connected", policy.vm_name);
}

/// Per-client display handler: intercepts `get_registry`.
struct FilterDisplayHandler {
    policy: Rc<FilterPolicy>,
    diag: Rc<RefCell<DiagRateLimiter>>,
    clipboard: Rc<RefCell<VirtualClipboardState>>,
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
            self.clipboard.clone(),
        ));
    }
}

#[derive(Debug)]
pub struct VirtualClipboardState {
    vm_name: String,
    devices: Vec<Weak<WlDataDevice>>,
    sources: HashMap<u64, Rc<RefCell<VirtualSource>>>,
    offers: HashMap<u64, Rc<RefCell<VirtualOffer>>>,
    selection: Option<Rc<RefCell<VirtualSource>>>,
}

#[derive(Debug)]
struct VirtualSource {
    source: Weak<WlDataSource>,
    mime_types: Vec<String>,
}

#[derive(Debug)]
struct VirtualOffer {
    source: Rc<RefCell<VirtualSource>>,
}

impl VirtualClipboardState {
    pub fn new(vm_name: String) -> Self {
        Self {
            vm_name,
            devices: Vec::new(),
            sources: HashMap::new(),
            offers: HashMap::new(),
            selection: None,
        }
    }

    fn register_source(&mut self, source: &Rc<WlDataSource>) {
        self.sources.entry(source.unique_id()).or_insert_with(|| {
            Rc::new(RefCell::new(VirtualSource {
                source: Rc::downgrade(source),
                mime_types: Vec::new(),
            }))
        });
    }

    fn add_source_mime(&mut self, source: &Rc<WlDataSource>, mime: &str) {
        self.register_source(source);
        if let Some(stored) = self.sources.get(&source.unique_id()) {
            let mut stored = stored.borrow_mut();
            if !stored.mime_types.iter().any(|existing| existing == mime) {
                stored.mime_types.push(mime.to_owned());
            }
        }
    }

    fn remove_source(&mut self, source: &Rc<WlDataSource>) -> bool {
        let id = source.unique_id();
        self.sources.remove(&id);
        if self.selection.as_ref().is_some_and(|selected| {
            selected
                .borrow()
                .source
                .upgrade()
                .is_some_and(|s| s.unique_id() == id)
        }) {
            self.selection = None;
            true
        } else {
            false
        }
    }

    fn set_selection(&mut self, source: Option<&Rc<WlDataSource>>) -> Option<Rc<WlDataSource>> {
        let old_strong = self
            .selection
            .as_ref()
            .and_then(|vs| vs.borrow().source.upgrade());
        self.selection = source.and_then(|source| self.sources.get(&source.unique_id()).cloned());
        // Return the old source only when it is being superseded by a different source
        // (or cleared), so the caller can send wl_data_source.cancelled.
        old_strong.filter(|old| source.is_none_or(|new| old.unique_id() != new.unique_id()))
    }

    fn receive_offer(&mut self, offer: &Rc<WlDataOffer>, mime_type: &str, fd: &Rc<OwnedFd>) {
        let Some(offer) = self.offers.get(&offer.unique_id()).cloned() else {
            return;
        };
        let Some(source) = offer.borrow().source.borrow().source.upgrade() else {
            // Source was already destroyed; the fd will drop and the receiver
            // will see EOF. Log so operators can see clipboard data loss events.
            log::debug!(
                "[d2b-wlproxy] vm={} clipboard: source gone at receive; returning EOF to requester mime={}",
                self.vm_name,
                mime_type,
            );
            return;
        };
        source.send_send(mime_type, fd);
    }

    fn remove_offer(&mut self, offer: &Rc<WlDataOffer>) {
        self.offers.remove(&offer.unique_id());
    }
}

fn register_virtual_device(
    clipboard: &Rc<RefCell<VirtualClipboardState>>,
    device: &Rc<WlDataDevice>,
) {
    clipboard.borrow_mut().devices.push(Rc::downgrade(device));
    send_selection_to_device(clipboard, device);
}

fn set_virtual_selection(
    clipboard: &Rc<RefCell<VirtualClipboardState>>,
    source: Option<&Rc<WlDataSource>>,
) {
    let old_source = clipboard.borrow_mut().set_selection(source);
    if let Some(old) = old_source {
        // Notify the owning app that its clipboard selection was superseded so it
        // can release any associated resources (Wayland protocol requirement).
        let vm_name = clipboard.borrow().vm_name.clone();
        log::debug!(
            "[d2b-wlproxy] vm={} clipboard: sending cancelled to superseded source id={}",
            vm_name,
            old.unique_id(),
        );
        old.send_cancelled();
    }
    broadcast_selection(clipboard);
}

fn remove_virtual_source(
    clipboard: &Rc<RefCell<VirtualClipboardState>>,
    source: &Rc<WlDataSource>,
) {
    let changed = clipboard.borrow_mut().remove_source(source);
    if changed {
        broadcast_selection(clipboard);
    }
}

fn broadcast_selection(clipboard: &Rc<RefCell<VirtualClipboardState>>) {
    let devices: Vec<Rc<WlDataDevice>> = {
        let mut state = clipboard.borrow_mut();
        state.devices.retain(|device| device.strong_count() > 0);
        state.devices.iter().filter_map(Weak::upgrade).collect()
    };
    for device in devices {
        send_selection_to_device(clipboard, &device);
    }
}

fn send_selection_to_device(
    clipboard: &Rc<RefCell<VirtualClipboardState>>,
    device: &Rc<WlDataDevice>,
) {
    let (vm_name, selection) = {
        let state = clipboard.borrow();
        (state.vm_name.clone(), state.selection.clone())
    };
    let Some(source) = selection else {
        device.send_selection(None);
        return;
    };
    let mimes = source.borrow().mime_types.clone();
    let offer = device.new_send_data_offer();
    offer.set_handler(VirtualOfferHandler {
        clipboard: Rc::downgrade(clipboard),
        vm_name: vm_name.clone(),
    });
    clipboard.borrow_mut().offers.insert(
        offer.unique_id(),
        Rc::new(RefCell::new(VirtualOffer {
            source: source.clone(),
        })),
    );
    for mime in mimes {
        offer.send_offer(&mime);
    }
    device.send_selection(Some(&offer));
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
    clipboard: Rc<RefCell<VirtualClipboardState>>,
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
    synthetic_clipboard: bool,
}

impl FilterRegistryHandler {
    pub fn new(
        policy: Rc<FilterPolicy>,
        diag: Rc<RefCell<DiagRateLimiter>>,
        clipboard: Rc<RefCell<VirtualClipboardState>>,
    ) -> Self {
        Self {
            policy,
            diag,
            clipboard,
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
        if iface_name == "wl_data_device_manager" {
            let adv_version = version.min(3);
            self.advertised_globals.insert(
                name,
                AdvertisedGlobal {
                    interface,
                    version: adv_version,
                    synthetic_clipboard: true,
                },
            );
            slf.send_global(name, interface, adv_version);
            return;
        }

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
                synthetic_clipboard: false,
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
                "[d2b-wlproxy] vm={} event=bind-denied reason=version-cap registry-name={} interface={} requested-version={} advertised-version={}",
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

        if advertised.synthetic_clipboard {
            let Some(manager) = id.try_downcast::<WlDataDeviceManager>() else {
                if let Some(client) = id.client() {
                    client.disconnect();
                }
                drop(id);
                return;
            };
            manager.set_forward_to_server(false);
            manager.set_handler(VirtualDataDeviceManagerHandler {
                clipboard: Rc::downgrade(&self.clipboard),
                vm_name: self.policy.vm_name.clone(),
            });
            return;
        }

        // Install per-interface handlers before forwarding, so we can
        // intercept the object's lifecycle from the first message.
        match id.try_downcast::<XdgWmBase>() {
            Some(wm_base) => {
                wm_base.set_handler(FilterXdgWmBaseHandler {
                    policy: self.policy.clone(),
                });
            }
            _ => match id.try_downcast::<WlEglstreamDisplay>() {
                Some(eglstream_display) => {
                    eglstream_display.set_handler(FilterEglstreamDisplayHandler {
                        vm: self.policy.vm_name.clone(),
                    });
                }
                _ => {
                    if let Some(dmabuf) = id.try_downcast::<ZwpLinuxDmabufV1>()
                        && !self.policy.dmabuf_filters.is_empty()
                    {
                        dmabuf.set_handler(DmabufHandler::new(self.policy.dmabuf_filters.clone()));
                    }
                }
            },
        }

        slf.send_bind(name, id);
    }
}

struct VirtualDataDeviceManagerHandler {
    clipboard: Weak<RefCell<VirtualClipboardState>>,
    vm_name: String,
}

impl WlDataDeviceManagerHandler for VirtualDataDeviceManagerHandler {
    fn handle_create_data_source(&mut self, _slf: &Rc<WlDataDeviceManager>, id: &Rc<WlDataSource>) {
        id.set_forward_to_server(false);
        id.set_handler(VirtualDataSourceHandler {
            clipboard: self.clipboard.clone(),
            vm_name: self.vm_name.clone(),
        });
        if let Some(clipboard) = self.clipboard.upgrade() {
            clipboard.borrow_mut().register_source(id);
            log::debug!(
                "[d2b-wlproxy] vm={} clipboard: source created id={}",
                self.vm_name,
                id.unique_id(),
            );
        }
    }

    fn handle_get_data_device(
        &mut self,
        _slf: &Rc<WlDataDeviceManager>,
        id: &Rc<WlDataDevice>,
        _seat: &Rc<wl_proxy::protocols::wayland::wl_seat::WlSeat>,
    ) {
        id.set_forward_to_server(false);
        id.set_forward_to_client(false);
        id.set_handler(VirtualDataDeviceHandler {
            clipboard: self.clipboard.clone(),
            vm_name: self.vm_name.clone(),
        });
        if let Some(clipboard) = self.clipboard.upgrade() {
            register_virtual_device(&clipboard, id);
            log::debug!(
                "[d2b-wlproxy] vm={} clipboard: device registered id={}",
                self.vm_name,
                id.unique_id(),
            );
        }
    }

    fn handle_release(&mut self, slf: &Rc<WlDataDeviceManager>) {
        slf.delete_id();
    }
}

struct VirtualDataSourceHandler {
    clipboard: Weak<RefCell<VirtualClipboardState>>,
    vm_name: String,
}

impl WlDataSourceHandler for VirtualDataSourceHandler {
    fn handle_offer(&mut self, slf: &Rc<WlDataSource>, mime_type: &str) {
        if let Some(clipboard) = self.clipboard.upgrade() {
            clipboard.borrow_mut().add_source_mime(slf, mime_type);
            log::debug!(
                "[d2b-wlproxy] vm={} clipboard: source id={} announced mime={}",
                self.vm_name,
                slf.unique_id(),
                mime_type,
            );
        }
    }

    fn handle_destroy(&mut self, slf: &Rc<WlDataSource>) {
        log::debug!(
            "[d2b-wlproxy] vm={} clipboard: source destroyed id={}",
            self.vm_name,
            slf.unique_id(),
        );
        if let Some(clipboard) = self.clipboard.upgrade() {
            remove_virtual_source(&clipboard, slf);
        }
        slf.delete_id();
    }

    fn handle_set_actions(
        &mut self,
        _slf: &Rc<WlDataSource>,
        _dnd_actions: wl_proxy::protocols::wayland::wl_data_device_manager::WlDataDeviceManagerDndAction,
    ) {
        // DND is denied in v1; action negotiation never leaves the VM boundary.
    }
}

struct VirtualDataDeviceHandler {
    clipboard: Weak<RefCell<VirtualClipboardState>>,
    vm_name: String,
}

impl WlDataDeviceHandler for VirtualDataDeviceHandler {
    fn handle_start_drag(
        &mut self,
        slf: &Rc<WlDataDevice>,
        _source: Option<&Rc<WlDataSource>>,
        _origin: &Rc<wl_proxy::protocols::wayland::wl_surface::WlSurface>,
        _icon: Option<&Rc<wl_proxy::protocols::wayland::wl_surface::WlSurface>>,
        _serial: u32,
    ) {
        log::warn!(
            "[d2b-wlproxy] vm={} denied wl_data_device.start_drag; DND is not virtualized in clipboard v1",
            self.vm_name,
        );
        if let Some(client) = slf.client() {
            client.disconnect();
        }
    }

    fn handle_set_selection(
        &mut self,
        _slf: &Rc<WlDataDevice>,
        source: Option<&Rc<WlDataSource>>,
        _serial: u32,
    ) {
        log::debug!(
            "[d2b-wlproxy] vm={} clipboard: set_selection source={}",
            self.vm_name,
            source.map_or(0, |s| s.unique_id()),
        );
        if let Some(clipboard) = self.clipboard.upgrade() {
            set_virtual_selection(&clipboard, source);
        }
    }

    fn handle_release(&mut self, slf: &Rc<WlDataDevice>) {
        slf.delete_id();
    }
}

struct VirtualOfferHandler {
    clipboard: Weak<RefCell<VirtualClipboardState>>,
    vm_name: String,
}

impl WlDataOfferHandler for VirtualOfferHandler {
    fn handle_receive(&mut self, slf: &Rc<WlDataOffer>, mime_type: &str, fd: &Rc<OwnedFd>) {
        log::debug!(
            "[d2b-wlproxy] vm={} clipboard: receive offer id={} mime={}",
            self.vm_name,
            slf.unique_id(),
            mime_type,
        );
        if let Some(clipboard) = self.clipboard.upgrade() {
            clipboard.borrow_mut().receive_offer(slf, mime_type, fd);
        }
    }

    fn handle_destroy(&mut self, slf: &Rc<WlDataOffer>) {
        if let Some(clipboard) = self.clipboard.upgrade() {
            clipboard.borrow_mut().remove_offer(slf);
        }
        slf.delete_id();
    }

    fn handle_accept(&mut self, _slf: &Rc<WlDataOffer>, _serial: u32, _mime_type: Option<&str>) {}

    fn handle_finish(&mut self, _slf: &Rc<WlDataOffer>) {}

    fn handle_set_actions(
        &mut self,
        _slf: &Rc<WlDataOffer>,
        _dnd_actions: wl_proxy::protocols::wayland::wl_data_device_manager::WlDataDeviceManagerDndAction,
        _preferred_action: wl_proxy::protocols::wayland::wl_data_device_manager::WlDataDeviceManagerDndAction,
    ) {
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
/// are network/socket transport surfaces and are outside d2b's intended
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
            "[d2b-wlproxy] vm={} denied wl_eglstream_display.create_stream with non-fd handle_type={}",
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
        log::debug!("[d2b-wlproxy] vm={} client disconnected", self.vm);
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

    fn clipboard() -> Rc<RefCell<VirtualClipboardState>> {
        Rc::new(RefCell::new(VirtualClipboardState::new("work".to_owned())))
    }

    #[test]
    fn filtered_globals_preserve_original_global_names() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut handler = FilterRegistryHandler::new(policy(), diag, clipboard());

        handler.advertised_globals.insert(
            7,
            AdvertisedGlobal {
                interface: ObjectInterface::WlCompositor,
                version: 6,
                synthetic_clipboard: false,
            },
        );
        handler.hidden_globals.insert(42);
        handler.advertised_globals.insert(
            99,
            AdvertisedGlobal {
                interface: ObjectInterface::WlShm,
                version: 2,
                synthetic_clipboard: false,
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
        let handler = FilterRegistryHandler::new(policy(), diag.clone(), clipboard());

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
    fn standard_clipboard_global_is_advertised_as_synthetic() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut handler = FilterRegistryHandler::new(policy(), diag, clipboard());
        // The generated WlRegistry send path needs a real object/client, so assert
        // the policy decision helper that handle_global uses for the synthetic path.
        let interface = ObjectInterface::WlDataDeviceManager;
        assert_eq!(interface.name(), "wl_data_device_manager");
        handler.advertised_globals.insert(
            11,
            AdvertisedGlobal {
                interface,
                version: 3,
                synthetic_clipboard: true,
            },
        );
        let advertised = handler.advertised_globals.get(&11).expect("synthetic");
        assert!(advertised.synthetic_clipboard);
        assert_eq!(advertised.interface, ObjectInterface::WlDataDeviceManager);
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
            synthetic_clipboard: false,
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
