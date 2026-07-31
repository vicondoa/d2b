#!/usr/bin/env python3
"""Layer-1 local runner and GitHub Actions workflow renderer."""

from __future__ import annotations

import argparse
import difflib
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Any


ROOT = pathlib.Path(__file__).resolve().parents[2]
MANIFEST = ROOT / "tests" / "layer1-jobs.json"
TEMPLATE = ROOT / "tests" / "ci" / "layer1-workflow.template.yml"
WORKFLOW = ROOT / ".github" / "workflows" / "pr-l1-static-fast.yml"
SELF_TEST = ROOT / "tests" / "unit" / "meta" / "ci-runner-regression.py"
CHECKOUT = "actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5"
CACHE = "actions/cache@0057852bfaa89a56745cba8c7296529d2fc39830"
INSTALL_NIX = "cachix/install-nix-action@23cf0fec1d55e0b1f2631aedd2a610c21ef8b077"
RUST_CACHE = "Swatinem/rust-cache@e18b497796c12c097a38f9edb9d0641fb99eee32"
# Caches /nix through the GitHub Actions cache. The developer host has a local
# attic substituter, which is why a fixture build that costs half an hour in CI
# completes in seconds there; runners cannot reach that endpoint, so without
# this they rebuild the entire Rust host-tool set from source on every run.
NIX_CACHE = "nix-community/cache-nix-action@7df957e333c1e5da7721f60227dbba6d06080569"
# The shell program must be resolvable by the Actions runner itself, which
# looks the first token up on PATH rather than against the workspace, and whose
# argument splitter does not preserve nested quoting. The runner resolves `sh`
# on PATH; hosted runners provide dash, which does not process Bash startup
# hooks before the wrapper can scrub them.
SCRUBBED_BASH = "sh tests/tools/ci-shell {0}"
PATH_START = r"(?P<prefix>^|[\s'\"`(\[{: =])"
ROOT_END = r"(?=$|[/\s'\"`),\]};:])"
ABSOLUTE_PATH = re.compile(
    PATH_START + r"/[^ \t\r\n'\"`),\]}>;,|]*"
)


def validate_job_id(job_id: str, context: str) -> None:
    if not re.fullmatch(r"[A-Za-z0-9_-]+", job_id):
        raise SystemExit(f"{MANIFEST}: invalid job id {job_id!r} in {context}")


def load_manifest() -> dict[str, Any]:
    with MANIFEST.open(encoding="utf-8") as fh:
        manifest = json.load(fh)
    if manifest.get("version") != 1:
        raise SystemExit(f"unsupported {MANIFEST} version: {manifest.get('version')!r}")
    jobs = manifest.get("jobs")
    if not isinstance(jobs, dict):
        raise SystemExit(f"{MANIFEST}: jobs must be an object")
    for job_id, job in jobs.items():
        validate_job_id(job_id, "jobs")
        ci_job_id = job.get("ciJobId")
        if ci_job_id is not None:
            if not isinstance(ci_job_id, str):
                raise SystemExit(f"{MANIFEST}: ciJobId for {job_id!r} must be a string")
            validate_job_id(ci_job_id, f"jobs.{job_id}.ciJobId")

    local_job_ids: list[str] = []
    for phase in manifest.get("local", {}).get("phases", []):
        for job_id in phase.get("jobs", []):
            validate_job_id(job_id, "local.phases")
            if job_id not in jobs:
                raise SystemExit(f"{MANIFEST}: local phase references unknown job {job_id!r}")
            local_job_ids.append(job_id)
    local_jobs = set(local_job_ids)
    if len(local_jobs) != len(local_job_ids):
        raise SystemExit(f"{MANIFEST}: a job appears in more than one local phase")
    ci_jobs = set(manifest.get("ci", {}).get("jobs", []))
    for job_id in manifest.get("ci", {}).get("jobs", []):
        validate_job_id(job_id, "ci.jobs")
        if job_id not in jobs:
            raise SystemExit(f"{MANIFEST}: ci.jobs references unknown job {job_id!r}")
        for need in jobs[job_id].get("needs", []):
            validate_job_id(need, f"jobs.{job_id}.needs")
            if need not in ci_jobs:
                raise SystemExit(
                    f"{MANIFEST}: CI job {job_id!r} needs unknown/non-CI job {need!r}"
                )
        if (
            jobs[job_id].get("makeTarget")
            and job_id not in local_jobs
            and not jobs[job_id].get("ciOnly")
        ):
            raise SystemExit(
                f"{MANIFEST}: CI job {job_id!r} has a local make target but is absent "
                "from local.phases and is not declared ciOnly"
            )
    for job_id in manifest.get("ci", {}).get("rollupNeeds", []):
        validate_job_id(job_id, "ci.rollupNeeds")
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


def ci_env_block(job: dict[str, Any], spaces: int) -> str:
    env = job.get("ciEnv", {})
    if not env:
        return ""
    prefix = " " * spaces
    lines = [f"{prefix}env:"]
    for name, value in env.items():
        if not re.fullmatch(r"[A-Z_][A-Z0-9_]*", name):
            raise SystemExit(f"{MANIFEST}: invalid CI environment name {name!r}")
        if not isinstance(value, str):
            raise SystemExit(f"{MANIFEST}: CI environment value for {name!r} must be a string")
        lines.append(f"{prefix}  {name}: {json.dumps(value)}")
    return "\n".join(lines) + "\n"


# Jobs whose nix store cache is worth its share of the repository cache budget.
#
# GitHub evicts repository caches by LRU against a fixed ~10 GB total, shared
# with the rust-cache entries this gate also depends on. Caching /nix for all
# twelve nix-using jobs would fan the key space out far past that total, and the
# steady state would be everything evicting everything - worse than not caching.
# So only the fixture-contract job gets an entry: it owns one bounded key and
# the narrow realized video dependency. Per-shard Nix-unit caches multiply the
# cap by the matrix width and exceed the repository budget even when each entry
# is individually bounded. The flake-eval shards finish in under a minute and
# the lint/drift jobs only evaluate.
#
# On sizing: gc-max-store-size-linux caps the UNCOMPRESSED /nix store before
# save, which is not the size of the resulting cache entry - the entry is
# compressed and materially smaller. Two capped stores plus rust-cache
# therefore sit inside the budget rather than consuming it outright, but this
# configuration is deliberately near enough to the cap that it should be
# re-measured against the repository cache-usage page after a run rather than
# assumed.
NIX_CACHED_JOBS = frozenset({"test-fixture-contracts"})
NIX_CACHE_MAX_STORE = "4G"
NIX_CACHE_FORMAT = "v1"
# Scope suffix for a matrix job, so shards do not share one key. Defined here
# rather than at the call site because a brace-doubled literal written inside an
# f-string replacement field is ordinary Python source, not escaped f-string
# text, and would emit four literal braces - an invalid GitHub expression.
MATRIX_CHECK_SCOPE = "-${{ matrix.check }}"
# Where the realized lane's targeted binary cache lives. Deliberately under the
# runner temp directory rather than the workspace: a volatile file inside $ROOT
# races the source capture that flake evaluation performs, which is the same
# reason tests/lib.sh keeps its bookkeeping out of the repository root. Written
# with single braces because this is ordinary module-level source, not f-string
# text - see MATRIX_CHECK_SCOPE above for the same hazard.
REALIZED_CACHE_DIR = "${{ runner.temp }}/d2b-realized-cache"


def nix_cache_hash_files(job: dict[str, Any]) -> str:
    del job
    return ", ".join(repr(pattern) for pattern in ["flake.lock", "**/*.nix"])


def nix_setup_step(job: dict[str, Any], scope_suffix: str = "") -> str:
    """Renders nix installation, plus a store cache for the jobs that earn one.

    The cache key is scoped to the job id (and, for a matrix job, to the shard
    via scope_suffix). Every nix job used to share one key, and because
    actions/cache never overwrites an existing entry, whichever job finished
    first froze the cache at whatever its store held. In practice that was a
    trivial lint or drift job, so the fixture-contract gate restored ~94 MB and
    then still logged "these 166 derivations will be built" - the cache hit,
    reported success, and saved nothing worth having.

    Scoping also bounds purge-prefixes, which with purge-created 0 could
    otherwise delete another job's entry.

    The job parameter is required rather than optional: defaulting it would make
    an omitted argument silently emit a shared key, which is the defect this
    exists to prevent.
    """
    scope = job["ciJobId"] + scope_suffix
    hash_files = nix_cache_hash_files(job)
    setup = f"""      - uses: {INSTALL_NIX}
        with:
          nix_path: nixpkgs=channel:nixos-unstable
          extra_nix_config: |
            experimental-features = nix-command flakes"""
    if job["ciJobId"] not in NIX_CACHED_JOBS:
        return setup
    return (
        setup
        + f"""
      - name: Nix store cache
        # Without this the job rebuilds the Rust host-tool set from source,
        # which is what makes the fixture-contract gate cost half an hour here
        # while completing in seconds on a host with a substituter. Restore is
        # best-effort: a cache miss is slow, never incorrect. See
        # NIX_CACHED_JOBS for why only some jobs carry an entry.
        uses: {NIX_CACHE}
        with:
          # The format epoch rotates immutable Actions caches when retention
          # semantics change. Fixture jobs use a source-complete hash; other
          # cached Nix jobs hash the flake and Nix corpus they evaluate.
          primary-key: nix-{NIX_CACHE_FORMAT}-${{{{ runner.os }}}}-{scope}-${{{{ hashFiles({hash_files}) }}}}
          restore-prefixes-first-match: nix-{NIX_CACHE_FORMAT}-${{{{ runner.os }}}}-{scope}-
          gc-max-store-size-linux: {NIX_CACHE_MAX_STORE}
          purge: true
          purge-prefixes: nix-{NIX_CACHE_FORMAT}-${{{{ runner.os }}}}-{scope}-
          purge-created: 0
          purge-primary-key: never"""
    )


def simple_nix_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
{ci_env_block(job, 4)}\
    steps:
      - uses: {CHECKOUT}
        with:
          persist-credentials: false
{nix_setup_step(job)}
      - name: {job["displayName"]}
        run: make {job["makeTarget"]}"""


def simple_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    steps:
      - uses: {CHECKOUT}
        with:
          persist-credentials: false
      - name: {job["displayName"]}
        run: make {job["makeTarget"]}"""


def tier0_job(job: dict[str, Any]) -> str:
    extra = "".join(
        f"\n      - name: {step['displayName']}\n        run: make {step['makeTarget']}"
        for step in job.get("extraMakeTargets", [])
    )
    return f"""  {job["ciJobId"]}:
    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    steps:
      - uses: {CHECKOUT}
        with:
          persist-credentials: false
      - name: {job["displayName"]}
        run: make {job["makeTarget"]}{extra}"""


def changelog_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
    if: github.event_name == 'pull_request'
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    steps:
      - uses: {CHECKOUT}
        with:
          persist-credentials: false
          fetch-depth: 0
      - name: {job["displayName"]}
        run: make {job["makeTarget"]}"""


def render_env(job: dict[str, Any]) -> str:
    """Renders a job's declared environment as workflow step `env:` entries.

    The environment comes from the manifest so the local runner and the
    workflow cannot disagree about which lane executes which layer. Hardcoding
    it here once let `test-rust` skip the fixture build in continuous
    integration while the manifest said nothing about it.
    """
    env = job.get("localEnv", {})
    if not env:
        return "          {}"
    return "\n".join(f'          {key}: "{value}"' for key, value in sorted(env.items()))


def heavy_gate_step(job: dict[str, Any]) -> str:
    """Renders the heavy-gate provisioning step for jobs that need the gate.

    The gate root is a protected runtime directory that a NixOS host gets from
    tmpfiles. A continuous-integration runner has no such directory, and the
    gate fails closed rather than falling back to an unprotected namespace, so
    a job whose target acquires a slot must provision it first.
    """
    if not job.get("requiresHeavyGate"):
        return ""
    return (
        "      - name: Provision the heavy-gate runtime root\n"
        "        run: make heavy-gate-provision\n"
    )


def rust_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    # Warm (rust-cache hit): ~8-12 min. Cold (no cache): ~43 min.
    timeout-minutes: {job["timeoutMinutes"]}
    env:
      # CI uses Swatinem/rust-cache (target-dir caching) and NOT sccache. That
      # is not a preference - sccache is incompatible with this workspace's
      # integration tests. Its server forwards only a whitelist of environment
      # variables to rustc, and CARGO_BIN_EXE_<name>, which Cargo injects so an
      # integration test can locate the binary it exercises, is not on it. Any
      # test using env!("CARGO_BIN_EXE_...") then fails to compile with
      # "environment variable ... not defined at compile time"; measured on this
      # gate, d2b-exec-runner/tests/tty_pty_integration.rs fails exactly that
      # way. It does not reproduce locally because CARGO_INCREMENTAL is on
      # there, which makes sccache treat those compilations as non-cacheable and
      # pass them through untouched.
      #
      # sccache remains enabled for local development, where the same shim is
      # reached through packages/.cargo/config.toml. Only CI opts out.
      #
      # CARGO_INCREMENTAL=0 is still set: incremental compilation artifacts are
      # non-deterministic and bloat the cache without benefit for CI (each PR
      # run starts from a different commit).
      CARGO_INCREMENTAL: "0"
    steps:
      - uses: {CHECKOUT}
        with:
          persist-credentials: false
          fetch-depth: 0
{nix_setup_step(job)}
      - name: Free runner disk for Rust gate
        run: |
          df -h /
          avail_kib=$(df -Pk / | awk 'NR == 2 {{ print $4 }}')
          # Reclaiming the preinstalled Android/dotnet/GHC trees and the Docker
          # image cache costs real wall time - measured across this workflow's
          # four Rust-profile jobs it ranged from 43 s to 3 min 48 s, the worst
          # of which sat on the critical path - and buys about 22 GiB on a
          # runner image that already arrives with 83-88 GiB free. Nothing in
          # this gate approaches that, so only pay the cost when an image
          # actually turns up short. This still fails safe: a fuller image
          # reclaims exactly as before.
          threshold_kib=$((70 * 1024 * 1024))
          if [ "$avail_kib" -ge "$threshold_kib" ]; then
            echo "free space $((avail_kib / 1024 / 1024)) GiB is at or above the $((threshold_kib / 1024 / 1024)) GiB threshold; skipping reclaim"
            exit 0
          fi
          echo "free space $((avail_kib / 1024 / 1024)) GiB is below the $((threshold_kib / 1024 / 1024)) GiB threshold; reclaiming"
          sudo rm -rf /usr/local/lib/android /usr/share/dotnet /opt/ghc /usr/local/.ghcup /opt/hostedtoolcache/CodeQL || true
          docker system prune -af || true
          df -h /
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
            packages/d2b-guest-shell-runner -> target
          cache-directories: |
            packages/d2b-priv-broker/target-layer1
            packages/d2b-priv-broker/target-fakebackends
            tests/tools/no-bash-ast-walker/target
            .scratch/rust-test-cache
          prefix-key: "v2-rust-api-json"
          shared-key: "test-rust-${{{{ runner.os }}}}"
          # The repository-local trees are keyed on rustc -vV by their owning
          # tests. Cargo fingerprints compiled inputs on restore, while failed
          # compile units are never cached and still rerun on every gate.
          # A single writer keeps concurrent saves from racing one shared key.
          # The main-workspace Rust shard is that writer; it produces both the
          # common Cargo target and the slow compiler/rustdoc scratch trees.
          # Every other Rust job restores the last complete entry. On a cold key
          # parallel shards may duplicate compilation, but only one entry wins.
          save-if: "{'true' if job["ciJobId"] == 'test-rust-main' else 'false'}"
{heavy_gate_step(job)}      - name: {job["displayName"]}
        env:
{render_env(job)}
        run: make {job["makeTarget"]}
"""


def rust_rollup_job(job: dict[str, Any]) -> str:
    needs = job["needs"]
    lines = [
        f'  {job["ciJobId"]}:',
        f'    needs: {yaml_list(needs)}',
        '    if: always()',
        f'    runs-on: {job["runsOn"]}',
        f'    timeout-minutes: {job["timeoutMinutes"]}',
        '    steps:',
        f'      - uses: {CHECKOUT}',
        '        with:',
        '          persist-credentials: false',
        f'      - name: {job["displayName"]}',
        '        run: |',
    ]
    lines.append("          failed=0")
    for need in needs:
        lines.append(f"          result='${{{{ needs.{need}.result }}}}'")
        lines.append(f'          echo "{need}=$result"')
        lines.append(f'          [ "$result" = success ] || failed=1')
    lines.append('          [ "$failed" -eq 0 ] || exit 1')
    lines.append('          echo "Both Rust gate shards passed."')
    return "\n".join(lines)


def nix_unit_discover_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    outputs:
      checks: ${{{{ steps.list.outputs.nixunitchecks }}}}
    steps:
      - uses: {CHECKOUT}
        with:
          persist-credentials: false
{nix_setup_step(job)}
      - id: list
        name: {job["displayName"]}
        run: |
          # Same partition tool as flake-eval-discover, so this lane is handed
          # exactly the names that lane drops from its instantiate-only matrix.
          partition=$(make -s test-flake-partition)
          echo "$partition"
          echo "$partition" >> "$GITHUB_OUTPUT"
"""


def nix_unit_shards_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    strategy:
      fail-fast: false
      max-parallel: {job["maxParallel"]}
      matrix:
        check: ${{{{ fromJSON(needs.nix-unit-discover.outputs.checks) }}}}
    steps:
      - uses: {CHECKOUT}
        with:
          persist-credentials: false
{nix_setup_step(job, MATRIX_CHECK_SCOPE)}
      - name: {job["displayName"]}
        env:
          # The matrix value is data, not shell source. The driver validates it
          # against both the safe-name grammar and the discovered check set.
          D2B_NIX_UNIT_CHECK: ${{{{ matrix.check }}}}
          D2B_NIX_UNIT_JOBS: "1"
        run: make test-nix-unit"""


def nix_unit_rollup_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
    needs: {yaml_list(job["needs"])}
    if: always()
    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    steps:
      - uses: {CHECKOUT}
        with:
          persist-credentials: false
      - name: {job["displayName"]}
        run: |
          discover='${{{{ needs.nix-unit-discover.result }}}}'
          shards='${{{{ needs.nix-unit-shards.result }}}}'
          echo "nix-unit-discover=$discover  nix-unit-shards=$shards"
          if [ "$discover" = success ] && [ "$shards" = success ]; then
            echo "Every discovered Nix-unit shard passed."
          else
            echo "::error::Nix-unit gate failed (discover=$discover, shards=$shards)"
            exit 1
          fi"""


def flake_discover_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    outputs:
      evalchecks: ${{{{ steps.list.outputs.evalchecks }}}}
      realizedchecks: ${{{{ steps.list.outputs.realizedchecks }}}}
    steps:
      - uses: {CHECKOUT}
        with:
          persist-credentials: false
{nix_setup_step(job)}
      - id: list
        name: {job["displayName"]}
        run: |
          # One enumeration produces every dispatch class, so the names dropped
          # from the eval matrix are provably the names another lane realizes.
          # The tool fails closed on an empty enumeration or a realized name
          # that is not a discovered check.
          partition=$(make -s test-flake-partition)
          echo "$partition"
          echo "$partition" >> "$GITHUB_OUTPUT"
"""


def flake_x86_shards_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    strategy:
      fail-fast: false
      max-parallel: {job["maxParallel"]}
      matrix:
        check: ${{{{ fromJSON(needs.flake-eval-discover.outputs.evalchecks) }}}}
    steps:
      - uses: {CHECKOUT}
        with:
          persist-credentials: false
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
{nix_setup_step(job, MATRIX_CHECK_SCOPE)}
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


def flake_x86_realized_job(job: dict[str, Any]) -> str:
    """Renders the lane for flake checks whose shard builds rather than evaluates.

    These are minutes-long where an instantiate-only shard is seconds, so they
    get their own matrix instead of a slot in the bounded eval matrix. Measured
    on the run that motivated this split: the single realized check ran 15.9
    min but was dispatched last, so it did not start until 12.4 min into the
    run and set a ~29 min critical path on its own.

    No gdb step here. The eval lane installs it for the segfault retry path in
    test-flake.sh, and that path is unreachable for a realized shard - those
    fail hard on any nonzero status instead of retrying.

    The shard carries a targeted binary cache rather than a whole-store one.
    Measured on this tree, the single realized check has five direct build
    inputs and cache.nixos.org already serves three of them; only the two
    patched VMM packages must ever be built, and they export to about 30 MB of
    compressed NAR. Caching just those recovers the entire ~16 min compile for
    an entry small enough not to compete for the repository's ~10 GB budget -
    unlike the 4G-capped store cache the fixture job needs, which is why
    NIX_CACHED_JOBS deliberately does not extend to this lane.

    A stale entry is harmless: store paths are content-addressed, so a changed
    derivation has a changed output path, the restored entry cannot satisfy it,
    and the shard builds exactly as it does today. The key therefore only has
    to be good enough for a useful hit rate, and restore-keys are safe.
    """
    cache_key = (
        "d2b-realized-v1-${{ runner.os }}-${{ matrix.check }}-"
        "${{ hashFiles('flake.lock', 'flake.nix', 'pkgs/**') }}"
    )
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    strategy:
      fail-fast: false
      max-parallel: {job["maxParallel"]}
      matrix:
        check: ${{{{ fromJSON(needs.flake-eval-discover.outputs.realizedchecks) }}}}
    steps:
      - uses: {CHECKOUT}
        with:
          persist-credentials: false
{nix_setup_step(job, MATRIX_CHECK_SCOPE)}
      - name: Realized-check input cache
        uses: {CACHE}
        with:
          path: {REALIZED_CACHE_DIR}
          key: {cache_key}
          restore-keys: |
            d2b-realized-v1-${{{{ runner.os }}}}-${{{{ matrix.check }}}}-
      - name: Restore prebuilt check inputs
        # Best-effort: a miss costs the build this cache exists to avoid, and
        # can never produce a wrong result, so it must not fail the shard.
        env:
          D2B_FLAKE_CHECK: ${{{{ matrix.check }}}}
        run: bash tests/tools/realized-check-cache.sh import "$D2B_FLAKE_CHECK" {REALIZED_CACHE_DIR}
      - name: {job["displayName"]}
        # D2B_FLAKE_CHECK is passed via the step environment, NOT interpolated
        # into the shell command: a flake check attr name is PR-controlled, so
        # `D2B_FLAKE_CHECK='${{{{ matrix.check }}}}' ...` would be a shell-injection
        # vector. test-flake.sh additionally rejects names outside [A-Za-z0-9._-].
        env:
          D2B_FLAKE_CHECK: ${{{{ matrix.check }}}}
        run: make test-flake
      - name: Publish built check inputs
        env:
          D2B_FLAKE_CHECK: ${{{{ matrix.check }}}}
        run: bash tests/tools/realized-check-cache.sh export "$D2B_FLAKE_CHECK" {REALIZED_CACHE_DIR}"""


def flake_x86_outputs_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    steps:
      - uses: {CHECKOUT}
        with:
          persist-credentials: false
{nix_setup_step(job)}
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
      - uses: {CHECKOUT}
        with:
          persist-credentials: false
      - name: {job["displayName"]}
        run: |
          discover='${{{{ needs.flake-eval-discover.result }}}}'
          shards='${{{{ needs.flake-eval-x86.result }}}}'
          realized='${{{{ needs.flake-eval-x86-realized.result }}}}'
          outputs='${{{{ needs.flake-eval-x86-outputs.result }}}}'
          echo "flake-eval-discover=$discover  flake-eval-x86=$shards  flake-eval-x86-realized=$realized  flake-eval-x86-outputs=$outputs"
          if [ "$discover" = success ] && [ "$shards" = success ] \\
            && [ "$realized" = success ] && [ "$outputs" = success ]; then
            echo "All x86_64-linux flake checks + outputs passed."
          else
            echo "::error::x86_64 flake gate failed (discover=$discover, shards=$shards, realized=$realized, outputs=$outputs)"
            exit 1
          fi"""


def flake_aarch64_smoke_job(job: dict[str, Any]) -> str:
    return f"""  {job["ciJobId"]}:
{needs_line(job)}    runs-on: {job["runsOn"]}
    timeout-minutes: {job["timeoutMinutes"]}
    steps:
      - uses: {CHECKOUT}
        with:
          persist-credentials: false
{nix_setup_step(job)}
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
    advisory_needs = {
        need
        for need in needs
        if manifest["jobs"][need].get("enforcement") == "advisory"
    }
    lines = [
        f"  {rollup}:",
        f"    needs: {yaml_list(needs)}",
        "    if: always()",
        "    runs-on: ubuntu-latest",
        "    timeout-minutes: 5",
        "    steps:",
        f"      - uses: {CHECKOUT}",
        "        with:",
        "          persist-credentials: false",
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
        "          require_advisory_success() {",
        "            name=\"$1\"",
        "            result=\"$2\"",
        "            echo \"advisory:$name=$result (not an enforcing pass)\"",
        "            if [ \"$result\" != success ]; then",
        "              echo \"::error::required advisory $name did not complete successfully "
        "(result=$result)\"",
        "              failed=1",
        "            fi",
        "          }",
    ]
    for need in needs:
        expr = "${{ needs." + need + ".result }}"
        if need in advisory_needs:
            lines.append(f"          require_advisory_success {need} '{expr}'")
        elif need in allowed_skipped:
            lines.append(f"          allow_success_or_skipped {need} '{expr}'")
        else:
            lines.append(f"          require_success {need} '{expr}'")
    lines.extend(
        [
            "          if [ \"$failed\" -ne 0 ]; then",
            "            exit 1",
            "          fi",
            "          echo \"All generated enforcing Layer-1 jobs passed.\"",
        ]
    )
    if advisory_needs:
        advisory_names = ", ".join(need for need in needs if need in advisory_needs)
        lines.append(
            "          echo \"Required advisory jobs completed "
            f"(not enforcing passes): {advisory_names}\""
        )
    return "\n".join(lines)


RENDERERS = {
    "tier0": tier0_job,
    "simple": simple_job,
    "simple-nix": simple_nix_job,
    "changelog": changelog_job,
    "rust": rust_job,
    "rust-rollup": rust_rollup_job,
    "nix-unit-discover": nix_unit_discover_job,
    "nix-unit-shards": nix_unit_shards_job,
    "nix-unit-rollup": nix_unit_rollup_job,
    "flake-discover": flake_discover_job,
    "flake-x86-shards": flake_x86_shards_job,
    "flake-x86-realized": flake_x86_realized_job,
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
        rendered = renderer(job)
        if job.get("enforcement") == "advisory":
            header = f"  {job['ciJobId']}:\n"
            advisory_name = json.dumps(
                f"Advisory - non-enforcing - {job['displayName']}"
            )
            if not rendered.startswith(header):
                raise SystemExit(
                    f"{MANIFEST}: renderer for {job_id!r} emitted an unexpected job header"
                )
            rendered = rendered.replace(header, f"{header}    name: {advisory_name}\n", 1)
        rendered_jobs.append(rendered)
    rendered_jobs.append(check_rollup_job(manifest))
    template = TEMPLATE.read_text(encoding="utf-8")
    workflow = template.replace("{{ workflow_name }}", manifest["ci"]["workflowName"])
    workflow = workflow.replace("{{ jobs }}", "\n\n".join(rendered_jobs))
    permissions = "permissions:\n  contents: read\n"
    if workflow.count(permissions) != 1:
        raise SystemExit(f"{TEMPLATE}: expected one workflow permissions block")
    workflow = workflow.replace(
        permissions,
        permissions + f"\ndefaults:\n  run:\n    shell: {SCRUBBED_BASH}\n",
    )
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


def command_self_test(args: argparse.Namespace) -> int:
    return subprocess.run([sys.executable, str(SELF_TEST), *args.tests], cwd=ROOT).returncode


def normalize_ansi_escape_sequences(line: str) -> str:
    output: list[str] = []
    index = 0
    while index < len(line):
        if line[index] != "\x1b":
            output.append(line[index])
            index += 1
            continue

        output.append(" ")
        index += 1
        if index >= len(line):
            continue
        introducer = line[index]
        if introducer == "[":
            index += 1
            while index < len(line):
                final = ord(line[index])
                index += 1
                if 0x40 <= final <= 0x7E:
                    break
        elif introducer in "]PX^_":
            index += 1
            while index < len(line):
                if line[index] == "\x07":
                    index += 1
                    break
                if (
                    line[index] == "\x1b"
                    and index + 1 < len(line)
                    and line[index + 1] == "\\"
                ):
                    index += 2
                    break
                index += 1
        elif 0x40 <= ord(introducer) <= 0x5F:
            index += 1
    return "".join(output)


def redact_diagnostic_line(line: str) -> str:
    # The Rust redactor may not be built when this earliest Layer-1 phase fails.
    # Apply its repo/home placeholders and absolute-path fallback in-process
    # rather than making failure reporting depend on Cargo.
    line = normalize_ansi_escape_sequences(line)
    line = "".join(
        character if character == "\t" or character.isprintable() else " "
        for character in line
    )
    sensitive_roots = [
        (str(ROOT.resolve()), "<repo>"),
        (str(ROOT), "<repo>"),
    ]
    home = os.environ.get("HOME")
    if home:
        sensitive_roots.append((str(pathlib.Path(home).resolve()), "<home>"))
        sensitive_roots.append((home.rstrip("/") or "/", "<home>"))
    for root, replacement in sorted(set(sensitive_roots), key=lambda item: -len(item[0])):
        pattern = re.compile(PATH_START + re.escape(root) + ROOT_END)
        line = pattern.sub(
            lambda match: f"{match.group('prefix')}{replacement}",
            line,
        )
    return ABSOLUTE_PATH.sub(
        lambda match: f"{match.group('prefix')}<path>",
        line,
    )


def run_job(job_id: str, job: dict[str, Any]) -> int:
    target = job.get("makeTarget")
    if not target:
        raise RuntimeError(f"local job {job_id!r} has no makeTarget")
    # Every target the continuous-integration job runs must also run locally,
    # or `make check` is not the pull-request-equivalent gate it claims to be.
    targets = [target, *(step["makeTarget"] for step in job.get("extraMakeTargets", []))]
    env = os.environ.copy()
    env.update(job.get("localEnv", {}))
    log_dir = pathlib.Path(tempfile.mkdtemp(prefix=f"d2b-{job_id}."))
    log_path = log_dir / "output.log"
    print(f"==> {target} ({job.get('displayName', job_id)})", flush=True)
    failed_target = None
    returncode = 0
    with log_path.open("wb") as log:
        for one in targets:
            proc = subprocess.run(
                ["make", one], cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT
            )
            returncode = proc.returncode
            if returncode != 0:
                failed_target = one
                break
    if returncode == 0:
        # An advisory job may legitimately no-op, so reporting it as `ok`
        # would let a gate that did nothing count towards a green run.
        if job.get("enforcement") == "advisory":
            print(f"advisory: {target} (not an enforcing gate)", flush=True)
        else:
            print(f"ok: {target}", flush=True)
        if os.environ.get("D2B_CHECK_KEEP_LOGS") != "1":
            try:
                log_path.unlink()
                log_dir.rmdir()
            except OSError:
                pass
        return 0
    assert failed_target is not None
    print(
        f"FAIL: {failed_target} for Layer-1 job {job_id} "
        f"(exit {returncode}); full retained log: "
        f"{redact_diagnostic_line(str(log_path))}; redacted retained tail:",
        file=sys.stderr,
        flush=True,
    )
    try:
        lines = log_path.read_text(encoding="utf-8", errors="replace").splitlines()
        for line in lines[-200:]:
            print(redact_diagnostic_line(line), file=sys.stderr)
    except OSError as exc:
        detail = exc.strerror or "I/O error"
        print(
            f"could not read retained output for Layer-1 job {job_id}: {detail}",
            file=sys.stderr,
        )
    return returncode


def selected_phases(manifest: dict[str, Any], include_preflight: bool) -> list[dict[str, Any]]:
    phases = manifest["local"]["phases"]
    if include_preflight:
        return phases
    return [phase for phase in phases if phase["id"] != "preflight"]


def command_run_local(args: argparse.Namespace) -> int:
    manifest = load_manifest()
    jobs = manifest["jobs"]
    # Resolved in two steps rather than via os.environ.get's default argument:
    # the environment lookup is str | None, the manifest fallback is untyped,
    # and folding them together makes the argument to int() an optional that a
    # type checker rejects. A boolean-or fallback would type-check but would
    # also treat an explicit "0" as unset, silently substituting the default
    # instead of reaching the >= 1 rejection below.
    raw_max_jobs: object = os.environ.get("D2B_CHECK_JOBS")
    if raw_max_jobs is None:
        raw_max_jobs = manifest["local"].get("defaultJobs", 4)
    try:
        max_jobs = int(raw_max_jobs)  # type: ignore[call-overload]
    except (TypeError, ValueError):
        print("D2B_CHECK_JOBS must be an integer", file=sys.stderr)
        return 2
    if max_jobs < 1:
        print("D2B_CHECK_JOBS must be >= 1", file=sys.stderr)
        return 2
    include_preflight = not args.skip_preflight
    phases = selected_phases(manifest, include_preflight)
    for phase in phases:
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
    enforcing = [
        job_id
        for phase in phases
        for job_id in phase["jobs"]
        if jobs[job_id].get("enforcement") != "advisory"
    ]
    advisory = [
        job_id
        for phase in phases
        for job_id in phase["jobs"]
        if jobs[job_id].get("enforcement") == "advisory"
    ]
    summary = f"Layer-1 manifest runner OK: {len(enforcing)} enforcing job(s)"
    if advisory:
        summary += f", {len(advisory)} advisory ({', '.join(advisory)})"
    print(summary)
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    render = subparsers.add_parser("render-workflow", help="render the GitHub Actions workflow")
    render.add_argument("--write", action="store_true", help="write the rendered workflow in place")
    render.set_defaults(func=command_render_workflow)

    check = subparsers.add_parser("check-workflow", help="fail if the rendered workflow is stale")
    check.set_defaults(func=command_check_workflow)

    self_test = subparsers.add_parser("self-test", help="run Layer-1 runner regressions")
    self_test.add_argument("tests", nargs="*", help=argparse.SUPPRESS)
    self_test.set_defaults(func=command_self_test)

    run = subparsers.add_parser("run-local", help="run local Layer-1 phases from the manifest")
    run.add_argument("--skip-preflight", action="store_true", help="skip the preflight phase")
    run.set_defaults(func=command_run_local)

    args = parser.parse_args()
    return int(args.func(args))


if __name__ == "__main__":
    raise SystemExit(main())
