//! The user's real terminal.
//!
//! Opens `/dev/tty` rather than stdin/stdout so the passthrough keeps working
//! when either standard stream is redirected, puts it into raw mode, and
//! guarantees restoration.
//!
//! Restoration is belt and braces: a `Drop` impl covers normal return, `?`
//! propagation and unwinding panics, and a panic hook covers `panic = "abort"`
//! builds where `Drop` never runs. Leaving a terminal in raw mode is one of the
//! most user-hostile failures this program could have.

use std::fs::File;
use std::io;
use std::os::fd::{AsFd, AsRawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::sync::{Mutex, OnceLock};

use nix::sys::termios::{self, SetArg};
use tokio::io::unix::AsyncFd;

/// Terminal dimensions in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TtySize {
    pub cols: u16,
    pub rows: u16,
}

impl TtySize {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }

    /// Convert to the kernel's `winsize`. Pixel dimensions are left at zero;
    /// nothing in this prototype reports pixel geometry.
    pub fn to_winsize(self) -> libc::winsize {
        libc::winsize {
            ws_row: self.rows,
            ws_col: self.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }
    }
}

impl std::fmt::Display for TtySize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}x{}", self.cols, self.rows)
    }
}

/// Saved original terminal settings, for the panic hook.
static ORIGINAL: OnceLock<Mutex<Option<libc::termios>>> = OnceLock::new();

fn original_slot() -> &'static Mutex<Option<libc::termios>> {
    ORIGINAL.get_or_init(|| Mutex::new(None))
}

/// Restore the saved terminal settings by reopening `/dev/tty`.
///
/// Deliberately reopens rather than capturing a descriptor, so it stays valid
/// no matter what has happened to the original handle.
fn restore_from_saved() {
    let Some(saved) = original_slot().lock().ok().and_then(|g| *g) else {
        return;
    };

    let Ok(file) = File::options().read(true).write(true).open("/dev/tty") else {
        return;
    };

    let termios = termios::Termios::from(saved);
    let _ = termios::tcsetattr(file.as_fd(), SetArg::TCSANOW, &termios);
}

/// Install the panic hook that restores cooked mode.
///
/// Idempotent; only the first call takes effect.
fn install_panic_hook() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_from_saved();
            previous(info);
        }));
    });
}

/// `/dev/tty` in raw mode, registered with the reactor.
pub struct RawTty {
    file: AsyncFd<File>,
    original: libc::termios,
}

impl RawTty {
    /// Open `/dev/tty` and switch it to raw mode.
    pub fn open() -> anyhow::Result<Self> {
        let file = File::options()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/tty")?;

        let original = termios::tcgetattr(file.as_fd())?;
        let mut raw = original.clone();
        termios::cfmakeraw(&mut raw);
        termios::tcsetattr(file.as_fd(), SetArg::TCSANOW, &raw)?;

        let original: libc::termios = original.into();

        if let Ok(mut slot) = original_slot().lock() {
            *slot = Some(original);
        }
        install_panic_hook();

        Ok(Self {
            file: AsyncFd::new(file)?,
            original,
        })
    }

    /// Current terminal size, via `TIOCGWINSZ`.
    pub fn size(&self) -> anyhow::Result<TtySize> {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        // SAFETY: `ws` is a live, correctly typed, uniquely borrowed winsize,
        // and the fd is owned by `self` and open for the duration of the call.
        let rc = unsafe {
            libc::ioctl(
                self.file.get_ref().as_raw_fd(),
                libc::TIOCGWINSZ,
                &raw mut ws,
            )
        };

        if rc < 0 {
            return Err(io::Error::last_os_error().into());
        }

        Ok(TtySize::new(ws.ws_col, ws.ws_row))
    }

    /// Read available input. Resolves only when the terminal is readable.
    pub async fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            let mut guard = self.file.readable().await?;
            match guard.try_io(|inner| {
                let fd = inner.get_ref().as_raw_fd();
                // SAFETY: `buf` is a live uniquely borrowed slice and `fd` is
                // owned by `self`.
                let n =
                    unsafe { libc::read(fd, buf.as_mut_ptr().cast::<libc::c_void>(), buf.len()) };
                if n < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(n as usize)
                }
            }) {
                Ok(result) => return result,
                // Spurious readiness; wait again.
                Err(_would_block) => continue,
            }
        }
    }

    /// Write output. Resolves once at least one byte has been accepted.
    pub async fn write(&self, buf: &[u8]) -> io::Result<usize> {
        loop {
            let mut guard = self.file.writable().await?;
            match guard.try_io(|inner| {
                let fd = inner.get_ref().as_raw_fd();
                // SAFETY: `buf` is a live borrowed slice and `fd` is owned by
                // `self`.
                let n = unsafe { libc::write(fd, buf.as_ptr().cast::<libc::c_void>(), buf.len()) };
                if n < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(n as usize)
                }
            }) {
                Ok(result) => return result,
                Err(_would_block) => continue,
            }
        }
    }
}

impl Drop for RawTty {
    fn drop(&mut self) {
        let termios = termios::Termios::from(self.original);
        let _ = termios::tcsetattr(self.file.get_ref().as_fd(), SetArg::TCSANOW, &termios);
        if let Ok(mut slot) = original_slot().lock() {
            *slot = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TtySize;

    #[test]
    fn size_renders_as_colsxrows() {
        assert_eq!(TtySize::new(120, 40).to_string(), "120x40");
    }

    #[test]
    fn winsize_conversion_puts_rows_and_cols_in_the_right_fields() {
        // Transposing these is a classic bug; the struct order is rows first
        // while the display convention is cols first.
        let ws = TtySize::new(120, 40).to_winsize();
        assert_eq!(ws.ws_col, 120);
        assert_eq!(ws.ws_row, 40);
        assert_eq!(ws.ws_xpixel, 0);
        assert_eq!(ws.ws_ypixel, 0);
    }
}
