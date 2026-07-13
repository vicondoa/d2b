#!/usr/bin/env python3
"""Layer-1 local runner and GitHub Actions workflow renderer."""

from __future__ import annotations

import argparse
import difflib
import json
import os
import pathlib
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "tests" / "layer1-jobs.json"
TEMPLATE = ROOT / "tests" / "ci" / "layer1-workflow.template.yml"
WORKFLOW = ROOT / ".github" / "workflows" / "pr-l1-static-fast.yml"
CHECKOUT = "actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5"
INSTALL_NIX = "cachix/install-nix-action@23cf0fec1d55e0b1f2631aedd2a610c21ef8b077"
RUST_CACHE = "Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32"


def load_manifest() -> dict[str, Any]:
    with MANIFEST.open(encoding="utf-8") as fh:
        manifest = json.load(fh)
    if manifest.get("version") != 1:
        raise SystemExit(f"unsupported {MANIFEST} version: {manifest.get('version')!r}")
    jobs = manifest.get("jobs")
    if not isinstance(jobs, dict):
        raise SystemExit(f"{MANIFEST}: jobs must be an object")
    for phase in manifest.get("local", {}).get("phases", []):
        for job_id in phase.get("jobs", []):
            if job_id not in jobs:
                raise SystemExit(f"{MANIFEST}: local phase references unknown job {job_id!r}")
    for job_id in manifest.get("ci", {}).get("jobs", []):
        if job_id not in jobs:
            raise SystemExit(f"{MANIFEST}: ci.jobs references unknown job {job_id!r}")
    for job_id in manifest.get("ci", {}).get("rollupNeeds", []):
        if job_id not in jobs:
            raise SystemExit(f"{MANIFEST}: ci.rollupNeeds references unknown job {job_id!r}")
    return manifest


def indent(text: str, spaces: int) -> str:
    prefix = " " * spaces
    return "\n".join(prefix + line if line else line for line in text.splitlines())


def yaml_list(values: list[str]) -> str:
    return "[" + ", ".join(values) + "]"


def needs_line(job: dict[str, Any]) -> str:
    needs = job.get("needs", [])
    return f"    needs: {yaml_list(needs)}\n" if needs else ""


def nix_setup_step() -> str:
    return f"""      - uses: {INSTALL_NIX}
        with:
          nix_path: nixpkgs=channel:nixos-unstable
          extra_nix_config: |
            experimental-features = nix-command flakes"""


def simple_nix_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    steps:
      - uses: {CHECKOUT}
{nix_setup_step()}
      - name: {job["displayName"]}
        run: make {job["makeTarget"]}"""


def tier0_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    steps:
      - uses: {CHECKOUT}
      - name: {job["displayName"]}
        run: make {job["makeTarget"]}
      - name: ADR index coverage guard
        run: bash tests/unit/meta/adr-index-coverage.sh
      - name: CI coverage structural guard
        run: bash tests/unit/meta/ci-coverage.sh
      - name: Test rearchitecture fail-closed gates
        run: bash tests/tools/gen-migration-ledger.sh --check"""


def changelog_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
    if: github.event_name == 'pull_request'
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    steps:
      - uses: {CHECKOUT}
        with:
          fetch-depth: 0
      - name: {job["displayName"]}
        run: bash scripts/changelog-check.sh"""


def rust_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    # Warm (rust-cache hit): ~8-12 min. Cold (no cache): ~43 min.
    timeout-minutes: {job["timeoutMinutes"]}
    env:
      # Disable sccache in CI - we use Swatinem/rust-cache (target-dir
      # caching) instead, which caches ALL compiled artifacts including
      # proc-macros and bin/lib crates that sccache cannot cache (~60% of
      # compilations are "non-cacheable" by sccache due to crate-type).
      # CARGO_INCREMENTAL=0 is still set: incremental compilation artifacts
      # are non-deterministic and bloat the cache without benefit for CI
      # (each PR run starts from a different commit).
      CARGO_INCREMENTAL: "0"
      # Override the repo .cargo/config.toml rustc-wrapper (sccache) so
      # rust-cache's post-step `cargo metadata` doesn't fail looking for
      # an sccache binary that isn't installed.
      CARGO_BUILD_RUSTC_WRAPPER: ""
    steps:
      - uses: {CHECKOUT}
        with:
          fetch-depth: 0
{nix_setup_step()}
      - name: Free runner disk for Rust gate
        run: |
          df -h
          sudo rm -rf /usr/local/lib/android /usr/share/dotnet /opt/ghc /usr/local/.ghcup /opt/hostedtoolcache/CodeQL || true
          docker system prune -af || true
          df -h
      - name: Install pinned Rust toolchain + ripgrep + acl
        # MUST run BEFORE Swatinem/rust-cache: the cache action reads
        # `rustc --version` to compute its key hash, so the pinned
        # toolchain must be the active default when the cache step runs.
        # Without this, the runner's pre-installed 1.96.0 is hashed,
        # and the cache is keyed on the wrong compiler version.
        run: |
          PINNED=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\\([^"]*\\)".*/\\1/p' packages/rust-toolchain.toml | head -1)
          rustup toolchain install "$PINNED" --profile minimal --component rustfmt --component clippy
          rustup default "$PINNED"
          echo "Rust toolchain: $(rustc --version)"
          sudo apt-get update && sudo apt-get install -y ripgrep acl
      - name: Rust dependency cache (target dirs + cargo registry)
        # Swatinem/rust-cache caches dependency artifacts in target dirs
        # and the cargo registry. It performs all I/O in its own action
        # process (JavaScript pre/post steps) via @actions/cache - no
        # ACTIONS_RUNTIME_TOKEN or cache credentials are exposed to `run:`
        # steps where untrusted crate code (build scripts, proc-macros,
        # `cargo test`) executes.
        uses: {RUST_CACHE}
        with:
          workspaces: |
            packages -> target
            packages/d2b-priv-broker -> target
          cache-directories: |
            packages/d2b-priv-broker/target-layer1
            packages/d2b-priv-broker/target-fakebackends
          prefix-key: "v0-rust"
          shared-key: "test-rust-${{{{ runner.os }}}}"
          save-if: "true"
      - name: {job["displayName"]}
        # Skip the fixture nix-build (~35 min) - the same fixtures are
        # already evaluated by the flake-eval-x86 (fixture-smoke) and
        # (fixture-smoke-full) shards. The contract tests that depend on
        # D2B_FIXTURES still run in those shards' eval; here we test only
        # the Rust compilation + unit/integration tests.
        env:
          D2B_SKIP_FIXTURE_BUILD: "1"
        run: make {job["makeTarget"]}"""


def flake_discover_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    outputs:
      checks: ${{{{ steps.list.outputs.checks }}}}
    steps:
      - uses: {CHECKOUT}
{nix_setup_step()}
      - id: list
        name: {job["displayName"]}
        run: |
          checks=$(make -s test-flake-list)
          echo "discovered checks: $checks"
          echo "checks=$checks" >> "$GITHUB_OUTPUT"
"""


def flake_x86_shards_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    strategy:
      fail-fast: false
      max-parallel: {job["maxParallel"]}
      matrix:
        check: ${{{{ fromJSON(needs.flake-eval-discover.outputs.checks) }}}}
    steps:
      - uses: {CHECKOUT}
      - name: Add swap (insurance for the heaviest single check)
        run: |
          # A single check instantiates in its own process and fits a 16 GB
          # runner (heaviest measured ~12 GB), so unlike the old monolith this
          # rarely touches swap. Add a modest swapfile purely as OOM insurance.
          SWAP=/mnt/d2b-ci-swap
          sudo swapoff "$SWAP" 2>/dev/null || true
          sudo rm -f "$SWAP"
          sudo fallocate -l 8G "$SWAP" || sudo dd if=/dev/zero of="$SWAP" bs=1M count=8192
          sudo chmod 600 "$SWAP"
          sudo mkswap "$SWAP"
          sudo swapon "$SWAP"
{nix_setup_step()}
      - name: Install flake shard diagnostics
        run: sudo apt-get update && sudo apt-get install -y gdb
      - name: {job["displayName"]}
        # D2B_FLAKE_CHECK is passed via the step environment, NOT interpolated
        # into the shell command: a flake check attr name is PR-controlled, so
        # `D2B_FLAKE_CHECK='${{{{ matrix.check }}}}' ...` would be a shell-injection
        # vector. test-flake.sh additionally rejects names outside [A-Za-z0-9._-].
        env:
          D2B_FLAKE_CHECK: ${{{{ matrix.check }}}}
        run: make test-flake"""


def flake_x86_outputs_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    steps:
      - uses: {CHECKOUT}
{nix_setup_step()}
      - name: {job["displayName"]}
        env:
          D2B_FLAKE_OUTPUTS: "1"
        run: make test-flake"""


def flake_x86_rollup_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
    needs: {yaml_list(job["needs"])}
    if: always()
    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    steps:
      - name: {job["displayName"]}
        run: |
          discover='${{{{ needs.flake-eval-discover.result }}}}'
          shards='${{{{ needs.flake-eval-x86.result }}}}'
          outputs='${{{{ needs.flake-eval-x86-outputs.result }}}}'
          echo "flake-eval-discover=$discover  flake-eval-x86=$shards  flake-eval-x86-outputs=$outputs"
          if [ "$discover" = success ] && [ "$shards" = success ] && [ "$outputs" = success ]; then
            echo "All x86_64-linux flake checks + outputs passed."
          else
            echo "::error::x86_64 flake gate failed (discover=$discover, shards=$shards, outputs=$outputs)"
            exit 1
          fi"""


def flake_aarch64_smoke_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    steps:
      - uses: {CHECKOUT}
{nix_setup_step()}
      - name: {job["displayName"]}
        run: |
          nix-instantiate --eval --strict \\
            -E 'let f = import ./tests/unit/smoke/smoke-eval-aarch64.nix; r = f {{}}; in r.drvPath' \\
            >/dev/null"""


def check_rollup_job(manifest: dict[str, Any]) -> str:
    ci = manifest["ci"]
    rollup = ci["rollupJob"]
    needs = ci["rollupNeeds"]
    allowed_skipped = set(ci.get("allowedSkippedRollupJobs", []))
    lines = [
        f"  {rollup}:",
        f"    needs: {yaml_list(needs)}",
        "    if: always()",
        "    runs-on: ubuntu-latest",
        "    timeout-minutes: 5",
        "    steps:",
        "      - name: Require generated Layer-1 gate graph to pass",
        "        run: |",
        "          failed=0",
        "          require_success() {",
        "            name=\"$1\"",
        "            result=\"$2\"",
        "            echo \"$name=$result\"",
        "            if [ \"$result\" != success ]; then",
        "              echo \"::error::$name did not pass (result=$result)\"",
        "              failed=1",
        "            fi",
        "          }",
        "          allow_success_or_skipped() {",
        "            name=\"$1\"",
        "            result=\"$2\"",
        "            echo \"$name=$result\"",
        "            if [ \"$result\" != success ] && [ \"$result\" != skipped ]; then",
        "              echo \"::error::$name did not pass (result=$result)\"",
        "              failed=1",
        "            fi",
        "          }",
    ]
    for need in needs:
        expr = "${{ needs." + need + ".result }}"
        if need in allowed_skipped:
            lines.append(f"          allow_success_or_skipped {need} '{expr}'")
        else:
            lines.append(f"          require_success {need} '{expr}'")
    lines.extend(
        [
            "          if [ \"$failed\" -ne 0 ]; then",
            "            exit 1",
            "          fi",
            "          echo \"All generated Layer-1 jobs passed.\"",
        ]
    )
    return "\n".join(lines)


RENDERERS = {
    "tier0": tier0_job,
    "simple-nix": simple_nix_job,
    "changelog": changelog_job,
    "rust": rust_job,
    "flake-discover": flake_discover_job,
    "flake-x86-shards": flake_x86_shards_job,
    "flake-x86-outputs": flake_x86_outputs_job,
    "flake-x86-rollup": flake_x86_rollup_job,
    "flake-aarch64-smoke": flake_aarch64_smoke_job,
}


def render_workflow(manifest: dict[str, Any]) -> str:
    jobs = manifest["jobs"]
    rendered_jobs = []
    for job_id in manifest["ci"]["jobs"]:
        job = jobs[job_id]
        kind = job["ciKind"]
        renderer = RENDERERS.get(kind)
        if renderer is None:
            raise SystemExit(f"{MANIFEST}: no renderer for ciKind {kind!r}")
        rendered_jobs.append(renderer(job))
    rendered_jobs.append(check_rollup_job(manifest))
    template = TEMPLATE.read_text(encoding="utf-8")
    workflow = template.replace("{{ workflow_name }}", manifest["ci"]["workflowName"])
    workflow = workflow.replace("{{ jobs }}", "\n\n".join(rendered_jobs))
    return workflow.rstrip() + "\n"


def command_render_workflow(args: argparse.Namespace) -> int:
    text = render_workflow(load_manifest())
    if args.write:
        WORKFLOW.write_text(text, encoding="utf-8")
    else:
        sys.stdout.write(text)
    return 0


def command_check_workflow(_: argparse.Namespace) -> int:
    expected = render_workflow(load_manifest())
    actual = WORKFLOW.read_text(encoding="utf-8") if WORKFLOW.exists() else ""
    if actual == expected:
        print("layer1 workflow: generated artifact is up to date")
        return 0
    diff = difflib.unified_diff(
        actual.splitlines(keepends=True),
        expected.splitlines(keepends=True),
        fromfile=str(WORKFLOW),
        tofile=f"{WORKFLOW} (regenerated)",
    )
    sys.stderr.writelines(diff)
    return 1


def run_job(job_id: str, job: dict[str, Any]) -> int:
    target = job.get("makeTarget")
    if not target:
        raise RuntimeError(f"local job {job_id!r} has no makeTarget")
    env = os.environ.copy()
    env.update(job.get("localEnv", {}))
    log_dir = pathlib.Path(tempfile.mkdtemp(prefix=f"d2b-{job_id}."))
    log_path = log_dir / "output.log"
    print(f"==> {target} ({job.get('displayName', job_id)})", flush=True)
    with log_path.open("wb") as log:
        proc = subprocess.run(["make", target], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT)
    if proc.returncode == 0:
        print(f"ok: {target}", flush=True)
        if os.environ.get("D2B_CHECK_KEEP_LOGS") != "1":
            try:
                log_path.unlink()
                log_dir.rmdir()
            except OSError:
                pass
        return 0
    print(f"FAIL: {target} (exit {proc.returncode}); tail of {log_path}:", file=sys.stderr, flush=True)
    try:
        lines = log_path.read_text(encoding="utf-8", errors="replace").splitlines()
        for line in lines[-200:]:
            print(line, file=sys.stderr)
    except OSError as exc:
        print(f"could not read {log_path}: {exc}", file=sys.stderr)
    return proc.returncode


def selected_phases(manifest: dict[str, Any], include_preflight: bool) -> list[dict[str, Any]]:
    phases = manifest["local"]["phases"]
    if include_preflight:
        return phases
    return [phase for phase in phases if phase["id"] != "preflight"]


def command_run_local(args: argparse.Namespace) -> int:
    manifest = load_manifest()
    jobs = manifest["jobs"]
    try:
        max_jobs = int(os.environ.get("D2B_CHECK_JOBS", manifest["local"].get("defaultJobs", 4)))
    except ValueError:
        print("D2B_CHECK_JOBS must be an integer", file=sys.stderr)
        return 2
    if max_jobs < 1:
        print("D2B_CHECK_JOBS must be >= 1", file=sys.stderr)
        return 2
    include_preflight = not args.skip_preflight
    for phase in selected_phases(manifest, include_preflight):
        mode = phase["mode"]
        phase_jobs = phase["jobs"]
        print(f"==> Layer-1 phase: {phase['id']} ({mode})", flush=True)
        if mode == "serial":
            for job_id in phase_jobs:
                rc = run_job(job_id, jobs[job_id])
                if rc != 0:
                    return rc
        elif mode == "parallel":
            failed = 0
            with ThreadPoolExecutor(max_workers=max_jobs) as pool:
                futures = {pool.submit(run_job, job_id, jobs[job_id]): job_id for job_id in phase_jobs}
                for future in as_completed(futures):
                    rc = future.result()
                    if rc != 0:
                        failed = rc
            if failed != 0:
                return failed
        else:
            print(f"unknown phase mode {mode!r}", file=sys.stderr)
            return 2
    print("Layer-1 manifest runner OK")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    render = subparsers.add_parser("render-workflow", help="render the GitHub Actions workflow")
    render.add_argument("--write", action="store_true", help="write the rendered workflow in place")
    render.set_defaults(func=command_render_workflow)

    check = subparsers.add_parser("check-workflow", help="fail if the rendered workflow is stale")
    check.set_defaults(func=command_check_workflow)

    run = subparsers.add_parser("run-local", help="run local Layer-1 phases from the manifest")
    run.add_argument("--skip-preflight", action="store_true", help="skip the preflight phase")
    run.set_defaults(func=command_run_local)

    args = parser.parse_args()
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
