#!/usr/bin/env python3
"""Regression coverage for the CI shell and local Layer-1 runner."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import json
import os
import pathlib
import re
import signal
import shlex
import shutil
import subprocess
import tempfile
import time
import types
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
SCRATCH = ROOT / ".scratch"
LAYER1_JOBS = ROOT / "tests" / "tools" / "layer1-jobs.py"
MAKEFILE = ROOT / "Makefile"
RUST_DRIVER = ROOT / "tests" / "test-rust.sh"
EXECUTION_MANIFEST_HELPER = ROOT / "tests" / "tools" / "execution-manifest.pl"


EXECUTION_MANIFEST_HARNESS = r"""
use strict;
use warnings;
use Errno qw(ECHILD);
use POSIX qw(WNOHANG);

my ($helper, $scenario, $manifest) = @ARGV;

sub fail {
    print STDERR "manifest-harness: $_[0]\n";
    exit 1;
}

fail("missing harness arguments")
    unless defined($helper) && defined($scenario) && defined($manifest);
require $helper;

my $manifest_parent = $manifest;
$manifest_parent =~ s{[^/]*\z}{};
my $manifest_base = $manifest;
$manifest_base =~ s{.*/}{};
my $fragment_dir = $manifest_parent . ".$manifest_base.fragments";

sub descriptor_snapshot {
    opendir(my $dir, "/proc/$$/fd") or fail("could not inspect descriptors");
    my @snapshot;
    while (my $entry = readdir($dir)) {
        next unless $entry =~ /\A\d+\z/;
        my $target = readlink("/proc/$$/fd/$entry");
        next unless defined($target);
        push @snapshot, "$entry\0$target";
    }
    closedir($dir) or fail("could not close descriptor inspection");
    return [sort @snapshot];
}

sub same_snapshot {
    my ($left, $right) = @_;
    return join("\n", @{$left}) eq join("\n", @{$right});
}

my $before = descriptor_snapshot();
my $path_calls = 0;
my $fork_calls = 0;
my $subreaper_calls = 0;
my $sleep_calls = 0;
my $clock_calls = 0;
my $active_child = 0;
my @kills;
my @waits;
my @descendant_reaps = (9001, 9002, -1);
my $scheduler_status = $scenario eq "failure-finalization" ? 37 : 0;
my $path_boundary = sub {
    my ($raw) = @_;
    fail("path boundary received an unexpected path") unless $raw eq $manifest;
    ++$path_calls;
    return main::open_manifest_parent($raw);
};

my $clock = sub {
    my @ticks = (0, 10);
    my $value = $ticks[$clock_calls];
    ++$clock_calls;
    return defined($value) ? $value : 10;
};

my $process_control = {
    fork => sub {
        ++$fork_calls;
        $active_child = 1;
        if ($scenario eq "term") {
            main::write_atomic_fragment($manifest, "rust-main-workspace", "passed");
            $SIG{TERM}->();
        } elsif ($scenario eq "success-finalization"
            || $scenario eq "failure-finalization") {
            main::write_atomic_fragment($manifest, "rust-main-workspace", "passed");
            no warnings qw(once redefine);
            *main::renameat_name = sub {
                main::fatal("synthetic publication failure");
            };
        }
        return 4242;
    },
    subreaper => sub {
        ++$subreaper_calls;
    },
    kill => sub {
        my ($signal, $group) = @_;
        push @kills, [$signal, $group];
        fail("unexpected process-control kill") unless $scenario eq "term";
        return 1;
    },
    waitpid => sub {
        my ($pid, $flags) = @_;
        if ($pid == -1) {
            push @waits, [$pid, $flags];
            my $adopted = shift @descendant_reaps;
            if ($adopted == -1) {
                $! = ECHILD;
            }
            return $adopted;
        }
        push @waits, [$pid, $flags];
        fail("unexpected process-control pid") unless $pid == 4242;
        if ($scenario eq "term") {
            fail("grace wait was not skipped") if $flags == WNOHANG;
            $active_child = 0;
            $? = 0;
            return $pid;
        }
        $? = $scheduler_status << 8;
        $active_child = 0;
        return $pid;
    },
};

my $sleep = sub {
    ++$sleep_calls;
    fail("real grace sleep was reached");
};

my $status = main::run_manifest_lifecycle(
    command => [$^X, "-e", "exit 0"],
    manifest => $manifest,
    target => "test-rust",
    commit => "injected-test",
    path_boundary => $path_boundary,
    clock => $clock,
    sleep => $sleep,
    process_control => $process_control,
);

fail("path boundary was not used exactly once") unless $path_calls == 1;
fail("scheduler fork was not injected exactly once") unless $fork_calls == 1;
my $expected_subreaper_calls = $scenario eq "term" ? 1 : 0;
fail("child subreaper setup did not match the shutdown-only contract")
    unless $subreaper_calls == $expected_subreaper_calls;
fail("injected child survived") if $active_child;
fail("evidence descriptor survived") unless same_snapshot($before, descriptor_snapshot());

if ($scenario eq "success-finalization") {
    fail("scheduler-success finalization did not return 74") unless $status == 74;
    fail("scheduler-success finalization unexpectedly published evidence")
        if -e $manifest;
} elsif ($scenario eq "failure-finalization") {
    fail("scheduler failure status was not preserved") unless $status == 37;
    fail("scheduler-failure finalization unexpectedly published evidence")
        if -e $manifest;
} elsif ($scenario eq "term") {
    fail("handled TERM did not preserve 143") unless $status == 143;
    fail("TERM did not use the injected zero-grace clock")
        unless $clock_calls == 2 && $sleep_calls == 0;
    my @expected_kills = ([15, -4242], [0, -4242], [9, -4242]);
    fail("TERM was not forwarded, escalated, and reaped")
        unless @kills == @expected_kills
            && !grep { $kills[$_][0] != $expected_kills[$_][0]
                || $kills[$_][1] != $expected_kills[$_][1] } 0 .. $#kills;
    fail("TERM did not perform the final reap")
        unless @waits >= 4 && $waits[-4][0] == 4242 && $waits[-4][1] == 0;
    my @expected_descendant_reaps = (
        [-1, WNOHANG],
        [-1, WNOHANG],
        [-1, WNOHANG],
    );
    fail("TERM did not drain adopted descendants")
        unless @waits >= 4
            && !grep {
                $waits[-3 + $_][0] != $expected_descendant_reaps[$_][0]
                    || $waits[-3 + $_][1] != $expected_descendant_reaps[$_][1]
            } 0 .. 2;
    fail("interrupted evidence was not published") unless -f $manifest;
    fail("interrupted fragment directory was not removed")
        if -d $fragment_dir;
} else {
    fail("unknown harness scenario");
}

print "status=$status path_calls=$path_calls fork_calls=$fork_calls\n";
"""


def make_target_block(source: str, target: str) -> str:
    """Return one Make target and its recipes, without executing Make."""
    match = re.search(
        rf"(?m)^{re.escape(target)}\s*:[^\n]*\n",
        source,
    )
    if match is None:
        raise AssertionError(f"Make target {target!r} is not defined")
    remainder = source[match.end() :]
    next_target = re.search(r"(?m)^[A-Za-z0-9_.-]+\s*:", remainder)
    end = match.end() + (next_target.start() if next_target else len(remainder))
    return source[match.start() : end]


def manifest_source() -> str:
    return "\n".join(
        (
            MAKEFILE.read_text(encoding="utf-8"),
            RUST_DRIVER.read_text(encoding="utf-8"),
        )
    )


def source_near(source: str, needle: str, radius: int = 1200) -> str:
    index = source.find(needle)
    if index < 0:
        raise AssertionError(f"source does not contain {needle!r}")
    return source[max(0, index - radius) : index + radius]


def load_layer1_jobs() -> types.ModuleType:
    spec = importlib.util.spec_from_file_location("d2b_layer1_jobs", LAYER1_JOBS)
    if spec is None or spec.loader is None:
        raise RuntimeError("cannot load the Layer-1 runner")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class CiRunnerRegressionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        SCRATCH.mkdir(exist_ok=True)

    def setUp(self) -> None:
        self.scratch = pathlib.Path(
            tempfile.mkdtemp(prefix="ci-runner-regression.", dir=SCRATCH)
        )

    def tearDown(self) -> None:
        shutil.rmtree(self.scratch)

    def run_execution_manifest_harness(
        self,
        scenario: str,
        manifest: pathlib.Path,
    ) -> subprocess.CompletedProcess[str]:
        perl = shutil.which("perl")
        self.assertIsNotNone(perl, "Perl is required for execution-manifest coverage")
        assert perl is not None
        env = os.environ.copy()
        env.pop("PERL5LIB", None)
        env.pop("PERL5OPT", None)
        return subprocess.run(
            [
                perl,
                "-e",
                EXECUTION_MANIFEST_HARNESS,
                str(EXECUTION_MANIFEST_HELPER),
                scenario,
                str(manifest),
            ],
            cwd=ROOT,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )

    def test_shell_bootstrap_rejects_bash_startup_poison(self) -> None:
        layer1_jobs = load_layer1_jobs()
        bootstrap = shlex.split(layer1_jobs.SCRUBBED_BASH)
        self.assertEqual(bootstrap[-1], "{0}")

        poison_marker = self.scratch / "bash-env-reached-bootstrap"
        step_marker = self.scratch / "step-ran"
        bash_env = self.scratch / "bash-env"
        step = self.scratch / "step.sh"
        bash_env.write_text(
            ': > "${D2B_POISON_MARKER:?}"\n',
            encoding="utf-8",
        )
        step.write_text(
            """#!/usr/bin/env bash
set -euo pipefail
[ -z "${BASH_ENV+x}" ]
! declare -F poisoned_helper >/dev/null
! declare -F dirname >/dev/null
: > "${D2B_STEP_MARKER:?}"
""",
            encoding="utf-8",
        )

        env = os.environ.copy()
        env.update(
            {
                "BASH_ENV": str(bash_env),
                "BASH_FUNC_poisoned_helper%%": "() { return 41; }",
                "BASH_FUNC_dirname%%": (
                    "() { printf '%s\\n' '/corrupted-bootstrap'; }"
                ),
                "D2B_POISON_MARKER": str(poison_marker),
                "D2B_STEP_MARKER": str(step_marker),
            }
        )
        command = [*bootstrap[:-1], str(step)]
        proc = subprocess.run(
            command,
            cwd=ROOT,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )

        failures = []
        if poison_marker.exists():
            failures.append("BASH_ENV was sourced before the environment scrub")
        if proc.returncode != 0:
            failures.append(
                f"scrubbed shell exited {proc.returncode}; stderr:\n{proc.stderr}"
            )
        if not step_marker.exists():
            failures.append("the protected step did not run")
        self.assertEqual(
            failures,
            [],
            msg="\n".join(failures),
        )

    def test_extra_target_failure_is_identified_and_redacted(self) -> None:
        layer1_jobs = load_layer1_jobs()
        log_dir = self.scratch / "retained-log"
        calls: list[str] = []

        def make_log_dir(*_args: object, **_kwargs: object) -> str:
            log_dir.mkdir()
            return str(log_dir)

        def run_make(
            argv: list[str],
            **kwargs: object,
        ) -> types.SimpleNamespace:
            target = argv[1]
            calls.append(target)
            output = kwargs["stdout"]
            if target == "check-tier0":
                return types.SimpleNamespace(returncode=0)
            output.write(
                (
                    f"repo failure: {ROOT}/private/output\n"
                    f"home failure: {pathlib.Path.home()}/private/output\n"
                    "system failure: /nix/store/private-output\n"
                ).encode()
            )
            return types.SimpleNamespace(returncode=37)

        job = {
            "displayName": "Tier 0 first-pass gate",
            "makeTarget": "check-tier0",
            "extraMakeTargets": [
                {
                    "makeTarget": "test-ci-coverage",
                    "displayName": "CI coverage structural guard",
                }
            ],
        }
        stdout = io.StringIO()
        stderr = io.StringIO()
        with (
            mock.patch.object(layer1_jobs.tempfile, "mkdtemp", make_log_dir),
            mock.patch.object(layer1_jobs.subprocess, "run", run_make),
            contextlib.redirect_stdout(stdout),
            contextlib.redirect_stderr(stderr),
        ):
            returncode = layer1_jobs.run_job("tier0", job)

        diagnostic = stderr.getvalue()
        self.assertEqual(returncode, 37)
        self.assertEqual(calls, ["check-tier0", "test-ci-coverage"])
        self.assertIn("FAIL: test-ci-coverage", diagnostic)
        self.assertIn("Layer-1 job tier0", diagnostic)
        self.assertIn("full retained log: <repo>/", diagnostic)
        self.assertNotIn(str(log_dir), diagnostic)
        self.assertNotIn(str(ROOT), diagnostic)
        self.assertNotIn(str(pathlib.Path.home()), diagnostic)
        self.assertNotIn("/nix/store", diagnostic)
        self.assertIn("<repo>/private/output", diagnostic)
        self.assertIn("<home>/private/output", diagnostic)
        self.assertIn("<path>", diagnostic)

    def test_workflow_keeps_advisory_jobs_required_but_non_enforcing(self) -> None:
        layer1_jobs = load_layer1_jobs()
        manifest = layer1_jobs.load_manifest()
        workflow = layer1_jobs.render_workflow(manifest)

        self.assertIn(
            'name: "Advisory - non-enforcing - Performance budget gate"',
            workflow,
        )
        self.assertIn(
            "require_advisory_success test-performance-budgets "
            "'${{ needs.test-performance-budgets.result }}'",
            workflow,
        )
        self.assertNotIn(
            "require_success test-performance-budgets "
            "'${{ needs.test-performance-budgets.result }}'",
            workflow,
        )
        self.assertIn("All generated enforcing Layer-1 jobs passed.", workflow)
        self.assertIn(
            "Required advisory jobs completed (not enforcing passes): "
            "test-performance-budgets",
            workflow,
        )

    def test_manifest_rejects_expression_bearing_ci_job_ids(self) -> None:
        layer1_jobs = load_layer1_jobs()
        with self.assertRaises(SystemExit):
            layer1_jobs.validate_job_id("bad-${{ github.token }}", "test")

    def test_rust_gate_is_three_required_shards_with_one_stable_rollup(self) -> None:
        layer1_jobs = load_layer1_jobs()
        manifest = layer1_jobs.load_manifest()
        workflow = layer1_jobs.render_workflow(manifest)

        rust_rollup = manifest["jobs"]["test-rust"]
        rust_shards = [
            "test-rust-api-surface",
            "test-rust-main",
            "test-rust-broker",
            "test-rust-guest-shell-runner",
            "test-rust-no-bash-ast",
            "test-rust-schema",
            "test-rust-inventory",
            "test-rust-supply-chain",
        ]
        self.assertEqual(
            rust_rollup["needs"],
            rust_shards,
        )
        self.assertEqual(rust_rollup["ciKind"], "rust-rollup")
        for shard in rust_shards:
            self.assertIn(f"run: make {shard}", workflow)
            self.assertIn(f"{shard}=$result", workflow)
        self.assertNotIn("  test-rust-remaining:", workflow)
        self.assertEqual(workflow.count('[ "$result" = success ] || failed=1'), 8)
        self.assertIn('[ "$failed" -eq 0 ] || exit 1', workflow)
        self.assertIn('echo "All Rust gate shards passed."', workflow)
        main_job = workflow.split("  test-rust-main:", 1)[1].split(
            "\n  test-rust-broker:",
            1,
        )[0]
        self.assertIn("Prune warm-local-only Rust cache trees", main_job)
        self.assertIn("public-census", main_job)
        self.assertIn("private-census", main_job)
        self.assertEqual(workflow.count("Prune warm-local-only Rust cache trees"), 1)
        self.assertEqual(manifest["ci"]["rollupNeeds"].count("test-rust"), 1)

    def test_expensive_rust_cache_surface_is_present(self) -> None:
        workflow = load_layer1_jobs().render_workflow(load_layer1_jobs().load_manifest())
        self.assertIn(".scratch/rust-test-cache", workflow)
        self.assertIn('prefix-key: "v2-rust-api-json"', workflow)

    def test_fixture_lane_owns_the_only_bounded_nix_store_cache(self) -> None:
        workflow = load_layer1_jobs().render_workflow(load_layer1_jobs().load_manifest())
        fixture_job = workflow.split("  test-fixture-contracts:", 1)[1].split("\n  test-proofs:", 1)[0]
        nix_unit_job = workflow.split("  nix-unit-shards:", 1)[1].split("\n  test-nix-unit:", 1)[0]
        self.assertIn("Nix store cache", fixture_job)
        self.assertIn("gc-max-store-size-linux: 4G", fixture_job)
        self.assertNotIn("Nix store cache", nix_unit_job)
        self.assertNotIn("nix-store --import", fixture_job)

    def test_nix_unit_ci_uses_one_runner_per_discovered_shard(self) -> None:
        layer1_jobs = load_layer1_jobs()
        manifest = layer1_jobs.load_manifest()
        workflow = layer1_jobs.render_workflow(manifest)

        self.assertEqual(
            manifest["jobs"]["test-nix-unit"]["needs"],
            ["nix-unit-discover", "nix-unit-shards"],
        )
        self.assertIn("matrix:", workflow)
        self.assertIn(
            "check: ${{ fromJSON(needs.nix-unit-discover.outputs.checks) }}",
            workflow,
        )
        self.assertIn("D2B_NIX_UNIT_CHECK: ${{ matrix.check }}", workflow)
        self.assertIn(
            "checks: ${{ steps.list.outputs.nixunitchecks }}",
            workflow,
        )
        self.assertEqual(manifest["jobs"]["nix-unit-shards"]["maxParallel"], 4)
        self.assertIn("      max-parallel: 4", workflow)
        self.assertIn('D2B_NIX_UNIT_JOBS: "1"', workflow)
        self.assertIn("Every discovered Nix-unit shard passed.", workflow)
        self.assertNotIn("run: make test-nix-unit\n\n  flake-eval-discover", workflow)

    def test_nix_unit_driver_is_bounded_and_waits_every_discovered_check(self) -> None:
        driver = (ROOT / "tests" / "test-nix-unit.sh").read_text(encoding="utf-8")

        self.assertIn('jobs=${D2B_NIX_UNIT_JOBS:-2}', driver)
        self.assertIn('1|2|3|4) ;;', driver)
        self.assertIn('D2B_NIX_UNIT_JOBS must be an integer from 1 through 4', driver)
        self.assertNotIn('[ "$jobs" -gt 4 ]', driver)
        self.assertIn('D2B_NIX_UNIT_CHECK', driver)
        self.assertIn('for check in "${checks[@]}"; do', driver)
        self.assertIn('while [ "$running" -ge "$jobs" ]; do', driver)
        self.assertIn('while [ "$running" -gt 0 ]; do', driver)
        self.assertIn('failures+=("$check")', driver)
        self.assertIn('nix build --no-link --print-out-paths', driver)

    def test_fixture_driver_realizes_minimal_and_excludes_real_binary_probe(self) -> None:
        driver = (ROOT / "tests" / "test-rust.sh").read_text(encoding="utf-8")
        fixture_driver = (ROOT / "tests" / "tools" / "eval-fixtures.sh").read_text(
            encoding="utf-8"
        )

        self.assertIn('bash "$ROOT/tests/tools/eval-fixtures.sh"', driver)
        self.assertIn("#checks.${system}.fixture-smoke", driver)
        self.assertIn("not binary(video_binary_contract)", driver)
        flake = (ROOT / "flake.nix").read_text(encoding="utf-8")
        self.assertIn("video-binary-contract =", flake)
        self.assertIn(
            'D2B_FLAKE_REALIZED_CHECKS="video-binary-contract"',
            (ROOT / "tests" / "tools" / "flake-check-classes.sh").read_text(encoding="utf-8"),
        )
        self.assertIn(
            'd2b_flake_check_is_realized "$D2B_FLAKE_CHECK"',
            (ROOT / "tests" / "test-flake.sh").read_text(encoding="utf-8"),
        )
        self.assertIn('nix build --no-link --print-out-paths', (ROOT / "tests" / "test-flake.sh").read_text(encoding="utf-8"))
        self.assertNotIn("checks.${contract_system}.fixture-smoke", driver)
        self.assertIn("nix eval", fixture_driver)
        self.assertNotIn("nix build", fixture_driver)

    def test_realized_flake_check_gets_its_own_unblocked_lane(self) -> None:
        manifest = load_layer1_jobs().load_manifest()
        workflow = load_layer1_jobs().render_workflow(manifest)

        # The realized lane must not share the bounded eval matrix: dispatched
        # there it queues behind two dozen sub-minute shards and sets the whole
        # run's critical path.
        self.assertEqual(
            manifest["jobs"]["flake-eval-x86-realized"]["needs"],
            ["flake-eval-discover"],
        )
        self.assertIn(
            "check: ${{ fromJSON(needs.flake-eval-discover.outputs.realizedchecks) }}",
            workflow,
        )
        self.assertIn(
            "check: ${{ fromJSON(needs.flake-eval-discover.outputs.evalchecks) }}",
            workflow,
        )
        # Both lanes stay required through the stable rollup context.
        self.assertIn(
            "flake-eval-x86-realized",
            manifest["jobs"]["test-flake-x86"]["needs"],
        )
        self.assertIn("realized='${{ needs.flake-eval-x86-realized.result }}'", workflow)
        self.assertIn('[ "$realized" = success ]', workflow)

    def test_flake_check_partition_is_total_and_single_sourced(self) -> None:
        partition = (
            ROOT / "tests" / "tools" / "flake-check-partition.sh"
        ).read_text(encoding="utf-8")
        classes = (
            ROOT / "tests" / "tools" / "flake-check-classes.sh"
        ).read_text(encoding="utf-8")
        workflow = load_layer1_jobs().render_workflow(load_layer1_jobs().load_manifest())

        # One classifier, consumed by the dispatcher and by the driver that
        # decides build-versus-instantiate, so a shard cannot be routed to the
        # realized lane and then merely evaluated there.
        self.assertIn("flake-check-classes.sh", partition)
        self.assertIn("d2b_flake_check_is_realized", classes)
        self.assertIn("d2b_flake_check_is_nix_unit", classes)

        # Fail closed rather than emitting empty matrices, which GitHub would
        # report as a vacuously green flake gate.
        self.assertIn("enumerated zero checks", partition)
        self.assertIn("is not a discovered flake check", partition)
        self.assertIn("partitioned $partitioned of $total checks", partition)

        # Each element is validated whole, so a name carrying the separator
        # splits into pieces that abort rather than dropping out of every lane.
        self.assertIn("enumeration is not a JSON array", partition)
        self.assertIn("enumeration element is not a quoted name", partition)
        self.assertIn("'\"'?*'\"') ;;", partition)

        # The rejected element is named so a contributor can find it, but it is
        # PR-controlled and goes to a public log, so it is rendered through the
        # sanitiser rather than interpolated raw. Pin both the mitigation and
        # its use: a future edit that drops either reintroduces log injection.
        self.assertIn(
            "render_rejected() {\n  printf '%s' \"${1:0:64}\" | tr -c 'A-Za-z0-9._-' '?'\n}",
            partition,
        )
        self.assertEqual(partition.count('$(render_rejected "'), 2)
        for raw in ('is not a quoted name: $token', '[A-Za-z0-9._-]: $name'):
            self.assertNotIn(raw, partition)

        # Both discovery jobs read the same partition, so the names dropped
        # from the eval matrix are exactly the names the Nix-unit lane runs.
        self.assertEqual(workflow.count("partition=$(make -s test-flake-partition)"), 2)

    def test_disk_reclaim_is_conditional_but_still_fails_safe(self) -> None:
        workflow = load_layer1_jobs().render_workflow(load_layer1_jobs().load_manifest())

        self.assertIn("threshold_kib=$((70 * 1024 * 1024))", workflow)
        self.assertIn('if [ "$avail_kib" -ge "$threshold_kib" ]; then', workflow)
        # The reclaim itself must survive: a fuller runner image still pays it.
        self.assertIn(
            "sudo rm -rf /usr/local/lib/android /usr/share/dotnet /opt/ghc "
            "/usr/local/.ghcup /opt/hostedtoolcache/CodeQL || true",
            workflow,
        )
        self.assertIn("docker system prune -af || true", workflow)

    def test_api_surface_json_gate_is_enforcing_and_cacheable(self) -> None:
        driver = (ROOT / "tests" / "test-rust.sh").read_text(encoding="utf-8")
        api_driver = (ROOT / "tests" / "tools" / "api-surface-json.sh").read_text(
            encoding="utf-8"
        )
        workflow = load_layer1_jobs().render_workflow(load_layer1_jobs().load_manifest())

        self.assertIn('bash "$ROOT/tests/tools/api-surface-json.sh"', driver)
        self.assertNotIn("D2B_SKIP_API_SURFACE", driver)
        self.assertIn('export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-D warnings"', driver)
        self.assertIn('export RUSTDOCFLAGS="${RUSTDOCFLAGS:+$RUSTDOCFLAGS }-D warnings"', driver)
        self.assertIn("nightly-2026-02-16", api_driver)
        self.assertEqual(api_driver.count('RUSTDOCFLAGS="-D warnings '), 2)
        self.assertIn("--document-hidden-items", api_driver)
        self.assertIn("--document-private-items", api_driver)
        self.assertIn("--workspace --lib --no-deps", api_driver)
        self.assertIn(".scratch/rust-test-cache/api-surface-", api_driver)
        self.assertIn('D2B_API_SURFACE_TARGET_DIR must be an absolute path', api_driver)
        self.assertIn('D2B_API_SURFACE_UPDATE must be 0 or 1', api_driver)
        makefile = (ROOT / "Makefile").read_text(encoding="utf-8")
        self.assertIn("api-surface-pin:", makefile)
        self.assertIn("D2B_API_SURFACE_UPDATE=1 bash tests/tools/api-surface-json.sh", makefile)
        self.assertIn('prefix-key: "v2-rust-api-json"', workflow)

    def test_rust_aggregate_is_a_make_owned_keep_going_dag(self) -> None:
        makefile = MAKEFILE.read_text(encoding="utf-8")
        aggregate = make_target_block(makefile, "test-rust")

        self.assertNotIn("bash tests/test-rust.sh", aggregate)
        self.assertIn("$(MAKE)", aggregate)
        self.assertIn("--keep-going", aggregate)
        self.assertIn("--output-sync=target", aggregate)
        for leaf in (
            "test-rust-leaf-api-surface",
            "test-rust-leaf-main-workspace",
            "test-rust-leaf-schema",
            "test-rust-leaf-inventory",
            "test-rust-leaf-fixture-contracts",
            "test-rust-leaf-broker",
            "test-rust-leaf-guest-shell-runner",
            "test-rust-leaf-no-bash-ast",
            "test-rust-leaf-supply-chain",
        ):
            self.assertRegex(
                makefile,
                rf"(?m)^{re.escape(leaf)}\s*:",
                msg=f"Rust DAG leaf {leaf} is not Make-owned",
            )

    def test_rust_manifest_preserves_baseline_subsurfaces(self) -> None:
        driver = RUST_DRIVER.read_text(encoding="utf-8")
        baseline_leaves = (
            "rust-api-surface",
            "rust-main-format",
            "rust-main-clippy",
            "rust-main-workspace-tests",
            "rust-contract-tests",
            "rust-cli-contract-tests",
            "rust-no-bash-ast",
            "rust-broker-default",
            "rust-broker-layer1",
            "rust-broker-fakebackends",
            "rust-guest-shell-runner",
            "rust-schema-reproducibility",
            "rust-deny-main",
            "rust-deny-broker",
            "rust-deny-guest",
            "rust-audit-main",
            "rust-audit-broker",
            "rust-audit-guest",
            "rust-stub-no-socket",
            "rust-assert-pinned",
        )
        for leaf in baseline_leaves:
            self.assertIn(leaf, driver, msg=f"missing Rust baseline leaf {leaf}")
        self.assertNotIn(
            '--leaf "$rust_mode"',
            driver,
            "manifest fragments must identify completed sub-surfaces, not leaf modes",
        )
        self.assertEqual(driver.count("rust_surface_success rust-contract-tests"), 1)
        self.assertEqual(driver.count("rust_surface_success rust-cli-contract-tests"), 1)

    def test_rust_fixture_surfaces_are_conditional_ordered_and_not_duplicated(self) -> None:
        makefile = MAKEFILE.read_text(encoding="utf-8")
        static = (ROOT / "tests" / "static.sh").read_text(encoding="utf-8")
        aggregate = make_target_block(makefile, "test-rust")
        focused = make_target_block(makefile, "test-rust-main")

        self.assertIn("test-rust-leaf-fixture-contracts", aggregate)
        self.assertNotIn(
            "test-rust-leaf-fixture-contracts: test-rust-leaf-main-workspace",
            makefile,
        )
        self.assertIn('if [ "$(D2B_SKIP_FIXTURE_BUILD)" = 1 ]', makefile)
        self.assertIn("elif command -v nix", makefile)
        self.assertIn("D2B_RUST_MAIN_LEAVES", focused)
        self.assertIn("D2B_SKIP_FIXTURE_BUILD=1 make test-rust", static)

        driver = RUST_DRIVER.read_text(encoding="utf-8")
        self.assertIn('fixture_target_dir="$workspace_target_dir"', driver)
        self.assertIn(
            'fixture_target_dir="$ROOT/.scratch/rust-test-cache/fixture-contracts"',
            driver,
        )
        self.assertIn('${D2B_RUST_COLD_PROFILE:-0}', driver)
        self.assertGreaterEqual(driver.count('CARGO_TARGET_DIR="$fixture_target_dir"'), 4)
        self.assertEqual(driver.count("rust_surface_success rust-contract-tests"), 1)
        self.assertEqual(driver.count("rust_surface_success rust-cli-contract-tests"), 1)
        self.assertEqual(driver.count("run_fixture_contract_tests\n"), 1)
        self.assertEqual(driver.count("run_cli_contract_tests \"$contract_fixtures\""), 1)

        api_driver = (ROOT / "tests" / "tools" / "api-surface-json.sh").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            'd2b_mktemp ".scratch/.d2b-api-surface.XXXXXX"',
            api_driver,
        )
        self.assertLess(
            api_driver.index('mkdir -p "$ROOT/.scratch"'),
            api_driver.index('d2b_mktemp ".scratch/.d2b-api-surface.XXXXXX"'),
        )
        self.assertIn('public_target="$target_root/public-census"', api_driver)
        self.assertIn('private_target="$target_root/private-census"', api_driver)
        self.assertIn('public_target="$target_root/census"', api_driver)
        self.assertIn('private_target="$target_root/census"', api_driver)
        self.assertIn('rm -rf "$target_root/census"', api_driver)
        self.assertNotIn('${D2B_RUST_COLD_PROFILE:-0}', api_driver)
        self.assertIn('if [ "$shared_census" = 1 ]', api_driver)
        self.assertIn('checker_target="$target_root/checker"', api_driver)
        self.assertIn(
            'CARGO_TARGET_DIR="$checker_target" cargo run --quiet --release --locked',
            api_driver,
        )
        self.assertIn('CARGO_BUILD_JOBS="$public_jobs"', api_driver)
        self.assertIn('CARGO_BUILD_JOBS="$private_jobs"', api_driver)
        self.assertIn("run_public_census &", api_driver)
        self.assertIn("run_private_census &", api_driver)
        self.assertIn('if [ "$api_jobs" -ge 2 ]', api_driver)

    def test_api_surface_scratch_creation_works_without_existing_parent(self) -> None:
        with tempfile.TemporaryDirectory(prefix="api-scratch-parent.") as raw_dir:
            root = pathlib.Path(raw_dir) / "repo"
            root.mkdir()
            self.assertFalse((root / ".scratch").exists())
            script = r"""
set -euo pipefail
ROOT="$1"
export ROOT
. "$2"
mkdir -p "$ROOT/.scratch"
scratch=$(d2b_mktemp ".scratch/.d2b-api-surface.XXXXXX")
test -d "$scratch"
case "$scratch" in
  "$ROOT/.scratch/"*) ;;
  *) exit 90 ;;
esac
"""
            result = subprocess.run(
                ["bash", "-c", script, "bash", str(root), str(ROOT / "tests/lib.sh")],
                cwd=ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                check=False,
            )
            self.assertEqual(
                result.returncode,
                0,
                msg=f"scratch allocation failed\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}",
            )

    def test_rust_fixture_leaf_sets_internal_opt_in_but_public_target_stays_closed(self) -> None:
        makefile = MAKEFILE.read_text(encoding="utf-8")
        fixture_leaf = make_target_block(makefile, "test-rust-leaf-fixture-contracts")
        public_target = make_target_block(makefile, "test-fixture-contracts")

        self.assertIn(
            "D2B_ENABLE_FIXTURE_BUILD=1",
            fixture_leaf,
            "the internal fixture leaf must explicitly opt into fixture materialisation",
        )
        self.assertIn(
            'if [ "$(D2B_SKIP_FIXTURE_BUILD)" = 1 ]',
            fixture_leaf,
            "the internal fixture leaf must preserve the skip path",
        )
        self.assertNotIn(
            "D2B_ENABLE_FIXTURE_BUILD=1 bash tests/test-rust.sh fixture-contracts",
            public_target,
            "the standalone fixture target must remain fail-closed",
        )

    def test_rust_inventory_precedes_broker_without_serializing_schema_and_main(self) -> None:
        makefile = MAKEFILE.read_text(encoding="utf-8")
        self.assertIn(
            "D2B_RUST_BROKER_PREREQS_aggregate := test-rust-leaf-inventory",
            makefile,
            "broker must wait for assert-pinned-tests lockfile enumeration",
        )
        self.assertIn(
            "D2B_RUST_SCHEMA_PREREQS_aggregate := test-rust-leaf-inventory",
            makefile,
        )
        self.assertIn(
            "D2B_RUST_MAIN_PREREQS_aggregate := test-rust-leaf-schema",
            makefile,
        )
        self.assertIn("D2B_RUST_MAIN_PREREQS_main :=", makefile)
        self.assertIn(
            "test-rust-leaf-main-workspace: $(D2B_RUST_MAIN_PREREQS)",
            makefile,
        )
        self.assertIn("D2B_RUST_BROKER_PREREQS_broker :=", makefile)
        self.assertIn("D2B_RUST_SCHEMA_PREREQS_schema :=", makefile)
        self.assertIn("D2B_RUST_MAIN_PREREQS_cold :=", makefile)
        self.assertIn("D2B_RUST_BROKER_PREREQS_cold :=", makefile)
        self.assertIn(
            "D2B_RUST_SCHEMA_PREREQS_cold := test-rust-leaf-inventory",
            makefile,
        )
        self.assertIn(
            "D2B_RUST_INVENTORY_PREREQS_cold := test-rust-leaf-fixture-contracts",
            makefile,
        )
        self.assertIn(
            "test-rust-leaf-fixture-contracts: $(D2B_RUST_FIXTURE_PREREQS)",
            makefile,
        )
        self.assertIn(
            "test-rust-leaf-inventory: $(D2B_RUST_INVENTORY_PREREQS)",
            makefile,
        )
        self.assertIn(
            "test-rust-leaf-broker: $(D2B_RUST_BROKER_PREREQS)",
            makefile,
        )
        self.assertIn(
            "test-rust-leaf-schema: $(D2B_RUST_SCHEMA_PREREQS)",
            makefile,
        )
        self.assertNotIn(
            "test-rust-leaf-broker: test-rust-leaf-schema",
            makefile,
            "broker should be allowed to overlap schema after inventory",
        )
        self.assertNotIn(
            "test-rust-leaf-broker: test-rust-leaf-main-workspace",
            makefile,
            "broker should be allowed to overlap main after schema",
        )

    def test_rust_ci_profiles_use_full_runner_budgets_without_duplicate_leaves(self) -> None:
        cases = (
            (
                "test-rust-api-surface",
                {"D2B_RUST_BUDGET": "4"},
                (
                    "1 active lane(s), api profile",
                    "bash tests/test-rust.sh api-surface",
                ),
                (),
            ),
            (
                "test-rust-main",
                {
                    "D2B_RUST_BUDGET": "4",
                    "D2B_SKIP_FIXTURE_BUILD": "1",
                },
                (
                    "1 active lane(s), main profile",
                    "bash tests/test-rust.sh main-workspace",
                ),
                (
                    "bash tests/test-rust.sh schema-reproducibility",
                    "bash tests/test-rust.sh inventory-stub",
                    "bash tests/test-rust.sh fixture-contracts",
                ),
            ),
            (
                "test-rust-broker",
                {"D2B_RUST_BUDGET": "4"},
                (
                    "1 active lane(s), broker profile",
                    "bash tests/test-rust.sh broker",
                ),
                ("bash tests/test-rust.sh inventory-stub",),
            ),
            (
                "test-rust-guest-shell-runner",
                {"D2B_RUST_BUDGET": "4"},
                (
                    "1 active lane(s), guest profile",
                    "bash tests/test-rust.sh guest-shell-runner",
                ),
                (),
            ),
            (
                "test-rust-no-bash-ast",
                {"D2B_RUST_BUDGET": "4"},
                (
                    "1 active lane(s), no-bash profile",
                    "bash tests/test-rust.sh no-bash-ast",
                ),
                (),
            ),
            (
                "test-rust-schema",
                {"D2B_RUST_BUDGET": "4"},
                (
                    "1 active lane(s), schema profile",
                    "bash tests/test-rust.sh schema-reproducibility",
                ),
                ("bash tests/test-rust.sh inventory-stub",),
            ),
            (
                "test-rust-inventory",
                {"D2B_RUST_BUDGET": "4"},
                (
                    "1 active lane(s), inventory profile",
                    "bash tests/test-rust.sh inventory-stub",
                ),
                (),
            ),
            (
                "test-rust-supply-chain",
                {"D2B_RUST_BUDGET": "4"},
                (
                    "1 active lane(s), supply profile",
                    "bash tests/test-rust.sh supply-chain",
                ),
                (),
            ),
        )
        for target, overrides, required, forbidden in cases:
            env = os.environ.copy()
            env.update(overrides)
            result = subprocess.run(
                ["make", "-n", target],
                cwd=ROOT,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                check=False,
            )
            self.assertEqual(
                result.returncode,
                0,
                msg=f"{target} dry run failed\n{result.stdout}\n{result.stderr}",
            )
            output = result.stdout + result.stderr
            for marker in required:
                self.assertIn(marker, output, msg=f"{target} missing {marker}")
            budget_match = re.search(
                r"Rust effective runtime budget: ([0-9]+) job",
                output,
            )
            self.assertIsNotNone(
                budget_match,
                msg=f"{target} did not report its effective budget",
            )
            assert budget_match is not None
            effective_budget = budget_match.group(1)
            self.assertGreaterEqual(int(effective_budget), 1)
            self.assertLessEqual(
                int(effective_budget),
                int(overrides["D2B_RUST_BUDGET"]),
                msg=f"{target} exceeded the requested Rust budget",
            )
            self.assertIn(
                f'D2B_RUST_CARGO_JOBS="{effective_budget}"',
                output,
                msg=f"{target} did not pass the effective Cargo budget",
            )
            self.assertIn(
                f'D2B_RUST_NEXTEST_THREADS="{effective_budget}"',
                output,
                msg=f"{target} did not pass the effective nextest budget",
            )
            for marker in forbidden:
                self.assertNotIn(marker, output, msg=f"{target} duplicated {marker}")

    def test_rust_cold_profile_restores_shared_target_bounded_execution(self) -> None:
        makefile = MAKEFILE.read_text(encoding="utf-8")
        driver = RUST_DRIVER.read_text(encoding="utf-8")
        api_driver = (ROOT / "tests/tools/api-surface-json.sh").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            'if [ "$$profile" = aggregate ] && [ ! -d packages/target ]',
            makefile,
        )
        cold_block = makefile.split("  cold) \\", 1)[1].split("  api) \\", 1)[0]
        self.assertIn('[ "$$active_lanes" -le 4 ] || active_lanes=4;', cold_block)
        self.assertIn("while [ \"$$surplus\" -gt 0 ]", cold_block)
        self.assertIn('quota_fixture="$$runtime_budget";', cold_block)
        self.assertIn('quota_schema="$$runtime_budget";', cold_block)
        self.assertIn('quota_inventory="$$runtime_budget";', cold_block)
        self.assertIn('"D2B_RUST_COLD_PROFILE=$$cold_profile"', makefile)
        cold_order = (
            "test-rust-leaf-api-surface test-rust-leaf-main-workspace "
            "test-rust-leaf-fixture-contracts test-rust-leaf-broker "
            "test-rust-leaf-guest-shell-runner test-rust-leaf-no-bash-ast "
            "test-rust-leaf-schema test-rust-leaf-supply-chain "
            "test-rust-leaf-inventory"
        )
        self.assertIn(cold_order, makefile)
        self.assertIn(
            "D2B_RUST_FIXTURE_PREREQS_cold := "
            "test-rust-leaf-api-surface test-rust-leaf-main-workspace "
            "test-rust-leaf-broker test-rust-leaf-guest-shell-runner "
            "test-rust-leaf-no-bash-ast test-rust-leaf-supply-chain",
            makefile,
        )
        self.assertIn('fixture_target_dir="$workspace_target_dir"', driver)
        self.assertIn('public_target="$target_root/public-census"', api_driver)
        self.assertIn('private_target="$target_root/private-census"', api_driver)
        self.assertIn('rm -rf "$target_root/census"', api_driver)

    def test_rust_cold_frontier_fits_budgets_one_through_twelve(self) -> None:
        for budget in range(1, 13):
            active_lanes = min(budget, 4)
            quotas = {"main": 1, "broker": 1, "api": 1}
            for turn in range(budget - active_lanes):
                quotas[("main", "broker", "api")[turn % 3]] += 1
            if active_lanes < 3:
                frontier = active_lanes
            elif active_lanes == 3:
                frontier = sum(quotas.values())
            else:
                frontier = sum(quotas.values()) + 1
            self.assertLessEqual(frontier, budget)

    def test_schema_leaf_uses_the_exported_cargo_job_budget_for_xtask(self) -> None:
        driver = RUST_DRIVER.read_text(encoding="utf-8")
        self.assertIn('export CARGO_BUILD_JOBS="$D2B_RUST_CARGO_JOBS"', driver)
        self.assertEqual(
            len(
                re.findall(
                    r"(?m)^\(cd [^\n]+ && cargo xtask gen-schemas\)$",
                    driver,
                )
            ),
            2,
        )
        self.assertNotRegex(driver, r"cargo\s+--jobs\s+[^\n]*\sxtask\b")

    def test_harness_free_binaries_receive_no_libtest_arguments(self) -> None:
        driver = RUST_DRIVER.read_text(encoding="utf-8")
        command = next(
            line
            for line in driver.splitlines()
            if 'cargo test --jobs "$D2B_RUST_CARGO_JOBS"' in line
            and '--test "$bin"' in line
        )
        self.assertNotIn("--test-threads", command)
        self.assertFalse(command.rstrip().endswith(" --"))

    def test_rust_exit_cleanup_and_nix_reentry_are_executable_without_duplicate_fragments(self) -> None:
        tree = self.scratch / "rust-reentry-tree"
        for relative in (
            "tests/tools",
            "packages/.cargo",
        ):
            (tree / relative).mkdir(parents=True, exist_ok=True)
        for relative in (
            "tests/test-rust.sh",
            "tests/lib.sh",
            "tests/tools/execution-manifest.pl",
        ):
            destination = tree / relative
            shutil.copy2(ROOT / relative, destination)
        (tree / "tests/test-rust.sh").chmod(0o755)
        (tree / "tests/tools/execution-manifest.pl").chmod(0o755)

        for relative in (
            "packages/Cargo.toml",
            "packages/Cargo.lock",
            "packages/deny.toml",
            "packages/d2b-priv-broker/Cargo.toml",
            "packages/d2b-priv-broker/Cargo.lock",
            "packages/d2b-priv-broker/deny.toml",
            "packages/d2b-guest-shell-runner/Cargo.toml",
            "packages/d2b-guest-shell-runner/Cargo.lock",
            "packages/d2b-guest-shell-runner/deny.toml",
        ):
            path = tree / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.touch()
        (tree / "packages/.cargo/config.toml").write_text(
            "[build]\n",
            encoding="utf-8",
        )
        for relative in (
            "packages/d2b-priv-broker/.cargo/config.toml",
            "packages/d2b-guest-shell-runner/.cargo/config.toml",
        ):
            config = tree / relative
            config.parent.mkdir(parents=True, exist_ok=True)
            config.write_text("[build]\n", encoding="utf-8")
        (tree / "packages/rust-toolchain.toml").write_text(
            '[toolchain]\nchannel = "1.97.0"\n',
            encoding="utf-8",
        )

        api_surface = tree / "tests/tools/api-surface-json.sh"
        api_surface.write_text(
            "#!/usr/bin/env bash\n"
            'exit "${D2B_TEST_RUST_PROBE_STATUS:?}"\n',
            encoding="utf-8",
        )
        api_surface.chmod(0o755)

        outer_bin = self.scratch / "rust-reentry-bin"
        child_bin = self.scratch / "rust-reentry-child-bin"
        outer_bin.mkdir()
        child_bin.mkdir()
        nix = outer_bin / "nix"
        nix.write_text(
            "#!/bin/sh\n"
            'while [ "$#" -gt 0 ]; do\n'
            '  if [ "$1" = "--command" ]; then\n'
            "    shift\n"
            '    export PATH="$D2B_TEST_RUST_CHILD_BIN:$PATH"\n'
            "    unset D2B_CLEANUPS_FILE\n"
            '    exec "$@"\n'
            "  fi\n"
            "  shift\n"
            "done\n"
            "exit 91\n",
            encoding="utf-8",
        )
        nix.chmod(0o755)
        rustup = child_bin / "rustup"
        rustup.write_text(
            "#!/bin/sh\n"
            'if [ "$1" = "toolchain" ]; then exit 0; fi\n'
            'if [ "$1" = "run" ] && [ "$3" = "cargo" ]; then\n'
            '  printf "cargo 1.97.0\\n"\n'
            "  exit 0\n"
            "fi\n"
            'if [ "$1" = "run" ] && [ "$3" = "rustc" ]; then\n'
            '  printf "rustc 1.97.0\\n"\n'
            "  exit 0\n"
            "fi\n"
            "exit 92\n",
            encoding="utf-8",
        )
        rustup.chmod(0o755)

        manifest = self.scratch / "rust-reentry-evidence.json"
        fragment_dir = self.scratch / ".rust-reentry-evidence.json.fragments"
        fragment_dir.mkdir(mode=0o700)
        cleanup_file = self.scratch / "parent-cleanups"
        cleanup_marker = self.scratch / "parent-cleanup-ran"
        cleanup_file.write_text(
            f'printf "%s" cleaned > "{cleanup_marker}"\n',
            encoding="utf-8",
        )

        for probe_status, expected_status in ((0, 0), (37, 37)):
            for fragment in fragment_dir.iterdir():
                fragment.unlink()
            cleanup_marker.unlink(missing_ok=True)
            cleanup_file.write_text(
                f'printf "%s" cleaned > "{cleanup_marker}"\n',
                encoding="utf-8",
            )
            env = os.environ.copy()
            env.update(
                {
                    "ROOT": str(tree),
                    "PATH": f"{outer_bin}:/run/current-system/sw/bin:/usr/bin:/bin",
                    "D2B_EXECUTION_MANIFEST": str(manifest),
                    "D2B_CLEANUPS_FILE": str(cleanup_file),
                    "D2B_TEST_RUST_CHILD_BIN": str(child_bin),
                    "D2B_TEST_RUST_PROBE_STATUS": str(probe_status),
                    "D2B_LOG": str(self.scratch / f"rust-reentry-{probe_status}.log"),
                }
            )
            result = subprocess.run(
                ["bash", str(tree / "tests/test-rust.sh"), "api-surface"],
                cwd=tree,
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                check=False,
            )
            self.assertEqual(
                result.returncode,
                expected_status,
                msg=(
                    f"Rust re-entry probe status mismatch for {probe_status}\n"
                    f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
                ),
            )
            self.assertTrue(
                cleanup_marker.is_file(),
                "the parent EXIT handler did not chain run_cleanups",
            )
            fragments = sorted(fragment_dir.glob("fragment.*"))
            self.assertEqual(
                len(fragments),
                1,
                "nested Rust invocation must be the sole fragment producer",
            )
            evidence = json.loads(fragments[0].read_text(encoding="utf-8"))
            self.assertEqual(evidence["leaf"], "rust-api-surface")
            self.assertEqual(
                evidence["run_status"],
                "passed" if probe_status == 0 else "failed",
            )

    def test_rust_exit_handler_disables_recursion_before_chaining_cleanup(self) -> None:
        driver = RUST_DRIVER.read_text(encoding="utf-8")
        handler = driver.split("rust_leaf_exit() {", 1)[1].split("trap rust_leaf_exit EXIT", 1)[0]
        self.assertIn("trap - EXIT", handler)
        self.assertIn("run_cleanups || true", handler)
        self.assertLess(handler.index("local rc=$?"), handler.index("trap - EXIT"))
        self.assertIn("exit \"$rc\"", handler)
        self.assertIn("rust_manifest_exit_publication_enabled", handler)

    def test_rust_manifest_emitter_errors_remain_visible_and_finalization_preserves_status(self) -> None:
        driver = RUST_DRIVER.read_text(encoding="utf-8")
        start = driver.index("publish_manifest_fragment()")
        end = driver.index("rust_surface_start()", start)
        emitter = driver[start:end]
        self.assertNotIn(">/dev/null", emitter)
        self.assertNotIn("|| true", emitter)
        self.assertIn("required execution-manifest fragment publication failed", driver)
        helper = (ROOT / "tests" / "tools" / "execution-manifest.pl").read_text(
            encoding="utf-8"
        )
        self.assertIn("return 74", helper)
        self.assertIn("finalization failed after scheduler success", helper)
        self.assertIn("preserving the scheduler status", helper)

    def test_execution_manifest_internal_errors_are_catchable_and_module_safe(self) -> None:
        helper = EXECUTION_MANIFEST_HELPER.read_text(encoding="utf-8")
        fatal_region = helper.split("sub fatal", 1)[1].split("sub close_handle", 1)[0]
        self.assertIn("die", fatal_region)
        self.assertNotIn("exit", fatal_region)
        self.assertIn("ExecutionManifest::Fatal", helper)
        self.assertIn("unless (caller)", helper)
        self.assertIn("safe_error_message", helper)
        perl = shutil.which("perl")
        self.assertIsNotNone(perl, "Perl is required for execution-manifest coverage")
        assert perl is not None
        with tempfile.TemporaryDirectory(prefix="execution-manifest-diagnostic.") as raw_dir:
            invalid_manifest = pathlib.Path(raw_dir) / ".." / "not-written.json"
            result = subprocess.run(
                [
                    perl,
                    str(EXECUTION_MANIFEST_HELPER),
                    "fragment",
                    "--manifest",
                    str(invalid_manifest),
                    "--leaf",
                    "probe",
                    "--status",
                    "passed",
                ],
                cwd=ROOT,
                env={
                    key: value
                    for key, value in os.environ.items()
                    if key not in {"PERL5LIB", "PERL5OPT"}
                },
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 1)
            self.assertEqual(
                result.stderr,
                "execution-manifest: D2B_EXECUTION_MANIFEST rejects parent traversal\n",
            )
            self.assertNotIn(str(raw_dir), result.stderr)

    def test_execution_manifest_injected_lifecycle_boundaries_are_executable(self) -> None:
        with tempfile.TemporaryDirectory(prefix="execution-manifest-regression.") as raw_dir:
            temp_dir = pathlib.Path(raw_dir)
            scenarios = {
                name: temp_dir / f"{name}.json"
                for name in (
                    "success-finalization",
                    "failure-finalization",
                    "term",
                )
            }
            results = {
                name: self.run_execution_manifest_harness(name, manifest)
                for name, manifest in scenarios.items()
            }

            for name, result in results.items():
                self.assertEqual(
                    result.returncode,
                    0,
                    msg=(
                        f"{name} harness failed with {result.returncode}\n"
                        f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
                    ),
                )
                self.assertNotIn(str(temp_dir), result.stdout)
                self.assertNotIn(str(temp_dir), result.stderr)

            self.assertIn(
                "finalization failed after scheduler success",
                results["success-finalization"].stderr,
            )
            self.assertIn(
                "synthetic publication failure",
                results["success-finalization"].stderr,
            )
            self.assertEqual(results["success-finalization"].stderr.count("\n"), 1)
            self.assertIn(
                "preserving the scheduler status",
                results["failure-finalization"].stderr,
            )
            self.assertIn(
                "synthetic publication failure",
                results["failure-finalization"].stderr,
            )
            self.assertEqual(results["failure-finalization"].stderr.count("\n"), 1)

            interrupted_manifest = scenarios["term"]
            evidence = json.loads(interrupted_manifest.read_text(encoding="utf-8"))
            self.assertEqual(evidence["run_status"], "interrupted")
            self.assertIn("rust-main-workspace", evidence["completed_leaves"])
            self.assertIn("scheduler-interrupted", evidence["failed_surfaces"])
            self.assertEqual(evidence["version"], 1)
            self.assertTrue((temp_dir / "term.json.lock").is_file())
            self.assertFalse((temp_dir / ".term.json.fragments").exists())

    def test_execution_manifest_lock_contention_is_executable_and_path_free(self) -> None:
        perl = shutil.which("perl")
        self.assertIsNotNone(perl, "Perl is required for execution-manifest coverage")
        assert perl is not None
        with tempfile.TemporaryDirectory(prefix="execution-manifest-lock.") as raw_dir:
            temp_dir = pathlib.Path(raw_dir)
            manifest = temp_dir / "evidence.json"
            holder = subprocess.Popen(
                [
                    perl,
                    str(EXECUTION_MANIFEST_HELPER),
                    "run",
                    "--manifest",
                    str(manifest),
                    "--target",
                    "test-rust",
                    "--commit",
                    "deadbeef",
                    "--",
                    perl,
                    "-e",
                    "sleep 30",
                ],
                cwd=ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            try:
                deadline = time.monotonic() + 5
                fragment_dir = temp_dir / ".evidence.json.fragments"
                while time.monotonic() < deadline and not fragment_dir.is_dir():
                    self.assertIsNone(holder.poll(), "lock holder exited before acquiring the lock")
                    time.sleep(0.02)
                self.assertTrue(fragment_dir.is_dir(), "lock holder did not acquire the manifest lock")

                contender = subprocess.run(
                    [
                        perl,
                        str(EXECUTION_MANIFEST_HELPER),
                        "run",
                        "--manifest",
                        str(manifest),
                        "--target",
                        "test-rust",
                        "--commit",
                        "deadbeef",
                        "--",
                        perl,
                        "-e",
                        "exit 0",
                    ],
                    cwd=ROOT,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                    check=False,
                )
                self.assertEqual(contender.returncode, 73)
                self.assertEqual(
                    contender.stderr,
                    "manifest-lock-contended: execution-manifest lock is active; "
                    "wait for the active run to finish and retry.\n",
                )
                self.assertNotIn(str(temp_dir), contender.stderr)
            finally:
                if holder.poll() is None:
                    holder.send_signal(signal.SIGTERM)
                try:
                    holder_stdout, holder_stderr = holder.communicate(timeout=15)
                except subprocess.TimeoutExpired:
                    holder.kill()
                    holder_stdout, holder_stderr = holder.communicate(timeout=5)
                    self.fail(
                        "lock holder did not terminate after SIGTERM\n"
                        f"stdout:\n{holder_stdout}\nstderr:\n{holder_stderr}"
                    )
            self.assertEqual(holder.returncode, 143)

    def test_rust_budget_validation_is_actionable_static_and_redacted(self) -> None:
        source = "\n".join(
            (
                MAKEFILE.read_text(encoding="utf-8"),
                RUST_DRIVER.read_text(encoding="utf-8"),
            )
        )
        self.assertIn("D2B_RUST_BUDGET", source)
        budget_region = source_near(source, "D2B_RUST_BUDGET", radius=2600)
        self.assertRegex(budget_region, r"(?i)positive integer")
        self.assertRegex(budget_region, r"(?m)\b(?:exit|return)\s+2\b")
        self.assertRegex(budget_region, r"(?i)(?:empty|zero|non[- ]digit|invalid)")

        invalid_lines = [
            line
            for line in budget_region.splitlines()
            if re.search(r"(?i)(?:invalid|positive integer|must be)", line)
        ]
        self.assertTrue(invalid_lines, "the invalid-budget branch has no static message")
        for line in invalid_lines:
            self.assertNotRegex(
                line,
                r"\$(?:\{)?(?:D2B_RUST_BUDGET|budget|requested|raw|value)"
                r"(?:\}|[A-Za-z0-9_:-])?",
                msg="invalid-budget diagnostics must not echo untrusted environment text",
            )

    def test_rust_logs_the_effective_budget_and_names_the_target_control(self) -> None:
        source = "\n".join(
            (
                MAKEFILE.read_text(encoding="utf-8"),
                RUST_DRIVER.read_text(encoding="utf-8"),
            )
        )
        logging_lines = [
            line
            for line in source.splitlines()
            if re.search(r"\b(?:log|echo|printf)\b", line)
        ]
        self.assertTrue(
            any(
                re.search(r"(?i)(?:effective|runtime)", line)
                and re.search(r"(?i)budget", line)
                for line in logging_lines
            ),
            "the Rust target must log the effective runtime budget",
        )
        self.assertTrue(
            any(
                "D2B_RUST_BUDGET" in line
                and re.search(r"(?i)(?:target|control|override)", line)
                for line in logging_lines
            ),
            "the budget log must direct contributors to D2B_RUST_BUDGET",
        )

    def test_rust_default_budget_reads_cache_aware_cgroup_limits(self) -> None:
        source = "\n".join(
            (
                MAKEFILE.read_text(encoding="utf-8"),
                RUST_DRIVER.read_text(encoding="utf-8"),
            )
        )
        for marker in (
            "/proc/self/cgroup",
            "/proc/meminfo",
            "MemAvailable",
            "memory.max",
            "memory.high",
            "memory.current",
            "memory.stat",
            "inactive_file",
        ):
            self.assertIn(marker, source)
        self.assertRegex(
            source,
            r"(?is)memory\.current.{0,700}inactive_file|inactive_file.{0,700}memory\.current",
        )
        self.assertRegex(source, r"(?i)(?:smaller|min(?:imum)?|least)")
        self.assertRegex(source, r"(?i)2\s*GiB")
        self.assertRegex(source, r"(?i)3\s*GiB")

    def test_rust_unreadable_cgroup_controller_fails_closed_to_budget_one(self) -> None:
        source = "\n".join(
            (
                MAKEFILE.read_text(encoding="utf-8"),
                RUST_DRIVER.read_text(encoding="utf-8"),
            )
        )
        self.assertRegex(
            source,
            r"(?is)(?:cgroup|controller).{0,400}(?:unreadable|cannot read|visibility).{0,400}(?:budget|worker).{0,100}(?:1|one)",
        )
        self.assertRegex(source, r"(?i)fix controller visibility")
        self.assertRegex(
            source,
            r"(?is)(?:outside|out of).{0,100}(?:the )?constrained environment",
        )

    def test_rust_runtime_frontier_fits_budgets_one_through_twelve(self) -> None:
        makefile = MAKEFILE.read_text(encoding="utf-8")
        self.assertIn("quota_api=$$((runtime_budget - lane_count + 1))", makefile)
        self.assertIn("frontier_quota=$$((quota_api + active_lanes - 1))", makefile)
        api_quotas = {}
        for budget in range(1, 13):
            active_lanes = min(budget, 9)
            api_quota = budget - 8 if budget > 9 else 1
            frontier = api_quota + active_lanes - 1
            self.assertLessEqual(frontier, budget)
            api_quotas[budget] = api_quota
        self.assertEqual(api_quotas[1], 1)
        self.assertEqual(api_quotas[9], 1)
        self.assertEqual(api_quotas[12], 4)

    def test_rust_nextest_quota_naming_is_consistent(self) -> None:
        makefile = MAKEFILE.read_text(encoding="utf-8")
        driver = RUST_DRIVER.read_text(encoding="utf-8")
        self.assertIn("D2B_RUST_NEXTEST_THREADS", makefile)
        self.assertIn("D2B_RUST_NEXTEST_THREADS", driver)
        self.assertNotIn("D2B_RUST_NEXTTEST_THREADS", makefile + driver)
        self.assertNotIn("D2B_RUST_NEXTEST_QUOTA_FLAG", makefile)
        self.assertNotIn("D2B_RUST_CARGO_QUOTA_FLAG", makefile)

    def test_rust_leaf_recipes_are_ordinary_and_drop_make_metadata_immediately(self) -> None:
        makefile = MAKEFILE.read_text(encoding="utf-8")
        for leaf in (
            "test-rust-leaf-api-surface",
            "test-rust-leaf-main-workspace",
            "test-rust-leaf-schema",
            "test-rust-leaf-inventory",
            "test-rust-leaf-fixture-contracts",
            "test-rust-leaf-broker",
            "test-rust-leaf-guest-shell-runner",
            "test-rust-leaf-no-bash-ast",
            "test-rust-leaf-supply-chain",
        ):
            block = make_target_block(makefile, leaf)
            recipes = [
                line
                for line in block.splitlines()
                if line.startswith("\t") and line.strip()
            ]
            self.assertTrue(recipes, f"Rust leaf {leaf} has no recipe")
            for recipe in recipes:
                self.assertFalse(
                    recipe.lstrip().startswith("+"),
                    msg=f"Rust leaf {leaf} must be an ordinary Make recipe",
                )
                self.assertNotIn(
                    "$(MAKE)",
                    recipe,
                    msg=f"Rust leaf {leaf} must not own recursive Make scheduling",
                )
        self.assertRegex(
            makefile,
            r"(?i)ordinary\s+(?:non[- ]submake|leaf)\s+recipe",
        )

        driver = RUST_DRIVER.read_text(encoding="utf-8")
        metadata = re.search(
            r"(?m)^\s*unset\s+MAKEFLAGS\s+MFLAGS\s+MAKELEVEL\s*$",
            driver,
        )
        self.assertIsNotNone(
            metadata,
            "the leaf must immediately remove inherited Make metadata",
        )
        assert metadata is not None
        first_tool_match = re.search(
            r"(?m)^\s*(?:cargo|nix|rustup)\s+",
            driver,
        )
        self.assertIsNotNone(first_tool_match, "Rust leaf setup has no tool command")
        assert first_tool_match is not None
        self.assertLess(
            metadata.start(),
            first_tool_match.start(),
            "MAKEFLAGS/MFLAGS/MAKELEVEL must be removed before leaf setup",
        )
        self.assertNotRegex(
            driver,
            r"(?m)^\s*(?:eval|exec)\b.*(?:jobserver|MAKEFLAGS|MFLAGS)",
        )

    def test_removed_no_argument_all_scheduler_is_rejected_actionably(self) -> None:
        driver = RUST_DRIVER.read_text(encoding="utf-8")

        self.assertNotIn('rust_mode="${1:-all}"', driver)
        self.assertRegex(
            driver,
            r"(?is)(?:no[- ]argument|all scheduler|all mode|removed).{0,400}"
            r"make\s+test-rust",
        )
        self.assertRegex(
            driver,
            r"(?is)(?:no[- ]argument|all scheduler|all mode|removed).{0,500}"
            r"(?:exit|return)\s+2\b",
        )

    def test_top_level_make_removes_prior_evidence_before_dispatch(self) -> None:
        makefile = MAKEFILE.read_text(encoding="utf-8")
        aggregate = make_target_block(makefile, "test-rust")
        self.assertIn("D2B_EXECUTION_MANIFEST", aggregate)
        invalidation = re.search(
            r"(?is)(?:(?:D2B_EXECUTION_MANIFEST|execution[- ]manifest).{0,300}"
            r"(?:remove|unlink|invalidate|rm\s+-f)|"
            r"(?:remove|unlink|invalidate|rm\s+-f).{0,180}"
            r"(?:D2B_EXECUTION_MANIFEST|execution[- ]manifest|manifest))",
            aggregate,
        )
        self.assertIsNotNone(
            invalidation,
            "top-level Make must invalidate requested evidence before dispatch",
        )
        assert invalidation is not None
        dispatch_positions = [
            position
            for position in (
                aggregate.find("$(MAKE)"),
                aggregate.find("bash tests/test-rust.sh"),
            )
            if position >= 0
        ]
        self.assertTrue(dispatch_positions, "Rust dispatch is not visible in Make")
        self.assertLess(
            invalidation.start(),
            min(dispatch_positions),
            "prior success evidence must be removed before any Rust leaf starts",
        )

    def test_manifest_uses_injected_clock_process_and_path_boundaries(self) -> None:
        source = manifest_source()
        region = source_near(source, "D2B_EXECUTION_MANIFEST", radius=5000)

        self.assertRegex(
            region,
            r"(?is)(?:manifest|shutdown).{0,180}"
            r"(?:clock|now).{0,180}(?:inject|boundary|test|hook|fn)",
        )
        self.assertRegex(
            region,
            r"(?is)(?:manifest|shutdown).{0,180}"
            r"(?:process|child).{0,180}(?:inject|boundary|test|hook|fn)",
        )
        self.assertRegex(
            region,
            r"(?is)(?:manifest|cleanup|path).{0,180}"
            r"(?:path|resolver|directory).{0,180}"
            r"(?:inject|boundary|test|hook|fn)",
        )
        self.assertRegex(region, r"(?i)10\s*(?:seconds|s)")
        self.assertNotRegex(
            source,
            r"D2B_(?!TEST_)[A-Z0-9_]*(?:SHUTDOWN|MANIFEST)[A-Z0-9_]*GRACE",
            msg="production shutdown grace must not become a public timing knob",
        )

    def test_manifest_fragments_are_versioned_same_filesystem_and_atomic(self) -> None:
        source = manifest_source()
        for field in (
            "version",
            "run_status",
            "completed_leaves",
            "failed_surfaces",
            "fragment",
        ):
            self.assertRegex(
                source,
                rf"(?i){re.escape(field)}",
                msg=f"execution evidence is missing {field}",
            )
        self.assertRegex(
            source,
            r"(?is)(?:same[- ]filesystem|same parent|adjacent|st_dev)",
        )
        self.assertRegex(
            source,
            r"(?is)(?:mktemp|mkdir|install).{0,260}(?:0700|mode[^0-9]*700)",
        )
        self.assertRegex(source, r"(?i)(?:atomic|rename|\bmv\b)")
        self.assertRegex(
            source,
            r"(?is)(?:fragment|temporary).{0,500}(?:rename|atomic|\bmv\b)",
        )
        self.assertRegex(
            source,
            r"(?is)(?:version|schema).{0,120}(?:1|v1)",
        )

    def test_manifest_anchors_parent_before_noninheritable_ofd_lock(self) -> None:
        source = manifest_source()
        for marker in (
            "O_CLOEXEC",
            "O_NOFOLLOW",
            "F_OFD_SETLK",
            "openat2",
            "RESOLVE_NO_SYMLINKS",
            "RESOLVE_NO_MAGICLINKS",
        ):
            self.assertIn(marker, source)

        parent_positions = [
            position
            for position in (
                source.find("openat2"),
                source.lower().find("manifest parent"),
                source.lower().find("parent_fd"),
                source.lower().find("anchor"),
            )
            if position >= 0
        ]
        lock_positions = [
            position
            for position in (
                source.find(".lock"),
                source.lower().find("lockfile"),
                source.find("F_OFD_SETLK"),
            )
            if position >= 0
        ]
        self.assertTrue(parent_positions, "manifest parent is not visibly anchored")
        self.assertTrue(lock_positions, "persistent manifest lock is not visible")
        self.assertLess(
            min(parent_positions),
            min(lock_positions),
            "the manifest parent must be anchored before relative lock creation",
        )
        self.assertRegex(
            source,
            r"(?is)(?:parent_fd|manifest parent|anchored parent).{0,700}"
            r"(?:openat|lockfile|\.lock).{0,700}F_OFD_SETLK",
        )
        lock_region = source_near(source, "F_OFD_SETLK", radius=1800)
        self.assertRegex(lock_region, r"0600")
        self.assertRegex(lock_region, r"(?i)(?:current uid|effective uid|geteuid|st_uid)")
        self.assertRegex(lock_region, r"(?i)(?:non[- ]blocking|F_OFD_SETLK)")
        self.assertNotIn("F_OFD_SETLKW", source)

    def test_manifest_lock_contention_is_fixed_actionable_and_path_free(self) -> None:
        source = manifest_source()
        self.assertIn("manifest-lock-contended", source)
        region = source_near(source, "manifest-lock-contended", radius=1600)
        for wording in (
            "execution-manifest lock",
            "wait",
            "retry",
        ):
            self.assertIn(wording, region.lower())

        diagnostic_lines = [
            line
            for line in region.splitlines()
            if "manifest-lock-contended" in line
            or "execution-manifest lock" in line.lower()
        ]
        self.assertTrue(diagnostic_lines)
        for line in diagnostic_lines:
            self.assertNotRegex(
                line,
                r"\$(?:\{)?(?:manifest|lock|path|tmp|dir)[A-Za-z0-9_}]?",
                msg="lock contention output must not interpolate a filesystem path",
            )
            self.assertNotRegex(
                line,
                r"/(?:tmp|home|run|nix|var)/",
                msg="lock contention output must not print an absolute path",
            )

    def test_manifest_rejects_unsafe_owner_mode_and_cleans_only_anchored_entries(self) -> None:
        source = manifest_source()
        for marker in (
            "RESOLVE_NO_SYMLINKS",
            "RESOLVE_NO_MAGICLINKS",
            "fstat",
            "st_uid",
            "st_mode",
            "0600",
            "0700",
            "openat",
            "unlinkat",
        ):
            self.assertIn(marker, source)

        safety_region = source_near(source, "RESOLVE_NO_SYMLINKS", radius=2600)
        self.assertRegex(
            safety_region,
            r"(?is)(?:reject|refus|mismatch|invalid).{0,260}"
            r"(?:owner|uid|mode|permission|symlink|magiclink)",
        )
        self.assertRegex(
            safety_region,
            r"(?is)(?:fragment|temporary).{0,500}"
            r"(?:0700|mode[^0-9]*700)",
        )
        self.assertRegex(
            safety_region,
            r"(?is)(?:fragment|temporary).{0,500}"
            r"(?:current uid|effective uid|geteuid|st_uid)",
        )
        cleanup_region = source_near(source, "unlinkat", radius=3000)
        self.assertRegex(
            cleanup_region,
            r"(?is)(?:invalid|unsafe|mismatch|reject).{0,260}continue",
        )
        self.assertNotRegex(
            cleanup_region,
            r"\brm\s+-rf\b",
            msg="stale evidence cleanup must not use path-based recursive removal",
        )

    def test_manifest_failed_and_interrupted_runs_publish_current_partial_evidence(self) -> None:
        source = manifest_source()
        for status in ("passed", "failed", "interrupted"):
            self.assertRegex(
                source,
                rf"(?is)run_status.{{0,100}}(?:[\"'=])?{status}",
                msg=f"manifest has no {status} status",
            )
        for field in ("completed_leaves", "failed_surfaces", "partial", "stale"):
            self.assertIn(field, source)
        self.assertRegex(
            source,
            r"(?is)(?:failed|interrupted).{0,700}"
            r"(?:finaliz|publish|replace).{0,700}"
            r"(?:atomic|manifest)",
        )
        self.assertRegex(
            source,
            r"(?is)(?:current|run[- ]specific).{0,260}"
            r"(?:stale|temporary).{0,260}(?:clean|remove|unlink)",
        )

    def test_manifest_shutdown_reaps_children_closes_fds_and_preserves_status(self) -> None:
        source = (
            manifest_source()
            + "\n"
            + EXECUTION_MANIFEST_HELPER.read_text(encoding="utf-8")
        )
        for marker in (
            "SIGTERM",
            "SIGKILL",
            "wait",
            "reap",
            "setsid",
            "O_CLOEXEC",
            "FD_CLOEXEC",
            "PR_SET_CHILD_SUBREAPER",
            "drain_adopted_descendants",
        ):
            self.assertIn(marker, source)
        shutdown_region = source_near(source, "SIGTERM", radius=3600)
        self.assertRegex(
            shutdown_region,
            r"(?is)SIGTERM.{0,1400}(?:10\s*(?:seconds|s)|clock|deadline)"
            r".{0,1400}SIGKILL",
        )
        self.assertRegex(
            shutdown_region,
            r"(?is)(?:SIGKILL|kill).{0,500}(?:wait|reap)",
        )
        self.assertRegex(
            source,
            r"(?is)(?:original|saved|preserv\w*)[-_ ]status.{0,1200}"
            r"(?:finaliz|publish).{0,1200}(?:exit|return)",
        )
        self.assertRegex(
            source,
            r"(?is)(?:close|closed|closing).{0,260}"
            r"(?:evidence|fragment|manifest).{0,260}(?:fd|file descriptor)",
        )
        self.assertRegex(
            source,
            r"(?is)(?:process group|kill\s+[-].*group|kill\s+--\s*-).{0,800}"
            r"(?:wait|reap)",
        )

    def test_manifest_subreaper_uses_supported_linux_prctl_numbers_and_fails_closed(self) -> None:
        helper = EXECUTION_MANIFEST_HELPER.read_text(encoding="utf-8")
        self.assertRegex(
            helper,
            r"(?s)syscall_number\(157,\s*167\).*sys_prctl",
        )
        self.assertIn("x86_64", helper)
        self.assertIn("aarch64", helper)
        self.assertRegex(
            helper,
            r"(?s)PR_SET_CHILD_SUBREAPER.*?syscall\(.*?sys_prctl\(\).*?"
            r"fatal\(.*?child subreaper",
        )
        self.assertIn('subreaper => sub { establish_child_subreaper() }', helper)
        self.assertRegex(
            helper,
            r"(?s)handled_signal.*?subreaper.*?kill.*?handled_signal",
        )
        pre_fork = helper.split("my $pid = $process_control->{fork}->();", 1)[0]
        self.assertNotIn("$process_control->{subreaper}->();", pre_fork)

    def test_manifest_interrupt_retries_are_bounded(self) -> None:
        helper = EXECUTION_MANIFEST_HELPER.read_text(encoding="utf-8")
        self.assertIn("MAX_INTERRUPT_RETRIES => 16", helper)
        for operation in ("sys_getdents", "syswrite", "sysread", "waitpid"):
            self.assertIn(operation, helper)
        for diagnostic in (
            "directory enumeration exceeded the interrupt retry limit",
            "evidence write exceeded the interrupt retry limit",
            "fragment read exceeded the interrupt retry limit",
            "descendant reap exceeded the interrupt retry limit",
        ):
            self.assertIn(diagnostic, helper)
        blocking_wait = helper.split(
            "my $reaped = $process_control->{waitpid}->(-1, 0);",
            1,
        )[1][:600]
        self.assertIn(
            'fatal("could not drain adopted scheduler descendants (errno $errno)")',
            blocking_wait,
        )
        self.assertEqual(
            helper.count(
                'fatal("could not drain adopted scheduler descendants (errno $errno)")'
            ),
            2,
        )
        self.assertGreaterEqual(helper.count("$! == EINTR"), 4)
        self.assertGreaterEqual(helper.count("$! == EAGAIN"), 3)

    def test_manifest_unexpected_blocking_waitpid_error_fails_closed(self) -> None:
        perl = shutil.which("perl")
        self.assertIsNotNone(perl, "Perl is required for execution-manifest coverage")
        assert perl is not None
        harness = r"""
use strict;
use warnings;
use Errno qw(EIO);
require $ARGV[0];
my $calls = 0;
my $error;
eval {
    main::drain_adopted_descendants({
        waitpid => sub {
            ++$calls;
            return 0 if $calls == 1;
            $! = EIO;
            return -1;
        },
    });
    1;
} or $error = $@;
exit 91 unless ref($error) && $error->isa("ExecutionManifest::Fatal");
exit 92 unless "$error" eq "could not drain adopted scheduler descendants (errno 5)";
exit 0;
"""
        result = subprocess.run(
            [perl, "-e", harness, str(EXECUTION_MANIFEST_HELPER)],
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )
        self.assertEqual(
            result.returncode,
            0,
            msg=f"waitpid failure did not fail closed\nstdout:\n{result.stdout}\nstderr:\n{result.stderr}",
        )

    def test_manifest_unexpected_nonblocking_waitpid_error_fails_closed(self) -> None:
        perl = shutil.which("perl")
        self.assertIsNotNone(perl, "Perl is required for execution-manifest coverage")
        assert perl is not None
        harness = r"""
use strict;
use warnings;
use Errno qw(EIO);
require $ARGV[0];
my $error;
eval {
    main::drain_adopted_descendants({
        waitpid => sub {
            $! = EIO;
            return -1;
        },
    });
    1;
} or $error = $@;
exit 91 unless ref($error) && $error->isa("ExecutionManifest::Fatal");
exit 92 unless "$error" eq "could not drain adopted scheduler descendants (errno 5)";
exit 0;
"""
        result = subprocess.run(
            [perl, "-e", harness, str(EXECUTION_MANIFEST_HELPER)],
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            check=False,
        )
        self.assertEqual(
            result.returncode,
            0,
            msg=(
                "nonblocking waitpid failure did not fail closed\n"
                f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
            ),
        )

    def test_diagnostic_redaction_normalizes_ansi_before_matching(self) -> None:
        layer1_jobs = load_layer1_jobs()
        diagnostic = f"error:\x1b[31m{ROOT}/private/output\x1b[0m"

        redacted = layer1_jobs.redact_diagnostic_line(diagnostic)

        self.assertIn("<repo>/private/output", redacted)
        self.assertNotIn(str(ROOT), redacted)
        self.assertNotIn("\x1b", redacted)
        self.assertNotIn("[31m", redacted)

    def test_diagnostic_redaction_recognizes_backtick_boundaries(self) -> None:
        layer1_jobs = load_layer1_jobs()
        diagnostic = f"failed to read `{ROOT}/private/output`"

        redacted = layer1_jobs.redact_diagnostic_line(diagnostic)

        self.assertEqual(redacted, "failed to read `<repo>/private/output`")
        self.assertNotIn(str(ROOT), redacted)


if __name__ == "__main__":
    unittest.main()
