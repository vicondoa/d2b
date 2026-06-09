#!/usr/bin/env bash
# tests/niri-vm-borders-eval.sh — eval-time tests for the opt-in niri
# window-rule include generation.
#
# Checks that:
#   - when disabled (default), no /etc/nixling/niri-vm-borders.kdl is
#     installed;
#   - when enabled with a graphics VM, the KDL contains the correct
#     window-rule and crosvm-hide rule;
#   - per-VM color overrides appear verbatim in the KDL;
#   - the default deterministic palette color is stable across two evals
#     of the same VM name;
#   - a non-graphics VM does not produce a window-rule block;
#   - the crosvm scanout-window hide rule is always present when the
#     feature is enabled.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/niri-vm-borders-eval.sh"

# Base nixosSystem boilerplate shared by every case.
base_modules='
  boot.loader.grub.enable = false;
  boot.loader.systemd-boot.enable = false;
  boot.initrd.includeDefaultModules = false;
  fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
  environment.etc."machine-id".text = "00000000000000000000000000000000";
  system.stateVersion = "25.11";
  users.users.alice = { isNormalUser = true; uid = 1000; };
  nixling.site = {
    waylandUser = "alice";
    launcherUsers = [ "alice" ];
  };
  nixling.envs.work = {
    lanSubnet = "10.20.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };
  nixling.vms.work = {
    enable = true;
    env = "work";
    index = 10;
    ssh.user = "alice";
    graphics.enable = true;
    graphics.crossDomainTrusted = true;
    config = {
      networking.hostName = lib.mkDefault "work";
      users.users.alice = { isNormalUser = true; uid = 1000; };
    };
  };
  nixling.vms.headless = {
    enable = true;
    env = "work";
    index = 11;
    ssh.user = "alice";
    config = {
      networking.hostName = lib.mkDefault "headless";
      users.users.alice = { isNormalUser = true; uid = 1000; };
    };
  };
'

eval_case() {
  local name="$1" override="$2" expr
  expr=$(cat <<EOF
let
  flake = builtins.getFlake (toString $ROOT);
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  nixos = nixosSystem {
    system = "x86_64-linux";
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: { $base_modules })
      $override
    ];
  };
  etcFiles = nixos.config.environment.etc;
  kdlKey = "nixling/niri-vm-borders.kdl";
  hasKdl = builtins.hasAttr kdlKey etcFiles;
  kdlText = if hasKdl then etcFiles.\${kdlKey}.text else "";
in {
  inherit hasKdl kdlText;
  # Match on the comment anchor "// Borders for VM: <name>" which
  # nixling emits for each graphics VM and which contains no backslashes.
  hasWorkRule     = builtins.match ".*// Borders for VM: work.*" kdlText != null;
  hasHeadlessRule = builtins.match ".*// Borders for VM: headless.*" kdlText != null;
  hasCrosvmRule   = builtins.match ".*match app-id=.*crosvm.*" kdlText != null;
  includeComment  = builtins.match ".*include.*niri-vm-borders\\.kdl.*" kdlText != null;
}
EOF
  )
  nix-instantiate --eval --strict --json --expr "$expr" 2>/dev/null \
    || fail "$name: eval failed"
}

# ── Case 1: feature disabled (default) ──────────────────────────────────────
log "-- case: disabled (default)"
disabled=$(eval_case "disabled" '({ ... }: { })')
actual=$(printf '%s' "$disabled" | jq -r '.hasKdl')
[ "$actual" = "false" ] \
  && ok "disabled: no KDL file installed" \
  || fail "disabled: expected hasKdl=false, got $actual"

# ── Case 2: enabled, one graphics VM ────────────────────────────────────────
log "-- case: enabled with graphics VM"
enabled=$(eval_case "enabled" \
  '({ ... }: { nixling.site.niriVmBorders.enable = true; })')

actual=$(printf '%s' "$enabled" | jq -r '.hasKdl')
[ "$actual" = "true" ] \
  && ok "enabled: KDL file installed" \
  || fail "enabled: expected hasKdl=true, got $actual"

actual=$(printf '%s' "$enabled" | jq -r '.hasWorkRule')
[ "$actual" = "true" ] \
  && ok "enabled: work VM rule present" \
  || fail "enabled: work VM window-rule missing"

actual=$(printf '%s' "$enabled" | jq -r '.hasHeadlessRule')
[ "$actual" = "false" ] \
  && ok "enabled: headless (non-graphics) VM has no rule" \
  || fail "enabled: headless VM should not have a window-rule"

actual=$(printf '%s' "$enabled" | jq -r '.hasCrosvmRule')
[ "$actual" = "true" ] \
  && ok "enabled: crosvm scanout-window hide rule present" \
  || fail "enabled: crosvm scanout-window hide rule missing"

actual=$(printf '%s' "$enabled" | jq -r '.includeComment')
[ "$actual" = "true" ] \
  && ok "enabled: include path comment present in KDL" \
  || fail "enabled: include path comment missing"

# ── Case 3: per-VM color override appears verbatim ──────────────────────────
log "-- case: per-VM color override"
override_color=$(eval_case "color-override" \
  '({ ... }: {
     nixling.site.niriVmBorders.enable = true;
     nixling.vms.work.graphics.niriBorderColor = "#aabbcc";
   })')

kdl_text=$(printf '%s' "$override_color" | jq -r '.kdlText')
if printf '%s' "$kdl_text" | grep -qF '"#aabbcc"'; then
  ok "color-override: custom color appears verbatim in KDL"
else
  fail "color-override: #aabbcc not found in generated KDL"
fi

# ── Case 4: default color is stable (same name → same color) ────────────────
log "-- case: default color stability"
color_a=$(eval_case "color-stable-a" \
  '({ ... }: { nixling.site.niriVmBorders.enable = true; })')
color_b=$(eval_case "color-stable-b" \
  '({ ... }: { nixling.site.niriVmBorders.enable = true; })')

kdl_a=$(printf '%s' "$color_a" | jq -r '.kdlText')
kdl_b=$(printf '%s' "$color_b" | jq -r '.kdlText')
if [ "$kdl_a" = "$kdl_b" ]; then
  ok "color-stable: identical config produces identical KDL"
else
  fail "color-stable: KDL output is not deterministic across evals"
fi

# ── Case 5: custom outputPath ────────────────────────────────────────────────
log "-- case: custom outputPath"
custom_path_expr=$(cat <<EOF
let
  flake = builtins.getFlake (toString $ROOT);
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  nixos = nixosSystem {
    system = "x86_64-linux";
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: { $base_modules })
      ({ ... }: {
         nixling.site.niriVmBorders.enable = true;
         nixling.site.niriVmBorders.outputPath = "/etc/nixling/custom-borders.kdl";
       })
    ];
  };
  etcFiles = nixos.config.environment.etc;
in {
  hasCustomKey  = builtins.hasAttr "nixling/custom-borders.kdl" etcFiles;
  hasDefaultKey = builtins.hasAttr "nixling/niri-vm-borders.kdl" etcFiles;
}
EOF
)
custom_path=$(nix-instantiate --eval --strict --json --expr "$custom_path_expr" 2>/dev/null \
  || fail "custom-outputPath: eval failed")

actual=$(printf '%s' "$custom_path" | jq -r '.hasCustomKey')
[ "$actual" = "true" ] \
  && ok "custom-outputPath: file installed at custom path" \
  || fail "custom-outputPath: file not found at custom path"

actual=$(printf '%s' "$custom_path" | jq -r '.hasDefaultKey')
[ "$actual" = "false" ] \
  && ok "custom-outputPath: default path not used" \
  || fail "custom-outputPath: file also installed at default path (should not be)"

log "==> tests/niri-vm-borders-eval.sh: all cases passed"
