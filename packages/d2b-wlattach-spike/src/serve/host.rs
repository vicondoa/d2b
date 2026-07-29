//! The persistent session host.
//!
//! A Smithay `wayland-server` that owns the application's connection and the
//! shadow surface tree. It never renders and never touches a GPU: its job is to
//! hold state so a fresh window frontend can rebuild the windows on a brand-new
//! compositor connection.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use smithay::input::{Seat, SeatHandler, SeatState, pointer::CursorImageStatus};
use smithay::output::{Mode, Output, PhysicalProperties, Subpixel};
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, DisplayHandle, Resource};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    CompositorClientState, CompositorHandler, CompositorState, SurfaceAttributes, with_states,
};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
    XdgToplevelSurfaceData,
};
use smithay::wayland::shm::{ShmHandler, ShmState};

use crate::model::ids::{IdAllocator, SurfaceId};

/// Per-client state Smithay requires.
#[derive(Default)]
pub struct ClientState {
    pub compositor: CompositorClientState,
}

impl smithay::reexports::wayland_server::backend::ClientData for ClientState {}

/// A pixel snapshot of a surface's last committed content.
///
/// For SHM we copy at commit time and release the application's buffer
/// immediately. That mirrors what a real compositor's software path does, keeps
/// the client's pool turning over while detached, and removes any question of
/// pool lifetime across a frontend restart. The zero-copy claim in the design is
/// scoped to DMA-BUF, which is handled by descriptor pass-through instead.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Snapshot {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    /// `wl_shm` format code.
    pub format: u32,
    pub pixels: Vec<u8>,
}

/// Everything retained about one surface across frontend generations.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct ShadowSurface {
    pub id: Option<SurfaceId>,
    pub title: String,
    pub app_id: String,
    pub snapshot: Option<Snapshot>,
    /// Set once the surface has a toplevel role.
    pub is_toplevel: bool,
}

/// The shadow tree, shared with the frontend-facing half of the daemon.
#[derive(Debug, Default)]
pub struct Shadow {
    pub surfaces: HashMap<u32, ShadowSurface>,
    /// Bumped whenever committed content changes, so the frontend half knows
    /// there is something new to send.
    pub revision: u64,
}

impl Shadow {
    pub fn toplevels(&self) -> Vec<(u32, ShadowSurface)> {
        let mut v: Vec<_> = self
            .surfaces
            .iter()
            .filter(|(_, s)| s.is_toplevel)
            .map(|(k, s)| (*k, s.clone()))
            .collect();
        v.sort_by_key(|(k, _)| *k);
        v
    }
}

pub struct SessionHost {
    pub compositor: CompositorState,
    pub shm: ShmState,
    pub xdg: XdgShellState,
    pub data_device: DataDeviceState,
    pub seats: SeatState<Self>,
    pub seat: Seat<Self>,
    pub keyboard: Option<smithay::input::keyboard::KeyboardHandle<Self>>,
    pub pointer: Option<smithay::input::pointer::PointerHandle<Self>>,
    pub output: Output,
    pub shadow: Arc<Mutex<Shadow>>,
    pub ids: IdAllocator,
    pub toplevels: Vec<ToplevelSurface>,
    pub running: bool,
}

impl SessionHost {
    pub fn new(dh: &DisplayHandle) -> Self {
        let compositor = CompositorState::new::<Self>(dh);
        // Only the formats we actually reconstruct.
        let shm = ShmState::new::<Self>(dh, Vec::new());
        let xdg = XdgShellState::new::<Self>(dh);
        // Required in the baseline: toolkits (GTK, foot) defer wl_seat creation
        // until wl_data_device_manager exists, so omitting it breaks input --
        // or, for foot, startup -- entirely. Clipboard transfer stays unsupported.
        let data_device = DataDeviceState::new::<Self>(dh);
        let mut seats = SeatState::new();
        let mut seat = seats.new_wl_seat(dh, "wlattach");
        // Without these the seat advertises no capabilities and toolkits will
        // not deliver input at all.
        let keyboard =
            match seat.add_keyboard(smithay::input::keyboard::XkbConfig::default(), 200, 25) {
                Ok(k) => Some(k),
                Err(e) => {
                    log::error!("no keyboard on the seat: {e}");
                    None
                }
            };
        let pointer = Some(seat.add_pointer());
        log::info!(
            "seat capabilities: keyboard={} pointer={}",
            keyboard.is_some(),
            pointer.is_some()
        );

        // One synthetic output whose identity is stable across frontend
        // generations, so the application never sees an output disappear.
        let output = Output::new(
            "wlattach-0".to_owned(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "d2b".to_owned(),
                model: "wlattach".to_owned(),
            },
        );
        let mode = Mode {
            size: (1280, 800).into(),
            refresh: 60_000,
        };
        output.change_current_state(Some(mode), None, None, Some((0, 0).into()));
        output.set_preferred(mode);
        let _global = output.create_global::<Self>(dh);

        Self {
            compositor,
            shm,
            xdg,
            data_device,
            seats,
            seat,
            keyboard,
            pointer,
            output,
            shadow: Arc::new(Mutex::new(Shadow::default())),
            ids: IdAllocator::new(),
            toplevels: Vec::new(),
            running: true,
        }
    }

    fn surface_key(surface: &WlSurface) -> u32 {
        surface.id().protocol_id()
    }
}

impl CompositorHandler for SessionHost {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        // A client without our data would be a Smithay invariant violation, but
        // we still refuse to panic on it (plan §5.3).
        client
            .get_data::<ClientState>()
            .map(|s| &s.compositor)
            .unwrap_or_else(|| {
                static FALLBACK: std::sync::OnceLock<CompositorClientState> =
                    std::sync::OnceLock::new();
                FALLBACK.get_or_init(CompositorClientState::default)
            })
    }

    fn commit(&mut self, surface: &WlSurface) {
        let key = Self::surface_key(surface);
        let mut snapshot = None;
        let mut cleared = false;
        let mut title = String::new();
        let mut app_id = String::new();

        with_states(surface, |states| {
            // Take the committed assignment so we own the decision about when
            // the client may reuse its memory.
            let mut attrs = states.cached_state.get::<SurfaceAttributes>();
            match attrs.current().buffer.take() {
                Some(smithay::wayland::compositor::BufferAssignment::NewBuffer(buf)) => {
                    snapshot = crate::serve::sys::copy_shm(&buf);
                    if snapshot.is_none() {
                        // A new buffer arrived but we could not copy it (over the
                        // size cap, or bad geometry). Retaining the previous frame
                        // would later be presented as if it were current, so drop
                        // it and let the surface remap on the next good frame.
                        cleared = true;
                    }
                    // We have our own copy, so the application may reuse the
                    // buffer immediately. Without this a single-buffered client
                    // stalls forever waiting for a release that never comes.
                    buf.release();
                }
                Some(smithay::wayland::compositor::BufferAssignment::Removed) => {
                    // A null commit unmaps the surface; the retained frame is no
                    // longer what the application is showing.
                    cleared = true;
                }
                None => {}
            }

            if let Some(data) = states.data_map.get::<XdgToplevelSurfaceData>()
                && let Ok(role) = data.lock()
            {
                title = role.title.clone().unwrap_or_default();
                app_id = role.app_id.clone().unwrap_or_default();
            }
        });

        log::debug!("commit key={key} snapshot={}", snapshot.is_some());
        if let Ok(mut shadow) = self.shadow.lock() {
            let entry = shadow.surfaces.entry(key).or_default();
            if entry.id.is_none() {
                entry.id = Some(self.ids.surface());
            }
            if !title.is_empty() {
                entry.title = title;
            }
            if !app_id.is_empty() {
                entry.app_id = app_id;
            }
            if cleared {
                entry.snapshot = None;
            } else if let Some(s) = snapshot {
                entry.snapshot = Some(s);
            }
            shadow.revision = shadow.revision.wrapping_add(1);
        }
    }
}

impl BufferHandler for SessionHost {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for SessionHost {
    fn shm_state(&self) -> &ShmState {
        &self.shm
    }
}

impl XdgShellHandler for SessionHost {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let key = Self::surface_key(surface.wl_surface());
        if let Ok(mut shadow) = self.shadow.lock() {
            let entry = shadow.surfaces.entry(key).or_default();
            entry.is_toplevel = true;
            if entry.id.is_none() {
                entry.id = Some(self.ids.surface());
            }
        }
        // Give the client a concrete size, then send the initial configure. It
        // must not attach a buffer before acking this.
        surface.with_pending_state(|s| {
            s.size = Some((1024, 700).into());
            s.states.set(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Activated);
        });
        surface.send_configure();
        self.toplevels.push(surface);
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {
        // Popups arrive in a later phase.
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: smithay::utils::Serial) {}

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let key = Self::surface_key(surface.wl_surface());
        if let Ok(mut shadow) = self.shadow.lock()
            && shadow.surfaces.remove(&key).is_some()
        {
            // Bump the revision, or publish() skips the write and the frontend
            // never learns the window is gone -- leaving it on screen.
            shadow.revision = shadow.revision.wrapping_add(1);
        }
        self.toplevels.retain(|t| t != &surface);
        // Destroying the last toplevel does NOT end the session. An application
        // may legitimately close a window and keep running; only the
        // application process or its Wayland connection going away ends it.
    }
}

impl SeatHandler for SessionHost {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seats
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {}
    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}
}

smithay::delegate_compositor!(SessionHost);
smithay::delegate_shm!(SessionHost);
smithay::delegate_xdg_shell!(SessionHost);
smithay::delegate_seat!(SessionHost);

impl SelectionHandler for SessionHost {
    type SelectionUserData = ();
}

impl ClientDndGrabHandler for SessionHost {}
impl ServerDndGrabHandler for SessionHost {}

impl DataDeviceHandler for SessionHost {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device
    }
}

smithay::delegate_data_device!(SessionHost);

impl smithay::wayland::output::OutputHandler for SessionHost {}

smithay::delegate_output!(SessionHost);

impl SessionHost {
    /// Forward a compositor close request to the application's toplevel.
    ///
    /// This is advisory: the application may prompt to save, ignore it, or close
    /// only that window. It is emphatically not a detach, and it does not end
    /// the session by itself.
    pub fn request_close(&self, key: u32) -> bool {
        for t in &self.toplevels {
            if Self::surface_key(t.wl_surface()) == key {
                t.send_close();
                return true;
            }
        }
        false
    }
}

impl SessionHost {
    /// The surface input should be delivered to.
    ///
    /// One toplevel is the common case for the prototype; the first is used.
    pub fn focus_surface(&self) -> Option<WlSurface> {
        self.toplevels.first().map(|t| t.wl_surface().clone())
    }
}

/// Deliver one frontend input event to the application.
///
/// Serials are minted here, in the session host's own space. Serials from the
/// frontend's compositor connection are never forwarded: they belong to a
/// different connection and would be meaningless — or worse, misinterpreted —
/// on this one.
pub fn apply_input(state: &mut SessionHost, ev: crate::wire::dto::InputEvent) {
    use crate::wire::dto::InputEvent as E;
    use smithay::backend::input::{ButtonState, KeyState};
    use smithay::input::keyboard::FilterResult;
    use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
    use smithay::utils::SERIAL_COUNTER;

    log::debug!("input {ev:?}");
    let Some(surface) = state.focus_surface() else {
        log::debug!("input dropped: no focus surface");
        return;
    };
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u32)
        .unwrap_or(0);

    match ev {
        E::PointerEnter { x, y } | E::PointerMotion { x, y } => {
            if let Some(p) = state.pointer.clone() {
                let serial = SERIAL_COUNTER.next_serial();
                p.motion(
                    state,
                    Some((surface, (0.0, 0.0).into())),
                    &MotionEvent {
                        location: (x, y).into(),
                        serial,
                        time,
                    },
                );
                p.frame(state);
            }
        }
        E::PointerLeave => {
            if let Some(p) = state.pointer.clone() {
                let serial = SERIAL_COUNTER.next_serial();
                p.motion(
                    state,
                    None,
                    &MotionEvent {
                        location: (0.0, 0.0).into(),
                        serial,
                        time,
                    },
                );
                p.frame(state);
            }
        }
        E::PointerButton { button, pressed } => {
            if let Some(p) = state.pointer.clone() {
                let serial = SERIAL_COUNTER.next_serial();
                p.button(
                    state,
                    &ButtonEvent {
                        button,
                        state: if pressed {
                            ButtonState::Pressed
                        } else {
                            ButtonState::Released
                        },
                        serial,
                        time,
                    },
                );
                p.frame(state);
            }
        }
        E::PointerAxis {
            horizontal,
            vertical,
        } => {
            if let Some(p) = state.pointer.clone() {
                let mut frame =
                    AxisFrame::new(time).source(smithay::backend::input::AxisSource::Wheel);
                if horizontal != 0.0 {
                    frame = frame.value(smithay::backend::input::Axis::Horizontal, horizontal);
                }
                if vertical != 0.0 {
                    frame = frame.value(smithay::backend::input::Axis::Vertical, vertical);
                }
                p.axis(state, frame);
                p.frame(state);
            }
        }
        E::KeyboardEnter => {
            if let Some(k) = state.keyboard.clone() {
                let serial = SERIAL_COUNTER.next_serial();
                k.set_focus(state, Some(surface), serial);
            }
        }
        E::KeyboardLeave => {
            if let Some(k) = state.keyboard.clone() {
                let serial = SERIAL_COUNTER.next_serial();
                k.set_focus(state, None, serial);
            }
        }
        E::KeyboardKey { key, pressed } => {
            if let Some(k) = state.keyboard.clone() {
                let serial = SERIAL_COUNTER.next_serial();
                // wl_keyboard reports evdev codes; xkb keycodes are evdev + 8.
                k.input::<(), _>(
                    state,
                    (key + 8).into(),
                    if pressed {
                        KeyState::Pressed
                    } else {
                        KeyState::Released
                    },
                    serial,
                    time,
                    |_, _, _| FilterResult::Forward,
                );
            }
        }
    }
}

/// Release frame callbacks for every surface, letting the application draw.
///
/// Clients render on the frame-callback clock: draw once, request a callback,
/// and wait. Nothing here ever ran that clock, so an application would render a
/// single frame and then freeze — accepting input but never able to show the
/// result.
///
/// This is also exactly the lever the design uses for detach: while no frontend
/// is attached the caller simply stops running this, so the application idles
/// like a minimised window instead of drawing frames nobody can see. Its timers,
/// network and background work carry on untouched.
pub fn send_frame_callbacks(state: &SessionHost, time: u32) {
    use smithay::wayland::compositor::{TraversalAction, with_surface_tree_downward};

    for toplevel in &state.toplevels {
        with_surface_tree_downward(
            toplevel.wl_surface(),
            (),
            |_, _, _| TraversalAction::DoChildren(()),
            |_, states, _| {
                let mut attrs = states.cached_state.get::<SurfaceAttributes>();
                for callback in attrs.current().frame_callbacks.drain(..) {
                    callback.done(time);
                }
            },
            |_, _, _| true,
        );
    }
}

/// Snapshot geometry, without the pixels.
///
/// Pixels travel out of band in a per-surface file that both sides map, so a
/// multi-megabyte frame is never serialised into the metadata stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SnapshotMeta {
    pub width: i32,
    pub height: i32,
    pub stride: i32,
    pub format: u32,
    pub len: u64,
    /// Bumped whenever the pixels change, so the frontend knows to redraw.
    pub seq: u64,
}

/// What the session host publishes to the frontend.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PublishedSurface {
    pub title: String,
    pub app_id: String,
    pub meta: Option<SnapshotMeta>,
}
