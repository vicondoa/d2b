//! Host clipboard state machine.
//!
//! Tracks the current host selection and captures focused-window attribution via
//! `HostClipboardAttributor`.
//!
//! No clipboard content, previews, or paths are logged anywhere in this module.

use std::time::Instant;

use crate::niri::{FocusedWindowSnapshot, HostClipboardAttributor, HostSelectionAttribution};
use crate::policy::AttributionQuality;
use crate::wayland::DataControlOffer;

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

/// Aggregates host clipboard observation and focused-window attribution.
pub struct HostClipboard<P> {
    attributor: HostClipboardAttributor<P>,
    current_selection: Option<HostSelection>,
}

impl<P: crate::niri::FocusedWindowProvider> HostClipboard<P> {
    pub fn new(attributor: HostClipboardAttributor<P>) -> Self {
        Self {
            attributor,
            current_selection: None,
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

    /// Peek at the current selection.
    pub fn current_selection(&self) -> Option<&HostSelection> {
        self.current_selection.as_ref()
    }

    /// Attribution quality of the current selection.
    pub fn current_attribution_quality(&self) -> Option<AttributionQuality> {
        self.current_selection
            .as_ref()
            .map(|s| s.attribution.quality)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::niri::{FocusedWindowProvider, NiriIpcError, NiriWindow};

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
        HostClipboard::new(attributor)
    }

    #[test]
    fn host_selection_records_focused_window_guess() {
        let mut clipboard = make_host_clipboard(Some(NiriWindow {
            id: Some(1),
            app_id: Some("org.example.Terminal".to_owned()),
            ..Default::default()
        }));

        clipboard.on_host_selection_changed(None, vec!["text/plain".to_owned()], false);

        assert_eq!(
            clipboard.current_attribution_quality(),
            Some(AttributionQuality::FocusedWindowGuess)
        );
    }
}
