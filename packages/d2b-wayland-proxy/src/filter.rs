//! wl-proxy handler implementations for the Wayland proxy.
//!
//! Handler chain:
//!   FilterStateHandler
//!     -> FilterDisplayHandler (per client)
//!       -> FilterRegistryHandler (per wl_registry)
//!         -> FilterXdgWmBaseHandler (when xdg_wm_base is bound)
//!           -> FilterXdgSurfaceHandler (per xdg_surface)
//!             -> FilterXdgToplevelHandler (per xdg_toplevel)

use rustix::event::PollFlags;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    io::Read,
    os::{fd::OwnedFd, unix::net::UnixStream},
    path::PathBuf,
    rc::{Rc, Weak},
    time::Instant,
};
use wl_proxy::{
    client::{Client, ClientHandler},
    fixed::Fixed,
    object::{Object, ObjectCoreApi, ObjectRcUtils},
    protocols::{
        ObjectInterface,
        drm::wl_drm::{WlDrm, WlDrmHandler},
        linux_dmabuf_v1::zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        stream::{
            wl_eglstream::WlEglstreamHandleType,
            wl_eglstream_display::{
                WlEglstreamDisplay, WlEglstreamDisplayCap, WlEglstreamDisplayHandler,
            },
        },
        viewporter::{
            wp_viewport::{WpViewport, WpViewportHandler},
            wp_viewporter::{WpViewporter, WpViewporterHandler},
        },
        wayland::{
            wl_buffer::WlBuffer,
            wl_compositor::{WlCompositor, WlCompositorHandler},
            wl_data_device::{WlDataDevice, WlDataDeviceHandler},
            wl_data_device_manager::{WlDataDeviceManager, WlDataDeviceManagerHandler},
            wl_data_offer::{WlDataOffer, WlDataOfferHandler},
            wl_data_source::{WlDataSource, WlDataSourceHandler},
            wl_display::{WlDisplay, WlDisplayHandler},
            wl_keyboard::{
                WlKeyboard, WlKeyboardHandler, WlKeyboardKeyState, WlKeyboardKeymapFormat,
            },
            wl_output::{WlOutput, WlOutputTransform},
            wl_pointer::{
                WlPointer, WlPointerAxis, WlPointerAxisRelativeDirection, WlPointerAxisSource,
                WlPointerButtonState, WlPointerHandler,
            },
            wl_registry::{WlRegistry, WlRegistryHandler},
            wl_seat::{WlSeat, WlSeatHandler},
            wl_shm::{WlShm, WlShmHandler},
            wl_shm_pool::WlShmPool,
            wl_subcompositor::{WlSubcompositor, WlSubcompositorHandler},
            wl_subsurface::{WlSubsurface, WlSubsurfaceHandler},
            wl_surface::{WlSurface, WlSurfaceHandler},
            wl_touch::{WlTouch, WlTouchHandler},
        },
        xdg_shell::{
            xdg_popup::XdgPopup,
            xdg_positioner::{
                XdgPositioner, XdgPositionerAnchor, XdgPositionerConstraintAdjustment,
                XdgPositionerGravity, XdgPositionerHandler,
            },
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
        BridgeTransferKind, BridgeTransferMetadata, LocalTransferFd,
    },
    clipboard::{
        ClipboardGlobalDisposition, ClipboardMimePolicy, ClipboardRoute, MimeDecision,
        global_disposition,
    },
    decoration::{
        SharedDecorationManager, WRAPPER_RAIL_WIDTH, WindowGeometry, tracking_shm_pool_handler,
    },
    diag::{DiagRateLimiter, DropReason, bounded_error_detail},
    dmabuf::DmabufHandler,
    identity::ProxyIdentity,
    policy::FilterPolicy,
};

const MAX_MIME_TYPES_PER_SOURCE: usize = 64;
const MAX_PENDING_BRIDGE_HANDOFFS: usize = 64;

/// State-level handler: creates per-client display handlers.
pub struct FilterStateHandler {
    policy: Rc<FilterPolicy>,
    diag: Rc<RefCell<DiagRateLimiter>>,
    clipboard: Rc<RefCell<VirtualClipboardState>>,
    decoration: Option<SharedDecorationManager>,
}

impl FilterStateHandler {
    pub fn new(
        policy: Rc<FilterPolicy>,
        diag: Rc<RefCell<DiagRateLimiter>>,
        clipboard: Rc<RefCell<VirtualClipboardState>>,
        decoration: Option<SharedDecorationManager>,
    ) -> Self {
        Self {
            policy,
            diag,
            clipboard,
            decoration,
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
            self.decoration.clone(),
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
    decoration: Option<SharedDecorationManager>,
) {
    client.set_handler(FilterClientHandler::new(
        policy.vm_name.clone(),
        Rc::downgrade(client),
        decoration.clone(),
    ));
    let handler = FilterDisplayHandler {
        policy: policy.clone(),
        diag,
        clipboard,
        decoration,
    };
    client.display().set_handler(handler);
    log::debug!(
        "[d2b-wlproxy] target={} new client connected",
        policy.vm_name
    );
}

/// Per-client display handler: intercepts `get_registry`.
struct FilterDisplayHandler {
    policy: Rc<FilterPolicy>,
    diag: Rc<RefCell<DiagRateLimiter>>,
    clipboard: Rc<RefCell<VirtualClipboardState>>,
    decoration: Option<SharedDecorationManager>,
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
            self.decoration.clone(),
        ));
    }
}

#[derive(Debug)]
pub struct VirtualClipboardState {
    identity: ProxyIdentity,
    vm_name: String,
    diag: Rc<RefCell<DiagRateLimiter>>,
    bridge_path: Option<PathBuf>,
    bridge: Option<UnixStream>,
    bridge_read_buffer: Vec<u8>,
    bridge_reconnect: BridgeReconnectMachine,
    next_bridge_retry: Option<Instant>,
    pending_bridge_handoffs: VecDeque<PendingBridgeHandoff>,
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
    offer: Weak<WlDataOffer>,
    source: Option<Rc<RefCell<VirtualSource>>>,
    source_id: u64,
}

#[derive(Debug)]
struct PendingBridgeHandoff {
    fd: LocalTransferFd,
    metadata: BridgeTransferMetadata,
}

#[derive(Clone, Default)]
struct PositionerState {
    size: Option<(i32, i32)>,
    anchor_rect: Option<(i32, i32, i32, i32)>,
    anchor: Option<XdgPositionerAnchor>,
    gravity: Option<XdgPositionerGravity>,
    constraint_adjustment: Option<XdgPositionerConstraintAdjustment>,
    offset: Option<(i32, i32)>,
    reactive: bool,
    parent_size: Option<(i32, i32)>,
    parent_configure: Option<u32>,
}

impl PositionerState {
    fn apply_to(&self, positioner: &Rc<XdgPositioner>, x_offset: i32) {
        if let Some((width, height)) = self.size {
            positioner.send_set_size(width, height);
        }
        if let Some((x, y, width, height)) = self.anchor_rect {
            positioner.send_set_anchor_rect(x.saturating_add(x_offset), y, width, height);
        }
        if let Some(anchor) = self.anchor {
            positioner.send_set_anchor(anchor);
        }
        if let Some(gravity) = self.gravity {
            positioner.send_set_gravity(gravity);
        }
        if let Some(constraint_adjustment) = self.constraint_adjustment {
            positioner.send_set_constraint_adjustment(constraint_adjustment);
        }
        if let Some((x, y)) = self.offset {
            positioner.send_set_offset(x, y);
        }
        if self.reactive {
            positioner.send_set_reactive();
        }
        if let Some((width, height)) = self.parent_size {
            positioner.send_set_parent_size(width.saturating_add(x_offset), height);
        }
        if let Some(serial) = self.parent_configure {
            positioner.send_set_parent_configure(serial);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingHandoffStep {
    Continue,
    Stop,
}

impl VirtualClipboardState {
    pub fn new(
        identity: impl Into<ProxyIdentity>,
        diag: Rc<RefCell<DiagRateLimiter>>,
        bridge_config: BridgeConfig,
    ) -> Self {
        let identity = identity.into();
        let vm_name = identity.log_label();
        Self {
            identity,
            vm_name,
            diag,
            bridge_path: bridge_config.socket_path.clone(),
            bridge: None,
            bridge_read_buffer: Vec::new(),
            bridge_reconnect: BridgeReconnectMachine::new(&bridge_config),
            next_bridge_retry: None,
            pending_bridge_handoffs: VecDeque::new(),
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
                        format!(
                            "[d2b-wlproxy] target={vm} event=clipboard-mime reason=source-mime-cap"
                        )
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
            log::debug!(
                "[d2b-wlproxy] target={} clipboard: host-backed receive offer={} mime={}",
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
                identity: self.identity.clone(),
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
                "[d2b-wlproxy] target={} clipboard: source gone at receive; returning EOF to requester mime={}",
                self.vm_name,
                bounded_log_mime(mime_type),
            );
            return;
        };
        match self.mime_policy.decide(self.route(), mime_type) {
            MimeDecision::PreserveSameEndpointRichMime => source.send_send(mime_type, fd),
            MimeDecision::MaterializeViaBridge => {
                let metadata = BridgeTransferMetadata {
                    identity: self.identity.clone(),
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
            let offer = offer.borrow();
            offer.offer.upgrade().is_some()
                && offer
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
            ClipboardRoute::SameEndpoint
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
        log::debug!(
            "[d2b-wlproxy] target={} clipboard: publish selection source={} mimes={}",
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
                                    "[d2b-wlproxy] target={vm} event=clipboard-bridge reason=copy-pipe-failed error={error}"
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
                identity: self.identity.clone(),
                mime_type: mime_type.clone(),
                source_id: source.unique_id(),
                kind: BridgeTransferKind::CopySelection,
            };
            log::debug!(
                "[d2b-wlproxy] target={} clipboard: handoff copy source={} mime={}",
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
        match connect_bridge_nonblocking(&path) {
            Ok(stream) => {
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
                            "[d2b-wlproxy] target={vm} event=clipboard-bridge reason=connect-failed error={error}"
                        )
                    });
                self.bridge_reconnect.connect_failed();
                self.schedule_bridge_retry();
            }
        }

        self.bridge.as_mut()
    }

    fn handoff_via_bridge(&mut self, fd: &OwnedFd, metadata: &BridgeTransferMetadata) {
        self.flush_pending_bridge_handoffs();
        let local_fd = match rustix::io::fcntl_dupfd_cloexec(fd, 0) {
            Ok(fd) => LocalTransferFd::new(fd),
            Err(error) => {
                let vm = self.vm_name.clone();
                let error = bounded_error_detail(error.to_string());
                self.diag
                    .borrow_mut()
                    .warn("clipboard-bridge", "handoff-fd-dup-failed", || {
                        format!(
                            "[d2b-wlproxy] target={vm} event=clipboard-bridge reason=handoff-fd-dup-failed error={error}"
                        )
                    });
                return;
            }
        };
        if !self.pending_bridge_handoffs.is_empty() {
            self.enqueue_bridge_handoff(local_fd, metadata);
            self.ensure_bridge_connected();
            return;
        }
        let Some(bridge) = self.ensure_bridge_connected() else {
            self.enqueue_bridge_handoff(local_fd, metadata);
            return;
        };
        match bridge.handoff_transfer_fd(&local_fd, metadata) {
            crate::bridge::HandoffStatus::Delivered => {
                let _ = local_fd.close_after_handoff(crate::bridge::HandoffStatus::Delivered);
                log::debug!(
                    "[d2b-wlproxy] target={} event=clipboard-bridge reason=handoff-delivered kind={:?} mime={}",
                    self.vm_name,
                    metadata.kind,
                    bounded_log_mime(&metadata.mime_type)
                );
            }
            crate::bridge::HandoffStatus::Backpressure => {
                self.enqueue_bridge_handoff(local_fd, metadata);
            }
            crate::bridge::HandoffStatus::Failed(error) => {
                let status = crate::bridge::HandoffStatus::Failed(error);
                let error = match status {
                    crate::bridge::HandoffStatus::Failed(error) => error,
                    _ => unreachable!(),
                };
                let _ = local_fd.close_after_handoff(crate::bridge::HandoffStatus::Failed(error));
                self.mark_bridge_disconnected();
                self.ensure_bridge_connected();
                let vm = self.vm_name.clone();
                let kind = metadata.kind;
                let mime = bounded_log_mime(&metadata.mime_type);
                let error = handoff_error_detail(error);
                self.diag
                    .borrow_mut()
                    .warn("clipboard-bridge", "handoff-deferred", || {
                        format!(
                            "[d2b-wlproxy] target={vm} event=clipboard-bridge reason=handoff-deferred kind={kind:?} mime={mime} error={error}"
                        )
                    });
            }
        }
    }

    fn enqueue_bridge_handoff(&mut self, fd: LocalTransferFd, metadata: &BridgeTransferMetadata) {
        if self.pending_bridge_handoffs.len() >= MAX_PENDING_BRIDGE_HANDOFFS {
            let vm = self.vm_name.clone();
            let kind = metadata.kind;
            let mime = bounded_log_mime(&metadata.mime_type);
            self.diag
                .borrow_mut()
                .warn("clipboard-bridge", "handoff-queue-full", || {
                    format!(
                        "[d2b-wlproxy] target={vm} event=clipboard-bridge reason=handoff-queue-full kind={kind:?} mime={mime}"
                    )
                });
            return;
        }
        self.pending_bridge_handoffs
            .push_back(PendingBridgeHandoff {
                fd,
                metadata: metadata.clone(),
            });
    }

    fn flush_pending_bridge_handoffs(&mut self) {
        loop {
            let Some(pending) = self.pending_bridge_handoffs.pop_front() else {
                break;
            };
            let Some(bridge) = self.ensure_bridge_connected() else {
                self.pending_bridge_handoffs.push_front(pending);
                break;
            };
            let status = bridge.handoff_transfer_fd(&pending.fd, &pending.metadata);
            if self.handle_pending_handoff_status(pending, status) == PendingHandoffStep::Stop {
                break;
            }
        }
    }

    fn handle_pending_handoff_status(
        &mut self,
        pending: PendingBridgeHandoff,
        status: crate::bridge::HandoffStatus,
    ) -> PendingHandoffStep {
        match status {
            crate::bridge::HandoffStatus::Delivered => {
                let _ = pending
                    .fd
                    .close_after_handoff(crate::bridge::HandoffStatus::Delivered);
                log::debug!(
                    "[d2b-wlproxy] target={} event=clipboard-bridge reason=queued-handoff-delivered kind={:?} mime={}",
                    self.vm_name,
                    pending.metadata.kind,
                    bounded_log_mime(&pending.metadata.mime_type)
                );
                PendingHandoffStep::Continue
            }
            crate::bridge::HandoffStatus::Backpressure => {
                self.pending_bridge_handoffs.push_front(pending);
                PendingHandoffStep::Stop
            }
            crate::bridge::HandoffStatus::Failed(error) => {
                let vm = self.vm_name.clone();
                let kind = pending.metadata.kind;
                let mime = bounded_log_mime(&pending.metadata.mime_type);
                let error = handoff_error_detail(error);
                self.mark_bridge_disconnected();
                self.pending_bridge_handoffs.push_front(pending);
                self.diag
                    .borrow_mut()
                    .warn("clipboard-bridge", "queued-handoff-failed", || {
                        format!(
                            "[d2b-wlproxy] target={vm} event=clipboard-bridge reason=queued-handoff-failed kind={kind:?} mime={mime} error={error}"
                        )
                    });
                PendingHandoffStep::Stop
            }
        }
    }

    pub fn drive_bridge_io(clipboard: &Rc<RefCell<Self>>, bridge_ready: bool) {
        let mut state = clipboard.borrow_mut();
        if state.pending_bridge_handoffs.is_empty() {
            if state.bridge.is_none() {
                state.ensure_bridge_connected();
            }
            return;
        }
        let had_bridge = state.bridge.is_some();
        if state.bridge.is_none() {
            state.ensure_bridge_connected();
        }
        if state.bridge.is_some() && (bridge_ready || !had_bridge) {
            state.flush_pending_bridge_handoffs();
        }
    }

    pub fn bridge_retry_deadline(&self) -> Option<Instant> {
        self.next_bridge_retry
    }

    fn pending_bridge_poll_flags(&self) -> PollFlags {
        let mut flags = PollFlags::IN;
        if !self.pending_bridge_handoffs.is_empty() {
            flags |= PollFlags::OUT;
        }
        flags
    }

    #[cfg(test)]
    fn pending_handoff_count_for_tests(&self) -> usize {
        self.pending_bridge_handoffs.len()
    }

    #[cfg(test)]
    fn has_connected_bridge_for_tests(&self) -> bool {
        self.bridge.is_some()
    }

    pub fn bridge_poll_stream_and_flags(&self) -> Option<(&UnixStream, PollFlags)> {
        self.bridge
            .as_ref()
            .map(|stream| (stream, self.pending_bridge_poll_flags()))
    }

    pub fn drain_bridge_messages(clipboard: &Rc<RefCell<Self>>) {
        let mut refresh = false;
        {
            let mut state = clipboard.borrow_mut();
            let vm = state.vm_name.clone();
            let mut disconnected = false;
            let mut read_bytes = Vec::new();
            if let Some(bridge) = state.ensure_bridge_connected() {
                loop {
                    let mut buf = [0_u8; 512];
                    match bridge.read(&mut buf) {
                        Ok(0) => {
                            disconnected = true;
                            break;
                        }

                        Ok(n) => read_bytes.extend_from_slice(&buf[..n]),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(error) => {
                            let error = bounded_error_detail(error.to_string());
                            state.diag.borrow_mut().warn(
                                "clipboard-bridge",
                                "read-failed",
                                || {
                                    format!(
                                        "[d2b-wlproxy] target={vm} event=clipboard-bridge reason=read-failed error={error}"
                                    )
                                },
                            );
                            disconnected = true;
                            break;
                        }
                    }
                }
            }
            state.bridge_read_buffer.extend_from_slice(&read_bytes);
            while let Some(newline) = state
                .bridge_read_buffer
                .iter()
                .position(|byte| *byte == b'\n')
            {
                let frame = state
                    .bridge_read_buffer
                    .drain(..=newline)
                    .collect::<Vec<_>>();
                if frame
                    .windows(br#""type":"refresh_selection""#.len())
                    .any(|window| window == br#""type":"refresh_selection""#)
                {
                    refresh = true;
                }
            }
            if state.bridge_read_buffer.len() > 4096 {
                state.bridge_read_buffer.clear();
            }
            if disconnected {
                state.mark_bridge_disconnected();
            }
        }
        if refresh {
            broadcast_selection(clipboard);
        }
    }

    fn mark_bridge_disconnected(&mut self) {
        self.bridge.take();
        self.bridge_read_buffer.clear();
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
            "[d2b-wlproxy] target={} clipboard: sending cancelled to superseded source id={}",
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
    let (vm_name, selection, route, external_mimes) = {
        let state = clipboard.borrow();
        (
            state.vm_name.clone(),
            state.selection.clone(),
            state.route(),
            state.mime_policy.external_mimes(),
        )
    };
    if matches!(route, ClipboardRoute::HostOrCrossRealm) {
        let offer = device.new_send_data_offer();
        offer.set_forward_to_server(false);
        offer.set_handler(VirtualOfferHandler {
            clipboard: Rc::downgrade(clipboard),
            vm_name: vm_name.clone(),
        });
        clipboard.borrow_mut().offers.insert(
            offer.unique_id(),
            Rc::new(RefCell::new(VirtualOffer {
                offer: Rc::downgrade(&offer),
                source: None,
                source_id: offer.unique_id(),
            })),
        );
        for mime in external_mimes {
            offer.send_offer(mime);
        }
        device.send_selection(Some(&offer));
        return;
    }
    let Some(source) = selection else {
        device.send_selection(None);
        return;
    };
    let mimes = source.borrow().mime_types.clone();
    let offer = device.new_send_data_offer();
    offer.set_forward_to_server(false);
    offer.set_handler(VirtualOfferHandler {
        clipboard: Rc::downgrade(clipboard),
        vm_name: vm_name.clone(),
    });
    clipboard.borrow_mut().offers.insert(
        offer.unique_id(),
        Rc::new(RefCell::new(VirtualOffer {
            offer: Rc::downgrade(&offer),
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
            out.push_str("...");
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

fn handoff_error_detail(error: Option<nix::errno::Errno>) -> String {
    error.map_or_else(
        || "short-write".to_owned(),
        |errno| bounded_error_detail(std::io::Error::from_raw_os_error(errno as i32).to_string()),
    )
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
    decoration: Option<SharedDecorationManager>,
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
        decoration: Option<SharedDecorationManager>,
    ) -> Self {
        Self {
            policy,
            diag,
            clipboard,
            decoration,
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
        log::debug!(
            "[d2b-wlproxy] target={} event=synthetic-clipboard-advertised interface=wl_data_device_manager registry-name={name} version={version}",
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
        if let Some(decoration) = &self.decoration {
            decoration
                .borrow_mut()
                .observe_global(slf, name, interface, version);
        }
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
                        "[d2b-wlproxy] target={vm} event=bind-denied reason=version-cap registry-name={name} interface={iface} requested-version={requested} advertised-version={advertised_version}"
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
                    decoration: self.decoration.clone(),
                    positioners: Rc::new(RefCell::new(HashMap::new())),
                });
            }
            _ => match id.try_downcast::<WlEglstreamDisplay>() {
                Some(eglstream_display) => {
                    eglstream_display.set_handler(FilterEglstreamDisplayHandler {
                        vm: self.policy.vm_name.clone(),
                        diag: self.diag.clone(),
                        decoration: self.decoration.clone(),
                    });
                }
                _ => {
                    if let Some(compositor) = id.try_downcast::<WlCompositor>() {
                        compositor.set_handler(FilterCompositorHandler {
                            decoration: self.decoration.clone(),
                        });
                        slf.send_bind(name, compositor);
                        return;
                    }
                    if let Some(shm) = id.try_downcast::<WlShm>() {
                        shm.set_handler(FilterShmHandler {
                            decoration: self.decoration.clone(),
                        });
                        slf.send_bind(name, shm);
                        return;
                    }
                    if let Some(subcompositor) = id.try_downcast::<WlSubcompositor>() {
                        if let Some(decoration) = &self.decoration {
                            subcompositor.set_handler(FilterSubcompositorHandler {
                                decoration: decoration.clone(),
                            });
                        }
                        slf.send_bind(name, subcompositor);
                        return;
                    }
                    if let Some(seat) = id.try_downcast::<WlSeat>() {
                        if let Some(decoration) = &self.decoration {
                            seat.set_handler(FilterSeatHandler {
                                decoration: decoration.clone(),
                            });
                        }
                        slf.send_bind(name, seat);
                        return;
                    }
                    if let Some(viewporter) = id.try_downcast::<WpViewporter>()
                        && let Some(decoration) = &self.decoration
                    {
                        viewporter.set_handler(FilterViewporterHandler {
                            decoration: decoration.clone(),
                        });
                    }
                    if let Some(dmabuf) = id.try_downcast::<ZwpLinuxDmabufV1>()
                        && (!self.policy.dmabuf_filters.is_empty() || self.decoration.is_some())
                    {
                        dmabuf.set_handler(DmabufHandler::new(
                            self.policy.dmabuf_filters.clone(),
                            self.diag.clone(),
                            self.decoration.clone(),
                        ));
                    }
                    if let Some(drm) = id.try_downcast::<WlDrm>() {
                        if let Some(decoration) = &self.decoration {
                            drm.set_handler(FilterDrmHandler {
                                decoration: decoration.clone(),
                            });
                        }
                        slf.send_bind(name, drm);
                        return;
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
                "[d2b-wlproxy] target={} clipboard: source created id={}",
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
                "[d2b-wlproxy] target={} clipboard: device registered id={}",
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
                "[d2b-wlproxy] target={} clipboard: source id={} announced mime={}",
                self.vm_name,
                slf.unique_id(),
                bounded_log_mime(mime_type),
            );
        }
    }

    fn handle_destroy(&mut self, slf: &Rc<WlDataSource>) {
        log::debug!(
            "[d2b-wlproxy] target={} clipboard: source destroyed id={}",
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
                    format!(
                        "[d2b-wlproxy] target={vm} event=clipboard-dnd reason=start-drag-denied"
                    )
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
        log::debug!(
            "[d2b-wlproxy] target={} clipboard: set_selection source={}",
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
            "[d2b-wlproxy] target={} clipboard: receive offer id={} mime={}",
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

struct FilterCompositorHandler {
    decoration: Option<SharedDecorationManager>,
}

impl WlCompositorHandler for FilterCompositorHandler {
    fn handle_create_surface(&mut self, slf: &Rc<WlCompositor>, id: &Rc<WlSurface>) {
        if let Some(decoration) = &self.decoration {
            decoration.borrow_mut().register_surface(id);
            id.set_handler(FilterSurfaceHandler {
                decoration: decoration.clone(),
            });
        }
        slf.send_create_surface(id);
    }
}

struct FilterSeatHandler {
    decoration: SharedDecorationManager,
}

impl WlSeatHandler for FilterSeatHandler {
    fn handle_get_pointer(&mut self, slf: &Rc<WlSeat>, id: &Rc<WlPointer>) {
        id.set_handler(FilterPointerHandler {
            decoration: self.decoration.clone(),
            focus: PointerFocus::None,
            target_surface: None,
            pending_forwarded_frame: false,
        });
        slf.send_get_pointer(id);
    }

    fn handle_get_keyboard(&mut self, slf: &Rc<WlSeat>, id: &Rc<WlKeyboard>) {
        id.set_handler(FilterKeyboardHandler {
            decoration: self.decoration.clone(),
        });
        slf.send_get_keyboard(id);
    }

    fn handle_get_touch(&mut self, slf: &Rc<WlSeat>, id: &Rc<WlTouch>) {
        id.set_handler(FilterTouchHandler {
            decoration: self.decoration.clone(),
            suppressed_ids: HashSet::new(),
            forwarded_ids: HashSet::new(),
            wrapper_forwarded_ids: HashSet::new(),
            pending_forwarded_frame: false,
        });
        slf.send_get_touch(id);
    }
}

struct FilterKeyboardHandler {
    decoration: SharedDecorationManager,
}

impl WlKeyboardHandler for FilterKeyboardHandler {
    fn handle_keymap(
        &mut self,
        slf: &Rc<WlKeyboard>,
        format: WlKeyboardKeymapFormat,
        fd: &Rc<OwnedFd>,
        size: u32,
    ) {
        if receiver_has_client(slf.client()) {
            slf.send_keymap(format, fd, size);
        }
    }

    fn handle_enter(
        &mut self,
        slf: &Rc<WlKeyboard>,
        serial: u32,
        surface: &Rc<WlSurface>,
        keys: &[u8],
    ) {
        if let Some(surface) = self.decoration.borrow().wrapper_input_target(surface) {
            if surface_belongs_to_receiver(&surface, slf.client()) {
                slf.send_enter(serial, &surface, keys);
            }
        } else if surface_belongs_to_receiver(surface, slf.client()) {
            slf.send_enter(serial, surface, keys);
        }
    }

    fn handle_leave(&mut self, slf: &Rc<WlKeyboard>, serial: u32, surface: &Rc<WlSurface>) {
        if let Some(surface) = self.decoration.borrow().wrapper_input_target(surface) {
            if surface_belongs_to_receiver(&surface, slf.client()) {
                slf.send_leave(serial, &surface);
            }
        } else if surface_belongs_to_receiver(surface, slf.client()) {
            slf.send_leave(serial, surface);
        }
    }

    fn handle_key(
        &mut self,
        slf: &Rc<WlKeyboard>,
        serial: u32,
        time: u32,
        key: u32,
        state: WlKeyboardKeyState,
    ) {
        if receiver_has_client(slf.client()) {
            slf.send_key(serial, time, key, state);
        }
    }

    fn handle_modifiers(
        &mut self,
        slf: &Rc<WlKeyboard>,
        serial: u32,
        mods_depressed: u32,
        mods_latched: u32,
        mods_locked: u32,
        group: u32,
    ) {
        if receiver_has_client(slf.client()) {
            slf.send_modifiers(serial, mods_depressed, mods_latched, mods_locked, group);
        }
    }

    fn handle_repeat_info(&mut self, slf: &Rc<WlKeyboard>, rate: i32, delay: i32) {
        if receiver_has_client(slf.client()) {
            slf.send_repeat_info(rate, delay);
        }
    }
}

struct FilterPointerHandler {
    decoration: SharedDecorationManager,
    focus: PointerFocus,
    target_surface: Option<Rc<WlSurface>>,
    pending_forwarded_frame: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PointerFocus {
    #[default]
    None,
    Direct,
    WrapperContent,
    Rail,
}

fn wrapper_content_pointer_x(surface_x: Fixed) -> Option<Fixed> {
    let rail_width = Fixed::from_i32_saturating(WRAPPER_RAIL_WIDTH as i32);
    (surface_x >= rail_width).then_some(surface_x - rail_width)
}

fn pointer_motion_x_for_focus(focus: PointerFocus, surface_x: Fixed) -> Option<Fixed> {
    match focus {
        PointerFocus::Direct => Some(surface_x),
        PointerFocus::WrapperContent => {
            Some(surface_x - Fixed::from_i32_saturating(WRAPPER_RAIL_WIDTH as i32))
        }
        PointerFocus::None | PointerFocus::Rail => None,
    }
}

impl WlPointerHandler for FilterPointerHandler {
    fn handle_enter(
        &mut self,
        slf: &Rc<WlPointer>,
        serial: u32,
        surface: &Rc<WlSurface>,
        surface_x: Fixed,
        surface_y: Fixed,
    ) {
        if let Some(target) = self.decoration.borrow().wrapper_input_target(surface) {
            if surface_belongs_to_receiver(&target, slf.client()) {
                self.target_surface = Some(target.clone());
                if let Some(x) = wrapper_content_pointer_x(surface_x) {
                    self.focus = PointerFocus::WrapperContent;
                    self.pending_forwarded_frame = true;
                    slf.send_enter(serial, &target, x, surface_y);
                } else {
                    self.focus = PointerFocus::Rail;
                }
            } else {
                self.focus = PointerFocus::None;
                self.target_surface = None;
            }
            return;
        }
        if !surface_belongs_to_receiver(surface, slf.client()) {
            self.focus = PointerFocus::None;
            self.target_surface = None;
            return;
        }
        self.focus = PointerFocus::Direct;
        self.target_surface = Some(surface.clone());
        self.pending_forwarded_frame = true;
        slf.send_enter(serial, surface, surface_x, surface_y);
    }

    fn handle_leave(&mut self, slf: &Rc<WlPointer>, serial: u32, surface: &Rc<WlSurface>) {
        if let Some(target) = self.decoration.borrow().wrapper_input_target(surface) {
            if surface_belongs_to_receiver(&target, slf.client())
                && self.focus == PointerFocus::WrapperContent
            {
                self.pending_forwarded_frame = true;
                slf.send_leave(serial, &target);
            }
            self.focus = PointerFocus::None;
            self.target_surface = None;
            return;
        }
        if !surface_belongs_to_receiver(surface, slf.client()) {
            self.focus = PointerFocus::None;
            self.target_surface = None;
            return;
        }
        self.focus = PointerFocus::None;
        self.target_surface = None;
        self.pending_forwarded_frame = true;
        slf.send_leave(serial, surface);
    }

    fn handle_motion(
        &mut self,
        slf: &Rc<WlPointer>,
        time: u32,
        surface_x: Fixed,
        surface_y: Fixed,
    ) {
        if !receiver_has_client(slf.client()) {
            self.focus = PointerFocus::None;
            self.target_surface = None;
            self.pending_forwarded_frame = false;
            return;
        }
        if self.focus == PointerFocus::WrapperContent
            && wrapper_content_pointer_x(surface_x).is_none()
        {
            self.focus = PointerFocus::Rail;
            self.pending_forwarded_frame = true;
            if let Some(target) = &self.target_surface {
                slf.send_leave(0, target);
            }
            return;
        }
        if self.focus == PointerFocus::Rail {
            if let Some(x) = wrapper_content_pointer_x(surface_x) {
                self.focus = PointerFocus::WrapperContent;
                self.pending_forwarded_frame = true;
                if let Some(target) = &self.target_surface {
                    slf.send_enter(0, target, x, surface_y);
                }
            }
            return;
        }
        let Some(x) = pointer_motion_x_for_focus(self.focus, surface_x) else {
            return;
        };
        self.pending_forwarded_frame = true;
        slf.send_motion(time, x, surface_y);
    }

    fn handle_button(
        &mut self,
        slf: &Rc<WlPointer>,
        serial: u32,
        time: u32,
        button: u32,
        state: WlPointerButtonState,
    ) {
        if !receiver_has_client(slf.client()) {
            self.focus = PointerFocus::None;
            self.pending_forwarded_frame = false;
            return;
        }
        if self.focus == PointerFocus::Rail || self.focus == PointerFocus::None {
            return;
        }
        self.pending_forwarded_frame = true;
        slf.send_button(serial, time, button, state);
    }

    fn handle_axis(&mut self, slf: &Rc<WlPointer>, time: u32, axis: WlPointerAxis, value: Fixed) {
        if !receiver_has_client(slf.client()) {
            self.focus = PointerFocus::None;
            self.pending_forwarded_frame = false;
            return;
        }
        if self.focus == PointerFocus::Rail || self.focus == PointerFocus::None {
            return;
        }
        self.pending_forwarded_frame = true;
        slf.send_axis(time, axis, value);
    }

    fn handle_frame(&mut self, slf: &Rc<WlPointer>) {
        if !receiver_has_client(slf.client()) {
            self.focus = PointerFocus::None;
            self.pending_forwarded_frame = false;
            return;
        }
        if !self.pending_forwarded_frame {
            return;
        }
        self.pending_forwarded_frame = false;
        slf.send_frame();
    }

    fn handle_axis_source(&mut self, slf: &Rc<WlPointer>, axis_source: WlPointerAxisSource) {
        if !receiver_has_client(slf.client()) {
            self.focus = PointerFocus::None;
            self.pending_forwarded_frame = false;
            return;
        }
        if self.focus == PointerFocus::Rail || self.focus == PointerFocus::None {
            return;
        }
        self.pending_forwarded_frame = true;
        slf.send_axis_source(axis_source);
    }

    fn handle_axis_stop(&mut self, slf: &Rc<WlPointer>, time: u32, axis: WlPointerAxis) {
        if !receiver_has_client(slf.client()) {
            self.focus = PointerFocus::None;
            self.pending_forwarded_frame = false;
            return;
        }
        if self.focus == PointerFocus::Rail || self.focus == PointerFocus::None {
            return;
        }
        self.pending_forwarded_frame = true;
        slf.send_axis_stop(time, axis);
    }

    fn handle_axis_discrete(&mut self, slf: &Rc<WlPointer>, axis: WlPointerAxis, discrete: i32) {
        if !receiver_has_client(slf.client()) {
            self.focus = PointerFocus::None;
            self.pending_forwarded_frame = false;
            return;
        }
        if self.focus == PointerFocus::Rail || self.focus == PointerFocus::None {
            return;
        }
        self.pending_forwarded_frame = true;
        slf.send_axis_discrete(axis, discrete);
    }

    fn handle_axis_value120(&mut self, slf: &Rc<WlPointer>, axis: WlPointerAxis, value120: i32) {
        if !receiver_has_client(slf.client()) {
            self.focus = PointerFocus::None;
            self.pending_forwarded_frame = false;
            return;
        }
        if self.focus == PointerFocus::Rail || self.focus == PointerFocus::None {
            return;
        }
        self.pending_forwarded_frame = true;
        slf.send_axis_value120(axis, value120);
    }

    fn handle_axis_relative_direction(
        &mut self,
        slf: &Rc<WlPointer>,
        axis: WlPointerAxis,
        direction: WlPointerAxisRelativeDirection,
    ) {
        if !receiver_has_client(slf.client()) {
            self.focus = PointerFocus::None;
            self.pending_forwarded_frame = false;
            return;
        }
        if self.focus == PointerFocus::Rail || self.focus == PointerFocus::None {
            return;
        }
        self.pending_forwarded_frame = true;
        slf.send_axis_relative_direction(axis, direction);
    }
}

struct FilterTouchHandler {
    decoration: SharedDecorationManager,
    suppressed_ids: HashSet<i32>,
    forwarded_ids: HashSet<i32>,
    wrapper_forwarded_ids: HashSet<i32>,
    pending_forwarded_frame: bool,
}

impl WlTouchHandler for FilterTouchHandler {
    fn handle_down(
        &mut self,
        slf: &Rc<WlTouch>,
        serial: u32,
        time: u32,
        surface: &Rc<WlSurface>,
        id: i32,
        x: Fixed,
        y: Fixed,
    ) {
        if !receiver_has_client(slf.client()) {
            self.suppressed_ids.clear();
            self.forwarded_ids.clear();
            self.wrapper_forwarded_ids.clear();
            self.pending_forwarded_frame = false;
            return;
        }
        if let Some(target) = self.decoration.borrow().wrapper_input_target(surface) {
            if surface_belongs_to_receiver(&target, slf.client()) {
                if let Some(adjusted_x) = wrapper_content_pointer_x(x) {
                    self.forwarded_ids.insert(id);
                    self.wrapper_forwarded_ids.insert(id);
                    self.pending_forwarded_frame = true;
                    slf.send_down(serial, time, &target, id, adjusted_x, y);
                } else {
                    self.suppressed_ids.insert(id);
                }
            }
            return;
        }
        if !surface_belongs_to_receiver(surface, slf.client()) {
            return;
        }
        self.forwarded_ids.insert(id);
        self.pending_forwarded_frame = true;
        slf.send_down(serial, time, surface, id, x, y);
    }

    fn handle_up(&mut self, slf: &Rc<WlTouch>, serial: u32, time: u32, id: i32) {
        if !receiver_has_client(slf.client()) {
            self.suppressed_ids.clear();
            self.forwarded_ids.clear();
            self.wrapper_forwarded_ids.clear();
            self.pending_forwarded_frame = false;
            return;
        }
        if self.suppressed_ids.remove(&id) {
            return;
        }
        self.forwarded_ids.remove(&id);
        self.wrapper_forwarded_ids.remove(&id);
        self.pending_forwarded_frame = true;
        slf.send_up(serial, time, id);
    }

    fn handle_motion(&mut self, slf: &Rc<WlTouch>, time: u32, id: i32, x: Fixed, y: Fixed) {
        if !receiver_has_client(slf.client()) {
            self.suppressed_ids.clear();
            self.forwarded_ids.clear();
            self.wrapper_forwarded_ids.clear();
            self.pending_forwarded_frame = false;
            return;
        }
        if self.suppressed_ids.contains(&id) {
            return;
        }
        self.pending_forwarded_frame = true;
        let adjusted_x = if self.wrapper_forwarded_ids.contains(&id) {
            wrapper_content_pointer_x(x).unwrap_or(Fixed::ZERO)
        } else {
            x
        };
        slf.send_motion(time, id, adjusted_x, y);
    }

    fn handle_shape(&mut self, slf: &Rc<WlTouch>, id: i32, major: Fixed, minor: Fixed) {
        if !receiver_has_client(slf.client()) {
            self.suppressed_ids.clear();
            self.forwarded_ids.clear();
            self.wrapper_forwarded_ids.clear();
            self.pending_forwarded_frame = false;
            return;
        }
        if self.suppressed_ids.contains(&id) {
            return;
        }
        self.pending_forwarded_frame = true;
        slf.send_shape(id, major, minor);
    }

    fn handle_orientation(&mut self, slf: &Rc<WlTouch>, id: i32, orientation: Fixed) {
        if !receiver_has_client(slf.client()) {
            self.suppressed_ids.clear();
            self.forwarded_ids.clear();
            self.wrapper_forwarded_ids.clear();
            self.pending_forwarded_frame = false;
            return;
        }
        if self.suppressed_ids.contains(&id) {
            return;
        }
        self.pending_forwarded_frame = true;
        slf.send_orientation(id, orientation);
    }

    fn handle_cancel(&mut self, slf: &Rc<WlTouch>) {
        if !receiver_has_client(slf.client()) {
            self.suppressed_ids.clear();
            self.forwarded_ids.clear();
            self.pending_forwarded_frame = false;
            return;
        }
        if self.forwarded_ids.is_empty() {
            self.suppressed_ids.clear();
            self.wrapper_forwarded_ids.clear();
            self.pending_forwarded_frame = false;
            return;
        }
        self.forwarded_ids.clear();
        self.suppressed_ids.clear();
        self.wrapper_forwarded_ids.clear();
        self.pending_forwarded_frame = false;
        slf.send_cancel();
    }

    fn handle_frame(&mut self, slf: &Rc<WlTouch>) {
        if !receiver_has_client(slf.client()) {
            self.suppressed_ids.clear();
            self.forwarded_ids.clear();
            self.wrapper_forwarded_ids.clear();
            self.pending_forwarded_frame = false;
            return;
        }
        if !self.pending_forwarded_frame {
            return;
        }
        self.pending_forwarded_frame = false;
        slf.send_frame();
    }
}

fn receiver_has_client(receiver: Option<Rc<Client>>) -> bool {
    receiver.is_some()
}

fn surface_belongs_to_receiver(surface: &Rc<WlSurface>, receiver: Option<Rc<Client>>) -> bool {
    object_belongs_to_receiver(surface, receiver)
}

fn object_belongs_to_receiver<T>(object: &Rc<T>, receiver: Option<Rc<Client>>) -> bool
where
    T: ObjectCoreApi,
{
    let Some(receiver) = receiver else {
        return false;
    };
    object
        .client()
        .is_some_and(|object_client| Rc::ptr_eq(&object_client, &receiver))
}

struct FilterSubcompositorHandler {
    decoration: SharedDecorationManager,
}

impl WlSubcompositorHandler for FilterSubcompositorHandler {
    fn handle_destroy(&mut self, slf: &Rc<WlSubcompositor>) {
        slf.send_destroy();
    }

    fn handle_get_subsurface(
        &mut self,
        slf: &Rc<WlSubcompositor>,
        id: &Rc<WlSubsurface>,
        surface: &Rc<WlSurface>,
        parent: &Rc<WlSurface>,
    ) {
        id.set_handler(FilterSubsurfaceHandler {
            decoration: self.decoration.clone(),
            parent: Rc::downgrade(parent),
            surface: Rc::downgrade(surface),
        });
        slf.send_get_subsurface(id, surface, parent);
        self.decoration
            .borrow_mut()
            .register_guest_subsurface(surface, parent);
    }
}

struct FilterSubsurfaceHandler {
    decoration: SharedDecorationManager,
    parent: Weak<WlSurface>,
    surface: Weak<WlSurface>,
}

impl FilterSubsurfaceHandler {
    fn raise_decoration(&self) {
        if let Some(parent) = self.parent.upgrade() {
            if let Some(surface) = self.surface.upgrade() {
                self.decoration
                    .borrow_mut()
                    .register_guest_subsurface(&surface, &parent);
            } else {
                self.decoration
                    .borrow_mut()
                    .raise_decoration_above_guest_subsurfaces(&parent);
            }
        }
    }
}

impl WlSubsurfaceHandler for FilterSubsurfaceHandler {
    fn handle_destroy(&mut self, slf: &Rc<WlSubsurface>) {
        slf.send_destroy();
        self.raise_decoration();
    }

    fn handle_set_position(&mut self, slf: &Rc<WlSubsurface>, x: i32, y: i32) {
        slf.send_set_position(x, y);
    }

    fn handle_place_above(&mut self, slf: &Rc<WlSubsurface>, sibling: &Rc<WlSurface>) {
        slf.send_place_above(sibling);
        self.raise_decoration();
    }

    fn handle_place_below(&mut self, slf: &Rc<WlSubsurface>, sibling: &Rc<WlSurface>) {
        slf.send_place_below(sibling);
        self.raise_decoration();
    }

    fn handle_set_sync(&mut self, slf: &Rc<WlSubsurface>) {
        slf.send_set_sync();
    }

    fn handle_set_desync(&mut self, slf: &Rc<WlSubsurface>) {
        slf.send_set_desync();
    }
}

struct FilterViewporterHandler {
    decoration: SharedDecorationManager,
}

impl WpViewporterHandler for FilterViewporterHandler {
    fn handle_destroy(&mut self, slf: &Rc<WpViewporter>) {
        slf.send_destroy();
    }

    fn handle_get_viewport(
        &mut self,
        slf: &Rc<WpViewporter>,
        id: &Rc<WpViewport>,
        surface: &Rc<WlSurface>,
    ) {
        self.decoration.borrow_mut().surface_get_viewport(surface);
        id.set_handler(FilterViewportHandler {
            decoration: self.decoration.clone(),
            surface: Rc::downgrade(surface),
        });
        slf.send_get_viewport(id, surface);
    }
}

struct FilterViewportHandler {
    decoration: SharedDecorationManager,
    surface: Weak<WlSurface>,
}

impl WpViewportHandler for FilterViewportHandler {
    fn handle_destroy(&mut self, slf: &Rc<WpViewport>) {
        if let Some(surface) = self.surface.upgrade() {
            self.decoration
                .borrow_mut()
                .surface_viewport_destroyed(&surface);
        }
        slf.send_destroy();
    }

    fn handle_set_source(
        &mut self,
        slf: &Rc<WpViewport>,
        x: Fixed,
        y: Fixed,
        width: Fixed,
        height: Fixed,
    ) {
        if let Some(surface) = self.surface.upgrade() {
            self.decoration
                .borrow_mut()
                .surface_set_viewport_source(&surface, x, y, width, height);
        }
        slf.send_set_source(x, y, width, height);
    }

    fn handle_set_destination(&mut self, slf: &Rc<WpViewport>, width: i32, height: i32) {
        if let Some(surface) = self.surface.upgrade() {
            self.decoration
                .borrow_mut()
                .surface_set_viewport_destination(&surface, width, height);
        }
        slf.send_set_destination(width, height);
    }
}

struct FilterShmHandler {
    decoration: Option<SharedDecorationManager>,
}

impl WlShmHandler for FilterShmHandler {
    fn handle_create_pool(
        &mut self,
        slf: &Rc<WlShm>,
        id: &Rc<WlShmPool>,
        fd: &Rc<OwnedFd>,
        size: i32,
    ) {
        if let Some(decoration) = &self.decoration {
            id.set_handler(tracking_shm_pool_handler(decoration));
        }
        slf.send_create_pool(id, fd, size);
    }
}

struct FilterSurfaceHandler {
    decoration: SharedDecorationManager,
}

impl WlSurfaceHandler for FilterSurfaceHandler {
    fn handle_destroy(&mut self, slf: &Rc<WlSurface>) {
        self.decoration.borrow_mut().surface_destroyed(slf);
        // Ordinary forwarded wl_surface destruction uses wl-proxy's normal
        // object lifetime. The shm-pool tracker owns the only cross-domain
        // delete_id workaround because wl-cross-domain-proxy reuses pool IDs.
        slf.send_destroy();
    }

    fn handle_attach(
        &mut self,
        slf: &Rc<WlSurface>,
        buffer: Option<&Rc<WlBuffer>>,
        x: i32,
        y: i32,
    ) {
        self.decoration.borrow_mut().surface_attach(slf, buffer);
        slf.send_attach(buffer, x, y);
    }

    fn handle_set_buffer_transform(&mut self, slf: &Rc<WlSurface>, transform: WlOutputTransform) {
        self.decoration
            .borrow_mut()
            .surface_set_buffer_transform(slf, transform);
        slf.send_set_buffer_transform(transform);
    }

    fn handle_set_buffer_scale(&mut self, slf: &Rc<WlSurface>, scale: i32) {
        self.decoration
            .borrow_mut()
            .surface_set_buffer_scale(slf, scale);
        slf.send_set_buffer_scale(scale);
    }

    fn handle_commit(&mut self, slf: &Rc<WlSurface>) {
        slf.send_commit();
        self.decoration.borrow_mut().surface_commit(slf);
    }

    fn handle_enter(&mut self, slf: &Rc<WlSurface>, output: &Rc<WlOutput>) {
        if object_belongs_to_receiver(output, slf.client()) {
            slf.send_enter(output);
        }
    }

    fn handle_leave(&mut self, slf: &Rc<WlSurface>, output: &Rc<WlOutput>) {
        if object_belongs_to_receiver(output, slf.client()) {
            slf.send_leave(output);
        }
    }
}

/// Handler for `xdg_wm_base`: intercepts `get_xdg_surface` to install our
/// surface handler on each new `xdg_surface`.
struct FilterXdgWmBaseHandler {
    policy: Rc<FilterPolicy>,
    decoration: Option<SharedDecorationManager>,
    positioners: Rc<RefCell<HashMap<u64, PositionerState>>>,
}

impl XdgWmBaseHandler for FilterXdgWmBaseHandler {
    fn handle_create_positioner(&mut self, slf: &Rc<XdgWmBase>, id: &Rc<XdgPositioner>) {
        self.positioners
            .borrow_mut()
            .entry(id.unique_id())
            .or_default();
        id.set_handler(FilterXdgPositionerHandler {
            positioners: self.positioners.clone(),
        });
        slf.send_create_positioner(id);
    }

    fn handle_get_xdg_surface(
        &mut self,
        slf: &Rc<XdgWmBase>,
        xdg_surface: &Rc<XdgSurface>,
        surface: &Rc<WlSurface>,
    ) {
        if let Some(decoration) = &self.decoration {
            xdg_surface.set_forward_to_server(false);
            decoration.borrow_mut().register_surface(surface);
        }
        xdg_surface.set_handler(FilterXdgSurfaceHandler {
            policy: self.policy.clone(),
            decoration: self.decoration.clone(),
            surface: Rc::downgrade(surface),
            wm_base: slf.clone(),
            server_xdg_surface_created: self.decoration.is_none(),
            positioners: self.positioners.clone(),
        });
        if self.decoration.is_none() {
            slf.send_get_xdg_surface(xdg_surface, surface);
        }
    }
}

struct FilterXdgPositionerHandler {
    positioners: Rc<RefCell<HashMap<u64, PositionerState>>>,
}

impl FilterXdgPositionerHandler {
    fn update(&self, positioner: &Rc<XdgPositioner>, update: impl FnOnce(&mut PositionerState)) {
        if let Some(state) = self
            .positioners
            .borrow_mut()
            .get_mut(&positioner.unique_id())
        {
            update(state);
        }
    }
}

impl XdgPositionerHandler for FilterXdgPositionerHandler {
    fn handle_destroy(&mut self, slf: &Rc<XdgPositioner>) {
        self.positioners.borrow_mut().remove(&slf.unique_id());
        slf.send_destroy();
    }

    fn handle_set_size(&mut self, slf: &Rc<XdgPositioner>, width: i32, height: i32) {
        self.update(slf, |state| state.size = Some((width, height)));
        slf.send_set_size(width, height);
    }

    fn handle_set_anchor_rect(
        &mut self,
        slf: &Rc<XdgPositioner>,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) {
        self.update(slf, |state| {
            state.anchor_rect = Some((x, y, width, height));
        });
        slf.send_set_anchor_rect(x, y, width, height);
    }

    fn handle_set_anchor(&mut self, slf: &Rc<XdgPositioner>, anchor: XdgPositionerAnchor) {
        self.update(slf, |state| state.anchor = Some(anchor));
        slf.send_set_anchor(anchor);
    }

    fn handle_set_gravity(&mut self, slf: &Rc<XdgPositioner>, gravity: XdgPositionerGravity) {
        self.update(slf, |state| state.gravity = Some(gravity));
        slf.send_set_gravity(gravity);
    }

    fn handle_set_constraint_adjustment(
        &mut self,
        slf: &Rc<XdgPositioner>,
        constraint_adjustment: XdgPositionerConstraintAdjustment,
    ) {
        self.update(slf, |state| {
            state.constraint_adjustment = Some(constraint_adjustment);
        });
        slf.send_set_constraint_adjustment(constraint_adjustment);
    }

    fn handle_set_offset(&mut self, slf: &Rc<XdgPositioner>, x: i32, y: i32) {
        self.update(slf, |state| state.offset = Some((x, y)));
        slf.send_set_offset(x, y);
    }

    fn handle_set_reactive(&mut self, slf: &Rc<XdgPositioner>) {
        self.update(slf, |state| state.reactive = true);
        slf.send_set_reactive();
    }

    fn handle_set_parent_size(
        &mut self,
        slf: &Rc<XdgPositioner>,
        parent_width: i32,
        parent_height: i32,
    ) {
        self.update(slf, |state| {
            state.parent_size = Some((parent_width, parent_height));
        });
        slf.send_set_parent_size(parent_width, parent_height);
    }

    fn handle_set_parent_configure(&mut self, slf: &Rc<XdgPositioner>, serial: u32) {
        self.update(slf, |state| state.parent_configure = Some(serial));
        slf.send_set_parent_configure(serial);
    }
}

/// Handler for `xdg_surface`: intercepts `get_toplevel` to install our
/// toplevel handler.
struct FilterXdgSurfaceHandler {
    policy: Rc<FilterPolicy>,
    decoration: Option<SharedDecorationManager>,
    surface: Weak<WlSurface>,
    wm_base: Rc<XdgWmBase>,
    server_xdg_surface_created: bool,
    positioners: Rc<RefCell<HashMap<u64, PositionerState>>>,
}

impl FilterXdgSurfaceHandler {
    fn ensure_server_xdg_surface(&mut self, slf: &Rc<XdgSurface>) -> bool {
        if self.server_xdg_surface_created {
            return true;
        }
        let Some(surface) = self.surface.upgrade() else {
            return false;
        };
        self.wm_base.send_get_xdg_surface(slf, &surface);
        slf.set_forward_to_server(true);
        self.server_xdg_surface_created = true;
        true
    }

    fn adjusted_positioner_for_wrapper_parent(
        &self,
        positioner: &Rc<XdgPositioner>,
    ) -> Rc<XdgPositioner> {
        let adjusted = self.wm_base.new_send_create_positioner();
        if let Some(state) = self.positioners.borrow().get(&positioner.unique_id()) {
            state.apply_to(&adjusted, i32::try_from(WRAPPER_RAIL_WIDTH).unwrap_or(0));
        }
        adjusted
    }
}

impl XdgSurfaceHandler for FilterXdgSurfaceHandler {
    fn handle_destroy(&mut self, slf: &Rc<XdgSurface>) {
        if let (Some(decoration), Some(surface)) = (&self.decoration, self.surface.upgrade()) {
            decoration
                .borrow_mut()
                .toplevel_destroyed(surface.unique_id());
        }
        if self.server_xdg_surface_created {
            slf.send_destroy();
        }
    }

    fn handle_get_toplevel(&mut self, slf: &Rc<XdgSurface>, toplevel: &Rc<XdgToplevel>) {
        let surface = self.surface.upgrade();
        let surface_id = surface.as_ref().map(|surface| surface.unique_id());
        toplevel.set_handler(FilterXdgToplevelHandler {
            policy: self.policy.clone(),
            decoration: self.decoration.clone(),
            surface_id,
        });
        if let (Some(decoration), Some(surface)) = (&self.decoration, surface.as_ref()) {
            let created = decoration.borrow_mut().create_wrapper_toplevel(
                surface,
                slf,
                toplevel,
                Rc::downgrade(decoration),
            );
            if created {
                return;
            }
            decoration.borrow_mut().mark_toplevel(surface);
        }
        if !self.ensure_server_xdg_surface(slf) {
            return;
        }
        slf.send_get_toplevel(toplevel);
    }

    fn handle_get_popup(
        &mut self,
        slf: &Rc<XdgSurface>,
        popup: &Rc<XdgPopup>,
        parent: Option<&Rc<XdgSurface>>,
        positioner: &Rc<XdgPositioner>,
    ) {
        if !self.ensure_server_xdg_surface(slf) {
            return;
        }
        let wrapper_parent = parent.and_then(|parent| {
            self.decoration
                .as_ref()
                .and_then(|decoration| decoration.borrow().wrapper_xdg_surface_for_guest(parent))
        });
        let adjusted_positioner = wrapper_parent
            .as_ref()
            .map(|_| self.adjusted_positioner_for_wrapper_parent(positioner));
        let parent = wrapper_parent.as_ref().or(parent);
        let positioner = adjusted_positioner.as_ref().unwrap_or(positioner);
        slf.send_get_popup(popup, parent, positioner);
        if let Some(positioner) = adjusted_positioner {
            positioner.send_destroy();
        }
    }

    fn handle_set_window_geometry(
        &mut self,
        slf: &Rc<XdgSurface>,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) {
        let geometry = WindowGeometry::new(x, y, width, height);
        if let (Some(decoration), Some(surface)) = (&self.decoration, self.surface.upgrade())
            && decoration
                .borrow_mut()
                .wrapper_set_window_geometry(surface.unique_id(), geometry)
        {
            return;
        }
        let geometry = self.decoration.as_ref().zip(self.surface.upgrade()).map_or(
            geometry,
            |(decoration, surface)| {
                decoration
                    .borrow()
                    .translate_window_geometry(&surface, geometry)
            },
        );
        slf.send_set_window_geometry(geometry.x, geometry.y, geometry.width, geometry.height);
    }

    fn handle_ack_configure(&mut self, slf: &Rc<XdgSurface>, serial: u32) {
        if let (Some(decoration), Some(surface)) = (&self.decoration, self.surface.upgrade())
            && decoration
                .borrow_mut()
                .wrapper_ack_configure(surface.unique_id(), serial)
        {
            return;
        }
        slf.send_ack_configure(serial);
    }
}

/// Handler for `xdg_toplevel`: rewrites app-id and title.
struct FilterXdgToplevelHandler {
    policy: Rc<FilterPolicy>,
    decoration: Option<SharedDecorationManager>,
    surface_id: Option<u64>,
}

impl XdgToplevelHandler for FilterXdgToplevelHandler {
    fn handle_destroy(&mut self, slf: &Rc<XdgToplevel>) {
        if let (Some(decoration), Some(surface_id)) = (&self.decoration, self.surface_id) {
            let mut decoration = decoration.borrow_mut();
            if decoration.has_wrapper(surface_id) {
                decoration.toplevel_destroyed(surface_id);
                return;
            }
            decoration.toplevel_destroyed(surface_id);
        }
        slf.send_destroy();
    }

    fn handle_set_app_id(&mut self, slf: &Rc<XdgToplevel>, app_id: &str) {
        let rewritten = self.policy.rewrite_app_id(app_id);
        if let (Some(decoration), Some(surface_id)) = (&self.decoration, self.surface_id)
            && decoration
                .borrow_mut()
                .wrapper_set_app_id(surface_id, &rewritten)
        {
            return;
        }
        slf.send_set_app_id(&rewritten);
    }

    fn handle_set_title(&mut self, slf: &Rc<XdgToplevel>, title: &str) {
        let rewritten = self.policy.rewrite_title(title);
        if let (Some(decoration), Some(surface_id)) = (&self.decoration, self.surface_id)
            && decoration
                .borrow_mut()
                .wrapper_set_title(surface_id, &rewritten)
        {
            return;
        }
        slf.send_set_title(&rewritten);
    }

    fn handle_set_fullscreen(
        &mut self,
        slf: &Rc<XdgToplevel>,
        output: Option<&Rc<wl_proxy::protocols::wayland::wl_output::WlOutput>>,
    ) {
        if let (Some(decoration), Some(surface_id)) = (&self.decoration, self.surface_id) {
            let mut decoration = decoration.borrow_mut();
            if decoration.wrapper_set_fullscreen(surface_id, output) {
                return;
            }
            decoration.toplevel_fullscreen_request(surface_id, true);
        }
        slf.send_set_fullscreen(output);
    }

    fn handle_unset_fullscreen(&mut self, slf: &Rc<XdgToplevel>) {
        if let (Some(decoration), Some(surface_id)) = (&self.decoration, self.surface_id) {
            let mut decoration = decoration.borrow_mut();
            if decoration.wrapper_unset_fullscreen(surface_id) {
                return;
            }
            decoration.toplevel_fullscreen_request(surface_id, false);
        }
        slf.send_unset_fullscreen();
    }

    fn handle_set_maximized(&mut self, slf: &Rc<XdgToplevel>) {
        if let (Some(decoration), Some(surface_id)) = (&self.decoration, self.surface_id)
            && decoration
                .borrow_mut()
                .wrapper_set_maximized(surface_id, true)
        {
            return;
        }
        slf.send_set_maximized();
    }

    fn handle_unset_maximized(&mut self, slf: &Rc<XdgToplevel>) {
        if let (Some(decoration), Some(surface_id)) = (&self.decoration, self.surface_id)
            && decoration
                .borrow_mut()
                .wrapper_set_maximized(surface_id, false)
        {
            return;
        }
        slf.send_unset_maximized();
    }

    fn handle_set_minimized(&mut self, slf: &Rc<XdgToplevel>) {
        if let (Some(decoration), Some(surface_id)) = (&self.decoration, self.surface_id)
            && decoration.borrow_mut().wrapper_set_minimized(surface_id)
        {
            return;
        }
        slf.send_set_minimized();
    }

    fn handle_set_min_size(&mut self, slf: &Rc<XdgToplevel>, width: i32, height: i32) {
        if let (Some(decoration), Some(surface_id)) = (&self.decoration, self.surface_id)
            && decoration
                .borrow_mut()
                .wrapper_set_min_size(surface_id, width, height)
        {
            return;
        }
        slf.send_set_min_size(width, height);
    }

    fn handle_set_max_size(&mut self, slf: &Rc<XdgToplevel>, width: i32, height: i32) {
        if let (Some(decoration), Some(surface_id)) = (&self.decoration, self.surface_id)
            && decoration
                .borrow_mut()
                .wrapper_set_max_size(surface_id, width, height)
        {
            return;
        }
        slf.send_set_max_size(width, height);
    }

    fn handle_configure(&mut self, slf: &Rc<XdgToplevel>, width: i32, height: i32, states: &[u8]) {
        if let (Some(decoration), Some(surface_id)) = (&self.decoration, self.surface_id) {
            let mut decoration = decoration.borrow_mut();
            if decoration.has_wrapper(surface_id) {
                return;
            }
            decoration.toplevel_configure(surface_id, states);
        }
        slf.send_configure(width, height, states);
    }
}

/// Handler for `wl_eglstream_display`: keep NVIDIA EGLStream constrained to
/// fd-backed streams. The protocol also defines inet/socket modes, but those
/// are network/socket transport surfaces and are outside d2b's intended
/// guest-to-host Wayland boundary.
struct FilterEglstreamDisplayHandler {
    vm: String,
    diag: Rc<RefCell<DiagRateLimiter>>,
    decoration: Option<SharedDecorationManager>,
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
            if let Some(decoration) = &self.decoration {
                id.set_handler(crate::decoration::tracking_buffer_handler(decoration));
                decoration.borrow_mut().record_buffer(id, width, height);
            }
            slf.send_create_stream(id, width, height, handle, r#type, attribs);
            return;
        }

        let vm = self.vm.clone();
        let handle_type = r#type;
        self.diag
            .borrow_mut()
            .warn("eglstream", "create-stream-denied", || {
                format!(
                    "[d2b-wlproxy] target={vm} event=eglstream reason=create-stream-denied handle-type={handle_type}"
                )
            });
        if let Some(client) = id.client() {
            client.disconnect();
        }
    }
}

struct FilterDrmHandler {
    decoration: SharedDecorationManager,
}

impl WlDrmHandler for FilterDrmHandler {
    fn handle_authenticate(&mut self, slf: &Rc<WlDrm>, id: u32) {
        slf.send_authenticate(id);
    }

    fn handle_create_buffer(
        &mut self,
        slf: &Rc<WlDrm>,
        id: &Rc<WlBuffer>,
        name: u32,
        width: i32,
        height: i32,
        stride: u32,
        format: u32,
    ) {
        id.set_handler(crate::decoration::tracking_buffer_handler(&self.decoration));
        self.decoration
            .borrow_mut()
            .record_buffer(id, width, height);
        slf.send_create_buffer(id, name, width, height, stride, format);
    }

    fn handle_create_planar_buffer(
        &mut self,
        slf: &Rc<WlDrm>,
        id: &Rc<WlBuffer>,
        name: u32,
        width: i32,
        height: i32,
        format: u32,
        offset0: i32,
        stride0: i32,
        offset1: i32,
        stride1: i32,
        offset2: i32,
        stride2: i32,
    ) {
        id.set_handler(crate::decoration::tracking_buffer_handler(&self.decoration));
        self.decoration
            .borrow_mut()
            .record_buffer(id, width, height);
        slf.send_create_planar_buffer(
            id, name, width, height, format, offset0, stride0, offset1, stride1, offset2, stride2,
        );
    }

    fn handle_create_prime_buffer(
        &mut self,
        slf: &Rc<WlDrm>,
        id: &Rc<WlBuffer>,
        name: &Rc<OwnedFd>,
        width: i32,
        height: i32,
        format: u32,
        offset0: i32,
        stride0: i32,
        offset1: i32,
        stride1: i32,
        offset2: i32,
        stride2: i32,
    ) {
        id.set_handler(crate::decoration::tracking_buffer_handler(&self.decoration));
        self.decoration
            .borrow_mut()
            .record_buffer(id, width, height);
        slf.send_create_prime_buffer(
            id, name, width, height, format, offset0, stride0, offset1, stride1, offset2, stride2,
        );
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

fn connect_bridge_nonblocking(path: &PathBuf) -> std::io::Result<UnixStream> {
    use nix::sys::socket::{AddressFamily, SockFlag, SockType, UnixAddr, connect, socket};
    use std::os::fd::AsRawFd;

    let fd = socket(
        AddressFamily::Unix,
        SockType::Stream,
        SockFlag::SOCK_NONBLOCK | SockFlag::SOCK_CLOEXEC,
        None,
    )
    .map_err(errno_to_io)?;
    let addr = UnixAddr::new(path).map_err(errno_to_io)?;
    match connect(fd.as_raw_fd(), &addr) {
        Ok(()) | Err(nix::errno::Errno::EINPROGRESS | nix::errno::Errno::EAGAIN) => {
            Ok(UnixStream::from(fd))
        }
        Err(error) => Err(errno_to_io(error)),
    }
}

fn errno_to_io(error: nix::errno::Errno) -> std::io::Error {
    std::io::Error::from_raw_os_error(error as i32)
}

/// Minimal `ClientHandler` that logs disconnections for debugging.
pub struct FilterClientHandler {
    vm: String,
    client: Weak<Client>,
    decoration: Option<SharedDecorationManager>,
}

impl FilterClientHandler {
    pub fn new(
        vm: String,
        client: Weak<Client>,
        decoration: Option<SharedDecorationManager>,
    ) -> Self {
        Self {
            vm,
            client,
            decoration,
        }
    }
}

impl ClientHandler for FilterClientHandler {
    fn disconnected(self: Box<Self>) {
        if let (Some(client), Some(decoration)) = (self.client.upgrade(), self.decoration.as_ref())
        {
            let mut objects: Vec<Rc<dyn Object>> = Vec::new();
            client.objects(&mut objects);
            let mut decoration = decoration.borrow_mut();
            for object in &objects {
                if let Some(surface) = object.try_downcast::<WlSurface>() {
                    decoration.surface_destroyed(&surface);
                }
            }
            for object in &objects {
                if let Some(buffer) = object.try_downcast::<WlBuffer>() {
                    decoration.remove_buffer(&buffer);
                }
            }
        }
        log::debug!("[d2b-wlproxy] target={} client disconnected", self.vm);
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
    use std::io::IoSliceMut;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixListener;

    use crate::{
        bridge::BridgeReconnectPolicy,
        policy::{FilterPolicy, PolicyInput},
    };

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
    fn wrapper_pointer_enter_translates_content_but_suppresses_rail() {
        let rail_edge = Fixed::from_i32_saturating(WRAPPER_RAIL_WIDTH as i32);
        let content_x = Fixed::from_i32_saturating(WRAPPER_RAIL_WIDTH as i32 + 42);

        assert_eq!(wrapper_content_pointer_x(Fixed::ZERO), None);
        assert_eq!(wrapper_content_pointer_x(rail_edge), Some(Fixed::ZERO));
        assert_eq!(
            wrapper_content_pointer_x(content_x),
            Some(Fixed::from_i32_saturating(42))
        );
    }

    #[test]
    fn pointer_motion_translation_matches_current_focus() {
        let wrapper_x = Fixed::from_i32_saturating(WRAPPER_RAIL_WIDTH as i32 + 10);
        let direct_x = Fixed::from_i32_saturating(10);

        assert_eq!(
            pointer_motion_x_for_focus(PointerFocus::WrapperContent, wrapper_x),
            Some(Fixed::from_i32_saturating(10))
        );
        assert_eq!(
            pointer_motion_x_for_focus(PointerFocus::Direct, direct_x),
            Some(direct_x)
        );
        assert_eq!(
            pointer_motion_x_for_focus(PointerFocus::Rail, wrapper_x),
            None
        );
        assert_eq!(
            pointer_motion_x_for_focus(PointerFocus::None, wrapper_x),
            None
        );
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
                offer: Weak::new(),
                source: Some(source.clone()),
                source_id: 1,
            })),
        );
        clipboard.offers.insert(
            3,
            Rc::new(RefCell::new(VirtualOffer {
                offer: Weak::new(),
                source: None,
                source_id: 3,
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
    fn nonblocking_bridge_connect_errors_trigger_backoff() {
        assert_eq!(
            errno_to_io(nix::errno::Errno::EAGAIN).raw_os_error(),
            Some(nix::errno::Errno::EAGAIN as i32)
        );
        assert_eq!(
            errno_to_io(nix::errno::Errno::EINPROGRESS).raw_os_error(),
            Some(nix::errno::Errno::EINPROGRESS as i32)
        );
    }

    #[test]
    fn bridge_handoff_is_queued_when_bridge_unavailable() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut clipboard = VirtualClipboardState::new(
            "work".to_owned(),
            diag,
            BridgeConfig::from_parts(
                Some(PathBuf::from("target/d2b-nonexistent-bridge.sock")),
                std::path::Path::new("/run/d2b/clipd"),
                None,
                "work",
                BridgeReconnectPolicy::default(),
            )
            .expect("bridge config"),
        );
        let (fd, _fd_peer) = UnixStream::pair().expect("transfer pair");
        let fd: OwnedFd = fd.into();
        let metadata = BridgeTransferMetadata {
            identity: ProxyIdentity::from("work"),
            mime_type: "text/plain".to_owned(),
            source_id: 7,
            kind: BridgeTransferKind::PasteRequest,
        };

        clipboard.handoff_via_bridge(&fd, &metadata);

        assert_eq!(clipboard.pending_handoff_count_for_tests(), 1);
        assert!(clipboard.bridge_retry_deadline().is_some());
    }

    #[test]
    fn flush_pending_bridge_handoffs_delivers_and_removes_queue_item() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut clipboard =
            VirtualClipboardState::new("work".to_owned(), diag, BridgeConfig::disabled());
        let (bridge, peer) = UnixStream::pair().expect("bridge pair");
        clipboard.bridge = Some(bridge);
        let (fd, _fd_peer) = UnixStream::pair().expect("transfer pair");
        clipboard
            .pending_bridge_handoffs
            .push_back(PendingBridgeHandoff {
                fd: fd.into(),
                metadata: BridgeTransferMetadata {
                    identity: ProxyIdentity::from("work"),
                    mime_type: "text/plain".to_owned(),
                    source_id: 7,
                    kind: BridgeTransferKind::PasteRequest,
                },
            });

        clipboard.flush_pending_bridge_handoffs();

        assert_eq!(clipboard.pending_handoff_count_for_tests(), 0);
        let mut frame = [0_u8; 256];
        let mut iov = [IoSliceMut::new(&mut frame)];
        let mut cmsg_space = vec![0_u8; crate::bridge::SCM_RIGHTS_MIN_CONTROL_BYTES];
        const {
            assert!(crate::bridge::SCM_RIGHTS_CONTROL_FD_SLOTS >= crate::bridge::SCM_RIGHTS_MIN_FDS)
        };
        let msg = nix::sys::socket::recvmsg::<()>(
            peer.as_raw_fd(),
            &mut iov,
            Some(&mut cmsg_space),
            nix::sys::socket::MsgFlags::MSG_CMSG_CLOEXEC,
        )
        .expect("recvmsg");
        assert!(crate::bridge::recv_flags_are_fail_closed(msg.flags));
        assert!(msg.bytes > 0);
        for cmsg in msg.cmsgs().expect("cmsgs") {
            if let nix::sys::socket::ControlMessageOwned::ScmRights(fds) = cmsg {
                for fd in fds {
                    let _ = nix::unistd::close(fd);
                }
            }
        }
    }

    #[test]
    fn pending_handoff_backpressure_stops_flush_and_requeues_front() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut clipboard =
            VirtualClipboardState::new("work".to_owned(), diag, BridgeConfig::disabled());
        let (fd, _fd_peer) = UnixStream::pair().expect("transfer pair");
        let pending = PendingBridgeHandoff {
            fd: fd.into(),
            metadata: BridgeTransferMetadata {
                identity: ProxyIdentity::from("work"),
                mime_type: "text/plain".to_owned(),
                source_id: 7,
                kind: BridgeTransferKind::PasteRequest,
            },
        };

        let step = clipboard
            .handle_pending_handoff_status(pending, crate::bridge::HandoffStatus::Backpressure);

        assert_eq!(step, PendingHandoffStep::Stop);
        assert_eq!(clipboard.pending_handoff_count_for_tests(), 1);
        assert_eq!(
            clipboard
                .pending_bridge_handoffs
                .front()
                .expect("requeued")
                .metadata
                .source_id,
            7
        );
    }

    #[test]
    fn queued_handoff_failure_preserves_queue_and_respects_backoff() {
        let root = PathBuf::from("target").join(format!(
            "wlp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create temp dir");
        let path = root.join("bridge.sock");
        let listener = UnixListener::bind(&path).expect("listener");
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut clipboard = VirtualClipboardState::new(
            "work".to_owned(),
            diag,
            BridgeConfig::from_parts(
                Some(path.clone()),
                std::path::Path::new("/run/d2b/clipd"),
                None,
                "work",
                BridgeReconnectPolicy::default(),
            )
            .expect("bridge config"),
        );
        let (bad_bridge, bad_peer) = UnixStream::pair().expect("bad bridge pair");
        drop(bad_peer);
        clipboard.bridge = Some(bad_bridge);
        clipboard.bridge_reconnect.start_connect();
        clipboard.bridge_reconnect.connect_succeeded();
        let metadata = BridgeTransferMetadata {
            identity: ProxyIdentity::from("work"),
            mime_type: "text/plain".to_owned(),
            source_id: 7,
            kind: BridgeTransferKind::PasteRequest,
        };
        for source_id in [7, 8] {
            let (fd, _peer) = UnixStream::pair().expect("transfer pair");
            let mut metadata = metadata.clone();
            metadata.source_id = source_id;
            clipboard
                .pending_bridge_handoffs
                .push_back(PendingBridgeHandoff {
                    fd: fd.into(),
                    metadata,
                });
        }

        clipboard.flush_pending_bridge_handoffs();

        assert_eq!(clipboard.pending_handoff_count_for_tests(), 2);
        assert!(!clipboard.has_connected_bridge_for_tests());
        assert!(clipboard.bridge_retry_deadline().is_some());
        drop(listener);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn handoff_error_detail_is_human_readable() {
        assert_eq!(handoff_error_detail(None), "short-write");
        let detail = handoff_error_detail(Some(nix::errno::Errno::EPIPE));

        assert!(!detail.chars().all(|ch| ch.is_ascii_digit()));
        assert!(!detail.is_empty());
    }

    #[test]
    fn filtered_globals_preserve_original_global_names() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut handler = FilterRegistryHandler::new(policy(), diag, clipboard(), None);

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
        let handler = FilterRegistryHandler::new(policy(), diag.clone(), clipboard(), None);

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
        let mut handler = FilterRegistryHandler::new(policy(), diag, clipboard(), None);
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
        let handler = FilterRegistryHandler::new(policy(), diag, clipboard(), None);

        assert_eq!(handler.allocate_synthetic_clipboard_name(7), u32::MAX);
    }

    #[test]
    fn synthetic_clipboard_global_avoids_server_and_existing_names() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut handler = FilterRegistryHandler::new(policy(), diag, clipboard(), None);
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
        let mut handler = FilterRegistryHandler::new(policy(), diag, clipboard(), None);
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
        let mut handler = FilterRegistryHandler::new(policy(), diag, clipboard(), None);
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
        let mut handler = FilterRegistryHandler::new(policy(), diag, clipboard(), None);
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
        let mut handler = FilterRegistryHandler::new(policy, diag, clipboard(), None);
        let (_synthetic, decision) =
            handler.prepare_global(13, ObjectInterface::ZwpPrimarySelectionDeviceManagerV1, 1);

        assert_eq!(decision, IncomingGlobalDecision::Hide);
        assert!(handler.hidden_globals.contains(&13));
        assert!(!handler.advertised_globals.contains_key(&13));
    }

    #[test]
    fn prepare_global_hides_text_input_v3() {
        let diag = Rc::new(RefCell::new(DiagRateLimiter::new("work".to_owned())));
        let mut handler = FilterRegistryHandler::new(policy(), diag, clipboard(), None);
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
