//! `d2b debug`: explain a Zone's resource rows in one command.
//!
//! Read-only by construction: the command reads rows through the same
//! `Get`/`List` reads every other resource command uses and composes what it
//! reads. It adds no wire surface and no daemon-side projection, so the
//! explanation it prints is the same one any client holding read access can
//! reproduce.
//!
//! Three layers, deliberately separate:
//!
//! 1. [`read_zone`] turns the zone's converted-type catalog into an
//!    [`ObservedRow`] set, one type-scoped read at a time, recording a
//!    degraded read instead of failing the report.
//! 2. [`compose`] is a pure function from that row set to a [`DebugReport`]:
//!    the ownership tree, the expansion rule, cycles, and owner-absent rows.
//! 3. [`render_human`] and [`report_json`] render the one composition, so the
//!    machine-readable record can never disagree with the tree.

use std::collections::{BTreeMap, HashSet};

use serde_json::{Value, json};

use crate::context::{
    OutputMode, RequestDeadline, ZoneContext, converted_resource_types, parse_resource_ref,
};
use crate::dispatch::GenericListArgs;
use crate::print_stdout;
use crate::resource::request_list;

/// Rows read in one `d2b debug` invocation before the command refuses.
///
/// The bound exists so a pathological zone fails with a named refusal rather
/// than an unbounded walk; it is generous enough that an ordinary zone never
/// meets it.
pub(crate) const ROW_BUDGET: usize = 4096;

/// Page size for each type-scoped read.
const PAGE_SIZE: u32 = 200;

/// Arguments for `d2b debug`.
#[derive(Debug, clap::Args, Clone)]
pub(crate) struct DebugArgs {
    /// Zone to explain.
    // The argument id is deliberately not `zone`: a positional sharing the
    // global flag's id makes clap drop `--zone` inside this subcommand.
    #[arg(value_name = "ZONE")]
    pub(crate) zone_ref: String,
    /// Resource to explain, as `<ResourceType>/<name>`. Absent renders the zone.
    #[arg(value_name = "TYPE/NAME")]
    pub(crate) resource_ref: Option<String>,
    /// Expand subtrees whose rows are all Ready.
    #[arg(long)]
    pub(crate) all: bool,
}

/// One row as the debug report sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservedRow {
    /// `type/name` as the row reports it.
    pub(crate) reference: String,
    pub(crate) uid: String,
    pub(crate) generation: u64,
    /// The generation the row's status was published for; `None` when the row
    /// carries no status.
    pub(crate) status_generation: Option<u64>,
    /// The wire phase the row's own read path reports.
    pub(crate) phase: String,
    /// The authored owner reference, when the row has one.
    pub(crate) owner_ref: Option<String>,
    /// The last structured driver failure, when the status carries one.
    pub(crate) failure: Option<FailureSummary>,
}

/// The structured failure detail an unready row expands to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FailureSummary {
    pub(crate) code: String,
    pub(crate) operation: Option<String>,
    pub(crate) stage: Option<String>,
    pub(crate) outcome: Option<String>,
    pub(crate) retryable: Option<bool>,
}

/// A type read the report could not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DegradedRead {
    pub(crate) resource_type: String,
    pub(crate) detail: String,
}

/// A read that could not proceed at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DebugReadError {
    pub(crate) class: &'static str,
    pub(crate) message: String,
    pub(crate) exit_code: i32,
}

/// Everything the read layer observed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ObservedZone {
    pub(crate) rows: Vec<ObservedRow>,
    pub(crate) degraded: Vec<DegradedRead>,
    /// The revision each read reported, in read order.
    pub(crate) revisions: Vec<u64>,
}

/// A node in the composed ownership tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DebugNode {
    pub(crate) row: ObservedRow,
    pub(crate) children: Vec<DebugNode>,
    /// The row's owner is not part of the read set.
    pub(crate) owner_absent: bool,
    /// This node repeats an ancestor, so the walk stopped here.
    pub(crate) cycle: bool,
}

/// The composed report both renderers read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DebugReport {
    pub(crate) zone: String,
    pub(crate) roots: Vec<DebugNode>,
    pub(crate) degraded: Vec<DegradedRead>,
    pub(crate) revisions: Vec<u64>,
    /// Total rows the report accounts for, including collapsed ones.
    pub(crate) total_rows: usize,
}

impl DebugReport {
    /// True when the report was assembled from reads at different revisions.
    pub(crate) fn composite(&self) -> bool {
        self.revisions
            .first()
            .is_some_and(|first| self.revisions.iter().any(|revision| revision != first))
    }

    /// How many rows the node's subtree holds, the node included. A cycle
    /// marker repeats a row already counted above it, so it contributes
    /// nothing to the total.
    pub(crate) fn subtree_rows(node: &DebugNode) -> usize {
        let own = usize::from(!node.cycle);
        own + node
            .children
            .iter()
            .map(DebugReport::subtree_rows)
            .sum::<usize>()
    }

    /// Counts rows by phase across the whole report.
    pub(crate) fn phase_counts(&self) -> BTreeMap<String, usize> {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for root in &self.roots {
            count_phases(root, &mut counts);
        }
        counts
    }
}

/// Tally one subtree's phases, collapsed rows included. A cycle marker
/// repeats a row already tallied above it.
fn count_phases(node: &DebugNode, counts: &mut BTreeMap<String, usize>) {
    if !node.cycle {
        *counts.entry(node.row.phase.clone()).or_default() += 1;
    }
    for child in &node.children {
        count_phases(child, counts);
    }
}

/// The plane a row is served by, derived from its type.
///
/// The envelope carries no plane, and the registry is the authority for the
/// mapping, so the renderer derives it rather than reading it.
fn row_plane(reference: &str) -> &'static str {
    let resource_type = reference
        .split_once('/')
        .map(|(resource_type, _)| resource_type)
        .unwrap_or(reference);
    match d2b_contracts::identity::resource_plane(resource_type) {
        d2b_contracts::identity::ResourcePlane::Manager => "manager",
        d2b_contracts::identity::ResourcePlane::Legacy => "legacy",
    }
}

/// Whether a phase counts as settled for the collapse rule.
fn settled(phase: &str) -> bool {
    matches!(phase, "Ready" | "Succeeded")
}

/// A node expands when it is not settled, when its status is unknown, or when
/// its owner is outside the read set.
fn expands(node: &DebugNode) -> bool {
    !settled(&node.row.phase) || node.row.status_generation.is_none() || node.owner_absent
}

/// Whether every node in the subtree is settled.
fn subtree_settled(node: &DebugNode) -> bool {
    settled(&node.row.phase) && node.children.iter().all(subtree_settled)
}

/// Render the subtree, collapsing settled subtrees that carry no reason to
/// expand.
fn render_node(node: &DebugNode, out: &mut String, indent: usize, expand_all: bool) {
    let pad = "  ".repeat(indent);
    let row = &node.row;
    out.push_str(&format!(
        "{pad}{} plane={} phase={} gen={} statusGen={}",
        row.reference,
        row_plane(&row.reference),
        row.phase,
        row.generation,
        row.status_generation
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    ));
    if let Some(owner) = &row.owner_ref {
        out.push_str(&format!(" owner={owner}"));
    }
    if node.owner_absent {
        out.push_str(" owner-absent");
    }
    if node.cycle {
        out.push_str(" cycle");
    }
    out.push('\n');

    let expanded = expand_all || expands(node);
    if !expanded && subtree_settled(node) {
        // A cycle marker repeats a row counted above it, so it contributes
        // nothing here and a subtree of nothing but markers has no rollup.
        let hidden = DebugReport::subtree_rows(node).saturating_sub(1);
        if hidden > 0 {
            out.push_str(&format!("{pad}  ({hidden} rows Ready)\n"));
        }
        return;
    }
    if let Some(failure) = &row.failure {
        out.push_str(&format!(
            "{pad}  failure code={} outcome={} operation={} stage={} retryable={}\n",
            failure.code,
            failure.outcome.as_deref().unwrap_or("unknown"),
            failure.operation.as_deref().unwrap_or("unknown"),
            failure.stage.as_deref().unwrap_or("unknown"),
            failure
                .retryable
                .map(|retryable| retryable.to_string())
                .unwrap_or_else(|| "unknown".to_owned()),
        ));
        if let Some(kind) = d2b_contracts::failure_kinds::FailureKind::from_code(&failure.code) {
            out.push_str(&format!("{pad}  means: {}\n", kind.means()));
            out.push_str(&format!("{pad}  likely cause: {}\n", kind.likely_cause()));
        }
    } else if let Some(reason) = absent_reason(row) {
        out.push_str(&format!("{pad}  {reason}\n"));
    }
    for child in &node.children {
        render_node(child, out, indent + 1, expand_all);
    }
}

/// Why an unready row carries no failure detail.
fn absent_reason(row: &ObservedRow) -> Option<String> {
    if settled(&row.phase) {
        return None;
    }
    match row.status_generation {
        None => Some("no status published for this row".to_owned()),
        Some(status_generation) if status_generation != row.generation => Some(format!(
            "status does not describe this spec: published for generation {status_generation}, row is at {}",
            row.generation
        )),
        Some(_) => Some(format!("phase {} carries no attached failure", row.phase)),
    }
}

/// Render the human tree.
pub(crate) fn render_human(report: &DebugReport, expand_all: bool) -> String {
    let mut out = String::new();
    let counts = report
        .phase_counts()
        .into_iter()
        .map(|(phase, count)| format!("{phase}={count}"))
        .collect::<Vec<_>>()
        .join(" ");
    out.push_str(&format!(
        "zone {} rows={} {}\n",
        report.zone, report.total_rows, counts
    ));
    if report.revisions.is_empty() {
        out.push_str("revision unknown\n");
    } else if report.composite() {
        out.push_str(&format!(
            "composite across revisions: {}\n",
            report
                .revisions
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    } else {
        out.push_str(&format!("revision {}\n", report.revisions[0]));
    }
    for degraded in &report.degraded {
        out.push_str(&format!(
            "type {} not read: {}\n",
            degraded.resource_type, degraded.detail
        ));
    }
    if report.roots.is_empty() {
        out.push_str("no rows\n");
        return out;
    }
    for root in &report.roots {
        render_node(root, &mut out, 0, expand_all);
    }
    out
}

/// The complete machine-readable record: every node, every failure, and every
/// degraded read, including the rows the human tree collapsed.
pub(crate) fn report_json(report: &DebugReport) -> Value {
    fn node_json(node: &DebugNode) -> Value {
        json!({
            "ref": node.row.reference,
            "uid": node.row.uid,
            "plane": row_plane(&node.row.reference),
            "phase": node.row.phase,
            "generation": node.row.generation,
            "statusGeneration": node.row.status_generation,
            "ownerRef": node.row.owner_ref,
            "ownerAbsent": node.owner_absent,
            "cycle": node.cycle,
            "mode": if node.cycle { "cycle" } else { "row" },
            "children": node.children.iter().map(node_json).collect::<Vec<_>>(),
            "failure": node.row.failure.as_ref().map(|failure| json!({
                "code": failure.code,
                "operation": failure.operation,
                "stage": failure.stage,
                "outcome": failure.outcome,
                "retryable": failure.retryable,
            })).unwrap_or(Value::Null),
        })
    }
    json!({
        "zoneRef": format!("Zone/{}", report.zone),
        "rows": report.total_rows,
        "revisions": report.revisions,
        "composite": report.composite(),
        "degradedReads": report.degraded.iter().map(|degraded| json!({
            "resourceType": degraded.resource_type,
            "detail": degraded.detail,
        })).collect::<Vec<_>>(),
        "roots": report.roots.iter().map(node_json).collect::<Vec<_>>(),
    })
}

/// Extract an observed row from a read envelope, or name why it could not be.
fn observed_row(value: &Value) -> Result<ObservedRow, String> {
    let resource_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| "envelope carries no resource type".to_owned())?;
    let metadata = value
        .get("metadata")
        .ok_or_else(|| "envelope carries no metadata".to_owned())?;
    let name = metadata
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "metadata carries no name".to_owned())?;
    let reference = format!("{resource_type}/{name}");
    let uid = metadata
        .get("uid")
        .and_then(Value::as_str)
        .ok_or_else(|| "metadata carries no uid".to_owned())?;
    let generation = metadata
        .get("generation")
        .and_then(Value::as_u64)
        .ok_or_else(|| "metadata carries no generation".to_owned())?;
    let owner_ref = metadata
        .get("ownerRef")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let status = value.get("status");
    let phase = status
        .and_then(|status| status.get("phase"))
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_owned();
    let status_generation = status
        .and_then(|status| status.get("statusGeneration"))
        .and_then(Value::as_u64);
    let failure = status
        .and_then(|status| status.get("resource"))
        .and_then(|resource| resource.get("driverFailure"))
        .map(|failure| FailureSummary {
            code: failure
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            operation: failure
                .get("operation")
                .and_then(Value::as_str)
                .map(str::to_owned),
            stage: failure
                .get("stage")
                .and_then(Value::as_str)
                .map(str::to_owned),
            outcome: failure
                .get("outcome")
                .and_then(Value::as_str)
                .map(str::to_owned),
            retryable: failure.get("retryable").and_then(Value::as_bool),
        });
    Ok(ObservedRow {
        reference: reference.to_owned(),
        uid: uid.to_owned(),
        generation,
        status_generation,
        phase,
        owner_ref,
        failure,
    })
}

/// Rows carried by a list response.
fn rows_of(value: &Value) -> Vec<&Value> {
    value
        .get("resources")
        .and_then(Value::as_array)
        .map(|rows| rows.iter().collect())
        .unwrap_or_default()
}

/// The error class a CLI failure carries, as its message prefix.
fn failure_class(message: &str) -> &str {
    match message.split_once(':') {
        Some((class, _)) if !class.contains(' ') => class,
        _ => "debug-read-refused",
    }
}

/// Classes that mean the zone is not answering at all, so the report has no
/// rows to explain and must refuse rather than print an empty zone.
fn aborts_the_report(class: &str) -> bool {
    matches!(
        class,
        "zone-unavailable" | "deadline-exceeded" | "exec-auth-error"
    )
}

/// Read one resource type, walking every page.
fn read_type(
    context: &ZoneContext,
    resource_type: &str,
    mode: OutputMode,
    deadline: RequestDeadline,
    budget: &mut usize,
    zone: &mut ObservedZone,
) -> Result<(), DebugReadError> {
    let mut page_token: Option<String> = None;
    loop {
        let args = GenericListArgs {
            resource_type: resource_type.to_owned(),
            execution_ref: None,
            domain: None,
            phase: None,
            label_selector: None,
            updates: false,
            page_token: page_token.clone(),
            limit: Some(PAGE_SIZE),
        };
        let value = request_list(context, &args, mode, deadline).map_err(|failure| {
            let class = failure_class(&failure.message);
            DebugReadError {
                class: if aborts_the_report(class) {
                    "zone-unavailable"
                } else {
                    "debug-read-refused"
                },
                message: failure.message,
                exit_code: 1,
            }
        })?;
        if let Some(revision) = value.get("snapshotRevision").and_then(Value::as_u64) {
            zone.revisions.push(revision);
        }
        for row in rows_of(&value) {
            if *budget == 0 {
                return Err(DebugReadError {
                    class: "debug-read-exhausted",
                    message: format!(
                        "read budget of {ROW_BUDGET} rows exhausted at {resource_type}"
                    ),
                    exit_code: 1,
                });
            }
            match observed_row(row) {
                Ok(row) => {
                    zone.rows.push(row);
                    *budget -= 1;
                }
                Err(detail) => {
                    zone.degraded.push(DegradedRead {
                        resource_type: resource_type.to_owned(),
                        detail,
                    });
                    return Ok(());
                }
            }
        }
        match value.get("nextCursor").and_then(Value::as_str) {
            Some(next) if !next.is_empty() => page_token = Some(next.to_owned()),
            _ => return Ok(()),
        }
    }
}

/// Read every converted type in the zone, or the one type of a named row.
pub(crate) fn read_zone(
    context: &ZoneContext,
    args: &DebugArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<ObservedZone, DebugReadError> {
    let mut budget = ROW_BUDGET;
    let mut zone = ObservedZone::default();
    let mut types: Vec<String> = converted_resource_types()
        .iter()
        .map(|resource_type| (*resource_type).to_owned())
        .collect();
    if let Some(reference) = &args.resource_ref {
        let parsed = parse_resource_ref(reference, None).map_err(|failure| DebugReadError {
            class: "ref-invalid",
            message: failure.message,
            exit_code: 2,
        })?;
        let named = parsed.resource_type().as_str().to_owned();
        if !types.contains(&named) {
            types.push(named);
        }
    }
    for resource_type in &types {
        match read_type(
            context,
            resource_type,
            mode,
            deadline,
            &mut budget,
            &mut zone,
        ) {
            Ok(()) => {}
            // An exhausted budget and a zone that is not answering are not
            // per-type outcomes: the first cannot be degraded into a smaller
            // report, and the second means there is no report to render.
            Err(error) if error.class == "zone-unavailable" || error.class == "debug-read-exhausted" => {
                return Err(error);
            }
            Err(error) => zone.degraded.push(DegradedRead {
                resource_type: resource_type.clone(),
                detail: error.message,
            }),
        }
    }
    Ok(zone)
}

/// Compose the report from the observed rows: a pure function of the row set.
pub(crate) fn compose(zone: &ObservedZone, zone_name: &str, selected: Option<&str>) -> DebugReport {
    // Ownership is stored as a `Type/name` reference, not a uid, so the
    // index is keyed on the same reference the named-row lookup uses.
    let by_ref: BTreeMap<&str, usize> = zone
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| (row.reference.as_str(), index))
        .collect();
    let mut children_of: BTreeMap<Option<usize>, Vec<usize>> = BTreeMap::new();
    let mut owner_absent: HashSet<usize> = HashSet::new();
    for (index, row) in zone.rows.iter().enumerate() {
        let owner = row
            .owner_ref
            .as_deref()
            .and_then(|owner| by_ref.get(owner).copied());
        if row.owner_ref.is_some() && owner.is_none() {
            owner_absent.insert(index);
        }
        children_of.entry(owner).or_default().push(index);
    }

    // The rows this report covers: the named row's subtree, or every root.
    let selected_index = selected.and_then(|reference| {
        zone.rows
            .iter()
            .position(|row| row.reference == reference)
    });

    // One recursive walk. `path` holds the current root's ancestors, so a
    // node already on the path is a cycle: it renders marked and is not
    // descended into. Each root gets a fresh path, and an index already
    // covered by an earlier root is not re-rooted, so the report counts each
    // row once.
    fn walk(
        index: usize,
        zone: &ObservedZone,
        children_of: &BTreeMap<Option<usize>, Vec<usize>>,
        owner_absent: &HashSet<usize>,
        path: &mut HashSet<usize>,
        visited: &mut HashSet<usize>,
    ) -> DebugNode {
        let row = zone.rows[index].clone();
        let on_path = !path.insert(index);
        if !on_path {
            visited.insert(index);
        }
        let mut children = Vec::new();
        if let Some(child_indexes) = children_of.get(&Some(index)) {
            for child in child_indexes {
                if visited.contains(child) {
                    // Walked already, and not an ancestor, so this is a
                    // second path to one row: render it marked, do not
                    // descend, and do not count it twice.
                    children.push(DebugNode {
                        row: zone.rows[*child].clone(),
                        children: Vec::new(),
                        owner_absent: owner_absent.contains(child),
                        cycle: true,
                    });
                } else {
                    children.push(walk(
                        *child,
                        zone,
                        children_of,
                        owner_absent,
                        path,
                        visited,
                    ));
                }
            }
        }
        if !on_path {
            path.remove(&index);
        }
        DebugNode {
            row,
            children,
            owner_absent: owner_absent.contains(&index),
            cycle: on_path,
        }
    }

    // Candidate roots: rows with no owner, rows whose owner is outside the
    // read set, and rows no root reaches (a mutual-ownership cycle roots
    // itself, so it stays visible instead of being dropped).
    let mut candidates: Vec<usize> = zone
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            row.owner_ref
                .as_deref()
                .and_then(|owner| by_ref.get(owner).copied())
                .is_none()
        })
        .map(|(index, _)| index)
        .collect();
    candidates.sort_unstable();

    let roots: Vec<DebugNode> = match selected_index {
        Some(index) => {
            let mut path = HashSet::new();
            let mut visited = HashSet::new();
            let mut node = walk(index, zone, &children_of, &HashSet::new(), &mut path, &mut visited);
            // A named row's owner may live outside the report by
            // construction, so it never renders as owner-absent.
            node.owner_absent = false;
            vec![node]
        }
        None => {
            let mut visited: HashSet<usize> = HashSet::new();
            let mut roots = Vec::new();
            for index in candidates.iter().copied().chain(0..zone.rows.len()) {
                if visited.contains(&index) {
                    continue;
                }
                let mut path = HashSet::new();
                roots.push(walk(
                    index,
                    zone,
                    &children_of,
                    &owner_absent,
                    &mut path,
                    &mut visited,
                ));
            }
            roots
        }
    };

    let total_rows = roots.iter().map(DebugReport::subtree_rows).sum();
    DebugReport {
        zone: zone_name.to_owned(),
        roots,
        degraded: zone.degraded.clone(),
        revisions: zone.revisions.clone(),
        total_rows,
    }
}

/// Run the read-only debug command.
pub(crate) fn run(
    context: &ZoneContext,
    args: &DebugArgs,
    mode: OutputMode,
    deadline: RequestDeadline,
) -> Result<i32, crate::CliFailure> {
    // The positional zone is the route's authority, so `d2b debug prod`
    // connects to `prod`. An explicitly selected global zone that disagrees
    // with it is a usage error, refused before any request.
    if context.has_explicit_zone() && context.zone_name() != args.zone_ref {
        return Err(context.failure(
            "ref-invalid",
            "debug zone disagrees with the selected Zone",
            mode,
            2,
        ));
    }
    let selected = match &args.resource_ref {
        Some(reference) => {
            let parsed = parse_resource_ref(reference, None)?;
            Some(parsed.to_canonical_string())
        }
        None => None,
    };
    let observed = match read_zone(context, args, mode, deadline) {
        Ok(observed) => observed,
        Err(error) => {
            return Err(context.failure(error.class, &error.message, mode, error.exit_code));
        }
    };
    if let Some(reference) = &selected
        && !observed.rows.iter().any(|row| &row.reference == reference)
    {
        // A type whose read did not complete cannot answer for the named row:
        // reporting not-found would present a refused read as an absent row.
        let selected_type = reference.split_once('/').map(|(resource_type, _)| resource_type);
        if let Some(degraded) = selected_type.and_then(|resource_type| {
            observed
                .degraded
                .iter()
                .find(|degraded| degraded.resource_type == resource_type)
        }) {
            return Err(context.failure("debug-read-refused", &degraded.detail, mode, 1));
        }
        return Err(context.failure(
            "resource-not-found",
            "resource was not found",
            mode,
            1,
        ));
    }
    let report = compose(&observed, context.zone_name(), selected.as_deref());
    if mode.is_json() {
        let mut rendered = serde_json::to_string_pretty(&report_json(&report)).map_err(|_| {
            context.failure("internal-error", "failed to render debug report", mode, 1)
        })?;
        rendered.push('\n');
        print_stdout(&rendered);
    } else {
        print_stdout(&render_human(&report, args.all));
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(
        reference: &str,
        uid: &str,
        owner: Option<&str>,
        phase: &str,
        generation: u64,
        status_generation: Option<u64>,
    ) -> ObservedRow {
        ObservedRow {
            reference: reference.to_owned(),
            uid: uid.to_owned(),
            generation,
            status_generation,
            phase: phase.to_owned(),
            owner_ref: owner.map(str::to_owned),
            failure: None,
        }
    }

    fn zone(rows: Vec<ObservedRow>) -> ObservedZone {
        ObservedZone {
            rows,
            degraded: Vec::new(),
            revisions: vec![7],
        }
    }

    #[test]
    fn a_failed_grandchild_keeps_its_ready_ancestors_expanded() {
        let mut failed = row("Process/worker", "uid-3", Some("Endpoint/api"), "Failed", 1, Some(1));
        failed.failure = Some(FailureSummary {
            code: "process-spec-invalid".to_owned(),
            operation: Some("Reconcile".to_owned()),
            stage: Some("validate".to_owned()),
            outcome: Some("refused".to_owned()),
            retryable: Some(false),
        });
        let observed = zone(vec![
            row("Guest/sandbox", "uid-1", None, "Ready", 1, Some(1)),
            row("Endpoint/api", "uid-2", Some("Guest/sandbox"), "Ready", 1, Some(1)),
            failed,
        ]);
        let report = compose(&observed, "dev", None);
        let human = render_human(&report, false);

        // The whole path stays expanded, and the count still accounts for
        // every row the read returned.
        assert_eq!(report.total_rows, 3);
        assert!(human.contains("Process/worker"), "{human}");
        assert!(human.contains("failure code=process-spec-invalid"), "{human}");
        assert!(human.contains("means:"), "the shared registry supplies the prose");
        assert!(
            human.contains("Process/worker plane=manager"),
            "every node names the plane it is served by: {human}"
        );
        assert_eq!(report_json(&report)["roots"][0]["plane"], "manager");
        assert!(
            !human.contains("rows Ready)"),
            "nothing collapses on the path to the failure: {human}"
        );
    }

    #[test]
    fn a_fully_settled_subtree_collapses_to_one_rollup() {
        let observed = zone(vec![
            row("Guest/healthy", "uid-1", None, "Ready", 1, Some(1)),
            row("Endpoint/api", "uid-2", Some("Guest/healthy"), "Ready", 1, Some(1)),
        ]);
        let report = compose(&observed, "dev", None);
        let collapsed = render_human(&report, false);
        let expanded = render_human(&report, true);

        assert!(collapsed.contains("(1 rows Ready)"), "{collapsed}");
        assert!(!collapsed.contains("Endpoint/api"), "{collapsed}");
        assert_eq!(report.total_rows, 2, "a collapsed row is still accounted for");
        assert!(expanded.contains("Endpoint/api"), "{expanded}");
        // Both renderings read the one composition, so the machine record
        // carries the collapse's Node without expanding the human tree.
        let machine = report_json(&report);
        assert_eq!(machine["rows"], 2);
        assert_eq!(machine["roots"][0]["children"][0]["ref"], "Endpoint/api");
    }

    #[test]
    fn a_row_whose_owner_is_outside_the_read_set_is_flagged_and_rooted() {
        let observed = zone(vec![row(
            "Volume/orphan",
            "uid-9",
            Some("Volume/ghost"),
            "Ready",
            1,
            Some(1),
        )]);
        let report = compose(&observed, "dev", None);
        let root = &report.roots[0];
        assert!(root.owner_absent);
        assert!(render_human(&report, false).contains("owner-absent"));
        assert_eq!(report_json(&report)["roots"][0]["ownerAbsent"], true);
    }

    #[test]
    fn an_ownership_cycle_terminates_and_is_marked() {
        let observed = zone(vec![
            row("Volume/a", "uid-a", Some("Volume/b"), "Ready", 1, Some(1)),
            row("Volume/b", "uid-b", Some("Volume/a"), "Ready", 1, Some(1)),
        ]);
        let report = compose(&observed, "dev", None);
        let human = render_human(&report, true);
        assert!(human.contains("cycle"), "{human}");
        assert_eq!(report.total_rows, 2, "the repeated node is marked, not counted twice");
    }

    #[test]
    fn an_unready_row_without_a_failure_names_why() {
        let observed = zone(vec![row("Host/local", "uid-1", None, "Pending", 3, Some(2))]);
        let report = compose(&observed, "dev", None);
        let human = render_human(&report, false);
        assert!(
            human.contains("status does not describe this spec"),
            "a skew is named rather than left blank: {human}"
        );

        let never = zone(vec![row("Host/local", "uid-1", None, "Pending", 1, None)]);
        let never_human = render_human(&compose(&never, "dev", None), false);
        assert!(never_human.contains("no status published"), "{never_human}");
    }

    #[test]
    fn an_unreachable_zone_aborts_rather_than_degrading_every_type() {
        // A zone that is not answering is not 33 degraded type reads; the
        // report must refuse instead of printing an empty zone.
        assert!(aborts_the_report(&failure_class(
            "zone-unavailable: Zone runtime is unavailable"
        )));
        assert!(aborts_the_report(&failure_class(
            "deadline-exceeded: request deadline expired"
        )));
        // A per-type answer stays a per-type outcome.
        assert!(!aborts_the_report(&failure_class(
            "capability-unavailable: resource types are not served"
        )));
        // An unprefixed message falls back to the per-type class rather than
        // aborting a report on an unrecognized string.
        assert_eq!(failure_class("unprefixed failure"), "debug-read-refused");
    }

    #[test]
    fn a_degraded_type_read_is_one_named_entry_and_does_not_read_as_empty() {
        let mut observed = zone(vec![row("Host/local", "uid-1", None, "Ready", 1, Some(1))]);
        observed.degraded.push(DegradedRead {
            resource_type: "Provider/system-core".to_owned(),
            detail: "capability-unavailable".to_owned(),
        });
        let report = compose(&observed, "dev", None);
        let human = render_human(&report, false);
        assert!(human.contains("type Provider/system-core not read"), "{human}");
        assert_eq!(
            report_json(&report)["degradedReads"][0]["resourceType"],
            "Provider/system-core"
        );
    }

    #[test]
    fn an_empty_row_set_renders_as_a_successful_empty_report() {
        let report = compose(&ObservedZone::default(), "dev", None);
        let human = render_human(&report, false);
        assert!(human.contains("rows=0"), "{human}");
        assert!(human.contains("no rows"), "{human}");
        assert_eq!(report_json(&report)["roots"].as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn the_named_form_renders_only_the_named_subtree() {
        let observed = zone(vec![
            row("Guest/sandbox", "uid-1", None, "Ready", 1, Some(1)),
            row("Process/worker", "uid-2", Some("Guest/sandbox"), "Failed", 1, Some(1)),
            row("Guest/other", "uid-3", None, "Ready", 1, Some(1)),
        ]);
        let report = compose(&observed, "dev", Some("Guest/sandbox"));
        let human = render_human(&report, true);
        assert!(human.contains("Guest/sandbox"), "{human}");
        assert!(human.contains("Process/worker"), "a named row's child is shown: {human}");
        assert!(!human.contains("Guest/other"), "{human}");
        assert_eq!(report.total_rows, 2);
    }

    #[test]
    fn the_report_marks_itself_composite_across_revisions() {
        let mut observed = zone(vec![row("Host/local", "uid-1", None, "Ready", 1, Some(1))]);
        observed.revisions = vec![4, 9];
        let report = compose(&observed, "dev", None);
        assert!(report.composite());
        assert!(render_human(&report, false).contains("composite across revisions: 4, 9"));
        assert_eq!(report_json(&report)["composite"], true);

        observed.revisions = vec![4, 4];
        assert!(!compose(&observed, "dev", None).composite());
    }
}
