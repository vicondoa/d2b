#!/usr/bin/env bash
# Layer-1 static checks for the nixling framework.
#
# Runs in seconds; catches:
#   - syntax errors in any nixling .nix file
#   - missing imports / option-type mismatches (via dry-build)
#   - `flake check` failures (eval of every package output)
#   - per-VM closure attributes failing to evaluate
#
# Exits non-zero on the first failure. Safe to run on any commit.
#
# Usage:
#   tests/static.sh

set -euo pipefail

# Derive ROOT from the script's own location (one dir above tests/) so
# `tests/static.sh` works from any clone of the repo, not just the
# maintainer's /etc/nixos checkout. Override with ROOT=/path/to/clone.
HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

# Scope a safe.directory entry for $ROOT to libgit2 (used by
# `nix flake check` and `nix eval`) without mutating the caller's git
# config. Pattern is the same as security-baseline.sh::nl_eval_attr.
# Required when running inside a sandbox where $ROOT is owned by a
# different uid than the caller.
_STATIC_GITCFG=$(mktemp -d -t nixling-static-gitcfg.XXXXXX)
trap 'rm -rf -- "$_STATIC_GITCFG"' EXIT
install -d -m 0700 "$_STATIC_GITCFG/git"
printf "[safe]\n\tdirectory = %s\n" "$ROOT" > "$_STATIC_GITCFG/git/config"
export XDG_CONFIG_HOME="$_STATIC_GITCFG"
export GIT_CONFIG_COUNT=1
export GIT_CONFIG_KEY_0=safe.directory
export GIT_CONFIG_VALUE_0="$ROOT"

# shellcheck source=lib.sh
. "$HERE/lib.sh"

log "==> Layer 1: parse + eval"

cd "$ROOT"

# W2 layout: nixos-modules/ + components/. Old paths
# (modules/nixling/router.nix, modules/nixling/vms.nix,
# modules/nixling/audio.nix, modules/nixling/audio-host.nix,
# modules/nixling/entra-id.nix) are gone:
#   * router.nix renamed to net.nix in W1.
#   * vms.nix is NOT lifted into the public flake (consumers declare
#     their own nixling.vms.<name> bindings).
#   * audio.nix split into components/audio/{guest,host}.nix.
#   * entra-id.nix moved to the sibling nixos-entra-id flake.
# Consumer-specific `vms/<name>.nix` paths are excluded — they only
# exist on the maintainer's host. The loop below skips any entry that
# isn't present on disk so the gate stays useful for the public flake
# AND for consumer trees that still carry workload VM definitions.
NL_FILES=(
  nixos-modules/default.nix
  nixos-modules/options.nix
  nixos-modules/assertions.nix
  nixos-modules/lib.nix
  nixos-modules/base.nix
  nixos-modules/host.nix
  nixos-modules/host-users.nix
  nixos-modules/host-wrapper.nix
  nixos-modules/host-polkit.nix
  nixos-modules/host-sidecars.nix
  nixos-modules/host-activation.nix
  nixos-modules/host-known-hosts.nix
  nixos-modules/host-audit.nix
  nixos-modules/network.nix
  nixos-modules/net.nix
  nixos-modules/store.nix
  nixos-modules/cli.nix
  nixos-modules/components/graphics.nix
  nixos-modules/components/tpm.nix
  nixos-modules/components/usbip.nix
  nixos-modules/components/home-manager.nix
  nixos-modules/components/audio/guest.nix
  nixos-modules/components/audio/host.nix
  tests/smoke-eval-aarch64.nix
  tests/smoke-eval-graphics.nix
  tests/smoke-eval-home-manager.nix
  tests/smoke-eval-extraspecialargs.nix
  tests/smoke-eval-tpm.nix
  flake.nix
)
log "--> nix-instantiate --parse"
for f in "${NL_FILES[@]}"; do
  if [ ! -f "$ROOT/$f" ]; then
    log "  skip (not present): $f"
    continue
  fi
  if nix-instantiate --parse "$ROOT/$f" >/dev/null 2>&1; then
    ok "parse: $f"
  else
    fail "parse: $f"
  fi
done

log "--> shellcheck --severity=warning on all nixling shell scripts"
mapfile -t SH_FILES < <(
  find "$ROOT/tests" "$ROOT/scripts" \
    -maxdepth 1 -name '*.sh' -type f 2>/dev/null | sort
)
if [ "${#SH_FILES[@]}" -eq 0 ]; then
  fail "shellcheck: no .sh files found under tests/ or scripts/"
fi
SC_FAILED=0
for f in "${SH_FILES[@]}"; do
  if nix-shell -p shellcheck --run "shellcheck --severity=warning -x $(printf '%q' "$f")" >/dev/null 2>&1; then
    ok "shellcheck: $f"
  else
    fail "shellcheck: $f"
    nix-shell -p shellcheck --run "shellcheck --severity=warning -x $(printf '%q' "$f")" 2>&1 \
      | head -20 >&2 || true
    SC_FAILED=$((SC_FAILED+1))
  fi
done

log "--> nix flake check --no-build --all-systems"
if nix flake check "$ROOT" --no-build --all-systems 2>&1 | tail -20 >> "$NL_LOG"; then
  ok "flake check"
else
  fail "flake check"
fi

# W3b H9 — smoke-eval gate. Forces a full module-system evaluation of
# a minimal consumer-style nixosSystem importing nixling.nixosModules.default.
# This catches regressions the bare `flake check` misses, e.g. lazy
# strings inside writeShellApplication that don't fire until the
# module is instantiated against a real config.
log "--> tests/smoke-eval.nix"
if [ -f "$ROOT/tests/smoke-eval.nix" ]; then
  if nix-instantiate --eval --strict --expr \
      "let f = import $ROOT/tests/smoke-eval.nix; r = f {}; in r.drvPath" \
      >/dev/null 2>&1; then
    ok "smoke-eval"
  else
    fail "smoke-eval"
    nix-instantiate --eval --strict --expr \
      "let f = import $ROOT/tests/smoke-eval.nix; r = f {}; in r.drvPath" 2>&1 \
      | tail -20 >&2 || true
  fi
fi

# W5 H9 — graphics-VM manifest regression. Mirrors smoke-eval.nix
# but with one graphics-enabled VM and a strict `deepSeq` on
# `config.nixling.manifest` so cli.nix's `vmLaunchScript` readOnly
# access path is exercised. This is the exact codepath that surfaced
# Spec correction #29 (default+readOnly+config collision); the
# headless smoke-eval missed it because manifest reads only fire
# from the graphics codepath.
log "--> tests/smoke-eval-graphics.nix"
if [ -f "$ROOT/tests/smoke-eval-graphics.nix" ]; then
  if nix-instantiate --eval --strict --expr \
      "let f = import $ROOT/tests/smoke-eval-graphics.nix; r = f {}; in r.drvPath" \
      >/dev/null 2>&1; then
    ok "smoke-eval-graphics"
  else
    fail "smoke-eval-graphics"
    nix-instantiate --eval --strict --expr \
      "let f = import $ROOT/tests/smoke-eval-graphics.nix; r = f {}; in r.drvPath" 2>&1 \
      | tail -20 >&2 || true
  fi
fi

# v0.1.0 H4 — Home Manager component regression. Mirrors smoke-eval.nix
# but flips `homeManager.enable = true` on the workload VM. The HM
# component imports `inputs.home-manager.nixosModules.home-manager`,
# which silently broke before v0.1.0 H4 added the `home-manager`
# flake input. Any future drop of the input or rename of the HM
# component would surface here.
log "--> tests/smoke-eval-home-manager.nix"
if [ -f "$ROOT/tests/smoke-eval-home-manager.nix" ]; then
  if nix-instantiate --eval --strict --expr \
      "let f = import $ROOT/tests/smoke-eval-home-manager.nix; r = f {}; in r.drvPath" \
      >/dev/null 2>&1; then
    ok "smoke-eval-home-manager"
  else
    fail "smoke-eval-home-manager"
    nix-instantiate --eval --strict --expr \
      "let f = import $ROOT/tests/smoke-eval-home-manager.nix; r = f {}; in r.drvPath" 2>&1 \
      | tail -20 >&2 || true
  fi
fi

# v0.1.6 H1 — `nixling.site.extraSpecialArgs` propagation regression.
# Spec correction #30: the v0.1.1 change wired
# `cfg.site.extraSpecialArgs` into the per-VM `specialArgs` in
# nixos-modules/host.nix:165. This test synthesizes a workload VM
# whose `config` module consumes a positional `sentinel` argument
# defined in `extraSpecialArgs`, then forces an env.etc value that
# pins `sentinel == "ok"`. A regression that drops the merge fails
# the eval at "called without required argument 'sentinel'".
log "--> tests/smoke-eval-extraspecialargs.nix"
if [ -f "$ROOT/tests/smoke-eval-extraspecialargs.nix" ]; then
  if nix-instantiate --eval --strict --expr \
      "let f = import $ROOT/tests/smoke-eval-extraspecialargs.nix; r = f {}; in r.drvPath" \
      >/dev/null 2>&1; then
    ok "smoke-eval-extraspecialargs"
  else
    fail "smoke-eval-extraspecialargs"
    nix-instantiate --eval --strict --expr \
      "let f = import $ROOT/tests/smoke-eval-extraspecialargs.nix; r = f {}; in r.drvPath" 2>&1 \
      | tail -20 >&2 || true
  fi
fi

# v0.1.6 Test-H6 — swtpm parent-dir ACL regression. Spec correction
# #35 (v0.1.4): nixling-<vm>-swtpm needs `--x` on
# /var/lib/nixling/vms/<vm> to traverse into its `swtpm/` leaf. The
# activation snippet only emits the grant for graphics+TPM VMs, so
# the smoke eval declares a `graphics.enable = true; tpm.enable =
# true` VM and asserts the literal `setfacl -m
# "u:nixling-<vm>-swtpm:--x" /var/lib/nixling/vms/<vm>` fragment
# appears in `system.activationScripts.nixlingVmStatePerms.text`.
log "--> tests/smoke-eval-tpm.nix"
if [ -f "$ROOT/tests/smoke-eval-tpm.nix" ]; then
  if nix-instantiate --eval --strict --expr \
      "let f = import $ROOT/tests/smoke-eval-tpm.nix; r = f {}; in r.drvPath" \
      >/dev/null 2>&1; then
    ok "smoke-eval-tpm"
  else
    fail "smoke-eval-tpm"
    nix-instantiate --eval --strict --expr \
      "let f = import $ROOT/tests/smoke-eval-tpm.nix; r = f {}; in r.drvPath" 2>&1 \
      | tail -20 >&2 || true
  fi
fi

# Phase 4 multi-arch gate. Sister of smoke-eval.nix that cross-evaluates
# a headless workload VM config on aarch64-linux. The refactor plan
# requires headless VMs (no graphics, no audio) to evaluate clean on
# aarch64 even though the cloud-hypervisor + crosvm pipeline is
# x86_64-only. Surface a regression here the moment an unconditional
# x86_64-only reference creeps back into the eval graph.
log "--> tests/smoke-eval-aarch64.nix"
if [ -f "$ROOT/tests/smoke-eval-aarch64.nix" ]; then
  if nix-instantiate --eval --strict --expr \
      "let f = import $ROOT/tests/smoke-eval-aarch64.nix; r = f {}; in r.drvPath" \
      >/dev/null 2>&1; then
    ok "smoke-eval-aarch64"
  else
    fail "smoke-eval-aarch64"
    nix-instantiate --eval --strict --expr \
      "let f = import $ROOT/tests/smoke-eval-aarch64.nix; r = f {}; in r.drvPath" 2>&1 \
      | tail -20 >&2 || true
  fi
fi

# W5 H1 — net VM systemd.network regression test. Verifies that
# net VMs neutralize the `10-eth-dhcp` catch-all from base.nix so
# the per-MAC `10-uplink`/`10-lan` static config actually applies.
log "--> tests/net-vm-network-eval.sh"
if [ -x "$ROOT/tests/net-vm-network-eval.sh" ]; then
  if bash "$ROOT/tests/net-vm-network-eval.sh" >/dev/null 2>&1; then
    ok "net-vm-network-eval"
  else
    fail "net-vm-network-eval"
    bash "$ROOT/tests/net-vm-network-eval.sh" 2>&1 | tail -20 >&2 || true
  fi
fi

# W3b H10 — eval-time-assertion regression tests. Exercises each
# eval-time invariant in the nixling option schema (CIDR shape,
# CIDR overlap, key validation, waylandUser presence, etc.) by
# constructing synthetic configs and asserting nix-instantiate
# FAILS with the expected error substring. Catches the case where
# a future refactor silently drops one of the assertions.
log "--> tests/assertions-eval.sh"
if [ -x "$ROOT/tests/assertions-eval.sh" ]; then
  if bash "$ROOT/tests/assertions-eval.sh" >/dev/null 2>&1; then
    ok "assertions-eval"
  else
    fail "assertions-eval"
    bash "$ROOT/tests/assertions-eval.sh" 2>&1 | tail -40 >&2 || true
  fi
fi

# v0.1.6 Test-H3/H4 + SWArch-M10 — autostart wiring invariants.
# Verifies (a) `systemd.services."nixling@<vm>"` is NEVER materialized
# as a per-instance attr (template-only; per-VM unit file would lack
# ExecStart/ExecStop), (b) autostart=true VMs go through
# `multi-user.target.wants -> nixling@<vm>.service`, and (c)
# `microvms.target.wants` is `[]` after SWArch-M10 (single autostart
# path; no duplicate microvm@<vm> direct-pull).
log "--> tests/autostart-wiring-eval.sh"
if [ -x "$ROOT/tests/autostart-wiring-eval.sh" ]; then
  if bash "$ROOT/tests/autostart-wiring-eval.sh" >/dev/null 2>&1; then
    ok "autostart-wiring-eval"
  else
    fail "autostart-wiring-eval"
    bash "$ROOT/tests/autostart-wiring-eval.sh" 2>&1 | tail -20 >&2 || true
  fi
fi

# v0.1.6 Test-H7 — restart-policy regression. Spec correction #37
# (v0.1.5): every per-VM lifecycle service in the framework MUST
# carry `restartIfChanged = false` OR equivalent
# `unitConfig.X-RestartIfChanged = false`. Six services in scope
# (nixling@ template, microvm@ template, per-VM virtiofsd, swtpm,
# snd, gpu). Synthesizes a graphics+audio+TPM workload VM so every
# per-VM sidecar materialises in one eval.
log "--> tests/restart-policy-eval.sh"
if [ -x "$ROOT/tests/restart-policy-eval.sh" ]; then
  if bash "$ROOT/tests/restart-policy-eval.sh" >/dev/null 2>&1; then
    ok "restart-policy-eval"
  else
    fail "restart-policy-eval"
    bash "$ROOT/tests/restart-policy-eval.sh" 2>&1 | tail -20 >&2 || true
  fi
fi

# Phase 5 (W4) — JSON manifest contract gate. Renders the manifest
# from the same smoke-eval consumer config and validates it against
# docs/reference/manifest-schema.json (JSON Schema Draft 2020-12). Catches:
#   - manifest.nix's computed values drifting from the documented
#     types (e.g. a refactor returning null for a field declared str),
#   - manifest.nix and docs/reference/manifest-schema.json drifting on field
#     names or required-vs-optional status,
#   - reserved `_*`-prefixed keys with an unexpected shape.
#
# Validation runs under nix-shell with python3 + jsonschema; nothing
# else in the test harness depends on Python today, but the jsonschema
# package is small (~50KB) and pulled lazily on first run.
log "--> manifest JSON contract (docs/reference/manifest-schema.json)"
if [ -f "$ROOT/docs/reference/manifest-schema.json" ] && [ -f "$ROOT/tests/smoke-eval.nix" ]; then
  _MANIFEST_DIR=$(mktemp -d -p "$ROOT" .manifest-gate.XXXXXX)
  _MANIFEST_JSON="$_MANIFEST_DIR/manifest.json"

  # Render the manifest's JSON text via the smoke-eval consumer config.
  # _manifestPkg.text is the bare `builtins.toJSON …` output we ship.
  _RENDER_OK=0
  if nix-instantiate --eval --strict --json --expr "
    let
      pkgs = import <nixpkgs> {};
      lib = pkgs.lib;
      flake = builtins.getFlake (toString $ROOT);
      nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
      nixos = nixosSystem {
        system = builtins.currentSystem;
        modules = [
          flake.nixosModules.default
          ({ lib, ... }: {
            boot.loader.grub.enable = false;
            boot.loader.systemd-boot.enable = false;
            boot.initrd.includeDefaultModules = false;
            fileSystems.\"/\" = { device = \"tmpfs\"; fsType = \"tmpfs\"; };
            environment.etc.\"machine-id\".text = \"00000000000000000000000000000000\";
            system.stateVersion = \"25.11\";
            users.users.alice = { isNormalUser = true; uid = 1000; };
            nixling.site = { waylandUser = \"alice\"; launcherUsers = [ \"alice\" ]; yubikey.enable = false; };
            nixling.envs.work = { lanSubnet = \"10.20.0.0/24\"; uplinkSubnet = \"192.0.2.0/30\"; };
            nixling.vms.corp-vm = {
              enable = true; env = \"work\"; index = 10; ssh.user = \"alice\";
              config = {
                networking.hostName = lib.mkDefault \"corp-vm\";
                users.users.alice = { isNormalUser = true; uid = 1000; };
              };
            };
          })
        ];
      };
    in nixos.config.nixling._manifestPkg.text
  " 2>/dev/null | jq -r . > "$_MANIFEST_JSON"; then
    _RENDER_OK=1
    ok "manifest-contract: rendered smoke manifest"
  else
    fail "manifest-contract: could not render smoke manifest"
  fi

  if [ "$_RENDER_OK" = "1" ]; then
    # 1. Schema syntactically valid JSON.
    if jq . "$ROOT/docs/reference/manifest-schema.json" >/dev/null 2>&1; then
      ok "manifest-contract: schema JSON syntax"
    else
      fail "manifest-contract: docs/reference/manifest-schema.json is not valid JSON"
    fi

    # 2. Manifest validates against schema (JSON Schema Draft 2020-12).
    #    Also asserts the schema is itself valid Draft 2020-12.
    if nix-shell -p "python3.withPackages (ps: [ ps.jsonschema ])" --run "
        python3 - <<PYEOF
import json, sys
import jsonschema
schema = json.load(open('$ROOT/docs/reference/manifest-schema.json'))
data = json.load(open('$_MANIFEST_JSON'))
jsonschema.Draft202012Validator.check_schema(schema)
validator = jsonschema.Draft202012Validator(schema)
errors = list(validator.iter_errors(data))
if errors:
    for e in errors:
        print('VALIDATION:', '/'.join(map(str, e.absolute_path)) or '<root>', '->', e.message, file=sys.stderr)
    sys.exit(1)
PYEOF
      " >/dev/null 2>&1; then
      ok "manifest-contract: smoke manifest validates against docs/reference/manifest-schema.json"
    else
      fail "manifest-contract: smoke manifest fails JSON Schema validation"
      nix-shell -p "python3.withPackages (ps: [ ps.jsonschema ])" --run "
          python3 - <<PYEOF
import json, sys
import jsonschema
schema = json.load(open('$ROOT/docs/reference/manifest-schema.json'))
data = json.load(open('$_MANIFEST_JSON'))
validator = jsonschema.Draft202012Validator(schema)
for e in validator.iter_errors(data):
    print('VALIDATION:', '/'.join(map(str, e.absolute_path)) or '<root>', '->', e.message, file=sys.stderr)
PYEOF
        " 2>&1 | tail -20 >&2 || true
    fi

    # 3. Cross-check: every per-VM field in the smoke manifest must be
    #    declared in the schema's $defs.vmEntry.required list. Catches
    #    the case where manifest.nix gains a new field but the schema
    #    isn't updated. (Schema-required fields missing from the
    #    manifest are caught by the Draft 2020-12 validation above.)
    _SCHEMA_REQUIRED=$(jq -r '.["$defs"].vmEntry.required[]' "$ROOT/docs/reference/manifest-schema.json" | sort -u)
    _MANIFEST_FIELDS=$(jq -r '
      [ .[] | select((type=="object") and (has("name"))) | keys[] ] | unique | .[]
    ' "$_MANIFEST_JSON" | sort -u)
    _UNDOC_FIELDS=$(comm -23 <(printf '%s\n' "$_MANIFEST_FIELDS") <(printf '%s\n' "$_SCHEMA_REQUIRED"))
    if [ -z "$_UNDOC_FIELDS" ]; then
      ok "manifest-contract: all manifest fields documented in schema"
    else
      fail "manifest-contract: undocumented per-VM fields in manifest: $(echo "$_UNDOC_FIELDS" | tr '\n' ' ')"
    fi

    # 4. _manifest.manifestVersion must be present and >= 1 (Phase 5
    #    locked v1 as the first documented schema).
    _RENDERED_VERSION=$(jq -r '._manifest.manifestVersion // empty' "$_MANIFEST_JSON")
    if [ -n "$_RENDERED_VERSION" ] && [ "$_RENDERED_VERSION" -ge 1 ]; then
      ok "manifest-contract: _manifest.manifestVersion = $_RENDERED_VERSION (>= 1)"
    else
      fail "manifest-contract: _manifest.manifestVersion missing or < 1"
    fi

    # 5. md ↔ json drift detection. The prose schema doc carries a
    #    "Per-VM entry" table whose first column is the field name.
    #    Every field documented in that table must appear in the JSON
    #    Schema's $defs.vmEntry.properties keys, and vice versa. Catches
    #    the case where the .md and .json are edited out of step (e.g.
    #    a field added to the JSON Schema but forgotten in the prose
    #    walkthrough's table).
    _SCHEMA_PROPS=$(jq -r '.["$defs"].vmEntry.properties | keys[]' "$ROOT/docs/reference/manifest-schema.json" | sort -u)
    # The per-VM-entry table lives between the "## Per-VM entry" header
    # and the next "### " sub-section header. Extract its first column
    # (the field name) by:
    #   - keeping only lines starting with `| \``,
    #   - dropping the table-header separator (the `|---` line is
    #     captured by the same prefix filter, then dropped by the awk
    #     pattern below).
    _MD_FIELDS=$(awk '
      /^## Per-VM entry$/ {in_section=1; next}
      in_section && /^### / {in_section=0}
      in_section && /^\| `[a-zA-Z]/ {
        # First column lives between the first pair of backticks.
        if (match($0, /`[^`]+`/)) {
          print substr($0, RSTART+1, RLENGTH-2)
        }
      }
    ' "$ROOT/docs/reference/manifest-schema.md" | sort -u)
    _MD_ONLY=$(comm -23 <(printf '%s\n' "$_MD_FIELDS") <(printf '%s\n' "$_SCHEMA_PROPS"))
    _SCHEMA_ONLY=$(comm -13 <(printf '%s\n' "$_MD_FIELDS") <(printf '%s\n' "$_SCHEMA_PROPS"))
    if [ -z "$_MD_ONLY" ] && [ -z "$_SCHEMA_ONLY" ]; then
      ok "manifest-contract: docs/reference/manifest-schema.{md,json} field inventories match"
    else
      [ -n "$_MD_ONLY" ] && fail "manifest-contract: in manifest-schema.md but missing from manifest-schema.json: $(echo "$_MD_ONLY" | tr '\n' ' ')"
      [ -n "$_SCHEMA_ONLY" ] && fail "manifest-contract: in manifest-schema.json but missing from manifest-schema.md: $(echo "$_SCHEMA_ONLY" | tr '\n' ' ')"
    fi
  fi

  rm -rf -- "$_MANIFEST_DIR"
fi

# The remaining gates evaluate a concrete consumer flake's
# `nixosConfigurations.<NL_HOST_CONFIG>` (default: `desktop`). On a fresh
# clone of the public framework flake, there is no host config — those
# gates simply skip with a SKIP line. On the maintainer's host (or any
# consumer who passes `NL_HOST_CONFIG=<their-host>`), they run as before.
NL_HOST_CONFIG=${NL_HOST_CONFIG:-desktop}
if nix eval --raw "$ROOT#nixosConfigurations.$NL_HOST_CONFIG.config.system.build.toplevel" >/dev/null 2>&1; then
  _HAS_HOST_CONFIG=1
else
  _HAS_HOST_CONFIG=0
  log "  SKIP: per-VM closure eval / dry-build / audio host-flake checks (no nixosConfigurations.$NL_HOST_CONFIG in $ROOT)"
fi

if [ "$_HAS_HOST_CONFIG" = "1" ]; then
  log "--> per-VM closure eval (.#nixling-<vm>)"
  # Enumerate VM names from the manifest baked into the CLI. The manifest
  # is exposed via `nixling status` (one VM per line under "vms:"), but
  # the cheapest source is direct nix eval.
  mapfile -t VMS < <(
    nix eval --json \
      "$ROOT#nixosConfigurations.$NL_HOST_CONFIG.config.nixling.vms" 2>/dev/null \
      | jq -r 'keys[]'
  )
  if [ "${#VMS[@]}" -eq 0 ]; then
    fail "no VMs declared (manifest empty?)"
  fi
  for vm in "${VMS[@]}"; do
    if nix eval --raw "$ROOT#nixling-$vm.outPath" >/dev/null 2>&1; then
      ok "eval: nixling-$vm"
    else
      fail "eval: nixling-$vm"
    fi
  done

  log "--> nixos-rebuild dry-build"
  if sudo -A nixos-rebuild dry-build --flake "$ROOT#$NL_HOST_CONFIG" >/dev/null 2>&1; then
    ok "dry-build"
  else
    fail "dry-build"
  fi
fi

# -----------------------------------------------------------------------------
# Audio: cheap eval assertions that the audio component is wired
# correctly. We don't enable audio.enable on any VM by default, so most
# of these are presence-of-option checks; for an end-to-end run flip a
# VM's audio.enable = true and re-run.
# -----------------------------------------------------------------------------
log "--> audio component"

if [ "$_HAS_HOST_CONFIG" = "1" ]; then

# 1. The shared systemd-user template unit must be present in the
#    rendered system.
if sudo -A nixos-rebuild build --flake "$ROOT#$NL_HOST_CONFIG" --no-link 2>/dev/null \
     | head -1 >/dev/null; then
  : # nothing — just trigger the build cache
fi

# 2. The audio.enable option must exist on every VM submodule.
if nix eval --raw "$ROOT#nixosConfigurations.$NL_HOST_CONFIG.options.nixling.vms.type.getSubOptions.x.audio.enable.declarations" \
     >/dev/null 2>&1 \
   || nix eval --json "$ROOT#nixosConfigurations.$NL_HOST_CONFIG.config.nixling.vms" 2>/dev/null \
     | jq -e '.[] | has("audio")' >/dev/null 2>&1; then
  ok "audio.enable option declared on nixling.vms.<name>"
else
  fail "audio.enable option missing on nixling.vms.<name>"
fi

SYS=$(nix eval --raw "$ROOT#nixosConfigurations.$NL_HOST_CONFIG.config.system.build.toplevel" 2>/dev/null) || SYS=""
if [ -n "$SYS" ]; then
  # v1 ships a host-side PipeWire client.conf.d stream rule that
  # null-targets vhost-device-sound's INPUT direction when nixling.mic
  # is "off" (and OUTPUT when nixling.speaker is "off") so it doesn't
  # auto-link to host devices uninvited. Note: this is a PipeWire
  # client.conf.d file, NOT a WirePlumber rule — see the placement-
  # notes block in audio-host.nix.
  #
  # security-r8-audio-6: the match key shifted from broad
  # `node.name=vhost-device-sound` + `application.name=~nixling-.*` to
  # per-direction custom props (`nixling.mic`, `nixling.speaker`) so
  # the rule fires ONLY when the corresponding direction is OFF. When
  # mic=on we WANT auto-routing; the old broad rule blocked it
  # forever regardless of audio-state.json.
  PW_RULE="$SYS/etc/pipewire/client.conf.d/90-nixling.conf"
  if [ -e "$PW_RULE" ] \
     && grep -q '"nixling.mic":[[:space:]]*"off"' "$PW_RULE" \
     && grep -q '"nixling.speaker":[[:space:]]*"off"' "$PW_RULE" \
     && grep -q '"target.object":[[:space:]]*"-1"' "$PW_RULE" \
     && grep -q 'stream.rules' "$PW_RULE" \
     && grep -q '"node.dont-fallback":[[:space:]]*true' "$PW_RULE" \
     && grep -q '"node.linger":[[:space:]]*true' "$PW_RULE"; then
    ok "pipewire client stream-rule installed: per-direction nixling.{mic,speaker}=off → target=-1 + dont-fallback + linger"
  else
    fail "pipewire client stream-rule missing or malformed at /etc/pipewire/client.conf.d/90-nixling.conf"
  fi
  if [ -e "$SYS/etc/wireplumber/wireplumber.conf.d/90-nixling.conf" ]; then
    fail "stale wireplumber rule present — should have moved to pipewire client.conf.d"
  else
    ok "no stale wireplumber.conf.d/90-nixling.conf (moved to pipewire client.conf.d)"
  fi
  # Phase 4 C3: nixling-<vm>-snd.service is now a per-VM system service (not user).
  SYS_UNITS=$(find -L "$SYS" -path '*systemd/system*' -name 'nixling-*-snd.service' -print -quit 2>/dev/null || true)
  if [ -n "$SYS_UNITS" ]; then
    ok "nixling-<vm>-snd.service unit(s) present in system closure (system service)"
  else
    fail "no nixling-<vm>-snd.service unit in system closure"
  fi
fi

fi  # end: _HAS_HOST_CONFIG

log "Layer 1 core gates OK"

# -----------------------------------------------------------------------------
# W6 7b — per-example flake check. Each `examples/<name>/flake.nix`
# pins `nixling.url = "path:../.."` so this runs the in-tree framework
# without a network fetch. Eval-only (`--no-build --all-systems`); a
# build-level gate already lives in the root flake's
# `checks.<system>.*` (also W6 7b). Skips gracefully if the examples/
# directory doesn't exist (some downstream consumers may strip it).
# -----------------------------------------------------------------------------
log "--> per-example flake check"
if [ -d "$ROOT/examples" ]; then
  shopt -s nullglob
  for ex in "$ROOT"/examples/*/; do
    [ -f "$ex/flake.nix" ] || continue
    name=$(basename "$ex")
    if (cd "$ex" && nix flake check --no-build --all-systems) >/dev/null 2>&1; then
      ok "example flake check: $name"
    else
      # W6fu M7: emit the diagnostic BEFORE `fail` so the operator
      # actually sees the underlying error. `fail` returns nonzero
      # under `set -e`, which would otherwise make any code after it
      # unreachable.
      (cd "$ex" && nix flake check --no-build --all-systems) 2>&1 | tail -20 >&2 || true
      fail "example flake check: $name"
    fi
  done
  shopt -u nullglob
else
  log "  (no examples/ directory — skipping)"
fi

log "Layer 1 examples OK"
log "Static checks OK"
