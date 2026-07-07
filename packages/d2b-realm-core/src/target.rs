//! Realm-native target address parser (ADR 0043).
//!
//! A public target address names a workload inside a realm, not a physical
//! node:
//!
//! ```text
//! <workload>.<realm>[.<ancestor>...].d2b
//! ```
//!
//! The canonical rendered form is DNS-shaped and has no scheme, for example
//! `builder.dev.d2b` or `api.payments.work.d2b`. Bare workload names are not
//! self-contained targets; they require an explicit default realm or alias table
//! supplied by the caller. Old ADR 0032 node-qualified forms are retained only
//! as typed migration diagnostics.

use std::collections::{BTreeMap, BTreeSet};

use crate::ids::{IdError, NodeId, RealmId, WorkloadId};
use crate::realm::RealmPath;
use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The reserved target-name suffix.
pub const TARGET_SUFFIX: &str = "d2b";

/// Legacy ADR 0032 CLI alias for the current host's local node.
///
/// ADR 0043 realm targets do not encode node labels. This constant remains only
/// for migration diagnostics and older callers while they move to
/// [`RealmTarget`].
pub const THIS_NODE_ALIAS: &str = "this";

/// A parsed ADR 0043 realm target: a workload inside a realm path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RealmTarget {
    /// The named workload (VM, session, or provider-backed workload).
    pub workload: WorkloadId,
    /// The realm path, most-specific realm first.
    pub realm: RealmPath,
}

impl RealmTarget {
    /// Construct a typed target from already-validated components.
    pub fn new(workload: WorkloadId, realm: RealmPath) -> Self {
        Self { workload, realm }
    }

    /// Parse a fully-qualified ADR 0043 target.
    ///
    /// Bare workload aliases require resolver context; use
    /// [`RealmTargetParser`] with a default realm or alias table when a caller
    /// intentionally supports those convenience forms.
    pub fn parse(raw: &str) -> Result<Self, RealmTargetParseError> {
        parse_realm_target(raw)
    }

    /// Render the canonical ADR 0043 target address.
    pub fn to_canonical(&self) -> String {
        format!(
            "{}.{}.{}",
            self.workload,
            self.realm.target_form(),
            TARGET_SUFFIX
        )
    }

    /// Compatibility shim for older callers. ADR 0043 targets never carry a
    /// node label, so this is always false.
    pub fn node_is_this(&self) -> bool {
        false
    }

    /// Compatibility shim for older callers. ADR 0043 targets never carry a
    /// node label, so the target is returned unchanged.
    pub fn with_local_node(self, _local: NodeId) -> Self {
        self
    }
}

impl core::fmt::Display for RealmTarget {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_canonical())
    }
}

impl Serialize for RealmTarget {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_canonical())
    }
}

impl<'de> Deserialize<'de> for RealmTarget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for RealmTarget {
    fn schema_name() -> String {
        "RealmTarget".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        target_string_schema()
    }
}

/// Temporary compatibility wrapper for older call sites. New code should use
/// [`RealmTarget`] so node labels do not re-enter normal routing paths.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetName {
    target: RealmTarget,
    /// Compatibility field for ADR 0032 callers. ADR 0043 targets never encode
    /// a node, so parsed targets carry the unresolved `this` alias until callers
    /// explicitly resolve it for legacy-local behavior.
    pub node: NodeId,
}

impl TargetName {
    /// Parse an ADR 0043 target address while preserving legacy field access.
    pub fn parse(raw: &str) -> Result<Self, TargetParseError> {
        RealmTarget::parse(raw).map(Self::from_realm_target)
    }

    /// Wrap a realm target for older APIs that still traffic in `TargetName`.
    pub fn from_realm_target(target: RealmTarget) -> Self {
        Self {
            target,
            node: NodeId::parse(THIS_NODE_ALIAS).expect("`this` is a valid label"),
        }
    }

    /// Borrow the ADR 0043 target.
    pub fn as_realm_target(&self) -> &RealmTarget {
        &self.target
    }

    /// Return the ADR 0043 target.
    pub fn into_realm_target(self) -> RealmTarget {
        self.target
    }

    /// True if the compatibility node field is still the unresolved `this` alias.
    pub fn node_is_this(&self) -> bool {
        self.node.as_str() == THIS_NODE_ALIAS
    }

    /// Resolve the compatibility `this` node field for older local-only callers.
    /// The node is not part of canonical ADR 0043 routing.
    pub fn with_local_node(mut self, local: NodeId) -> Self {
        if self.node_is_this() {
            self.node = local;
        }
        self
    }

    /// Render the canonical ADR 0043 target address.
    pub fn to_canonical(&self) -> String {
        self.target.to_canonical()
    }
}

impl core::ops::Deref for TargetName {
    type Target = RealmTarget;

    fn deref(&self) -> &Self::Target {
        &self.target
    }
}

impl From<RealmTarget> for TargetName {
    fn from(target: RealmTarget) -> Self {
        Self::from_realm_target(target)
    }
}

impl From<TargetName> for RealmTarget {
    fn from(target: TargetName) -> Self {
        target.into_realm_target()
    }
}

impl core::fmt::Display for TargetName {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.to_canonical())
    }
}

impl Serialize for TargetName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.target.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TargetName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        RealmTarget::deserialize(deserializer).map(Self::from_realm_target)
    }
}

impl JsonSchema for TargetName {
    fn schema_name() -> String {
        "TargetName".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        target_string_schema()
    }
}

fn target_string_schema() -> Schema {
    Schema::Object(SchemaObject {
        instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
        string: Some(Box::new(StringValidation {
            min_length: Some(1),
            max_length: Some(388),
            pattern: Some("^[a-z][a-z0-9-]*(\\.[a-z][a-z0-9-]*)+\\.d2b$".to_owned()),
        })),
        ..Default::default()
    })
}

/// A parsed old ADR 0032 node-qualified target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyNodeQualifiedTarget {
    /// The named workload.
    pub workload: WorkloadId,
    /// The legacy physical/provider node label that must be removed from the
    /// public target.
    pub node: NodeId,
    /// The realm path that followed the node label.
    pub realm: RealmPath,
}

impl LegacyNodeQualifiedTarget {
    /// Parse an old ADR 0032 node-qualified target for migration diagnostics.
    /// This helper must not be used for normal routing.
    pub fn parse(raw: &str) -> Result<Self, RealmTargetParseError> {
        parse_legacy_node_qualified(raw)
    }

    /// Return the ADR 0043 target obtained by dropping the legacy node label.
    pub fn suggested_realm_target(&self) -> RealmTarget {
        RealmTarget::new(self.workload.clone(), self.realm.clone())
    }

    /// Render the legacy diagnostic form without treating it as canonical.
    pub fn diagnostic_form(&self) -> String {
        format!(
            "{}.{}.{}.{}",
            self.workload,
            self.node,
            self.realm.target_form(),
            TARGET_SUFFIX
        )
    }
}

/// Why a realm target failed to parse. Every variant is fail-closed: an input
/// that does not unambiguously name a single workload is rejected rather than
/// guessed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealmTargetParseError {
    /// The input (after stripping an old `d2b://` prefix) was empty.
    Empty,
    /// A bare workload alias was supplied without resolver context.
    BareAliasRequiresContext,
    /// A multi-label human form omitted the reserved `.d2b` suffix.
    MissingSuffix,
    /// The input was only the reserved suffix (`d2b` / `d2b://d2b`).
    MissingWorkload,
    /// A fully-qualified target omitted the required realm path.
    MissingRealm,
    /// A label was a list-only selector (`all` or `*`), which never names a
    /// single persisted target.
    SelectorNotAllowed,
    /// The reserved suffix word `d2b` appeared in a non-suffix label.
    ReservedLabel,
    /// A workload/node/realm label was malformed (shape or length).
    BadLabel(IdError),
    /// The realm path was empty or exceeded the realm-path bounds.
    BadRealmPath,
    /// A bare convenience alias matched more than one local target.
    AliasAmbiguous {
        alias: WorkloadId,
        candidates: Vec<RealmTarget>,
    },
    /// The address matches an old ADR 0032 node-qualified shape for a known
    /// legacy node label. It is reported as a migration diagnostic, not accepted
    /// as a normal ADR 0043 route.
    LegacyNodeQualified {
        legacy: LegacyNodeQualifiedTarget,
        suggested: RealmTarget,
    },
}

/// Temporary compatibility name for older call sites.
pub type TargetParseError = RealmTargetParseError;

impl core::fmt::Display for RealmTargetParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RealmTargetParseError::Empty => write!(f, "target address is empty"),
            RealmTargetParseError::BareAliasRequiresContext => write!(
                f,
                "bare workload aliases require an explicit default realm or alias table"
            ),
            RealmTargetParseError::MissingSuffix => write!(
                f,
                "multi-label target must end in the reserved `.{TARGET_SUFFIX}` suffix"
            ),
            RealmTargetParseError::MissingWorkload => write!(f, "target address names no workload"),
            RealmTargetParseError::MissingRealm => write!(
                f,
                "realm target must include a realm label before `.{TARGET_SUFFIX}`"
            ),
            RealmTargetParseError::SelectorNotAllowed => write!(
                f,
                "`all` and `*` are list-only selectors, not target labels"
            ),
            RealmTargetParseError::ReservedLabel => write!(
                f,
                "`{TARGET_SUFFIX}` is reserved for the target-name suffix"
            ),
            RealmTargetParseError::BadLabel(e) => write!(f, "malformed target label: {e}"),
            RealmTargetParseError::BadRealmPath => {
                write!(f, "realm path is empty or exceeds bounds")
            }
            RealmTargetParseError::AliasAmbiguous { alias, candidates } => {
                let rendered = candidates
                    .iter()
                    .map(RealmTarget::to_canonical)
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(
                    f,
                    "bare alias `{alias}` is ambiguous; use one of: {rendered}"
                )
            }
            RealmTargetParseError::LegacyNodeQualified { legacy, suggested } => write!(
                f,
                "target `{}` uses the old ADR 0032 node-qualified grammar; remove node label `{}` and use `{}`",
                legacy.diagnostic_form(),
                legacy.node,
                suggested.to_canonical()
            ),
        }
    }
}

impl std::error::Error for RealmTargetParseError {}

/// Context-aware ADR 0043 parser for callers that intentionally support bare
/// aliases or need old node-qualified migration diagnostics.
#[derive(Debug, Clone, Default)]
pub struct RealmTargetParser {
    default_realm: Option<RealmPath>,
    aliases: BTreeMap<WorkloadId, Vec<RealmTarget>>,
    legacy_node_labels: BTreeSet<NodeId>,
}

impl RealmTargetParser {
    /// Build a parser with no bare-alias context and no legacy-node diagnostics.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure a default realm for otherwise-unmapped bare workload aliases.
    pub fn with_default_realm(mut self, realm: RealmPath) -> Self {
        self.default_realm = Some(realm);
        self
    }

    /// Add a local bare-alias target candidate.
    pub fn with_alias(mut self, alias: WorkloadId, target: RealmTarget) -> Self {
        self.aliases.entry(alias).or_default().push(target);
        self
    }

    /// Mark a legacy ADR 0032 node label. A target whose second label matches
    /// this set is returned as a migration diagnostic when a realm path follows.
    pub fn with_legacy_node_label(mut self, node: NodeId) -> Self {
        self.legacy_node_labels.insert(node);
        self
    }

    /// Parse with the configured alias and migration-diagnostic context.
    pub fn parse(&self, raw: &str) -> Result<RealmTarget, RealmTargetParseError> {
        let labels = target_labels(raw)?;
        if labels.had_suffix {
            self.parse_qualified(labels.labels)
        } else if labels.had_scheme {
            Err(RealmTargetParseError::MissingSuffix)
        } else {
            self.parse_bare(labels.labels)
        }
    }

    fn parse_bare(&self, labels: Vec<&str>) -> Result<RealmTarget, RealmTargetParseError> {
        match labels.as_slice() {
            [alias] => {
                reject_reserved_labels(&labels)?;
                let workload =
                    WorkloadId::parse(*alias).map_err(RealmTargetParseError::BadLabel)?;
                if let Some(alias_candidates) = self.aliases.get(&workload) {
                    let mut candidates = Vec::new();
                    for candidate in alias_candidates {
                        if !candidates.contains(candidate) {
                            candidates.push(candidate.clone());
                        }
                    }
                    return match candidates.as_slice() {
                        [target] => Ok(target.clone()),
                        [] => unreachable!("alias map stores only non-empty candidate vectors"),
                        _ => Err(RealmTargetParseError::AliasAmbiguous {
                            alias: workload,
                            candidates,
                        }),
                    };
                }

                self.default_realm
                    .clone()
                    .map(|realm| RealmTarget::new(workload, realm))
                    .ok_or(RealmTargetParseError::BareAliasRequiresContext)
            }
            _ => Err(RealmTargetParseError::MissingSuffix),
        }
    }

    fn parse_qualified(&self, labels: Vec<&str>) -> Result<RealmTarget, RealmTargetParseError> {
        if let Some(legacy) = legacy_for_known_node(&labels, &self.legacy_node_labels)? {
            let suggested = legacy.suggested_realm_target();
            return Err(RealmTargetParseError::LegacyNodeQualified { legacy, suggested });
        }
        parse_qualified_labels(labels)
    }
}

fn parse_realm_target(raw: &str) -> Result<RealmTarget, RealmTargetParseError> {
    let labels = target_labels(raw)?;
    if !labels.had_suffix {
        if labels.had_scheme {
            return Err(RealmTargetParseError::MissingSuffix);
        }
        return match labels.labels.as_slice() {
            [_] => Err(RealmTargetParseError::BareAliasRequiresContext),
            _ => Err(RealmTargetParseError::MissingSuffix),
        };
    }
    parse_qualified_labels(labels.labels)
}

fn parse_legacy_node_qualified(
    raw: &str,
) -> Result<LegacyNodeQualifiedTarget, RealmTargetParseError> {
    let labels = target_labels(raw)?;
    if !labels.had_suffix {
        return Err(RealmTargetParseError::MissingSuffix);
    }
    parse_legacy_labels(&labels.labels)
}

fn parse_qualified_labels(labels: Vec<&str>) -> Result<RealmTarget, RealmTargetParseError> {
    reject_reserved_labels(&labels)?;
    if labels.is_empty() {
        return Err(RealmTargetParseError::MissingWorkload);
    }

    let workload = WorkloadId::parse(labels[0]).map_err(RealmTargetParseError::BadLabel)?;
    if labels.len() < 2 {
        return Err(RealmTargetParseError::MissingRealm);
    }
    let realm = parse_realm_labels(&labels[1..])?;
    Ok(RealmTarget::new(workload, realm))
}

fn parse_legacy_labels(
    labels: &[&str],
) -> Result<LegacyNodeQualifiedTarget, RealmTargetParseError> {
    reject_reserved_labels(labels)?;
    if labels.is_empty() {
        return Err(RealmTargetParseError::MissingWorkload);
    }
    if labels.len() < 3 {
        return Err(RealmTargetParseError::MissingRealm);
    }

    let workload = WorkloadId::parse(labels[0]).map_err(RealmTargetParseError::BadLabel)?;
    let node = NodeId::parse(labels[1]).map_err(RealmTargetParseError::BadLabel)?;
    let realm = parse_realm_labels(&labels[2..])?;
    Ok(LegacyNodeQualifiedTarget {
        workload,
        node,
        realm,
    })
}

fn legacy_for_known_node(
    labels: &[&str],
    legacy_node_labels: &BTreeSet<NodeId>,
) -> Result<Option<LegacyNodeQualifiedTarget>, RealmTargetParseError> {
    if labels.len() < 3 || legacy_node_labels.is_empty() {
        return Ok(None);
    }
    let Ok(node) = NodeId::parse(labels[1]) else {
        return Ok(None);
    };
    if !legacy_node_labels.contains(&node) {
        return Ok(None);
    }
    parse_legacy_labels(labels).map(Some)
}

fn parse_realm_labels(labels: &[&str]) -> Result<RealmPath, RealmTargetParseError> {
    let realm_labels = labels
        .iter()
        .map(|l| RealmId::parse(*l).map_err(RealmTargetParseError::BadLabel))
        .collect::<Result<Vec<_>, _>>()?;
    RealmPath::new(realm_labels).ok_or(RealmTargetParseError::BadRealmPath)
}

fn reject_reserved_labels(labels: &[&str]) -> Result<(), RealmTargetParseError> {
    for label in labels {
        match *label {
            "all" | "*" => return Err(RealmTargetParseError::SelectorNotAllowed),
            TARGET_SUFFIX => return Err(RealmTargetParseError::ReservedLabel),
            _ => {}
        }
    }
    Ok(())
}

struct TargetLabels<'a> {
    labels: Vec<&'a str>,
    had_suffix: bool,
    had_scheme: bool,
}

fn target_labels(raw: &str) -> Result<TargetLabels<'_>, RealmTargetParseError> {
    let (body, had_scheme) = match raw.strip_prefix("d2b://") {
        Some(rest) => (rest, true),
        None => (raw, false),
    };
    if body.is_empty() {
        return Err(RealmTargetParseError::Empty);
    }

    let mut labels: Vec<&str> = body.split('.').collect();
    let had_suffix = labels.last() == Some(&TARGET_SUFFIX);
    if had_suffix {
        labels.pop();
        if labels.is_empty() {
            return Err(RealmTargetParseError::MissingWorkload);
        }
    }

    Ok(TargetLabels {
        labels,
        had_suffix,
        had_scheme,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::realm::MAX_REALM_LABELS;

    fn realm(labels: &[&str]) -> RealmPath {
        RealmPath::new(labels.iter().map(|l| RealmId::parse(*l).unwrap()).collect()).unwrap()
    }

    fn workload(raw: &str) -> WorkloadId {
        WorkloadId::parse(raw).unwrap()
    }

    fn node(raw: &str) -> NodeId {
        NodeId::parse(raw).unwrap()
    }

    fn parsed(raw: &str) -> RealmTarget {
        RealmTarget::parse(raw).unwrap_or_else(|e| panic!("parse {raw:?}: {e}"))
    }

    #[test]
    fn canonical_examples_parse_and_render() {
        let builder = parsed("builder.dev.d2b");
        assert_eq!(builder.workload.as_str(), "builder");
        assert_eq!(builder.realm.target_form(), "dev");
        assert_eq!(builder.to_canonical(), "builder.dev.d2b");

        let browser = parsed("browser.work.d2b");
        assert_eq!(browser.workload.as_str(), "browser");
        assert_eq!(browser.realm.target_form(), "work");
        assert_eq!(browser.to_string(), "browser.work.d2b");

        let api = parsed("api.payments.work.d2b");
        assert_eq!(api.workload.as_str(), "api");
        assert_eq!(api.realm.target_form(), "payments.work");
        assert_eq!(api.realm.storage_form(), "work/payments");
        assert_eq!(api.to_canonical(), "api.payments.work.d2b");
    }

    #[test]
    fn optional_scheme_is_accepted_but_not_rendered_canonically() {
        let target = parsed("d2b://builder.dev.d2b");
        assert_eq!(target.to_canonical(), "builder.dev.d2b");
    }

    #[test]
    fn serde_and_schema_are_canonical_string_shaped() {
        let target = parsed("api.payments.work.d2b");
        assert_eq!(
            serde_json::to_string(&target).unwrap(),
            "\"api.payments.work.d2b\""
        );
        assert_eq!(
            serde_json::from_str::<RealmTarget>("\"api.payments.work.d2b\"").unwrap(),
            target
        );
        assert!(
            serde_json::from_str::<RealmTarget>(
                r#"{"workload":"api","realm":["payments","work"]}"#
            )
            .is_err()
        );

        let schema = schemars::schema_for!(RealmTarget);
        assert!(matches!(
            schema.schema.instance_type,
            Some(SingleOrVec::Single(single)) if *single == InstanceType::String
        ));
    }

    #[test]
    fn bare_alias_requires_context_by_default() {
        assert_eq!(
            RealmTarget::parse("builder"),
            Err(RealmTargetParseError::BareAliasRequiresContext)
        );
    }

    #[test]
    fn parser_resolves_bare_aliases_with_default_realm() {
        let parser = RealmTargetParser::new().with_default_realm(realm(&["dev"]));
        let target = parser.parse("builder").unwrap();
        assert_eq!(target.to_canonical(), "builder.dev.d2b");
    }

    #[test]
    fn scheme_form_requires_fully_qualified_suffix() {
        let parser = RealmTargetParser::new()
            .with_default_realm(realm(&["dev"]))
            .with_alias(
                workload("builder"),
                RealmTarget::new(workload("builder"), realm(&["work"])),
            );
        assert_eq!(
            RealmTarget::parse("d2b://builder"),
            Err(RealmTargetParseError::MissingSuffix)
        );
        assert_eq!(
            parser.parse("d2b://builder"),
            Err(RealmTargetParseError::MissingSuffix)
        );
    }

    #[test]
    fn parser_rejects_ambiguous_bare_aliases() {
        let parser = RealmTargetParser::new()
            .with_alias(
                workload("browser"),
                RealmTarget::new(workload("browser"), realm(&["work"])),
            )
            .with_alias(
                workload("browser"),
                RealmTarget::new(workload("browser"), realm(&["dev"])),
            );
        let err = parser.parse("browser").unwrap_err();
        match err {
            RealmTargetParseError::AliasAmbiguous { alias, candidates } => {
                assert_eq!(alias.as_str(), "browser");
                let rendered = candidates
                    .iter()
                    .map(RealmTarget::to_canonical)
                    .collect::<Vec<_>>();
                assert_eq!(rendered, vec!["browser.work.d2b", "browser.dev.d2b"]);
            }
            other => panic!("expected AliasAmbiguous, got {other:?}"),
        }
    }

    #[test]
    fn parser_dedupes_identical_alias_targets_before_ambiguity() {
        let target = RealmTarget::new(workload("browser"), realm(&["work"]));
        let parser = RealmTargetParser::new()
            .with_alias(workload("browser"), target.clone())
            .with_alias(workload("browser"), target.clone());
        assert_eq!(parser.parse("browser").unwrap(), target);
    }

    #[test]
    fn target_name_preserves_legacy_field_access_without_node_routing() {
        let target = TargetName::parse("api.aca.work.d2b").unwrap();
        assert_eq!(target.workload.as_str(), "api");
        assert_eq!(target.realm.target_form(), "aca.work");
        assert_eq!(target.node.as_str(), THIS_NODE_ALIAS);
        assert!(target.node_is_this());
        assert_eq!(target.to_canonical(), "api.aca.work.d2b");

        let local = node("local-node");
        let with_local = target.with_local_node(local.clone());
        assert_eq!(with_local.node, local);
        assert_eq!(with_local.realm.target_form(), "aca.work");
        assert_eq!(with_local.to_canonical(), "api.aca.work.d2b");
    }

    #[test]
    fn multi_label_without_suffix_is_rejected() {
        assert_eq!(
            RealmTarget::parse("builder.dev"),
            Err(RealmTargetParseError::MissingSuffix)
        );
        assert_eq!(
            RealmTarget::parse("d2b://api.payments.work"),
            Err(RealmTargetParseError::MissingSuffix)
        );
    }

    #[test]
    fn workload_suffix_without_realm_is_rejected() {
        assert_eq!(
            RealmTarget::parse("demo.d2b"),
            Err(RealmTargetParseError::MissingRealm)
        );
    }

    #[test]
    fn known_legacy_node_qualified_target_is_migration_diagnostic() {
        let parser = RealmTargetParser::new().with_legacy_node_label(node("aca"));
        let err = parser.parse("demo.aca.work.d2b").unwrap_err();
        match err {
            RealmTargetParseError::LegacyNodeQualified { legacy, suggested } => {
                assert_eq!(legacy.workload.as_str(), "demo");
                assert_eq!(legacy.node.as_str(), "aca");
                assert_eq!(legacy.realm.target_form(), "work");
                assert_eq!(suggested.to_canonical(), "demo.work.d2b");
            }
            other => panic!("expected LegacyNodeQualified, got {other:?}"),
        }
    }

    #[test]
    fn legacy_parser_is_diagnostic_helper_only() {
        let legacy = LegacyNodeQualifiedTarget::parse("demo.aca.work.d2b").unwrap();
        assert_eq!(legacy.diagnostic_form(), "demo.aca.work.d2b");
        assert_eq!(
            legacy.suggested_realm_target().to_canonical(),
            "demo.work.d2b"
        );
    }

    #[test]
    fn selectors_and_reserved_labels_are_rejected() {
        assert_eq!(
            RealmTarget::parse("all.work.d2b"),
            Err(RealmTargetParseError::SelectorNotAllowed)
        );
        assert_eq!(
            RealmTarget::parse("demo.all.d2b"),
            Err(RealmTargetParseError::SelectorNotAllowed)
        );
        assert_eq!(
            RealmTarget::parse("*.work.d2b"),
            Err(RealmTargetParseError::SelectorNotAllowed)
        );
        assert_eq!(
            RealmTarget::parse("d2b.work.d2b"),
            Err(RealmTargetParseError::ReservedLabel)
        );
        assert_eq!(
            RealmTarget::parse("demo.d2b.work.d2b"),
            Err(RealmTargetParseError::ReservedLabel)
        );
    }

    #[test]
    fn empty_and_suffix_only_are_rejected() {
        assert_eq!(RealmTarget::parse(""), Err(RealmTargetParseError::Empty));
        assert_eq!(
            RealmTarget::parse("d2b://"),
            Err(RealmTargetParseError::Empty)
        );
        assert_eq!(
            RealmTarget::parse("d2b"),
            Err(RealmTargetParseError::MissingWorkload)
        );
        assert_eq!(
            RealmTarget::parse(".d2b"),
            Err(RealmTargetParseError::BadLabel(IdError::Empty))
        );
    }

    #[test]
    fn malformed_labels_are_rejected() {
        assert!(matches!(
            RealmTarget::parse("Demo.work.d2b"),
            Err(RealmTargetParseError::BadLabel(_))
        ));
        assert!(matches!(
            RealmTarget::parse("demo.-bad.d2b"),
            Err(RealmTargetParseError::BadLabel(_))
        ));
    }

    #[test]
    fn realm_path_bounds_are_enforced() {
        let too_many = (0..=MAX_REALM_LABELS)
            .map(|i| format!("r{i}"))
            .collect::<Vec<_>>()
            .join(".");
        let raw = format!("demo.{too_many}.d2b");
        assert_eq!(
            RealmTarget::parse(&raw),
            Err(RealmTargetParseError::BadRealmPath)
        );

        let long = "a".repeat(128);
        let raw = format!("demo.{long}.{long}.d2b");
        assert_eq!(
            RealmTarget::parse(&raw),
            Err(RealmTargetParseError::BadRealmPath)
        );
    }
}
