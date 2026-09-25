//! SeccompProfile ResourceType contract: one posture row.
//!
//! A `SeccompProfile` row is self-contained: the syscall allowlist, the
//! namespace set, the cgroup set, and the device-node binds live inside the
//! row's spec. There are no file paths and no external profile blobs, so a
//! role that references the profile by `seccompRef` resolves a row whose
//! content is the profile, and the referencing resource fails admission when
//! the row is not committed.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use d2b_contracts_resource::v3::execution_policy::{BoundedToken, redacted_debug};

/// Canonical `SeccompProfile` ResourceType name.
pub const SECCOMP_PROFILE_RESOURCE_TYPE: &str = "SeccompProfile";
/// Maximum syscalls in one allowlist.
pub const MAX_SECCOMP_SYSCALLS: usize = 1024;
/// Maximum device-node binds in one profile.
pub const MAX_SECCOMP_DEVICE_BINDS: usize = 64;
/// Maximum bytes of one device-node path.
pub const MAX_DEVICE_NODE_PATH_BYTES: usize = 255;

/// A validated absolute device-node path.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct DeviceNodePath(String);

impl DeviceNodePath {
    /// Parse an absolute device path with no control characters.
    ///
    /// # Errors
    ///
    /// Returns [`SeccompProfileContractError::InvalidDevicePath`] when the
    /// path does not start with `/dev/`, exceeds the byte bound, or
    /// carries a NUL or control character.
    pub fn parse(value: impl Into<String>) -> Result<Self, SeccompProfileContractError> {
        let value = value.into();
        if !value.starts_with("/dev/")
            || value.len() > MAX_DEVICE_NODE_PATH_BYTES
            || value.contains('\u{0}')
            || value.chars().any(char::is_control)
        {
            return Err(SeccompProfileContractError::InvalidDevicePath);
        }
        Ok(Self(value))
    }
}

redacted_debug!(DeviceNodePath);

impl<'de> Deserialize<'de> for DeviceNodePath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for DeviceNodePath {
    fn schema_name() -> String {
        "DeviceNodePath".to_owned()
    }

    fn json_schema(_: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        let mut schema = schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::String,
            ))),
            ..Default::default()
        };
        schema.string().pattern = Some("^/dev/[^\\u0000]+$".to_owned());
        schema.string().max_length = Some(MAX_DEVICE_NODE_PATH_BYTES as u32);
        schemars::schema::Schema::Object(schema)
    }
}

/// The namespace set one profile isolates.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SeccompNamespaces {
    #[serde(default)]
    user: bool,
    #[serde(default)]
    mount: bool,
    #[serde(default)]
    pid: bool,
    #[serde(default)]
    net: bool,
    #[serde(default)]
    uts: bool,
    #[serde(default)]
    ipc: bool,
    #[serde(default)]
    cgroup: bool,
    #[serde(default)]
    time: bool,
}

/// The cgroup set one profile admits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SeccompCgroups {
    #[serde(default)]
    controllers: Vec<BoundedToken>,
}

impl SeccompCgroups {
    /// Construct one cgroup set.
    pub fn new(controllers: Vec<BoundedToken>) -> Self {
        Self { controllers }
    }
}

/// The node kind of one device bind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceNodeKind {
    /// A character device.
    Char,
    /// A block device.
    Block,
}

/// The access one device bind grants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SeccompDeviceAccess {
    /// Read access.
    Read,
    /// Write access.
    Write,
    /// Read and write access.
    ReadWrite,
    /// Node creation access.
    Mknod,
}

/// One device-node bind declared inside a profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceBind {
    path: DeviceNodePath,
    kind: DeviceNodeKind,
    major: u32,
    minor: u32,
    access: SeccompDeviceAccess,
}

impl DeviceBind {
    /// Construct one device bind.
    pub fn new(
        path: DeviceNodePath,
        kind: DeviceNodeKind,
        major: u32,
        minor: u32,
        access: SeccompDeviceAccess,
    ) -> Self {
        Self {
            path,
            kind,
            major,
            minor,
            access,
        }
    }
}

/// The `SeccompProfile` desired spec.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeccompProfileSpec {
    syscalls: Vec<BoundedToken>,
    namespaces: SeccompNamespaces,
    cgroups: SeccompCgroups,
    devices: Vec<DeviceBind>,
}

impl SeccompProfileSpec {
    /// Construct a profile spec after checking the list bounds.
    ///
    /// # Errors
    ///
    /// Returns [`SeccompProfileContractError::TooManySyscalls`] when the
    /// allowlist exceeds the syscall bound and
    /// [`SeccompProfileContractError::TooManyDeviceBinds`] when the
    /// profile declares more device binds than the bound.
    pub fn new(
        syscalls: Vec<BoundedToken>,
        namespaces: SeccompNamespaces,
        cgroups: SeccompCgroups,
        devices: Vec<DeviceBind>,
    ) -> Result<Self, SeccompProfileContractError> {
        if syscalls.len() > MAX_SECCOMP_SYSCALLS {
            return Err(SeccompProfileContractError::TooManySyscalls);
        }
        if devices.len() > MAX_SECCOMP_DEVICE_BINDS {
            return Err(SeccompProfileContractError::TooManyDeviceBinds);
        }
        Ok(Self {
            syscalls,
            namespaces,
            cgroups,
            devices,
        })
    }
}

redacted_debug!(SeccompProfileSpec);

impl<'de> Deserialize<'de> for SeccompProfileSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            syscalls: Vec<BoundedToken>,
            #[serde(default)]
            namespaces: SeccompNamespaces,
            #[serde(default)]
            cgroups: SeccompCgroups,
            #[serde(default)]
            devices: Vec<DeviceBind>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(wire.syscalls, wire.namespaces, wire.cgroups, wire.devices)
            .map_err(serde::de::Error::custom)
    }
}

/// One invalid SeccompProfile declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeccompProfileContractError {
    /// The device path is not under `/dev`, over bound, or carries control
    /// characters.
    InvalidDevicePath,
    /// The allowlist is over the syscall bound.
    TooManySyscalls,
    /// The profile carries more device binds than the bound.
    TooManyDeviceBinds,
}

impl core::fmt::Display for SeccompProfileContractError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::InvalidDevicePath => "device bind path is not an absolute /dev path",
            Self::TooManySyscalls => "seccomp profile declares too many syscalls",
            Self::TooManyDeviceBinds => "seccomp profile declares too many device binds",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for SeccompProfileContractError {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn profile() -> SeccompProfileSpec {
        SeccompProfileSpec::new(
            vec![
                BoundedToken::parse("read").unwrap(),
                BoundedToken::parse("write").unwrap(),
                BoundedToken::parse("exit-group").unwrap(),
            ],
            SeccompNamespaces {
                user: false,
                mount: true,
                pid: true,
                net: true,
                uts: true,
                ipc: true,
                cgroup: true,
                time: false,
            },
            SeccompCgroups::new(vec![BoundedToken::parse("devices").unwrap()]),
            vec![DeviceBind::new(
                DeviceNodePath::parse("/dev/dri/renderD128").unwrap(),
                DeviceNodeKind::Char,
                226,
                128,
                SeccompDeviceAccess::ReadWrite,
            )],
        )
        .expect("profile validates")
    }

    #[test]
    fn device_paths_outside_dev_and_control_characters_are_refused() {
        for path in ["/etc/passwd", "dev/null", "/dev/\u{0}", "/dev/a\u{7}b"] {
            assert!(DeviceNodePath::parse(path).is_err(), "{path:?}");
        }
        assert!(DeviceNodePath::parse("/dev/null").is_ok());
    }

    #[test]
    fn list_bounds_fail_closed() {
        let syscalls = (0..MAX_SECCOMP_SYSCALLS + 1)
            .map(|_| BoundedToken::parse("read").unwrap())
            .collect();
        assert_eq!(
            SeccompProfileSpec::new(
                syscalls,
                SeccompNamespaces::default(),
                SeccompCgroups::default(),
                Vec::new(),
            ),
            Err(SeccompProfileContractError::TooManySyscalls)
        );
    }

    #[test]
    fn the_wire_shape_round_trips_and_is_closed() {
        let profile = profile();
        let bytes =
            d2b_contracts_resource::v3::resource_schema::canonical_json_bytes(&profile).expect("canonical");
        let restored: SeccompProfileSpec = serde_json::from_slice(&bytes).expect("parses");
        assert_eq!(restored, profile);
        let unknown = json!({
            "syscalls": [],
            "profilePath": "/etc/seccomp.json"
        });
        assert!(serde_json::from_value::<SeccompProfileSpec>(unknown).is_err());
    }
}
