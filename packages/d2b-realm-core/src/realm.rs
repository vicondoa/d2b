//! Realm model (ADR 0032). A realm has an entrypoint mode and a
//! DNS-style path written most-specific-first.

use crate::ids::RealmId;
use serde::{Deserialize, Deserializer, Serialize};

/// Where a realm's entrypoint runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum EntrypointMode {
    /// Local-only / trusted-host realm dispatched on the host `d2bd`.
    HostResident,
    /// Realm fronted by a dedicated local gateway guest VM.
    GatewayBacked,
}

/// A realm path: an ordered list of labels written most-specific realm
/// first (e.g. `payments.work` for child `payments` of parent `work`).
/// Internally policy may store it parent-first as `work/payments`; the
/// target-name form stays DNS-shaped. Bounded in label count and total
/// rendered length so it cannot become an unbounded side channel.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, schemars::JsonSchema)]
#[serde(transparent)]
#[schemars(
    length(min = 1, max = 16),
    description = "Most-specific-first realm labels. Bounded to 16 labels and a 255-byte total rendered (dotted) length, enforced at construction and decode."
)]
pub struct RealmPath(Vec<RealmId>);

/// Maximum number of labels in a realm path.
pub const MAX_REALM_LABELS: usize = 16;
/// Maximum total bytes of a realm path's rendered target form.
pub const MAX_REALM_PATH_BYTES: usize = 255;

// Fail-closed decode: the non-empty + bound invariants are enforced on the
// wire, not just in the constructor.
impl<'de> Deserialize<'de> for RealmPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let labels = Vec::<RealmId>::deserialize(deserializer)?;
        Self::new(labels)
            .ok_or_else(|| serde::de::Error::custom("realm path is empty or exceeds bounds"))
    }
}

impl RealmPath {
    /// Build from most-specific-first labels. Empty paths, paths with more
    /// than [`MAX_REALM_LABELS`] labels, and paths whose target form exceeds
    /// [`MAX_REALM_PATH_BYTES`] are rejected (fail-closed).
    pub fn new(labels: Vec<RealmId>) -> Option<Self> {
        if labels.is_empty() || labels.len() > MAX_REALM_LABELS {
            return None;
        }
        // total bytes of the dotted target form (labels + separators).
        let total: usize =
            labels.iter().map(|l| l.as_str().len()).sum::<usize>() + labels.len().saturating_sub(1);
        if total > MAX_REALM_PATH_BYTES {
            return None;
        }
        Some(Self(labels))
    }

    /// The reserved local realm (`local`).
    pub fn local() -> Self {
        Self(vec![
            RealmId::parse("local").expect("`local` is a valid label"),
        ])
    }

    /// Labels, most-specific first.
    pub fn labels(&self) -> &[RealmId] {
        &self.0
    }

    /// Canonical parent-first storage form (e.g. `work/payments`).
    pub fn storage_form(&self) -> String {
        self.0
            .iter()
            .rev()
            .map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join("/")
    }

    /// DNS-shaped target form (e.g. `payments.work`).
    pub fn target_form(&self) -> String {
        self.0
            .iter()
            .map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join(".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn realm_path_forms() {
        let p = RealmPath::new(vec![
            RealmId::parse("payments").unwrap(),
            RealmId::parse("work").unwrap(),
        ])
        .unwrap();
        assert_eq!(p.target_form(), "payments.work");
        assert_eq!(p.storage_form(), "work/payments");
        assert!(RealmPath::new(vec![]).is_none());
    }

    #[test]
    fn realm_path_deserialize_rejects_empty() {
        assert!(serde_json::from_str::<RealmPath>("[\"work\"]").is_ok());
        assert!(serde_json::from_str::<RealmPath>("[]").is_err());
        // a malformed inner label is rejected too (RealmId is fail-closed).
        assert!(serde_json::from_str::<RealmPath>("[\"Work\"]").is_err());
    }

    #[test]
    fn realm_path_rejects_too_many_labels() {
        let many: Vec<RealmId> = (0..MAX_REALM_LABELS + 1)
            .map(|i| RealmId::parse(format!("r{i}")).unwrap())
            .collect();
        assert!(RealmPath::new(many).is_none());
        let ok: Vec<RealmId> = (0..MAX_REALM_LABELS)
            .map(|i| RealmId::parse(format!("r{i}")).unwrap())
            .collect();
        assert!(RealmPath::new(ok).is_some());
    }

    #[test]
    fn realm_path_rejects_over_byte_cap() {
        // Two 128-byte labels render to 257 bytes (> MAX_REALM_PATH_BYTES),
        // rejected at construction AND at decode.
        let long = "a".repeat(128);
        let labels = vec![
            RealmId::parse(long.clone()).unwrap(),
            RealmId::parse(long.clone()).unwrap(),
        ];
        assert!(RealmPath::new(labels).is_none());
        let json = format!("[\"{long}\",\"{long}\"]");
        assert!(serde_json::from_str::<RealmPath>(&json).is_err());
    }
}
