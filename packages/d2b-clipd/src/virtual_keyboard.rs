use std::fs::File;
use std::io::Write;
use std::os::fd::AsFd;
use std::time::Duration;

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
maximum = 10;\n\
<D2B1> = 9;\n\
};\n\
xkb_types \"d2b\" { include \"complete\" };\n\
xkb_compatibility \"d2b\" { include \"complete\" };\n\
xkb_symbols \"d2b\" {\n\
key <D2B1> {[v]};\n\
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

    let keymap = create_keymap_memfd()?;

    keyboard.keymap(
        1,
        keymap.as_fd(),
        PASTE_KEYMAP.len().try_into().unwrap_or(u32::MAX),
    );
    event_queue
        .roundtrip(&mut VirtualKeyboardState)
        .map_err(|error| VirtualKeyboardError::Wayland(error.to_string()))?;

    keyboard.modifiers(4, 0, 0, 0);
    event_queue
        .roundtrip(&mut VirtualKeyboardState)
        .map_err(|error| VirtualKeyboardError::Wayland(error.to_string()))?;

    keyboard.key(0, 1, 1);
    event_queue
        .roundtrip(&mut VirtualKeyboardState)
        .map_err(|error| VirtualKeyboardError::Wayland(error.to_string()))?;
    std::thread::sleep(Duration::from_millis(2));

    keyboard.key(0, 1, 0);
    event_queue
        .roundtrip(&mut VirtualKeyboardState)
        .map_err(|error| VirtualKeyboardError::Wayland(error.to_string()))?;
    std::thread::sleep(Duration::from_millis(2));

    keyboard.modifiers(0, 0, 0, 0);
    event_queue
        .roundtrip(&mut VirtualKeyboardState)
        .map_err(|error| VirtualKeyboardError::Wayland(error.to_string()))?;
    keyboard.destroy();
    let _ = connection.flush();

    Ok(())
}

fn create_keymap_memfd() -> Result<File, VirtualKeyboardError> {
    let keymap_fd = rustix::fs::memfd_create("d2b-clipd-keymap", rustix::fs::MemfdFlags::CLOEXEC)
        .map_err(|error| VirtualKeyboardError::Keymap(error.to_string()))?;
    let mut keymap = File::from(keymap_fd);
    keymap
        .write_all(PASTE_KEYMAP)
        .and_then(|_| keymap.flush())
        .map_err(|error| VirtualKeyboardError::Keymap(error.to_string()))?;
    Ok(keymap)
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
        assert!(keymap.contains("[v]"));
        assert!(!keymap.contains("Control_L"));
        assert!(!keymap.contains("D2B2"));
        assert!(!keymap.contains("include \"evdev\""));
    }

    #[test]
    fn keymap_fd_is_anonymous_memfd() {
        use std::os::fd::AsRawFd;

        let keymap = create_keymap_memfd().expect("memfd keymap");
        let link =
            std::fs::read_link(format!("/proc/self/fd/{}", keymap.as_raw_fd())).expect("fd link");

        assert!(
            link.to_string_lossy().contains("memfd:d2b-clipd-keymap"),
            "expected anonymous memfd, got {}",
            link.display()
        );
    }
}
