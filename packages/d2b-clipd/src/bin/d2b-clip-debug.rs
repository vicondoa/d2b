use std::{
    io::Write,
    os::fd::AsFd,
    time::{Duration, Instant},
};

use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, delegate_noop, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_callback, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
        wl_data_source, wl_keyboard, wl_registry, wl_seat, wl_shm, wl_subcompositor, wl_surface,
    },
};

const COPY_HOLD_TIMEOUT: Duration = Duration::from_secs(60);
const PASTE_SELECTION_TIMEOUT: Duration = Duration::from_secs(5);
const PASTE_READ_TIMEOUT: Duration = Duration::from_secs(120);

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    if let Err(error) = run(std::env::args().skip(1)) {
        eprintln!("d2b-clip-debug: {error}");
        std::process::exit(2);
    }
}

fn run(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let mut iter = args.into_iter();
    match iter.next().as_deref() {
        Some("wl-copy") => {
            let text = iter
                .next()
                .ok_or_else(|| "usage: d2b-clip-debug wl-copy <text>".to_owned())?;
            if iter.next().is_some() {
                return Err("usage: d2b-clip-debug wl-copy <text>".to_owned());
            }
            run_wl_copy(text)
        }
        Some("wl-paste") => {
            let mime = iter.next().unwrap_or_else(|| "text/plain".to_owned());
            if iter.next().is_some() {
                return Err("usage: d2b-clip-debug wl-paste [mime]".to_owned());
            }
            run_wl_paste(mime)
        }
        Some("--help" | "-h") | None => {
            Err("usage: d2b-clip-debug wl-copy <text> | d2b-clip-debug wl-paste [mime]".to_owned())
        }
        Some(other) => Err(format!("unknown debug command: {other}")),
    }
}

struct CopyState {
    token: Vec<u8>,
    _manager: wl_data_device_manager::WlDataDeviceManager,
    _seat: wl_seat::WlSeat,
    _device: wl_data_device::WlDataDevice,
    _source: wl_data_source::WlDataSource,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for CopyState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_data_source::WlDataSource, ()> for CopyState {
    fn event(
        state: &mut Self,
        _proxy: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        if let wl_data_source::Event::Send { mime_type, fd } = event {
            eprintln!("send {mime_type:?}");
            let _ = write_all_fd_debug(&fd, &state.token, Instant::now() + Duration::from_secs(5));
        }
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for CopyState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_data_device::WlDataDevice,
        _event: wl_data_device::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }

    event_created_child!(CopyState, wl_data_device::WlDataDevice, [
        0 => (wl_data_offer::WlDataOffer, ()),
    ]);
}

delegate_noop!(CopyState: ignore wl_data_offer::WlDataOffer);
delegate_noop!(CopyState: ignore wl_seat::WlSeat);
delegate_noop!(CopyState: ignore wl_callback::WlCallback);
delegate_noop!(CopyState: ignore wl_compositor::WlCompositor);
delegate_noop!(CopyState: ignore wl_subcompositor::WlSubcompositor);
delegate_noop!(CopyState: ignore wl_surface::WlSurface);
delegate_noop!(CopyState: ignore wl_shm::WlShm);
delegate_noop!(CopyState: ignore wl_keyboard::WlKeyboard);
delegate_noop!(CopyState: wl_data_device_manager::WlDataDeviceManager);

fn run_wl_copy(text: String) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("connect wayland: {e}"))?;
    let (globals, mut queue) =
        registry_queue_init::<CopyState>(&conn).map_err(|e| format!("registry init: {e}"))?;
    let qh = queue.handle();
    let manager = globals
        .bind::<wl_data_device_manager::WlDataDeviceManager, _, _>(&qh, 1..=3, ())
        .map_err(|e| format!("bind data manager: {e}"))?;
    let seat = globals
        .bind::<wl_seat::WlSeat, _, _>(&qh, 1..=9, ())
        .map_err(|e| format!("bind seat: {e}"))?;
    let device = manager.get_data_device(&seat, &qh, ());
    let source = manager.create_data_source(&qh, ());
    source.offer("text/plain".to_owned());
    source.offer("text/plain;charset=utf-8".to_owned());
    device.set_selection(Some(&source), 0);
    conn.flush()
        .map_err(|e| format!("flush set_selection: {e}"))?;

    let mut state = CopyState {
        token: text.into_bytes(),
        _manager: manager,
        _seat: seat,
        _device: device,
        _source: source,
    };
    let deadline = Instant::now() + COPY_HOLD_TIMEOUT;
    while Instant::now() < deadline {
        dispatch_once_until(&mut queue, &mut state, deadline)?;
    }
    Ok(())
}

#[derive(Default)]
struct PasteState {
    selected: Option<wl_data_offer::WlDataOffer>,
    mimes: Vec<String>,
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for PasteState {
    fn event(
        state: &mut Self,
        _proxy: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                eprintln!("data_offer {}", id.id().protocol_id());
            }
            wl_data_device::Event::Selection { id } => {
                eprintln!(
                    "selection {:?}",
                    id.as_ref().map(|offer| offer.id().protocol_id())
                );
                state.selected = id;
            }
            _ => {}
        }
    }

    event_created_child!(PasteState, wl_data_device::WlDataDevice, [
        0 => (wl_data_offer::WlDataOffer, ()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, ()> for PasteState {
    fn event(
        state: &mut Self,
        proxy: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                eprintln!("offer {} {mime_type:?}", proxy.id().protocol_id());
                state.mimes.push(mime_type);
            }
            wl_data_offer::Event::SourceActions { .. } | wl_data_offer::Event::Action { .. } => {}
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for PasteState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(PasteState: ignore wl_seat::WlSeat);
delegate_noop!(PasteState: ignore wl_callback::WlCallback);
delegate_noop!(PasteState: ignore wl_compositor::WlCompositor);
delegate_noop!(PasteState: ignore wl_subcompositor::WlSubcompositor);
delegate_noop!(PasteState: ignore wl_surface::WlSurface);
delegate_noop!(PasteState: ignore wl_shm::WlShm);
delegate_noop!(PasteState: ignore wl_keyboard::WlKeyboard);
delegate_noop!(PasteState: wl_data_device_manager::WlDataDeviceManager);

fn run_wl_paste(mime: String) -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("connect wayland: {e}"))?;
    let (globals, mut queue) =
        registry_queue_init::<PasteState>(&conn).map_err(|e| format!("registry init: {e}"))?;
    let qh = queue.handle();
    let manager = globals
        .bind::<wl_data_device_manager::WlDataDeviceManager, _, _>(&qh, 1..=3, ())
        .map_err(|e| format!("bind data manager: {e}"))?;
    let seat = globals
        .bind::<wl_seat::WlSeat, _, _>(&qh, 1..=9, ())
        .map_err(|e| format!("bind seat: {e}"))?;
    let _device = manager.get_data_device(&seat, &qh, ());
    let mut state = PasteState::default();
    let deadline = Instant::now() + PASTE_SELECTION_TIMEOUT;
    while state.selected.is_none() && Instant::now() < deadline {
        dispatch_once_until(&mut queue, &mut state, deadline)?;
    }
    eprintln!("mimes={:?}", state.mimes);
    let offer = state.selected.ok_or_else(|| "no selection".to_owned())?;
    let (read_fd, write_fd) =
        rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).map_err(|e| e.to_string())?;
    offer.receive(mime, write_fd.as_fd());
    conn.flush().map_err(|e| format!("flush receive: {e}"))?;
    drop(write_fd);
    drop(offer);

    let bytes = read_fd_to_vec_debug(read_fd, Instant::now() + PASTE_READ_TIMEOUT)?;
    std::io::stdout()
        .write_all(&bytes)
        .map_err(|e| format!("write stdout: {e}"))?;
    Ok(())
}

fn dispatch_once_until<State>(
    queue: &mut EventQueue<State>,
    state: &mut State,
    deadline: Instant,
) -> Result<usize, String> {
    let dispatched = queue
        .dispatch_pending(state)
        .map_err(|e| format!("dispatch pending: {e}"))?;
    if dispatched > 0 {
        return Ok(dispatched);
    }
    queue.flush().map_err(|e| format!("flush wayland: {e}"))?;
    let Some(guard) = queue.prepare_read() else {
        return Ok(0);
    };
    let now = Instant::now();
    if now >= deadline {
        drop(guard);
        return Err("timed out waiting for Wayland event".to_owned());
    }
    let timeout = deadline
        .saturating_duration_since(now)
        .as_millis()
        .min(i32::MAX as u128) as i32;
    let mut poll_fd = [rustix::event::PollFd::from_borrowed_fd(
        guard.connection_fd(),
        rustix::event::PollFlags::IN
            | rustix::event::PollFlags::ERR
            | rustix::event::PollFlags::HUP,
    )];
    match rustix::event::poll(&mut poll_fd, timeout) {
        Ok(0) => {
            drop(guard);
            Err("timed out waiting for Wayland event".to_owned())
        }
        Ok(_)
            if poll_fd[0]
                .revents()
                .intersects(rustix::event::PollFlags::ERR | rustix::event::PollFlags::HUP) =>
        {
            drop(guard);
            Err("Wayland connection closed while waiting for event".to_owned())
        }
        Ok(_) => {
            guard.read().map_err(|e| format!("read wayland: {e}"))?;
            queue
                .dispatch_pending(state)
                .map_err(|e| format!("dispatch pending: {e}"))
        }
        Err(rustix::io::Errno::INTR) => Ok(0),
        Err(error) => {
            drop(guard);
            Err(format!("poll Wayland socket: {error}"))
        }
    }
}

fn read_fd_to_vec_debug(fd: std::os::fd::OwnedFd, deadline: Instant) -> Result<Vec<u8>, String> {
    rustix::io::ioctl_fionbio(fd.as_fd(), true).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    loop {
        let mut buf = [0_u8; 4096];
        match rustix::io::read(&fd, &mut buf) {
            Ok(0) => return Ok(out),
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(rustix::io::Errno::INTR) => {}
            Err(rustix::io::Errno::AGAIN) => {
                wait_fd(&fd, rustix::event::PollFlags::IN, deadline)?;
            }
            Err(error) => return Err(format!("read paste pipe: {error}")),
        }
    }
}

fn write_all_fd_debug(
    fd: &std::os::fd::OwnedFd,
    mut data: &[u8],
    deadline: Instant,
) -> Result<(), String> {
    rustix::io::ioctl_fionbio(fd.as_fd(), true).map_err(|e| e.to_string())?;
    while !data.is_empty() {
        match rustix::io::write(fd, data) {
            Ok(0) => return Err("write returned zero".to_owned()),
            Ok(n) => data = &data[n..],
            Err(rustix::io::Errno::INTR) => {}
            Err(rustix::io::Errno::AGAIN) => {
                wait_fd(fd, rustix::event::PollFlags::OUT, deadline)?;
            }
            Err(error) => return Err(format!("write selection fd: {error}")),
        }
    }
    Ok(())
}

fn wait_fd(
    fd: &std::os::fd::OwnedFd,
    interest: rustix::event::PollFlags,
    deadline: Instant,
) -> Result<(), String> {
    let now = Instant::now();
    if now >= deadline {
        return Err("debug Wayland transfer timed out".to_owned());
    }
    let timeout = deadline
        .saturating_duration_since(now)
        .as_millis()
        .min(i32::MAX as u128) as i32;
    let mut poll_fd = [rustix::event::PollFd::new(
        fd,
        interest | rustix::event::PollFlags::ERR | rustix::event::PollFlags::HUP,
    )];
    match rustix::event::poll(&mut poll_fd, timeout) {
        Ok(0) => Err("debug Wayland transfer timed out".to_owned()),
        Ok(_) => {
            let revents = poll_fd[0].revents();
            if revents.contains(rustix::event::PollFlags::ERR) {
                return Err("debug Wayland transfer fd errored".to_owned());
            }
            if revents.intersects(interest) {
                return Ok(());
            }
            if revents.contains(rustix::event::PollFlags::HUP) {
                if interest.contains(rustix::event::PollFlags::IN) {
                    return Ok(());
                }
                return Err("debug Wayland transfer fd closed".to_owned());
            }
            Ok(())
        }
        Err(rustix::io::Errno::INTR) => Ok(()),
        Err(error) => Err(format!("poll debug transfer fd: {error}")),
    }
}
