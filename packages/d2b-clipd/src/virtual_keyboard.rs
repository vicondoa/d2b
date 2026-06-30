use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::AsFd;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use thiserror::Error;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
    zwp_virtual_keyboard_manager_v1, zwp_virtual_keyboard_v1,
};

const PASTE_KEYMAP: &[u8] = b"xkb_keymap {\n\
xkb_keycodes \"d2b\" {\n\
minimum = 8;\n\
maximum = 11;\n\
<D2B1> = 9;\n\
<D2B2> = 10;\n\
};\n\
xkb_types \"d2b\" { include \"complete\" };\n\
xkb_compatibility \"d2b\" { include \"complete\" };\n\
xkb_symbols \"d2b\" {\n\
key <D2B1> {[Control_L]};\n\
key <D2B2> {[v, V]};\n\
};\n\
};\n\0";

#[derive(Debug, Error)]
pub enum VirtualKeyboardError {
    #[error("Wayland connection failed: {0}")]
    Wayland(String),
    #[error("wl_seat is unavailable")]
    MissingSeat,
    #[error("zwp_virtual_keyboard_manager_v1 is unavailable")]
    MissingVirtualKeyboard,
    #[error("temporary keymap setup failed: {0}")]
    Keymap(String),
}

struct VirtualKeyboardState;

pub fn paste_ctrl_v() -> Result<(), VirtualKeyboardError> {
    let connection = Connection::connect_to_env()
        .map_err(|error| VirtualKeyboardError::Wayland(error.to_string()))?;
    let (globals, mut event_queue) = registry_queue_init::<VirtualKeyboardState>(&connection)
        .map_err(|error| VirtualKeyboardError::Wayland(error.to_string()))?;
    let qh = event_queue.handle();

    let seat = globals
        .bind::<wl_seat::WlSeat, _, _>(&qh, 1..=9, ())
        .map_err(|_| VirtualKeyboardError::MissingSeat)?;
    let manager = globals
        .bind::<zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1, _, _>(&qh, 1..=1, ())
        .map_err(|_| VirtualKeyboardError::MissingVirtualKeyboard)?;
    let keyboard = manager.create_virtual_keyboard(&seat, &qh, ());

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "d2b-clipd-keymap-{}-{nanos}.xkb",
        std::process::id()
    ));
    let mut keymap = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| VirtualKeyboardError::Keymap(error.to_string()))?;
    keymap
        .write_all(PASTE_KEYMAP)
        .and_then(|_| keymap.flush())
        .map_err(|error| VirtualKeyboardError::Keymap(error.to_string()))?;

    keyboard.keymap(
        1,
        keymap.as_fd(),
        PASTE_KEYMAP.len().try_into().unwrap_or(u32::MAX),
    );
    event_queue
        .roundtrip(&mut VirtualKeyboardState)
        .map_err(|error| VirtualKeyboardError::Wayland(error.to_string()))?;

    keyboard.key(0, 1, 1);
    keyboard.modifiers(4, 0, 0, 0);
    connection
        .flush()
        .map_err(|error| VirtualKeyboardError::Wayland(error.to_string()))?;
    std::thread::sleep(Duration::from_millis(10));

    keyboard.key(0, 2, 1);
    connection
        .flush()
        .map_err(|error| VirtualKeyboardError::Wayland(error.to_string()))?;
    std::thread::sleep(Duration::from_millis(6));

    keyboard.key(0, 2, 0);
    connection
        .flush()
        .map_err(|error| VirtualKeyboardError::Wayland(error.to_string()))?;
    std::thread::sleep(Duration::from_millis(6));

    keyboard.modifiers(0, 0, 0, 0);
    keyboard.key(0, 1, 0);
    keyboard.destroy();
    let _ = connection.flush();
    let _ = std::fs::remove_file(path);

    Ok(())
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for VirtualKeyboardState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(VirtualKeyboardState: ignore wl_seat::WlSeat);
delegate_noop!(VirtualKeyboardState: ignore zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1);
delegate_noop!(VirtualKeyboardState: ignore zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_keymap_contains_only_synthetic_control_v_keys() {
        let keymap = std::str::from_utf8(PASTE_KEYMAP).expect("utf8 keymap");
        assert!(keymap.contains("Control_L"));
        assert!(keymap.contains("[v, V]"));
        assert!(!keymap.contains("include \"evdev\""));
    }
}
