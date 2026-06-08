#!/usr/bin/env bash
# tests/daemon-autostart-eval.sh — P2 ph2-p2-daemon-autostart.
#
# Asserts the static surface of the nixlingd autostart contract:
#
# 1. The `nixling.daemon.autostart.parallelism` NixOS option exists,
#    defaults to 3, and accepts an override.
# 2. The Rust autostart module exposes the documented public surface
#    (AutostartPlan / VmAutostartEntry / AutostartConfig /
#    AutostartReport / Outcome / VmStarter / build_autostart_plan /
#    execute_autostart) and the DEFAULT_PARALLELISM constant agrees
#    with the NixOS default.
# 3. The daemon's `serve()` actually invokes
#    `run_startup_autostart` on startup so the contract isn't dead
#    code.
# 4. The contract is documented in
#    docs/reference/daemon-autostart.md and cross-referenced from
#    docs/reference/daemon-api.md.
#
# Static + small nixpkgs eval — runs in seconds.

set -uo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
cd "$ROOT" || exit 1

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/daemon-autostart-eval.sh"

# ---------------------------------------------------------------
# (2) Rust public surface.
# ---------------------------------------------------------------
MOD="packages/nixlingd/src/autostart.rs"
[ -f "$MOD" ] || fail "missing $MOD"
for sym in \
    "pub struct AutostartPlan" \
    "pub struct VmAutostartEntry" \
    "pub struct AutostartConfig" \
    "pub struct AutostartReport" \
    "pub struct AutostartOutcome" \
    "pub enum Outcome" \
    "pub trait VmStarter" \
    "pub fn build_autostart_plan" \
    "pub async fn execute_autostart" \
    "pub const DEFAULT_PARALLELISM"; do
    grep -qF "$sym" "$MOD" || fail "autostart.rs missing '$sym'"
    ok "$sym"
done

# DEFAULT_PARALLELISM must be 3 to match the NixOS default.
if ! grep -qE 'pub const DEFAULT_PARALLELISM: usize = 3;' "$MOD"; then
    fail "autostart.rs DEFAULT_PARALLELISM must equal 3"
fi
ok "DEFAULT_PARALLELISM = 3"

# Module is published.
grep -q "pub mod autostart;" packages/nixlingd/src/lib.rs \
    || fail "nixlingd/src/lib.rs does not declare 'pub mod autostart'"
ok "pub mod autostart"

# Daemon serve() actually invokes the autostart pass.
grep -q "run_startup_autostart" packages/nixlingd/src/lib.rs \
    || fail "nixlingd/src/lib.rs does not call run_startup_autostart"
ok "serve() wires run_startup_autostart"

# Production starter exists.
grep -q "struct BrokerVmStarter" packages/nixlingd/src/lib.rs \
    || fail "BrokerVmStarter (production VmStarter impl) missing"
ok "BrokerVmStarter present"

# Config field is exposed on DaemonConfig.
grep -q "autostart_parallelism" packages/nixlingd/src/lib.rs \
    || fail "DaemonConfig missing autostart_parallelism field"
ok "DaemonConfig.autostart_parallelism present"

# ---------------------------------------------------------------
# (4) Documentation surface.
# ---------------------------------------------------------------
DOC="docs/reference/daemon-autostart.md"
[ -f "$DOC" ] || fail "missing $DOC"
for needle in \
    "Net VMs first" \
    "Concurrency cap" \
    "Degraded" \
    "Idempotent" \
    "parallelism" \
    "nixling.daemon.autostart"; do
    grep -qF "$needle" "$DOC" || fail "daemon-autostart.md missing '$needle'"
done
ok "docs/reference/daemon-autostart.md covers the contract"

# Daemon API doc cross-references it.
grep -q "daemon-autostart" docs/reference/daemon-api.md \
    || fail "docs/reference/daemon-api.md does not cross-reference daemon-autostart.md"
ok "daemon-api.md cross-references daemon-autostart.md"

# ---------------------------------------------------------------
# (1) NixOS option default + override.
# ---------------------------------------------------------------
EXPR=$(cat <<'EOF'
let
  flake = builtins.getFlake (toString @ROOT@);
  nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
  mkSystem = extra: nixosSystem {
    system = "x86_64-linux";
    modules = [
      flake.nixosModules.default
      ({ lib, ... }: {
        boot.loader.grub.enable = false;
        boot.loader.systemd-boot.enable = false;
        boot.initrd.includeDefaultModules = false;
        fileSystems."/" = { device = "tmpfs"; fsType = "tmpfs"; };
        environment.etc."machine-id".text = "00000000000000000000000000000000";
        system.stateVersion = "25.11";
        users.users.alice = { isNormalUser = true; uid = 1000; };
        nixling.site = { waylandUser = "alice"; launcherUsers = [ "alice" ]; yubikey.enable = false; };
        nixling.envs.work = { lanSubnet = "10.20.0.0/24"; uplinkSubnet = "192.0.2.0/30"; };
      })
      extra
    ];
  };
  defaultSys = mkSystem ({ ... }: {});
  overrideSys = mkSystem ({ ... }: { nixling.daemon.autostart.parallelism = 7; });
in {
  default = defaultSys.config.nixling.daemon.autostart.parallelism;
  override = overrideSys.config.nixling.daemon.autostart.parallelism;
}
EOF
)
EXPR="${EXPR//@ROOT@/$ROOT}"

OUT=$(nix-instantiate --eval --strict --json --expr "$EXPR" 2>/dev/null) \
    || fail "nix-instantiate failed; cannot evaluate nixling.daemon.autostart.parallelism"

DEFAULT_VAL=$(printf '%s' "$OUT" | jq -r '.default')
OVERRIDE_VAL=$(printf '%s' "$OUT" | jq -r '.override')

[ "$DEFAULT_VAL" = "3" ] \
    || fail "nixling.daemon.autostart.parallelism default = $DEFAULT_VAL; expected 3"
ok "nixling.daemon.autostart.parallelism default = 3"

[ "$OVERRIDE_VAL" = "7" ] \
    || fail "nixling.daemon.autostart.parallelism override = $OVERRIDE_VAL; expected 7"
ok "nixling.daemon.autostart.parallelism override honored (= 7)"

echo "PASS daemon-autostart-eval.sh"
