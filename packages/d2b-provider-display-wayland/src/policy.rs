//! Closed Wayland global and dmabuf policy compilation.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// Compiled interface catalog used for fail-closed policy validation.
pub const KNOWN_GLOBALS: &[&str] = &[
    "wl_compositor",
    "wl_shm",
    "wl_seat",
    "wl_output",
    "wl_subcompositor",
    "xdg_wm_base",
    "wl_data_device_manager",
    "zwlr_data_control_manager_v1",
    "zwp_primary_selection_device_manager_v1",
    "zwp_linux_dmabuf_v1",
    "zwp_pointer_constraints_v1",
    "zwp_relative_pointer_manager_v1",
    "zwlr_layer_shell_v1",
    "wp_drm_lease_device_v1",
    "zwp_virtual_keyboard_manager_v1",
];

const REQUIRED_GLOBALS: &[&str] = &["wl_compositor", "wl_shm", "wl_seat", "xdg_wm_base"];
const CLIPBOARD_GLOBALS: &[&str] = &[
    "wl_data_device_manager",
    "zwlr_data_control_manager_v1",
    "zwp_primary_selection_device_manager_v1",
];

/// Policy compilation warnings with a closed semantic vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyWarning {
    /// An allow entry attempted to enable clipboard-manager globals.
    ClipboardBoundaryIgnored,
    /// An unclassified global was explicitly allowed.
    UnclassifiedGlobalAllowed,
}

/// Policy input layer.
#[derive(Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterInput {
    allow_globals: Vec<String>,
    deny_globals: Vec<String>,
    max_versions: BTreeMap<String, u32>,
    dmabuf_allow: Vec<String>,
    dmabuf_deny: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FilterInputWire {
    allow_globals: Vec<String>,
    deny_globals: Vec<String>,
    max_versions: BTreeMap<String, u32>,
    dmabuf_allow: Vec<String>,
    dmabuf_deny: Vec<String>,
}

impl TryFrom<FilterInputWire> for FilterInput {
    type Error = PolicyCompileError;

    fn try_from(value: FilterInputWire) -> Result<Self, Self::Error> {
        let filter = Self {
            allow_globals: value.allow_globals,
            deny_globals: value.deny_globals,
            max_versions: value.max_versions,
            dmabuf_allow: value.dmabuf_allow,
            dmabuf_deny: value.dmabuf_deny,
        };
        filter.validate_bounds()?;
        Ok(filter)
    }
}

impl FilterInput {
    /// Construct a bounded policy layer.
    pub fn new<A, D, V, M>(
        allow_globals: A,
        deny_globals: D,
        max_versions: V,
        dmabuf_allow: M,
    ) -> Result<Self, PolicyCompileError>
    where
        A: IntoIterator,
        A::Item: Into<String>,
        D: IntoIterator,
        D::Item: Into<String>,
        V: IntoIterator<Item = (String, u32)>,
        M: IntoIterator,
        M::Item: Into<String>,
    {
        let value = Self {
            allow_globals: allow_globals.into_iter().map(Into::into).collect(),
            deny_globals: deny_globals.into_iter().map(Into::into).collect(),
            max_versions: max_versions.into_iter().collect(),
            dmabuf_allow: dmabuf_allow.into_iter().map(Into::into).collect(),
            dmabuf_deny: Vec::new(),
        };
        value.validate_bounds()?;
        Ok(value)
    }

    /// Add an allowed global to this layer.
    pub fn allow_globals(&self) -> &[String] {
        &self.allow_globals
    }

    /// Add a denied global to this layer.
    pub fn deny_globals(&self) -> &[String] {
        &self.deny_globals
    }

    /// Borrow version caps.
    pub const fn max_versions(&self) -> &BTreeMap<String, u32> {
        &self.max_versions
    }

    /// Borrow allowed dmabuf rules.
    pub fn dmabuf_allow(&self) -> &[String] {
        &self.dmabuf_allow
    }

    /// Borrow denied dmabuf rules.
    pub fn dmabuf_deny(&self) -> &[String] {
        &self.dmabuf_deny
    }

    fn validate_bounds(&self) -> Result<(), PolicyCompileError> {
        if self.allow_globals.len() > 128
            || self.deny_globals.len() > 128
            || self.max_versions.len() > 128
            || self.dmabuf_allow.len() > 64
            || self.dmabuf_deny.len() > 64
            || self
                .allow_globals
                .iter()
                .chain(&self.deny_globals)
                .any(|value| value.len() > 63)
            || self
                .dmabuf_allow
                .iter()
                .chain(&self.dmabuf_deny)
                .any(|value| value.chars().count() > 128)
        {
            return Err(PolicyCompileError::BoundsExceeded);
        }
        Ok(())
    }
}

impl core::fmt::Debug for FilterInput {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("FilterInput")
            .field("allow_count", &self.allow_globals.len())
            .field("deny_count", &self.deny_globals.len())
            .field("version_count", &self.max_versions.len())
            .finish()
    }
}

/// Closed policy compilation errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyCompileError {
    /// The layer exceeded a fixed count or byte bound.
    BoundsExceeded,
    /// The policy named a global that is not in the compiled catalog.
    UnknownInterface(String),
}

impl core::fmt::Display for PolicyCompileError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::BoundsExceeded => "wayland-policy-bounds-exceeded",
            Self::UnknownInterface(_) => "unknown-interface-rejected",
        })
    }
}

impl std::error::Error for PolicyCompileError {}

impl<'de> Deserialize<'de> for FilterInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = FilterInputWire::deserialize(deserializer)?;
        Self::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// Compiled, immutable Wayland policy.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CompiledWaylandPolicy {
    allowed_globals: BTreeSet<String>,
    denied_globals: BTreeSet<String>,
    max_versions: BTreeMap<String, u32>,
    dmabuf_allowed: BTreeSet<String>,
    dmabuf_denied: BTreeSet<String>,
    warnings: Vec<PolicyWarning>,
    digest: String,
}

impl CompiledWaylandPolicy {
    /// Return the policy digest used in sealed launch configuration.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Check whether a global is admitted.
    pub fn is_allowed(&self, global: &str) -> bool {
        self.allowed_globals.contains(global) && !self.denied_globals.contains(global)
    }

    /// Borrow bounded compile warnings.
    pub fn warnings(&self) -> &[PolicyWarning] {
        &self.warnings
    }

    /// Return a configured version cap.
    pub fn max_version(&self, interface: &str) -> Option<u32> {
        self.max_versions.get(interface).copied()
    }

    /// Borrow the effective dmabuf allow rules.
    pub fn dmabuf_allowed(&self) -> &BTreeSet<String> {
        &self.dmabuf_allowed
    }

    /// Borrow the effective dmabuf deny rules.
    pub fn dmabuf_denied(&self) -> &BTreeSet<String> {
        &self.dmabuf_denied
    }

    /// Check whether a dmabuf rule is admitted.
    pub fn is_dmabuf_allowed(&self, rule: &str) -> bool {
        self.dmabuf_allowed.contains(rule) && !self.dmabuf_denied.contains(rule)
    }
}

/// The policy compiler used by display-controller.
pub struct WaylandPolicy;

impl WaylandPolicy {
    /// Compile defaults, Zone policy, and session overrides in that order.
    pub fn compile(
        defaults: &FilterInput,
        zone: &FilterInput,
        session: &FilterInput,
    ) -> Result<CompiledWaylandPolicy, PolicyCompileError> {
        for layer in [defaults, zone, session] {
            validate_layer(layer)?;
        }
        let mut allowed_globals = REQUIRED_GLOBALS
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<BTreeSet<_>>();
        let mut denied_globals = BTreeSet::new();
        let mut warnings = Vec::new();
        let mut max_versions = BTreeMap::new();
        let mut dmabuf_allowed = BTreeSet::new();
        let mut dmabuf_denied = BTreeSet::new();
        for layer in [defaults, zone, session] {
            for global in layer.allow_globals() {
                if CLIPBOARD_GLOBALS.contains(&global.as_str()) {
                    warnings.push(PolicyWarning::ClipboardBoundaryIgnored);
                } else if !REQUIRED_GLOBALS.contains(&global.as_str()) {
                    allowed_globals.insert(global.clone());
                    denied_globals.remove(global);
                    if !KNOWN_GLOBALS.contains(&global.as_str()) {
                        warnings.push(PolicyWarning::UnclassifiedGlobalAllowed);
                    }
                }
            }
            for global in layer.deny_globals() {
                if !REQUIRED_GLOBALS.contains(&global.as_str()) {
                    denied_globals.insert(global.clone());
                    allowed_globals.remove(global);
                }
            }
            max_versions.extend(
                layer
                    .max_versions()
                    .iter()
                    .map(|(key, value)| (key.clone(), *value)),
            );
            for rule in layer.dmabuf_allow() {
                dmabuf_allowed.insert(rule.clone());
                dmabuf_denied.remove(rule);
            }
            for rule in layer.dmabuf_deny() {
                dmabuf_denied.insert(rule.clone());
                dmabuf_allowed.remove(rule);
            }
        }
        for required in REQUIRED_GLOBALS {
            denied_globals.remove(*required);
            allowed_globals.insert((*required).to_owned());
        }
        warnings.sort_unstable_by_key(|warning| *warning as u8);
        warnings.dedup();
        let digest = digest(
            &allowed_globals,
            &denied_globals,
            &max_versions,
            &dmabuf_allowed,
            &dmabuf_denied,
        );
        Ok(CompiledWaylandPolicy {
            allowed_globals,
            denied_globals,
            max_versions,
            dmabuf_allowed,
            dmabuf_denied,
            warnings,
            digest,
        })
    }
}

impl FilterInput {
    /// Add bounded dmabuf deny rules to this policy layer.
    pub fn with_dmabuf_deny(
        mut self,
        rules: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, PolicyCompileError> {
        self.dmabuf_deny = rules.into_iter().map(Into::into).collect();
        self.validate_bounds()?;
        Ok(self)
    }
}

fn validate_layer(layer: &FilterInput) -> Result<(), PolicyCompileError> {
    for global in layer.allow_globals().iter().chain(layer.deny_globals()) {
        if !KNOWN_GLOBALS.contains(&global.as_str()) {
            return Err(PolicyCompileError::UnknownInterface(global.clone()));
        }
    }
    if layer
        .dmabuf_allow()
        .iter()
        .chain(layer.dmabuf_deny())
        .any(|rule| rule.chars().count() > 128)
    {
        return Err(PolicyCompileError::BoundsExceeded);
    }
    Ok(())
}

fn digest(
    allowed: &BTreeSet<String>,
    denied: &BTreeSet<String>,
    versions: &BTreeMap<String, u32>,
    dmabuf_allowed: &BTreeSet<String>,
    dmabuf_denied: &BTreeSet<String>,
) -> String {
    let mut hasher = Sha256::new();
    for value in allowed {
        hasher.update(b"allow\0");
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    for value in denied {
        hasher.update(b"deny\0");
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    for (key, value) in versions {
        hasher.update(b"version\0");
        hasher.update(key.as_bytes());
        hasher.update([0]);
        hasher.update(value.to_le_bytes());
    }
    for value in dmabuf_allowed {
        hasher.update(b"dmabuf-allow\0");
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    for value in dmabuf_denied {
        hasher.update(b"dmabuf-deny\0");
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    format!("sha256:{:x}", hasher.finalize())
}
