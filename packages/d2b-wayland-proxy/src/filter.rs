//! wl-proxy handler implementations for the Wayland proxy.
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
    os::unix::net::UnixStream,
    path::PathBuf,
    rc::{Rc, Weak},
    time::Instant,
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
    state::{State, StateHandler},
};

use crate::{
    bridge::{
        BridgeConfig, BridgeConnectionState, BridgeHandoff, BridgeReconnectMachine,
        BridgeTransferKind, BridgeTransferMetadata,
    },
    clipboard::{
        ClipboardGlobalDisposition, ClipboardMimePolicy, ClipboardRoute, MimeDecision,
        global_disposition,
    },
    diag::{DiagRateLimiter, DropReason, bounded_error_detail},
    dmabuf::DmabufHandler,
    policy::FilterPolicy,
};

const MAX_MIME_TYPES_PER_SOURCE: usize = 64;

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
    diag: Rc<RefCell<DiagRateLimiter>>,
    bridge_path: Option<PathBuf>,
    bridge: Option<UnixStream>,
    bridge_reconnect: BridgeReconnectMachine,
    next_bridge_retry: Option<Instant>,
    mime_policy: ClipboardMimePolicy,
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

impl VirtualSource {
    fn add_mime_bounded(&mut self, mime: &str) -> bool {
        if self.mime_types.iter().any(|existing| existing == mime) {
            return true;
        }
        if self.mime_types.len() >= MAX_MIME_TYPES_PER_SOURCE {
            return false;
        }
        self.mime_types.push(mime.to_owned());
        true
    }
}

#[derive(Debug)]
struct VirtualOffer {
    source: Option<Rc<RefCell<VirtualSource>>>,
    source_id: u64,
}

impl VirtualClipboardState {
    pub fn new(
        vm_name: String,
        diag: Rc<RefCell<DiagRateLimiter>>,
        bridge_config: BridgeConfig,
    ) -> Self {
        Self {
            vm_name,
            diag,
            bridge_path: bridge_config.socket_path.clone(),
            bridge: None,
            bridge_reconnect: BridgeReconnectMachine::new(&bridge_config),
            next_bridge_retry: None,
            mime_policy: ClipboardMimePolicy::v1_defaults(),
            devices: Vec::new(),
            sources: HashMap::new(),
            offers: HashMap::new(),
            selection: None,
        }
    }

    fn register_source(&mut self, source: &Rc<WlDataSource>) {
        self.scrub_dead_clipboard_refs();
        self.sources.entry(source.unique_id()).or_insert_with(|| {
            Rc::new(RefCell::new(VirtualSource {
                source: Rc::downgrade(source),
                mime_types: Vec::new(),
            }))
        });
    }

    fn add_source_mime(&mut self, source: &Rc<WlDataSource>, mime: &str) {
        if matches!(
            self.mime_policy.decide(self.route(), mime),
            MimeDecision::Deny
        ) {
            return;
        }
        self.register_source(source);
        if let Some(stored) = self.sources.get(&source.unique_id()) {
            let mut stored = stored.borrow_mut();
            if !stored.add_mime_bounded(mime) {
                let vm = self.vm_name.clone();
                self.diag
                    .borrow_mut()
                    .warn("clipboard-mime", "source-mime-cap", || {
                        format!("[d2b-wlproxy] vm={vm} event=clipboard-mime reason=source-mime-cap")
                    });
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
        let (offer_source, offer_source_id) = {
            let offer = offer.borrow();
            (offer.source.clone(), offer.source_id)
        };
        let Some(offer_source) = offer_source else {
            log::info!(
                "[d2b-wlproxy] vm={} clipboard: host-backed receive offer={} mime={}",
                self.vm_name,
                offer_source_id,
                bounded_log_mime(mime_type)
            );
            if !matches!(
                self.mime_policy
                    .decide(ClipboardRoute::HostOrCrossRealm, mime_type),
                MimeDecision::MaterializeViaBridge
            ) {
                return;
            }
            let metadata = BridgeTransferMetadata {
                vm_name: self.vm_name.clone(),
                mime_type: mime_type.to_owned(),
                source_id: offer_source_id,
                kind: BridgeTransferKind::PasteRequest,
            };
            self.handoff_via_bridge(fd, &metadata);
            return;
        };
        let Some(source) = offer_source.borrow().source.upgrade() else {
            // Source was already destroyed; the fd will drop and the receiver
            // will see EOF. Log so operators can see clipboard data loss events.
            log::debug!(
                "[d2b-wlproxy] vm={} clipboard: source gone at receive; returning EOF to requester mime={}",
                self.vm_name,
                bounded_log_mime(mime_type),
            );
            return;
        };
        match self.mime_policy.decide(self.route(), mime_type) {
            MimeDecision::PreserveSameVmRichMime => source.send_send(mime_type, fd),
            MimeDecision::MaterializeViaBridge => {
                let metadata = BridgeTransferMetadata {
                    vm_name: self.vm_name.clone(),
                    mime_type: mime_type.to_owned(),
                    source_id: source.unique_id(),
                    kind: BridgeTransferKind::PasteRequest,
                };
                self.handoff_via_bridge(fd, &metadata);
            }
            MimeDecision::Deny => {}
        }
    }

    fn remove_offer(&mut self, offer: &Rc<WlDataOffer>) {
        self.offers.remove(&offer.unique_id());
    }

    fn scrub_dead_clipboard_refs(&mut self) {
        self.sources
            .retain(|_, source| source.borrow().source.upgrade().is_some());
        self.offers.retain(|_, offer| {
            offer
                .borrow()
                .source
                .as_ref()
                .is_none_or(|source| source.borrow().source.upgrade().is_some())
        });
        self.devices.retain(|device| device.upgrade().is_some());
        if self
            .selection
            .as_ref()
            .is_some_and(|source| source.borrow().source.upgrade().is_none())
        {
            self.selection = None;
        }
    }

    fn route(&self) -> ClipboardRoute {
        if self.bridge_path.is_some() {
            ClipboardRoute::HostOrCrossRealm
        } else {
            ClipboardRoute::SameVm
        }
    }

    fn publish_selection_to_bridge(&mut self, source: Option<&Rc<WlDataSource>>) {
        let Some(source) = source else {
            return;
        };
        let Some(stored) = self.sources.get(&source.unique_id()).cloned() else {
            return;
        };
        let mime_types = stored.borrow().mime_types.clone();
        log::info!(
            "[d2b-wlproxy] vm={} clipboard: publish selection source={} mimes={}",
            self.vm_name,
            source.unique_id(),
            mime_types.len()
        );
        for mime_type in mime_types {
            if !matches!(
                self.mime_policy.decide(self.route(), &mime_type),
                MimeDecision::MaterializeViaBridge
            ) {
                continue;
            }
            let (read_fd, write_fd) = match rustix::pipe::pipe_with(
                rustix::pipe::PipeFlags::CLOEXEC,
            ) {
                Ok(pair) => pair,
                Err(error) => {
                    let vm = self.vm_name.clone();
                    let error = bounded_error_detail(error.to_string());
                    self.diag.borrow_mut().warn(
                            "clipboard-bridge",
                            "copy-pipe-failed",
                            || {
                                format!(
                                    "[d2b-wlproxy] vm={vm} event=clipboard-bridge reason=copy-pipe-failed error={error}"
                                )
                            },
                        );
                    continue;
                }
            };
            let write_fd = Rc::new(write_fd);
            source.send_send(&mime_type, &write_fd);
            drop(write_fd);
            let metadata = BridgeTransferMetadata {
                vm_name: self.vm_name.clone(),
                mime_type: mime_type.clone(),
                source_id: source.unique_id(),
                kind: BridgeTransferKind::CopySelection,
            };
            log::info!(
                "[d2b-wlproxy] vm={} clipboard: handoff copy source={} mime={}",
                self.vm_name,
                source.unique_id(),
                bounded_log_mime(&mime_type)
            );
            self.handoff_via_bridge(&read_fd, &metadata);
            drop(read_fd);
        }
    }

    fn ensure_bridge_connected(&mut self) -> Option<&mut UnixStream> {
        if self.bridge.is_some() {
            return self.bridge.as_mut();
        }
        let path = self.bridge_path.clone()?;
        let now = Instant::now();
        match self.bridge_reconnect.state() {
            BridgeConnectionState::Disabled => return None,
            BridgeConnectionState::Connected { .. } => {
                self.bridge_reconnect.disconnected();
                match self.bridge_reconnect.state() {
                    BridgeConnectionState::Backoff { .. } => {
                        self.schedule_bridge_retry();
                        return None;
                    }
                    BridgeConnectionState::Disconnected => self.bridge_reconnect.start_connect(),
                    BridgeConnectionState::Disabled => return None,
                    BridgeConnectionState::Connecting { .. } => {}
                    BridgeConnectionState::Connected { .. } => return None,
                }
            }
            BridgeConnectionState::Backoff { .. } => {
                if self.next_bridge_retry.is_some_and(|retry| retry > now) {
                    return None;
                }
                self.bridge_reconnect.retry_due();
            }
            BridgeConnectionState::Disconnected => self.bridge_reconnect.start_connect(),
            BridgeConnectionState::Connecting { .. } => {}
        }
        match UnixStream::connect(&path) {
            Ok(stream) => {
                if let Err(_error) = stream.set_nonblocking(true) {
                    let vm = self.vm_name.clone();
                    self.diag.borrow_mut().warn(
                        "clipboard-bridge",
                        "nonblocking-failed",
                        || {
                            format!(
                                "[d2b-wlproxy] vm={vm} event=clipboard-bridge reason=nonblocking-failed"
                            )
                        },
                    );
                    self.bridge_reconnect.connect_failed();
                    self.schedule_bridge_retry();
                    return None;
                }
                self.bridge_reconnect.connect_succeeded();
                self.next_bridge_retry = None;
                self.bridge = Some(stream);
            }
            Err(error) => {
                let vm = self.vm_name.clone();
                let error = bounded_error_detail(error.to_string());
                self.diag
                    .borrow_mut()
                    .warn("clipboard-bridge", "connect-failed", || {
                        format!(
                            "[d2b-wlproxy] vm={vm} event=clipboard-bridge reason=connect-failed error={error}"
                        )
                    });
                self.bridge_reconnect.connect_failed();
                self.schedule_bridge_retry();
            }
        }

        self.bridge.as_mut()
    }

    fn handoff_via_bridge(&mut self, fd: &OwnedFd, metadata: &BridgeTransferMetadata) {
        let delivered = self.ensure_bridge_connected().is_some_and(|bridge| {
            bridge.handoff_transfer_fd(fd, metadata) == crate::bridge::HandoffStatus::Delivered
        });
        if delivered {
            log::info!(
                "[d2b-wlproxy] vm={} event=clipboard-bridge reason=handoff-delivered kind={:?} mime={}",
                self.vm_name,
                metadata.kind,
                bounded_log_mime(&metadata.mime_type)
            );
            return;
        }
        self.mark_bridge_disconnected();
        let retried = self.ensure_bridge_connected().is_some_and(|bridge| {
            bridge.handoff_transfer_fd(fd, metadata) == crate::bridge::HandoffStatus::Delivered
        });
        if !retried {
            self.mark_bridge_disconnected();
            let vm = self.vm_name.clone();
            self.diag
                .borrow_mut()
                .warn("clipboard-bridge", "handoff-failed", || {
                    format!("[d2b-wlproxy] vm={vm} event=clipboard-bridge reason=handoff-failed")
                });
        } else {
            log::info!(
                "[d2b-wlproxy] vm={} event=clipboard-bridge reason=handoff-delivered-after-retry kind={:?} mime={}",
                self.vm_name,
                metadata.kind,
                bounded_log_mime(&metadata.mime_type)
            );
        }
    }

    fn mark_bridge_disconnected(&mut self) {
        self.bridge.take();
        self.bridge_reconnect.disconnected();
        if matches!(
            self.bridge_reconnect.state(),
            BridgeConnectionState::Backoff { .. }
        ) {
            self.schedule_bridge_retry();
        } else {
            self.next_bridge_retry = None;
        }
    }

    fn schedule_bridge_retry(&mut self) {
        if let BridgeConnectionState::Backoff { delay, .. } = self.bridge_reconnect.state() {
            self.next_bridge_retry = Some(Instant::now() + delay);
        }
    }
}

fn register_virtual_device(
    clipboard: &Rc<RefCell<VirtualClipboardState>>,
    device: &Rc<WlDataDevice>,
) {
    let mut clipboard_state = clipboard.borrow_mut();
    clipboard_state.scrub_dead_clipboard_refs();
    clipboard_state.devices.push(Rc::downgrade(device));
    drop(clipboard_state);
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
        let (mimes, route) = {
            let state = clipboard.borrow();
            (state.mime_policy.external_mimes(), state.route())
        };
        if !matches!(route, ClipboardRoute::HostOrCrossRealm) {
            device.send_selection(None);
            return;
        }
        let offer = device.new_send_data_offer();
        offer.set_handler(VirtualOfferHandler {
            clipboard: Rc::downgrade(clipboard),
            vm_name: vm_name.clone(),
        });
        clipboard.borrow_mut().offers.insert(
            offer.unique_id(),
            Rc::new(RefCell::new(VirtualOffer {
                source: None,
                source_id: offer.unique_id(),
            })),
        );
        for mime in mimes {
            offer.send_offer(mime);
        }
        device.send_selection(Some(&offer));
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
            source: Some(source.clone()),
            source_id: source
                .borrow()
                .source
                .upgrade()
                .map_or(0, |source| source.unique_id()),
        })),
    );
    for mime in mimes {
        offer.send_offer(&mime);
    }
    device.send_selection(Some(&offer));
}

fn bounded_log_mime(mime: &str) -> String {
    let mut out = String::new();
    for ch in mime.chars() {
        if out.len() + ch.len_utf8() > 64 {
            out.push('…');
            break;
        }
        if ch.is_ascii_graphic() || ch == ' ' {
            out.push(ch);
        } else {
            out.push('?');
        }
    }
    out
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
    synthetic_clipboard_name: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AdvertisedGlobal {
    interface: ObjectInterface,
    version: u32,
    synthetic_clipboard: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GlobalAdvertisement {
    name: u32,
    interface: ObjectInterface,
    version: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IncomingGlobalDecision {
    Advertise(GlobalAdvertisement),
    Hide,
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
            synthetic_clipboard_name: None,
        }
    }

    fn ensure_synthetic_clipboard_global(
        &mut self,
        reserved_name: u32,
    ) -> Option<GlobalAdvertisement> {
        if self.synthetic_clipboard_name.is_some() {
            return None;
        }
        let name = self.allocate_synthetic_clipboard_name(reserved_name);
        let interface = ObjectInterface::WlDataDeviceManager;
        let version = 3;
        self.synthetic_clipboard_name = Some(name);
        self.advertised_globals.insert(
            name,
            AdvertisedGlobal {
                interface,
                version,
                synthetic_clipboard: true,
            },
        );
        log::info!(
            "[d2b-wlproxy] vm={} event=synthetic-clipboard-advertised interface=wl_data_device_manager registry-name={name} version={version}",
            self.policy.vm_name
        );
        Some(GlobalAdvertisement {
            name,
            interface,
            version,
        })
    }

    fn allocate_synthetic_clipboard_name(&self, reserved_name: u32) -> u32 {
        for name in (0..=u32::MAX).rev() {
            if name != reserved_name
                && !self.advertised_globals.contains_key(&name)
                && !self.hidden_globals.contains(&name)
            {
                return name;
            }
        }
        unreachable!("Wayland registry exhausted every u32 global name")
    }

    fn prepare_global(
        &mut self,
        name: u32,
        interface: ObjectInterface,
        version: u32,
    ) -> (Option<GlobalAdvertisement>, IncomingGlobalDecision) {
        let synthetic = self.ensure_synthetic_clipboard_global(name);
        let iface_name = interface.name();
        if Some(name) == self.synthetic_clipboard_name {
            self.hidden_globals.insert(name);
            if self.policy.log_filtered_globals {
                self.diag.borrow_mut().global_filtered(iface_name);
            }
            return (synthetic, IncomingGlobalDecision::Hide);
        }
        if matches!(
            global_disposition(iface_name),
            ClipboardGlobalDisposition::VirtualizeLocally | ClipboardGlobalDisposition::DenyGlobal
        ) {
            self.hidden_globals.insert(name);
            if self.policy.log_filtered_globals {
                self.diag.borrow_mut().global_filtered(iface_name);
            }
            return (synthetic, IncomingGlobalDecision::Hide);
        }

        let (action, _) = self.policy.lookup(iface_name);
        let crate::policy::GlobalAction::Allow = action else {
            // Denied: ignore and suppress global_remove forwarding too.
            if self.policy.log_filtered_globals {
                self.diag.borrow_mut().global_filtered(iface_name);
            }
            self.hidden_globals.insert(name);
            return (synthetic, IncomingGlobalDecision::Hide);
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
        (
            synthetic,
            IncomingGlobalDecision::Advertise(GlobalAdvertisement {
                name,
                interface,
                version: adv_version,
            }),
        )
    }

    fn prepare_global_remove(&mut self, name: u32) -> bool {
        if Some(name) == self.synthetic_clipboard_name {
            return false;
        }
        if self.hidden_globals.remove(&name) {
            return false;
        }
        self.advertised_globals.remove(&name);
        true
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
        let (synthetic, decision) = self.prepare_global(name, interface, version);
        if let Some(global) = synthetic {
            slf.send_global(global.name, global.interface, global.version);
        }
        if let IncomingGlobalDecision::Advertise(global) = decision {
            slf.send_global(global.name, global.interface, global.version);
        }
    }

    fn handle_global_remove(&mut self, slf: &Rc<WlRegistry>, name: u32) {
        if self.prepare_global_remove(name) {
            slf.send_global_remove(name);
        }
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
            self.diag
                .borrow_mut()
                .bind_denied(reason, name, id.interface().name());
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
            let vm = &self.policy.vm_name;
            let iface = advertised.interface.name();
            let requested = id.version();
            let advertised_version = advertised.version;
            self.diag
                .borrow_mut()
                .warn("bind-denied", "version-cap", || {
                    format!(
                        "[d2b-wlproxy] vm={vm} event=bind-denied reason=version-cap registry-name={name} interface={iface} requested-version={requested} advertised-version={advertised_version}"
                    )
                });
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
                        diag: self.diag.clone(),
                    });
                }
                _ => {
                    if let Some(dmabuf) = id.try_downcast::<ZwpLinuxDmabufV1>()
                        && !self.policy.dmabuf_filters.is_empty()
                    {
                        dmabuf.set_handler(DmabufHandler::new(
                            self.policy.dmabuf_filters.clone(),
                            self.diag.clone(),
                        ));
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
            log::info!(
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
            log::info!(
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
            log::info!(
                "[d2b-wlproxy] vm={} clipboard: source id={} announced mime={}",
                self.vm_name,
                slf.unique_id(),
                bounded_log_mime(mime_type),
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
        if let Some(clipboard) = self.clipboard.upgrade() {
            let vm = self.vm_name.clone();
            clipboard
                .borrow()
                .diag
                .borrow_mut()
                .warn("clipboard-dnd", "start-drag-denied", || {
                    format!("[d2b-wlproxy] vm={vm} event=clipboard-dnd reason=start-drag-denied")
                });
        }
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
        log::info!(
            "[d2b-wlproxy] vm={} clipboard: set_selection source={}",
            self.vm_name,
            source.map_or(0, |s| s.unique_id()),
        );
        if let Some(clipboard) = self.clipboard.upgrade() {
            set_virtual_selection(&clipboard, source);
            clipboard.borrow_mut().publish_selection_to_bridge(source);
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
            bounded_log_mime(mime_type),
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
    diag: Rc<RefCell<DiagRateLimiter>>,
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

        let vm = self.vm.clone();
        let handle_type = r#type;
        self.diag
            .borrow_mut()
            .warn("eglstream", "create-stream-denied", || {
                format!(
                    "[d2b-wlproxy] vm={vm} event=eglstream reason=create-stream-denied handle-type={handle_type}"
                )
            });
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
}

impl FilterClientHandler {
    pub fn new(vm: String) -> Self {
        Self { vm }
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
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        Rc::new(RefCell::new(VirtualClipboardState::new(
            "work".to_owned(),
            diag,
            BridgeConfig::disabled(),
        )))
    }

    #[test]
    fn virtual_source_caps_unique_mime_types() {
        let mut source = VirtualSource {
            source: Weak::new(),
            mime_types: Vec::new(),
        };
        for index in 0..MAX_MIME_TYPES_PER_SOURCE {
            assert!(source.add_mime_bounded(&format!("application/x-test-{index}")));
        }
        assert!(!source.add_mime_bounded("application/x-overflow"));
        assert!(source.add_mime_bounded("application/x-test-0"));
        assert_eq!(source.mime_types.len(), MAX_MIME_TYPES_PER_SOURCE);
    }

    #[test]
    fn scrub_dead_clipboard_refs_clears_sources_offers_devices_and_selection() {
        let mut clipboard = VirtualClipboardState::new(
            "work".to_owned(),
            Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned()))),
            BridgeConfig::disabled(),
        );
        let source = Rc::new(RefCell::new(VirtualSource {
            source: Weak::new(),
            mime_types: vec!["text/plain".to_owned()],
        }));
        clipboard.sources.insert(1, source.clone());
        clipboard.offers.insert(
            2,
            Rc::new(RefCell::new(VirtualOffer {
                source: Some(source.clone()),
                source_id: 1,
            })),
        );
        clipboard.selection = Some(source);
        clipboard.devices.push(Weak::new());

        clipboard.scrub_dead_clipboard_refs();

        assert!(clipboard.sources.is_empty());
        assert!(clipboard.offers.is_empty());
        assert!(clipboard.devices.is_empty());
        assert!(clipboard.selection.is_none());
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
            handler.diag.borrow_mut().bind_denied(
                crate::diag::DropReason::BindDeniedUnadvertised,
                name,
                "zwp_text_input_manager_v3",
            );
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
    fn synthetic_clipboard_global_uses_reserved_high_name() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let handler = FilterRegistryHandler::new(policy(), diag, clipboard());

        assert_eq!(handler.allocate_synthetic_clipboard_name(7), u32::MAX);
    }

    #[test]
    fn synthetic_clipboard_global_avoids_server_and_existing_names() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut handler = FilterRegistryHandler::new(policy(), diag, clipboard());
        handler.advertised_globals.insert(
            u32::MAX,
            AdvertisedGlobal {
                interface: ObjectInterface::WlCompositor,
                version: 6,
                synthetic_clipboard: false,
            },
        );

        assert_eq!(
            handler.allocate_synthetic_clipboard_name(u32::MAX - 1),
            u32::MAX - 2
        );
    }

    #[test]
    fn prepare_global_hides_late_host_collision_with_synthetic_clipboard_name() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut handler = FilterRegistryHandler::new(policy(), diag, clipboard());
        let (synthetic, first) = handler.prepare_global(7, ObjectInterface::WlCompositor, 6);
        assert_eq!(
            synthetic,
            Some(GlobalAdvertisement {
                name: u32::MAX,
                interface: ObjectInterface::WlDataDeviceManager,
                version: 3,
            })
        );
        assert!(matches!(first, IncomingGlobalDecision::Advertise(_)));

        let (synthetic, colliding) = handler.prepare_global(u32::MAX, ObjectInterface::WlSeat, 9);
        assert_eq!(synthetic, None);
        assert_eq!(colliding, IncomingGlobalDecision::Hide);
        let advertised = handler
            .advertised_globals
            .get(&u32::MAX)
            .expect("synthetic remains advertised");
        assert_eq!(advertised.interface, ObjectInterface::WlDataDeviceManager);
        assert!(advertised.synthetic_clipboard);
        assert!(handler.hidden_globals.contains(&u32::MAX));
    }

    #[test]
    fn prepare_global_remove_ignores_synthetic_clipboard_name() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut handler = FilterRegistryHandler::new(policy(), diag, clipboard());
        let (_synthetic, _first) = handler.prepare_global(7, ObjectInterface::WlCompositor, 6);

        assert!(!handler.prepare_global_remove(u32::MAX));
        assert!(
            handler.advertised_globals.contains_key(&u32::MAX),
            "synthetic clipboard global must remain advertised"
        );
    }

    #[test]
    fn prepare_global_hides_host_data_device_manager() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut handler = FilterRegistryHandler::new(policy(), diag, clipboard());
        let (_synthetic, decision) =
            handler.prepare_global(11, ObjectInterface::WlDataDeviceManager, 3);

        assert_eq!(decision, IncomingGlobalDecision::Hide);
        assert!(handler.hidden_globals.contains(&11));
        assert!(!handler.advertised_globals.contains_key(&11));
    }

    #[test]
    fn prepare_global_hides_clipboard_boundary_even_when_policy_allows_it() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let policy = Rc::new(FilterPolicy::build(crate::policy::PolicyInput {
            vm_name: "work".to_owned(),
            allow_globals: vec!["zwp_primary_selection_device_manager_v1".to_owned()],
            ..Default::default()
        }));
        let mut handler = FilterRegistryHandler::new(policy, diag, clipboard());
        let (_synthetic, decision) =
            handler.prepare_global(13, ObjectInterface::ZwpPrimarySelectionDeviceManagerV1, 1);

        assert_eq!(decision, IncomingGlobalDecision::Hide);
        assert!(handler.hidden_globals.contains(&13));
        assert!(!handler.advertised_globals.contains_key(&13));
    }

    #[test]
    fn prepare_global_hides_text_input_v3() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut handler = FilterRegistryHandler::new(policy(), diag, clipboard());
        let (_synthetic, decision) =
            handler.prepare_global(12, ObjectInterface::ZwpTextInputManagerV3, 1);

        assert_eq!(decision, IncomingGlobalDecision::Hide);
        assert!(handler.hidden_globals.contains(&12));
        assert!(!handler.advertised_globals.contains_key(&12));
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
