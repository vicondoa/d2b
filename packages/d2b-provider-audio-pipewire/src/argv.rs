//! Signed vhost-user-sound component-template projection.

use serde_json::{Value, json};

/// Rendered private component template.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedAudioTemplate {
    /// Catalog executable reference, not a host path.
    pub executable_ref: String,
    /// Private template argv, never a live Process spec field.
    pub argv: Vec<String>,
    /// Canonical Process spec projection with no argv.
    pub process_spec_json: Value,
}

/// Template validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTemplateError {
    /// Guest name was empty or malformed.
    GuestNameInvalid,
    /// Binary path was not the per-Guest copy.
    NotPerGuestCopy,
}

impl core::fmt::Display for AudioTemplateError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::GuestNameInvalid => "audio-guest-name-invalid",
            Self::NotPerGuestCopy => "audio-binary-not-per-guest-copy",
        })
    }
}

impl std::error::Error for AudioTemplateError {}

/// Private vhost-user-sound component template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioComponentTemplate {
    guest_name: String,
    binary_path: String,
}

impl AudioComponentTemplate {
    /// Construct a template for the exact guest-owned binary copy.
    pub fn new(
        guest_name: impl Into<String>,
        binary_path: impl Into<String>,
    ) -> Result<Self, AudioTemplateError> {
        let guest_name = guest_name.into();
        let binary_path = binary_path.into();
        if guest_name.is_empty()
            || guest_name.len() > 63
            || !guest_name.bytes().enumerate().all(|(index, byte)| {
                (index == 0 && byte.is_ascii_lowercase())
                    || (index > 0
                        && (byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'))
            })
        {
            return Err(AudioTemplateError::GuestNameInvalid);
        }
        let expected = format!("/run/d2b/vms/{guest_name}/d2b-{guest_name}");
        if binary_path != expected || binary_path.contains("/nix/store/") {
            return Err(AudioTemplateError::NotPerGuestCopy);
        }
        Ok(Self {
            guest_name,
            binary_path,
        })
    }

    /// Render the private template projection.
    pub fn render(&self) -> RenderedAudioTemplate {
        RenderedAudioTemplate {
            executable_ref: "vhost-user-sound-worker".to_owned(),
            argv: vec![
                self.binary_path.clone(),
                "--backend".to_owned(),
                "pipewire".to_owned(),
            ],
            process_spec_json: json!({
                "template": "vhost-user-sound-worker",
                "guest": self.guest_name,
            }),
        }
    }
}
