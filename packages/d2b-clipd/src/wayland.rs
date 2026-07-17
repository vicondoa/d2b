//! Host Wayland data-control client.
//!
//! Connects to the host compositor as a regular Wayland client and binds
//! `ext_data_control_manager_v1` (preferred) or `zwlr_data_control_manager_v1`
//! (fallback) to observe host clipboard selection changes and request payload
//! data from the clipboard source.
//!
//! Neither FD is ever forwarded to the picker process; the offer handle stays
//! on the daemon thread.

use std::collections::HashMap;
use std::os::fd::{AsFd, OwnedFd};

use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, backend::WaylandError, delegate_noop,
};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1, ext_data_control_manager_v1, ext_data_control_offer_v1,
    ext_data_control_source_v1,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1, zwlr_data_control_manager_v1, zwlr_data_control_offer_v1,
    zwlr_data_control_source_v1,
};

use crate::policy::{
    MAX_OFFER_MIME_TYPES, is_bounded_secret_hint, is_mime_allowed, normalize_mime,
};

const MAX_PENDING_DATA_OFFERS: usize = 64;

// ─── Public event type ───────────────────────────────────────────────────────

/// Events emitted by [`DataControlClient`] to the main event loop.
/// None of these are `Send`; they must be consumed on the same thread.
#[derive(Debug)]
pub enum HostClipboardEvent {
    /// Host selection changed.  `allowed_mimes` contains only MIME types from
    /// the policy allowlist.  `has_secret` indicates a password-manager hint.
    /// Call [`DataControlOffer::receive`] (then flush + drop write end) to get
    /// the data.  `offer` is `None` when the selection has no allowed MIME
    /// types (i.e. the content cannot be pasted).
    SelectionChanged {
        offer: Option<DataControlOffer>,
        allowed_mimes: Vec<String>,
        has_secret: bool,
    },
    /// Host selection cleared (no active selection).
    SelectionCleared,
    /// Compositor asked our source to write `mime_type` to `fd`.
    SourceSendRequest {
        source_id: u64,
        mime_type: String,
        /// Caller must write data then drop/close this fd.
        fd: OwnedFd,
    },
    /// Our data-control source was cancelled (another app took the selection).
    SourceCancelled { source_id: u64 },
    /// The data-control device was finished by the compositor (seat removed).
    DeviceFinished,
}

// ─── Offer handle ────────────────────────────────────────────────────────────

/// A live data-control offer from which clipboard data can be requested.
/// Drop to destroy the protocol object.
#[derive(Debug)]
pub enum DataControlOffer {
    Ext(ext_data_control_offer_v1::ExtDataControlOfferV1),
    Zwlr(zwlr_data_control_offer_v1::ZwlrDataControlOfferV1),
}

impl DataControlOffer {
    /// Send a `receive` request: the compositor will write `mime_type` data
    /// to `write_fd`. Caller must flush the connection then drop `write_fd`.
    pub fn receive(&self, mime_type: String, write_fd: &OwnedFd) {
        match self {
            Self::Ext(offer) => offer.receive(mime_type, write_fd.as_fd()),
            Self::Zwlr(offer) => offer.receive(mime_type, write_fd.as_fd()),
        }
    }

    pub fn destroy(self) {
        match self {
            Self::Ext(offer) => offer.destroy(),
            Self::Zwlr(offer) => offer.destroy(),
        }
    }
}

// ─── Source handle ───────────────────────────────────────────────────────────

/// A data-control source that exposes d2b clipboard data to the compositor.
#[derive(Debug)]
pub enum DataControlSource {
    Ext(ext_data_control_source_v1::ExtDataControlSourceV1),
    Zwlr(zwlr_data_control_source_v1::ZwlrDataControlSourceV1),
}

impl DataControlSource {
    pub fn offer_mime(&self, mime: String) {
        match self {
            Self::Ext(s) => s.offer(mime),
            Self::Zwlr(s) => s.offer(mime),
        }
    }

    pub fn destroy(self) {
        match self {
            Self::Ext(s) => s.destroy(),
            Self::Zwlr(s) => s.destroy(),
        }
    }
}

// ─── Internal state ──────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct PendingOffer {
    allowed_mimes: Vec<String>,
    has_secret: bool,
}

#[derive(Debug)]
enum DataControlManagerState {
    Probing,
    Ext {
        manager: ext_data_control_manager_v1::ExtDataControlManagerV1,
        device: Option<ext_data_control_device_v1::ExtDataControlDeviceV1>,
    },
    Zwlr {
        manager: zwlr_data_control_manager_v1::ZwlrDataControlManagerV1,
        device: Option<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1>,
    },
    Unavailable,
}

impl DataControlManagerState {
    fn is_probing(&self) -> bool {
        matches!(self, Self::Probing)
    }

    fn protocol_name(&self) -> &'static str {
        match self {
            Self::Ext { .. } => "ext_data_control_manager_v1",
            Self::Zwlr { .. } => "zwlr_data_control_manager_v1",
            _ => "none",
        }
    }
}

struct WlState {
    manager_state: DataControlManagerState,
    seat: Option<wl_seat::WlSeat>,
    /// Offers assembling MIME lists before `selection` event arrives.
    /// Keyed by Wayland object id (protocol_id from ObjectId).
    pending: HashMap<u32, PendingOffer>,
    /// Fully assembled offer proxies, keyed by protocol_id, kept alive.
    live: HashMap<u32, DataControlOffer>,
    /// Sequence counter for source ids emitted in events.
    source_seq: u64,
    /// Events to deliver to the main loop.
    pub events: Vec<HostClipboardEvent>,
}

impl WlState {
    fn new() -> Self {
        Self {
            manager_state: DataControlManagerState::Probing,
            seat: None,
            pending: HashMap::new(),
            live: HashMap::new(),
            source_seq: 0,
            events: Vec::new(),
        }
    }

    fn next_source_seq(&mut self) -> u64 {
        self.source_seq += 1;
        self.source_seq
    }

    /// Create the data-control device once we have both a seat and a manager.
    fn try_create_device(&mut self, qh: &QueueHandle<Self>) {
        let Some(seat) = &self.seat else { return };
        match &self.manager_state {
            DataControlManagerState::Ext {
                manager,
                device: None,
            } => {
                let dev = manager.get_data_device(seat, qh, ());
                if let DataControlManagerState::Ext { device, .. } = &mut self.manager_state {
                    *device = Some(dev);
                }
            }
            DataControlManagerState::Zwlr {
                manager,
                device: None,
            } => {
                let dev = manager.get_data_device(seat, qh, ());
                if let DataControlManagerState::Zwlr { device, .. } = &mut self.manager_state {
                    *device = Some(dev);
                }
            }
            _ => {}
        }
    }

    /// Handle a new offer object created by a `data_offer` device event.
    fn on_data_offer_ext(&mut self, offer: ext_data_control_offer_v1::ExtDataControlOfferV1) {
        if self.pending.len() >= MAX_PENDING_DATA_OFFERS {
            offer.destroy();
            return;
        }
        let id = offer.id().protocol_id();
        self.pending.insert(id, PendingOffer::default());
        self.live.insert(id, DataControlOffer::Ext(offer));
    }

    fn on_data_offer_zwlr(&mut self, offer: zwlr_data_control_offer_v1::ZwlrDataControlOfferV1) {
        if self.pending.len() >= MAX_PENDING_DATA_OFFERS {
            offer.destroy();
            return;
        }
        let id = offer.id().protocol_id();
        self.pending.insert(id, PendingOffer::default());
        self.live.insert(id, DataControlOffer::Zwlr(offer));
    }

    /// Handle `offer(mime_type)` on a pending offer.
    fn on_offer_mime(&mut self, proxy_id: u32, mime_type: String) {
        if let Some(pending) = self.pending.get_mut(&proxy_id) {
            if is_bounded_secret_hint(&mime_type) {
                pending.has_secret = true;
                return;
            }
            if !is_mime_allowed(&mime_type) || pending.allowed_mimes.len() == MAX_OFFER_MIME_TYPES {
                return;
            }
            let mime_type = normalize_mime(&mime_type);
            if !pending.allowed_mimes.contains(&mime_type) {
                pending.allowed_mimes.push(mime_type);
            }
        }
    }

    /// Handle `selection(Some(offer))` - move from pending to active event.
    fn on_selection_ext(
        &mut self,
        offer: Option<ext_data_control_offer_v1::ExtDataControlOfferV1>,
    ) {
        match offer {
            None => self.events.push(HostClipboardEvent::SelectionCleared),
            Some(proxy) => {
                let id = proxy.id().protocol_id();
                self.finalize_selection(id);
            }
        }
    }

    fn on_selection_zwlr(
        &mut self,
        offer: Option<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1>,
    ) {
        match offer {
            None => self.events.push(HostClipboardEvent::SelectionCleared),
            Some(proxy) => {
                let id = proxy.id().protocol_id();
                self.finalize_selection(id);
            }
        }
    }

    fn finalize_selection(&mut self, id: u32) {
        let pending = match self.pending.remove(&id) {
            Some(p) => p,
            None => {
                // Unknown offer id - ignore (may be primary selection or stale).
                return;
            }
        };
        // `live` may be absent in tests that directly drive WlState without a
        // compositor, or in rare race conditions (offer destroyed before selection).
        let live = self.live.remove(&id);

        let has_secret = pending.has_secret;
        let allowed_mimes = pending.allowed_mimes;

        // Emit the event regardless so the host clipboard can track attribution
        // and clear stale state.  offer is None when no allowed MIME types exist
        // (content unpasteable) or when the live proxy is unavailable.
        let offer = if allowed_mimes.is_empty() {
            if let Some(o) = live {
                o.destroy();
            }
            None
        } else {
            live
        };
        self.events.push(HostClipboardEvent::SelectionChanged {
            offer,
            allowed_mimes,
            has_secret,
        });
    }
}

// ─── wl_registry dispatch ────────────────────────────────────────────────────

impl Dispatch<wl_registry::WlRegistry, ()> for WlState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_seat" if state.seat.is_none() => {
                let seat = registry.bind::<wl_seat::WlSeat, _, _>(name, 1_u32.min(version), qh, ());
                state.seat = Some(seat);
                state.try_create_device(qh);
            }
            "ext_data_control_manager_v1" if state.manager_state.is_probing() => {
                let manager = registry
                    .bind::<ext_data_control_manager_v1::ExtDataControlManagerV1, _, _>(
                        name,
                        1_u32.min(version),
                        qh,
                        (),
                    );
                state.manager_state = DataControlManagerState::Ext {
                    manager,
                    device: None,
                };
                state.try_create_device(qh);
            }
            "zwlr_data_control_manager_v1" if state.manager_state.is_probing() => {
                let manager = registry
                    .bind::<zwlr_data_control_manager_v1::ZwlrDataControlManagerV1, _, _>(
                        name,
                        1_u32.min(version),
                        qh,
                        (),
                    );
                state.manager_state = DataControlManagerState::Zwlr {
                    manager,
                    device: None,
                };
                state.try_create_device(qh);
            }
            _ => {}
        }
    }
}

// wl_seat – we only need it to exist; we don't act on capability events.
delegate_noop!(WlState: ignore wl_seat::WlSeat);

// ─── ext_data_control_manager_v1 ─────────────────────────────────────────────

delegate_noop!(WlState: ignore ext_data_control_manager_v1::ExtDataControlManagerV1);

// ─── ext_data_control_device_v1 ──────────────────────────────────────────────

impl Dispatch<ext_data_control_device_v1::ExtDataControlDeviceV1, ()> for WlState {
    fn event(
        state: &mut Self,
        _: &ext_data_control_device_v1::ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use ext_data_control_device_v1::Event;
        match event {
            Event::DataOffer { id } => state.on_data_offer_ext(id),
            Event::Selection { id } => state.on_selection_ext(id),
            Event::Finished => state.events.push(HostClipboardEvent::DeviceFinished),
            // Primary selection events are deliberately not acted upon (high-frequency, ADR 0042).
            _ => {}
        }
    }

    wayland_client::event_created_child!(WlState, ext_data_control_device_v1::ExtDataControlDeviceV1, [
        ext_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (ext_data_control_offer_v1::ExtDataControlOfferV1, ())
    ]);
}

// ─── ext_data_control_offer_v1 ───────────────────────────────────────────────

impl Dispatch<ext_data_control_offer_v1::ExtDataControlOfferV1, ()> for WlState {
    fn event(
        state: &mut Self,
        proxy: &ext_data_control_offer_v1::ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.on_offer_mime(proxy.id().protocol_id(), mime_type);
        }
    }
}

// ─── ext_data_control_source_v1 ──────────────────────────────────────────────

impl Dispatch<ext_data_control_source_v1::ExtDataControlSourceV1, u64> for WlState {
    fn event(
        state: &mut Self,
        _: &ext_data_control_source_v1::ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        &source_id: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_data_control_source_v1::Event;
        match event {
            Event::Send { mime_type, fd } if is_mime_allowed(&mime_type) => {
                state.events.push(HostClipboardEvent::SourceSendRequest {
                    source_id,
                    mime_type: normalize_mime(&mime_type),
                    fd,
                });
            }
            Event::Cancelled => {
                state
                    .events
                    .push(HostClipboardEvent::SourceCancelled { source_id });
            }
            _ => {}
        }
    }
}

// ─── zwlr_data_control_manager_v1 ────────────────────────────────────────────

delegate_noop!(WlState: ignore zwlr_data_control_manager_v1::ZwlrDataControlManagerV1);

// ─── zwlr_data_control_device_v1 ─────────────────────────────────────────────

impl Dispatch<zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, ()> for WlState {
    fn event(
        state: &mut Self,
        _: &zwlr_data_control_device_v1::ZwlrDataControlDeviceV1,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        use zwlr_data_control_device_v1::Event;
        match event {
            Event::DataOffer { id } => state.on_data_offer_zwlr(id),
            Event::Selection { id } => state.on_selection_zwlr(id),
            Event::Finished => state.events.push(HostClipboardEvent::DeviceFinished),
            _ => {}
        }
    }

    wayland_client::event_created_child!(WlState, zwlr_data_control_device_v1::ZwlrDataControlDeviceV1, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ())
    ]);
}

// ─── zwlr_data_control_offer_v1 ──────────────────────────────────────────────

impl Dispatch<zwlr_data_control_offer_v1::ZwlrDataControlOfferV1, ()> for WlState {
    fn event(
        state: &mut Self,
        proxy: &zwlr_data_control_offer_v1::ZwlrDataControlOfferV1,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.on_offer_mime(proxy.id().protocol_id(), mime_type);
        }
    }
}

// ─── zwlr_data_control_source_v1 ─────────────────────────────────────────────

impl Dispatch<zwlr_data_control_source_v1::ZwlrDataControlSourceV1, u64> for WlState {
    fn event(
        state: &mut Self,
        _: &zwlr_data_control_source_v1::ZwlrDataControlSourceV1,
        event: zwlr_data_control_source_v1::Event,
        &source_id: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_data_control_source_v1::Event;
        match event {
            Event::Send { mime_type, fd } if is_mime_allowed(&mime_type) => {
                state.events.push(HostClipboardEvent::SourceSendRequest {
                    source_id,
                    mime_type: normalize_mime(&mime_type),
                    fd,
                });
            }
            Event::Cancelled => {
                state
                    .events
                    .push(HostClipboardEvent::SourceCancelled { source_id });
            }
            _ => {}
        }
    }
}

// ─── Public client handle ─────────────────────────────────────────────────────

/// Host clipboard observer; wraps the Wayland connection and event queue.
pub struct DataControlClient {
    conn: Connection,
    event_queue: EventQueue<WlState>,
    state: WlState,
}

/// Error from connecting or polling the Wayland data-control client.
#[derive(Debug, thiserror::Error)]
pub enum DataControlError {
    #[error("failed to connect to Wayland display: {0}")]
    Connect(String),
    #[error("Wayland protocol error: {0}")]
    Protocol(String),
    #[error(
        "no data-control protocol available (tried ext_data_control_manager_v1 and zwlr_data_control_manager_v1)"
    )]
    ProtocolUnavailable,
}

impl DataControlClient {
    /// Connect to the host compositor via `$WAYLAND_DISPLAY` or
    /// `$WAYLAND_SOCKET`.  Performs an initial roundtrip to discover globals
    /// and a second roundtrip to finish device creation.
    pub fn connect() -> Result<Self, DataControlError> {
        let conn =
            Connection::connect_to_env().map_err(|e| DataControlError::Connect(e.to_string()))?;
        let mut event_queue: EventQueue<WlState> = conn.new_event_queue();
        let mut state = WlState::new();
        let qh = event_queue.handle();

        let display = conn.display();
        let _registry = display.get_registry(&qh, ());

        // First roundtrip: discover globals (seat + data-control manager).
        event_queue
            .roundtrip(&mut state)
            .map_err(|e| DataControlError::Protocol(e.to_string()))?;

        if matches!(state.manager_state, DataControlManagerState::Unavailable)
            || matches!(state.manager_state, DataControlManagerState::Probing)
        {
            state.manager_state = DataControlManagerState::Unavailable;
            return Err(DataControlError::ProtocolUnavailable);
        }

        // Second roundtrip: finish device creation and receive any initial
        // selection event.
        event_queue
            .roundtrip(&mut state)
            .map_err(|e| DataControlError::Protocol(e.to_string()))?;

        log::info!(
            "d2b-clipd: data-control connected via {}",
            state.manager_state.protocol_name()
        );

        Ok(Self {
            conn,
            event_queue,
            state,
        })
    }

    /// Create a new data-control source that offers the given MIME types.
    /// Returns a `(DataControlSource, source_id)` pair; the source_id appears
    /// in subsequent `SourceSendRequest` and `SourceCancelled` events.
    pub fn create_source(
        &mut self,
        mimes: &[String],
    ) -> Result<(DataControlSource, u64), DataControlError> {
        let qh = self.event_queue.handle();
        let source_id = self.state.next_source_seq();
        let source = match &self.state.manager_state {
            DataControlManagerState::Ext { manager, .. } => {
                let s = manager.create_data_source(&qh, source_id);
                DataControlSource::Ext(s)
            }
            DataControlManagerState::Zwlr { manager, .. } => {
                let s = manager.create_data_source(&qh, source_id);
                DataControlSource::Zwlr(s)
            }
            _ => return Err(DataControlError::ProtocolUnavailable),
        };
        let mut offered = Vec::new();
        for mime in mimes {
            if offered.len() == MAX_OFFER_MIME_TYPES || !is_mime_allowed(mime) {
                continue;
            }
            let mime = normalize_mime(mime);
            if !offered.contains(&mime) {
                source.offer_mime(mime.clone());
                offered.push(mime);
            }
        }
        Ok((source, source_id))
    }

    /// Set this source as the current clipboard selection (for paste-back from
    /// VM clipboard to host).
    pub fn set_selection(&self, source: &DataControlSource) -> Result<(), DataControlError> {
        match (&self.state.manager_state, source) {
            (
                DataControlManagerState::Ext {
                    device: Some(dev), ..
                },
                DataControlSource::Ext(s),
            ) => {
                dev.set_selection(Some(s));
                Ok(())
            }
            (
                DataControlManagerState::Zwlr {
                    device: Some(dev), ..
                },
                DataControlSource::Zwlr(s),
            ) => {
                dev.set_selection(Some(s));
                Ok(())
            }
            _ => Err(DataControlError::ProtocolUnavailable),
        }
    }

    /// Flush outgoing requests to the compositor.
    pub fn flush(&self) -> Result<(), DataControlError> {
        self.conn
            .flush()
            .map_err(|e| DataControlError::Protocol(e.to_string()))
    }

    /// Dispatch pending events and return any `HostClipboardEvent`s collected.
    pub fn dispatch_pending(&mut self) -> Result<Vec<HostClipboardEvent>, DataControlError> {
        self.event_queue
            .dispatch_pending(&mut self.state)
            .map_err(|e| DataControlError::Protocol(e.to_string()))?;
        Ok(std::mem::take(&mut self.state.events))
    }

    /// Prepare a non-blocking read from the Wayland socket.
    /// Call after `poll` returns POLLIN on [`Self::as_fd`].
    pub fn prepare_and_read(&self) -> Result<(), DataControlError> {
        match self.conn.prepare_read() {
            Some(guard) => match guard.read() {
                Ok(_) => Ok(()),
                Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    Ok(())
                }
                Err(error) => Err(DataControlError::Protocol(error.to_string())),
            },
            None => Ok(()), // no events buffered / another path already prepared
        }
    }

    /// Borrowed fd suitable for use with `rustix::io::poll`.
    pub fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.conn.as_fd()
    }

    /// Raw Wayland socket fd (deprecated; prefer `as_fd`).
    pub fn as_raw_fd(&self) -> std::os::unix::io::RawFd {
        use std::os::fd::AsRawFd;
        self.conn.as_fd().as_raw_fd()
    }

    /// Whether the data-control protocol is available.
    pub fn is_available(&self) -> bool {
        !matches!(
            self.state.manager_state,
            DataControlManagerState::Unavailable | DataControlManagerState::Probing
        )
    }
}

// ─── Pure-model tests (no Wayland compositor needed) ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_offer_accumulates_mimes_and_allows_only_allowlisted() {
        let mut state = WlState::new();
        // Simulate a data_offer for a hypothetical protocol object id 5.
        state.pending.insert(5, PendingOffer::default());

        // Offer MIME events arrive before selection.
        state.on_offer_mime(5, "text/plain".to_owned());
        state.on_offer_mime(5, "text/html".to_owned());
        state.on_offer_mime(5, "application/octet-stream".to_owned());
        state.on_offer_mime(5, "x-kde-passwordManagerHint".to_owned());

        let pending = state.pending.remove(&5).expect("pending");
        assert!(pending.has_secret, "password hint must be detected");
        assert_eq!(
            pending.allowed_mimes,
            ["text/plain", "text/html"],
            "only allowlisted mimes"
        );
    }

    #[test]
    fn pending_offer_deduplicates_and_bounds_mimes() {
        let mut state = WlState::new();
        state.pending.insert(5, PendingOffer::default());
        for _ in 0..MAX_OFFER_MIME_TYPES * 2 {
            state.on_offer_mime(5, "text/plain".to_owned());
        }
        state.on_offer_mime(5, "x".repeat(crate::policy::MAX_MIME_TYPE_BYTES + 1));
        let pending = state.pending.remove(&5).unwrap();
        assert_eq!(pending.allowed_mimes, ["text/plain"]);
        assert!(!pending.has_secret);
    }

    #[test]
    fn selection_cleared_emits_event() {
        let mut state = WlState::new();
        state.on_selection_ext(None);
        assert!(
            matches!(
                state.events.as_slice(),
                [HostClipboardEvent::SelectionCleared]
            ),
            "expected SelectionCleared"
        );
    }

    #[test]
    fn offer_with_no_allowed_mimes_still_emits_selection_changed() {
        let mut state = WlState::new();
        state.pending.insert(
            7,
            PendingOffer {
                allowed_mimes: Vec::new(),
                has_secret: false,
            },
        );
        state.finalize_selection(7);
        // A SelectionChanged is emitted (attribution tracking) but offer is None
        // (the content cannot be pasted since all MIME types are policy-denied).
        assert!(
            matches!(
                state.events.as_slice(),
                [HostClipboardEvent::SelectionChanged { offer: None, allowed_mimes, .. }]
                    if allowed_mimes.is_empty()
            ),
            "expected SelectionChanged with offer=None for denied-mime selection; got {:?}",
            state.events
        );
    }
}
