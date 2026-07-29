//! The disposable window frontend.
//!
//! A plain Wayland client. It holds no durable state: everything it shows is
//! replayed to it by the session host after it connects, and it is rebuilt from
//! scratch on every attach. Killing it must not disturb the application.

use std::collections::HashMap;
use std::io::Write;
use std::os::fd::{AsFd, OwnedFd};

use wayland_client::protocol::{
    wl_buffer::WlBuffer,
    wl_compositor::WlCompositor,
    wl_registry::{self, WlRegistry},
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
    wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols::xdg::shell::client::{
    xdg_surface::{self, XdgSurface},
    xdg_toplevel::{self, XdgToplevel},
    xdg_wm_base::{self, XdgWmBase},
};

use crate::serve::host::ShadowSurface;

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
    pending: Option<ShadowSurface>,
    closed: bool,
    /// The buffer currently attached. Destroyed when superseded, otherwise a
    /// continuously-redrawing application leaks one wl_buffer and one pool per
    /// frame for the frontend.s whole lifetime.
    attached: Option<WlBuffer>,
}

pub struct Frontend {
    compositor: Option<WlCompositor>,
    shm: Option<WlShm>,
    wm_base: Option<XdgWmBase>,
    windows: HashMap<u32, Window>,
    /// Set when the compositor asks a window to close. Advisory: forwarded to
    /// the session host, which forwards it to the application, which decides.
    pub close_requested: Vec<u32>,
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
            running: true,
        }
    }
}

impl Frontend {
    /// Bind the globals we need. Fails closed if any is absent.
    pub fn bind(
        conn: &Connection,
    ) -> Result<(Self, wayland_client::EventQueue<Self>), FrontendError> {
        let display = conn.display();
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        display.get_registry(&qh, ());

        let mut state = Frontend::default();
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
    pub fn upsert(
        &mut self,
        key: u32,
        shadow: &ShadowSurface,
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
            // Commit with no buffer, then wait for the configure.
            surface.commit();

            self.windows.insert(
                key,
                Window {
                    surface,
                    xdg_surface,
                    toplevel,
                    configured: false,
                    pending: Some(shadow.clone()),
                    closed: false,
                    attached: None,
                },
            );
            return Ok(());
        }

        if let Some(w) = self.windows.get_mut(&key) {
            w.pending = Some(shadow.clone());
        }
        self.flush_pending(key, qh)
    }

    /// Attach the retained content once the compositor has configured us.
    fn flush_pending(&mut self, key: u32, qh: &QueueHandle<Self>) -> Result<(), FrontendError> {
        let shm = self
            .shm
            .as_ref()
            .ok_or(FrontendError::MissingGlobal("wl_shm"))?
            .clone();

        let Some(w) = self.windows.get_mut(&key) else {
            return Ok(());
        };
        if !w.configured || w.closed {
            return Ok(());
        }
        let Some(shadow) = w.pending.take() else {
            return Ok(());
        };
        let Some(snap) = shadow.snapshot.as_ref() else {
            // The surface has no content: either nothing retained yet, or the
            // application committed a null buffer. Unmap rather than leave a
            // stale frame on screen. It remaps on the next real frame.
            if w.attached.take().is_some() {
                w.surface.attach(None, 0, 0);
                w.surface.commit();
            }
            return Ok(());
        };

        let buffer = make_shm_buffer(&shm, snap, qh)?;
        w.surface.attach(Some(&buffer), 0, 0);
        if let Some(old) = w.attached.replace(buffer.clone()) {
            old.destroy();
        }
        w.surface
            .damage_buffer(0, 0, snap.width.max(1), snap.height.max(1));
        w.surface.commit();
        Ok(())
    }

    pub fn window_count(&self) -> usize {
        self.windows.len()
    }
}

/// Build a `wl_buffer` from a retained snapshot using an anonymous file.
fn make_shm_buffer(
    shm: &WlShm,
    snap: &crate::serve::host::Snapshot,
    qh: &QueueHandle<Frontend>,
) -> Result<WlBuffer, FrontendError> {
    let mut file = tempfile_anon()?;
    file.write_all(&snap.pixels)?;
    file.flush()?;

    let len = snap.pixels.len() as i32;
    let pool = shm.create_pool(file.as_fd(), len.max(1), qh, ());
    let format = match snap.format {
        0 => wl_shm::Format::Argb8888,
        1 => wl_shm::Format::Xrgb8888,
        other => wl_shm::Format::try_from(other).unwrap_or(wl_shm::Format::Argb8888),
    };
    let buffer = pool.create_buffer(
        0,
        snap.width.max(1),
        snap.height.max(1),
        snap.stride.max(1),
        format,
        qh,
        (),
    );
    pool.destroy();
    Ok(buffer)
}

/// An unlinked temporary file to back the shm pool.
fn tempfile_anon() -> std::io::Result<std::fs::File> {
    let fd: OwnedFd = rustix::fs::memfd_create("wlattach-shm", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(std::io::Error::from)?;
    Ok(std::fs::File::from(fd))
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

impl Dispatch<XdgSurface, u32> for Frontend {
    fn event(
        state: &mut Self,
        surface: &XdgSurface,
        event: xdg_surface::Event,
        key: &u32,
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

impl Dispatch<XdgToplevel, u32> for Frontend {
    fn event(
        state: &mut Self,
        _: &XdgToplevel,
        event: xdg_toplevel::Event,
        key: &u32,
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
            if let Some(b) = w.attached {
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
    pub fn reconcile(&mut self, live: &[u32]) {
        let gone: Vec<u32> = self
            .windows
            .keys()
            .copied()
            .filter(|k| !live.contains(k))
            .collect();
        for key in gone {
            if let Some(w) = self.windows.remove(&key) {
                if let Some(b) = w.attached {
                    b.destroy();
                }
                w.toplevel.destroy();
                w.xdg_surface.destroy();
                w.surface.destroy();
            }
        }
    }
}
