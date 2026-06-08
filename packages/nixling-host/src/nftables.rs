//! W3 host-prepare module: `nftables`.
//!
//! Owned by scope **s3** (nft + USBIP firewall skeleton) per the W3
//! file-ownership map.
//!
//! This module implements:
//!
//! - the deterministic `inet nixling` chain layout from plan.md
//!   §"W3 `inet nixling` chain layout" (prerouting/forward/output/input
//!   only, NO raw/mangle/nat);
//! - host firewall manager detection (firewalld/ufw/Docker/libvirt/
//!   iptables-nft);
//! - the 7-row firewall coexistence policy matrix from plan.md
//!   §"W3 firewall coexistence policy";
//! - drift detection via canonical hashing of the `inet nixling` table
//!   JSON;
//! - USBIP source-based firewall carve-out rule construction (skeleton,
//!   ordering invariant: specific carve-outs BEFORE the generic
//!   allow/drop rules in the `forward` chain).
//!
//! Rationale for not depending on `nftnl` / libnftnl: the panel ADR for
//! W3 s3 ("W3 firewall coexistence policy matrix + `inet nixling`
//! chain layout") rejected pulling libnftnl into this crate because
//! the integrator-prep nix build environment ships nft(8) but does
//! NOT ship libnftnl-dev. The fallback approach: this module produces
//! a structured [`NftBatch`] (a typed in-memory description) and an
//! `nft -f -` text rendering; the broker side feeds that text to the
//! real `nft` binary at apply time, and re-hashes the result via
//! `nft list table inet nixling -j`. The fake backend behind
//! `feature = "fake-backends"` short-circuits the apply step so unit
//! tests can drive the full coexistence matrix without a live nft
//! kernel surface.

use nixling_core::host_w3::{CoexistencePolicy, FirewallManager};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256 as Sha256Hasher};
use std::fmt;

/// Hex-encoded SHA-256 digest. Returned by [`hash_inet_nixling_table`]
/// and consumed by the broker as the `table_hash_before`/`_after`
/// audit fields.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Sha256(String);

impl Sha256 {
    /// Build a digest from raw input bytes.
    pub fn of(bytes: &[u8]) -> Self {
        let mut hasher = Sha256Hasher::new();
        hasher.update(bytes);
        let out = hasher.finalize();
        let hex = out.iter().map(|b| format!("{b:02x}")).collect::<String>();
        Self(hex)
    }

    /// Hex string view.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Errors returned by the s3 nftables surface. Discriminants are
/// kebab-case to match the broker audit log + the wider
/// `nixling-core::error` taxonomy. The [`Self::as_kebab_case`] helper
/// is the canonical mapping consumed by
/// [`nixling_core::error::Error::internal_io`] when an error needs to
/// surface through the broker wire as a typed [`nixling_core::error`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NftError {
    /// A foreign nft rule sits above the `inet nixling` chains at a
    /// priority that would shadow the nixling decision.
    ForeignNftRuleShadowsNixling { details: String },
    /// The detected firewall manager does not match the declared
    /// coexistence policy. Carries the detected manager and the
    /// policy the bundle declared so audit can record both.
    FirewallCoexistenceMismatch {
        detected: FirewallManager,
        declared: CoexistencePolicy,
    },
    /// Attempted to flush a foreign nft table/chain. W3 NEVER flushes
    /// foreign rules; this error is fail-closed.
    NftForeignRuleFlushAttempted { target: String },
    /// The post-apply hash of `inet nixling` does not match the
    /// pre-apply hash recorded in host.json.
    InetNixlingDrift { before: Sha256, after: Sha256 },
}

impl NftError {
    /// Stable kebab-case discriminant for audit logs + the typed-error
    /// mapping into `nixling-core::error::Error`.
    pub const fn as_kebab_case(&self) -> &'static str {
        match self {
            Self::ForeignNftRuleShadowsNixling { .. } => "foreign-nft-rule-shadows-nixling",
            Self::FirewallCoexistenceMismatch { .. } => "firewall-coexistence-mismatch",
            Self::NftForeignRuleFlushAttempted { .. } => "nft-foreign-rule-flush-attempted",
            Self::InetNixlingDrift { .. } => "inet-nixling-drift",
        }
    }

    /// Map to a `nixling-core::error::Error` via the
    /// [`nixling_core::error::Error::internal_io`] constructor. The
    /// stable kebab-case discriminant is the opaque reason; the
    /// broker audit log records the structured variant separately so
    /// no operator-visible message loses the typed detail.
    pub fn to_core_error(&self) -> nixling_core::error::Error {
        nixling_core::error::Error::internal_io(self.as_kebab_case())
    }
}

impl fmt::Display for NftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_kebab_case())
    }
}

impl std::error::Error for NftError {}

/// Parser error for the limited `nft -f -` script dialect nixling emits
/// for the managed `inet nixling` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseNftScriptError {
    detail: String,
}

impl ParseNftScriptError {
    fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for ParseNftScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.detail)
    }
}

impl std::error::Error for ParseNftScriptError {}

/// Hook priority constants (Linux nft conventions). Values chosen to
/// match plan.md §"W3 `inet nixling` chain layout":
///
/// | Chain        | Priority |
/// | ------------ | -------- |
/// | `prerouting` | `-150` (equal to mangle) |
/// | `forward`    | `-5`  (just before filter) |
/// | `output`     | `-5`  |
/// | `input`      | `-5`  |
pub mod priority {
    pub const PREROUTING: i32 = -150;
    pub const FORWARD: i32 = -5;
    pub const OUTPUT: i32 = -5;
    pub const INPUT: i32 = -5;
}

/// Chain hook kinds permitted under `inet nixling`. The variants
/// intentionally do NOT include `raw`, `mangle`, or `nat` per plan.md
/// §"W3 `inet nixling` chain layout".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChainHook {
    Prerouting,
    Forward,
    Output,
    Input,
}

impl ChainHook {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Prerouting => "prerouting",
            Self::Forward => "forward",
            Self::Output => "output",
            Self::Input => "input",
        }
    }
}

/// Default policy on a chain (`accept` or `drop`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChainPolicy {
    Accept,
    Drop,
}

impl ChainPolicy {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Drop => "drop",
        }
    }
}

/// A single chain in the `inet nixling` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NftChain {
    pub name: String,
    pub hook: ChainHook,
    pub priority: i32,
    pub policy: ChainPolicy,
    /// Rules in order. The reconcile contract requires specific
    /// carve-outs (e.g. USBIP per-busid) to be inserted BEFORE the
    /// generic allow/drop rules — [`NftBatch::add_usbip_carveout`]
    /// enforces this invariant.
    pub rules: Vec<NftRule>,
}

/// A single nft rule. The `expr` field is the rendered nft expression
/// (e.g. `"ip saddr 10.10.0.5 accept"`); `comment` carries the
/// mandatory `nixling managed: <ownership-id>` marker per plan.md
/// §"W3 `inet nixling` chain layout".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NftRule {
    pub expr: String,
    pub comment: String,
    /// Specific (per-busid, per-flow) carve-outs sort before generic
    /// allow/drop rules. The reconcile algorithm relies on this flag
    /// to keep ordering deterministic across re-apply.
    #[serde(default)]
    pub specific_carveout: bool,
}

/// A batch of declarative nft state for the `inet nixling` table.
///
/// The broker side reads this from the trusted bundle, renders it via
/// [`NftBatch::render_nft_script`], and feeds the script to
/// `nft -f -`. Foreign tables and chains are NEVER touched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NftBatch {
    pub table_family: &'static str,
    pub table_name: &'static str,
    pub chains: Vec<NftChain>,
}

impl NftBatch {
    /// Marker prefix that EVERY rule and chain comment must carry. The
    /// drift / preservation gate uses it to distinguish nixling-managed
    /// from foreign state.
    pub const COMMENT_PREFIX: &'static str = "nixling managed: ";

    /// Parse the limited nixling-managed `nft -f -` script dialect back
    /// into a structured batch so runtime checks can re-assert ordering
    /// invariants before touching the live table.
    pub fn parse(script: &str) -> Result<Self, ParseNftScriptError> {
        struct PendingChain {
            name: String,
            hook: Option<ChainHook>,
            priority: Option<i32>,
            policy: Option<ChainPolicy>,
            rules: Vec<NftRule>,
        }

        impl PendingChain {
            fn finish(self, line_no: usize) -> Result<NftChain, ParseNftScriptError> {
                let hook = self.hook.ok_or_else(|| {
                    ParseNftScriptError::new(format!(
                        "line {line_no}: chain `{}` missing `type filter hook ... priority ...` declaration",
                        self.name
                    ))
                })?;
                let priority = self.priority.ok_or_else(|| {
                    ParseNftScriptError::new(format!(
                        "line {line_no}: chain `{}` missing hook priority",
                        self.name
                    ))
                })?;
                let policy = self.policy.ok_or_else(|| {
                    ParseNftScriptError::new(format!(
                        "line {line_no}: chain `{}` missing `policy ...;` declaration",
                        self.name
                    ))
                })?;
                Ok(NftChain {
                    name: self.name,
                    hook,
                    priority,
                    policy,
                    rules: self.rules,
                })
            }
        }

        let mut saw_table = false;
        let mut saw_table_end = false;
        let mut chains = Vec::new();
        let mut current: Option<PendingChain> = None;

        for (index, raw_line) in script.lines().enumerate() {
            let line_no = index + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if saw_table_end {
                return Err(ParseNftScriptError::new(format!(
                    "line {line_no}: unexpected content after table close"
                )));
            }
            if !saw_table {
                let rest = line.strip_prefix("table ").ok_or_else(|| {
                    ParseNftScriptError::new(format!(
                        "line {line_no}: expected `table <family> <name> {{` header"
                    ))
                })?;
                let (table_decl, closed_inline) = if let Some(decl) = rest.strip_suffix("{}") {
                    (decl.trim(), true)
                } else if let Some(decl) = rest.strip_suffix('{') {
                    (decl.trim(), false)
                } else {
                    return Err(ParseNftScriptError::new(format!(
                        "line {line_no}: malformed table header `{line}`"
                    )));
                };
                let mut tokens = table_decl.split_whitespace();
                let family = tokens.next().ok_or_else(|| {
                    ParseNftScriptError::new(format!(
                        "line {line_no}: missing nft table family in `{line}`"
                    ))
                })?;
                let table = tokens.next().ok_or_else(|| {
                    ParseNftScriptError::new(format!(
                        "line {line_no}: missing nft table name in `{line}`"
                    ))
                })?;
                if tokens.next().is_some() {
                    return Err(ParseNftScriptError::new(format!(
                        "line {line_no}: malformed table header `{line}`"
                    )));
                }
                if !family.eq_ignore_ascii_case("inet") || table != "nixling" {
                    return Err(ParseNftScriptError::new(format!(
                        "line {line_no}: only `table inet nixling` is supported, got `table {family} {table}`"
                    )));
                }
                saw_table = true;
                saw_table_end = closed_inline;
                continue;
            }
            if let Some(chain) = current.as_mut() {
                if line == "}" {
                    if let Some(finished) = current.take() {
                        chains.push(finished.finish(line_no)?);
                    }
                    continue;
                }
                if line.starts_with("type ") {
                    let normalized = line.trim_end_matches(';').replace(';', "");
                    let tokens: Vec<_> = normalized.split_whitespace().collect();
                    let (hook, priority, policy) = match tokens.as_slice() {
                        ["type", "filter", "hook", hook, "priority", priority] => {
                            (*hook, *priority, None)
                        }
                        ["type", "filter", "hook", hook, "priority", priority, "policy", policy] => {
                            (*hook, *priority, Some(*policy))
                        }
                        _ => {
                            return Err(ParseNftScriptError::new(format!(
                                "line {line_no}: unsupported chain header `{line}`"
                            )));
                        }
                    };
                    chain.hook = Some(Self::parse_hook_token(hook, line_no)?);
                    chain.priority = Some(priority.parse().map_err(|_| {
                        ParseNftScriptError::new(format!(
                            "line {line_no}: invalid hook priority `{priority}`"
                        ))
                    })?);
                    if let Some(policy) = policy {
                        chain.policy = Some(Self::parse_policy_token(policy, line_no)?);
                    }
                    continue;
                }
                if line.starts_with("policy ") {
                    let normalized = line.trim_end_matches(';').replace(';', "");
                    let tokens: Vec<_> = normalized.split_whitespace().collect();
                    let policy = match tokens.as_slice() {
                        ["policy", policy] => *policy,
                        _ => {
                            return Err(ParseNftScriptError::new(format!(
                                "line {line_no}: malformed policy line `{line}`"
                            )));
                        }
                    };
                    chain.policy = Some(Self::parse_policy_token(policy, line_no)?);
                    continue;
                }
                chain.rules.push(Self::parse_rule_line(line, line_no)?);
                continue;
            }
            if line == "}" {
                saw_table_end = true;
                continue;
            }
            let rest = line.strip_prefix("chain ").ok_or_else(|| {
                ParseNftScriptError::new(format!(
                    "line {line_no}: expected `chain <name> {{` or `}}`"
                ))
            })?;
            let name = rest.strip_suffix('{').ok_or_else(|| {
                ParseNftScriptError::new(format!("line {line_no}: malformed chain header `{line}`"))
            })?;
            let name = name.trim();
            if name.is_empty() {
                return Err(ParseNftScriptError::new(format!(
                    "line {line_no}: chain name must not be empty"
                )));
            }
            current = Some(PendingChain {
                name: name.to_owned(),
                hook: None,
                priority: None,
                policy: None,
                rules: Vec::new(),
            });
        }

        if !saw_table {
            return Err(ParseNftScriptError::new(
                "missing `table inet nixling` header",
            ));
        }
        if let Some(chain) = current {
            return Err(ParseNftScriptError::new(format!(
                "unterminated chain `{}` block",
                chain.name
            )));
        }
        if !saw_table_end {
            return Err(ParseNftScriptError::new(
                "unterminated `table inet nixling` block",
            ));
        }

        Ok(Self {
            table_family: "inet",
            table_name: "nixling",
            chains,
        })
    }

    fn parse_hook_token(token: &str, line_no: usize) -> Result<ChainHook, ParseNftScriptError> {
        match token {
            "prerouting" => Ok(ChainHook::Prerouting),
            "forward" => Ok(ChainHook::Forward),
            "output" => Ok(ChainHook::Output),
            "input" => Ok(ChainHook::Input),
            _ => Err(ParseNftScriptError::new(format!(
                "line {line_no}: unsupported nft hook `{token}`"
            ))),
        }
    }

    fn parse_policy_token(token: &str, line_no: usize) -> Result<ChainPolicy, ParseNftScriptError> {
        match token {
            "accept" => Ok(ChainPolicy::Accept),
            "drop" => Ok(ChainPolicy::Drop),
            _ => Err(ParseNftScriptError::new(format!(
                "line {line_no}: unsupported chain policy `{token}`"
            ))),
        }
    }

    fn parse_rule_line(line: &str, line_no: usize) -> Result<NftRule, ParseNftScriptError> {
        let rule = line.trim_end_matches(';').trim();
        let (expr, comment) = rule.rsplit_once(" comment \"").ok_or_else(|| {
            ParseNftScriptError::new(format!(
                "line {line_no}: managed rule missing trailing `comment \"...\"`: `{line}`"
            ))
        })?;
        let comment = comment.strip_suffix('"').ok_or_else(|| {
            ParseNftScriptError::new(format!(
                "line {line_no}: unterminated nft rule comment in `{line}`"
            ))
        })?;
        if expr.is_empty() {
            return Err(ParseNftScriptError::new(format!(
                "line {line_no}: nft rule expression must not be empty"
            )));
        }
        Ok(NftRule {
            expr: expr.to_owned(),
            comment: comment.to_owned(),
            specific_carveout: expr.contains("usbip-") || comment.contains("usbip-carveout-"),
        })
    }

    /// Append a USBIP source-based carve-out rule to the `forward`
    /// chain, preserving the specific-before-generic ordering
    /// invariant.
    ///
    /// Returns an error if the batch is malformed (no `forward` chain).
    pub fn add_usbip_carveout(&mut self, bus_id: &BusId) -> Result<(), NftError> {
        let chain = self
            .chains
            .iter_mut()
            .find(|c| matches!(c.hook, ChainHook::Forward))
            .ok_or_else(|| NftError::ForeignNftRuleShadowsNixling {
                details: "forward chain missing from inet nixling batch".to_owned(),
            })?;

        let rule = NftRule {
            expr: format!("meta iifname \"usbip-{bus_id}\" accept", bus_id = bus_id.0),
            comment: format!("{}usbip-carveout-{}", NftBatch::COMMENT_PREFIX, bus_id.0),
            specific_carveout: true,
        };

        // Insert carve-out at the front of the specific-carveout block,
        // ahead of any generic rule. The reconcile gate
        // (`assert_carveout_ordering`) verifies this invariant.
        let insert_at = chain
            .rules
            .iter()
            .position(|r| !r.specific_carveout)
            .unwrap_or(chain.rules.len());
        chain.rules.insert(insert_at, rule);
        Ok(())
    }

    /// Render the batch as an `nft -f -` script. Used by the broker
    /// apply path to feed `nft` without depending on libnftnl.
    pub fn render_nft_script(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "table {} {} {{\n",
            self.table_family, self.table_name
        ));
        for chain in &self.chains {
            out.push_str(&format!(
                "  chain {name} {{\n    type filter hook {hook} priority {prio}; policy {policy};\n",
                name = chain.name,
                hook = chain.hook.as_str(),
                prio = chain.priority,
                policy = chain.policy.as_str(),
            ));
            for rule in &chain.rules {
                out.push_str(&format!(
                    "    {expr} comment \"{comment}\"\n",
                    expr = rule.expr,
                    comment = rule.comment,
                ));
            }
            out.push_str("  }\n");
        }
        out.push_str("}\n");
        out
    }

    /// Canonical-hash the batch by hashing its rendered nft script.
    pub fn canonical_hash(&self) -> Sha256 {
        Sha256::of(self.render_nft_script().as_bytes())
    }

    /// Assert that every chain keeps specific carve-outs strictly
    /// before generic rules. Returns an error pinpointing the
    /// first chain where the invariant is violated.
    pub fn assert_carveout_ordering(&self) -> Result<(), NftError> {
        for chain in &self.chains {
            let mut seen_generic = false;
            for rule in &chain.rules {
                if rule.specific_carveout && seen_generic {
                    return Err(NftError::ForeignNftRuleShadowsNixling {
                        details: format!(
                            "chain {name}: specific carve-out '{c}' appears after a generic rule",
                            name = chain.name,
                            c = rule.comment
                        ),
                    });
                }
                if !rule.specific_carveout {
                    seen_generic = true;
                }
            }
        }
        Ok(())
    }
}

/// USBIP busid newtype. The broker re-validates the busid lexical form
/// against the trusted bundle before passing it down here.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BusId(pub String);

impl BusId {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

impl fmt::Display for BusId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------
// Public API per the s3 contract
// ---------------------------------------------------------------------

/// Build the canonical `inet nixling` chain layout from plan.md
/// §"W3 `inet nixling` chain layout". Used by the broker as the
/// declarative starting point for `ApplyNftables`.
///
/// The chains emitted are EXACTLY: `prerouting`, `forward`, `output`,
/// `input`. NO `raw`/`mangle`/`nat` hooks are allocated under
/// `inet nixling`. This is the contract — adding hooks here requires
/// an ADR.
pub fn build_inet_nixling_chains() -> NftBatch {
    NftBatch {
        table_family: "inet",
        table_name: "nixling",
        chains: vec![
            NftChain {
                name: "prerouting".to_owned(),
                hook: ChainHook::Prerouting,
                priority: priority::PREROUTING,
                policy: ChainPolicy::Accept,
                rules: vec![],
            },
            NftChain {
                name: "forward".to_owned(),
                hook: ChainHook::Forward,
                priority: priority::FORWARD,
                policy: ChainPolicy::Drop,
                rules: vec![],
            },
            NftChain {
                name: "output".to_owned(),
                hook: ChainHook::Output,
                priority: priority::OUTPUT,
                policy: ChainPolicy::Accept,
                rules: vec![],
            },
            NftChain {
                name: "input".to_owned(),
                hook: ChainHook::Input,
                priority: priority::INPUT,
                policy: ChainPolicy::Accept,
                rules: vec![],
            },
        ],
    }
}

/// Inputs to [`detect_firewall_manager`]. Real callers pass
/// [`DetectorProbe::live`]; tests pass a fake.
#[derive(Debug, Clone)]
pub struct DetectorProbe {
    pub firewalld_active: bool,
    pub ufw_active: bool,
    pub docker_active: bool,
    pub libvirt_active: bool,
    pub iptables_reports_nf_tables: bool,
}

impl DetectorProbe {
    /// Clean-host shape used when nothing was detected.
    pub const fn none() -> Self {
        Self {
            firewalld_active: false,
            ufw_active: false,
            docker_active: false,
            libvirt_active: false,
            iptables_reports_nf_tables: false,
        }
    }
}

/// Detect the active firewall manager. Returns [`FirewallManager::None`]
/// on a clean host. Multiple incompatible managers collapse to
/// [`FirewallManager::Unknown`].
///
/// The live probe shell-outs are deliberately NOT performed here; the
/// caller is expected to pre-populate a [`DetectorProbe`]:
///
/// - `systemctl is-active firewalld` → `firewalld_active`
/// - `systemctl is-active ufw`       → `ufw_active`
/// - `docker info` succeeds          → `docker_active`
/// - `systemctl is-active libvirtd`  → `libvirt_active`
/// - `iptables --version` output contains `(nf_tables)` → `iptables_reports_nf_tables`
///
/// This split keeps `nixling-host` `#![forbid(unsafe_code)]` and
/// trivially testable; the live shell-out lives in the broker side.
pub fn detect_firewall_manager(probe: &DetectorProbe) -> FirewallManager {
    let mut hits: Vec<FirewallManager> = Vec::new();
    if probe.firewalld_active {
        hits.push(FirewallManager::Firewalld);
    }
    if probe.ufw_active {
        hits.push(FirewallManager::Ufw);
    }
    if probe.docker_active {
        hits.push(FirewallManager::Docker);
    }
    if probe.libvirt_active {
        hits.push(FirewallManager::Libvirt);
    }

    match hits.as_slice() {
        [] => {
            if probe.iptables_reports_nf_tables {
                FirewallManager::IptablesNft
            } else {
                FirewallManager::None
            }
        }
        [only] => *only,
        // Multiple incompatible managers collapse to Unknown per plan
        // §"W3 firewall coexistence policy" (row "unknown manager").
        _ => FirewallManager::Unknown,
    }
}

/// Evaluate the 7-row firewall coexistence matrix from plan.md
/// §"W3 firewall coexistence policy". Returns `Ok(())` when the
/// declared bundle policy is admissible for the detected manager;
/// returns [`NftError::FirewallCoexistenceMismatch`] otherwise.
///
/// Default policy per row (the bundle MUST declare exactly this default
/// unless an ADR overrides it):
///
/// | Detected            | Allowed declared policies         |
/// | ------------------- | --------------------------------- |
/// | `Firewalld`         | `Refuse`                          |
/// | `Ufw`               | `Refuse`                          |
/// | `Docker`            | `RequireUnmanaged`                |
/// | `Libvirt`           | `RequireUnmanaged`                |
/// | `IptablesNft`       | `Coexist` (L2 readback gates it)  |
/// | `Unknown`           | `Refuse`                          |
/// | `None`              | `Coexist`                         |
pub fn evaluate_coexistence_policy(
    detected: FirewallManager,
    declared: CoexistencePolicy,
) -> Result<(), NftError> {
    let allowed = match detected {
        FirewallManager::Firewalld => CoexistencePolicy::Refuse,
        FirewallManager::Ufw => CoexistencePolicy::Refuse,
        FirewallManager::Docker => CoexistencePolicy::RequireUnmanaged,
        FirewallManager::Libvirt => CoexistencePolicy::RequireUnmanaged,
        FirewallManager::IptablesNft => CoexistencePolicy::Coexist,
        FirewallManager::Unknown => CoexistencePolicy::Refuse,
        FirewallManager::None => CoexistencePolicy::Coexist,
    };
    if declared == allowed {
        Ok(())
    } else {
        Err(NftError::FirewallCoexistenceMismatch { detected, declared })
    }
}

/// Hash the live `inet nixling` table for drift detection. Real
/// callers invoke `nft list table inet nixling -j` and pipe the output
/// into this function as a byte slice; tests supply a fake JSON
/// snapshot.
///
/// The canonical-hash discipline strips runtime-volatile JSON fields
/// (`handle`, `index`) before hashing so two textually-different JSON
/// dumps describing the same logical table produce the same digest.
pub fn hash_inet_nixling_table(nft_list_json: &[u8]) -> Sha256 {
    let canonical = canonicalize_nft_json(nft_list_json);
    Sha256::of(canonical.as_bytes())
}

fn canonicalize_nft_json(input: &[u8]) -> String {
    // Strip volatile fields ("handle", "index") that the kernel
    // assigns per-add and would otherwise generate spurious drift.
    let parsed: serde_json::Value = match serde_json::from_slice(input) {
        Ok(v) => v,
        Err(_) => {
            // Non-JSON inputs hash by raw bytes so callers still get a
            // determinstic digest; the broker treats this as a probe
            // failure separately.
            return String::from_utf8_lossy(input).into_owned();
        }
    };
    let cleaned = strip_volatile(parsed);
    serde_json::to_string(&cleaned).unwrap_or_default()
}

fn strip_volatile(v: serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::with_capacity(map.len());
            // Iterate in sorted key order so the rendered JSON is
            // stable across HashMap iteration orderings.
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, val) in entries {
                if k == "handle" || k == "index" {
                    continue;
                }
                out.insert(k, strip_volatile(val));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(strip_volatile).collect())
        }
        other => other,
    }
}

/// Append a USBIP source-based carve-out for the given busid to a
/// fresh `inet nixling` batch. Ordering invariant (specific BEFORE
/// generic) is enforced by [`NftBatch::add_usbip_carveout`] +
/// [`NftBatch::assert_carveout_ordering`].
pub fn add_usbip_firewall_carveout(bus_id: &BusId) -> Result<NftBatch, NftError> {
    let mut batch = build_inet_nixling_chains();
    // Seed a generic drop rule first so the test can prove ordering;
    // the broker side seeds the real generic rules from the trusted
    // bundle's per-env policy.
    if let Some(forward) = batch
        .chains
        .iter_mut()
        .find(|c| matches!(c.hook, ChainHook::Forward))
    {
        forward.rules.push(NftRule {
            expr: "drop".to_owned(),
            comment: format!("{}default-deny-forward", NftBatch::COMMENT_PREFIX),
            specific_carveout: false,
        });
    }
    batch.add_usbip_carveout(bus_id)?;
    batch.assert_carveout_ordering()?;
    Ok(batch)
}

/// Assert that none of the chains under `inet nixling` accidentally
/// trip the "no raw/mangle/nat hooks" invariant. Belt-and-braces
/// alongside the typestate enforcement in [`ChainHook`].
pub fn assert_no_forbidden_hooks(batch: &NftBatch) -> Result<(), NftError> {
    for chain in &batch.chains {
        match chain.hook {
            ChainHook::Prerouting | ChainHook::Forward | ChainHook::Output | ChainHook::Input => {}
        }
        // The enum has no raw/mangle/nat variant, so a hook that
        // round-trips through it can never be one of those — but we
        // also defend against a buggy chain.name suggesting otherwise.
        let lname = chain.name.to_ascii_lowercase();
        if lname == "raw" || lname == "mangle" || lname == "nat" {
            return Err(NftError::ForeignNftRuleShadowsNixling {
                details: format!(
                    "chain name '{name}' suggests a forbidden hook family",
                    name = chain.name
                ),
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// Fake backend (feature = "fake-backends" or cfg(test))
// ---------------------------------------------------------------------

#[cfg(any(test, feature = "fake-backends"))]
pub mod fake {
    //! In-memory fake backend for the s3 L1c canary matrix tests.

    use super::*;
    use std::cell::RefCell;

    /// In-memory recording of every fake apply attempt.
    #[derive(Debug, Default)]
    pub struct FakeNftBackend {
        applied: RefCell<Vec<NftBatch>>,
        /// Foreign rules seeded by the test harness. Reconcile must
        /// preserve these byte-for-byte across repeat apply.
        foreign: RefCell<Vec<String>>,
    }

    impl FakeNftBackend {
        pub fn new() -> Self {
            Self::default()
        }

        /// Seed a foreign rule (e.g. an iptables-nft generated table)
        /// the reconcile path MUST preserve.
        pub fn seed_foreign(&self, rule: impl Into<String>) {
            self.foreign.borrow_mut().push(rule.into());
        }

        /// Apply a batch (recording it) and return the post-apply
        /// hash. Refuses to "flush" any foreign rule.
        pub fn apply(&self, batch: &NftBatch) -> Result<Sha256, NftError> {
            assert_no_forbidden_hooks(batch)?;
            batch.assert_carveout_ordering()?;
            self.applied.borrow_mut().push(batch.clone());
            // Preservation invariant: foreign rules must be unchanged.
            for foreign in self.foreign.borrow().iter() {
                if batch.render_nft_script().contains(foreign) {
                    return Err(NftError::NftForeignRuleFlushAttempted {
                        target: foreign.clone(),
                    });
                }
            }
            Ok(batch.canonical_hash())
        }

        pub fn applied_batches(&self) -> Vec<NftBatch> {
            self.applied.borrow().clone()
        }

        pub fn foreign_rules(&self) -> Vec<String> {
            self.foreign.borrow().clone()
        }
    }
}

// ---------------------------------------------------------------------
// Unit tests covering the L1c canary rows owned by s3
// (`nft-coexistence-*`, `foreign-nft-rule-preserved`,
// `usbip-firewall-skeleton`).
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_core::host_w3::{CoexistencePolicy, FirewallManager};

    #[test]
    fn build_inet_nixling_chains_layout_matches_plan() {
        let batch = build_inet_nixling_chains();
        assert_eq!(batch.table_family, "inet");
        assert_eq!(batch.table_name, "nixling");
        let by_hook: std::collections::HashMap<_, _> = batch
            .chains
            .iter()
            .map(|c| (c.hook, (c.priority, c.policy)))
            .collect();
        assert_eq!(
            by_hook[&ChainHook::Prerouting],
            (priority::PREROUTING, ChainPolicy::Accept)
        );
        assert_eq!(
            by_hook[&ChainHook::Forward],
            (priority::FORWARD, ChainPolicy::Drop)
        );
        assert_eq!(
            by_hook[&ChainHook::Output],
            (priority::OUTPUT, ChainPolicy::Accept)
        );
        assert_eq!(
            by_hook[&ChainHook::Input],
            (priority::INPUT, ChainPolicy::Accept)
        );
        assert_eq!(batch.chains.len(), 4, "exactly 4 chains; no raw/mangle/nat");
    }

    #[test]
    fn no_raw_mangle_nat_hooks() {
        let batch = build_inet_nixling_chains();
        assert!(assert_no_forbidden_hooks(&batch).is_ok());
    }

    /// 7-row coexistence matrix — these are the L1c canaries
    /// `nft-coexistence-{firewalld,ufw,docker,libvirt,iptables-nft,
    /// unknown-manager,no-manager}` from plan.md §"W3 pre-merge canary
    /// matrix".
    #[test]
    fn coexistence_matrix_all_7_rows() {
        use CoexistencePolicy::*;
        use FirewallManager::*;
        // Allowed combinations:
        assert!(evaluate_coexistence_policy(Firewalld, Refuse).is_ok());
        assert!(evaluate_coexistence_policy(Ufw, Refuse).is_ok());
        assert!(evaluate_coexistence_policy(Docker, RequireUnmanaged).is_ok());
        assert!(evaluate_coexistence_policy(Libvirt, RequireUnmanaged).is_ok());
        assert!(evaluate_coexistence_policy(IptablesNft, Coexist).is_ok());
        assert!(evaluate_coexistence_policy(Unknown, Refuse).is_ok());
        assert!(evaluate_coexistence_policy(None, Coexist).is_ok());

        // Mismatches fail closed with the kebab-case discriminant.
        let err = evaluate_coexistence_policy(Firewalld, Coexist).unwrap_err();
        assert_eq!(err.as_kebab_case(), "firewall-coexistence-mismatch");
        let err = evaluate_coexistence_policy(Docker, Coexist).unwrap_err();
        assert_eq!(err.as_kebab_case(), "firewall-coexistence-mismatch");
        let err = evaluate_coexistence_policy(None, Refuse).unwrap_err();
        assert_eq!(err.as_kebab_case(), "firewall-coexistence-mismatch");
    }

    #[test]
    fn detector_clean_host() {
        let probe = DetectorProbe::none();
        assert_eq!(detect_firewall_manager(&probe), FirewallManager::None);
    }

    #[test]
    fn detector_single_manager_unambiguous() {
        let mut probe = DetectorProbe::none();
        probe.firewalld_active = true;
        assert_eq!(detect_firewall_manager(&probe), FirewallManager::Firewalld);
        probe = DetectorProbe::none();
        probe.ufw_active = true;
        assert_eq!(detect_firewall_manager(&probe), FirewallManager::Ufw);
        probe = DetectorProbe::none();
        probe.docker_active = true;
        assert_eq!(detect_firewall_manager(&probe), FirewallManager::Docker);
        probe = DetectorProbe::none();
        probe.libvirt_active = true;
        assert_eq!(detect_firewall_manager(&probe), FirewallManager::Libvirt);
        probe = DetectorProbe::none();
        probe.iptables_reports_nf_tables = true;
        assert_eq!(
            detect_firewall_manager(&probe),
            FirewallManager::IptablesNft
        );
    }

    #[test]
    fn detector_multiple_managers_unknown() {
        let mut probe = DetectorProbe::none();
        probe.firewalld_active = true;
        probe.docker_active = true;
        assert_eq!(detect_firewall_manager(&probe), FirewallManager::Unknown);
    }

    #[test]
    fn usbip_carveout_inserted_before_generic() {
        let batch = add_usbip_firewall_carveout(&BusId::new("1-1.2")).expect("carveout");
        let forward = batch
            .chains
            .iter()
            .find(|c| matches!(c.hook, ChainHook::Forward))
            .expect("forward chain present");
        assert!(
            forward.rules[0].specific_carveout,
            "specific carve-out must be first in chain"
        );
        assert!(
            forward.rules[0].comment.contains("usbip-carveout-1-1.2"),
            "carveout comment carries busid"
        );
        assert!(
            !forward.rules.last().unwrap().specific_carveout,
            "generic rule remains last"
        );
        batch
            .assert_carveout_ordering()
            .expect("ordering preserved");
    }

    #[test]
    fn comment_marker_prefix_on_every_managed_rule() {
        let batch = add_usbip_firewall_carveout(&BusId::new("2-1")).expect("carveout");
        let forward = batch
            .chains
            .iter()
            .find(|c| matches!(c.hook, ChainHook::Forward))
            .expect("forward chain present");
        for rule in &forward.rules {
            assert!(
                rule.comment.starts_with(NftBatch::COMMENT_PREFIX),
                "rule comment '{c}' missing nixling-managed marker",
                c = rule.comment
            );
        }
    }

    #[test]
    fn drift_detection_strips_volatile_fields() {
        let a = br#"{"nftables":[{"rule":{"handle":1,"index":0,"family":"inet"}}]}"#;
        let b = br#"{"nftables":[{"rule":{"handle":99,"index":7,"family":"inet"}}]}"#;
        assert_eq!(hash_inet_nixling_table(a), hash_inet_nixling_table(b));
    }

    #[test]
    fn drift_detection_catches_real_change() {
        let a = br#"{"nftables":[{"rule":{"family":"inet","expr":"accept"}}]}"#;
        let b = br#"{"nftables":[{"rule":{"family":"inet","expr":"drop"}}]}"#;
        assert_ne!(hash_inet_nixling_table(a), hash_inet_nixling_table(b));
    }

    #[test]
    fn nft_error_to_core_error_uses_kebab_case() {
        let err = NftError::FirewallCoexistenceMismatch {
            detected: FirewallManager::Firewalld,
            declared: CoexistencePolicy::Coexist,
        };
        let core = err.to_core_error();
        assert_eq!(
            core.kind(),
            nixling_core::error::Kind::InternalIo,
            "broker maps via InternalIo for now; ADR records the longer-term plan"
        );
    }

    /// Fake backend exercises foreign-rule preservation: seeded foreign
    /// rules MUST NOT appear in the rendered nixling batch (we never
    /// represent them in our typed model), and repeat-apply is stable.
    #[test]
    fn fake_backend_preserves_foreign_rules() {
        let backend = fake::FakeNftBackend::new();
        backend.seed_foreign("ip saddr 192.0.2.10 accept");
        let batch = build_inet_nixling_chains();
        let h1 = backend.apply(&batch).expect("first apply");
        let h2 = backend.apply(&batch).expect("repeat apply");
        assert_eq!(h1, h2, "repeat apply is stable");
        // Foreign rules remain in the backend, untouched.
        assert_eq!(backend.foreign_rules(), vec!["ip saddr 192.0.2.10 accept"]);
    }

    #[test]
    fn fake_backend_refuses_carveout_after_generic() {
        let backend = fake::FakeNftBackend::new();
        let mut batch = build_inet_nixling_chains();
        if let Some(fwd) = batch
            .chains
            .iter_mut()
            .find(|c| matches!(c.hook, ChainHook::Forward))
        {
            // Append a generic rule, then a specific carve-out
            // out-of-order (without going through add_usbip_carveout).
            fwd.rules.push(NftRule {
                expr: "drop".to_owned(),
                comment: format!("{}default-deny", NftBatch::COMMENT_PREFIX),
                specific_carveout: false,
            });
            fwd.rules.push(NftRule {
                expr: "ip saddr 10.0.0.1 accept".to_owned(),
                comment: format!("{}usbip-carveout-3-1", NftBatch::COMMENT_PREFIX),
                specific_carveout: true,
            });
        }
        let err = backend.apply(&batch).unwrap_err();
        assert_eq!(err.as_kebab_case(), "foreign-nft-rule-shadows-nixling");
    }

    /// W3fu2 H3 (test-1 / software-1): idempotency oracle for the
    /// production [`hash_inet_nixling_table`] drift digest. Hashing
    /// the same canonical `nft list table inet nixling -j` output
    /// twice MUST produce the same digest — this is the
    /// apply→dry-run-empty invariant the broker relies on for
    /// drift detection.
    #[test]
    fn idempotency_hash_table_stable() {
        let nft_json = br#"{
            "nftables": [
                { "table": { "family": "inet", "name": "nixling" } },
                { "chain": { "family": "inet", "table": "nixling", "name": "forward", "type": "filter", "hook": "forward", "prio": -150, "policy": "drop" } },
                { "rule": { "family": "inet", "table": "nixling", "chain": "forward", "expr": [ { "match": {} } ] } }
            ]
        }"#;
        let h1 = hash_inet_nixling_table(nft_json);
        let h2 = hash_inet_nixling_table(nft_json);
        assert_eq!(h1, h2, "same input → same digest");
    }

    /// W3fu2 H3 (test-1 / software-1): idempotency oracle for the
    /// canonical-hash discipline. Kernel-assigned `handle` and `index`
    /// fields in the `nft list table inet nixling -j` output are
    /// runtime-volatile; canonicalization strips them so two textually
    /// different dumps describing the same logical table produce the
    /// same digest. Without this invariant the broker would flag false
    /// drift on every kernel re-add.
    #[test]
    fn idempotency_hash_volatile_stripped() {
        let without_volatile = br#"{
            "nftables": [
                { "table": { "family": "inet", "name": "nixling" } },
                { "chain": { "family": "inet", "table": "nixling", "name": "forward", "type": "filter", "hook": "forward", "prio": -150, "policy": "drop" } }
            ]
        }"#;
        let with_volatile = br#"{
            "nftables": [
                { "table": { "family": "inet", "name": "nixling", "handle": 17 } },
                { "chain": { "family": "inet", "table": "nixling", "name": "forward", "type": "filter", "hook": "forward", "prio": -150, "policy": "drop", "handle": 3, "index": 7 } }
            ]
        }"#;
        let h_clean = hash_inet_nixling_table(without_volatile);
        let h_volatile = hash_inet_nixling_table(with_volatile);
        assert_eq!(
            h_clean, h_volatile,
            "handle/index are volatile and must not change the canonical digest"
        );
    }
}
