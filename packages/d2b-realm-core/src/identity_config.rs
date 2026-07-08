//! Metadata-only host config for realm identity lifecycle inputs.
//!
//! This artifact is intentionally inert: it carries only strict realm-core
//! identifiers, opaque refs, fingerprints, and invariants so host daemons can
//! validate declared identity metadata without loading keys or credentials.

use crate::{
    enrollment::{KeyFingerprint, RealmIdentityFingerprint},
    ids::{ControllerGenerationCredentialRef, RealmIdentityRef},
    realm::RealmPath,
    token::ProtocolToken,
};
use serde::{Deserialize, Serialize};

pub const REALM_IDENTITY_CONFIG_SCHEMA_VERSION: &str = "v2";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmIdentityConfigJson {
    pub schema_version: String,
    pub runtime_state: RealmIdentityConfigRuntimeState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub realms: Vec<RealmIdentityConfigEntry>,
    pub invariants: RealmIdentityConfigInvariants,
}

impl RealmIdentityConfigJson {
    pub fn validate_metadata_only(
        &self,
    ) -> Result<RealmIdentityConfigSummary, RealmIdentityConfigError> {
        if self.schema_version != REALM_IDENTITY_CONFIG_SCHEMA_VERSION {
            return Err(RealmIdentityConfigError::UnsupportedSchemaVersion {
                found: self.schema_version.clone(),
            });
        }
        if self.runtime_state != RealmIdentityConfigRuntimeState::MetadataOnly {
            return Err(RealmIdentityConfigError::UnsupportedRuntimeState);
        }
        self.invariants.validate()?;

        let mut summary = RealmIdentityConfigSummary::default();
        for realm in &self.realms {
            summary.realm_count += 1;
            if realm.realm_identity_ref.is_some() {
                summary.identity_ref_count += 1;
            }
            if realm.realm_identity_fingerprint.is_some() {
                summary.identity_fingerprint_count += 1;
            }
            if realm.controller_credential_ref.is_some() {
                summary.controller_credential_ref_count += 1;
            }
            if realm.controller_credential_fingerprint.is_some() {
                summary.controller_credential_fingerprint_count += 1;
            }
        }
        Ok(summary)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RealmIdentityConfigRuntimeState {
    MetadataOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmIdentityConfigEntry {
    pub realm: RealmPath,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realm_identity_ref: Option<RealmIdentityRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub realm_identity_fingerprint: Option<RealmIdentityFingerprint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_credential_ref: Option<ControllerGenerationCredentialRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controller_credential_fingerprint: Option<KeyFingerprint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_bundle_ref: Option<ProtocolToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enrollment_ref: Option<ProtocolToken>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_policy_ref: Option<ProtocolToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmIdentityConfigInvariants {
    pub metadata_only: bool,
    pub no_secret_material: bool,
    pub preserves_runtime_behavior: bool,
}

impl RealmIdentityConfigInvariants {
    fn validate(&self) -> Result<(), RealmIdentityConfigError> {
        if !self.metadata_only {
            return Err(RealmIdentityConfigError::InvariantDisabled("metadataOnly"));
        }
        if !self.no_secret_material {
            return Err(RealmIdentityConfigError::InvariantDisabled(
                "noSecretMaterial",
            ));
        }
        if !self.preserves_runtime_behavior {
            return Err(RealmIdentityConfigError::InvariantDisabled(
                "preservesRuntimeBehavior",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RealmIdentityConfigSummary {
    pub realm_count: usize,
    pub identity_ref_count: usize,
    pub identity_fingerprint_count: usize,
    pub controller_credential_ref_count: usize,
    pub controller_credential_fingerprint_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealmIdentityConfigError {
    UnsupportedSchemaVersion { found: String },
    UnsupportedRuntimeState,
    InvariantDisabled(&'static str),
}

impl core::fmt::Display for RealmIdentityConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnsupportedSchemaVersion { found } => write!(
                f,
                "unsupported realm identity schemaVersion {found:?}; expected {REALM_IDENTITY_CONFIG_SCHEMA_VERSION:?}"
            ),
            Self::UnsupportedRuntimeState => {
                f.write_str("unsupported realm identity runtimeState; expected metadata-only")
            }
            Self::InvariantDisabled(field) => {
                write!(f, "realm identity invariant {field} must be true")
            }
        }
    }
}

impl std::error::Error for RealmIdentityConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn valid_json() -> String {
        format!(
            r#"{{
              "schemaVersion": "v2",
              "runtimeState": "metadata-only",
              "realms": [
                {{
                  "realm": ["work"],
                  "realmIdentityRef": "idref-work",
                  "realmIdentityFingerprint": "{FP}",
                  "controllerCredentialRef": "cgref-work",
                  "controllerCredentialFingerprint": "{FP}",
                  "trustBundleRef": "trust-work",
                  "enrollmentRef": "enroll-work",
                  "rotationPolicyRef": "rotate-work"
                }}
              ],
              "invariants": {{
                "metadataOnly": true,
                "noSecretMaterial": true,
                "preservesRuntimeBehavior": true
              }}
            }}"#
        )
    }

    #[test]
    fn strict_metadata_only_config_parses_and_summarizes() {
        let config: RealmIdentityConfigJson =
            serde_json::from_str(&valid_json()).expect("identity config parses");
        let summary = config
            .validate_metadata_only()
            .expect("identity config validates");
        assert_eq!(summary.realm_count, 1);
        assert_eq!(summary.identity_ref_count, 1);
        assert_eq!(summary.controller_credential_ref_count, 1);
    }

    #[test]
    fn rejects_secret_material_and_secret_shaped_refs() {
        let with_private_key = valid_json().replace(
            r#""rotationPolicyRef": "rotate-work""#,
            r#""rotationPolicyRef": "rotate-work", "privateKey": "nope""#,
        );
        assert!(serde_json::from_str::<RealmIdentityConfigJson>(&with_private_key).is_err());

        let with_secret_ref = valid_json().replace("idref-work", "secret-identity");
        assert!(serde_json::from_str::<RealmIdentityConfigJson>(&with_secret_ref).is_err());
    }

    #[test]
    fn rejects_disabled_invariants() {
        let disabled = valid_json().replace(
            r#""noSecretMaterial": true"#,
            r#""noSecretMaterial": false"#,
        );
        let config: RealmIdentityConfigJson =
            serde_json::from_str(&disabled).expect("shape parses");
        let err = config
            .validate_metadata_only()
            .expect_err("disabled invariant rejects");
        assert_eq!(
            err,
            RealmIdentityConfigError::InvariantDisabled("noSecretMaterial")
        );
    }
}
