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
/// Maximum cgroup controllers in one profile.
pub const MAX_SECCOMP_CGROUP_CONTROLLERS: usize = 16;
/// Maximum bytes of one device-node path.
pub const MAX_DEVICE_NODE_PATH_BYTES: usize = 255;

/// A validated absolute device-node path.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct DeviceNodePath(String);

impl DeviceNodePath {
    /// Parse an absolute device path with no control characters.
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

    /// Borrow the path.
    pub fn as_str(&self) -> &str {
        &self.0
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

impl SeccompNamespaces {
    /// Construct one namespace set.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        user: bool,
        mount: bool,
        pid: bool,
        net: bool,
        uts: bool,
        ipc: bool,
        cgroup: bool,
        time: bool,
    ) -> Self {
        Self {
            user,
            mount,
            pid,
            net,
            uts,
            ipc,
            cgroup,
            time,
        }
    }

    /// Whether a user namespace is used.
    pub const fn user(&self) -> bool {
        self.user
    }

    /// Whether a mount namespace is isolated.
    pub const fn mount(&self) -> bool {
        self.mount
    }

    /// Whether a PID namespace is isolated.
    pub const fn pid(&self) -> bool {
        self.pid
    }

    /// Whether a network namespace is isolated.
    pub const fn net(&self) -> bool {
        self.net
    }

    /// Whether a UTS namespace is isolated.
    pub const fn uts(&self) -> bool {
        self.uts
    }

    /// Whether an IPC namespace is isolated.
    pub const fn ipc(&self) -> bool {
        self.ipc
    }

    /// Whether a cgroup namespace is isolated.
    pub const fn cgroup(&self) -> bool {
        self.cgroup
    }

    /// Whether a time namespace is isolated.
    pub const fn time(&self) -> bool {
        self.time
    }
}

/// The cgroup set one profile admits.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SeccompCgroups {
    #[serde(default)]
    controllers: Vec<BoundedToken>,
}

impl SeccompCgroups {
    /// Construct one cgroup set after checking the controller bound.
    pub fn new(controllers: Vec<BoundedToken>) -> Result<Self, SeccompProfileContractError> {
        if controllers.len() > MAX_SECCOMP_CGROUP_CONTROLLERS {
            return Err(SeccompProfileContractError::TooManyCgroupControllers);
        }
        Ok(Self { controllers })
    }

    /// The admitted cgroup controller names.
    pub fn controllers(&self) -> &[BoundedToken] {
        &self.controllers
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

    /// The device-node path.
    pub const fn path(&self) -> &DeviceNodePath {
        &self.path
    }

    /// The node kind.
    pub const fn kind(&self) -> DeviceNodeKind {
        self.kind
    }

    /// The major device number.
    pub const fn major(&self) -> u32 {
        self.major
    }

    /// The minor device number.
    pub const fn minor(&self) -> u32 {
        self.minor
    }

    /// The granted access class.
    pub const fn access(&self) -> SeccompDeviceAccess {
        self.access
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

    /// The syscall allowlist.
    pub fn syscalls(&self) -> &[BoundedToken] {
        &self.syscalls
    }

    /// The namespace set.
    pub const fn namespaces(&self) -> &SeccompNamespaces {
        &self.namespaces
    }

    /// The cgroup set.
    pub const fn cgroups(&self) -> &SeccompCgroups {
        &self.cgroups
    }

    /// The inline device-node binds.
    pub fn devices(&self) -> &[DeviceBind] {
        &self.devices
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
    /// The profile carries more cgroup controllers than the bound.
    TooManyCgroupControllers,
}

impl core::fmt::Display for SeccompProfileContractError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::InvalidDevicePath => "device bind path is not an absolute /dev path",
            Self::TooManySyscalls => "seccomp profile declares too many syscalls",
            Self::TooManyDeviceBinds => "seccomp profile declares too many device binds",
            Self::TooManyCgroupControllers => {
                "seccomp profile declares too many cgroup controllers"
            }
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
            SeccompNamespaces::new(false, true, true, true, true, true, true, false),
            SeccompCgroups::new(vec![BoundedToken::parse("devices").unwrap()]).unwrap(),
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
    fn a_profile_carries_its_content_inline() {
        let profile = profile();
        assert_eq!(profile.syscalls().len(), 3);
        assert!(profile.namespaces().mount());
        assert!(!profile.namespaces().user());
        assert_eq!(profile.cgroups().controllers().len(), 1);
        assert_eq!(profile.devices()[0].major(), 226);
        assert_eq!(profile.devices()[0].access(), SeccompDeviceAccess::ReadWrite);
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
        let controllers = (0..MAX_SECCOMP_CGROUP_CONTROLLERS + 1)
            .map(|_| BoundedToken::parse("devices").unwrap())
            .collect();
        assert!(SeccompCgroups::new(controllers).is_err());
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
