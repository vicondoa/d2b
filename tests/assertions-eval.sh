#!/usr/bin/env bash
# tests/assertions-eval.sh — eval-time-assertion regression tests
# (consolidated harness).
#
# Pre-, this gate invoked `nix-instantiate --eval --strict`
# 31 times, once per case. Each invocation re-booted the NixOS
# module-system evaluator from cold, totalling ~32 min wall + ~150 G
# /nix/store growth per gate run.
#
# Collapses the 25 simple `run_assertion_test` cases into a
# single batched `nix-instantiate --eval --strict --json` against
# `tests/eval-cases/assertions.nix`. Each case is read out of the
# resulting JSON and per-case asserted in shell. The remaining 6
# cases (3 success cases, 3 feature-gated skip cases) keep their
# original per-case eval because they need either complex skip logic
# or a non-`assertion`-list shape.
#
# Run via:
#   tests/assertions-eval.sh
# Wired into tests/static.sh.

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=$(dirname "$HERE")

# shellcheck source=lib.sh
. "$HERE/lib.sh"

SCRATCH=$(nl_mktemp .assertions-eval.XXXXXX)

PASS=0
FAIL=0
SKIP=0

export EVAL_EXPR_FILE=""
EVAL_OUT_FILE=""
EVAL_ERR_FILE=""

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; PASS=$((PASS+1)); }
fail() { log "  FAIL: $*"; FAIL=$((FAIL+1)); }
skip() { log "  SKIP: $*"; SKIP=$((SKIP+1)); }

show_stderr_tail() {
  local file="$1"
  log "    --- stderr (tail) ---"
  tail -15 "$file" | sed 's/^/      /' >&2
}

stderr_contains_all() {
  local file="$1"
  shift
  local needle
  for needle in "$@"; do
    if ! grep -q -F -- "$needle" "$file"; then
      return 1
    fi
  done
}

# ---------------------------------------------------------------------------
# Legacy `mk_expr` / `run_eval_json` helpers — used by the batched
# harness's focused fallback path (for the 3 throw cases that still
# need a per-case stderr capture) AND by the 6 non-batched cases at
# the tail. switches the default forcing expression away from
# `system.build.toplevel.drvPath` and onto `nixos.config.assertions`
# so eval-only assertion probes stop materializing a toplevel closure.
# ---------------------------------------------------------------------------

ASSERTIONS_FORCE_EXPR=$(cat <<'EOF'
let
  assertions = nixos.config.assertions;
  assertionBools = builtins.map (a: a.assertion) assertions;
  failingMessages = builtins.map (a: a.message) (
    builtins.filter (a: !a.assertion) assertions
  );
in
  builtins.deepSeq assertionBools {
    assertionsTotal = builtins.length assertions;
    inherit failingMessages;
  }
EOF
)

mk_expr() {
  local override="$1"
  local system="${2:-x86_64-linux}"
  local body="${3:-$ASSERTIONS_FORCE_EXPR}"
  # Per-case eval shares the SAME minimal-evalModules evaluator as the
  # batched harness (tests/eval-cases/shared.nix `mkEval`): nixling's
  # own modules + misc/assertions.nix + namespace sinks, NOT a full
  # `nixosSystem`. That keeps batch and per-case semantics identical and
  # drops each per-case eval from ~28s (1,370 baseModules) to ~1s. The
  # shared `baseModule` is byte-identical to the previously-inlined one.
  cat <<EOF
let
  shared = import $ROOT/tests/eval-cases/shared.nix { flakeRoot = $ROOT; };
  inherit (shared) lib;
  nixos = shared.mkEval {
    system = "$system";
    override = $override;
  };
in
  $body
EOF
}

run_eval_json() {
  local name="$1" override="$2" body="$3" system="${4:-x86_64-linux}"
  local expr_file out_file err_file
  expr_file="$SCRATCH/$name.nix"
  out_file="$SCRATCH/$name.json"
  err_file="$SCRATCH/$name.stderr"
  mk_expr "$override" "$system" "$body" > "$expr_file"
  EVAL_EXPR_FILE="$expr_file"
  EVAL_OUT_FILE="$out_file"
  EVAL_ERR_FILE="$err_file"
  if nix-instantiate --eval --strict --json \
       --expr "$(cat "$expr_file")" \
       > "$out_file" 2> "$err_file"; then
    return 0
  fi
  return 1
}

# ---------------------------------------------------------------------------
# Batched harness
# ---------------------------------------------------------------------------
#
# Run ONE nix-instantiate --eval --strict --json against the
# consolidated case file and stash the JSON for per-case assertion.
#
# Per-case contract from tests/eval-cases/shared.nix:
#   { name = {
#       expectedSubstring : string;
#       kind              : "expect-failure" | "expect-success";
#       evalSucceeded     : bool;     # tryEval (deepSeq config.assertions)
#       throwMessage      : string;   # populated by fallback path
#       failingMessages   : [string]; # config.assertions where .assertion == false
#       assertionsTotal   : int;
#       warnings          : [string];
#     }; ... }

BATCH_FILE="$SCRATCH/assertions-batch.json"
BATCH_ERR="$SCRATCH/assertions-batch.stderr"

log '==> tests/assertions-eval.sh (batched harness)'
log '  --> nix-instantiate --eval --strict --json tests/eval-cases/assertions.nix'

# The batch attribute is the whole imported attrset; the wrapper does
# not pre-filter to attribute names so the evaluator forces every
# case's `failingMessages` / `evalSucceeded` payload exactly once.
if ! nix-instantiate --eval --strict --json --expr \
    "import $ROOT/tests/eval-cases/assertions.nix { flakeRoot = $ROOT; }" \
    > "$BATCH_FILE" 2> "$BATCH_ERR"; then
  log "  FAIL: batch eval of tests/eval-cases/assertions.nix did not produce JSON"
  show_stderr_tail "$BATCH_ERR"
  exit 1
fi

if ! jq -e 'type == "object"' "$BATCH_FILE" >/dev/null; then
  log "  FAIL: batch JSON output was not an attrset"
  show_stderr_tail "$BATCH_ERR"
  exit 1
fi

batch_case_keys=$(jq -r 'keys[]' "$BATCH_FILE")
batch_case_count=$(printf '%s\n' "$batch_case_keys" | wc -l)
log "  --> batch produced $batch_case_count case results"

# Fallback: cases where the batch eval threw before assertions were
# computable need a focused per-case nix-instantiate to capture the
# real error text. Used for option-type-check throws (e.g.
# waylandUser = null on a graphics VM, or aarch64 platform-gate
# throws that fire from inside host.nix before assertions are
# reachable). We still force `nixos.config.assertions`; the only
# difference vs the batch path is the narrower per-case override.
declare -A FALLBACK_OVERRIDE
declare -A FALLBACK_SYSTEM

FALLBACK_OVERRIDE["graphics-without-wayland-user"]='({ ... }: {
  nixling.site.waylandUser = null;
  nixling.vms.corp-vm.graphics.enable = true;
})'
FALLBACK_SYSTEM["graphics-without-wayland-user"]="x86_64-linux"

FALLBACK_OVERRIDE["platform-gate-graphics-aarch64"]='({ ... }: {
  nixling.vms.corp-vm.graphics.enable = true;
})'
FALLBACK_SYSTEM["platform-gate-graphics-aarch64"]="aarch64-linux"

FALLBACK_OVERRIDE["platform-gate-audio-aarch64"]='({ ... }: {
  nixling.vms.corp-vm.audio.enable = true;
})'
FALLBACK_SYSTEM["platform-gate-audio-aarch64"]="aarch64-linux"

fallback_per_case() {
  local case_name="$1" expected_substr="$2"
  local override="${FALLBACK_OVERRIDE[$case_name]:-}"
  local system="${FALLBACK_SYSTEM[$case_name]:-x86_64-linux}"
  if [ -z "$override" ]; then
    fail "$case_name: batch eval threw but no fallback override registered"
    return 1
  fi
  local expr_file out_file err_file
  expr_file="$SCRATCH/$case_name.fallback.nix"
  out_file="$SCRATCH/$case_name.fallback.out"
  err_file="$SCRATCH/$case_name.fallback.stderr"
  mk_expr "$override" "$system" > "$expr_file"
  if nix-instantiate --eval --strict --show-trace \
       --expr "$(cat "$expr_file")" \
       > "$out_file" 2> "$err_file"; then
    fail "$case_name: fallback eval unexpectedly succeeded (batch reported evalSucceeded=false)"
    return 1
  fi
  if grep -q -F -- "$expected_substr" "$err_file"; then
    ok "$case_name (found via fallback: '$expected_substr')"
    return 0
  fi
  fail "$case_name: fallback eval failed but stderr did not match '$expected_substr'"
  show_stderr_tail "$err_file"
  return 1
}

# Per-case assertion. Reads the case's batch JSON entry, applies the
# `expect-failure` / `expect-success` contract.
assert_batched_case() {
  local case_name="$1"
  local case_json
  case_json=$(jq -c --arg n "$case_name" '.[$n]' "$BATCH_FILE")
  if [ -z "$case_json" ] || [ "$case_json" = "null" ]; then
    fail "$case_name: case not registered in tests/eval-cases/assertions.nix"
    return 1
  fi

  local kind expected ev_ok fail_count
  kind=$(printf '%s' "$case_json" | jq -r '.kind')
  expected=$(printf '%s' "$case_json" | jq -r '.expectedSubstring')
  ev_ok=$(printf '%s' "$case_json" | jq -r '.evalSucceeded')
  fail_count=$(printf '%s' "$case_json" | jq -r '.failingMessages | length')

  case "$kind" in
    expect-failure)
      if [ "$ev_ok" = "true" ]; then
        # Eval succeeded; we need at least one failing assertion whose
        # message contains the expected substring.
        if [ "$fail_count" = "0" ]; then
          fail "$case_name: eval succeeded with no failing assertions; expected a failing assertion containing '$expected'"
          return 1
        fi
        local matched
        matched=$(printf '%s' "$case_json" \
          | jq -r --arg s "$expected" '.failingMessages | map(select(contains($s))) | length')
        if [ "$matched" != "0" ]; then
          ok "$case_name (found in failingMessages: '$expected')"
          return 0
        fi
        fail "$case_name: $fail_count failing assertion(s) fired but none contained '$expected'"
        log "    --- failingMessages ---"
        printf '%s' "$case_json" | jq -r '.failingMessages[]' | sed 's/^/      /' >&2
        return 1
      else
        # Eval threw before assertions could be read; fall back to a
        # per-case focused eval that surfaces the throw text.
        fallback_per_case "$case_name" "$expected"
      fi
      ;;
    expect-success)
      if [ "$ev_ok" != "true" ]; then
        fail "$case_name: expected eval to succeed (no failing assertions) but eval threw"
        fallback_per_case "$case_name" "$expected" || true
        return 1
      fi
      if [ "$fail_count" != "0" ]; then
        fail "$case_name: expected zero failing assertions; got $fail_count"
        log "    --- failingMessages ---"
        printf '%s' "$case_json" | jq -r '.failingMessages[]' | sed 's/^/      /' >&2
        return 1
      fi
      ok "$case_name (no failing assertions; eval succeeded)"
      ;;
    *)
      fail "$case_name: unknown case kind '$kind'"
      return 1
      ;;
  esac
}

# 25 batched cases — exact 1:1 with the cases attribute set in
# tests/eval-cases/assertions.nix.
BATCHED_CASES=(
  private-key-in-authorized-keys
  graphics-without-wayland-user
  wayland-user-missing
  vm-name-invalid
  vm-name-reserved-launcher
  vm-name-reserved-sys-prefix
  env-name-invalid
  env-name-too-long
  vm-env-missing
  vm-env-disabled
  vm-index-duplicate
  static-ip-and-env-mutually-exclusive
  lansubnet-wrong-mask
  uplinksubnet-wrong-mask
  lansubnet-nonzero-host
  overlap-containment
  env-vs-host-overlap
  state-dir-override-rejected
  store-state-dir-override-rejected
  allow-east-west-requires-site-ack
  platform-gate-graphics-aarch64
  platform-gate-audio-aarch64
  graphics-with-autostart
  graphics-xwayland-unsupported
  audit-without-observability
  observability-reserved-cid
  principal-uid-collision
)

for case_name in "${BATCHED_CASES[@]}"; do
  assert_batched_case "$case_name"
done

# ---------------------------------------------------------------------------
# Non-batched cases: success-shape probes that read custom JSON values
# (not config.assertions) AND feature-gated observability cases with
# complex skip logic that depends on whether downstream features have
# landed in this worktree. These keep the legacy per-case
# `nix-instantiate --eval` invocation; the time cost is small relative
# to the original 31-case wall (~5 of 9 + tmpdir/default-path).
# ---------------------------------------------------------------------------

# Auto-obs feature gating helper (unchanged from legacy).
feature_auto_obs_ready() {
  local vm_name="$1"
  local probe_name="__probe-auto-obs-${vm_name//[^a-zA-Z0-9]/-}"
  local override body
  override=$(cat <<EOF
({ ... }: {
  nixling.observability.enable = true;
  nixling.observability.vmName = "$vm_name";
})
EOF
)
  body=$(cat <<EOF
{
  hasObsEnv = builtins.hasAttr "obs" nixos.config.nixling.envs;
  hasObsVm = builtins.hasAttr "$vm_name" nixos.config.nixling.vms;
}
EOF
)
  run_eval_json "$probe_name" "$override" "$body" || return 1
  jq -e '.hasObsEnv and .hasObsVm' "$EVAL_OUT_FILE" >/dev/null 2>&1
}

feature_transport_vsock_ready() {
  run_eval_json \
    "__probe-transport-vsock" \
    '({ ... }: {
       nixling.observability.enable = true;
       nixling.vms.corp-vm.observability.enable = true;
     })' \
    'builtins.hasAttr "nixling-otel-relay@" nixos.config.systemd.services' \
    || return 1
  jq -e '. == true' "$EVAL_OUT_FILE" >/dev/null 2>&1
}

# ---------------------------------------------------------------------------
# Success-case probes (eval must succeed with a particular shape) and
# the 3 feature-gated observability cases.
# ---------------------------------------------------------------------------

test_tmpdir_tmpfiles_rule() {
  if ! run_eval_json \
      "tmpdir-tmpfiles-rule" \
      '({ ... }: { })' \
      '{
         tmpDir = toString nixos.config.nixling.site.tmpDir;
         hasTmpRule = builtins.any
           (r: lib.hasPrefix ("D " + toString nixos.config.nixling.site.tmpDir + " ") r)
           nixos.config.systemd.tmpfiles.rules;
       }'; then
    fail 'tmpdir-tmpfiles-rule: eval failed'
    show_stderr_tail "$EVAL_ERR_FILE"
    return 1
  fi
  if jq -e '.tmpDir == "/var/lib/nixling/tmp" and .hasTmpRule == true' "$EVAL_OUT_FILE" >/dev/null 2>&1; then
    ok 'tmpdir-tmpfiles-rule'
  else
    fail 'tmpdir-tmpfiles-rule: tmpDir or tmpfiles cleanup rule missing'
    return 1
  fi
}

test_default_path_literals_are_allowed() {
  if run_eval_json \
      "default-path-literals-allowed" \
      '({ ... }: {
         nixling.site.stateDir = /var/lib/nixling;
         nixling.store.stateDir = /var/lib/nixling/vms;
       })' \
      'builtins.length (builtins.filter (a: !a.assertion) nixos.config.assertions)'; then
    if jq -e '. == 0' "$EVAL_OUT_FILE" >/dev/null 2>&1; then
      ok 'default-path-literals-allowed'
    else
      fail 'default-path-literals-allowed: reserved-path assertions still fired for explicit default literals'
    fi
  else
    fail 'default-path-literals-allowed: explicit default path literals should not trip reserved-path assertions'
    show_stderr_tail "$EVAL_ERR_FILE"
  fi
}

test_disabled_vm_audit_is_ignored() {
  if run_eval_json \
      "disabled-vm-audit-is-ignored" \
      '({ ... }: {
         nixling.vms.disabled = {
           enable = false;
           audit.enable = true;
         };
       })' \
      'builtins.length (builtins.filter (a: !a.assertion) nixos.config.assertions)'; then
    if jq -e '. == 0' "$EVAL_OUT_FILE" >/dev/null 2>&1; then
      ok 'disabled-vm-audit-is-ignored'
    else
      fail 'disabled-vm-audit-is-ignored: disabled VM tripped an assertion despite enable=false'
    fi
  else
    fail 'disabled-vm-audit-is-ignored: eval failed'
    show_stderr_tail "$EVAL_ERR_FILE"
  fi
}

test_observability_cid_collision() {
  local override err_file
  override=$(cat <<'EOF'
({ lib, ... }: {
  nixling.observability.enable = true;
  nixling.envs.aaa = {
    lanSubnet = "10.30.0.0/24";
    uplinkSubnet = "198.51.100.0/30";
  };
  nixling.envs.bbb = {
    lanSubnet = "10.31.0.0/24";
    uplinkSubnet = "198.18.0.0/30";
  };
  nixling.vms.corp-vm.env = lib.mkForce "aaa";
  nixling.vms.corp-vm.index = lib.mkForce 110;
  nixling.vms.corp-vm.observability.enable = true;
  nixling.vms.other-vm = {
    enable = true;
    env = "bbb";
    index = 10;
    ssh.user = "alice";
    observability.enable = true;
    config = {
      networking.hostName = lib.mkDefault "other-vm";
      users.users.alice = { isNormalUser = true; uid = 1000; };
    };
  };
})
EOF
)

  if run_eval_json \
      'observability-cid-collision' \
      "$override" \
      "$ASSERTIONS_FORCE_EXPR"; then
    if jq -e '([.failingMessages[]?] | join("\n")) as $m | ($m | contains("CID")) and ($m | contains("corp-vm")) and ($m | contains("other-vm"))' "$EVAL_OUT_FILE" >/dev/null 2>&1; then
      ok "observability-cid-collision (found in failingMessages: 'CID', 'corp-vm', 'other-vm')"
      return 0
    fi
    if feature_transport_vsock_ready; then
      fail 'observability-cid-collision: eval succeeded but the CID collision was not reported in failingMessages'
      return 1
    fi
    skip 'observability-cid-collision: TODO post-integration — transport-vsock relay/assertions have not landed in this worktree'
    return 0
  fi

  err_file="$EVAL_ERR_FILE"
  if stderr_contains_all "$err_file" 'CID' 'corp-vm' 'other-vm'; then
    ok "observability-cid-collision (found in stderr: 'CID', 'corp-vm', 'other-vm')"
    return 0
  fi

  if feature_transport_vsock_ready; then
    fail 'observability-cid-collision: eval failed but neither failingMessages nor stderr named the colliding VMs/CID'
    show_stderr_tail "$err_file"
    return 1
  fi

  skip 'observability-cid-collision: TODO post-integration — transport-vsock CID-collision assertion has not landed in this worktree'
}

test_observability_vmname_reserved_prefix_exempt() {
  local override
  override=$(cat <<'EOF'
({ ... }: {
  nixling.observability.enable = true;
  nixling.observability.vmName = "sys-custom-obs";
})
EOF
)

  if run_eval_json \
      'observability-vmname-reserved-prefix-exempt' \
      "$override" \
      'builtins.hasAttr "sys-custom-obs" nixos.config.nixling.vms'; then
    if jq -e '. == true' "$EVAL_OUT_FILE" >/dev/null 2>&1; then
      ok 'observability-vmname-reserved-prefix-exempt'
      return 0
    fi
    if feature_auto_obs_ready 'sys-custom-obs'; then
      fail 'observability-vmname-reserved-prefix-exempt: auto-declared VM missing despite observability.enable = true'
      return 1
    fi
    skip 'observability-vmname-reserved-prefix-exempt: TODO post-integration — auto-obs-vm has not landed in this worktree'
    return 0
  fi

  if feature_auto_obs_ready 'sys-custom-obs'; then
    fail 'observability-vmname-reserved-prefix-exempt: eval failed even though cfg.vmName should be exempt from the reserved sys- prefix rule'
    show_stderr_tail "$EVAL_ERR_FILE"
    return 1
  fi

  skip 'observability-vmname-reserved-prefix-exempt: TODO post-integration — auto-obs-vm has not landed in this worktree'
}

test_observability_vmname_collision() {
  local override
  override=$(cat <<'EOF'
({ lib, ... }: {
  nixling.observability.enable = true;
  nixling.observability.vmName = "obs-stack";
  nixling.vms.obs-stack = {
    ssh.user = "alice";
    config = {
      users.users.alice = { isNormalUser = true; uid = 1000; };
    };
  };
})
EOF
)

  if run_eval_json \
      'observability-vmname-extension-allowed' \
      "$override" \
      'builtins.length (builtins.filter (a: !a.assertion) nixos.config.assertions)'; then
    if jq -e '. == 0' "$EVAL_OUT_FILE" >/dev/null 2>&1; then
      ok 'observability-vmname-extension-allowed (consumer can extend auto-declared obs VM)'
      return 0
    fi
    fail 'observability-vmname-extension-allowed: failing assertions fired despite consumer extension being permitted'
    return 1
  fi

  if feature_auto_obs_ready 'obs-stack'; then
    fail 'observability-vmname-extension-allowed: eval failed but consumer-side extension should be permitted'
    show_stderr_tail "$EVAL_ERR_FILE"
    return 1
  fi

  skip 'observability-vmname-extension-allowed: TODO post-integration — auto-obs-vm has not landed in this worktree'
}

# ---------------------------------------------------------------------------

test_tmpdir_tmpfiles_rule
test_default_path_literals_are_allowed
test_disabled_vm_audit_is_ignored
test_observability_cid_collision
test_observability_vmname_reserved_prefix_exempt
test_observability_vmname_collision

log "==> assertions-eval: $PASS passed, $FAIL failed, $SKIP skipped"
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
