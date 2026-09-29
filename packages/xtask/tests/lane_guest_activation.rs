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
    path::PathBuf,
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

/// A scratch directory carrying the two programs the tail reads, replaced.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn scratch(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    let dir = env::temp_dir()
        .join(format!("d2b-lane-activation-{}-{label}-{nanos}", std::process::id()));
    fs::create_dir_all(dir.join("bin")).expect("the scratch directory is created");
    for (name, source) in [
        (
            "cat",
            // The kernel's own report of the pool. Any other path is a change
            // to the file this test has not been taught about, so it is
            // refused rather than answered.
            r#"#!/bin/sh
for argument in "$@"; do
  case "$argument" in
    /proc/sys/kernel/random/*) printf '%s\n' "$D2B_FAKE_ENTROPY"; exit 0 ;;
  esac
done
echo "lane activation test: the activation tail read something else: $*" >&2
exit 99
"#,
        ),
        (
            "journalctl",
            // The boot journal, in the two shapes the readiness wait has to
            // survive. Arguments are ignored: the unit only ever reads it
            // whole.
            r#"#!/bin/sh
printf '%s\n' "$D2B_FAKE_JOURNAL"
"#,
        ),
    ] {
        let path = dir.join("bin").join(name);
        fs::write(&path, source).expect("the stub is written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("the stub is executable");
    }
    dir
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

/// Run the tail with the kernel reporting `entropy` bits and the journal
/// holding `journal`.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn activate(label: &str, entropy: &str, journal: &str) -> Activation {
    let dir = scratch(label);
    let console = dir.join("console.txt");
    let script = dir.join("activation.sh");
    fs::write(&script, harness(&render_tail())).expect("the harness is written");

    let path = format!(
        "{}:{}",
        dir.join("bin").display(),
        env::var("PATH").unwrap_or_default()
    );
    let mut child = Command::new("/bin/sh")
        .arg(&script)
        .env("PATH", path)
        .env("CONSOLE", &console)
        .env("D2B_FAKE_ENTROPY", entropy)
        .env("D2B_FAKE_JOURNAL", journal)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the activation tail runs under /bin/sh");

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
                panic!("the activation tail never returned: {label}");
            }
            Err(error) => panic!("waiting on the activation tail: {error}"),
        }
    };

    let console = fs::read_to_string(&console).unwrap_or_default();
    let _ = fs::remove_dir_all(&dir);
    Activation {
        status: status.code().unwrap_or(-1),
        console,
    }
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
