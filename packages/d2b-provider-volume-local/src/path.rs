//! Anchored relative-path values used by volume-local effect requests.
//!
//! These types validate names without opening a host path. The core effect
//! adapter owns the retained directory descriptor and resolves these values
//! beneath it.

use std::fmt;

use d2b_contracts::v3::ResourceUid;

/// A rejected anchored path value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathError {
    /// A leaf was empty, reserved, too long, or contained a forbidden byte.
    InvalidLeaf,
    /// A relative path had no component.
    EmptyPath,
}

impl PathError {
    /// Return the stable, path-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidLeaf => "volume-path-leaf-invalid",
            Self::EmptyPath => "volume-path-empty",
        }
    }
}

impl fmt::Display for PathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for PathError {}

/// One validated component of an anchored relative path.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeafName(String);

impl LeafName {
    /// Parse one portable filesystem leaf.
    pub fn parse(value: impl Into<String>) -> Result<Self, PathError> {
        let value = value.into();
        let bytes = value.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 255
            || value == "."
            || value == ".."
            || !bytes
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(PathError::InvalidLeaf);
        }
        Ok(Self(value))
    }

    /// Borrow the validated leaf for an effect adapter request.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for LeafName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LeafName(<redacted>)")
    }
}

/// A non-empty path relative to a retained Volume root descriptor.
#[derive(Clone, PartialEq, Eq)]
pub struct RelativePath {
    components: Vec<LeafName>,
    rendered: String,
}

impl RelativePath {
    /// Build a relative path from independently validated components.
    pub fn from_components(
        components: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, PathError> {
        let components = components
            .into_iter()
            .map(|component| LeafName::parse(component.into()))
            .collect::<Result<Vec<_>, _>>()?;
        if components.is_empty() {
            return Err(PathError::EmptyPath);
        }
        let rendered = components
            .iter()
            .map(LeafName::as_str)
            .collect::<Vec<_>>()
            .join("/");
        Ok(Self {
            components,
            rendered,
        })
    }

    /// Parse a slash-separated relative path.
    pub fn parse(value: &str) -> Result<Self, PathError> {
        if value.starts_with('/') || value.ends_with('/') || value.contains("//") {
            return Err(PathError::InvalidLeaf);
        }
        Self::from_components(value.split('/'))
    }

    /// Borrow the validated representation for an effect adapter request.
    pub fn as_str(&self) -> &str {
        &self.rendered
    }

    /// Borrow the final path component.
    pub fn leaf(&self) -> &LeafName {
        self.components
            .last()
            .expect("the constructor rejects empty component lists")
    }

    /// Borrow every component in root-to-leaf order.
    pub fn components(&self) -> &[LeafName] {
        &self.components
    }
}

impl fmt::Debug for RelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RelativePath(<redacted>)")
    }
}

/// Proof that the effect adapter holds a Volume root directory descriptor.
///
/// This value deliberately exposes neither a descriptor nor a host path.
pub struct AnchoredDir {
    _private: (),
}

impl AnchoredDir {
    /// Issue an opaque held-root proof from an effect adapter.
    pub const fn held() -> Self {
        Self { _private: () }
    }
}

impl fmt::Debug for AnchoredDir {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AnchoredDir(<redacted>)")
    }
}

/// One Volume-owned resource beneath an anchored directory.
pub struct AnchoredResource {
    resource_uid: ResourceUid,
    directory: AnchoredDir,
    leaf: LeafName,
}

impl AnchoredResource {
    /// Bind a resource identity and leaf to an adapter-issued root proof.
    pub const fn new(resource_uid: ResourceUid, directory: AnchoredDir, leaf: LeafName) -> Self {
        Self {
            resource_uid,
            directory,
            leaf,
        }
    }

    /// Borrow the immutable Volume resource identity.
    pub const fn resource_uid(&self) -> &ResourceUid {
        &self.resource_uid
    }

    /// Borrow the adapter-issued root proof.
    pub const fn directory(&self) -> &AnchoredDir {
        &self.directory
    }

    /// Borrow the validated resource leaf.
    pub const fn leaf(&self) -> &LeafName {
        &self.leaf
    }
}

impl fmt::Debug for AnchoredResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AnchoredResource(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_reject_escape_and_separator_forms() {
        for value in [
            "",
            "/state",
            "state/",
            "state//data",
            "../state",
            "state/..",
        ] {
            assert!(RelativePath::parse(value).is_err(), "accepted {value:?}");
        }
        let path = RelativePath::parse("state/public").expect("valid relative path");
        assert_eq!(path.as_str(), "state/public");
        assert_eq!(path.leaf().as_str(), "public");
        assert_eq!(format!("{path:?}"), "RelativePath(<redacted>)");
    }
}
