//! Realm model (ADR 0032). A realm has an entrypoint mode and a
//! DNS-style path written most-specific-first.

use crate::ids::RealmId;
use serde::{Deserialize, Serialize};

/// Where a realm's entrypoint runs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum EntrypointMode {
    /// Local-only / trusted-host realm dispatched on the host `nixlingd`.
    HostResident,
    /// Realm fronted by a dedicated local gateway guest VM.
    GatewayBacked,
}

/// A realm path: an ordered list of labels written most-specific realm
/// first (e.g. `payments.work` for child `payments` of parent `work`).
/// Internally policy may store it parent-first as `work/payments`; the
/// target-name form stays DNS-shaped.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
    schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct RealmPath(Vec<RealmId>);

impl RealmPath {
    /// Build from most-specific-first labels. Empty paths are rejected.
    pub fn new(labels: Vec<RealmId>) -> Option<Self> {
        if labels.is_empty() {
            None
        } else {
            Some(Self(labels))
        }
    }

    /// The reserved local realm (`local`).
    pub fn local() -> Self {
        Self(vec![RealmId::parse("local").expect("`local` is a valid label")])
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
}
