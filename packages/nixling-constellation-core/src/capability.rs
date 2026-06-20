//! The capability model (ADR 0032). Capabilities are **positive
//! assertions**: a node/provider advertises exactly what it supports, and
//! an absent capability means a typed refusal, never a silent fallback.

use crate::token::ProtocolToken;
use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{ArrayValidation, InstanceType, Schema, SchemaObject, SingleOrVec},
};
use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeSet;
use std::fmt;

/// Schema version for capability negotiation metadata.
pub const CAPABILITY_NEGOTIATION_SCHEMA_VERSION: u32 = 1;
/// Maximum number of capability assertions accepted from a peer.
pub const MAX_CAPABILITY_SET_LEN: usize = 64;

/// A named, independently-authorized capability. Display, clipboard,
/// audio, HID, and USB are deliberately distinct so display forwarding
/// cannot smuggle clipboard or device access.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    /// Workload create/start/stop/inspect.
    Lifecycle,
    /// Command execution.
    Exec,
    /// Interactive pseudo-terminal.
    Pty,
    /// Durable execution logs with resume cursors.
    Logs,
    /// Bounded file copy.
    FileCopy,
    /// One stream per connection; never a generic network bridge.
    PortForward,
    /// virtio-vsock availability.
    Vsock,
    /// virtiofs share availability.
    Virtiofs,
    /// Semantic Wayland window/protocol forwarding.
    WindowForwarding,
    /// Encoded frame/video stream for environments without host Wayland.
    DisplayStreaming,
    /// Clipboard bridge (separate from display).
    Clipboard,
    /// Audio playback.
    AudioPlayback,
    /// Audio capture.
    AudioCapture,
    /// Named HID device operations.
    Hid,
    /// Named USB device operations.
    Usb,
    /// Local/runtime GPU acceleration (not automatically relay-exportable).
    GpuAccel,
    /// Snapshots.
    Snapshots,
    /// Device hotplug.
    Hotplug,
    /// Ephemeral provider-managed sessions.
    EphemeralSessions,
    /// Provider-managed isolation boundary (not host-owned KVM).
    ProviderManagedIsolation,
}

impl Capability {
    /// A short, stable, low-cardinality kebab-case code (for messages and
    /// audit; never a secret).
    pub fn code(self) -> &'static str {
        match self {
            Capability::Lifecycle => "lifecycle",
            Capability::Exec => "exec",
            Capability::Pty => "pty",
            Capability::Logs => "logs",
            Capability::FileCopy => "file-copy",
            Capability::PortForward => "port-forward",
            Capability::Vsock => "vsock",
            Capability::Virtiofs => "virtiofs",
            Capability::WindowForwarding => "window-forwarding",
            Capability::DisplayStreaming => "display-streaming",
            Capability::Clipboard => "clipboard",
            Capability::AudioPlayback => "audio-playback",
            Capability::AudioCapture => "audio-capture",
            Capability::Hid => "hid",
            Capability::Usb => "usb",
            Capability::GpuAccel => "gpu-accel",
            Capability::Snapshots => "snapshots",
            Capability::Hotplug => "hotplug",
            Capability::EphemeralSessions => "ephemeral-sessions",
            Capability::ProviderManagedIsolation => "provider-managed-isolation",
        }
    }

    /// Parse a stable capability code. Unknown capability strings are not
    /// errors at the capability-set boundary; older peers ignore them during
    /// negotiation so forward-compatible advertisements do not drop a whole
    /// session.
    pub fn from_code(code: &str) -> Option<Self> {
        match code {
            "lifecycle" => Some(Capability::Lifecycle),
            "exec" => Some(Capability::Exec),
            "pty" => Some(Capability::Pty),
            "logs" => Some(Capability::Logs),
            "file-copy" => Some(Capability::FileCopy),
            "port-forward" => Some(Capability::PortForward),
            "vsock" => Some(Capability::Vsock),
            "virtiofs" => Some(Capability::Virtiofs),
            "window-forwarding" => Some(Capability::WindowForwarding),
            "display-streaming" => Some(Capability::DisplayStreaming),
            "clipboard" => Some(Capability::Clipboard),
            "audio-playback" => Some(Capability::AudioPlayback),
            "audio-capture" => Some(Capability::AudioCapture),
            "hid" => Some(Capability::Hid),
            "usb" => Some(Capability::Usb),
            "gpu-accel" => Some(Capability::GpuAccel),
            "snapshots" => Some(Capability::Snapshots),
            "hotplug" => Some(Capability::Hotplug),
            "ephemeral-sessions" => Some(Capability::EphemeralSessions),
            "provider-managed-isolation" => Some(Capability::ProviderManagedIsolation),
            _ => None,
        }
    }
}

/// A set of advertised capabilities. Routing is by required capability;
/// callers fail closed when a required capability is absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CapabilitySet(BTreeSet<Capability>);

impl CapabilitySet {
    /// An empty set (advertises nothing).
    pub fn empty() -> Self {
        Self(BTreeSet::new())
    }

    /// Build from an iterator of capabilities.
    pub fn from_caps<I: IntoIterator<Item = Capability>>(caps: I) -> Self {
        caps.into_iter().collect()
    }

    /// Add a capability (builder style).
    pub fn with(mut self, cap: Capability) -> Self {
        self.0.insert(cap);
        self
    }

    /// True iff the capability is advertised.
    pub fn has(&self, cap: Capability) -> bool {
        self.0.contains(&cap)
    }

    /// Iterate the advertised capabilities in a stable order.
    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        self.0.iter().copied()
    }

    /// Deterministic low-cardinality fingerprint for audit/negotiation.
    pub fn stable_fingerprint(&self) -> String {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let mut codes = self.iter().map(Capability::code).collect::<Vec<_>>();
        codes.sort_unstable();
        for code in codes {
            for byte in code.as_bytes() {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
            hash ^= 0xff;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("cap-v{CAPABILITY_NEGOTIATION_SCHEMA_VERSION}-{hash:016x}")
    }

    /// Build a versioned, auditable negotiation record.
    pub fn negotiation(&self) -> CapabilityNegotiation {
        CapabilityNegotiation {
            schema_version: CAPABILITY_NEGOTIATION_SCHEMA_VERSION,
            capabilities: self.clone(),
            fingerprint: self.stable_fingerprint(),
        }
    }

    /// Capabilities shared with `other`.
    pub fn intersection(&self, other: &Self) -> Self {
        Self(self.0.intersection(&other.0).copied().collect())
    }

    /// True iff every advertised capability is also present in `other`.
    pub fn is_subset_of(&self, other: &Self) -> bool {
        self.0.is_subset(&other.0)
    }
}

impl FromIterator<Capability> for CapabilitySet {
    fn from_iter<I: IntoIterator<Item = Capability>>(caps: I) -> Self {
        Self(caps.into_iter().collect())
    }
}

impl<'de> Deserialize<'de> for CapabilitySet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CapabilitySetVisitor;

        impl<'de> Visitor<'de> for CapabilitySetVisitor {
            type Value = CapabilitySet;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "a bounded capability-code array")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut count = 0_usize;
                let mut caps = BTreeSet::new();
                while let Some(capability) = seq.next_element::<ProtocolToken>()? {
                    count += 1;
                    if count > MAX_CAPABILITY_SET_LEN {
                        return Err(serde::de::Error::custom(format!(
                            "capability set exceeds {MAX_CAPABILITY_SET_LEN} entries"
                        )));
                    }
                    if let Some(capability) = Capability::from_code(capability.as_str()) {
                        caps.insert(capability);
                    }
                }
                Ok(CapabilitySet(caps))
            }
        }

        deserializer.deserialize_seq(CapabilitySetVisitor)
    }
}

impl JsonSchema for CapabilitySet {
    fn schema_name() -> String {
        "CapabilitySet".to_owned()
    }

    fn json_schema(r#gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Array))),
            array: Some(Box::new(ArrayValidation {
                items: Some(SingleOrVec::Single(Box::new(
                    r#gen.subschema_for::<ProtocolToken>(),
                ))),
                max_items: Some(MAX_CAPABILITY_SET_LEN as u32),
                unique_items: Some(true),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

/// Versioned negotiated capability set. The fingerprint is deterministic and
/// bounded so audit records can cite the negotiated set without expanding it
/// into every event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityNegotiation {
    /// Capability negotiation schema version.
    pub schema_version: u32,
    /// Positive capability assertions.
    pub capabilities: CapabilitySet,
    /// Deterministic bounded fingerprint of `capabilities`.
    #[schemars(length(max = 64))]
    pub fingerprint: String,
}

impl<'de> Deserialize<'de> for CapabilityNegotiation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            schema_version: u32,
            capabilities: CapabilitySet,
            fingerprint: ProtocolToken,
        }

        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            schema_version: raw.schema_version,
            capabilities: raw.capabilities,
            fingerprint: raw.fingerprint.as_str().to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::MAX_PROTOCOL_TOKEN_LEN;

    #[test]
    fn absent_capability_is_not_advertised() {
        let caps = CapabilitySet::empty().with(Capability::Lifecycle);
        assert!(caps.has(Capability::Lifecycle));
        assert!(!caps.has(Capability::WindowForwarding));
        // display and clipboard are independent.
        let disp = CapabilitySet::from_iter([Capability::WindowForwarding]);
        assert!(disp.has(Capability::WindowForwarding));
        assert!(!disp.has(Capability::Clipboard));
    }

    #[test]
    fn capability_set_decode_ignores_unknown_future_capabilities() {
        let caps: CapabilitySet =
            serde_json::from_str("[\"exec\",\"future-capability\",\"logs\"]").unwrap();
        assert!(caps.has(Capability::Exec));
        assert!(caps.has(Capability::Logs));
        assert!(!caps.has(Capability::Clipboard));
    }

    #[test]
    fn capability_set_decode_rejects_unbounded_inputs() {
        let overlong = format!("[\"{}\"]", "x".repeat(MAX_PROTOCOL_TOKEN_LEN + 1));
        assert!(serde_json::from_str::<CapabilitySet>(&overlong).is_err());

        let too_many = format!(
            "[{}]",
            std::iter::repeat_n("\"exec\"", MAX_CAPABILITY_SET_LEN + 1)
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(serde_json::from_str::<CapabilitySet>(&too_many).is_err());
    }

    #[test]
    fn capability_negotiation_decode_allows_unknown_future_fields() {
        let caps: CapabilityNegotiation = serde_json::from_str(
            "{\"schemaVersion\":1,\"capabilities\":[\"exec\"],\
             \"fingerprint\":\"cap-v1-af63bd4c8601b7df\",\"futureField\":true}",
        )
        .unwrap();
        assert!(caps.capabilities.has(Capability::Exec));
    }

    #[test]
    fn capability_negotiation_decode_bounds_fingerprint() {
        let json = format!(
            "{{\"schemaVersion\":1,\"capabilities\":[],\"fingerprint\":\"{}\"}}",
            "x".repeat(MAX_PROTOCOL_TOKEN_LEN + 1)
        );
        assert!(serde_json::from_str::<CapabilityNegotiation>(&json).is_err());
    }

    #[test]
    fn stable_fingerprint_orders_by_capability_code() {
        let a = CapabilitySet::from_caps([Capability::Logs, Capability::Exec]);
        let b = CapabilitySet::from_caps([Capability::Exec, Capability::Logs]);
        assert_eq!(a.stable_fingerprint(), b.stable_fingerprint());
    }

    #[test]
    fn capability_fingerprint_is_stable_and_order_independent() {
        let a = CapabilitySet::from_caps([Capability::Exec, Capability::Logs]);
        let b = CapabilitySet::from_caps([Capability::Logs, Capability::Exec]);
        let c = CapabilitySet::from_caps([Capability::Exec]);
        assert_eq!(a.stable_fingerprint(), b.stable_fingerprint());
        assert_ne!(a.stable_fingerprint(), c.stable_fingerprint());
        let negotiation = a.negotiation();
        assert_eq!(
            negotiation.schema_version,
            CAPABILITY_NEGOTIATION_SCHEMA_VERSION
        );
        assert_eq!(negotiation.fingerprint, a.stable_fingerprint());
        assert!(negotiation.fingerprint.len() < 32);
    }
}
