#!/usr/bin/env python3
"""Regression coverage for the CI shell and local Layer-1 runner."""

from __future__ import annotations

import contextlib
import importlib.util
import io
import os
import pathlib
import shlex
import shutil
import subprocess
import tempfile
import types
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[3]
SCRATCH = ROOT / ".scratch"
LAYER1_JOBS = ROOT / "tests" / "tools" / "layer1-jobs.py"


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

    def test_rust_gate_is_two_required_shards_with_one_stable_rollup(self) -> None:
        layer1_jobs = load_layer1_jobs()
        manifest = layer1_jobs.load_manifest()
        workflow = layer1_jobs.render_workflow(manifest)

        rust_rollup = manifest["jobs"]["test-rust"]
        self.assertEqual(
            rust_rollup["needs"],
            ["test-rust-main", "test-rust-remaining"],
        )
        self.assertEqual(rust_rollup["ciKind"], "rust-rollup")
        self.assertIn("run: make test-rust-main", workflow)
        self.assertIn("run: make test-rust-remaining", workflow)
        self.assertIn("test-rust-main=$result", workflow)
        self.assertIn("test-rust-remaining=$result", workflow)
        self.assertEqual(workflow.count('[ "$result" = success ] || failed=1'), 2)
        self.assertIn('[ "$failed" -eq 0 ] || exit 1', workflow)
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
