//! The live broker privilege posture oracle, ported from its fixture.
//!
//! It boots a d2b daemon host, starts the socket-activated privileged broker,
//! derives the expected posture from the rendered systemd unit, and checks the
//! live `/proc/<pid>` state for the hardening invariants that matter at
//! runtime. It is the hermetic successor to the retired self-hosted L1c shell
//! oracle. The assertions are the fixture's, in the fixture's order and with
//! the fixture's own command text and bounds: the unit's `User`, `Group`,
//! `CapabilityBoundingSet`, `AmbientCapabilities`, `NoNewPrivileges` and
//! `Slice` are read back off systemd and compared with the process's own uid,
//! gid, capability sets, `NoNewPrivs`, seccomp mode and cgroup path.
//!
//! The fixture's two parsing helpers come with it: `parse_cap_set`, which
//! accepts either a hexadecimal mask or a list of capability names and treats
//! an empty bounding set as the full kernel mask, and `parse_unit_bool`. Both
//! refuse a value they do not understand rather than guessing, which is what
//! makes the oracle about the declaration rather than about the observation.
//!
//! The guest is the reusable daemon node, which the fixture already booted
//! from `nix/test-support/host-integration-node.nix`, so nothing about it
//! moved here.
//!
//! `start_all()` is not restated here: it is the lane's own boot of the
//! guest the check runs against.

use std::{collections::BTreeMap, time::Duration};

use crate::legacy::{shlex_quote, GuestControl, LegacyError, LegacyResult};

/// The bound the broker socket gets, the fixture's own: it is
/// socket-activated, so it is up once systemd has bound and ACLed it.
const BROKER_SOCKET: Duration = Duration::from_secs(30);

/// The bound the daemon gets before it reports readiness, the fixture's own.
const DAEMON_UP: Duration = Duration::from_secs(180);

/// The capability names the kernel numbers, in the order the fixture listed
/// them. The index of a name in this list is the bit the kernel assigns it.
const CAPABILITY_NAMES: [&str; 41] = [
    "CHOWN",
    "DAC_OVERRIDE",
    "DAC_READ_SEARCH",
    "FOWNER",
    "FSETID",
    "KILL",
    "SETGID",
    "SETUID",
    "SETPCAP",
    "LINUX_IMMUTABLE",
    "NET_BIND_SERVICE",
    "NET_BROADCAST",
    "NET_ADMIN",
    "NET_RAW",
    "IPC_LOCK",
    "IPC_OWNER",
    "SYS_MODULE",
    "SYS_RAWIO",
    "SYS_CHROOT",
    "SYS_PTRACE",
    "SYS_PACCT",
    "SYS_ADMIN",
    "SYS_BOOT",
    "SYS_NICE",
    "SYS_RESOURCE",
    "SYS_TIME",
    "SYS_TTY_CONFIG",
    "MKNOD",
    "LEASE",
    "AUDIT_WRITE",
    "AUDIT_CONTROL",
    "SETFCAP",
    "MAC_OVERRIDE",
    "MAC_ADMIN",
    "SYSLOG",
    "WAKE_ALARM",
    "BLOCK_SUSPEND",
    "AUDIT_READ",
    "PERFMON",
    "BPF",
    "CHECKPOINT_RESTORE",
];

/// The broker's main pid, waited for the way the fixture waited for it: the
/// unit publishes one, and a process it names is readable.
const WAIT_FOR_MAIN_PID: &str = concat!(
    "for i in $(seq 1 100); do ",
    "pid=$(systemctl show -p MainPID --value d2b-broker.service); ",
    "if [ -n \"$pid\" ] && [ \"$pid\" != 0 ] && [ -r \"/proc/$pid/status\" ]; then ",
    "echo \"$pid\"; exit 0; fi; ",
    "sleep 0.2; ",
    "done; ",
    "echo 'd2b-broker.service did not publish a MainPID within 20s:'; ",
    "systemctl status --no-pager d2b-broker.service; ",
    "exit 1",
);

/// The rendered posture, in the seven properties the oracle compares against
/// the live process.
const RENDERED_POSTURE: &str = concat!(
    "systemctl show d2b-broker.service ",
    "-p CapabilityBoundingSet ",
    "-p AmbientCapabilities ",
    "-p NoNewPrivileges ",
    "-p User ",
    "-p Group ",
    "-p Slice ",
    "-p SystemCallFilter",
);

/// The live process's own namespace set, as the fixture read it: the kinds it
/// asked for, by name, with the link each one points at.
fn namespace_report(pid: &str) -> String {
    format!(
        concat!(
            "for ns in cgroup ipc mnt net pid time time_for_children user uts; do ",
            "[ -e /proc/{pid}/ns/$ns ] && printf '%s=%s\\n' \"$ns\" \"$(readlink /proc/{pid}/ns/$ns)\"; ",
            "done",
        ),
        pid = pid,
    )
}

/// The check's assertions, in the order its fixture made them.
pub fn assertions(control: &mut GuestControl) -> LegacyResult<()> {
    control.stage("boot");
    control.diag_unit("broker-socket", "d2b-broker.socket", BROKER_SOCKET)?;
    control.diag_unit("daemon-up", "d2bd.service", DAEMON_UP)?;

    // The broker is socket-activated, but starting the service directly keeps
    // a live Type=notify process long enough to read its /proc posture.
    control.stage("broker-start");
    control.succeed(&["systemctl start d2b-broker.service"], None)?;
    let broker_pid = control.succeed(&[WAIT_FOR_MAIN_PID], None)?.trim().to_owned();
    control.announce(&format!("live d2b-broker PID: {broker_pid}"));

    control.stage("broker-posture");
    let unit_raw = control.succeed(&[RENDERED_POSTURE], None)?;
    control.announce(&format!("rendered d2b-broker.service posture:\n{unit_raw}"));

    let status_raw = control.succeed(&[&format!("cat /proc/{broker_pid}/status")], None)?;
    let cgroup_raw = control.succeed(&[&format!("cat /proc/{broker_pid}/cgroup")], None)?;
    let ns_raw = control.succeed(&[&namespace_report(&broker_pid)], None)?;
    control.announce(&format!(
        "live /proc status subset:\n{}",
        status_raw
            .lines()
            .filter(|line| {
                [
                    "Uid:",
                    "Gid:",
                    "Groups:",
                    "CapEff:",
                    "CapBnd:",
                    "CapAmb:",
                    "NoNewPrivs:",
                    "Seccomp:",
                ]
                .iter()
                .any(|prefix| line.starts_with(prefix))
            })
            .collect::<Vec<_>>()
            .join("\n")
    ));
    control.announce(&format!("live cgroup:\n{cgroup_raw}"));
    control.announce(&format!("live namespaces:\n{ns_raw}"));

    let unit = equals_properties(&unit_raw);
    let status = colon_properties(&status_raw);
    let cap_last_cap: u32 = control
        .succeed(&["cat /proc/sys/kernel/cap_last_cap"], None)?
        .trim()
        .parse()
        .map_err(|error| {
            LegacyError::Assertion(format!(
                "the kernel's cap_last_cap is not a number: {error}"
            ))
        })?;
    let full_cap_mask = full_capability_mask(cap_last_cap);

    control.stage("posture-oracle");
    let user = required(&unit, "User")?;
    let group = required(&unit, "Group")?;
    let expected_uid: u64 = control
        .succeed(&[&format!("id -u {}", shlex_quote(user))], None)?
        .trim()
        .parse()
        .map_err(|error| LegacyError::Assertion(format!("the broker's uid is not a number: {error}")))?;
    let expected_gid: u64 = control
        .succeed(
            &[&format!("getent group {} | cut -d: -f3", shlex_quote(group))],
            None,
        )?
        .trim()
        .parse()
        .map_err(|error| LegacyError::Assertion(format!("the broker's gid is not a number: {error}")))?;
    let expected_cap_bnd = parse_cap_set(
        required(&unit, "CapabilityBoundingSet")?,
        cap_last_cap,
        full_cap_mask,
        true,
    )?;
    let expected_cap_amb = parse_cap_set(
        required(&unit, "AmbientCapabilities")?,
        cap_last_cap,
        full_cap_mask,
        false,
    )?;
    let expected_nonewprivs = parse_unit_bool(required(&unit, "NoNewPrivileges")?)?;
    let expected_slice = required(&unit, "Slice")?.trim().to_owned();

    let actual_uids = integers(required(&status, "Uid")?)?;
    let actual_gids = integers(required(&status, "Gid")?)?;
    let actual_cap_eff = hex(required(&status, "CapEff")?)?;
    let actual_cap_bnd = hex(required(&status, "CapBnd")?)?;
    let actual_cap_amb = hex(required(&status, "CapAmb")?)?;
    let actual_nonewprivs: u32 = required(&status, "NoNewPrivs")?
        .trim()
        .parse()
        .map_err(|error| LegacyError::Assertion(format!("NoNewPrivs is not a number: {error}")))?;
    let actual_seccomp: u32 = required(&status, "Seccomp")?
        .trim()
        .parse()
        .map_err(|error| LegacyError::Assertion(format!("Seccomp is not a number: {error}")))?;
    let cgroup_paths = cgroup_raw
        .lines()
        .filter(|line| line.contains(':'))
        .map(|line| line.splitn(3, ':').nth(2).unwrap_or_default().to_owned())
        .collect::<Vec<_>>();

    // The process's identity is the unit's declaration of it.
    if !actual_uids.iter().all(|uid| *uid == expected_uid) {
        return Err(LegacyError::Assertion(format!(
            "broker Uid must match rendered User={user} ({expected_uid}), got {}",
            render_integers(&actual_uids)
        )));
    }
    if expected_uid != 0 {
        return Err(LegacyError::Assertion(format!(
            "broker must run as root uid 0, rendered User={user}"
        )));
    }
    if !actual_gids.iter().all(|gid| *gid == expected_gid) {
        return Err(LegacyError::Assertion(format!(
            "broker Gid must match rendered Group={group} ({expected_gid}), got {}",
            render_integers(&actual_gids)
        )));
    }

    // The capability sets: bounded exactly as declared, never the full kernel
    // set, and effective bits inside the bounding set.
    if actual_cap_bnd != expected_cap_bnd {
        return Err(LegacyError::Assertion(format!(
            "CapBnd must match rendered CapabilityBoundingSet: expected 0x{expected_cap_bnd:x}, got 0x{actual_cap_bnd:x}"
        )));
    }
    if actual_cap_bnd == full_cap_mask {
        return Err(LegacyError::Assertion(format!(
            "CapBnd is the full kernel capability mask 0x{full_cap_mask:x}, not the bounded broker set"
        )));
    }
    if actual_cap_eff & !actual_cap_bnd != 0 {
        return Err(LegacyError::Assertion(format!(
            "CapEff 0x{actual_cap_eff:x} contains bits outside CapBnd 0x{actual_cap_bnd:x}"
        )));
    }
    if actual_cap_amb != expected_cap_amb {
        return Err(LegacyError::Assertion(format!(
            "CapAmb must match rendered AmbientCapabilities: expected 0x{expected_cap_amb:x}, got 0x{actual_cap_amb:x}"
        )));
    }
    if actual_cap_amb != 0 {
        return Err(LegacyError::Assertion(format!(
            "broker must not carry ambient capabilities, got 0x{actual_cap_amb:x}"
        )));
    }

    // The hardening flags: no-new-privileges as declared, and a seccomp
    // filter actually installed rather than merely declared.
    let rendered_nonewprivs = required(&unit, "NoNewPrivileges")?;
    if actual_nonewprivs != expected_nonewprivs {
        return Err(LegacyError::Assertion(format!(
            "NoNewPrivs must match rendered NoNewPrivileges={rendered_nonewprivs}, got {actual_nonewprivs}"
        )));
    }
    if actual_seccomp != 2 {
        return Err(LegacyError::Assertion(format!(
            "broker must run in seccomp filter mode (2), got {actual_seccomp}"
        )));
    }

    // The process lives in the slice its unit declared, and in the d2b slice
    // the discipline is about.
    if !cgroup_paths.iter().any(|path| path.contains(&expected_slice)) {
        return Err(LegacyError::Assertion(format!(
            "broker cgroup path must contain rendered Slice={expected_slice}, got {}",
            render_strings(&cgroup_paths)
        )));
    }
    if !cgroup_paths.iter().any(|path| path.contains("d2b.slice")) {
        return Err(LegacyError::Assertion(format!(
            "broker cgroup path must contain d2b.slice, got {}",
            render_strings(&cgroup_paths)
        )));
    }

    Ok(())
}

/// The `Key=Value` lines of one command's output, as a map - the fixture's
/// own reader for the rendered unit.
fn equals_properties(output: &str) -> BTreeMap<String, String> {
    output
        .lines()
        .filter(|line| line.contains('='))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

/// The `Key: Value` lines of one command's output, as a map - the fixture's
/// own reader for `/proc/<pid>/status`. Each value is stripped; a line
/// without the separator is not a property and is skipped.
fn colon_properties(output: &str) -> BTreeMap<String, String> {
    output
        .lines()
        .filter(|line| line.contains(':'))
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.to_owned(), value.trim().to_owned()))
        .collect()
}

/// One property, or the failure a missing one is: the fixture's own `KeyError`
/// became a check failure rather than a lane failure, and it names the key.
fn required<'a>(properties: &'a BTreeMap<String, String>, key: &str) -> LegacyResult<&'a String> {
    properties.get(key).ok_or_else(|| {
        LegacyError::Assertion(format!("the command that reports {key} did not report it"))
    })
}

/// A whitespace-separated list of decimal numbers.
fn integers(value: &str) -> LegacyResult<Vec<u64>> {
    value
        .split_whitespace()
        .map(|part| {
            part.parse::<u64>().map_err(|error| {
                LegacyError::Assertion(format!("{part:?} is not a number: {error}"))
            })
        })
        .collect()
}

/// A hexadecimal capability mask, as `/proc/<pid>/status` prints one.
fn hex(value: &str) -> LegacyResult<u64> {
    let trimmed = value.trim();
    let digits = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    u64::from_str_radix(digits, 16).map_err(|error| {
        LegacyError::Assertion(format!("{trimmed:?} is not a capability mask: {error}"))
    })
}

/// The full kernel capability mask at a given `cap_last_cap`.
fn full_capability_mask(cap_last_cap: u32) -> u64 {
    if cap_last_cap >= 64 {
        u64::MAX
    } else {
        (1_u64 << (cap_last_cap + 1)) - 1
    }
}

/// One rendered capability set, as the fixture's `parse_cap_set` read it.
///
/// An empty value is the full mask for a bounding set - an empty bounding set
/// is no bound at all, which is what the kernel starts a process with - and
/// zero for an ambient set. A value beginning `0x` is a mask. Anything else is
/// a list of capability names, each of which has to be one the kernel numbers
/// and no higher than `cap_last_cap`, because a unit that declares the latter
/// is declaring something this kernel cannot grant.
fn parse_cap_set(
    value: &str,
    cap_last_cap: u32,
    full_cap_mask: u64,
    empty_is_full: bool,
) -> LegacyResult<u64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(if empty_is_full { full_cap_mask } else { 0 });
    }
    if trimmed
        .get(..2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("0x"))
    {
        return hex(trimmed);
    }
    let mut mask = 0_u64;
    for token in trimmed.split_whitespace() {
        if token.is_empty() {
            continue;
        }
        let mut norm = token.to_uppercase().replace('-', "_");
        if let Some(stripped) = norm.strip_prefix("CAP_") {
            norm = stripped.to_owned();
        }
        let bit = CAPABILITY_NAMES
            .iter()
            .position(|name| *name == norm)
            .ok_or_else(|| {
                LegacyError::Assertion(format!("unknown capability from systemd unit: {token}"))
            })?;
        if bit as u32 > cap_last_cap {
            return Err(LegacyError::Assertion(format!(
                "systemd unit declares capability {token} above kernel cap_last_cap={cap_last_cap}"
            )));
        }
        mask |= 1 << bit;
    }
    Ok(mask)
}

/// One rendered systemd boolean, as the fixture's `parse_unit_bool` read it.
/// A value that is not one of the two forms is refused rather than guessed at.
fn parse_unit_bool(value: &str) -> LegacyResult<u32> {
    let norm = value.trim().to_lowercase();
    match norm.as_str() {
        "yes" | "true" | "1" => Ok(1),
        "no" | "false" | "0" | "" => Ok(0),
        _ => Err(LegacyError::Assertion(format!(
            "unknown systemd boolean value: '{value}'"
        ))),
    }
}

/// A list of numbers, rendered the way the fixture's own failure rendered
/// one.
fn render_integers(values: &[u64]) -> String {
    let rows = values
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{rows}]")
}

/// A list of strings, rendered the way the fixture's own failure rendered
/// one.
fn render_strings(values: &[String]) -> String {
    let rows = values
        .iter()
        .map(|value| format!("'{value}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{rows}]")
}
