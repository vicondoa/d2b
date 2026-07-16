//! Exact client attribution bookkeeping for provider-neutral Wayland bridges.
//!
//! Canonical target and provider identity are stored separately from rewritten
//! app-id/title metadata. Presentation metadata never grants clipboard authority.

use std::collections::HashMap;

use crate::identity::ProxyIdentity;

const MAX_CLIENT_METADATA_CHARS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ProxyClientId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientAttribution {
    pub client_id: ProxyClientId,
    pub identity: ProxyIdentity,
    pub app_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClientAttributionBook {
    identity: ProxyIdentity,
    clients: HashMap<ProxyClientId, ClientAttribution>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttributionError {
    #[error("Wayland client attribution requires ComponentSession identity")]
    UnauthenticatedIdentity,
}

impl ClientAttributionBook {
    pub fn new(identity: ProxyIdentity) -> Self {
        Self {
            identity,
            clients: HashMap::new(),
        }
    }

    pub fn new_authenticated(identity: ProxyIdentity) -> Result<Self, AttributionError> {
        identity
            .require_component_session()
            .map_err(|_| AttributionError::UnauthenticatedIdentity)?;
        Ok(Self::new(identity))
    }

    pub fn ensure_client(&mut self, client_id: ProxyClientId) -> &mut ClientAttribution {
        self.clients
            .entry(client_id)
            .or_insert_with(|| ClientAttribution {
                client_id,
                identity: self.identity.clone(),
                app_id: None,
                title: None,
            })
    }

    pub fn update_app_id(&mut self, client_id: ProxyClientId, app_id: impl Into<String>) {
        self.ensure_client(client_id).app_id = Some(Self::bound_metadata(app_id.into()));
    }

    pub fn update_title(&mut self, client_id: ProxyClientId, title: impl Into<String>) {
        self.ensure_client(client_id).title = Some(Self::bound_metadata(title.into()));
    }

    pub fn snapshot(&self, client_id: ProxyClientId) -> Option<ClientAttribution> {
        self.clients.get(&client_id).cloned()
    }

    pub fn remove_client(&mut self, client_id: ProxyClientId) {
        self.clients.remove(&client_id);
    }

    fn bound_metadata(value: String) -> String {
        let mut out = String::new();
        for ch in value.chars().take(MAX_CLIENT_METADATA_CHARS) {
            if ch.is_control() {
                out.push('�');
            } else {
                out.push(ch);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use d2b_core::workload_identity::WorkloadTarget;
    use d2b_realm_core::WorkloadProviderKind;

    use super::*;

    fn unsafe_local_identity() -> ProxyIdentity {
        ProxyIdentity::canonical(
            WorkloadTarget::parse("tools.host.d2b").unwrap(),
            WorkloadProviderKind::UnsafeLocal,
        )
    }

    #[test]
    fn attribution_is_exact_to_client_target_and_provider() {
        let identity = unsafe_local_identity();
        let mut book = ClientAttributionBook::new(identity.clone());

        book.update_app_id(ProxyClientId(7), "org.example.Editor");
        book.update_title(ProxyClientId(7), "notes.txt");

        let snapshot = book.snapshot(ProxyClientId(7)).expect("client tracked");
        assert_eq!(snapshot.client_id, ProxyClientId(7));
        assert_eq!(snapshot.identity, identity);
        assert_eq!(snapshot.app_id.as_deref(), Some("org.example.Editor"));
        assert_eq!(snapshot.title.as_deref(), Some("notes.txt"));
    }

    #[test]
    fn presentation_metadata_cannot_change_endpoint_identity() {
        let mut book = ClientAttributionBook::new(unsafe_local_identity());
        book.update_app_id(ProxyClientId(1), "d2b.other.realm.d2b.Terminal");
        book.update_title(ProxyClientId(1), "[isolated] misleading");

        let snapshot = book.snapshot(ProxyClientId(1)).expect("client tracked");
        assert_eq!(snapshot.identity.target().to_canonical(), "tools.host.d2b");
        assert_eq!(
            snapshot.identity.provider_kind(),
            WorkloadProviderKind::UnsafeLocal
        );
    }

    #[test]
    fn attribution_entries_are_per_client() {
        let mut book = ClientAttributionBook::new(unsafe_local_identity());
        book.update_app_id(ProxyClientId(1), "app.one");
        book.update_app_id(ProxyClientId(2), "app.two");
        book.update_title(ProxyClientId(2), "second");

        assert_eq!(
            book.snapshot(ProxyClientId(1))
                .expect("client one")
                .app_id
                .as_deref(),
            Some("app.one")
        );
        assert!(
            book.snapshot(ProxyClientId(1))
                .expect("client one")
                .title
                .is_none()
        );
        assert_eq!(
            book.snapshot(ProxyClientId(2))
                .expect("client two")
                .app_id
                .as_deref(),
            Some("app.two")
        );
    }

    #[test]
    fn removing_client_drops_attribution() {
        let mut book = ClientAttributionBook::new(unsafe_local_identity());
        book.update_app_id(ProxyClientId(1), "app.one");
        book.remove_client(ProxyClientId(1));
        assert!(book.snapshot(ProxyClientId(1)).is_none());
    }

    #[test]
    fn attribution_metadata_is_bounded() {
        let mut book = ClientAttributionBook::new(unsafe_local_identity());
        book.update_title(
            ProxyClientId(1),
            "x".repeat(MAX_CLIENT_METADATA_CHARS + 100),
        );
        let snapshot = book.snapshot(ProxyClientId(1)).expect("client tracked");
        assert_eq!(
            snapshot.title.as_deref().unwrap().chars().count(),
            MAX_CLIENT_METADATA_CHARS
        );
    }

    #[test]
    fn control_attribution_rejects_non_session_identity() {
        assert_eq!(
            ClientAttributionBook::new_authenticated(unsafe_local_identity()).unwrap_err(),
            AttributionError::UnauthenticatedIdentity
        );
    }
}
