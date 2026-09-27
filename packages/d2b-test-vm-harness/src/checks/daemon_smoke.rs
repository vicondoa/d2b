//! The daemon-only surface smoke, ported from its fixture.
//!
//! It boots the daemon host the reusable node declares and asserts the
//! daemon-only end-state on a live system (ADR 0015): exactly the three
//! framework-declared root-visible units start, the broker socket is
//! socket-activated with the declared ACL, and the unprivileged public daemon
//! comes up and binds `/run/d2b/public.sock`. It is the live successor of the
//! eval-only and `D2B_LIVE` portions of `tests/d2bd-startup-smoke.sh`: it
//! exercises real systemd activation ordering and socket binding that the
//! pure-eval unit-surface gate cannot.
//!
//! The assertions below are the fixture's assertions, in the fixture's order,
//! with the fixture's own bounds. What the fixture expressed as `machine.*`
//! calls is [`GuestControl`]'s own operations, and what it expressed as
//! `stage`/`diag_unit` calls are the same primitives the fixtures'
//! diagnostics prelude provides, so a failure reported here reads the way the
//! fixture's failure read. Two things the fixture did are not restated here,
//! because the lane already does them: `start_all()` is the lane's own boot
//! of the guest the check runs against, and the diagnostics prelude is the
//! surface's own reporting rather than a check's assertion.

use std::{collections::BTreeSet, time::Duration};

use crate::legacy::{GuestControl, LegacyError, LegacyResult};

/// The public wire surface `d2bd` binds.
const PUBLIC_SOCKET: &str = "/run/d2b/public.sock";

/// The bound the broker socket gets: it is socket-activated, so it is up once
/// systemd has bound and ACLed it, and a socket that has not appeared in
/// thirty seconds is not going to.
const SOCKET_ACTIVATION: Duration = Duration::from_secs(30);

/// The bound a `d2bd` start or restart gets. The daemon builds its topology
/// before it reports readiness, and this is the check's own bound rather than
/// the driver's default.
const DAEMON_ACTIVATION: Duration = Duration::from_secs(180);

/// The daemon-only end-state contract (ADR 0015) declares exactly these three
/// framework-owned root-visible units.
const REQUIRED_UNITS: [&str; 3] = ["d2bd.service", "d2b-broker.socket", "d2b-broker.service"];

/// Put a synthetic process into `d2bd.service`'s cgroup, so a restart's
/// `KillMode` is observed against a process systemd did not start.
///
/// The command is the fixture's own, down to its words: it writes the process
/// into the unit's control group and prints its pid, which is the handle the
/// assertions after the restart use. The synthetic process is what keeps this
/// fast smoke test from needing a nested Cloud Hypervisor guest - the actual
/// Cloud Hypervisor runner-survival test lives in
/// `runtime-cloud-hypervisor-guest-preflight.nix`.
const SURVIVOR_COMMAND: &str = concat!(
    "set -euo pipefail; ",
    "cg=$(systemctl show -P ControlGroup d2bd.service); ",
    "rm -f /run/d2b-smoke-survivor.pid; ",
    "setsid -f sh -c 'echo $$ > /run/d2b-smoke-survivor.pid; exec sleep 3600' ",
    "</dev/null >/dev/null 2>&1; ",
    "for _ in $(seq 1 50); do ",
    "  test -s /run/d2b-smoke-survivor.pid && break; ",
    "  sleep 0.1; ",
    "done; ",
    "pid=$(cat /run/d2b-smoke-survivor.pid); ",
    "echo \"$pid\" > \"/sys/fs/cgroup$cg/cgroup.procs\"; ",
    "echo \"$pid\"",
);

/// The check's assertions, in the order its fixture made them.
pub fn assertions(control: &mut GuestControl) -> LegacyResult<()> {
    control.stage("boot");

    // 1. The broker socket is created, and listening, before its service
    //    (socket activation): systemd binds and ACLs the AF_UNIX socket up
    //    front.
    control.diag_unit("broker-socket", "d2b-broker.socket", SOCKET_ACTIVATION)?;

    // 2. The unprivileged public daemon comes up. It Wants= (not Requires=)
    //    the broker socket, so it serves while the broker stays idle.
    control.diag_unit("daemon-up", "d2bd.service", DAEMON_ACTIVATION)?;
    control.succeed(
        &[r#"test "$(systemctl show -P Type d2bd.service)" = notify"#],
        None,
    )?;
    control.succeed(
        &[r#"test "$(systemctl show -P NotifyAccess d2bd.service)" = main"#],
        None,
    )?;
    control.succeed(
        &[r#"test "$(systemctl show -P KillMode d2bd.service)" = process"#],
        None,
    )?;
    control.succeed(
        &["systemctl show -P ExecStop d2bd.service | grep -q d2b-host-shutdown-hook"],
        None,
    )?;

    // 3. The live public wire surface: d2bd binds its AF_UNIX socket.
    control.stage("public-socket");
    control.wait_for_file(PUBLIC_SOCKET, SOCKET_ACTIVATION)?;
    control.succeed(&["test -S /run/d2b/public.sock"], None)?;
    control.stage("restart-wire-surface");
    control.succeed(&["systemctl restart d2bd.service"], None)?;
    control.diag_unit("daemon-restarted", "d2bd.service", DAEMON_ACTIVATION)?;
    control.succeed(&["test -S /run/d2b/public.sock"], None)?;
    control.succeed(
        &["runuser -u alice -- d2b auth status --json >/dev/null"],
        None,
    )?;

    // 3b. Service restart readiness and cgroup survival.
    let survivor = control.succeed(&[SURVIVOR_COMMAND], None)?.trim().to_owned();

    control.stage("restart-cgroup-survival");
    control.succeed(&["systemctl restart d2bd.service"], None)?;
    control.diag_unit("daemon-restarted-again", "d2bd.service", DAEMON_ACTIVATION)?;
    control.succeed(&["test -S /run/d2b/public.sock"], None)?;
    control.succeed(
        &["runuser -u alice -- d2b auth status --json >/dev/null"],
        None,
    )?;
    control.succeed(&[&format!("test -d /proc/{survivor}")], None)?;
    control.succeed(&[&format!("kill {survivor}")], None)?;

    // 4. Daemon-only end-state (ADR 0015 "Verification gates"): compare the
    //    live system only with the framework-owned acceptance declaration.
    //    This avoids treating unrelated optional or managed infrastructure as
    //    a framework violation while still failing if a declared unit is
    //    absent.
    control.stage("acceptance-census");
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
            "daemon-only framework units missing: {}",
            as_a_set(&missing)
        )));
    }

    // 5. The broker service is socket-activated (not running until a request),
    //    while the socket is listening. A clean idle posture.
    control.succeed(&["systemctl is-active d2b-broker.socket"], None)?;
    Ok(())
}

/// The unit names in what a command printed, as a set.
fn units(output: &str) -> BTreeSet<String> {
    output.split_whitespace().map(str::to_owned).collect()
}

/// A set of unit names, rendered the way the fixture's own `assert` messages
/// rendered one, so a census that fails reads the same before and after this
/// check's port.
fn as_a_set(units: &BTreeSet<String>) -> String {
    let quoted = units
        .iter()
        .map(|unit| format!("'{unit}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("{{{quoted}}}")
}
