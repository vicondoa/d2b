#![forbid(unsafe_code)]

//! The lane guest's activation contract, pinned by running it.
//!
//! `nix/test-support/guest-image.nix` materialises `d2b-lane-activation`,
//! the unit whose `D2B_LANE_READY` line on the serial console is the whole
//! activation contract the launcher waits on. The unit is a shell script, and
//! its last act before that marker is a readiness wait, so the contract it
//! actually implements is only knowable by running it.
//!
//! This test lifts the tail of that unit out of the Nix file, renders the
//! interpolations the file's own comment documents, and runs it under `/bin/sh`
//! with the two things it reads faked: the boot journal and the kernel's
//! entropy report. A predicate that reads either input wrongly is observable
//! here, which is the point: the bug this pins was a wait on a kernel printk
//! that a boot can lose, and no assertion about the file's text would have
//! seen it.
//!
//! The lane's own host side lives in `d2b-test-vm-harness`, whose tests run in
//! the KVM lane rather than in the Layer-1 gate. This is where the gate can
//! reach it.

use std::{
    env, fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime},
};

/// A kernel that printed the message the old wait grepped for. The bug this
/// file pins was a boot that never got this line into its journal even though
/// the pool behind it was up.
const JOURNAL_WITH_THE_CRNG_PRINTK: &str =
    "[    0.019314] random: crng init done\n[    6.490000] systemd[1]: Started Journal Service.";

/// The same boot with the pre-journald ring-buffer content lost, which is what
/// the captured hang looked like: 748 forwarded entries, none of them from
/// before t=6.5s.
const JOURNAL_WITHOUT_IT: &str = "[    6.490000] systemd[1]: Started Journal Service.";

const MARKER: &str = "D2B_LANE_READY";
const POOL_FAILURE: &str = "the random pool was not initialised";

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn repo_file(relative: &str) -> String {
    let mut candidates = Vec::new();
    if let Some(base) = env::var_os("TEST_SRCDIR").map(PathBuf::from) {
        if let Some(workspace) = env::var_os("TEST_WORKSPACE") {
            candidates.push(base.join(workspace).join(relative));
        }
        candidates.push(base.join("_main").join(relative));
    }
    if let Some(root) = env::var_os("D2B_REPO_ROOT").map(PathBuf::from) {
        candidates.push(root.join(relative));
    }
    if let Ok(current_dir) = env::current_dir() {
        candidates.push(current_dir.join(relative));
    }
    // Cargo can run the integration tests with the package dir as CWD;
    // resolve from the manifest dir (packages/xtask -> repo root) too.
    // Runtime lookup (not env!): Bazel's process_wrapper forbids embedding
    // CARGO_MANIFEST_DIR at compile time, and under Bazel it is unset, so
    // the runfiles candidates above apply instead.
    if let Ok(manifest) = env::var("CARGO_MANIFEST_DIR") {
        candidates.push(PathBuf::from(manifest).join("../../").join(relative));
    }

    candidates
        .into_iter()
        .find_map(|path| fs::read_to_string(path).ok())
        .unwrap_or_else(|| panic!("repository file is not discoverable: {relative}"))
}

/// The unit's own script, from the line that announces the random-pool wait
/// through the marker it writes. The helpers above that tail are the unit's
/// own shell functions; the harness below supplies the three of them this tail
/// can reach.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn activation_tail() -> String {
    let source = repo_file("nix/test-support/guest-image.nix");
    let lines: Vec<&str> = source.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.trim() == "say \"waiting for the random pool\"")
        .unwrap_or_else(|| {
            panic!("the lane activation unit no longer announces the random-pool wait")
        });
    let marker = lines[start..]
        .iter()
        .position(|line| line.contains("${activationMarker}"))
        .unwrap_or_else(|| {
            panic!("the lane activation unit no longer writes its activation marker")
        });
    lines[start..=start + marker].join("\n")
}

/// The Nix file's `script` is an indented string: it escapes `${` and `''`,
/// and the file says so in the comment above the script. Resolving those is
/// the whole of the render; the rest of the tail is already the shell text
/// the guest runs.
fn render_tail() -> String {
    let rendered = activation_tail()
        .replace(">/dev/${serialDevice}", ">>\"$CONSOLE\"")
        .replace("${activationMarker}", MARKER)
        .replace("${nodeShape}", "daemon")
        .replace("''${", "${");
    for interpolation in ["${activationMarker}", "${nodeShape}", "${serialDevice}", "''${"] {
        assert!(
            !rendered.contains(interpolation),
            "the activation tail still carries the Nix interpolation {interpolation}:\n{rendered}"
        );
    }
    rendered
}

fn harness(body: &str) -> String {
    format!(
        r#"#!/bin/sh
set -eu
say() {{ printf '\nd2b-lane-activation: %s\n' "$1" >>"$CONSOLE"; }}
first_failed() {{ :; }}
report_failure() {{ say "$1 failed"; exit 1; }}
# The deadline is already in the past on purpose. The block under test has one
# bounded failure branch and reaching it is the point of two of these cases; a
# test that had to wait out a real 420s to get there would be a test nobody
# runs.
deadline=1
units=""
{body}
"#
    )
}

/// The kernel's own report of the pool. Any other path is a change to the
/// file this test has not been taught about, so it is refused rather than
/// answered.
const ENTROPY_CAT_STUB: &str = r#"#!/bin/sh
for argument in "$@"; do
  case "$argument" in
    /proc/sys/kernel/random/*) printf '%s\n' "$D2B_FAKE_ENTROPY"; exit 0 ;;
  esac
done
echo "lane activation test: the activation tail read something else: $*" >&2
exit 99
"#;

/// The boot journal, in the two shapes the readiness wait has to survive.
/// Arguments are ignored: the unit only ever reads it whole.
const JOURNALCTL_STUB: &str = r#"#!/bin/sh
printf '%s\n' "$D2B_FAKE_JOURNAL"
"#;

/// A scratch directory carrying the programs one run of the unit reads,
/// replaced. `stubs` is that run's whole set rather than a fixed pair, because
/// the entropy `cat` above refuses any path its test was not written for, and
/// a run that fakes a different pair of inputs has to be handed a different
/// one rather than layered on top of it.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn scratch(label: &str, stubs: &[(&str, &str)]) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let dir = env::temp_dir()
        .join(format!("d2b-lane-activation-{}-{label}-{nanos}", std::process::id()));
    fs::create_dir_all(dir.join("bin")).expect("the scratch directory is created");
    for (name, source) in stubs {
        let path = dir.join("bin").join(name);
        fs::write(&path, source).expect("the stub is written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("the stub is executable");
    }
    dir
}

/// The scratch stubs ahead of the real programs, so a stub standing in for a
/// coreutils name wins over the coreutils one.
fn stub_path(dir: &Path) -> String {
    format!(
        "{}:{}",
        dir.join("bin").display(),
        env::var("PATH").unwrap_or_default()
    )
}

/// What one run of the activation tail did: the unit's exit status and
/// everything it wrote to the serial console.
#[derive(Debug)]
struct Activation {
    status: i32,
    console: String,
}

impl Activation {
    fn activated(&self) -> bool {
        self.console.contains(MARKER)
    }
}

/// Run a lifted unit to completion and report what it wrote to the console
/// named by `CONSOLE`.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn run_activation(mut command: Command, console: &Path, label: &str) -> Activation {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the activation unit runs under /bin/sh");

    // Bounded, because a readiness loop is exactly the kind of thing that
    // stops: a test that can hang the gate is worse than no test.
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < Duration::from_secs(30) => {
                thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("the activation unit never returned: {label}");
            }
            Err(error) => panic!("waiting on the activation unit: {error}"),
        }
    };

    Activation {
        status: status.code().unwrap_or(-1),
        console: fs::read_to_string(console).unwrap_or_default(),
    }
}

/// Run the tail with the kernel reporting `entropy` bits and the journal
/// holding `journal`.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn activate(label: &str, entropy: &str, journal: &str) -> Activation {
    let dir = scratch(
        label,
        &[("cat", ENTROPY_CAT_STUB), ("journalctl", JOURNALCTL_STUB)],
    );
    let console = dir.join("console.txt");
    let script = dir.join("activation.sh");
    fs::write(&script, harness(&render_tail())).expect("the harness is written");

    let mut command = Command::new("/bin/sh");
    command
        .arg(&script)
        .env("PATH", stub_path(&dir))
        .env("CONSOLE", &console)
        .env("D2B_FAKE_ENTROPY", entropy)
        .env("D2B_FAKE_JOURNAL", journal);
    let activation = run_activation(command, &console, label);
    let _ = fs::remove_dir_all(&dir);
    activation
}

#[test]
fn a_pool_the_kernel_calls_initialised_activates_without_reading_the_journal() {
    // The regression this pins. The kernel reports the pool up and the
    // journal never received the printk that says so - the exact shape of the
    // captured hang, which sat on this wait for the launcher's whole 600s
    // bound with a pool that had been ready since 0.02s. A wait that reads
    // the journal cannot pass this.
    let activation = activate("ready-no-journal", "256", JOURNAL_WITHOUT_IT);

    assert_eq!(
        activation.status, 0,
        "a guest whose kernel reports the pool initialised must activate\n{}",
        activation.console
    );
    assert!(
        activation.activated(),
        "and it must write the activation marker\n{}",
        activation.console
    );
}

#[test]
fn a_journal_that_claims_the_pool_is_up_does_not_release_a_cold_one() {
    // The other direction. The line in the journal is a report the kernel
    // printed at 0.02s and never retracted; the entropy report is the pool's
    // state now. Reading the first in place of the second would declare a
    // guest ready whose getrandom() is still blocked, and the lane snapshots
    // exactly that guest.
    let activation = activate("cold-pool-warm-journal", "0", JOURNAL_WITH_THE_CRNG_PRINTK);

    assert_eq!(
        activation.status, 1,
        "a pool the kernel has not initialised must not activate\n{}",
        activation.console
    );
    assert!(
        !activation.activated(),
        "and the marker must not be written\n{}",
        activation.console
    );
    assert!(
        activation.console.contains(POOL_FAILURE),
        "the guest must say so itself\n{}",
        activation.console
    );
}

#[test]
fn a_cold_pool_fails_at_the_deadline_with_its_own_diagnosis() {
    // The bound is the other half of the contract: the guest has to reach a
    // failure of its own inside the launcher's 600s, or the launcher reports
    // an absence and the guest's reason never reaches the console tail.
    let activation = activate("cold-pool", "0", JOURNAL_WITHOUT_IT);

    assert_eq!(
        activation.status, 1,
        "a pool that never initialises must fail the unit\n{}",
        activation.console
    );
    assert!(
        activation.console.contains(POOL_FAILURE),
        "and it must fail with the diagnosis, not silently\n{}",
        activation.console
    );
    assert!(
        !activation.activated(),
        "a failed pool must not be reported as activation\n{}",
        activation.console
    );
}

#[test]
fn the_threshold_is_the_level_the_kernel_itself_uses() {
    // 256 bits is where the kernel declares the pool initialised, so 255 is
    // a pool that is not ready and 256 is one that is. Anything else here is
    // a different predicate wearing the same line.
    for (entropy, initialised) in [("255", false), ("256", true)] {
        let activation = activate(&format!("threshold-{entropy}"), entropy, JOURNAL_WITHOUT_IT);
        assert_eq!(
            activation.status,
            if initialised { 0 } else { 1 },
            "at {entropy} bits of entropy the pool is {}",
            if initialised { "initialised" } else { "not initialised" },
        );
        assert_eq!(
            activation.activated(),
            initialised,
            "at {entropy} bits the marker is {}written",
            if initialised { "" } else { "not " },
        );
    }
}

/// The console line every ordering report is opened with, and so the span the
/// launcher keeps whole.
const STALL_REPORT_OPENS: &str = "ordering state for ";

/// The guest's own activation bounds, read out of the Nix file rather than
/// restated here. What is under test is the relationship between the three
/// numbers as they ship, and a second copy of them in a test file is a copy
/// that can drift away from the first and leave the test passing on a
/// configuration the guest no longer has.
#[derive(Debug)]
struct Bounds {
    timeout: u64,
    stall: u64,
    repeat: u64,
}

impl Bounds {
    /// When each stall report comes due, in seconds after the unit began
    /// waiting for it: the first at the stall bound, every later one a repeat
    /// after the one before, and nothing at or past the deadline.
    fn reports_due(&self) -> Vec<u64> {
        let mut due = Vec::new();
        let mut at = self.stall;
        while at < self.timeout {
            due.push(at);
            at += self.repeat;
        }
        due
    }
}

fn activation_bounds() -> Bounds {
    let source = repo_file("nix/test-support/guest-image.nix");
    let declared = |name: &str| -> u64 {
        let prefix = format!("activation{name}Seconds = ");
        let value = source
            .lines()
            .find_map(|line| line.trim().strip_prefix(prefix.as_str()))
            .unwrap_or_else(|| panic!("the guest image no longer declares {prefix}"));
        value
            .trim()
            .trim_end_matches(';')
            .parse()
            .unwrap_or_else(|error| panic!("{prefix}is not a plain number: {value:?} ({error})"))
    };
    let bounds = Bounds {
        timeout: declared("Timeout"),
        stall: declared("Stall"),
        repeat: declared("StallRepeat"),
    };
    assert!(bounds.repeat > 0, "a repeat of zero would report every second");
    bounds
}

/// The launcher's own bound on the same wait, read from the harness line that
/// applies it. The guest's numbers mean nothing on their own: a report the
/// launcher has stopped reading is not a report.
fn launcher_activation_bound() -> u64 {
    let source = repo_file("packages/d2b-test-vm-harness/src/bin/d2b-test-vm-harness.rs");
    // Anchored on the `spec.` assignment, not on the environment variable: the
    // harness reads that variable from a second place, for a second bound.
    let marker = "spec.activation_timeout = Duration::from_secs(optional_u64(ACTIVATION_TIMEOUT, ";
    source
        .lines()
        .find_map(|line| line.split_once(marker))
        .unwrap_or_else(|| panic!("the harness no longer applies a default activation bound"))
        .1
        .split_once(')')
        .unwrap_or_else(|| panic!("the default activation bound is closed with a paren"))
        .0
        .trim()
        .parse()
        .unwrap_or_else(|error| panic!("the default activation bound is not a number ({error})"))
}

/// How finely the faked guest clock is sampled, in guest seconds. The stall
/// gate compares elapsed times against three fixed bounds and nothing else, so
/// the loop sees the same sequence of seconds for any tick that divides all
/// three - which the test checks before it runs. Sampling once a real second
/// would mean 420 turns of a shell loop to learn the same three numbers, and a
/// gate is not the place to spend that.
const CLOCK_TICK_SECONDS: u64 = 5;

/// The unit's view of systemd: nothing on the boot has failed, so the unit is
/// never diverted into its failure report, and the unit it polls never
/// activates, which is the state under test.
fn systemctl_stub() -> String {
    r#"#!/bin/sh
case "$1" in
  is-active) exit 1 ;;
  list-units)
    case "$2" in --failed) exit 0 ;; esac
    printf 'd2b-daemon.service loaded active running d2b-daemon.service\n'
    ;;
  show) printf 'd2b-daemon.service\n' ;;
esac
exit 0
"#
    .to_owned()
}

/// The guest's clock. Refuses any question the wait loop does not ask, so a
/// second clock read somewhere else in the unit is a failure here rather than
/// a run that quietly measures real time.
fn date_stub() -> String {
    r#"#!/bin/sh
if [ "$#" -ne 1 ] || [ "$1" != "+%s" ]; then
  echo "lane activation test: the wait loop asked the clock something else: $*" >&2
  exit 99
fi
cat "$D2B_TEST_CLOCK"
"#
    .to_owned()
}

/// The wait loop's own one-second pause, which here is where the guest's clock
/// moves instead of where the test's does.
fn sleep_stub() -> String {
    format!(
        r#"#!/bin/sh
if [ "$#" -ne 1 ] || [ "$1" != "1" ]; then
  echo "lane activation test: the wait loop slept for something other than one second: $*" >&2
  exit 99
fi
now=$(cat "$D2B_TEST_CLOCK")
echo $((now + {CLOCK_TICK_SECONDS})) >"$D2B_TEST_CLOCK"
"#
    )
}

/// The unit's own script, from its shell preamble through the close of the
/// wait loop. That range is the whole unit - every helper the loop reaches is
/// defined above it - so the stall gate runs here exactly as the guest runs
/// it, rather than as a restatement of it.
fn render_wait(bounds: &Bounds, units_file: &Path, accepted_units: &Path) -> String {
    let source = repo_file("nix/test-support/guest-image.nix");
    let lines: Vec<&str> = source.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.trim() == "set -eu")
        .unwrap_or_else(|| {
            panic!("the lane activation unit no longer opens with its own shell preamble")
        });
    let end = lines
        .iter()
        .position(|line| line.trim() == "done <\"$units_file\"")
        .unwrap_or_else(|| panic!("the lane activation unit no longer closes its wait loop"));
    let script = lines[start..=end]
        .join("\n")
        .replace(">/dev/${serialDevice}", ">>\"$CONSOLE\"")
        .replace(
            "/run/d2b-lane-acceptance-units",
            units_file.to_string_lossy().as_ref(),
        )
        .replace("${acceptanceUnitsFile}", accepted_units.to_string_lossy().as_ref())
        .replace(
            "${toString activationTimeoutSeconds}",
            &bounds.timeout.to_string(),
        )
        .replace("${toString activationStallSeconds}", &bounds.stall.to_string())
        .replace(
            "${toString activationStallRepeatSeconds}",
            &bounds.repeat.to_string(),
        )
        .replace("''${", "${");
    for interpolation in [
        "${serialDevice}",
        "${acceptanceUnitsFile}",
        "${toString",
        "''${",
        "/run/d2b-lane-acceptance-units",
    ] {
        assert!(
            !script.contains(interpolation),
            "the activation wait still carries {interpolation}:\n{script}"
        );
    }
    script
}

/// What one run of the wait loop did against a unit that never activates: the
/// unit's exit status, everything it wrote to the serial console, and the
/// seconds after it began waiting at which it described the stall.
#[derive(Debug)]
struct Stall {
    status: i32,
    console: String,
    times: Vec<u64>,
}

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn stall_run(bounds: &Bounds, label: &str) -> Stall {
    // The tick has to land the loop's samples on the same seconds a
    // one-second poll would, or the times below are this test's and not the
    // guest's.
    for bound in [bounds.stall, bounds.repeat, bounds.timeout] {
        assert_eq!(
            bound % CLOCK_TICK_SECONDS,
            0,
            "the faked clock samples every {CLOCK_TICK_SECONDS}s, which reproduces a \
             one-second poll only if it divides the {bound}s bound as well",
        );
    }

    let systemctl = systemctl_stub();
    let date = date_stub();
    let sleep = sleep_stub();
    let dir = scratch(
        label,
        &[
            ("systemctl", &systemctl),
            ("date", &date),
            ("sleep", &sleep),
            ("journalctl", JOURNALCTL_STUB),
        ],
    );
    let console = dir.join("console.txt");
    let clock = dir.join("clock");
    fs::write(&clock, "0\n").expect("the faked clock starts at zero");

    // The one unit the contract waits on, written where the unit looks for it.
    let accepted_units = dir.join("acceptance-units");
    fs::write(&accepted_units, "d2b-daemon.service\n").expect("the acceptance units are written");
    let units_file = dir.join("acceptance-units.copy");

    let script = dir.join("activation.sh");
    fs::write(&script, render_wait(bounds, &units_file, &accepted_units))
        .expect("the activation unit is written");

    let mut command = Command::new("/bin/sh");
    command
        .arg(&script)
        .env("PATH", stub_path(&dir))
        .env("CONSOLE", &console)
        .env("D2B_TEST_CLOCK", &clock)
        .env("D2B_FAKE_JOURNAL", JOURNAL_WITHOUT_IT);
    let activation = run_activation(command, &console, label);
    let _ = fs::remove_dir_all(&dir);

    Stall {
        times: stall_report_times(&activation.console),
        status: activation.status,
        console: activation.console,
    }
}

/// The seconds after the unit began waiting at which each stall report was
/// written, in the order the console carries them. The report the unit writes
/// at its own deadline names a different reason and is not one of these.
fn stall_report_times(console: &str) -> Vec<u64> {
    let mut times = Vec::new();
    let mut rest = console;
    while let Some(opened) = rest.find(STALL_REPORT_OPENS) {
        rest = &rest[opened + STALL_REPORT_OPENS.len()..];
        let reason = match rest.split_once('(') {
            Some((_, reason)) => reason,
            None => break,
        };
        let reason = reason.split(')').next().unwrap_or_default();
        if let Some(elapsed) = reason.strip_prefix("not active after ") {
            times.push(
                elapsed
                    .trim_end_matches('s')
                    .parse()
                    .expect("the unit writes the elapsed seconds as a number"),
            );
        }
    }
    times
}

#[test]
fn a_unit_that_never_activates_is_described_again_before_the_deadline() {
    // The configuration bug this pins. The guest's own activation deadline is
    // 420s while the repeat between stall reports was 600s, which put the
    // second report due at 690s - past a bound the unit had already hit. The
    // stall was described exactly once, at 90s, and a boot that was still
    // changing was reported from a single fixed account of its first ninety
    // seconds. The launcher's reader is built for a span: it keeps an ordering
    // report whole from the line the guest opened it with to the line it
    // closed it with, and one report leaves that span with nothing to span.
    let bounds = activation_bounds();
    let due = bounds.reports_due();
    assert!(
        due.len() > 1,
        "a unit that is not active for the whole {}s deadline is described {} time(s), and \
         one report is a snapshot of a boot that had not finished moving",
        bounds.timeout,
        due.len(),
    );

    let stall = stall_run(&bounds, "stall-repeats");

    assert_eq!(
        stall.status, 1,
        "a unit that never activates must fail the unit at its own deadline\n{}",
        stall.console
    );
    assert_eq!(
        stall.times, due,
        "the stall must be described at each bound the image sets\n{}",
        stall.console
    );
    assert!(
        stall
            .times
            .windows(2)
            .all(|pair| pair[1] - pair[0] >= bounds.repeat),
        "each report is a later look at a boot still moving, so no two of them land inside \
         one repeat of {}s\n{}",
        bounds.repeat,
        stall.console
    );
}

#[test]
fn every_stall_report_is_written_while_the_launcher_is_still_reading() {
    // The other half of the setting, and the half the old comment got wrong: it
    // claimed both values sat "well inside the launcher's own bound", and a
    // repeat equal to that bound is not inside it. A report is worth only what
    // the reader does with it, and the reader stops at its own deadline, so
    // every report has to land before that and before the guest's own shorter
    // one.
    let bounds = activation_bounds();
    let launcher = launcher_activation_bound();
    let stall = stall_run(&bounds, "stall-within-bounds");

    assert_eq!(
        stall.times.first().copied(),
        Some(bounds.stall),
        "the first report is due at the {}s stall bound\n{}",
        bounds.stall,
        stall.console
    );
    for at in &stall.times {
        assert!(
            *at < bounds.timeout,
            "a report at {at}s is past the guest's own {}s deadline\n{}",
            bounds.timeout,
            stall.console
        );
        assert!(
            *at < launcher,
            "a report at {at}s is past the launcher's own {launcher}s bound, where the \
             console is no longer being read\n{}",
            stall.console
        );
    }
    assert!(
        stall.console.contains("activation deadline reached"),
        "and the last thing the unit says before giving up is the ordering state at the \
         deadline, which is the one report that has to be there\n{}",
        stall.console
    );
}
