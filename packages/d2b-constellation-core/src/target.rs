//! Constellation **target address** parser (ADR 0032 §"target address").
//!
//! A target address names an object in the constellation for CLI routing,
//! authz, audit, and display. It is **not** a network address: it does not
//! imply IP reachability, DNS, SSH, a socket path, or a routable overlay.
//!
//! Canonical persisted form:
//!
//! ```text
//! d2b://<workload>.<node>.<realm-path>.d2b
//! ```
//!
//! Human CLI forms (most-specific realm first, DNS-shaped):
//!
//! ```text
//! <workload>                          local workload on this host (v1-compatible)
//! <workload>.d2b                  local workload, current-node form
//! <workload>.<node>.d2b           local-realm workload on a named local node
//! <workload>.<node>.<realm>.d2b   workload in a named realm
//! <workload>.<node>.<child>.<parent>.d2b   workload in a nested realm
//! d2b://<workload>.<node>.<realm-path>.d2b  canonical machine form
//! ```
//!
//! Label-count rules (labels before the reserved `.d2b` suffix):
//!
//! - one label  -> `<workload>.this.local`
//! - two labels -> `<workload>.<node>.local`
//! - three or more -> `<workload>.<node>.<realm-path>`; the first two labels
//!   are always workload and node, every remaining label is part of the
//!   realm path (most-specific realm first).
//!
//! Parsing is **fail-closed**: reserved selectors (`all`, `*`), the reserved
//! suffix word `d2b` in a non-suffix position, malformed labels, and
//! ambiguous multi-label inputs without the `.d2b` suffix are rejected
//! with a typed [`TargetParseError`].

use crate::ids::{IdError, NodeId, RealmId, WorkloadId};
use crate::realm::RealmPath;

/// The reserved target-name suffix.
pub const TARGET_SUFFIX: &str = "d2b";

/// CLI alias for the current host's local node. The parser preserves it
/// verbatim; [`TargetName::with_local_node`] resolves it to a configured
/// [`NodeId`].
pub const THIS_NODE_ALIAS: &str = "this";

/// Why a target address failed to parse. Every variant is fail-closed: an
/// input that does not unambiguously name a single workload is rejected
/// rather than guessed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetParseError {
    /// The input (after stripping `d2b://`) was empty.
    Empty,
    /// A multi-label human form omitted the reserved `.d2b` suffix, which
    /// makes the workload/node/realm split ambiguous (e.g. `build-vm.work`).
    MissingSuffix,
    /// The input was only the reserved suffix (`d2b` / `d2b://d2b`).
    MissingWorkload,
    /// A label was a list-only selector (`all` or `*`), which never names a
    /// single persisted target.
    SelectorNotAllowed,
    /// The reserved suffix word `d2b` appeared in a non-suffix label.
    ReservedLabel,
    /// A workload/node/realm label was malformed (shape or length).
    BadLabel(IdError),
    /// The realm path was empty or exceeded the realm-path bounds.
    BadRealmPath,
}

impl core::fmt::Display for TargetParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TargetParseError::Empty => write!(f, "target address is empty"),
            TargetParseError::MissingSuffix => write!(
                f,
                "multi-label target must end in the reserved `.{TARGET_SUFFIX}` suffix"
            ),
            TargetParseError::MissingWorkload => write!(f, "target address names no workload"),
            TargetParseError::SelectorNotAllowed => {
                write!(
                    f,
                    "`all` and `*` are list-only selectors, not target labels"
                )
            }
            TargetParseError::ReservedLabel => {
                write!(
                    f,
                    "`{TARGET_SUFFIX}` is reserved for the target-name suffix"
                )
            }
            TargetParseError::BadLabel(e) => write!(f, "malformed target label: {e}"),
            TargetParseError::BadRealmPath => write!(f, "realm path is empty or exceeds bounds"),
        }
    }
}

impl std::error::Error for TargetParseError {}

/// A parsed constellation target address: a workload on a node within a
/// realm. The realm defaults to the reserved [`RealmPath::local`] realm when
/// the human form omits it, and the node defaults to the [`THIS_NODE_ALIAS`]
/// (`this`) when omitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetName {
    /// The named workload (VM, session, or sandbox).
    pub workload: WorkloadId,
    /// The node that owns the workload. May be the unresolved `this` alias.
    pub node: NodeId,
    /// The realm containing the node, most-specific realm first.
    pub realm: RealmPath,
}

impl TargetName {
    /// Parse a human or canonical (`d2b://...`) target address, fail-closed.
    ///
    /// The `d2b://` scheme prefix is optional; when present the fully-qualified
    /// `.d2b` suffix is required (it is the canonical machine form).
    pub fn parse(raw: &str) -> Result<Self, TargetParseError> {
        let (body, had_scheme) = match raw.strip_prefix("d2b://") {
            Some(rest) => (rest, true),
            None => (raw, false),
        };
        if body.is_empty() {
            return Err(TargetParseError::Empty);
        }

        let mut labels: Vec<&str> = body.split('.').collect();
        let had_suffix = labels.last() == Some(&TARGET_SUFFIX);
        if had_suffix {
            labels.pop();
            if labels.is_empty() {
                return Err(TargetParseError::MissingWorkload);
            }
        } else if had_scheme || labels.len() != 1 {
            // The canonical `d2b://` form is always fully qualified, and any
            // multi-label human form must carry the `.d2b` suffix so the
            // workload/node/realm split is unambiguous. Only a single bare
            // `<workload>` label may omit the suffix (v1 compatibility).
            return Err(TargetParseError::MissingSuffix);
        }

        // Reject reserved selectors / the suffix word in a non-suffix slot
        // before constructing typed ids so the error is precise.
        for label in &labels {
            match *label {
                "all" | "*" => return Err(TargetParseError::SelectorNotAllowed),
                TARGET_SUFFIX => return Err(TargetParseError::ReservedLabel),
                _ => {}
            }
        }

        let workload = WorkloadId::parse(labels[0]).map_err(TargetParseError::BadLabel)?;
        let node = match labels.get(1) {
            Some(n) => NodeId::parse(*n).map_err(TargetParseError::BadLabel)?,
            None => NodeId::parse(THIS_NODE_ALIAS).expect("`this` is a valid label"),
        };
        let realm = if labels.len() > 2 {
            let realm_labels = labels[2..]
                .iter()
                .map(|l| RealmId::parse(*l).map_err(TargetParseError::BadLabel))
                .collect::<Result<Vec<_>, _>>()?;
            RealmPath::new(realm_labels).ok_or(TargetParseError::BadRealmPath)?
        } else {
            RealmPath::local()
        };

        Ok(TargetName {
            workload,
            node,
            realm,
        })
    }

    /// True if the node is the unresolved `this` alias.
    pub fn node_is_this(&self) -> bool {
        self.node.as_str() == THIS_NODE_ALIAS
    }

    /// Replace the unresolved `this` node alias with the configured local
    /// node. A target whose node is already explicit is returned unchanged.
    pub fn with_local_node(mut self, local: NodeId) -> Self {
        if self.node_is_this() {
            self.node = local;
        }
        self
    }

    /// Render the canonical `d2b://<workload>.<node>.<realm-path>.d2b`
    /// form. The realm path is always rendered explicitly (the local realm is
    /// never elided); `this` is preserved unless already resolved via
    /// [`TargetName::with_local_node`].
    pub fn to_canonical(&self) -> String {
        format!(
            "d2b://{}.{}.{}.{}",
            self.workload,
            self.node,
            self.realm.target_form(),
            TARGET_SUFFIX,
        )
    }
}

impl core::fmt::Display for TargetName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_canonical())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(raw: &str) -> TargetName {
        TargetName::parse(raw).unwrap_or_else(|e| panic!("parse {raw:?}: {e}"))
    }

    #[test]
    fn bare_workload_defaults_to_this_local() {
        let t = parsed("demo");
        assert_eq!(t.workload.as_str(), "demo");
        assert_eq!(t.node.as_str(), "this");
        assert!(t.node_is_this());
        assert_eq!(t.realm, RealmPath::local());
        assert_eq!(t.to_canonical(), "d2b://demo.this.local.d2b");
    }

    #[test]
    fn workload_suffix_form_is_this_local() {
        let t = parsed("demo.d2b");
        assert_eq!(t.workload.as_str(), "demo");
        assert_eq!(t.node.as_str(), "this");
        assert_eq!(t.realm, RealmPath::local());
    }

    #[test]
    fn two_labels_are_workload_and_node_local_realm() {
        // Per ADR: `build-vm.work.d2b` is workload `build-vm` on local
        // node `work`, NOT realm `work`.
        let t = parsed("build-vm.work.d2b");
        assert_eq!(t.workload.as_str(), "build-vm");
        assert_eq!(t.node.as_str(), "work");
        assert_eq!(t.realm, RealmPath::local());
    }

    #[test]
    fn three_labels_name_a_realm() {
        // The canonical ACA target: workload `demo`, node `aca`, realm `work`.
        let t = parsed("demo.aca.work.d2b");
        assert_eq!(t.workload.as_str(), "demo");
        assert_eq!(t.node.as_str(), "aca");
        assert_eq!(t.realm.target_form(), "work");
        assert_eq!(t.realm.storage_form(), "work");
        assert_eq!(t.to_canonical(), "d2b://demo.aca.work.d2b");
    }

    #[test]
    fn nested_realm_path_is_most_specific_first() {
        let t = parsed("api.build.payments.work.d2b");
        assert_eq!(t.workload.as_str(), "api");
        assert_eq!(t.node.as_str(), "build");
        // most-specific-first target form, parent-first storage form.
        assert_eq!(t.realm.target_form(), "payments.work");
        assert_eq!(t.realm.storage_form(), "work/payments");
    }

    #[test]
    fn d2b_scheme_round_trips_to_canonical() {
        let t = parsed("d2b://demo.aca.work.d2b");
        assert_eq!(t.to_canonical(), "d2b://demo.aca.work.d2b");
        // canonical re-parses to the same target.
        assert_eq!(parsed(&t.to_canonical()), t);
    }

    #[test]
    fn with_local_node_resolves_this() {
        let local = NodeId::parse("laptop").unwrap();
        let t = parsed("personal-dev.d2b").with_local_node(local.clone());
        assert_eq!(t.node, local);
        assert!(!t.node_is_this());
        assert_eq!(t.to_canonical(), "d2b://personal-dev.laptop.local.d2b");
        // an explicit node is left untouched.
        let explicit = parsed("demo.aca.work.d2b").with_local_node(local);
        assert_eq!(explicit.node.as_str(), "aca");
    }

    #[test]
    fn multi_label_without_suffix_is_rejected() {
        assert_eq!(
            TargetName::parse("build-vm.work"),
            Err(TargetParseError::MissingSuffix)
        );
        assert_eq!(
            TargetName::parse("d2b://demo.aca.work"),
            Err(TargetParseError::MissingSuffix)
        );
    }

    #[test]
    fn selectors_and_reserved_labels_are_rejected() {
        assert_eq!(
            TargetName::parse("all.d2b"),
            Err(TargetParseError::SelectorNotAllowed)
        );
        assert_eq!(
            TargetName::parse("demo.all.d2b"),
            Err(TargetParseError::SelectorNotAllowed)
        );
        assert_eq!(
            TargetName::parse("*.d2b"),
            Err(TargetParseError::SelectorNotAllowed)
        );
        // `d2b` only valid as the suffix.
        assert_eq!(
            TargetName::parse("d2b.d2b"),
            Err(TargetParseError::ReservedLabel)
        );
    }

    #[test]
    fn empty_and_suffix_only_are_rejected() {
        assert_eq!(TargetName::parse(""), Err(TargetParseError::Empty));
        assert_eq!(TargetName::parse("d2b://"), Err(TargetParseError::Empty));
        assert_eq!(
            TargetName::parse("d2b"),
            Err(TargetParseError::MissingWorkload)
        );
        assert_eq!(
            TargetName::parse(".d2b"),
            Err(TargetParseError::BadLabel(IdError::Empty))
        );
    }

    #[test]
    fn malformed_labels_are_rejected() {
        assert!(matches!(
            TargetName::parse("Demo.d2b"),
            Err(TargetParseError::BadLabel(_))
        ));
        assert!(matches!(
            TargetName::parse("demo.-bad.d2b"),
            Err(TargetParseError::BadLabel(_))
        ));
    }
}
