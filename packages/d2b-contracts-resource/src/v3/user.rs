//! User primitive ResourceType base spec.
//!
//! `User` is the named identity that ACL principals, Process user domains,
//! and Host or Guest `defaultUserRef` fields resolve. The Zone-local resource
//! name and the OS username are separate: `metadata.name` is the canonical
//! Zone-local key, and `spec.osUsername` is the actual username resolved
//! through NSS.
//!
//! The User base spec carries no credential material, public key, PAM
//! configuration, or authentication token of any kind; credentials are
//! `Credential` resources.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::execution_policy::{
    BoundedText, PrimitiveSpecError, parsed_deserialize, redacted_debug, string_schema,
};

/// The canonical ResourceType name for this module.
pub const USER_RESOURCE_TYPE: &str = "User";
/// Maximum bytes in one OS username.
pub const MAX_OS_USERNAME_BYTES: usize = 255;
/// Maximum bytes in one OS group name.
pub const MAX_OS_GROUP_BYTES: usize = 63;
/// Maximum additional group memberships verified for one User.
pub const MAX_USER_GROUPS: usize = 64;

/// A validated OS username presented to NSS.
///
/// The username is validated by host OS username rules rather than the
/// ResourceName grammar, so it may carry an underscore or another character
/// the ResourceName grammar excludes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct OsUsername(String);

impl OsUsername {
    /// Parse a bounded username with no NUL, control, or separator byte.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_OS_USERNAME_BYTES
            || value
                .chars()
                .any(|character| character.is_control() || matches!(character, '/' | '\\'))
        {
            return Err(PrimitiveSpecError::InvalidText);
        }
        Ok(Self(value))
    }

    /// Borrow the username.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(OsUsername);
parsed_deserialize!(OsUsername);
string_schema!(OsUsername, 1, MAX_OS_USERNAME_BYTES);

/// A validated OS group name.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct OsGroupName(String);

impl OsGroupName {
    /// Parse a `^[a-z_][a-z0-9_-]*$` group name bounded to 63 bytes.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_OS_GROUP_BYTES {
            return Err(PrimitiveSpecError::InvalidToken);
        }
        let mut bytes = value.bytes();
        let head_ok = matches!(bytes.next(), Some(b'a'..=b'z' | b'_'));
        let tail_ok = bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        });
        if head_ok && tail_ok {
            Ok(Self(value))
        } else {
            Err(PrimitiveSpecError::InvalidToken)
        }
    }

    /// Borrow the group name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(OsGroupName);
parsed_deserialize!(OsGroupName);
string_schema!(OsGroupName, 1, MAX_OS_GROUP_BYTES);

/// The User ResourceType base spec.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserSpec {
    os_username: OsUsername,
    display_name: BoundedText,
    groups: Vec<OsGroupName>,
}

impl UserSpec {
    /// Construct a User base spec after checking the group bound.
    pub fn new(
        os_username: OsUsername,
        display_name: BoundedText,
        groups: Vec<OsGroupName>,
    ) -> Result<Self, PrimitiveSpecError> {
        if groups.len() > MAX_USER_GROUPS {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        Ok(Self {
            os_username,
            display_name,
            groups,
        })
    }

    /// Construct the canonical minimal User base spec.
    pub fn minimal(os_username: OsUsername) -> Self {
        Self {
            os_username,
            display_name: BoundedText::parse(String::new()).expect("empty text is always valid"),
            groups: Vec::new(),
        }
    }

    /// Borrow the OS username resolved through NSS.
    pub const fn os_username(&self) -> &OsUsername {
        &self.os_username
    }

    /// Borrow the human-readable display name.
    pub const fn display_name(&self) -> &BoundedText {
        &self.display_name
    }

    /// Borrow the verified additional group memberships.
    pub fn groups(&self) -> &[OsGroupName] {
        &self.groups
    }
}

redacted_debug!(UserSpec);

impl<'de> Deserialize<'de> for UserSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            os_username: OsUsername,
            #[serde(default)]
            display_name: Option<BoundedText>,
            #[serde(default)]
            groups: Vec<OsGroupName>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let display_name = match wire.display_name {
            Some(name) => name,
            None => BoundedText::parse(String::new()).map_err(serde::de::Error::custom)?,
        };
        Self::new(wire.os_username, display_name, wire.groups).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::{execution_policy::to_base_object, resource_schema::canonical_json_bytes};

    const MINIMAL_USER_SPEC: &[u8] = br#"{"displayName":"","groups":[],"osUsername":"alice"}"#;

    #[test]
    fn schema_vector_pins_the_minimal_user_base_spec() {
        let spec = UserSpec::minimal(OsUsername::parse("alice").unwrap());
        assert_eq!(canonical_json_bytes(&spec).unwrap(), MINIMAL_USER_SPEC);
        let parsed: UserSpec = serde_json::from_slice(MINIMAL_USER_SPEC).unwrap();
        assert_eq!(parsed, spec);
    }

    #[test]
    fn os_username_uses_host_rules_not_the_resource_name_grammar() {
        assert!(OsUsername::parse("alice_admin").is_ok());
        assert_eq!(
            OsUsername::parse("alice/admin"),
            Err(PrimitiveSpecError::InvalidText)
        );
        assert_eq!(
            OsUsername::parse("alice\u{0000}"),
            Err(PrimitiveSpecError::InvalidText)
        );
        assert_eq!(OsUsername::parse(""), Err(PrimitiveSpecError::InvalidText));
        assert_eq!(
            OsUsername::parse("a".repeat(MAX_OS_USERNAME_BYTES + 1)),
            Err(PrimitiveSpecError::InvalidText)
        );
    }

    #[test]
    fn group_names_and_bounds_fail_closed() {
        assert!(OsGroupName::parse("_wheel").is_ok());
        assert_eq!(
            OsGroupName::parse("Wheel"),
            Err(PrimitiveSpecError::InvalidToken)
        );
        let groups = (0..MAX_USER_GROUPS + 1)
            .map(|_| OsGroupName::parse("wheel").unwrap())
            .collect();
        assert_eq!(
            UserSpec::new(
                OsUsername::parse("alice").unwrap(),
                BoundedText::parse("").unwrap(),
                groups,
            ),
            Err(PrimitiveSpecError::TooManyEntries)
        );
    }

    #[test]
    fn no_credential_or_key_material_is_admitted() {
        for rejected in [
            br#"{"osUsername":"alice","sshPublicKey":"ssh-ed25519 AAAA"}"#.as_slice(),
            br#"{"osUsername":"alice","password":"x"}"#,
            br#"{"osUsername":"alice","uid":1000}"#,
            br#"{"osUsername":"alice","providerRef":"Provider/system-core"}"#,
        ] {
            assert!(serde_json::from_slice::<UserSpec>(rejected).is_err());
        }
        let base = to_base_object(&UserSpec::minimal(OsUsername::parse("alice").unwrap())).unwrap();
        for reserved in ["providerRef", "updatePolicy", "provider"] {
            assert!(base.get(reserved).is_none());
        }
    }

    #[test]
    fn diagnostics_never_echo_the_os_username() {
        let marker = format!("user-{:x}", std::process::id());
        let spec = UserSpec::minimal(OsUsername::parse(marker.clone()).unwrap());
        assert!(!format!("{spec:?}").contains(&marker));
        assert!(!format!("{:?}", spec.os_username()).contains(&marker));
        assert_eq!(spec.os_username().as_str(), marker);
    }
}
