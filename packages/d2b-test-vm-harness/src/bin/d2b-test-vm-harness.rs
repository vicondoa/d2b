//! The lane's guest self-check.
//!
//! One invocation proves the three things the lane's own harness owns,
//! against a real guest built by the guest-image action from the re-homed
//! guest node:
//!
//!   1. the host satisfies every virtualization precondition, or the lane
//!      stops before a guest starts;
//!   2. the guest boots with exactly the invocation its configuration
//!      declares, reaches its activation contract, and attaches only
//!      devices that can carry a snapshot;
//!   3. tearing it down leaves no emulator process and no working directory
//!      behind, and doing that repeatedly changes nothing.
//!
//! Everything it needs arrives as runfile paths in the environment, because
//! that is how the Bazel test target hands it the emulator from the pinned
//! nix package set and the guest image from the guest-image action.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

use d2b_test_vm_harness::{
    ActiveGuest, Assertions, Footprint, GuestSpec, HarnessError, HostFacts, LegacyCheck,
    LegacyGuest, SnapshotPoint, boot, checks, host, manifest::GuestManifest, report,
};
use serde_json::json;

/// The image the lane was pointed at. Supplied by the Bazel test target as a
/// runfile path.
const IMAGE: &str = "D2B_TEST_VM_HARNESS_IMAGE";
/// The emulator binary, from the pinned nix package set the guest closure was
/// realized from.
const EMULATOR: &str = "D2B_TEST_VM_HARNESS_EMULATOR";
/// The lane's working directory, which outlives each individual guest.
const WORK_ROOT: &str = "D2B_TEST_VM_HARNESS_WORK_ROOT";
/// How long the guest has to activate.
const ACTIVATION_TIMEOUT: &str = "D2B_TEST_VM_HARNESS_ACTIVATION_TIMEOUT_SECS";
/// How many boot and teardown cycles to run.
const CYCLES: &str = "D2B_TEST_VM_HARNESS_CYCLES";

/// The block-graph node name the boot-time snapshot-capability proof attaches
/// its unsnapshottable device under.
const REFUSAL_NODE: &str = "lane_refusal";

/// The device id the proof attaches. The emulator reports a hot-attached
/// device by its qdev path rather than by this id, so the id exists to be
/// legible in the emulator's own diagnostics.
const REFUSAL_DEVICE: &str = "lane-refusal";

/// How large the proof's raw device is. The size is irrelevant to the
/// judgement - the format is what has no snapshot support - and only has to
/// be big enough for the guest to accept the disk.
const REFUSAL_DEVICE_BYTES: u64 = 8 * 1024 * 1024;

fn main() -> ExitCode {
    let mut arguments = env::args().skip(1);
    let outcome = match arguments.next().as_deref() {
        Some("lane") => run_lane(arguments.collect()),
        Some("self-check") | None => run().map(|report| {
            for line in report {
                report_line(&line);
            }
        }),
        Some(other) => Err(HarnessError::Configuration(format!(
            "unknown subcommand {other:?}; the harness runs the lane or a guest self-check"
        ))),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            report_line(&format!("FAIL {error}"));
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// The lane.
//
// One image per check, one pool member per distinct emulator invocation, one
// result per check. The shape of the run, and why each part is here:
//
//   * the images are read off the graph, each one built from its own check's
//     fixture, and each one's manifest says what that check's guest is;
//   * the checks are grouped by the invocation they declare, because that -
//     not the closure - is what decides whether two guests can be booted
//     side by side on one host and run one after another on one member;
//   * the pool is sized from the distinct invocations against the budget the
//     guest configurations declared and what this host has free, so the
//     bound is a number the run measured rather than a constant somebody
//     picked;
//   * every member is proven snapshottable from its declaration before the
//     first one boots, and again from the running guest's own block graph
//     before its snapshot is taken;
//   * a member that has run a nested guest is retired rather than restored;
//   * every check runs against a snapshot-restored copy of its own guest,
//     after a marker gate has compared that guest fresh against that guest
//     restored.

/// The image list the lane's target generated, one guest image per line.
const IMAGES: &str = "D2B_TEST_VM_HARNESS_IMAGES";
/// The checks a contributor selected, as the existing selection variables
/// carry them: a whitespace- or comma-separated list of check names.
const CHECKS: &str = "D2B_VM_CHECK";
/// Bazel's own filter, which `--test_filter` sets for a test action.
///
/// The flag is the one a contributor reaches for without being told this
/// lane has selection variables, and the lane ignoring it is the worst
/// available failure: `bazel test --test_filter=<check>` looks like it
/// selected one check and quietly runs all of them, which costs a pool of
/// guests and reports a verdict for a check nobody asked about.
const TEST_FILTER: &str = "TESTBRIDGE_TEST_ONLY";

/// One check, its own guest, and what that guest costs.
#[derive(Clone)]
struct LaneGuest {
    image_dir: PathBuf,
    manifest: GuestManifest,
    name: String,
    nested_guest: bool,
    /// Where this check's assertions are: the lane's own Rust, or the
    /// evaluated script its fixture is.
    assertions: Assertions,
    footprint: Footprint,
}

/// A set of checks whose guests are booted the same way, and which therefore
/// run one at a time rather than all at once.
#[derive(Clone)]
struct InvocationGroup {
    key: String,
    guests: Vec<LaneGuest>,
}

impl InvocationGroup {
    /// What one member of this group costs: the most expensive guest the
    /// group holds, because the group runs its members one after another and
    /// the expensive one is the one that has to fit.
    fn footprint(&self) -> Footprint {
        self.guests
            .iter()
            .map(|guest| guest.footprint)
            .max_by_key(|footprint| (footprint.memory_size_mib, footprint.cores))
            .unwrap_or(Footprint {
                memory_size_mib: 0,
                cores: 0,
                working_directory_mib: 0,
            })
    }

    /// The checks in this group, by name.
    fn names(&self) -> String {
        self.guests
            .iter()
            .map(|guest| guest.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// What one check produced, and what a reader of the lane's output needs to
/// act on it.
struct CheckResult {
    name: String,
    group: String,
    passed: bool,
    seconds: f64,
    detail: String,
}

/// The equivalence gate's marker: the guest-side facts a restored guest has
/// to reproduce for the restore to be indistinguishable from the fresh boot
/// the checks were written against.
///
/// It is a marker rather than a checksum on purpose. A checksum of the guest
/// would differ between the two by everything the boot legitimately changed
/// between the two moments - the uptime, the boot id, whatever a periodic
/// job wrote - and a gate that reports that difference has told the reader
/// nothing about whether the restore worked. What it compares instead is the
/// set of conditions each check's own assertions were written against: the
/// guest's declared activation units are active, the d2b host-tool set the
/// closure was built with is installed, the state disk the node mounted is
/// mounted where it mounted it, and the guest's random pool is out of its
/// initialising state. A restore that drops any of those is a restore that
/// would break a check, and that is the failure worth catching.
///
/// The guest's block devices are among them, by the name the guest's kernel
/// gave each one and its size: the kernel names a disk by the order the
/// devices were created, so a restore that put this member's disks back in
/// the emulator's report order rather than the launch's would hand the guest
/// a different `/dev/vda` than the fresh boot it is compared with - and every
/// fact a check anchors to a device identity (a volume-local marker anchors
/// its root by `(device, inode)`, and an inode means nothing without the
/// device it was read from) would be read against the wrong disk.
///
/// What this gate does *not* prove, and must not be read as proving: it
/// observes the guest's configuration surface as a booted guest reports it,
/// so it says nothing about RAM or in-flight state. The restore is disk-only -
/// an internal snapshot is refused while a VirtFS export is mounted in the
/// guest, and every lane guest mounts one - so a guest restored onto its disk
/// and reset is compared against the fresh boot at the same surface, not
/// across the memory each had.
const EQUIVALENCE_MARKER: &str = r#"
import subprocess
import sys

start_all()

def report(label, command):
    status, output = machine.execute(command, timeout=60)
    print("d2b-lane-marker {}={} {}".format(label, status, output.strip()))

units_file = "/etc/d2b/daemon-acceptance-units"
try:
    with open(units_file) as handle:
        units = [line.strip() for line in handle if line.strip()]
except OSError:
    units = ["multi-user.target"]

for unit in units:
    status, output = machine.execute(
        "systemctl show {} --property=ActiveState --property=SubState --property=Result"
        " --no-pager".format(unit),
        timeout=60,
    )
    print("d2b-lane-marker unit {}= {} {}".format(unit, status, output.strip().replace("\n", " ")))

report("acceptanceUnitsFile", "cat {}".format(units_file))
report("stateDisk", "findmnt -n -o TARGET,SOURCE,FSTYPE /var/lib/d2b || true")
report("hostTools", "ls /run/d2b-host-tools 2>/dev/null || ls /nix/var/nix/profiles/default/bin 2>/dev/null | head -n 0; echo inventory")
report("activation", "systemctl show d2b-lane-activation --property=ActiveState --property=Result --no-pager")
report("crng", "journalctl -b --no-pager -o cat | grep -c 'crng init done' || true")
report("backdoor", "systemctl is-active backdoor.service")
report("blockDevices", "for device in /sys/block/vd*; do printf '%s=%s ' \"$(basename \"$device\")\" \"$(cat \"$device/size\")\"; done; echo")
print("d2b-lane-marker complete")
"#;

/// Run the lane: read the images, size the pool, run the checks, report.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn run_lane(arguments: Vec<String>) -> Result<(), HarnessError> {
    let emulator = required_path(EMULATOR)?;
    let work_root = work_root(env::current_dir().map_err(|error| {
        HarnessError::io("reading the lane's working directory", error)
    })?)?;
    fs::create_dir_all(&work_root)
        .map_err(|error| HarnessError::io(format!("creating {}", work_root.display()), error))?;

    // The host preconditions gate everything after them, and they are asked
    // before a single guest image is read: a host that cannot run the lane
    // should learn so in a second, not after eleven closures.
    host::require_this_host()?;

    let selected = selection(&arguments);
    let guests = read_guests(&selected)?;
    if guests.is_empty() {
        return Err(HarnessError::Configuration(
            "no guest image carried a check the lane could select".to_owned(),
        ));
    }

    // A device that cannot carry an internal snapshot fails the lane here,
    // before the pool is built and before any guest boots. The running
    // guest's own block graph is asked again once it is up, but a pool that
    // boots five members and then refuses the sixth has spent the run.
    for guest in &guests {
        let refused = guest.manifest.unsnapshottable_drives();
        if !refused.is_empty() {
            return Err(HarnessError::Configuration(format!(
                "the guest image for check '{}' attaches a writable device that cannot carry an \
                 internal snapshot, and the lane will not run without restore: {}",
                guest.name,
                refused.join("; ")
            )));
        }
    }

    let groups = group_by_invocation(guests);
    let (admitted, deferred) = admit(&groups, &work_root)?;
    for group in admitted.iter().chain(deferred.iter()) {
        report_line(&format!(
            "pool: {} ({} vCPU, {} MiB, {} MiB of working directory) <- {}",
            group.names(),
            group.footprint().cores,
            group.footprint().memory_size_mib,
            group.footprint().working_directory_mib,
            group.key,
        ));
    }

    // Two waves, not one. A group the host cannot hold alongside the others
    // is deferred to a second wave, never dropped: dropping it left the lane
    // reporting green over a subset of its own checks, with the log
    // promising a second wave that nothing ever performed.
    let mut outcomes = run_wave(&admitted, &emulator, &work_root);
    if !deferred.is_empty() {
        report_line(&format!(
            "pool: second wave, {} now that the first wave's members have finished",
            deferred
                .iter()
                .map(|group| group.names())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        outcomes.extend(run_wave(&deferred, &emulator, &work_root));
    }

/// Run one wave of groups concurrently, and hand back what each reported.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn run_wave(
    groups: &[InvocationGroup],
    emulator: &Path,
    work_root: &Path,
) -> Vec<std::result::Result<CheckResult, (String, HarnessError)>> {
    if groups.is_empty() {
        return Vec::new();
    }
    thread::scope(|scope| {
        let handles: Vec<_> = groups
            .iter()
            .map(|group| scope.spawn(move || run_group(group, emulator, work_root)))
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| panic!("a pool group thread panicked"))
            })
            .collect()
    })
}

/// One invocation group's checks, in the order they were selected.
///
/// Sequential on purpose: a group is a set of checks whose guests are booted
/// the same way, and the pool restores a member between them rather than
/// booting a second member of the same shape beside it. The concurrency is
/// across groups, which is where the memory and the vCPUs differ.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn run_group(
    group: &InvocationGroup,
    emulator: &Path,
    work_root: &Path,
) -> Vec<std::result::Result<CheckResult, (String, HarnessError)>> {
    group
        .guests
        .iter()
        .map(|guest| run_check(guest, emulator, work_root))
        .collect()
}

    let mut results: Vec<CheckResult> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    for outcome in outcomes {
        match outcome {
            Ok(result) => {
                report_line(&format!(
                    "{}: {} in {:.1}s",
                    if result.passed { "PASS" } else { "FAIL" },
                    result.name,
                    result.seconds,
                ));
                if !result.passed {
                    failures.push(result.name.clone());
                }
                results.push(result);
            }
            Err((name, error)) => {
                report_line(&format!("FAIL {name}: {error}"));
                failures.push(name.clone());
                results.push(CheckResult {
                    name,
                    group: String::new(),
                    passed: false,
                    seconds: 0.0,
                    detail: error.to_string(),
                });
            }
        }
    }

    write_junit(&results)?;
    if failures.is_empty() {
        report_line(&format!("lane: {} checks, all green", results.len()));
        Ok(())
    } else {
        Err(HarnessError::Configuration(format!(
            "{} of {} checks failed: {}",
            failures.len(),
            results.len(),
            failures.join(", ")
        )))
    }
}

/// The checks this run was asked for, in the order the three sources are
/// consulted: the target's own `--check` argument, then the selection
/// variable contributors already use, then Bazel's own `--test_filter`.
/// The first that names anything wins, so an explicit request is never
/// widened by a default that happens to be set.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn selection(arguments: &[String]) -> Vec<String> {
    let from_arguments: Vec<String> = arguments
        .iter()
        .skip_while(|argument| *argument != "--check")
        .skip(1)
        .flat_map(|argument| argument.split([',', ' ']))
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    if !from_arguments.is_empty() {
        return from_arguments;
    }
    let named = env::var(CHECKS).unwrap_or_default();
    if !named.trim().is_empty() {
        return split_names(&named);
    }
    split_names(&env::var(TEST_FILTER).unwrap_or_default())
}

/// One selection, however it was written: a whitespace-, comma- or
/// newline-separated list of check names.
fn split_names(selection: &str) -> Vec<String> {
    selection
        .split([',', ' ', '\n', '\t'])
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Every check the lane can run, read off the images the target declared.
///
/// A name is matched against both the check's own name and the name its
/// fixture gave it, because a contributor filtering the lane has whichever
/// of the two they have read.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn read_guests(selected: &[String]) -> Result<Vec<LaneGuest>, HarnessError> {
    let listing = fs::read_to_string(required_path(IMAGES)?).map_err(|error| {
        HarnessError::io("reading the lane's image list", error)
    })?;
    let mut guests = Vec::new();
    for line in listing.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let image_dir = resolve_runfile(PathBuf::from(line));
        let manifest = GuestManifest::load(&image_dir)?;
        let Some(check) = &manifest.check else {
            continue;
        };
        if !selected.is_empty()
            && !selected.iter().any(|name| name == &check.name || name == &check.test_name)
        {
            continue;
        }
        guests.push(LaneGuest {
            image_dir,
            footprint: manifest.footprint,
            name: check.name.clone(),
            nested_guest: check.nested_guest,
            assertions: check.assertions,
            manifest,
        });
    }
    guests.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(guests)
}

/// A path out of the lane's image listing, resolved the way the rest of the
/// harness resolves a runfile.
///
/// The listing is generated by the test target from `$(rlocationpath)`, so
/// every line is relative to the *runfiles root* - `_main/...` - while a
/// test's own working directory is the `_main` directory inside that tree.
/// Taking a line as written would look for `_main/_main/...`. An absolute path
/// is left alone, and with no runfiles tree declared the path is the caller's
/// own, which is what a contributor running the harness by hand means.
fn resolve_runfile(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }
    let root = [env::var_os("RUNFILES_DIR"), env::var_os("TEST_SRCDIR")]
        .into_iter()
        .flatten()
        .next();
    match root {
        Some(root) => PathBuf::from(root).join(&path),
        None => path,
    }
}

/// Group the checks by the invocation their guests declare.
///
/// The order is the group's cheapest member first and the group name after
/// it, so the pool admits the smallest groups first and the admission is
/// reproducible rather than dependent on which label the graph happened to
/// list.
fn group_by_invocation(guests: Vec<LaneGuest>) -> Vec<InvocationGroup> {
    let mut groups: Vec<InvocationGroup> = Vec::new();
    for guest in guests {
        let key = guest.manifest.invocation_key();
        match groups.iter_mut().find(|group| group.key == key) {
            Some(group) => group.guests.push(guest),
            None => groups.push(InvocationGroup {
                key,
                guests: vec![guest],
            }),
        }
    }
    groups.sort_by(|left, right| {
        let left_cost = left.footprint();
        let right_cost = right.footprint();
        (left_cost.memory_size_mib, left_cost.cores, left.key.clone())
            .cmp(&(right_cost.memory_size_mib, right_cost.cores, right.key.clone()))
    });
    groups
}

/// How many of the groups this host can hold at once, and which.
///
/// The pool's size is the number of distinct invocations, bounded by what the
/// host has: the budget the guest configurations declared, applied to the
/// memory and the vCPUs this host has free, and the working directory the
/// admitted members will copy their root disks into. A group that does not
/// fit is not an error - it waits for a member to finish - and a host that
/// cannot hold even the cheapest group is a host that cannot run the lane at
/// all, which is worth saying plainly rather than booting a guest that
/// cannot get the memory its node declared.
/// The groups that fit the budget now, and the ones deferred to a second wave.
fn admit(
    groups: &[InvocationGroup],
    work_root: &Path,
) -> Result<(Vec<InvocationGroup>, Vec<InvocationGroup>), HarnessError> {
    let first = groups
        .first()
        .ok_or_else(|| HarnessError::Configuration("the lane has no checks to run".to_owned()))?;
    let budget = first.guests[0].manifest.pool;
    let available_memory = HostFacts::available_memory_mib().ok_or_else(|| {
        HarnessError::Configuration(
            "this host does not report how much memory it has free, so the pool cannot be sized \
             against a budget rather than a guess"
                .to_owned(),
        )
    })?;
    let available_cores = HostFacts::available_cores();
    // Multiply first, then divide, the way the core budget below does.
    // Written as `available * numerator.checked_div(denominator)` the
    // method call binds to the numerator alone, so the share is taken of `2`
    // rather than of what this host has, and a two-in-three budget is a
    // truncation to zero - a pool that admits nothing, reported as a
    // configuration the guest never got to say anything about.
    let memory_budget = available_memory
        .checked_mul(budget.memory_share_numerator)
        .map(|scaled| scaled / budget.memory_share_denominator.max(1))
        .filter(|share| *share > 0)
        .ok_or_else(|| {
            HarnessError::Configuration(
                "the declared memory budget is not a share of anything".to_owned(),
            )
        })?;
    let core_budget = (available_cores * budget.core_share_numerator
        / budget.core_share_denominator.max(1))
        .max(1);
    report_line(&format!(
        "budget: {} MiB of the {} MiB this host has free, and {} of its {} vCPUs",
        memory_budget,
        available_memory,
        core_budget,
        available_cores,
    ));

    let free_directory = HostFacts::free_working_directory_mib(work_root);
    let mut memory_used: u64 = 0;
    let mut cores_used: u64 = 0;
    let mut directory_used: u64 = 0;
    let mut admitted = Vec::new();
    let mut deferred: Vec<InvocationGroup> = Vec::new();
    for (index, group) in groups.iter().enumerate() {
        let cost = group.footprint();
        let fits_memory = memory_used + cost.memory_size_mib <= memory_budget;
        let fits_cores = cores_used + u64::from(cost.cores) <= core_budget;
        let fits_directory = free_directory.is_none_or(|free| {
            directory_used + cost.working_directory_mib <= free
        });
        if index == 0 && !(fits_memory && fits_cores) {
            return Err(HarnessError::Configuration(format!(
                "this host cannot hold even the cheapest pool member: the smallest invocation \
                 needs {} MiB and {} vCPUs, and the budget is {} MiB and {} vCPUs out of {} MiB \
                 and {} vCPUs free",
                cost.memory_size_mib,
                cost.cores,
                memory_budget,
                core_budget,
                available_memory,
                available_cores
            )));
        }
        if !(fits_memory && fits_cores && fits_directory) {
            report_line(&format!(
                "pool: {} does not fit the budget yet and will run in a second wave once the \
                 first wave's members have finished",
                group.names()
            ));
            deferred.push(group.clone());
            continue;
        }
        memory_used += cost.memory_size_mib;
        cores_used += u64::from(cost.cores);
        directory_used += cost.working_directory_mib;
        admitted.push(group.clone());
    }
    if let Some(free) = free_directory {
        report_line(&format!(
            "working directory: {} MiB needed for the admitted pool, {} MiB free at {}",
            directory_used,
            free,
            work_root.display()
        ));
    } else {
        report_line(&format!(
            "working directory: {} MiB needed for the admitted pool; free space at {} could not \
             be measured, so the declared bound is enforced by each member's own copy",
            directory_used,
            work_root.display()
        ));
    }
    Ok((admitted, deferred))
}
/// Boot one check's own guest, prove the restore against it, and run the
/// check on a restored copy of it.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn run_check(
    guest: &LaneGuest,
    emulator: &Path,
    work_root: &Path,
) -> std::result::Result<CheckResult, (String, HarnessError)> {
    let name = guest.name.clone();
    let started = Instant::now();
    let outcome = run_check_inner(guest, emulator, work_root, started);
    match outcome {
        Ok(result) => Ok(result),
        Err(error) => Err((name, error)),
    }
}

#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn run_check_inner(
    guest: &LaneGuest,
    emulator: &Path,
    work_root: &Path,
    started: Instant,
) -> Result<CheckResult, HarnessError> {
    let mut spec = GuestSpec::new(
        guest.manifest.clone(),
        &guest.image_dir,
        emulator,
        work_root,
        &guest.name,
    );
    spec.activation_timeout = Duration::from_secs(
        optional_u64(ACTIVATION_TIMEOUT, 1800)?,
    );
    let mut active = boot(&spec)?;
    // The declared pass said this guest's drives can be snapshotted; the
    // running guest's own block graph is the authority, and it is asked
    // before the snapshot rather than at the first restore.
    active.require_snapshottable()?;
    let mut point = SnapshotPoint::new(&spec)?;
    let marker = guest.manifest.activation.marker.clone();
    let activation_bound = spec.activation_timeout;

    let mut surface = LegacyGuest::attach(&mut active)?;
    let fresh = surface
        .run(&LegacyCheck::new(format!("{}-marker", guest.name), EQUIVALENCE_MARKER))
        .map_err(|error| {
            HarnessError::Configuration(format!(
                "the equivalence marker could not be read from the fresh guest: {error}"
            ))
        })?;
    if !fresh.passed {
        return Err(HarnessError::Configuration(format!(
            "the equivalence marker failed against a freshly booted guest, so it cannot say \
             anything about a restored one:\n{}",
            fresh.detail
        )));
    }
    // The member's snapshot, taken here: the guest has activated, every
    // writable device has been proven restorable, and no check has run.
    active.take_snapshot(&mut point)?;
    report_line(&format!(
        "{}: snapshot taken against every writable device, the guest's own disks underneath",
        guest.name
    ));

    // The gate. What it proves is narrower than a RAM+disk restore could have
    // proved, and it says so: the restore is disk-only, so what is compared
    // is a guest *booted from the snapshot-restored disk* against the fresh
    // boot the check's assertions were written against - same units, same
    // mounts, same host tools, same random pool, reported by the guest
    // itself.
    let restored = restore_and_measure(
        &mut active,
        &mut surface,
        &mut point,
        &guest.name,
        &marker,
        activation_bound,
    )?;
    let restored_marker = surface
        .run(&LegacyCheck::new(format!("{}-marker", guest.name), EQUIVALENCE_MARKER))
        .map_err(|error| {
            HarnessError::Configuration(format!(
                "the equivalence marker could not be read from the restored guest: {error}"
            ))
        })?;
    if !restored_marker.passed {
        return Err(HarnessError::Configuration(format!(
            "the equivalence marker failed against a guest booted from the restored disk:\n{}",
            restored_marker.detail
        )));
    }
    let fresh_text = marker_text(&fresh.detail);
    let restored_text = marker_text(&restored_marker.detail);
    if fresh_text != restored_text {
        return Err(HarnessError::Configuration(format!(
            "a guest booted from the snapshot-restored disk is not the fresh boot its check was \
             written against\n--- fresh boot\n{}\n--- booted from the restored disk\n{}",
            fresh_text, restored_text
        )));
    }
    report_line(&format!(
        "{}: a guest booted from the restored disk matches the fresh boot on every marker \
         ({} bytes); the restore itself took {restored:.1}s",
        guest.name,
        fresh_text.len()
    ));

    // The check runs on a guest that was handed back by a restore, never on
    // the one the gate read its markers from: the marker runs are themselves
    // writes, and a check that inherited them would be a check handed state
    // no fresh member would have had.
    let second = restore_and_measure(
        &mut active,
        &mut surface,
        &mut point,
        &guest.name,
        &marker,
        activation_bound,
    )?;
    report_line(&format!(
        "{}: second restore onto the same snapshot took {second:.1}s and wrote a layer of its own",
        guest.name
    ));
    let outcome = match guest.assertions {
        Assertions::Rust => {
            let assertions = checks::assertions(&guest.name).ok_or_else(|| {
                HarnessError::Configuration(format!(
                    "the guest image for check '{}' says its assertions are the lane's own, and \
                     the lane has no module that carries them",
                    guest.name
                ))
            })?;
            surface.run_ported(&guest.name, assertions)?
        }
        Assertions::Python => {
            let script = fs::read_to_string(guest.image_dir.join("check.py")).map_err(|error| {
                HarnessError::io(
                    format!("reading the assertions of check '{}'", guest.name),
                    error,
                )
            })?;
            surface.run(&LegacyCheck::new(&guest.name, script))?
        }
    };
    let seconds = started.elapsed().as_secs_f64();
    report_line(&format!(
        "{}: {} after the restore ({seconds:.1}s total on this member)",
        guest.name,
        if guest.nested_guest {
            "ran a nested guest; this member is retired rather than restored"
        } else {
            "ran against a guest booted from the member's snapshot"
        },
    ));
    if guest.nested_guest {
        // Retired, not restored. A guest with a live guest inside it has no
        // defined restored state, and a member that has run a nested guest
        // is torn down here rather than handed to the next check.
        report_line(&format!(
            "{}: retired without another restore; every layer it wrote is removed with its \
             working directory",
            guest.name
        ));
    }
    active.shutdown()?;
    Ok(CheckResult {
        name: guest.name.clone(),
        group: guest.manifest.invocation_key(),
        passed: outcome.passed,
        seconds,
        detail: outcome.detail,
    })
}

/// Put the member back on its snapshot, wait for the guest to boot from the
/// restored disk, and hand the command channel to the guest's new shell.
///
/// The console resynchronisation is the part that is easy to get wrong: the
/// channel is one connection for the life of the emulator process, so the
/// bytes the guest wrote while it was shutting down are still in it when the
/// new guest comes up. Reading a command's output from that position decodes
/// whatever the old guest left behind.
///
/// Two durations are reported, because they are two different costs and only
/// one of them is the pool's: the restore is the lane's own block-graph work
/// on the host, and the wait is the guest rebooting onto the disk it was
/// handed. A pool whose member is restored between checks pays both, so a
/// measurement that stopped at the first would compare a restore against a
/// fresh boot and leave out the boot.
fn restore_and_measure(
    active: &mut ActiveGuest,
    surface: &mut LegacyGuest,
    point: &mut SnapshotPoint,
    name: &str,
    marker: &str,
    bound: Duration,
) -> Result<f64, HarnessError> {
    let seen = active.activations(marker);
    let seconds = active.restore(point)?.as_secs_f64();
    let waiting = Instant::now();
    active.await_reactivation(seen, bound, marker)?;
    let reactivation = waiting.elapsed().as_secs_f64();
    surface.resync()?;
    report_line(&format!(
        "{name}: restored onto the snapshot in {seconds:.1}s, and the guest activated again \
         {reactivation:.1}s after that"
    ));
    Ok(seconds)
}

/// The marker lines out of a check's own output, with the lane's own log
/// lines around them dropped: the surface's log describes what it did, and
/// two runs of the same gate legitimately log different durations.
fn marker_text(detail: &str) -> String {
    detail
        .lines()
        .filter(|line| line.trim_start().starts_with("d2b-lane-marker"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Write the lane's JUnit document: one testcase per check, carrying that
/// check's own diagnostics.
///
/// Bazel names the file through `XML_OUTPUT_FILE` when it runs a test under
/// its XML wrapper, which is the default; the fallback keeps the document
/// findable when the harness is run by hand.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn write_junit(results: &[CheckResult]) -> Result<(), HarnessError> {
    let path = env::var_os("XML_OUTPUT_FILE").map(PathBuf::from).unwrap_or_else(|| {
        let directory = env::var_os("TEST_SRCDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let target = env::var("TEST_TARGET").unwrap_or_else(|_| "host_integration_lane".to_owned());
        directory.join(format!("{}.test.xml", target.replace(['/', ':'], "_")))
    });
    let mut document = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites>\n");
    for result in results {
        document.push_str(&format!(
            "  <testsuite name=\"{}\" tests=\"1\" failures=\"{}\">\n    <testcase classname=\"d2b.host-integration.{}\" name=\"{}\" time=\"{seconds:.3}\">\n",
            escape(&result.group),
            u8::from(!result.passed),
            escape(&result.group),
            escape(&result.name),
            seconds = result.seconds,
        ));
        if result.passed {
            document.push_str("    </testcase>\n");
        } else {
            document.push_str(&format!(
                "      <failure message=\"check {} did not pass\">{}</failure>\n    </testcase>\n",
                escape(&result.name),
                escape(&result.detail),
            ));
        }
        document.push_str("  </testsuite>\n");
    }
    document.push_str("</testsuites>\n");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| HarnessError::io(format!("creating {}", parent.display()), error))?;
    }
    fs::write(&path, document)
        .map_err(|error| HarnessError::io(format!("writing {}", path.display()), error))
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The lane's working directory, resolved against the caller's own directory
/// when the caller named a relative one. The lane's directory has to outlive
/// each individual guest, so it cannot be a per-action temporary directory
/// the current Bazel release does not expose to a sandboxed action.
fn work_root(caller_dir: std::path::PathBuf) -> Result<PathBuf, HarnessError> {
    let declared = required_path(WORK_ROOT)?;
    Ok(if declared.is_absolute() {
        declared
    } else {
        caller_dir.join(declared)
    })
}

fn report_line(line: &str) {
    report(line);
}

fn run() -> Result<Vec<String>, HarnessError> {
    let image_dir = required_path(IMAGE)?;
    let emulator = required_path(EMULATOR)?;
    let work_root = work_root(env::current_dir().map_err(|error| {
        HarnessError::io("reading the lane's working directory", error)
    })?)?;
    let activation_timeout = Duration::from_secs(optional_u64(ACTIVATION_TIMEOUT, 1200)?);
    let cycles = optional_u64(CYCLES, 1)?;

    // The host preconditions come first and gate everything after them: a
    // host that cannot run the lane must stop before a guest is built or
    // booted, not degrade into a slower run.
    let facts = host::require_this_host()?;
    let mut lines = vec![format!(
        "host: /dev/kvm usable, nested virtualization and nested-state save present ({})",
        facts
            .module_parameters
            .iter()
            .map(|(name, value)| format!("{name}={}", value.as_deref().unwrap_or("absent")))
            .collect::<Vec<_>>()
            .join(" ")
    )];

    let manifest = GuestManifest::load(&image_dir)?;
    let contract = manifest.activation.contract.clone();
    lines.push(format!(
        "image: shape {} booted {} with {} vCPU and {} MiB of memory on a {} MiB {} disk",
        manifest.node_shape,
        manifest.boot.method,
        manifest.machine.cores,
        manifest.machine.memory_size_mib,
        manifest.image.disk_size_mib,
        manifest.image.disk_format,
    ));

    let mut spec = GuestSpec::new(
        manifest,
        image_dir,
        emulator,
        work_root,
        "lane-harness",
    );
    spec.activation_timeout = activation_timeout;
    let work_dir = spec.work_dir();

    for cycle in 0..cycles {
        lines.push(format!("cycle {}: booting", cycle + 1));
        let mut guest = boot(&spec)?;
        guest.require_snapshottable()?;
        lines.push(format!(
            "cycle {}: activated ({contract}), every attached writable device carries a snapshot",
            cycle + 1,
        ));
        guest.shutdown()?;
        require_removed(&work_dir)?;
        lines.push(format!(
            "cycle {}: torn down, no emulator process and no working directory left",
            cycle + 1
        ));
    }

    lines.push(refuse_unsnapshottable_device(&spec)?);
    Ok(lines)
}

/// A guest that left its working directory behind has not been torn down, and
/// a second cycle would inherit whatever the first one left.
fn require_removed(work_dir: &Path) -> Result<(), HarnessError> {
    if work_dir.exists() {
        return Err(HarnessError::Configuration(format!(
            "{} survived teardown",
            work_dir.display()
        )));
    }
    Ok(())
}

/// Prove, against a real booted guest, that a writable device the lane
/// cannot snapshot stops the lane at boot.
///
/// The device is a raw image inside the guest's own working directory,
/// attached through the monitor the way any other device is attached, and it
/// is the case the refusal exists for: a writable node whose format has no
/// internal snapshot support, which a later `snapshot-save` would refuse for
/// the whole guest rather than for that one node. The guest is judged once
/// before the device is attached and once after, so the refusal is
/// attributable to the device rather than to the guest.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn refuse_unsnapshottable_device(spec: &GuestSpec) -> Result<String, HarnessError> {
    let work_dir = spec.work_dir();
    let mut guest = boot(spec)?;
    guest.require_snapshottable()?;

    let image = work_dir.join("refusal.img");
    let backing = fs::File::create(&image)
        .map_err(|error| HarnessError::io(format!("creating {}", image.display()), error))?;
    backing
        .set_len(REFUSAL_DEVICE_BYTES)
        .map_err(|error| HarnessError::io(format!("sizing {}", image.display()), error))?;
    drop(backing);

    let monitor = guest
        .monitor()
        .ok_or_else(|| HarnessError::Configuration("the guest has no monitor".to_owned()))?;
    monitor.execute_with(
        "blockdev-add",
        json!({
            "node-name": REFUSAL_NODE,
            "driver": "raw",
            "file": { "driver": "file", "filename": image.to_string_lossy() },
        }),
    )?;
    monitor.execute_with(
        "device_add",
        json!({
            "driver": "virtio-blk-pci",
            "drive": REFUSAL_NODE,
            "id": REFUSAL_DEVICE,
        }),
    )?;

    let error = guest
        .require_snapshottable()
        .expect_err("a writable raw device stops the lane at boot");
    let HarnessError::NotSnapshottable { devices } = error else {
        return Err(HarnessError::Configuration(format!(
            "a writable raw device was refused for the wrong reason: {error}"
        )));
    };
    let named = devices
        .iter()
        .map(|device| device.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let refused = devices
        .iter()
        .find(|device| device.file == image.to_string_lossy())
        .ok_or_else(|| {
            HarnessError::Configuration(format!(
                "the refusal did not name the device the lane attached, but named: {named}"
            ))
        })?
        .clone();
    guest.shutdown()?;
    require_removed(&work_dir)?;
    Ok(format!(
        "boot-time snapshot capability: a writable raw device stops the lane before any check runs \
         ({} bytes at {}, refused as {} on {})",
        REFUSAL_DEVICE_BYTES,
        refused.file,
        refused.format,
        refused.device,
    ))
}

fn required_path(name: &str) -> Result<PathBuf, HarnessError> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| HarnessError::Configuration(format!("{name} is not set")))
}

fn optional_u64(name: &str, default: u64) -> Result<u64, HarnessError> {
    match env::var(name) {
        Err(_) => Ok(default),
        Ok(value) if value.is_empty() => Ok(default),
        Ok(value) => value.parse().map_err(|error| {
            HarnessError::Configuration(format!("{name} is not a number: {value} ({error})"))
        }),
    }
}
