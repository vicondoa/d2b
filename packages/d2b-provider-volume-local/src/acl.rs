//! ACL reconciliation decisions for Volume layout entries.
//!
//! ACL values are decoded only long enough to establish typed principal
//! identity and bounded grant counts.  The effect adapter owns the actual
//! ACL syscalls and is the only layer that sees filesystem ACL state.

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::volume::{ForeignChildPolicy, LayoutEntry};

use crate::error::VolumeLocalError;

/// One bounded ACL grant summary.
#[derive(Clone, PartialEq, Eq)]
pub struct AclGrantSummary {
    principal: ResourceRef,
    permission_count: u8,
}

impl AclGrantSummary {
    /// Borrow the typed User principal.
    pub const fn principal(&self) -> &ResourceRef {
        &self.principal
    }

    /// Return the number of declared permission characters without exposing
    /// their value.
    pub const fn permission_count(&self) -> u8 {
        self.permission_count
    }
}

impl core::fmt::Debug for AclGrantSummary {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AclGrantSummary")
            .field("permission_count", &self.permission_count)
            .finish_non_exhaustive()
    }
}

/// The POSIX permission bits of one declared `[rwx]{1,3}` grant spelling.
fn grant_permission_bits(permissions: &str) -> Result<u8, VolumeLocalError> {
    if permissions.is_empty() || permissions.len() > 3 {
        return Err(VolumeLocalError::InvalidSpec);
    }
    let mut bits = 0u8;
    for byte in permissions.bytes() {
        bits |= match byte {
            b'r' => 0b100,
            b'w' => 0b010,
            b'x' => 0b001,
            _ => return Err(VolumeLocalError::InvalidSpec),
        };
    }
    Ok(bits)
}

/// The group-class bits of one declared four-digit octal mode.
fn mode_group_bits(mode: &str) -> Result<u8, VolumeLocalError> {
    let bytes = mode.as_bytes();
    if bytes.len() != 4 || bytes.iter().any(|byte| !(b'0'..=b'7').contains(byte)) {
        return Err(VolumeLocalError::InvalidSpec);
    }
    Ok(bytes[2] - b'0')
}

/// The ACL bindings declared by one layout entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AclBinding {
    access: Vec<AclGrantSummary>,
    default: Vec<AclGrantSummary>,
    foreign_child_policy: ForeignChildPolicy,
}

impl AclBinding {
    /// Decode one entry's bounded ACL projection.
    ///
    /// A declaration whose grants exceed the mode's group-class bits is
    /// refused: named entries force an ACL mask, and the kernel mirrors that
    /// mask into the file's group bits, so applying it would silently widen
    /// the declared mode.
    pub fn from_entry(entry: &LayoutEntry) -> Result<Self, VolumeLocalError> {
        let rendered = serde_json::to_value(entry).map_err(|_| VolumeLocalError::InvalidSpec)?;
        Self::from_rendered(&rendered)
    }

    /// Decode the canonical rendering of one entry.
    ///
    /// The rendering is the frozen contract the lifecycle fields are also read
    /// back from, so the widening check sees the same declaration the entry
    /// was built from.
    pub(crate) fn from_rendered(rendered: &serde_json::Value) -> Result<Self, VolumeLocalError> {
        let grants = |name: &str| -> Result<(Vec<AclGrantSummary>, u8), VolumeLocalError> {
            let values = rendered
                .get(name)
                .and_then(serde_json::Value::as_array)
                .ok_or(VolumeLocalError::InvalidSpec)?;
            let mut summaries = Vec::with_capacity(values.len());
            let mut permissions = 0u8;
            for grant in values {
                let principal = grant
                    .get("principal")
                    .and_then(|principal| principal.get("ref"))
                    .and_then(serde_json::Value::as_str)
                    .ok_or(VolumeLocalError::InvalidSpec)
                    .and_then(|reference| {
                        let reference = ResourceRef::parse(reference)
                            .map_err(|_| VolumeLocalError::InvalidSpec)?;
                        if reference.resource_type().as_str() != "User" {
                            return Err(VolumeLocalError::InvalidSpec);
                        }
                        Ok(reference)
                    })?;
                let spelling = grant
                    .get("permissions")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(VolumeLocalError::InvalidSpec)?;
                permissions |= grant_permission_bits(spelling)?;
                summaries.push(AclGrantSummary {
                    principal,
                    permission_count: u8::try_from(spelling.len())
                        .map_err(|_| VolumeLocalError::InvalidSpec)?,
                });
            }
            Ok((summaries, permissions))
        };
        let foreign_child_policy = rendered
            .get("foreignChildPolicy")
            .cloned()
            .ok_or(VolumeLocalError::InvalidSpec)
            .and_then(|value| {
                serde_json::from_value(value).map_err(|_| VolumeLocalError::InvalidSpec)
            })?;
        let (access, access_permissions) = grants("accessAcl")?;
        let (default, default_permissions) = grants("defaultAcl")?;
        let mode = rendered
            .get("mode")
            .and_then(serde_json::Value::as_str)
            .ok_or(VolumeLocalError::InvalidSpec)?;
        if (access_permissions | default_permissions) & !mode_group_bits(mode)? != 0 {
            return Err(VolumeLocalError::InvalidSpec);
        }
        Ok(Self {
            access,
            default,
            foreign_child_policy,
        })
    }

    /// Return whether an access ACL is declared.
    pub const fn has_access(&self) -> bool {
        !self.access.is_empty()
    }

    /// Return whether a default ACL is declared.
    pub const fn has_default(&self) -> bool {
        !self.default.is_empty()
    }

    /// Return the foreign-child policy.
    pub const fn foreign_child_policy(&self) -> ForeignChildPolicy {
        self.foreign_child_policy
    }

    /// Return the total number of declared grants.
    pub fn grant_count(&self) -> usize {
        self.access.len() + self.default.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression (security review): a named grant the declared mode's group
    /// class does not cover would have the ACL mask mirrored into the file's
    /// group bits, silently widening the mode, so decode refuses the entry.
    #[test]
    fn grants_wider_than_the_group_class_are_refused() {
        let widened_access = serde_json::from_value(serde_json::json!({
            "path": "",
            "type": "directory",
            "ownerRef": "User/d2bd",
            "groupRef": "User/d2bd",
            "mode": "0700",
            "accessAcl": [{ "principal": { "ref": "User/alice" }, "permissions": "rwx" }],
            "defaultAcl": [],
            "foreignChildPolicy": "preserve"
        }))
        .expect("valid entry");
        assert_eq!(
            AclBinding::from_entry(&widened_access),
            Err(VolumeLocalError::InvalidSpec)
        );

        // The default ACL carries the same mask, so it is refused the same way.
        let widened_default = serde_json::from_value(serde_json::json!({
            "path": "",
            "type": "directory",
            "ownerRef": "User/d2bd",
            "groupRef": "User/d2bd",
            "mode": "0750",
            "accessAcl": [],
            "defaultAcl": [{ "principal": { "ref": "User/alice" }, "permissions": "rwx" }],
            "foreignChildPolicy": "preserve"
        }))
        .expect("valid entry");
        assert_eq!(
            AclBinding::from_entry(&widened_default),
            Err(VolumeLocalError::InvalidSpec)
        );
    }

    /// The Device TPM Provider's state Volume declares `0770` with the two TPM
    /// principals as its only named entries; their grants stay inside the
    /// group class, so the declaration keeps working.
    #[test]
    fn tpm_state_volume_grants_fit_the_group_class() {
        let state_volume = serde_json::from_value(serde_json::json!({
            "path": "",
            "type": "directory",
            "ownerRef": "User/d2b-guest-swtpm",
            "groupRef": "User/d2b-guest-swtpm",
            "mode": "0770",
            "accessAcl": [
                { "principal": { "ref": "User/d2b-guest-swtpm" }, "permissions": "rwx" },
                { "principal": { "ref": "User/d2b-guest-swtpm-flush" }, "permissions": "rx" }
            ],
            "defaultAcl": [
                { "principal": { "ref": "User/d2b-guest-swtpm" }, "permissions": "rwx" },
                { "principal": { "ref": "User/d2b-guest-swtpm-flush" }, "permissions": "rw" }
            ],
            "foreignChildPolicy": "preserve"
        }))
        .expect("valid entry");
        let binding = AclBinding::from_entry(&state_volume).expect("coherent grants decode");
        assert!(binding.has_access());
        assert!(binding.has_default());
        assert_eq!(binding.grant_count(), 4);
    }
}
