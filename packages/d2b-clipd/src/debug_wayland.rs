use std::{
    fs::File,
    io::{Read, Write},
    os::fd::AsFd,
    time::{Duration, Instant},
};

use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop, event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{
        wl_callback, wl_compositor, wl_data_device, wl_data_device_manager, wl_data_offer,
        wl_data_source, wl_keyboard, wl_registry, wl_seat, wl_shm, wl_subcompositor, wl_surface,
    },
};

const COPY_HOLD_TIMEOUT: Duration = Duration::from_secs(60);
const PASTE_SELECTION_TIMEOUT: Duration = Duration::from_secs(5);
const PASTE_READ_TIMEOUT: Duration = Duration::from_secs(120);

pub fn run(args: impl IntoIterator<Item = String>) -> Result<(), String> {
    let mut iter = args.into_iter();
    match iter.next().as_deref() {
        Some("wl-copy") => {
            let text = iter
                .next()
                .ok_or_else(|| "usage: d2b-clipd debug wl-copy <text>".to_owned())?;
            if iter.next().is_some() {
                return Err("usage: d2b-clipd debug wl-copy <text>".to_owned());
            }
            run_wl_copy(text)
        }
        Some("wl-paste") => {
            let mime = iter.next().unwrap_or_else(|| "text/plain".to_owned());
            if iter.next().is_some() {
                return Err("usage: d2b-clipd debug wl-paste [mime]".to_owned());
            }
            run_wl_paste(mime)
        }
        Some("--help" | "-h") | None => Err(
            "usage: d2b-clipd debug wl-copy <text> | d2b-clipd debug wl-paste [mime]".to_owned(),
        ),
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
            eprintln!("send {mime_type}");
            let mut file = File::from(fd);
            let _ = file.write_all(&state.token);
            let _ = file.flush();
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
        queue
            .blocking_dispatch(&mut state)
            .map_err(|e| format!("dispatch: {e}"))?;
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
                eprintln!("offer {} {}", proxy.id().protocol_id(), mime_type);
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
        queue
            .blocking_dispatch(&mut state)
            .map_err(|e| format!("dispatch selection: {e}"))?;
    }
    eprintln!("mimes={:?}", state.mimes);
    let offer = state.selected.ok_or_else(|| "no selection".to_owned())?;
    let (read_fd, write_fd) =
        rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).map_err(|e| e.to_string())?;
    offer.receive(mime, write_fd.as_fd());
    conn.flush().map_err(|e| format!("flush receive: {e}"))?;
    drop(write_fd);
    drop(offer);

    let mut file = File::from(read_fd);
    let mut bytes = Vec::new();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = file.read_to_end(&mut bytes).map(|_| bytes);
        let _ = tx.send(result);
    });
    let bytes = rx
        .recv_timeout(PASTE_READ_TIMEOUT)
        .map_err(|_| "timed out reading paste pipe".to_owned())?
        .map_err(|e| format!("read paste pipe: {e}"))?;
    std::io::stdout()
        .write_all(&bytes)
        .map_err(|e| format!("write stdout: {e}"))?;
    Ok(())
}
