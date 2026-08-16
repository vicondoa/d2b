//! Controller-created runtime Volume specification.

use d2b_contracts::v3::ResourceRef;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Runtime Volume finalizer.
pub const RUNTIME_VOLUME_FINALIZER: &str = "runtime-qemu-media.d2bus.org/runtime-volume";

/// Runtime Volume layout entry type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VolumeLayoutType {
    /// Runtime directory.
    Directory,
    /// QMP or serial socket.
    UnixSocket,
}

/// Runtime Volume layout entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayoutEntry {
    /// Relative layout path.
    pub path: String,
    /// Entry type.
    pub entry_type: VolumeLayoutType,
    /// Required mode.
    pub mode: String,
    /// Cleanup policy.
    pub cleanup_policy: String,
    /// Adoption policy.
    pub adoption_policy: String,
    /// Restart policy.
    pub restart_policy: String,
}

/// Runtime Volume named view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeVolumeView {
    /// Relative view path.
    pub path: String,
    /// View rights.
    pub rights: Vec<String>,
}

/// Runtime Volume hard quota.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VolumeQuota {
    /// Byte limit.
    pub max_bytes: u64,
    /// Inode limit.
    pub max_inodes: u32,
    /// Enforcement mode.
    pub enforcement: String,
}

/// Controller-created runtime tmpfs Volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeVolumeSpec {
    /// Deterministic Volume name.
    pub name: String,
    /// Zone containing the Volume.
    pub zone: String,
    /// Owning Guest.
    pub owner_ref: ResourceRef,
    /// Provider reference.
    pub provider_ref: ResourceRef,
    /// Source kind.
    pub source_kind: String,
    /// Source policy identifier.
    pub source_policy_id: String,
    /// Layout entries.
    pub layout: Vec<LayoutEntry>,
    /// Named views.
    pub views: Vec<(String, RuntimeVolumeView)>,
    /// Hard quota.
    pub quota: VolumeQuota,
    /// Finalizer.
    pub finalizer: String,
}

impl RuntimeVolumeSpec {
    /// Build the canonical per-Guest runtime Volume.
    pub fn new(
        guest_ref: ResourceRef,
        zone: impl Into<String>,
        quota_bytes: u64,
        quota_inodes: u32,
    ) -> Result<Self, VolumeSpecError> {
        Self::new_with_provider(
            guest_ref,
            zone,
            ResourceRef::parse("Provider/volume-local").expect("frozen Volume Provider ref"),
            quota_bytes,
            quota_inodes,
        )
    }

    /// Build the runtime Volume with an explicit typed Volume Provider.
    pub fn new_with_provider(
        guest_ref: ResourceRef,
        zone: impl Into<String>,
        provider_ref: ResourceRef,
        quota_bytes: u64,
        quota_inodes: u32,
    ) -> Result<Self, VolumeSpecError> {
        if guest_ref.resource_type().as_str() != "Guest"
            || provider_ref.resource_type().as_str() != "Provider"
            || !(1024 * 1024..=256 * 1024 * 1024).contains(&quota_bytes)
            || !(64..=65_536).contains(&quota_inodes)
        {
            return Err(VolumeSpecError::Invalid);
        }
        let zone = zone.into();
        if zone.is_empty() || zone.len() > 63 || !valid_token(&zone) {
            return Err(VolumeSpecError::Invalid);
        }
        let name = format!(
            "{}-runtime",
            short_guest_key(&guest_ref.to_canonical_string())
        );
        let layout = vec![
            layout(
                "",
                VolumeLayoutType::Directory,
                "0700",
                "preserve-across-controller-restart",
            ),
            layout(
                "qmp.sock",
                VolumeLayoutType::UnixSocket,
                "0600",
                "clear-on-runner-restart",
            ),
            layout(
                "serial.sock",
                VolumeLayoutType::UnixSocket,
                "0600",
                "clear-on-runner-restart",
            ),
        ];
        Ok(Self {
            name,
            zone,
            owner_ref: guest_ref,
            provider_ref,
            source_kind: "tmpfs".to_owned(),
            source_policy_id: "runtime-qemu-media-runtime-tmpfs".to_owned(),
            layout,
            views: vec![
                (
                    "runner".to_owned(),
                    RuntimeVolumeView {
                        path: String::new(),
                        rights: vec![
                            "read".to_owned(),
                            "write".to_owned(),
                            "create".to_owned(),
                            "delete".to_owned(),
                            "traverse".to_owned(),
                        ],
                    },
                ),
                (
                    "controller-observe".to_owned(),
                    RuntimeVolumeView {
                        path: String::new(),
                        rights: vec!["read".to_owned(), "traverse".to_owned()],
                    },
                ),
            ],
            quota: VolumeQuota {
                max_bytes: quota_bytes,
                max_inodes: quota_inodes,
                enforcement: "hard".to_owned(),
            },
            finalizer: RUNTIME_VOLUME_FINALIZER.to_owned(),
        })
    }

    /// Return the owner reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Return the required cleanup policy.
    pub fn cleanup_policy(&self) -> &str {
        self.layout
            .first()
            .map(|entry| entry.cleanup_policy.as_str())
            .unwrap_or("")
    }

    /// Validate the canonical runtime Volume shape.
    pub fn validate(&self) -> Result<(), VolumeSpecError> {
        let expected_layout = vec![
            layout(
                "",
                VolumeLayoutType::Directory,
                "0700",
                "preserve-across-controller-restart",
            ),
            layout(
                "qmp.sock",
                VolumeLayoutType::UnixSocket,
                "0600",
                "clear-on-runner-restart",
            ),
            layout(
                "serial.sock",
                VolumeLayoutType::UnixSocket,
                "0600",
                "clear-on-runner-restart",
            ),
        ];
        let expected_views = vec![
            (
                "runner".to_owned(),
                RuntimeVolumeView {
                    path: String::new(),
                    rights: vec![
                        "read".to_owned(),
                        "write".to_owned(),
                        "create".to_owned(),
                        "delete".to_owned(),
                        "traverse".to_owned(),
                    ],
                },
            ),
            (
                "controller-observe".to_owned(),
                RuntimeVolumeView {
                    path: String::new(),
                    rights: vec!["read".to_owned(), "traverse".to_owned()],
                },
            ),
        ];
        let expected_name = format!(
            "{}-runtime",
            short_guest_key(&self.owner_ref.to_canonical_string())
        );
        if self.owner_ref.resource_type().as_str() != "Guest"
            || self.provider_ref.resource_type().as_str() != "Provider"
            || !valid_token(&self.zone)
            || self.name != expected_name
            || self.source_kind != "tmpfs"
            || self.source_policy_id != "runtime-qemu-media-runtime-tmpfs"
            || self.finalizer != RUNTIME_VOLUME_FINALIZER
            || self.layout != expected_layout
            || self.views != expected_views
            || !(1024 * 1024..=256 * 1024 * 1024).contains(&self.quota.max_bytes)
            || !(64..=65_536).contains(&self.quota.max_inodes)
            || self.quota.enforcement != "hard"
        {
            return Err(VolumeSpecError::Invalid);
        }
        Ok(())
    }
}

/// Runtime Volume specification failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeSpecError {
    /// The shape or bound is invalid.
    Invalid,
}

fn layout(
    path: &str,
    entry_type: VolumeLayoutType,
    mode: &str,
    restart_policy: &str,
) -> LayoutEntry {
    LayoutEntry {
        path: path.to_owned(),
        entry_type,
        mode: mode.to_owned(),
        cleanup_policy: "vm-stop-with-proof".to_owned(),
        adoption_policy: "quarantine-on-ambiguity".to_owned(),
        restart_policy: restart_policy.to_owned(),
    }
}

fn short_guest_key(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
