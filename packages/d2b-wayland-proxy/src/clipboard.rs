//! Clipboard virtualization policy.
//!
//! d2b-wayland-proxy synthesizes the guest-visible standard
//! `wl_data_device_manager` path locally. Guest `wl_data_*` objects are never
//! bound into the host compositor clipboard namespace. Same-VM transfers are
//! routed within the proxy; host and cross-realm materialization routes through
//! d2b-clipd.

use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardGlobalDisposition {
    /// Hide the compositor global from the guest in v1.
    DenyGlobal,
    /// Synthesize a guest-facing global locally; do not bind upstream.
    VirtualizeLocally,
    /// Not part of the clipboard/DND boundary.
    NotClipboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardObjectForwarding {
    NeverForwardUpstream,
    NotClipboard,
}

pub fn global_disposition(interface: &str) -> ClipboardGlobalDisposition {
    match interface {
        "wl_data_device_manager" => ClipboardGlobalDisposition::VirtualizeLocally,
        "ext_data_control_manager_v1"
        | "zwlr_data_control_manager_v1"
        | "zwp_primary_selection_device_manager_v1"
        | "wp_primary_selection_device_manager_v1"
        | "wp_primary_selection_unstable_v1"
        | "gtk_primary_selection_device_manager"
        | "xdg_toplevel_drag_manager_v1" => ClipboardGlobalDisposition::DenyGlobal,
        _ => ClipboardGlobalDisposition::NotClipboard,
    }
}

pub fn object_forwarding(interface: &str) -> ClipboardObjectForwarding {
    match interface {
        "wl_data_device_manager"
        | "wl_data_device"
        | "wl_data_source"
        | "wl_data_offer"
        | "zwp_primary_selection_device_manager_v1"
        | "zwp_primary_selection_device_v1"
        | "zwp_primary_selection_source_v1"
        | "zwp_primary_selection_offer_v1"
        | "wp_primary_selection_device_manager_v1"
        | "gtk_primary_selection_device_manager"
        | "ext_data_control_manager_v1"
        | "ext_data_control_device_v1"
        | "ext_data_control_source_v1"
        | "ext_data_control_offer_v1"
        | "zwlr_data_control_manager_v1"
        | "zwlr_data_control_device_v1"
        | "zwlr_data_control_source_v1"
        | "zwlr_data_control_offer_v1"
        | "xdg_toplevel_drag_manager_v1"
        | "xdg_toplevel_drag_v1" => ClipboardObjectForwarding::NeverForwardUpstream,
        _ => ClipboardObjectForwarding::NotClipboard,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardRoute {
    SameVm,
    HostOrCrossRealm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MimeDecision {
    PreserveSameVmRichMime,
    MaterializeViaBridge,
    Deny,
}

#[derive(Debug, Clone)]
pub struct ClipboardMimePolicy {
    external_allowlist: BTreeSet<&'static str>,
}

impl ClipboardMimePolicy {
    pub fn v1_defaults() -> Self {
        Self {
            external_allowlist: [
                "text/plain;charset=utf-8",
                "text/plain",
                "text/html",
                "image/png",
            ]
            .into_iter()
            .collect(),
        }
    }

    pub fn decide(&self, route: ClipboardRoute, mime: &str) -> MimeDecision {
        match route {
            ClipboardRoute::SameVm => MimeDecision::PreserveSameVmRichMime,
            ClipboardRoute::HostOrCrossRealm if self.external_allowlist.contains(mime) => {
                MimeDecision::MaterializeViaBridge
            }
            ClipboardRoute::HostOrCrossRealm => MimeDecision::Deny,
        }
    }

    pub fn external_mimes(&self) -> Vec<&'static str> {
        let mut mimes = self.external_allowlist.iter().copied().collect::<Vec<_>>();
        mimes.sort_unstable();
        mimes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_clipboard_is_reserved_for_virtualization_not_forwarding() {
        assert_eq!(
            global_disposition("wl_data_device_manager"),
            ClipboardGlobalDisposition::VirtualizeLocally
        );
        for iface in [
            "wl_data_device_manager",
            "wl_data_device",
            "wl_data_source",
            "wl_data_offer",
        ] {
            assert_eq!(
                object_forwarding(iface),
                ClipboardObjectForwarding::NeverForwardUpstream,
                "{iface} must not be forwarded into the host clipboard namespace"
            );
        }
    }

    #[test]
    fn privileged_primary_and_dnd_protocols_are_not_forwarded() {
        for iface in [
            "ext_data_control_manager_v1",
            "zwlr_data_control_manager_v1",
            "zwp_primary_selection_device_manager_v1",
            "wp_primary_selection_device_manager_v1",
            "gtk_primary_selection_device_manager",
            "xdg_toplevel_drag_manager_v1",
        ] {
            assert_ne!(
                global_disposition(iface),
                ClipboardGlobalDisposition::NotClipboard
            );
            assert_eq!(
                object_forwarding(iface),
                ClipboardObjectForwarding::NeverForwardUpstream
            );
        }
    }

    #[test]
    fn same_vm_custom_mime_placeholder_preserves_rich_semantics() {
        let policy = ClipboardMimePolicy::v1_defaults();

        assert_eq!(
            policy.decide(
                ClipboardRoute::SameVm,
                "application/vnd.libreoffice.rich-text"
            ),
            MimeDecision::PreserveSameVmRichMime
        );
    }

    #[test]
    fn external_custom_mime_is_denied_while_allowlisted_text_materializes() {
        let policy = ClipboardMimePolicy::v1_defaults();

        assert_eq!(
            policy.decide(ClipboardRoute::HostOrCrossRealm, "text/plain;charset=utf-8"),
            MimeDecision::MaterializeViaBridge
        );
        assert_eq!(
            policy.decide(ClipboardRoute::HostOrCrossRealm, "application/octet-stream"),
            MimeDecision::Deny
        );
    }
}
