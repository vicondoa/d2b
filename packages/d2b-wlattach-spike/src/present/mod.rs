//! The disposable window frontend.
//!
//! A plain Wayland client. It holds no durable state: everything it shows is
//! replayed to it by the session host after it connects, and it is rebuilt from
//! scratch on every attach. Killing it must not disturb the application.

use std::collections::HashMap;

use std::os::fd::AsFd;

use wayland_client::protocol::{
    wl_buffer::WlBuffer,
    wl_compositor::WlCompositor,
    wl_keyboard::{self, WlKeyboard},
    wl_pointer::{self, WlPointer},
    wl_registry::{self, WlRegistry},
    wl_seat::{self, WlSeat},
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
    wl_surface::WlSurface,
};

use crate::wire::dto::InputEvent;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{
    xdg_surface::{self, XdgSurface},
    xdg_toplevel::{self, XdgToplevel},
    xdg_wm_base::{self, XdgWmBase},
};

use crate::serve::host::PublishedSurface;

#[derive(Debug, thiserror::Error)]
pub enum FrontendError {
    #[error("could not connect to the host compositor")]
    Connect,
    #[error("the host compositor is missing a required global")]
    MissingGlobal(&'static str),
    #[error("wayland protocol error")]
    Protocol,
    #[error("io error")]
    Io(#[from] std::io::Error),
}

/// One window this frontend is currently showing.
struct Window {
    surface: WlSurface,
    xdg_surface: XdgSurface,
    toplevel: XdgToplevel,
    /// Set once the compositor has configured us and we may attach a buffer.
    configured: bool,
    pending: Option<PublishedSurface>,
    /// The pool mapped over the session host's pixel file, plus the buffer over
    /// it. Both are kept for as long as the geometry is unchanged: rebuilding
    /// them every frame cost a pool creation, a buffer creation and two
    /// compositor round-trips per frame.
    pool: Option<WlShmPool>,
    buffer: Option<WlBuffer>,
    geometry: Option<(i32, i32, i32, u32, u64)>,
}

pub struct Frontend {
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    wm_base: Option<XdgWmBase>,
    windows: HashMap<u64, Window>,
    /// Set when the compositor asks a window to close. Advisory: forwarded to
    /// the session host, which forwards it to the application, which decides.
    pub close_requested: Vec<u64>,
    /// Input observed from the host compositor, drained and forwarded upward.
    pub input: Vec<InputEvent>,
    /// Last pointer position forwarded. Our own buffer commits make the
    /// compositor re-emit motion at an unchanged position; forwarding those
    /// floods the application with hundreds of no-op motions a second, which
    /// keeps resetting hover and tooltip timers.
    last_pointer: Option<(f64, f64)>,
    seat: Option<WlSeat>,
    /// `wl_seat.capabilities` may be sent more than once; only take the pointer
    /// and keyboard the first time, or we leak a seat object per event.
    has_pointer: bool,
    has_keyboard: bool,
    /// Where the session host writes per-surface pixel files.
    px_dir: std::path::PathBuf,
    pub running: bool,
}

impl Default for Frontend {
    fn default() -> Self {
        Self {
            compositor: None,
            shm: None,
            wm_base: None,
            windows: HashMap::new(),
            close_requested: Vec::new(),
            input: Vec::new(),
            last_pointer: None,
            seat: None,
            has_pointer: false,
            has_keyboard: false,
            px_dir: std::path::PathBuf::new(),
            running: true,
        }
    }
}

impl Frontend {
    /// Bind the globals we need. Fails closed if any is absent.
    pub fn bind(
        conn: &Connection,
        px_dir: std::path::PathBuf,
    ) -> Result<(Self, wayland_client::EventQueue<Self>), FrontendError> {
        let display = conn.display();
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        display.get_registry(&qh, ());

        let mut state = Frontend {
            px_dir,
            ..Frontend::default()
        };
        queue
            .roundtrip(&mut state)
            .map_err(|_| FrontendError::Protocol)?;

        if state.compositor.is_none() {
            return Err(FrontendError::MissingGlobal("wl_compositor"));
        }
        if state.shm.is_none() {
            return Err(FrontendError::MissingGlobal("wl_shm"));
        }
        if state.wm_base.is_none() {
            return Err(FrontendError::MissingGlobal("xdg_wm_base"));
        }
        Ok((state, queue))
    }

    /// Create or update the window for one shadow surface.
    ///
    /// The initial commit deliberately carries **no** buffer: an `xdg_surface`
    /// that has not yet been configured must not have one attached, and we must
    /// wait for the compositor's *fresh* configure rather than replaying any
    /// serial from a previous generation.
    /// Create or update the window for one published surface.
    ///
    /// The initial commit deliberately carries **no** buffer: an `xdg_surface`
    /// that has not yet been configured must not have one attached, and we must
    /// wait for the compositor's *fresh* configure rather than replaying any
    /// serial from a previous generation.
    pub fn upsert(
        &mut self,
        key: u64,
        shadow: &PublishedSurface,
        qh: &QueueHandle<Self>,
    ) -> Result<(), FrontendError> {
        if !self.windows.contains_key(&key) {
            let compositor = self
                .compositor
                .as_ref()
                .ok_or(FrontendError::MissingGlobal("wl_compositor"))?;
            let wm_base = self
                .wm_base
                .as_ref()
                .ok_or(FrontendError::MissingGlobal("xdg_wm_base"))?;

            let surface = compositor.create_surface(qh, ());
            let xdg_surface = wm_base.get_xdg_surface(&surface, qh, key);
            let toplevel = xdg_surface.get_toplevel(qh, key);

            if !shadow.title.is_empty() {
                toplevel.set_title(shadow.title.clone());
            }
            if !shadow.app_id.is_empty() {
                // App-id is preserved exactly, never suffixed: compositor window
                // rules match on it.
                toplevel.set_app_id(shadow.app_id.clone());
            }
            surface.commit();

            self.windows.insert(
                key,
                Window {
                    surface,
                    xdg_surface,
                    toplevel,
                    configured: false,
                    pending: Some(shadow.clone()),
                    pool: None,
                    buffer: None,
                    geometry: None,
                },
            );
            return Ok(());
        }

        if let Some(w) = self.windows.get_mut(&key) {
            w.pending = Some(shadow.clone());
        }
        self.flush_pending(key, qh)
    }

    /// Show the retained content once the compositor has configured us.
    ///
    /// The pool and buffer are reused whenever the geometry is unchanged. The
    /// session host writes new pixels into the same file in place, so a redraw
    /// is just damage plus commit — no allocation and no round-trip.
    fn flush_pending(&mut self, key: u64, qh: &QueueHandle<Self>) -> Result<(), FrontendError> {
        let px_dir = self.px_dir.clone();
        let shm = self
            .shm
            .as_ref()
            .ok_or(FrontendError::MissingGlobal("wl_shm"))?
            .clone();

        let Some(w) = self.windows.get_mut(&key) else {
            return Ok(());
        };
        if !w.configured {
            return Ok(());
        }
        let Some(shadow) = w.pending.take() else {
            return Ok(());
        };
        let Some(meta) = shadow.meta else {
            // No content: either nothing retained yet, or the application
            // committed a null buffer. Unmap rather than leave a stale frame.
            if let Some(b) = w.buffer.take() {
                w.surface.attach(None, 0, 0);
                w.surface.commit();
                b.destroy();
            }
            if let Some(p) = w.pool.take() {
                p.destroy();
            }
            w.geometry = None;
            return Ok(());
        };

        let geom = (meta.width, meta.height, meta.stride, meta.format, meta.len);
        if w.geometry != Some(geom) {
            // Geometry changed: rebuild the pool and buffer over the file.
            if let Some(b) = w.buffer.take() {
                b.destroy();
            }
            if let Some(p) = w.pool.take() {
                p.destroy();
            }
            // Opened read-write: the compositor maps the pool MAP_SHARED and a
            // read-only descriptor fails the mmap outright.
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(px_dir.join(format!("px-{key}.raw")))?;
            let len = meta.len.max(1) as i32;
            let pool = shm.create_pool(file.as_fd(), len, qh, ());
            let format = match meta.format {
                1 => wl_shm::Format::Xrgb8888,
                other => wl_shm::Format::try_from(other).unwrap_or(wl_shm::Format::Argb8888),
            };
            let buffer = pool.create_buffer(
                0,
                meta.width.max(1),
                meta.height.max(1),
                meta.stride.max(1),
                format,
                qh,
                (),
            );
            w.pool = Some(pool);
            w.buffer = Some(buffer);
            w.geometry = Some(geom);
        }

        // Re-attach every frame, even when reusing the same buffer. The session
        // host rewrites the pixels in place, and a compositor is entitled to
        // keep its uploaded texture until a buffer is attached again — without
        // this the window shows its first frame and never updates.
        if let Some(b) = w.buffer.as_ref() {
            w.surface.attach(Some(b), 0, 0);
        }
        w.surface
            .damage_buffer(0, 0, meta.width.max(1), meta.height.max(1));
        w.surface.commit();
        Ok(())
    }

    pub fn window_count(&self) -> usize {
        self.windows.len()
    }
}

impl Dispatch<WlRegistry, ()> for Frontend {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor =
                        Some(registry.bind::<WlCompositor, _, _>(name, version.min(4), qh, ()));
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind::<WlShm, _, _>(name, 1, qh, ()));
                }
                "xdg_wm_base" => {
                    state.wm_base =
                        Some(registry.bind::<XdgWmBase, _, _>(name, version.min(4), qh, ()));
                }
                "wl_seat" => {
                    state.seat = Some(registry.bind::<WlSeat, _, _>(name, version.min(5), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<XdgWmBase, ()> for Frontend {
    fn event(
        _: &mut Self,
        base: &XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            base.pong(serial);
        }
    }
}

impl Dispatch<XdgSurface, u64> for Frontend {
    fn event(
        state: &mut Self,
        surface: &XdgSurface,
        event: xdg_surface::Event,
        key: &u64,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            // Acknowledge the *new* serial from this connection. Serials from a
            // previous generation are meaningless here and are never replayed.
            surface.ack_configure(serial);
            if let Some(w) = state.windows.get_mut(key) {
                w.configured = true;
            }
            let _ = state.flush_pending(*key, qh);
        }
    }
}

impl Dispatch<XdgToplevel, u64> for Frontend {
    fn event(
        state: &mut Self,
        _: &XdgToplevel,
        event: xdg_toplevel::Event,
        key: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event {
            // Purely advisory: report it upward and change nothing locally. The
            // application may ignore it or show a save prompt, and if we marked
            // the window closed here we would freeze its updates for good.
            state.close_requested.push(*key);
        }
    }
}

impl Dispatch<WlSeat, ()> for Frontend {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: wayland_client::WEnum::Value(c),
        } = event
        {
            if c.contains(wl_seat::Capability::Pointer) && !state.has_pointer {
                seat.get_pointer(qh, ());
                state.has_pointer = true;
            }
            if c.contains(wl_seat::Capability::Keyboard) && !state.has_keyboard {
                seat.get_keyboard(qh, ());
                state.has_keyboard = true;
            }
        }
    }
}

impl Dispatch<WlPointer, ()> for Frontend {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Serials are deliberately dropped: they belong to this compositor
        // connection and must never cross into the application.s.
        match event {
            wl_pointer::Event::Enter {
                surface_x,
                surface_y,
                ..
            } => {
                state.last_pointer = Some((surface_x, surface_y));
                state.input.push(InputEvent::PointerEnter {
                    x: surface_x,
                    y: surface_y,
                });
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => state.input.push(InputEvent::PointerMotion {
                x: surface_x,
                y: surface_y,
            }),
            wl_pointer::Event::Button {
                button, state: st, ..
            } => state.input.push(InputEvent::PointerButton {
                button,
                pressed: matches!(
                    st,
                    wayland_client::WEnum::Value(wl_pointer::ButtonState::Pressed)
                ),
            }),
            wl_pointer::Event::Axis { axis, value, .. } => {
                let (h, v) = match axis {
                    wayland_client::WEnum::Value(wl_pointer::Axis::HorizontalScroll) => {
                        (value, 0.0)
                    }
                    _ => (0.0, value),
                };
                state.input.push(InputEvent::PointerAxis {
                    horizontal: h,
                    vertical: v,
                });
            }
            wl_pointer::Event::Leave { .. } => {
                state.last_pointer = None;
                state.input.push(InputEvent::PointerLeave);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlKeyboard, ()> for Frontend {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { .. } => state.input.push(InputEvent::KeyboardEnter),
            wl_keyboard::Event::Leave { .. } => state.input.push(InputEvent::KeyboardLeave),
            wl_keyboard::Event::Key { key, state: st, .. } => {
                state.input.push(InputEvent::KeyboardKey {
                    key,
                    pressed: matches!(
                        st,
                        wayland_client::WEnum::Value(wl_keyboard::KeyState::Pressed)
                    ),
                })
            }
            _ => {}
        }
    }
}

delegate_noop!(Frontend: ignore WlCompositor);
delegate_noop!(Frontend: ignore WlSurface);
delegate_noop!(Frontend: ignore WlShm);
delegate_noop!(Frontend: ignore WlShmPool);
delegate_noop!(Frontend: ignore WlBuffer);

impl Frontend {
    /// Tear down every window without signalling anything to the application.
    ///
    /// This is what makes detach different from close: the application is never
    /// told, and simply stops receiving frame callbacks.
    pub fn teardown(&mut self) {
        for (_, w) in self.windows.drain() {
            if let Some(b) = w.buffer {
                b.destroy();
            }
            w.toplevel.destroy();
            w.xdg_surface.destroy();
            w.surface.destroy();
        }
        self.running = false;
    }
}

impl Frontend {
    /// Destroy any window that is no longer in the published surface set.
    ///
    /// Without this, an application that closes one window of several leaves a
    /// stale frame on screen forever.
    pub fn reconcile(&mut self, live: &[u64]) {
        let gone: Vec<u64> = self
            .windows
            .keys()
            .copied()
            .filter(|k| !live.contains(k))
            .collect();
        for key in gone {
            if let Some(w) = self.windows.remove(&key) {
                if let Some(b) = w.buffer {
                    b.destroy();
                }
                w.toplevel.destroy();
                w.xdg_surface.destroy();
                w.surface.destroy();
            }
        }
    }
}
