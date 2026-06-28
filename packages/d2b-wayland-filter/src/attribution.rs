//! Exact guest-client attribution bookkeeping for the VM Wayland bridge.
//!
//! These types intentionally store the authenticated VM id separately from the
//! host-visible app-id rewrite prefix. App ids and titles are guest metadata for
//! policy/UI context; they are not authority.

use std::collections::HashMap;

/// Per-proxy client connection id assigned by `d2b-wayland-filter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GuestClientId(pub u64);

/// Authenticated d2b VM identity for this Wayland bridge session.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VmId(String);

impl VmId {
    pub fn new(value: impl Into<String>) -> Result<Self, AttributionError> {
        let value = value.into();
        if value.is_empty() || value.contains('/') || value.contains('\0') {
            return Err(AttributionError::InvalidVmId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Metadata known for one guest client connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestClientAttribution {
    pub client_id: GuestClientId,
    pub vm_id: VmId,
    pub app_id: Option<String>,
    pub title: Option<String>,
}

/// In-memory per-VM attribution table updated by Wayland object handlers.
#[derive(Debug, Clone)]
pub struct ClientAttributionBook {
    vm_id: VmId,
    clients: HashMap<GuestClientId, GuestClientAttribution>,
}

impl ClientAttributionBook {
    pub fn new(vm_id: VmId) -> Self {
        Self {
            vm_id,
            clients: HashMap::new(),
        }
    }

    pub fn ensure_client(&mut self, client_id: GuestClientId) -> &mut GuestClientAttribution {
        self.clients
            .entry(client_id)
            .or_insert_with(|| GuestClientAttribution {
                client_id,
                vm_id: self.vm_id.clone(),
                app_id: None,
                title: None,
            })
    }

    pub fn update_app_id(&mut self, client_id: GuestClientId, app_id: impl Into<String>) {
        self.ensure_client(client_id).app_id = Some(app_id.into());
    }

    pub fn update_title(&mut self, client_id: GuestClientId, title: impl Into<String>) {
        self.ensure_client(client_id).title = Some(title.into());
    }

    pub fn snapshot(&self, client_id: GuestClientId) -> Option<GuestClientAttribution> {
        self.clients.get(&client_id).cloned()
    }

    pub fn remove_client(&mut self, client_id: GuestClientId) {
        self.clients.remove(&client_id);
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AttributionError {
    #[error("invalid VM id")]
    InvalidVmId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attribution_is_exact_to_client_and_vm() {
        let vm = VmId::new("work").expect("valid vm");
        let mut book = ClientAttributionBook::new(vm.clone());

        book.update_app_id(GuestClientId(7), "org.example.Editor");
        book.update_title(GuestClientId(7), "notes.txt");

        let snapshot = book.snapshot(GuestClientId(7)).expect("client tracked");
        assert_eq!(snapshot.client_id, GuestClientId(7));
        assert_eq!(snapshot.vm_id, vm);
        assert_eq!(snapshot.app_id.as_deref(), Some("org.example.Editor"));
        assert_eq!(snapshot.title.as_deref(), Some("notes.txt"));
    }

    #[test]
    fn attribution_does_not_derive_vm_from_app_id_prefix() {
        let vm = VmId::new("work").expect("valid vm");
        let mut book = ClientAttributionBook::new(vm);

        book.update_app_id(GuestClientId(1), "d2b.personal.org.example.Terminal");

        let snapshot = book.snapshot(GuestClientId(1)).expect("client tracked");
        assert_eq!(snapshot.vm_id.as_str(), "work");
        assert_eq!(
            snapshot.app_id.as_deref(),
            Some("d2b.personal.org.example.Terminal")
        );
    }

    #[test]
    fn attribution_entries_are_per_client() {
        let vm = VmId::new("work").expect("valid vm");
        let mut book = ClientAttributionBook::new(vm);

        book.update_app_id(GuestClientId(1), "app.one");
        book.update_app_id(GuestClientId(2), "app.two");
        book.update_title(GuestClientId(2), "second");

        assert_eq!(
            book.snapshot(GuestClientId(1))
                .expect("client one")
                .app_id
                .as_deref(),
            Some("app.one")
        );
        assert_eq!(
            book.snapshot(GuestClientId(1))
                .expect("client one")
                .title
                .as_deref(),
            None
        );
        assert_eq!(
            book.snapshot(GuestClientId(2))
                .expect("client two")
                .app_id
                .as_deref(),
            Some("app.two")
        );
    }

    #[test]
    fn removing_client_drops_attribution() {
        let vm = VmId::new("work").expect("valid vm");
        let mut book = ClientAttributionBook::new(vm);

        book.update_app_id(GuestClientId(1), "app.one");
        book.remove_client(GuestClientId(1));

        assert!(book.snapshot(GuestClientId(1)).is_none());
    }
}
