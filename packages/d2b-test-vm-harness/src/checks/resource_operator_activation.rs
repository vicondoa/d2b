//! The authenticated Resource operator and framework census check, ported
//! from its fixture.
//!
//! It reaches the installed d2b CLI, the public socket, the systemd restart
//! boundary, and the framework-declared daemon unit surface in a real NixOS
//! guest - the things the native controller canaries cannot reach. It is
//! deliberately separate from those canaries, and the census does not sweep
//! every d2b-prefixed unit on an operator host, because optional or managed
//! infrastructure is outside its ownership.
//!
//! The assertions are the fixture's, in the fixture's order and with the
//! fixture's own command text and bounds: the host row, the user row, the
//! provider row and the controller process all reach `Ready` with a settled
//! generation, the debug surface reports the rows it could not read rather
//! than presenting them as empty, a row that does not exist and a user
//! without the role are both refused, and after a `d2bd` restart the
//! controller process is adopted rather than restarted. The census at the
//! end compares the live unit surface with the framework's own declaration.
//!
//! The fixture's row projections ride with it: `diag_projection` is the jq
//! program the diagnostics print a timed-out wait's rows with, and the two
//! row builders are the fixture's own `live_rows` and `saved_rows`.
//!
//! The guest is the reusable daemon node plus the fixture's own contributions
//! (nftables, the acceptance artifacts and zones, the two users, and `jq`),
//! declared in `nix/test-support/host-integration-node.nix`.
//!
//! `start_all()` is not restated here: it is the lane's own boot of the
//! guest the check runs against.

use std::{collections::BTreeSet, time::Duration};

use crate::legacy::{DiagRow, GuestControl, LegacyError, LegacyResult};

/// The bound the two units' waits get, the fixture's own.
const UNIT_BOUND: Duration = Duration::from_secs(180);

/// The bound the broker socket's wait gets, the fixture's own.
const SOCKET_BOUND: Duration = Duration::from_secs(30);

/// The bound the public socket's file waits get, the fixture's own.
const PUBLIC_SOCKET_BOUND: Duration = Duration::from_secs(30);

/// The bound the command waits get, the fixture's own.
const WAIT: Duration = Duration::from_secs(60);

/// The bound the controller pid wait gets, the fixture's own.
const PID_BOUND: Duration = Duration::from_secs(30);

/// The framework-declared daemon units the census compares the live system
/// with.
const REQUIRED_UNITS: [&str; 3] = ["d2bd.service", "d2b-broker.socket", "d2b-broker.service"];

/// The row projection the shared diagnostics print on a timed-out wait; it
/// mirrors the fields each wait asserts on (issue #513).
const DIAG_PROJECTION: &str = concat!(
    "[.resources[] | {type: .type, name: .metadata.name, ",
    "owner: .metadata.ownerRef, uid: .metadata.uid, ",
    "gen: .metadata.generation, phase: .status.phase, ",
    "obs: .status.observedGeneration, ",
    "conditions: [.status.conditions[]? | {type: .type, reason: .reason}]}]",
);

/// The provider session line `d2bd` logs once the external controller's
/// ResourceV3 session is live.
const PROVIDER_SESSION_LIVE: &str = concat!(
    "journalctl -u d2bd.service --no-pager -o cat ",
    "| grep -F 'external Provider controller ResourceV3 session live'",
);

/// The host row must be `Ready` with its generation settled, before the
/// restart.
const HOST_ROW: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Host >/run/d2b-host-before.json && ",
    "jq -e '.resources[] | select(.type == \"Host\" and ",
    ".metadata.name == \"host-system\") | ",
    "(.status.phase == \"Ready\" and ",
    ".status.observedGeneration == .metadata.generation)' ",
    "/run/d2b-host-before.json",
);

/// The user row must be `Ready` with its generation settled.
const USER_ROW: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list User ",
    ">/run/d2b-user-before.json && ",
    "jq -e '.resources[] | select(.type == \"User\" and ",
    ".metadata.name == \"alice\") | ",
    "(.status.phase == \"Ready\" and ",
    ".status.observedGeneration == .metadata.generation)' ",
    "/run/d2b-user-before.json",
);

/// The provider row must carry an identity and a generation.
const PROVIDER_ROW: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Provider ",
    ">/run/d2b-provider-before.json && ",
    "jq -e '.resources[] | select(.type == \"Provider\" and ",
    ".metadata.name == \"network-local\") | ",
    "(.metadata.uid != null and .metadata.generation > 0)' ",
    "/run/d2b-provider-before.json",
);

/// Exactly one controller process, owned by the provider, `Ready` and
/// settled.
const CONTROLLER_PROCESS: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Process ",
    ">/run/d2b-process-before.json && ",
    "jq -e '([.resources[] | select(.type == \"Process\" and ",
    ".metadata.ownerRef == \"Provider/network-local\")] | length == 1) and ",
    "(.resources[] | select(.type == \"Process\" and ",
    ".metadata.ownerRef == \"Provider/network-local\") | ",
    "(.metadata.uid != null and .metadata.generation > 0 and ",
    ".status.phase == \"Ready\" and ",
    ".status.observedGeneration == .metadata.generation))' ",
    "/run/d2b-process-before.json",
);

/// Exactly one process whose command is the acceptance controller.
const ONE_CONTROLLER_PROCESS: &str = concat!(
    "test \"$(ps -eo pid=,args= | awk '$NF ~ /acceptance-controller$/ {print $1}' ",
    "| wc -l)\" -eq 1",
);

/// The first such process's pid.
const CONTROLLER_PID: &str =
    "ps -eo pid=,args= | awk '$NF ~ /acceptance-controller$/ {print $1; exit}'";

/// A dump of the processes whose command mentions the controller.
const CONTROLLER_PROCESSES: &str =
    "ps -eo pid=,args= | grep acceptance-controller || true";

/// The debug surface, on a zone this fixture has just settled. `alice` can
/// read Process, Host and User but not Zone, so the report is expected to name
/// exactly those types it could not read rather than present them as empty,
/// and to still explain the rows it did read.
const DEBUG_ZONE_REPORT: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json debug work >/run/d2b-debug-zone.json && ",
    "jq -e '.zoneRef == \"Zone/work\" ",
    "and (.degradedReads | map(.resourceType) | index(\"Zone\")) != null ",
    "and (.degradedReads | map(.resourceType) | index(\"Process\")) == null ",
    "and (.degradedReads | map(.resourceType) | index(\"Host\")) == null' ",
    "/run/d2b-debug-zone.json >/dev/null && ",
    "jq -e '[.roots[].ref] | index(\"Host/host-system\") != null ",
    "and index(\"User/alice\") != null' ",
    "/run/d2b-debug-zone.json >/dev/null",
);

/// The named-row half of the debug surface: one controller row, read by name,
/// with no children.
const DEBUG_NAMED_ROW: &str = concat!(
    "name=$(runuser -u alice -- env ",
    "D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Process | ",
    "jq -r '.resources[] | select(.metadata.name | startswith(",
    "\"controller-\")) | .metadata.name') && ",
    "test -n \"$name\" && ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json debug work \"Process/$name\" ",
    ">/run/d2b-debug-row.json && ",
    "jq -e '.roots | length == 1' /run/d2b-debug-row.json >/dev/null && ",
    "jq -e --arg name \"Process/$name\" ",
    "'.roots[0].ref == $name and .roots[0].phase == \"Ready\" ",
    "and (.roots[0].children | length == 0)' ",
    "/run/d2b-debug-row.json >/dev/null",
);

/// The human rendering of the same report. No TTY in the lane, so the human
/// tree needs the explicit flag; this is the one place the tree renderer is
/// exercised live.
const DEBUG_HUMAN_REPORT: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --human debug work >/run/d2b-debug-human.txt && ",
    "grep -F 'zone work rows=' /run/d2b-debug-human.txt >/dev/null && ",
    "grep -F 'Host/host-system' /run/d2b-debug-human.txt >/dev/null && ",
    "grep -F 'type Zone not read' /run/d2b-debug-human.txt >/dev/null",
);

/// A row that does not exist is refused.
const DEBUG_ABSENT_ROW: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work debug work Process/absent-row >/dev/null 2>&1",
);

/// A user without the role is refused.
const UNAUTHORIZED_READ: &str = concat!(
    "runuser -u bob -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Process ",
    ">/run/d2b-unauthorized-resource.log 2>&1",
);

/// The controller process survives a `d2bd` restart: one process, the same
/// uid and generation as before, `Ready` and settled.
const PROCESS_ADOPTED_AFTER_RESTART: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Process ",
    ">/run/d2b-process-after.json && ",
    "test \"$(ps -eo pid=,args= | awk '$NF ~ /acceptance-controller$/ {print $1}' ",
    "| wc -l)\" -eq 1 && ",
    "jq -e --slurpfile before /run/d2b-process-before.json ",
    "'([.resources[] | select(.type == \"Process\" and ",
    ".metadata.ownerRef == \"Provider/network-local\")] | length == 1) and ",
    "(.resources[] | select(.type == \"Process\" and ",
    ".metadata.ownerRef == \"Provider/network-local\") as $after | ",
    "($before[0].resources[] | select(.type == \"Process\" and ",
    ".metadata.ownerRef == \"Provider/network-local\")) as $old | ",
    "($after.metadata.uid == $old.metadata.uid and ",
    "$after.metadata.generation == $old.metadata.generation and ",
    "$after.status.phase == \"Ready\" and ",
    "$after.status.observedGeneration == $after.metadata.generation))' ",
    "/run/d2b-process-after.json",
);

/// The restart is observed at least twenty seconds after the adoption, so the
/// resync assertion below is about a process that stayed up rather than about
/// a read that arrived before the daemon had finished.
const PROCESS_RESYNCED_AFTER_RESTART: &str = concat!(
    "test $(( $(date +%s) - $(cat /run/d2b-resource-restart-observed-at) )) -ge 20 && ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Process ",
    ">/run/d2b-process-after-resync.json && ",
    "test \"$(ps -eo pid=,args= | awk '$NF ~ /acceptance-controller$/ {print $1}' ",
    "| wc -l)\" -eq 1 && ",
    "jq -e '([.resources[] | select(.type == \"Process\" and ",
    ".metadata.ownerRef == \"Provider/network-local\")] | length == 1) and ",
    "(.resources[] | select(.type == \"Process\" and ",
    ".metadata.ownerRef == \"Provider/network-local\") | ",
    "(.metadata.uid != null and .metadata.generation > 0 and ",
    ".status.phase == \"Ready\" and ",
    ".status.observedGeneration == .metadata.generation))' ",
    "/run/d2b-process-after-resync.json",
);

/// The host row after the restart: same uid and generation, a revision that
/// did not go backwards, and `Ready`.
const HOST_ROW_AFTER_RESTART: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Host ",
    ">/run/d2b-host-after.json && ",
    "jq -e --slurpfile before /run/d2b-host-before.json ",
    "'.resources[] | select(.type == \"Host\" and ",
    ".metadata.name == \"host-system\") as $after | ",
    "($before[0].resources[] | select(.type == \"Host\" and ",
    ".metadata.name == \"host-system\")) as $old | ",
    "($after.metadata.uid == $old.metadata.uid and ",
    "$after.metadata.generation == $old.metadata.generation and ",
    "$after.metadata.revision >= $old.metadata.revision and ",
    "$after.status.phase == \"Ready\")' /run/d2b-host-after.json",
);

/// The check's assertions, in the order its fixture made them.
pub fn assertions(control: &mut GuestControl) -> LegacyResult<()> {
    control.stage("boot");
    control.wait_for_unit("nftables.service", None, UNIT_BOUND)?;
    control.succeed(&["nft list table inet d2b"], None)?;
    control.wait_for_unit("d2b-broker.socket", None, SOCKET_BOUND)?;
    control.diag_unit("daemon-up", "d2bd.service", UNIT_BOUND)?;
    control.wait_for_file("/run/d2b/public.sock", PUBLIC_SOCKET_BOUND)?;

    // The external provider's controller session comes up behind the daemon,
    // and it is the Process rows that say so.
    let process_rows = live_rows("Process rows", "Process");
    let process_explain = [("d2bd.service", "ResourceV3 session")];
    control.diag_wait(
        "provider-session-live",
        PROVIDER_SESSION_LIVE,
        WAIT,
        &[row(&process_rows)],
        &process_explain,
    )?;
    control.succeed(
        &["runuser -u alice -- d2b auth status --json >/run/d2b-auth-before.json"],
        None,
    )?;

    // The declared rows settle, one at a time, each with the journal line
    // that explains it.
    let host_rows = saved_rows("Host rows", "/run/d2b-host-before.json");
    control.diag_wait(
        "host-row",
        HOST_ROW,
        WAIT,
        &[saved_row(&host_rows)],
        &[("d2bd.service", "host-system")],
    )?;
    control.succeed(&[USER_ROW], None)?;
    control.succeed(&[PROVIDER_ROW], None)?;
    let controller_rows = saved_rows("Process rows", "/run/d2b-process-before.json");
    control.diag_wait(
        "network-controller-process",
        CONTROLLER_PROCESS,
        WAIT,
        &[
            saved_row(&controller_rows),
            live_row(&process_rows, "Controller Process rows"),
        ],
        &[("d2bd.service", "network-local")],
    )?;
    let controller_processes = ("controller processes", CONTROLLER_PROCESSES);
    control.diag_wait(
        "controller-pid",
        ONE_CONTROLLER_PROCESS,
        PID_BOUND,
        &[controller_processes],
        &[("d2bd.service", "acceptance-controller")],
    )?;
    let controller_pid_before = control.succeed(&[CONTROLLER_PID], None)?.trim().to_owned();

    // The debug surface, as the two readers it has: the JSON report and the
    // human tree.
    let debug_explain = [("d2bd.service", "acceptance-controller")];
    control.diag_run(
        "debug-surface-zone-report",
        DEBUG_ZONE_REPORT,
        &[live_row(&process_rows, "Process rows")],
        &debug_explain,
    )?;
    control.diag_run(
        "debug-surface-named-row",
        DEBUG_NAMED_ROW,
        &[live_row(&process_rows, "Process rows")],
        &debug_explain,
    )?;
    control.diag_run(
        "debug-surface-human-report",
        DEBUG_HUMAN_REPORT,
        &[live_row(&process_rows, "Process rows")],
        &debug_explain,
    )?;

    // A row nobody declared, and a user without the role: both refused.
    control.fail(&[DEBUG_ABSENT_ROW], None)?;
    control.fail(&[UNAUTHORIZED_READ], None)?;

    // The restart boundary: the daemon comes back, and the controller process
    // is adopted rather than restarted.
    control.stage("restart");
    control.succeed(&["systemctl restart d2bd.service"], None)?;
    control.diag_unit("daemon-restarted", "d2bd.service", UNIT_BOUND)?;
    control.wait_for_file("/run/d2b/public.sock", PUBLIC_SOCKET_BOUND)?;
    let after_rows = saved_rows("Process rows", "/run/d2b-process-after.json");
    let before_rows = saved_rows("Pre-restart Process rows", "/run/d2b-process-before.json");
    control.diag_wait(
        "process-adopted-after-restart",
        PROCESS_ADOPTED_AFTER_RESTART,
        WAIT,
        &[saved_row(&after_rows), saved_row(&before_rows)],
        &[("d2bd.service", "network-local")],
    )?;
    let controller_pid_after = control.succeed(&[CONTROLLER_PID], None)?.trim().to_owned();
    if controller_pid_after != controller_pid_before {
        return Err(LegacyError::Assertion(format!(
            "controller PID changed across d2bd restart: \
             {controller_pid_before} -> {controller_pid_after}"
        )));
    }
    control.succeed(&["date +%s >/run/d2b-resource-restart-observed-at"], None)?;
    let resynced_rows = saved_rows("Process rows", "/run/d2b-process-after-resync.json");
    control.diag_wait(
        "process-resynced-after-restart",
        PROCESS_RESYNCED_AFTER_RESTART,
        WAIT,
        &[saved_row(&resynced_rows), controller_processes],
        &[("d2bd.service", "network-local")],
    )?;
    control.succeed(
        &["runuser -u alice -- d2b auth status --json >/run/d2b-auth-after.json"],
        None,
    )?;
    control.succeed(&[HOST_ROW_AFTER_RESTART], None)?;

    // The census: compare the live system only with the framework-owned
    // acceptance declaration, so optional or managed infrastructure is not
    // read as a framework violation while a declared unit that is absent
    // still fails.
    let declared = units(&control.succeed(&["cat /etc/d2b/daemon-acceptance-units"], None)?);
    let required = REQUIRED_UNITS
        .iter()
        .map(|unit| (*unit).to_owned())
        .collect::<BTreeSet<String>>();
    if declared != required {
        return Err(LegacyError::Assertion(format!(
            "unexpected framework acceptance census: {}",
            as_a_set(&declared)
        )));
    }
    let live = units(&control.succeed(
        &["systemctl list-units --no-pager --all --plain | awk '{print $1}' | sort"],
        None,
    )?);
    let missing = required
        .difference(&live)
        .cloned()
        .collect::<BTreeSet<String>>();
    if !missing.is_empty() {
        return Err(LegacyError::Assertion(format!(
            "framework daemon units missing: {}",
            as_a_set(&missing)
        )));
    }

    // Provider packages are code loaded by d2bd, never framework-declared
    // persistent services.
    let provider_units = units(
        &declared
            .iter()
            .filter(|unit| unit.contains("provider"))
            .filter(|unit| unit.ends_with(".service") || unit.ends_with(".socket"))
            .cloned()
            .collect::<Vec<_>>()
            .join(" "),
    );
    if !provider_units.is_empty() {
        return Err(LegacyError::Assertion(format!(
            "Provider-owned persistent units found: {}",
            as_a_list(&provider_units)
        )));
    }

    Ok(())
}

/// The live rows of one resource type, as the fixture's own `live_rows` built
/// them: a labelled jq projection of what the public surface answers.
fn live_rows(label: &str, resource_type: &str) -> (String, String) {
    (
        label.to_owned(),
        format!(
            "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock \
             d2b --zone work --json list {resource_type} 2>/dev/null | \
             jq -c '{DIAG_PROJECTION}' 2>/dev/null || true"
        ),
    )
}

/// The rows of a file the check just saved, as the fixture's own `saved_rows`
/// built them: the same projection, falling back to the file itself.
fn saved_rows(label: &str, path: &str) -> (String, String) {
    (
        label.to_owned(),
        format!(
            "jq -c '{DIAG_PROJECTION}' {path} 2>/dev/null || cat {path} 2>/dev/null || true"
        ),
    )
}

/// One row of a labelled pair, as the diagnostics rows are passed.
fn row<'a>(pair: &'a (String, String)) -> DiagRow<'a> {
    (pair.0.as_str(), pair.1.as_str())
}

/// One `live_rows` pair, relabelled the way the fixture relabelled it.
fn live_row<'a>(pair: &'a (String, String), label: &'a str) -> DiagRow<'a> {
    (label, pair.1.as_str())
}

/// One `saved_rows` pair.
fn saved_row<'a>(pair: &'a (String, String)) -> DiagRow<'a> {
    row(pair)
}

/// The unit names in what a command printed, as a set.
fn units(output: &str) -> BTreeSet<String> {
    output.split_whitespace().map(str::to_owned).collect()
}

/// A set of unit names, rendered the way the fixture's own `assert` messages
/// rendered one.
fn as_a_set(units: &BTreeSet<String>) -> String {
    let quoted = units
        .iter()
        .map(|unit| format!("'{unit}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{{quoted}}}")
}

/// A sorted list of unit names, rendered the way the fixture's own `assert`
/// message rendered one.
fn as_a_list(units: &BTreeSet<String>) -> String {
    let quoted = units
        .iter()
        .map(|unit| format!("'{unit}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{quoted}]")
}
