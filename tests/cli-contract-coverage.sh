#!/usr/bin/env bash
# Fail closed when docs/reference/cli-contract.md drifts
# away from the actual CLI parser/help surface.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
DOC=${DOC:-$ROOT/docs/reference/cli-contract.md}
NL_LOG=${NL_LOG:-$ROOT/.cli-contract-coverage.log}
export NL_LOG

# shellcheck source=lib.sh
. "$HERE/lib.sh"
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

cd "$ROOT"

if [ ! -f "$DOC" ]; then
  fail "cli-contract-coverage: required doc input missing"
  exit 1
fi

if [ -z "${NIXLING_CLI_CONTRACT_COVERAGE_IN_NIX_SHELL:-}" ] && ! command -v python3 >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "cli-contract-coverage: neither python3 nor nix is on PATH"
    exit 1
  fi
  log "  python3 not on PATH; re-entering via nix shell"
  export NIXLING_CLI_CONTRACT_COVERAGE_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" nixpkgs#python3 --command bash "$0" "$@"
fi

scratch=$(nl_mktemp .cli-contract-coverage.XXXXXX)
mkdir -p "$scratch/home" "$scratch/runtime"

bundle_root=$(nl_cli_smoke_bundle_tree)
system_fixture="$scratch/system-state.json"
host_fixture="$scratch/host-pass.json"
auth_fixture="$scratch/auth-launcher.json"
nl_write_system_state_fixture "$system_fixture"
nl_write_host_check_fixture_pass "$host_fixture" "$bundle_root"
nl_write_auth_status_fixture "$auth_fixture" launcher
native_bin=$(nl_cli_native_bin)
legacy_bin=$(nl_legacy_cli_bin)

python3 - "$DOC" "$native_bin" "$legacy_bin" "$bundle_root" "$system_fixture" "$host_fixture" "$auth_fixture" "$scratch/home" "$scratch/runtime" <<'PY'
import os
import re
import subprocess
import sys
from pathlib import Path

(
    doc_path,
    native_bin,
    legacy_bin,
    bundle_root,
    system_fixture,
    host_fixture,
    auth_fixture,
    home_dir,
    runtime_dir,
) = sys.argv[1:]

DOC = Path(doc_path).read_text(encoding="utf-8")
# console + audio {status,mic,speaker,off} are `rust-native shim`: the Rust CLI
# parses them natively and returns a typed `not-yet-implemented` envelope. They
# are not legacy-bash dispatches, so the `legacy-bash` help-output comparison
# below never applies to them.
EXPECTED_DISPOSITION = {
    "list": "rust-native",
    "vm start": "rust-native",
    "vm stop": "rust-native",
    "vm restart": "rust-native",
    "vm exec": "rust-native",
    "status": "rust-native",
    "status --check-bridges": "rust-native",
    "usb attach": "rust-native",
    "usb detach": "rust-native",
    "usb probe": "rust-native",
    "console": "rust-native shim",
    "audio status": "rust-native shim",
    "audio mic": "rust-native shim",
    "audio speaker": "rust-native shim",
    "audio off": "rust-native shim",
    "build": "rust-native",
    "switch": "rust-native",
    "boot": "rust-native",
    "test": "rust-native",
    "rollback": "rust-native",
    "generations": "rust-native",
    "gc": "rust-native",
    "store verify": "rust-native",
    "trust": "rust-native",
    "rotate-known-host": "rust-native",
    "keys list": "rust-native",
    "keys show": "rust-native",
    "keys rotate": "rust-native",
    "audit": "rust-native",
    "host check": "rust-native",
    "host prepare": "rust-native",
    "host destroy": "rust-native",
    "auth status": "rust-native",
}
JSON_SCHEMA = {
    "list": "list.schema.json",
    "status": "status.schema.json",
    "audit": "audit.schema.json",
    "host check": "host-check.schema.json",
    "auth status": "auth-status.schema.json",
    "store verify": "store-verify.schema.json",
    "vm exec": [
        "vm-exec-create.schema.json",
        "vm-exec-list.schema.json",
        "vm-exec-status.schema.json",
        "vm-exec-logs.schema.json",
        "vm-exec-kill.schema.json",
    ],
}
HELP_GROUPS = {
    "list": ["list"],
    "status": ["status", "status --check-bridges"],
    "vm exec": ["vm exec"],
    "keys list": ["keys list"],
    "keys show": ["keys show"],
    "usb attach": ["usb attach"],
    "usb detach": ["usb detach"],
    "usb probe": ["usb probe"],
    "store verify": ["store verify"],
    "audit": ["audit"],
    "host check": ["host check"],
    "auth status": ["auth status"],
}
FLAG_INVOCATIONS = {
    "list": {
        "--json": ["list", "--json"],
        "--human": ["list", "--human"],
    },
    "status": {
        "--json": ["status", "--vm", "corp-vm", "--json"],
        "--human": ["status", "--human"],
        "--vm": ["status", "--vm", "corp-vm", "--human"],
    },
    "status --check-bridges": {
        "--check-bridges": ["status", "--check-bridges"],
    },
    "vm exec": {
        "-d": ["vm", "exec", "-d", "corp-vm", "--", "sleep", "60"],
        "--detach": ["vm", "exec", "--detach", "corp-vm", "--", "sleep", "60"],
        "-i": ["vm", "exec", "-i", "-t", "corp-vm", "--", "bash"],
        "--interactive": ["vm", "exec", "--interactive", "--tty", "corp-vm", "--", "bash"],
        "-t": ["vm", "exec", "-t", "corp-vm", "--", "bash"],
        "--tty": ["vm", "exec", "--tty", "corp-vm", "--", "bash"],
        "--env": ["vm", "exec", "--env", "KEY=VALUE", "corp-vm", "--", "env"],
        "--cwd": ["vm", "exec", "--cwd", "/home/alice", "corp-vm", "--", "pwd"],
        "--json": ["vm", "exec", "corp-vm", "logs", "exec-1", "--json"],
        "--human": ["vm", "exec", "corp-vm", "logs", "exec-1", "--human"],
        "--stdout-offset": ["vm", "exec", "corp-vm", "logs", "exec-1", "--stdout-offset=4"],
        "--stderr-offset": ["vm", "exec", "corp-vm", "logs", "exec-1", "--stderr-offset=8"],
        "--max-len": ["vm", "exec", "corp-vm", "logs", "exec-1", "--max-len=4096"],
    },
    "keys list": {
        "--json": ["keys", "list", "--json"],
        "--human": ["keys", "list", "--human"],
    },
    "keys show": {
        "--json": ["keys", "show", "corp-vm", "--json"],
        "--human": ["keys", "show", "corp-vm", "--human"],
    },
    "usb attach": {
        "--dry-run": ["usb", "attach", "corp-vm", "1-2", "--dry-run"],
        "--apply": ["usb", "attach", "corp-vm", "1-2", "--apply"],
        "--json": ["usb", "attach", "corp-vm", "1-2", "--dry-run", "--json"],
        "--human": ["usb", "attach", "corp-vm", "1-2", "--dry-run", "--human"],
    },
    "usb detach": {
        "--dry-run": ["usb", "detach", "corp-vm", "1-2", "--dry-run"],
        "--apply": ["usb", "detach", "corp-vm", "1-2", "--apply"],
        "--json": ["usb", "detach", "corp-vm", "1-2", "--dry-run", "--json"],
        "--human": ["usb", "detach", "corp-vm", "1-2", "--dry-run", "--human"],
    },
    "usb probe": {
        "--json": ["usb", "probe", "--json"],
        "--human": ["usb", "probe", "--human"],
    },
    "store verify": {
        "--repair": ["store", "verify", "corp-vm", "--repair", "--json"],
        "--json": ["store", "verify", "corp-vm", "--json"],
        "--human": ["store", "verify", "corp-vm", "--human"],
    },
    "audit": {
        "--strict": ["audit", "--strict", "--json"],
        "--json": ["audit", "--json"],
        "--human": ["audit", "--human"],
    },
    "host check": {
        "--read-only": ["host", "check", "--read-only", "--human"],
        "--strict": ["host", "check", "--strict", "--read-only", "--human"],
        "--json": ["host", "check", "--read-only", "--json"],
        "--human": ["host", "check", "--read-only", "--human"],
    },
    "auth status": {
        "--json": ["auth", "status", "--test-uid", "1000", "--json"],
        "--human": ["auth", "status", "--test-uid", "1000", "--human"],
    },
}

section_matches = list(re.finditer(r"^### `([^`]+)`\n", DOC, flags=re.M))
sections = {}
for index, match in enumerate(section_matches):
    name = match.group(1)
    start = match.end()
    end = section_matches[index + 1].start() if index + 1 < len(section_matches) else DOC.find("## Dispatch capability table")
    sections[name] = DOC[start:end]

try:
    dispatch_block = DOC.split("## Dispatch capability table", 1)[1]
except IndexError:
    dispatch_block = ""
dispatch_rows = dict(re.findall(r"\| `([^`]+)` \| `([^`]+)` \|", dispatch_block))

missing = []
required_labels = [
    "**Synopsis:**",
    "**Flags**",
    "**Arguments**",
    "**Exit codes**",
    "**Human example**",
]


def extract_block(section: str, start_label: str, end_label: str | None) -> str:
    try:
        after = section.split(start_label, 1)[1]
    except IndexError:
        return ""
    if end_label is None:
        return after
    return after.split(end_label, 1)[0]


def parse_doc_flags(section: str) -> set[str]:
    flags_block = extract_block(section, "**Flags**", "**Arguments**")
    documented = set()
    for line in flags_block.splitlines():
        stripped = line.strip()
        if not stripped.startswith("|"):
            continue
        cells = [cell.strip() for cell in stripped.strip("|").split("|")]
        if not cells:
            continue
        first = cells[0]
        if first == "_(none)_":
            continue
        for token in re.findall(r"`([^`]+)`", first):
            if token.startswith("-"):
                documented.add(token)
    return documented


for command, disposition in EXPECTED_DISPOSITION.items():
    section = sections.get(command)
    if section is None:
        missing.append(f"missing section: {command}")
        continue
    for label in required_labels:
        if label not in section:
            missing.append(f"{command}: missing {label}")
    disposition_match = re.search(r"\*\*W2 disposition:\*\* `([^`]+)`", section)
    if disposition_match is not None:
        if disposition_match.group(1) != disposition:
            missing.append(f"{command}: disposition mismatch")
    else:
        for label in ["**Status**", "**Native**", "**Bash**"]:
            if label not in section:
                missing.append(f"{command}: missing {label}")
    if command not in dispatch_rows:
        missing.append(f"dispatch table missing row: {command}")
    elif dispatch_rows[command] != disposition:
        missing.append(f"dispatch table disposition mismatch for {command}: {dispatch_rows[command]}")
    exit_block = extract_block(section, "**Exit codes**", "**Human example**")
    if not re.search(r"\| `\d+` \|", exit_block):
        missing.append(f"{command}: missing exit-code rows")
    if re.search(r"\*\*Human example\*\*\s+```text\n.+?\n```", section, flags=re.S) is None:
        missing.append(f"{command}: missing human example code fence")
    if command in JSON_SCHEMA:
        schema_names = JSON_SCHEMA[command]
        if isinstance(schema_names, str):
            schema_names = [schema_names]
        for schema_name in schema_names:
            if schema_name not in section:
                missing.append(f"{command}: missing schema link {schema_name}")
        if re.search(r"\*\*`--json` example\*\*.+?```json\n.+?\n```", section, flags=re.S) is None:
            missing.append(f"{command}: missing --json example block")

base_env = os.environ.copy()
base_env.update(
    {
        "HOME": home_dir,
        "XDG_RUNTIME_DIR": runtime_dir,
        "NO_COLOR": "1",
        "NIXLING_LEGACY_BASH_OPT_IN": "1",
        "NIXLING_LEGACY_CLI": legacy_bin,
        "NIXLING_MANIFEST_PATH": str(Path(bundle_root) / "vms.json"),
        "NIXLING_BUNDLE_PATH": str(Path(bundle_root) / "bundle.json"),
        "NIXLING_PUBLIC_SOCKET": str(Path(runtime_dir) / "missing.sock"),
        "NIXLING_TEST_SYSTEM_STATE_JSON": system_fixture,
        "NIXLING_HOST_CHECK_FIXTURE": host_fixture,
        "NIXLING_AUTH_STATUS_FIXTURE": auth_fixture,
        "NIXLING_TEST_LAUNCHER_UIDS": "1000",
        "NIXLING_AUDIT_TESTMODE_KVM_MODE": "660",
    }
)
Path(home_dir).mkdir(parents=True, exist_ok=True)
Path(runtime_dir).mkdir(parents=True, exist_ok=True)


def run_command(argv: list[str], env: dict[str, str]) -> tuple[int, str]:
    proc = subprocess.run(argv, capture_output=True, text=True, env=env)
    return proc.returncode, proc.stdout + proc.stderr


def parse_help_flags(output: str) -> set[str]:
    flags = set()
    in_options = False
    for line in output.splitlines():
        stripped = line.strip()
        if stripped == "Options:":
            in_options = True
            continue
        if not in_options:
            continue
        if stripped.endswith(":") and stripped != "Options:" and not stripped.startswith("-"):
            break
        for token in re.findall(r"(?<!\w)(--[a-z0-9][a-z0-9-]*|-\w)\b", line):
            if token in {"-h", "--help"}:
                continue
            flags.add(token)
    return flags


for help_group, grouped_commands in HELP_GROUPS.items():
    documented_flags = set()
    for command in grouped_commands:
        documented_flags |= parse_doc_flags(sections[command])
    rc, output = run_command([native_bin, *help_group.split(), "--help"], base_env)
    if "Usage:" not in output:
        missing.append(f"{help_group}: --help did not render usage text (rc={rc})")
        continue
    actual_flags = parse_help_flags(output)
    if help_group == "vm exec":
        for token in ["--stdout-offset", "--stderr-offset", "--max-len"]:
            if token in output:
                actual_flags.add(token)
    if actual_flags != documented_flags:
        missing.append(
            f"{help_group}: help flags {sorted(actual_flags)} != documented {sorted(documented_flags)}"
        )

for command in HELP_GROUPS.values():
    for section_name in command:
        documented_flags = parse_doc_flags(sections[section_name])
        invocation_map = FLAG_INVOCATIONS.get(section_name, {})
        for flag in sorted(documented_flags):
            args = invocation_map.get(flag)
            if args is None:
                missing.append(f"{section_name}: no acceptance probe configured for {flag}")
                continue
            rc, probe_output = run_command([native_bin, *args], base_env)
            usage_allowed = section_name == "vm exec" and flag in {
                "-i",
                "--interactive",
                "-t",
                "--tty",
            }
            usage_like = (
                "Usage:" in probe_output
                or "unexpected argument" in probe_output
                or "unrecognized" in probe_output
                or "unknown option" in probe_output
            )
            if rc == 2 and usage_like and not usage_allowed:
                missing.append(f"{section_name}: documented flag {flag} was rejected with usage exit 2")

for command, disposition in EXPECTED_DISPOSITION.items():
    if disposition != "legacy-bash":
        continue
    args = command.split() + ["--help"]
    shim_rc, shim_output = run_command([native_bin, *args], base_env)
    if shim_rc == 2 and "Usage:" not in shim_output:
        continue
    legacy_rc, legacy_output = run_command([legacy_bin, *args], base_env)
    if shim_rc != legacy_rc or shim_output != legacy_output:
        missing.append(
            f"{command}: shim legacy dispatch mismatch (shim rc={shim_rc}, legacy rc={legacy_rc})"
        )

if missing:
    for item in missing:
        print(item, file=sys.stderr)
    sys.exit(1)
PY

ok "cli-contract-coverage: docs and CLI parser/help surfaces stay aligned"

# ---------------------------------------------------------------------
# Closed-table coverage for
# tests/golden/cli-output/host-{check,prepare,destroy,install}-*.{txt,json}
# enforcing:
#   * every row in the closed CLI error-code table (plan.md
#     §" CLI contract docs + per-error golden coverage", §2691-2839)
#     has a paired .txt + .json golden;
#   * every paired .json carries the seven mandated envelope fields
#     (kind / code / exit_code / what_was_checked / observed_state /
#     remediation / docs_anchor);
#   * no orphan .txt/.json: every host-* golden under tests/golden/
#     cli-output/ is listed in the closed table.
# ---------------------------------------------------------------------
GOLDEN_DIR=${GOLDEN_DIR:-$ROOT/tests/golden/cli-output}

python3 - "$GOLDEN_DIR" <<'PY'
import json
import re
import sys
from pathlib import Path

golden_dir = Path(sys.argv[1])

# Closed CLI table (verb, code). Must stay in lockstep with
# tests/fixtures/gen-w3-cli-goldens.py. Updating either side without
# the other will fail this gate.
W3_ROWS = {
    # host check — Tier-all (and Tier-0 conditional rows tagged below)
    ("host-check", "cgroup-delegation-refused"),
    ("host-check", "cgroup-v2-unified-not-present"),
    ("host-check", "cgroup-controllers-missing"),
    ("host-check", "cgroup-kill-on-ancestor-refused"),
    ("host-check", "ifname-too-long"),
    ("host-check", "ifname-collision"),
    ("host-check", "ipv6-sysctl-drift"),
    ("host-check", "nm-managed-foreign-conflict"),
    ("host-check", "nm-reload-failed"),
    ("host-check", "foreign-nft-rule-shadows-nixling"),
    ("host-check", "firewall-coexistence-mismatch"),
    ("host-check", "host-modules-locked"),
    ("host-check", "modprobe-denied-not-in-matrix"),
    ("host-check", "minijail-too-old"),
    ("host-check", "ch-net-handoff-not-supported"),
    ("host-check", "runner-shape-drift"),
    ("host-check", "single-writer-conflict"),
    ("host-check", "tier-0-legacy-uses-nixos-module"),
    ("host-check", "host-lan-cidr-ambiguous"),
    # host prepare --apply
    ("host-prepare", "cgroup-delegation-refused"),
    ("host-prepare", "route-preflight-no-default-route"),
    ("host-prepare", "route-preflight-foreign-default-route"),
    ("host-prepare", "dnsmasq-not-bound"),
    ("host-prepare", "path-safety-violation"),
    ("host-prepare", "nm-reload-failed"),
    ("host-prepare", "bridge-port-flag-drift"),
    ("host-prepare", "nft-foreign-rule-flush-attempted"),
    ("host-prepare", "firewall-coexistence-mismatch"),
    ("host-prepare", "tier-0-legacy-uses-nixos-module"),
    ("host-prepare", "single-writer-conflict"),
    ("host-prepare", "legacy-no-prepare-apply"),
    # host destroy --apply
    ("host-destroy", "vm-still-running-refused"),
    ("host-destroy", "tier-0-legacy-uses-nixos-module"),
    ("host-destroy", "legacy-no-destroy-apply"),
    # host install
    ("host-install", "not-yet-implemented"),
    # Inherited onboarding rows on host check
    ("host-check", "daemon-down"),
    ("host-check", "socket-perms-wrong"),
    ("host-check", "missing-group"),
    ("host-check", "unsupported-kernel"),
    ("host-check", "no-kvm"),
    ("host-check", "no-cgroup-v2"),
    ("host-check", "nftables-conflict"),
    ("host-check", "hardlink-fs-mismatch"),
    ("host-check", "manifest-skew"),
    ("host-check", "profile-rejects-root"),
    ("host-check", "seccomp-denial"),
    ("host-check", "tap-creation-denied"),
    ("host-check", "stale-lock"),
}

REQUIRED_FIELDS = {
    "kind",
    "code",
    "exit_code",
    "what_was_checked",
    "observed_state",
    "remediation",
    "docs_anchor",
}

# Stable docs anchor format: docs/reference/error-codes.md#<code>
ANCHOR_RE = re.compile(r"^docs/reference/error-codes\.md#[a-z0-9-]+$")

violations = []

# Coverage: every row in the closed table must have paired goldens.
for verb, code in sorted(W3_ROWS):
    stem = f"{verb}-{code}"
    txt = golden_dir / f"{stem}.txt"
    js = golden_dir / f"{stem}.json"
    if not txt.exists():
        violations.append(f"missing human golden: {txt.relative_to(golden_dir.parent.parent)}")
    if not js.exists():
        violations.append(f"missing JSON golden: {js.relative_to(golden_dir.parent.parent)}")
        continue
    try:
        env = json.loads(js.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        violations.append(f"{js.name}: invalid JSON ({exc})")
        continue
    missing_fields = REQUIRED_FIELDS - env.keys()
    if missing_fields:
        violations.append(
            f"{js.name}: JSON envelope missing required field(s): {sorted(missing_fields)}"
        )
    if env.get("code") != code:
        violations.append(
            f"{js.name}: envelope `code` is {env.get('code')!r}, expected {code!r}"
        )
    anchor = env.get("docs_anchor", "")
    if not ANCHOR_RE.match(anchor):
        violations.append(
            f"{js.name}: docs_anchor {anchor!r} does not match docs/reference/error-codes.md#<code>"
        )
    if not isinstance(env.get("exit_code"), int):
        violations.append(f"{js.name}: exit_code must be an integer")

# Orphan gate: every host-{check,prepare,destroy,install}-*.{txt,json}
# in the golden tree must correspond to a closed-table row.
known = {f"{verb}-{code}" for verb, code in W3_ROWS}
host_verb_pattern = re.compile(r"^(host-check|host-prepare|host-destroy|host-install)-(.+)\.(txt|json)$")
for path in sorted(golden_dir.iterdir()):
    if not path.is_file():
        continue
    match = host_verb_pattern.match(path.name)
    if not match:
        continue
    verb, code, _ext = match.groups()
    if (verb, code) not in W3_ROWS:
        violations.append(
            f"orphan golden: {path.name} has no row in the W3 closed CLI error-code table"
        )
    _ = known  # silence linter

if violations:
    for item in violations:
        print(item, file=sys.stderr)
    sys.exit(1)
PY

ok "cli-contract-coverage: closed golden table (48 rows × {.txt, .json}) is complete and orphan-free"
