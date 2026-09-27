//! The guest network-agent capability confinement check, ported from its
//! fixture.
//!
//! It gives a live process the three capabilities the network agent is
//! declared to need, inside a dedicated Linux network namespace, and asserts
//! the effective set it ends up with - then proves that starting it added no
//! such capability to any process sharing the host network namespace. The
//! assertions are the fixture's, in the fixture's order and with the
//! fixture's own command text and bounds, including the three helper reads
//! the fixture's script defined and this module carries as functions: the
//! namespace of a process, the inode of a namespace, a process's effective
//! capability set, the host namespace's per-process capability table, and a
//! service's process identities.
//!
//! The fixture declared a plain NixOS node - it never wanted the d2b daemon
//! host - so the guest it boots, with the namespace unit, the agent unit and
//! the unprivileged user both run as, is declared in
//! `nix/test-support/host-integration-node.nix` beside the reusable nodes.
//!
//! `start_all()` is not restated here: it is the lane's own boot of the
//! guest the check runs against.

use std::{collections::{BTreeMap, BTreeSet}, time::Duration};

use crate::legacy::{GuestControl, LegacyError, LegacyResult};

/// The effective network capabilities the agent unit declares:
/// `CAP_NET_ADMIN`, `CAP_NET_BIND_SERVICE` and `CAP_NET_RAW`.
const CAPABILITY_MASK: u64 = (1 << 10) | (1 << 12) | (1 << 13);

/// The bound the guest's own `multi-user.target` gets, the fixture's own.
const BOOT: Duration = Duration::from_secs(180);

/// The bound the agent unit gets once it has been started, the fixture's
/// own; the unit is already started, so this is the wait for systemd to have
/// settled it.
const AGENT_UP: Duration = Duration::from_secs(60);

/// The fixture's own table of processes in the host network namespace, down
/// to its words: the host namespace's inode, every process sharing it, each
/// one's `CapEff`, and the start time that keeps a recycled pid from being
/// mistaken for the process it replaced.
///
/// A raw string, because the `\n` in the `printf` is the shell's escape for
/// the newline that separates the rows - the same two characters the
/// fixture's Python sent.
const HOST_NAMESPACE_CAPABILITIES: &str = concat!(
    "host_ns=$(readlink /proc/1/ns/net); ",
    "for status in /proc/[0-9]*/status; do ",
    "pid='${status#/proc/}'; pid='${pid%/status}'; ",
    "ns=$(readlink /proc/$pid/ns/net 2>/dev/null) || continue; ",
    "test \"$ns\" = \"$host_ns\" || continue; ",
    "cap=$(while IFS=: read -r key value; do ",
    "test \"$key\" = CapEff && { printf '%s' \"$value\"; break; }; done < \"$status\"); ",
    "start=$(cut -d' ' -f22 /proc/$pid/stat 2>/dev/null) || continue; ",
    r#"printf '%s %s %s\n' "$pid" "$start" "$cap"; "#,
    "done",
);

/// The check's assertions, in the order its fixture made them.
pub fn assertions(control: &mut GuestControl) -> LegacyResult<()> {
    control.stage("boot");
    control.wait_for_unit("multi-user.target", None, BOOT)?;

    // The namespace the agent's own unit is confined to, up before the agent
    // that requires it.
    control.stage("agent-netns");
    control.succeed(&["systemctl start d2b-test-agent-netns.service"], None)?;

    // The baseline is the host namespace's capability table before the agent
    // exists, so the last assertion can be about what changed rather than
    // about what is there.
    control.stage("baseline-caps");
    let host_namespace = network_namespace(control, "1")?;
    let baseline = host_namespace_capabilities(control)?;

    control.stage("agent-up");
    control.succeed(&["systemctl start d2b-test-guest-agent.service"], None)?;
    control.diag_unit("guest-agent-up", "d2b-test-guest-agent.service", AGENT_UP)?;
    let agent_pid = control
        .succeed(
            &["systemctl show -P MainPID d2b-test-guest-agent.service"],
            None,
        )?
        .trim()
        .to_owned();
    if agent_pid.is_empty() || agent_pid == "0" {
        return Err(LegacyError::Assertion(
            "network agent did not start".to_owned(),
        ));
    }

    let agent_namespace = network_namespace(control, &agent_pid)?;
    let agent_namespace_inode = network_namespace_inode(control, &format!("/proc/{agent_pid}/ns/net"))?;
    let declared_namespace_inode = network_namespace_inode(control, "/run/netns/d2b-test-agent")?;
    if agent_namespace_inode != declared_namespace_inode {
        return Err(LegacyError::Assertion(
            "network agent did not inherit the declared Guest network namespace".to_owned(),
        ));
    }
    if agent_namespace == host_namespace {
        return Err(LegacyError::Assertion(
            "network agent unexpectedly shares the host network namespace".to_owned(),
        ));
    }

    // The declared set is exactly what the process holds: every declared
    // capability present, and nothing outside the declaration.
    control.stage("confinement-assertions");
    let agent_capabilities = effective_capabilities(control, &agent_pid)?;
    if agent_capabilities & CAPABILITY_MASK != CAPABILITY_MASK {
        return Err(LegacyError::Assertion(
            "network agent is missing a required effective network capability".to_owned(),
        ));
    }
    if agent_capabilities & !CAPABILITY_MASK != 0 {
        return Err(LegacyError::Assertion(
            "network agent received an undeclared effective capability".to_owned(),
        ));
    }

    // The agent's main process is inside the control group of the unit that
    // declared the capabilities, rather than a process systemd started and
    // then lost track of.
    let service_identities = service_processes(control, "d2b-test-guest-agent.service")?;
    let agent_start = control
        .succeed(&[&format!("cut -d' ' -f22 /proc/{agent_pid}/stat")], None)?
        .trim()
        .to_owned();
    if !service_identities.contains(&(agent_pid.clone(), agent_start)) {
        return Err(LegacyError::Assertion(
            "network agent main process is outside its service control group".to_owned(),
        ));
    }

    // Starting the agent added nothing to a process that already shared the
    // host network namespace ...
    let after = host_namespace_capabilities(control)?;
    let gained = baseline
        .iter()
        .filter_map(|((pid, start), before)| {
            let current = after.get(&(pid.clone(), start.clone()))?;
            let gained = current & CAPABILITY_MASK & !before;
            (gained != 0).then(|| {
                (
                    pid.clone(),
                    start.clone(),
                    before & CAPABILITY_MASK,
                    current & CAPABILITY_MASK,
                )
            })
        })
        .collect::<Vec<_>>();
    if !gained.is_empty() {
        return Err(LegacyError::Assertion(format!(
            "starting the network agent added effective network capabilities to \
             an existing host-network-namespace process: {}",
            render_gained(&gained)
        )));
    }

    // ... and the agent's own service left no capability-bearing process
    // sharing that namespace.
    let service_leaks = service_identities
        .iter()
        .filter_map(|(pid, start)| {
            let capabilities = after.get(&(pid.clone(), start.clone()))? & CAPABILITY_MASK;
            (capabilities != 0).then(|| (pid.clone(), start.clone(), capabilities))
        })
        .collect::<Vec<_>>();
    if !service_leaks.is_empty() {
        return Err(LegacyError::Assertion(format!(
            "network agent service left a capability-bearing process in the host \
             network namespace: {}",
            render_service_leaks(&service_leaks)
        )));
    }

    Ok(())
}

/// The network namespace one process is in, as `readlink` reports it.
fn network_namespace(control: &mut GuestControl, pid: &str) -> LegacyResult<String> {
    Ok(control
        .succeed(&[&format!("readlink /proc/{pid}/ns/net")], None)?
        .trim()
        .to_owned())
}

/// The device and inode of a namespace, as `stat` reports them.
fn network_namespace_inode(control: &mut GuestControl, path: &str) -> LegacyResult<String> {
    Ok(control
        .succeed(&[&format!("stat -Lc '%d:%i' {path}")], None)?
        .trim()
        .to_owned())
}

/// The effective capability set of one process, read out of its status.
fn effective_capabilities(control: &mut GuestControl, pid: &str) -> LegacyResult<u64> {
    let status = control.succeed(&[&format!("cat /proc/{pid}/status")], None)?;
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("CapEff:") {
            return parse_capabilities(value.trim()).ok_or_else(|| {
                LegacyError::Assertion(format!(
                    "process {pid} has an unreadable CapEff status field: {value:?}"
                ))
            });
        }
    }
    Err(LegacyError::Assertion(format!(
        "process {pid} has no CapEff status field"
    )))
}

/// The effective capability set of every process in the host network
/// namespace, keyed by `(pid, start time)`.
fn host_namespace_capabilities(
    control: &mut GuestControl,
) -> LegacyResult<BTreeMap<(String, String), u64>> {
    let rows = control.succeed(&[HOST_NAMESPACE_CAPABILITIES], None)?;
    let mut capabilities = BTreeMap::new();
    for row in rows.lines() {
        let mut fields = row.split_whitespace();
        let (pid, start, cap) = match (fields.next(), fields.next(), fields.next(), fields.next()) {
            (Some(pid), Some(start), Some(cap), None) => (pid, start, cap),
            _ => {
                return Err(LegacyError::Assertion(format!(
                    "host network namespace capability row is not '<pid> <start> <cap>': {row:?}"
                )));
            }
        };
        let cap = parse_capabilities(cap).ok_or_else(|| {
            LegacyError::Assertion(format!(
                "host network namespace process {pid} has an unreadable capability set: {cap:?}"
            ))
        })?;
        capabilities.insert((pid.to_owned(), start.to_owned()), cap);
    }
    Ok(capabilities)
}

/// The `(pid, start time)` identities of every process in one unit's control
/// group.
fn service_processes(
    control: &mut GuestControl,
    unit: &str,
) -> LegacyResult<BTreeSet<(String, String)>> {
    let control_group = control
        .succeed(&[&format!("systemctl show -P ControlGroup {unit}")], None)?
        .trim()
        .to_owned();
    if !control_group.starts_with('/') {
        return Err(LegacyError::Assertion(format!(
            "{unit} has invalid control group '{control_group}'"
        )));
    }
    let rows = control.succeed(
        &[&format!(
            "find /sys/fs/cgroup{control_group} -name cgroup.procs -type f -exec cat {{}} +"
        )],
        None,
    )?;
    let mut identities = BTreeSet::new();
    for pid in rows.lines() {
        let start = control
            .succeed(&[&format!("cut -d' ' -f22 /proc/{pid}/stat")], None)?
            .trim()
            .to_owned();
        identities.insert((pid.to_owned(), start));
    }
    Ok(identities)
}

/// A capability set as these processes report it: hexadecimal, with the
/// `0x` prefix the kernel does not print but a reader may.
fn parse_capabilities(value: &str) -> Option<u64> {
    let digits = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    u64::from_str_radix(digits, 16).ok()
}

/// The processes a gain was observed on, rendered the way the fixture's own
/// failure rendered the list it built.
fn render_gained(gained: &[(String, String, u64, u64)]) -> String {
    let rows = gained
        .iter()
        .map(|(pid, start, before, after)| format!("('{pid}', '{start}', {before}, {after})"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{rows}]")
}

/// The capability-bearing processes one service left behind, rendered the
/// way the fixture's own failure rendered the list it built.
fn render_service_leaks(leaks: &[(String, String, u64)]) -> String {
    let rows = leaks
        .iter()
        .map(|(pid, start, capabilities)| format!("('{pid}', '{start}', {capabilities})"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{rows}]")
}
