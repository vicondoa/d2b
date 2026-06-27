//! Audio policy state DTOs for per-VM audio grant tracking.
//!
//! Manages the versioned audio-state document written to
//! `/var/lib/d2b/vms/<vm>/state/audio-state.json`. Callers never
//! access the file directly; they call [`parse_audio_state`] to
//! decode whatever version is on disk and [`AudioPolicyState::to_v2_bytes`]
//! to write canonical v2 JSON.

use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Metadata, Schema, SchemaObject, SingleOrVec},
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

// ── LevelPercent ────────────────────────────────────────────────────────────

/// A volume or gain level in the range `0..=100`.
///
/// Values outside the range are rejected at construction; the `From<u8>`
/// conversion is intentionally absent so callers cannot bypass the check
/// via an implicit coercion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct LevelPercent(u8);

impl LevelPercent {
    /// Construct a [`LevelPercent`], returning an error when `value > 100`.
    pub fn new(value: u8) -> Result<Self, LevelPercentError> {
        if value > 100 {
            return Err(LevelPercentError::OutOfRange(value));
        }
        Ok(Self(value))
    }

    /// Return the raw value as a `u8`.
    pub fn get(self) -> u8 {
        self.0
    }
}

/// Error returned when a [`LevelPercent`] value is out of range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelPercentError {
    /// The supplied value exceeds 100.
    OutOfRange(u8),
}

impl std::fmt::Display for LevelPercentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfRange(v) => write!(f, "level {v} is out of range; must be 0..=100"),
        }
    }
}

impl std::error::Error for LevelPercentError {}

impl<'de> Deserialize<'de> for LevelPercent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = u8::deserialize(deserializer)?;
        Self::new(v).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for LevelPercent {
    fn schema_name() -> String {
        "LevelPercent".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Integer))),
            number: Some(Box::new(schemars::schema::NumberValidation {
                minimum: Some(0.0),
                maximum: Some(100.0),
                ..Default::default()
            })),
            metadata: Some(Box::new(Metadata {
                description: Some("Audio level in the range 0..=100.".to_owned()),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

// ── AudioGrant ──────────────────────────────────────────────────────────────

/// On/off grant for a single audio channel (mic or speaker).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AudioGrant {
    On,
    Off,
}

impl AudioGrant {
    /// Returns `true` when the grant is `On`.
    pub fn is_on(self) -> bool {
        matches!(self, Self::On)
    }

    /// Wire string used in PipeWire property injection (`"on"` / `"off"`).
    pub fn as_wire_str(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
        }
    }
}

impl std::fmt::Display for AudioGrant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire_str())
    }
}

// ── AudioPolicyState ────────────────────────────────────────────────────────

/// Versioned audio-policy state for a single VM.
///
/// This is the in-memory representation of `audio-state.json`. Parse with
/// [`parse_audio_state`]; write v2 JSON with [`AudioPolicyState::to_v2_bytes`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioPolicyState {
    /// Canonical schema version; always `"v2"` for newly written documents.
    pub schema_version: String,
    /// Microphone grant.
    pub mic: AudioGrant,
    /// Speaker grant.
    pub speaker: AudioGrant,
    /// Speaker output volume, `0..=100`. `None` means "unset; use system default".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_level: Option<LevelPercent>,
    /// Microphone input gain, `0..=100`. `None` means "unset; use system default".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mic_gain: Option<LevelPercent>,
}

impl AudioPolicyState {
    /// Construct a new default v2 state (both channels off, levels unset).
    pub fn default_v2() -> Self {
        Self {
            schema_version: "v2".to_owned(),
            mic: AudioGrant::Off,
            speaker: AudioGrant::Off,
            speaker_level: None,
            mic_gain: None,
        }
    }

    /// Set the microphone grant; returns a new value (builder pattern).
    #[must_use]
    pub fn with_mic(self, grant: AudioGrant) -> Self {
        Self { mic: grant, ..self }
    }

    /// Set the speaker grant; returns a new value (builder pattern).
    #[must_use]
    pub fn with_speaker(self, grant: AudioGrant) -> Self {
        Self {
            speaker: grant,
            ..self
        }
    }

    /// Set the speaker output level; returns a new value.
    #[must_use]
    pub fn with_speaker_level(self, level: LevelPercent) -> Self {
        Self {
            speaker_level: Some(level),
            ..self
        }
    }

    /// Clear the speaker output level (revert to system default).
    #[must_use]
    pub fn without_speaker_level(self) -> Self {
        Self {
            speaker_level: None,
            ..self
        }
    }

    /// Set the microphone input gain; returns a new value.
    #[must_use]
    pub fn with_mic_gain(self, gain: LevelPercent) -> Self {
        Self {
            mic_gain: Some(gain),
            ..self
        }
    }

    /// Clear the microphone input gain (revert to system default).
    #[must_use]
    pub fn without_mic_gain(self) -> Self {
        Self {
            mic_gain: None,
            ..self
        }
    }

    /// Serialize to canonical v2 JSON bytes (compact, deterministic key order).
    ///
    /// The returned bytes are suitable for atomic write to `audio-state.json`.
    /// The `schemaVersion` is always set to `"v2"` regardless of how the
    /// document was originally parsed.
    pub fn to_v2_bytes(&self) -> Result<Vec<u8>, AudioPolicyError> {
        let canonical = AudioPolicyState {
            schema_version: "v2".to_owned(),
            mic: self.mic,
            speaker: self.speaker,
            speaker_level: self.speaker_level,
            mic_gain: self.mic_gain,
        };
        serde_json::to_vec(&canonical).map_err(|err| {
            AudioPolicyError::Serialize(format!("could not serialize audio state: {err}"))
        })
    }
}

// ── parse_audio_state ───────────────────────────────────────────────────────

/// Errors produced by audio-state parsing and serialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioPolicyError {
    /// The raw bytes are not valid JSON.
    InvalidJson(String),
    /// A required field is absent or has an unexpected type/value.
    InvalidField(String),
    /// The schema version token is not recognised.
    UnknownSchemaVersion(String),
    /// Serialization failed (should never happen in practice).
    Serialize(String),
}

impl std::fmt::Display for AudioPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidJson(msg) => write!(f, "audio-state is not valid JSON: {msg}"),
            Self::InvalidField(msg) => write!(f, "audio-state field error: {msg}"),
            Self::UnknownSchemaVersion(v) => {
                write!(f, "audio-state has unknown schemaVersion: {v:?}")
            }
            Self::Serialize(msg) => write!(f, "audio-state serialize error: {msg}"),
        }
    }
}

impl std::error::Error for AudioPolicyError {}

/// Decode an audio-state JSON document, supporting v1 and v2 on-disk formats.
///
/// # v1 format
/// ```json
/// { "mic": "on", "speaker": "off" }
/// ```
/// v1 documents have no `schemaVersion` field and no level fields.
///
/// # v2 format
/// ```json
/// {
///   "schemaVersion": "v2",
///   "mic": "off",
///   "speaker": "on",
///   "speakerLevel": 75,
///   "micGain": 80
/// }
/// ```
/// v2 documents carry `schemaVersion = "v2"`.  Level fields are optional.
///
/// The returned [`AudioPolicyState`] always carries `schema_version = "v2"` so
/// that subsequent writes via [`AudioPolicyState::to_v2_bytes`] produce v2
/// output regardless of which on-disk format was read.
pub fn parse_audio_state(bytes: &[u8]) -> Result<AudioPolicyState, AudioPolicyError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|err| AudioPolicyError::InvalidJson(err.to_string()))?;

    match value.get("schemaVersion").and_then(Value::as_str) {
        None => parse_v1(&value),
        Some("v2") => parse_v2(&value),
        Some(other) => Err(AudioPolicyError::UnknownSchemaVersion(other.to_owned())),
    }
}

fn parse_grant(value: &Value, key: &str) -> Result<AudioGrant, AudioPolicyError> {
    let raw = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AudioPolicyError::InvalidField(format!("missing string field {key:?}")))?;
    match raw {
        "on" => Ok(AudioGrant::On),
        "off" => Ok(AudioGrant::Off),
        other => Err(AudioPolicyError::InvalidField(format!(
            "field {key:?} has invalid value {other:?}; expected \"on\" or \"off\""
        ))),
    }
}

fn parse_optional_level(
    value: &Value,
    key: &str,
) -> Result<Option<LevelPercent>, AudioPolicyError> {
    let Some(raw) = value.get(key) else {
        return Ok(None);
    };
    let n = raw.as_u64().and_then(|v| u8::try_from(v).ok()).ok_or_else(|| {
        AudioPolicyError::InvalidField(format!(
            "field {key:?} must be an integer; got {raw}"
        ))
    })?;
    LevelPercent::new(n).map(Some).map_err(|err| {
        AudioPolicyError::InvalidField(format!("field {key:?}: {err}"))
    })
}

fn parse_v1(value: &Value) -> Result<AudioPolicyState, AudioPolicyError> {
    let mic = parse_grant(value, "mic")?;
    let speaker = parse_grant(value, "speaker")?;
    Ok(AudioPolicyState {
        schema_version: "v2".to_owned(),
        mic,
        speaker,
        speaker_level: None,
        mic_gain: None,
    })
}

fn parse_v2(value: &Value) -> Result<AudioPolicyState, AudioPolicyError> {
    let mic = parse_grant(value, "mic")?;
    let speaker = parse_grant(value, "speaker")?;
    let speaker_level = parse_optional_level(value, "speakerLevel")?;
    let mic_gain = parse_optional_level(value, "micGain")?;
    Ok(AudioPolicyState {
        schema_version: "v2".to_owned(),
        mic,
        speaker,
        speaker_level,
        mic_gain,
    })
}

// ── Provider capability matrix ───────────────────────────────────────────────

/// Audio enforcement capability class for a provider.
///
/// Distinguishes what the host and guest sides can enforce for a given
/// runtime provider, per the ADR 0041 capability matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AudioEnforcementKind {
    /// Full enforcement: policy applied and confirmed in-guest via guestd.
    GuestdEnforced,
    /// Host-side enforcement only; no guest-side confirmation available.
    HostOnly,
    /// Not supported for this provider; enforcement will not be attempted.
    Unsupported,
}

/// Per-provider audio capability row, used by the daemon to select the
/// correct enforcement path before dispatching an audio op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AudioProviderCapability {
    /// Whether the host side can enforce audio policy for this provider.
    pub host_enforcement: AudioEnforcementKind,
    /// Whether the guest side (via guestd) can enforce audio policy.
    pub guest_enforcement: AudioEnforcementKind,
    /// Whether the provider requires a local audio-state file on the host.
    pub needs_local_state_file: bool,
}

impl AudioProviderCapability {
    /// Capability row for Cloud Hypervisor NixOS VMs.
    pub fn cloud_hypervisor_nixos() -> Self {
        Self {
            host_enforcement: AudioEnforcementKind::GuestdEnforced,
            guest_enforcement: AudioEnforcementKind::GuestdEnforced,
            needs_local_state_file: true,
        }
    }

    /// Capability row for qemu-media VMs: host-only, no guest enforcement.
    pub fn qemu_media() -> Self {
        Self {
            host_enforcement: AudioEnforcementKind::HostOnly,
            guest_enforcement: AudioEnforcementKind::Unsupported,
            needs_local_state_file: true,
        }
    }

    /// Capability row for ACA sandbox targets: guest-only via relay, no
    /// local host state.
    pub fn aca_sandbox() -> Self {
        Self {
            host_enforcement: AudioEnforcementKind::Unsupported,
            guest_enforcement: AudioEnforcementKind::GuestdEnforced,
            needs_local_state_file: false,
        }
    }
}

/// Console access capability class for a provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ConsoleBackendKind {
    /// Local hypervisor console backend (CH serial socket or broker-owned fd).
    LocalHypervisor,
    /// Provider relay transport (ACA sandbox via ADR 0032 guestd route).
    ProviderRelay,
    /// Not supported for this provider.
    Unsupported,
}

/// Per-provider console capability row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsoleProviderCapability {
    /// How the console stream is established for this provider.
    pub backend: ConsoleBackendKind,
    /// Whether a persistent drain keeps the ring buffer populated even when
    /// no client is attached.
    pub persistent_drain: bool,
}

impl ConsoleProviderCapability {
    /// Capability row for Cloud Hypervisor NixOS VMs.
    pub fn cloud_hypervisor_nixos() -> Self {
        Self {
            backend: ConsoleBackendKind::LocalHypervisor,
            persistent_drain: true,
        }
    }

    /// Capability row for qemu-media VMs.
    pub fn qemu_media() -> Self {
        Self {
            backend: ConsoleBackendKind::LocalHypervisor,
            persistent_drain: true,
        }
    }

    /// Capability row for ACA sandbox targets.
    pub fn aca_sandbox() -> Self {
        Self {
            backend: ConsoleBackendKind::ProviderRelay,
            persistent_drain: false,
        }
    }
}

