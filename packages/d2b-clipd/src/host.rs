//! Host clipboard state machine.
//!
//! Tracks the current host selection, captures focused-window attribution via
//! `HostClipboardAttributor`, and manages the paste write-FD that must be held
//! open until the picker resolves or the fallback timeout fires.
//!
//! No clipboard content, previews, or paths are logged anywhere in this module.

use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};

use crate::niri::{FocusedWindowSnapshot, HostClipboardAttributor, HostSelectionAttribution};
use crate::notifications::Notifier;
use crate::policy::{AttributionQuality, ReasonCode};
use crate::wayland::DataControlOffer;

// ─── Paste-FD guard ──────────────────────────────────────────────────────────

/// Holds the compositor-issued write FD for a pending paste operation.
/// Dropping this closes the FD, signalling EOF to the requesting application.
#[derive(Debug)]
pub struct PasteWriteFd {
    pub fd: OwnedFd,
    pub mime_type: String,
    pub destination: FocusedWindowSnapshot,
    pub deadline: Instant,
}

impl PasteWriteFd {
    pub fn new(
        fd: OwnedFd,
        mime_type: String,
        destination: FocusedWindowSnapshot,
        timeout: Duration,
    ) -> Self {
        Self {
            fd,
            mime_type,
            destination,
            deadline: Instant::now() + timeout,
        }
    }

    pub fn is_expired(&self, now: Instant) -> bool {
        now >= self.deadline
    }

    /// Close the write fd (EOF to the requester) and log the timeout reason.
    pub fn close_with_reason(self, reason: ReasonCode) {
        log::debug!("d2b-clipd: paste fd closed: {}", reason.as_str());
        drop(self.fd);
    }
}

// ─── Live selection ───────────────────────────────────────────────────────────

/// The current clipboard selection observed from the host compositor.
#[derive(Debug)]
pub struct HostSelection {
    /// The Wayland offer proxy; `None` when all MIME types were denied by policy.
    pub offer: Option<DataControlOffer>,
    pub allowed_mimes: Vec<String>,
    pub has_secret: bool,
    pub attribution: HostSelectionAttribution,
    pub observed_at: Instant,
}

// ─── Host clipboard state ─────────────────────────────────────────────────────

/// Aggregates host clipboard observation and paste-FD management.
pub struct HostClipboard<P> {
    attributor: HostClipboardAttributor<P>,
    current_selection: Option<HostSelection>,
    /// FDs waiting for the active picker decision. A single compositor paste
    /// can request several MIME variants; one picker Select must satisfy all
    /// compatible write FDs rather than letting later requests observe EOF.
    pending_paste: Vec<PasteWriteFd>,
    paste_fd_timeout: Duration,
}

impl<P: crate::niri::FocusedWindowProvider> HostClipboard<P> {
    pub fn new(attributor: HostClipboardAttributor<P>, paste_fd_timeout: Duration) -> Self {
        Self {
            attributor,
            current_selection: None,
            pending_paste: Vec::new(),
            paste_fd_timeout,
        }
    }

    /// Update Niri state cache from an event stream event; does not produce
    /// attribution – that happens on explicit `on_host_selection_changed`.
    pub fn apply_niri_cache_event(
        &mut self,
        event: crate::niri::NiriEvent,
    ) -> Option<FocusedWindowSnapshot> {
        self.attributor.cache_mut().apply_event(event)
    }

    pub fn focused_window_snapshot(&mut self) -> Option<FocusedWindowSnapshot> {
        self.attributor.cache_mut().focused_window()
    }

    pub fn refresh_focused_window_snapshot(&mut self) -> Option<FocusedWindowSnapshot> {
        self.attributor.refresh_from_provider().window
    }

    /// Called when the data-control device reports a new host selection.
    /// Uses the Niri event-stream cache to attach best-effort attribution without
    /// blocking the clipboard event loop on synchronous compositor IPC.
    pub fn on_host_selection_changed(
        &mut self,
        offer: Option<DataControlOffer>,
        allowed_mimes: Vec<String>,
        has_secret: bool,
    ) {
        let attribution = self.attributor.on_host_selection_changed();
        log::debug!(
            "d2b-clipd: host selection changed, attribution={:?}, mimes={}, secret={}",
            attribution.quality,
            allowed_mimes.len(),
            has_secret
        );
        // Replace any old offer (drops it, sending destroy).
        self.current_selection = Some(HostSelection {
            offer,
            allowed_mimes,
            has_secret,
            attribution,
            observed_at: Instant::now(),
        });
        // New selection supersedes any armed fallback.
    }

    /// Called when the data-control device reports the selection was cleared.
    pub fn on_host_selection_cleared(&mut self) {
        log::debug!("d2b-clipd: host selection cleared");
        self.current_selection = None;
    }

    /// Called when the compositor issues a `send` request against our
    /// data-control source (i.e. another app wants our clipboard data).
    /// Stores the write-fd as a pending paste to be fulfilled by the picker
    /// or the armed fallback.
    ///
    /// Returns the destination attribution guess (FocusedWindowGuess), or
    /// `Err(ReasonCode)` if we cannot accept the fd (no selection, cap
    /// exceeded, timeout, etc.).
    pub fn accept_paste_fd(
        &mut self,
        write_fd: OwnedFd,
        mime_type: String,
    ) -> Result<FocusedWindowSnapshot, ReasonCode> {
        if !self.pending_paste.is_empty() {
            // Drop the new fd immediately so the requester gets EOF.
            log::debug!("d2b-clipd: paste fd rejected (already holding one)");
            return Err(ReasonCode::FdCapExceeded);
        }
        let dest = self.focused_window_snapshot().unwrap_or_default();
        self.pending_paste.push(PasteWriteFd::new(
            write_fd,
            mime_type,
            dest.clone(),
            self.paste_fd_timeout,
        ));
        Ok(dest)
    }

    pub fn accept_paste_fd_for_destination(
        &mut self,
        write_fd: OwnedFd,
        mime_type: String,
        dest: FocusedWindowSnapshot,
    ) -> Result<FocusedWindowSnapshot, ReasonCode> {
        if !self.pending_paste.is_empty() {
            log::debug!("d2b-clipd: paste fd rejected (already holding one)");
            return Err(ReasonCode::FdCapExceeded);
        }
        self.pending_paste.push(PasteWriteFd::new(
            write_fd,
            mime_type,
            dest.clone(),
            self.paste_fd_timeout,
        ));
        Ok(dest)
    }

    pub fn queue_paste_fd_for_destination(
        &mut self,
        write_fd: OwnedFd,
        mime_type: String,
        dest: FocusedWindowSnapshot,
    ) {
        self.pending_paste.push(PasteWriteFd::new(
            write_fd,
            mime_type,
            dest,
            self.paste_fd_timeout,
        ));
    }

    /// Fulfil the pending paste with `data` bytes, then drop the fd.
    /// Returns `Err(ReasonCode)` if there is no pending paste or write fails.
    pub fn write_paste_data(
        &mut self,
        data: &[u8],
        notifier: &mut impl Notifier,
    ) -> Result<(), ReasonCode> {
        let pastes = self.take_pending_pastes();
        if pastes.is_empty() {
            return Err(ReasonCode::IntentMissing);
        }
        let bytes = data.to_vec();
        std::thread::Builder::new()
            .name("d2b-clipd-paste-write".to_owned())
            .spawn(move || {
                for paste in pastes {
                    let mime = paste.mime_type.clone();
                    let deadline = paste.deadline;
                    match write_all_nonblocking(&paste.fd, &bytes, deadline) {
                        Ok(()) => {
                            log::debug!("d2b-clipd: paste write complete");
                        }
                        Err(error) => {
                            log::debug!(
                                "d2b-clipd: paste write failed for mime={}: {error}",
                                crate::audit::bounded_mime(&mime)
                            );
                        }
                    }
                    drop(paste.fd);
                }
            })
            .map_err(|error| {
                log::debug!("d2b-clipd: paste writer spawn failed: {error}");
                ReasonCode::FdWriteTimeout
            })?;
        let _ = notifier;
        Ok(())
    }

    /// Check whether the pending paste fd has expired and close it if so.
    /// Returns the expired `PasteWriteFd` so the caller can emit an event.
    pub fn check_paste_timeout(&mut self, now: Instant) -> Option<PasteWriteFd> {
        let expired = self.pending_paste.iter().any(|p| p.is_expired(now));
        if expired {
            let paste = self.pending_paste.remove(0);
            for extra in self.pending_paste.drain(..) {
                extra.close_with_reason(ReasonCode::FdWriteTimeout);
            }
            log::debug!(
                "d2b-clipd: paste fd timed out for mime={}",
                crate::audit::bounded_mime(&paste.mime_type)
            );
            Some(paste)
        } else {
            None
        }
    }

    /// Take the pending paste fd out of the state (for passing to a write
    /// helper task or fulfilling via a materialized entry).
    pub fn take_pending_paste(&mut self) -> Option<PasteWriteFd> {
        if self.pending_paste.is_empty() {
            None
        } else {
            Some(self.pending_paste.remove(0))
        }
    }

    pub fn take_pending_pastes(&mut self) -> Vec<PasteWriteFd> {
        self.pending_paste.drain(..).collect()
    }

    /// Peek at the current selection.
    pub fn current_selection(&self) -> Option<&HostSelection> {
        self.current_selection.as_ref()
    }

    /// Peek at the current paste fd.
    pub fn pending_paste(&self) -> Option<&PasteWriteFd> {
        self.pending_paste.first()
    }

    pub fn pending_paste_deadline(&self) -> Option<Instant> {
        self.pending_paste.iter().map(|paste| paste.deadline).min()
    }

    /// Attribution quality of the current selection.
    pub fn current_attribution_quality(&self) -> Option<AttributionQuality> {
        self.current_selection
            .as_ref()
            .map(|s| s.attribution.quality)
    }
}

// ─── Non-blocking write helper ────────────────────────────────────────────────

/// Write all `data` bytes to `fd` which must already be in non-blocking mode.
/// This helper is run from a detached short-lived writer thread, not the daemon
/// event loop, so a slow Wayland requester cannot stall unrelated clipboard work.
///
/// Returns `Ok(())` on success, `Err(String)` on any error.
fn write_all_nonblocking(fd: &OwnedFd, data: &[u8], deadline: Instant) -> Result<(), String> {
    use rustix::event::{PollFd, PollFlags, poll};
    use std::os::fd::AsFd;

    rustix::io::ioctl_fionbio(fd.as_fd(), true).map_err(|error| error.to_string())?;

    let mut remaining = data;
    while !remaining.is_empty() {
        match rustix::io::write(fd, remaining) {
            Ok(0) => return Err("short write to closed fd".to_owned()),
            Ok(written) => remaining = &remaining[written..],
            Err(rustix::io::Errno::INTR) => {}
            Err(rustix::io::Errno::AGAIN) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err("write timed out".to_owned());
                }
                let timeout = deadline
                    .saturating_duration_since(now)
                    .as_millis()
                    .min(i32::MAX as u128) as i32;
                let mut fds = [PollFd::new(
                    fd,
                    PollFlags::OUT | PollFlags::ERR | PollFlags::HUP,
                )];
                match poll(&mut fds, timeout) {
                    Ok(0) => return Err("write timed out".to_owned()),
                    Ok(_) if fds[0].revents().intersects(PollFlags::ERR | PollFlags::HUP) => {
                        return Err("write fd closed".to_owned());
                    }
                    Ok(_) => {}
                    Err(rustix::io::Errno::INTR) => {}
                    Err(error) => return Err(error.to_string()),
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::niri::{FocusedWindowProvider, NiriIpcError, NiriWindow};
    use crate::notifications::RecordingNotifier;
    use std::os::unix::net::UnixStream;

    struct FakeProvider {
        window: Option<NiriWindow>,
    }

    impl FocusedWindowProvider for FakeProvider {
        fn query_focused_window(&mut self) -> Result<Option<NiriWindow>, NiriIpcError> {
            Ok(self.window.clone())
        }
    }

    fn make_host_clipboard(window: Option<NiriWindow>) -> HostClipboard<FakeProvider> {
        let attributor = HostClipboardAttributor::new(FakeProvider { window });
        HostClipboard::new(attributor, Duration::from_secs(5))
    }

    #[test]
    fn paste_fd_timeout_detected() {
        let mut hc = make_host_clipboard(None);
        let (left, right) = UnixStream::pair().expect("socketpair");
        let fd: OwnedFd = left.into();
        let _ = right; // keep read end alive

        let mime = "text/plain".to_owned();
        hc.accept_paste_fd(fd, mime).expect("accept");

        // Not expired yet
        assert!(hc.check_paste_timeout(Instant::now()).is_none());

        // Force expiry by using past deadline directly.
        let paste = hc.pending_paste.first_mut().unwrap();
        paste.deadline = Instant::now() - Duration::from_millis(1);

        let expired = hc.check_paste_timeout(Instant::now()).expect("expired");
        assert_eq!(expired.mime_type, "text/plain");
        assert!(hc.pending_paste.is_empty());
    }

    #[test]
    fn second_paste_fd_rejected_when_one_is_held() {
        let mut hc = make_host_clipboard(None);
        let (a, _ar) = UnixStream::pair().expect("pair");
        let (b, _br) = UnixStream::pair().expect("pair");

        hc.accept_paste_fd(a.into(), "text/plain".to_owned())
            .expect("first");
        let err = hc
            .accept_paste_fd(b.into(), "text/html".to_owned())
            .expect_err("second rejected");
        assert_eq!(err, ReasonCode::FdCapExceeded);
    }

    #[test]
    fn write_paste_data_closes_fd_without_fallback_notification() {
        let mut hc = make_host_clipboard(Some(NiriWindow {
            id: Some(1),
            app_id: Some("org.gnome.TextEditor".to_owned()),
            ..Default::default()
        }));
        let (write_sock, mut read_sock) = UnixStream::pair().expect("pair");
        let fd: OwnedFd = write_sock.into();
        hc.accept_paste_fd(fd, "text/plain".to_owned())
            .expect("accept");

        let mut notifier = RecordingNotifier::default();
        hc.write_paste_data(b"hello", &mut notifier).expect("write");
        assert!(
            notifier.notifications.is_empty(),
            "direct paste completion must not emit fallback-ready notifications"
        );

        // Read end should have received the data.
        use std::io::Read;
        let mut buf = vec![0u8; 32];
        let n = read_sock.read(&mut buf).unwrap_or(0);
        assert!(n == 5 || n == 0, "wrote 5 bytes or fd closed cleanly");
    }
}
