//! Enrollment and key-lifecycle metadata for realms.
//!
//! These DTOs carry only fingerprints, ids, timestamps, and status metadata.
//! They never carry private keys, public key bytes, provider credentials, or
//! signed credential material.

use crate::ids::{ControllerGenerationId, CorrelationId, EnrollmentId, RealmId, RevocationId};
use crate::realm::RealmPath;
use crate::token::ProtocolToken;
use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};

const FINGERPRINT_PREFIX: &str = "sha256:";
const FINGERPRINT_HEX_LEN: usize = 64;

/// Bounded cryptographic fingerprint. This is metadata, not key material.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct KeyFingerprint(String);

impl KeyFingerprint {
    /// Parse `sha256:<64 lowercase hex chars>`.
    pub fn parse(raw: impl Into<String>) -> Option<Self> {
        let raw = raw.into();
        let hex = raw.strip_prefix(FINGERPRINT_PREFIX)?;
        if hex.len() == FINGERPRINT_HEX_LEN
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'))
        {
            Some(Self(raw))
        } else {
            None
        }
    }

    /// Borrow the fingerprint.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for KeyFingerprint {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("KeyFingerprint(<sha256>)")
    }
}

impl<'de> Deserialize<'de> for KeyFingerprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("invalid key fingerprint"))
    }
}

impl JsonSchema for KeyFingerprint {
    fn schema_name() -> String {
        "KeyFingerprint".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                min_length: Some((FINGERPRINT_PREFIX.len() + FINGERPRINT_HEX_LEN) as u32),
                max_length: Some((FINGERPRINT_PREFIX.len() + FINGERPRINT_HEX_LEN) as u32),
                pattern: Some("^sha256:[0-9a-f]{64}$".to_owned()),
            })),
            ..Default::default()
        })
    }
}

/// Role of a pinned realm key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RealmKeyRole {
    /// Long-lived realm identity key.
    RealmIdentity,
    /// Parent realm trust anchor pinned into a child.
    ParentTrustAnchor,
    /// Child realm identity observed and pinned by a parent.
    ChildIdentity,
    /// Controller-generation signing key.
    ControllerGeneration,
}

/// Parent/child key pinning metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyPin {
    /// Realm whose key is pinned.
    pub realm: RealmPath,
    /// Parent or child label at the trust edge.
    pub peer: RealmId,
    /// Key role represented by this fingerprint.
    pub role: RealmKeyRole,
    /// Fingerprint only; no key bytes.
    pub fingerprint: KeyFingerprint,
    /// Controller generation that produced or accepted this pin.
    pub controller_generation: ControllerGenerationId,
    /// Issuance/observation time as Unix seconds.
    pub pinned_at_unix_seconds: u64,
}

/// Enrollment lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EnrollmentStatus {
    Pending,
    Accepted,
    Rejected,
    Superseded,
    Revoked,
}

/// Metadata-only enrollment record for a child realm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnrollmentRecord {
    /// Enrollment record id.
    pub enrollment_id: EnrollmentId,
    /// Parent realm that admits the child.
    pub parent_realm: RealmPath,
    /// Child realm being enrolled.
    pub child_realm: RealmPath,
    /// Active controller generation for this enrollment.
    pub controller_generation: ControllerGenerationId,
    /// Parent trust-anchor pin metadata.
    pub parent_key_pin: KeyPin,
    /// Child identity pin metadata.
    pub child_key_pin: KeyPin,
    /// Current status.
    pub status: EnrollmentStatus,
    /// Bounded provider/bootstrap method token.
    pub bootstrap_method: ProtocolToken,
    /// Enrollment creation time.
    pub created_at_unix_seconds: u64,
    /// Last status update time.
    pub updated_at_unix_seconds: u64,
    /// Correlation id for audit across parent/child records.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub correlation_id: Option<CorrelationId>,
}

/// Thing revoked by a parent or realm controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RevocationTarget {
    RealmKey {
        realm: RealmPath,
        role: RealmKeyRole,
        fingerprint: KeyFingerprint,
    },
    ControllerGeneration {
        realm: RealmPath,
        controller_generation: ControllerGenerationId,
    },
    Enrollment {
        enrollment_id: EnrollmentId,
    },
}

/// Revocation lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RevocationStatus {
    Pending,
    Effective,
    Propagated,
    Superseded,
}

/// Metadata-only revocation record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RevocationRecord {
    /// Revocation record id.
    pub revocation_id: RevocationId,
    /// Realm issuing the revocation.
    pub issuer_realm: RealmPath,
    /// Issuer controller generation.
    pub issuer_controller_generation: ControllerGenerationId,
    /// Target metadata.
    pub target: RevocationTarget,
    /// Current status.
    pub status: RevocationStatus,
    /// Low-cardinality reason code.
    pub reason: ProtocolToken,
    /// Issue time.
    pub issued_at_unix_seconds: u64,
    /// Optional effective time.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub effective_at_unix_seconds: Option<u64>,
    /// Cross-realm audit correlation id.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub correlation_id: Option<CorrelationId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::RealmId;
    use schemars::schema_for;

    fn fp() -> KeyFingerprint {
        KeyFingerprint::parse(format!("sha256:{}", "a".repeat(64))).unwrap()
    }

    fn realm(label: &str) -> RealmPath {
        RealmPath::new(vec![RealmId::parse(label).unwrap()]).unwrap()
    }

    fn pin(role: RealmKeyRole) -> KeyPin {
        KeyPin {
            realm: realm("work"),
            peer: RealmId::parse("local-root").unwrap(),
            role,
            fingerprint: fp(),
            controller_generation: ControllerGenerationId::parse("gen-1").unwrap(),
            pinned_at_unix_seconds: 10,
        }
    }

    #[test]
    fn key_fingerprint_rejects_key_material_shapes() {
        assert!(KeyFingerprint::parse(format!("sha256:{}", "a".repeat(64))).is_some());
        assert!(KeyFingerprint::parse("-----BEGIN PUBLIC KEY-----").is_none());
        assert!(KeyFingerprint::parse(format!("sha256:{}", "A".repeat(64))).is_none());
        assert_eq!(format!("{:?}", fp()), "KeyFingerprint(<sha256>)");
    }

    #[test]
    fn enrollment_record_rejects_unknown_fields() {
        let record = EnrollmentRecord {
            enrollment_id: EnrollmentId::parse("enroll-1").unwrap(),
            parent_realm: realm("local-root"),
            child_realm: realm("work"),
            controller_generation: ControllerGenerationId::parse("gen-1").unwrap(),
            parent_key_pin: pin(RealmKeyRole::ParentTrustAnchor),
            child_key_pin: pin(RealmKeyRole::ChildIdentity),
            status: EnrollmentStatus::Accepted,
            bootstrap_method: ProtocolToken::parse("host-local").unwrap(),
            created_at_unix_seconds: 10,
            updated_at_unix_seconds: 11,
            correlation_id: Some(CorrelationId::parse("corr-1").unwrap()),
        };
        let mut value = serde_json::to_value(&record).unwrap();
        value.as_object_mut().unwrap().insert(
            "privateKey".to_owned(),
            serde_json::Value::String("must-not-appear".to_owned()),
        );
        assert!(serde_json::from_value::<EnrollmentRecord>(value).is_err());
    }

    #[test]
    fn revocation_record_carries_metadata_only() {
        let record = RevocationRecord {
            revocation_id: RevocationId::parse("rev-1").unwrap(),
            issuer_realm: realm("local-root"),
            issuer_controller_generation: ControllerGenerationId::parse("gen-2").unwrap(),
            target: RevocationTarget::ControllerGeneration {
                realm: realm("work"),
                controller_generation: ControllerGenerationId::parse("gen-1").unwrap(),
            },
            status: RevocationStatus::Effective,
            reason: ProtocolToken::parse("controller-compromised").unwrap(),
            issued_at_unix_seconds: 20,
            effective_at_unix_seconds: Some(21),
            correlation_id: None,
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(!json.contains("private"));
        assert!(serde_json::from_str::<RevocationRecord>(&json).is_ok());
    }

    #[test]
    fn enrollment_schemas_are_generated() {
        assert!(schema_for!(EnrollmentRecord).schema.metadata.is_some());
        assert!(schema_for!(RevocationRecord).schema.metadata.is_some());
    }
}
