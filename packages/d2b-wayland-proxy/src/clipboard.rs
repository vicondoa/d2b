//! Clipboard virtualization policy.
//!
//! d2b-wayland-proxy synthesizes the guest-visible standard
//! `wl_data_device_manager` path locally. Downstream `wl_data_*` objects are never
//! bound into the host compositor clipboard namespace. Same-endpoint transfers are
//! routed within the proxy; host and cross-realm materialization routes through
//! d2b-clipd.

use std::collections::BTreeSet;

pub const MAX_CLIPBOARD_PAYLOAD_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_CLIPBOARD_MIME_BYTES: usize = 128;
pub const TRANSFER_CREDIT_SCOPE_COUNT: usize = 6;

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
    SameEndpoint,
    HostOrCrossRealm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MimeDecision {
    PreserveSameEndpointRichMime,
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
        if mime.is_empty() || mime.len() > MAX_CLIPBOARD_MIME_BYTES {
            return MimeDecision::Deny;
        }
        match route {
            ClipboardRoute::SameEndpoint => MimeDecision::PreserveSameEndpointRichMime,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferCreditLimits {
    pub packet: u32,
    pub request: u32,
    pub operation: u32,
    pub session: u32,
    pub process: u32,
    pub host: u32,
}

impl TransferCreditLimits {
    pub const COMPONENT_SESSION_DEFAULT: Self = Self {
        packet: 1,
        request: 1,
        operation: 1,
        session: 64,
        process: 2_048,
        host: 8_192,
    };

    const fn as_array(self) -> [u32; TRANSFER_CREDIT_SCOPE_COUNT] {
        [
            self.packet,
            self.request,
            self.operation,
            self.session,
            self.process,
            self.host,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransferCreditUsage {
    pub packet: u32,
    pub request: u32,
    pub operation: u32,
    pub session: u32,
    pub process: u32,
    pub host: u32,
}

impl TransferCreditUsage {
    const fn as_array(self) -> [u32; TRANSFER_CREDIT_SCOPE_COUNT] {
        [
            self.packet,
            self.request,
            self.operation,
            self.session,
            self.process,
            self.host,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClipboardTransferLimits {
    pub max_payload_bytes: u64,
    pub credits: TransferCreditLimits,
}

impl Default for ClipboardTransferLimits {
    fn default() -> Self {
        Self {
            max_payload_bytes: MAX_CLIPBOARD_PAYLOAD_BYTES,
            credits: TransferCreditLimits::COMPONENT_SESSION_DEFAULT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TransferAdmissionError {
    #[error("clipboard payload exceeds the authenticated transfer limit")]
    PayloadLimit,
    #[error("clipboard transfer must carry exactly one descriptor")]
    DescriptorCount,
    #[error("clipboard descriptor credit is exhausted")]
    CreditExhausted,
    #[error("clipboard credit arithmetic overflowed")]
    CreditOverflow,
    #[error("clipboard transfer limits are invalid")]
    InvalidLimits,
}

pub fn admit_component_session_transfer(
    payload_bytes: u64,
    descriptor_count: u32,
    usage: TransferCreditUsage,
    limits: ClipboardTransferLimits,
) -> Result<TransferCreditUsage, TransferAdmissionError> {
    if limits.max_payload_bytes == 0
        || limits.max_payload_bytes > MAX_CLIPBOARD_PAYLOAD_BYTES
        || limits.credits.as_array().contains(&0)
    {
        return Err(TransferAdmissionError::InvalidLimits);
    }
    if payload_bytes > limits.max_payload_bytes {
        return Err(TransferAdmissionError::PayloadLimit);
    }
    if descriptor_count != 1 {
        return Err(TransferAdmissionError::DescriptorCount);
    }

    let current = usage.as_array();
    let limits = limits.credits.as_array();
    let mut next = [0_u32; TRANSFER_CREDIT_SCOPE_COUNT];
    for index in 0..TRANSFER_CREDIT_SCOPE_COUNT {
        next[index] = current[index]
            .checked_add(descriptor_count)
            .ok_or(TransferAdmissionError::CreditOverflow)?;
        if next[index] > limits[index] {
            return Err(TransferAdmissionError::CreditExhausted);
        }
    }
    Ok(TransferCreditUsage {
        packet: next[0],
        request: next[1],
        operation: next[2],
        session: next[3],
        process: next[4],
        host: next[5],
    })
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
    fn same_endpoint_custom_mime_placeholder_preserves_rich_semantics() {
        let policy = ClipboardMimePolicy::v1_defaults();

        assert_eq!(
            policy.decide(
                ClipboardRoute::SameEndpoint,
                "application/vnd.libreoffice.rich-text"
            ),
            MimeDecision::PreserveSameEndpointRichMime
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

    #[test]
    fn component_session_transfer_payload_and_credits_are_bounded() {
        let limits = ClipboardTransferLimits::default();
        let admitted = admit_component_session_transfer(
            MAX_CLIPBOARD_PAYLOAD_BYTES,
            1,
            TransferCreditUsage::default(),
            limits,
        )
        .unwrap();
        assert_eq!(admitted.packet, 1);
        assert_eq!(admitted.host, 1);

        assert_eq!(
            admit_component_session_transfer(
                MAX_CLIPBOARD_PAYLOAD_BYTES + 1,
                1,
                TransferCreditUsage::default(),
                limits,
            ),
            Err(TransferAdmissionError::PayloadLimit)
        );
        assert_eq!(
            admit_component_session_transfer(1, 0, TransferCreditUsage::default(), limits),
            Err(TransferAdmissionError::DescriptorCount)
        );
        assert_eq!(
            admit_component_session_transfer(1, 2, TransferCreditUsage::default(), limits),
            Err(TransferAdmissionError::DescriptorCount)
        );
    }

    #[test]
    fn every_component_session_credit_scope_fails_closed() {
        let limits = ClipboardTransferLimits::default();
        for exhausted_scope in 0..TRANSFER_CREDIT_SCOPE_COUNT {
            let mut values = [0_u32; TRANSFER_CREDIT_SCOPE_COUNT];
            values[exhausted_scope] = limits.credits.as_array()[exhausted_scope];
            let usage = TransferCreditUsage {
                packet: values[0],
                request: values[1],
                operation: values[2],
                session: values[3],
                process: values[4],
                host: values[5],
            };
            assert_eq!(
                admit_component_session_transfer(1, 1, usage, limits),
                Err(TransferAdmissionError::CreditExhausted)
            );
        }
    }

    #[test]
    fn oversized_or_empty_mime_is_denied_before_routing() {
        let policy = ClipboardMimePolicy::v1_defaults();
        assert_eq!(
            policy.decide(ClipboardRoute::SameEndpoint, ""),
            MimeDecision::Deny
        );
        assert_eq!(
            policy.decide(
                ClipboardRoute::SameEndpoint,
                &"x".repeat(MAX_CLIPBOARD_MIME_BYTES + 1)
            ),
            MimeDecision::Deny
        );
    }
}
