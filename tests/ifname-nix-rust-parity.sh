#!/usr/bin/env bash
# tests/ifname-nix-rust-parity.sh— gate.
#
# Asserts that every `ifNameMappings[].derivedIfname` value emitted by
# the Nix host-json emitter (`nixos-modules/host-json.nix`) is accepted
# by the Rust `nixling_host::ifname::looks_nixling_owned` predicate.
#
# Replaced the original shell-regex oracle
# (`^nl-[bt][0-9A-F]{8}$`) with a cargo-test invocation of the real
# Rust function, fed the rendered host.json path via the
# `NIXLING_IFNAME_PARITY_HOST_JSON` env var. The previous regex would
# silently keep passing if a future Rust change tightened the predicate
# (e.g. added a length cap, role-tag set, or alphabet restriction);
# the cargo-test path exercises the production code so any drift fails
# closed.
#
# Earlier, the Nix emitter produced names like `nl-bridge-c4df354a`
# (18 bytes, > IFNAMSIZ-1 and rejected by `looks_nixling_owned` because
# `bridge-...` is not a single Crockford-alphabet char). The emitter
# now uses single-char role tags (`b` / `t`) plus
# 8 upper-case hex chars (a strict subset of the Crockford base32
# alphabet) so the format matches what the Rust predicate accepts.
# The hash algorithms still differ (Nix: SHA-256; Rust: FNV-1a /
# Crockford) — that is OK because the broker uses the predicate to
# *filter* nixling-owned interfaces, not to reconstruct them; the
# algorithms only need to agree on the format the predicate
# recognises.
#
# Scratch state lives outside $ROOT.

set -euo pipefail
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(cd "$HERE/.." && pwd)}
# shellcheck source=lib.sh
. "$HERE/lib.sh"

nl_activate_rust_toolchain_path || true

cd "$ROOT"

if [ -z "${NIXLING_IFNAME_NIX_RUST_PARITY_IN_NIX_SHELL:-}" ] && ! command -v cargo >/dev/null 2>&1; then
  if ! command -v nix >/dev/null 2>&1; then
    fail "ifname-nix-rust-parity: neither cargo nor nix is on PATH"
  fi
  export NIXLING_IFNAME_NIX_RUST_PARITY_IN_NIX_SHELL=1
  exec nix shell --quiet --inputs-from "$ROOT" \
    nixpkgs#cargo nixpkgs#rustc nixpkgs#rustfmt nixpkgs#clippy nixpkgs#gcc nixpkgs#sccache \
    nixpkgs#jq \
    --command bash "$0" "$@"
fi

log "W3fu2 H4 ifname Nix↔Rust parity (host.json ifNameMappings[].derivedIfname against looks_nixling_owned)"

if ! host_path=$(nl_smoke_bundle_host_json); then
  fail "ifname-nix-rust-parity: could not render smoke host.json"
fi
scratch=$(nl_mktemp .ifname-nix-rust-parity.XXXXXX)

# Quick sanity grep so the gate can show what it is testing.
count=$(jq -r '(.ifNameMappings // []) | length' "$host_path")
log "  smoke host.json: $count ifNameMappings entries"

# The previous shape returned OK before dispatching to the Rust oracle
# when `count` was zero, which would
# let a regression that drops all Nix-emitted mappings pass invisibly.
# An empty `ifNameMappings` array in the smoke bundle is itself a
# real regression of the emitter— fail closed.
if [ "$count" -eq 0 ]; then
  fail "ifname-nix-rust-parity: smoke host.json has empty/missing ifNameMappings; W3 emitter regression suspected. The gate cannot prove parity if the emitter produces zero names."
fi

runtime_path="$scratch/host-runtime.json"
runtime_host_path="$scratch/host-runtime-host.json"
sentinel=$(jq -r '
  .ifNameMappings[0].role
  | if . == "workload-lan" then "nl-tA1B2C3D4" else "nl-bA1B2C3D4" end
' "$host_path")

jq --arg sentinel "$sentinel" '
  {
    schemaVersion: "v2",
    bundleVersion: 4,
    generatedAt: "2025-05-30T00:00:00Z",
    ifnames: (
      (.ifNameMappings // [])
      | map({
          env,
          vm: (.vm // null),
          userVisibleName,
          derivedIfname,
          roleTag: (
            if .role == "workload-lan" then "wkl"
            elif .role == "uplink" then "upl"
            else "nvl"
            end
          )
        })
    )
  }
  | .ifnames[0].derivedIfname = $sentinel
' "$host_path" > "$runtime_path"

modules=$(_nl_smoke_config_modules)
NIXLING_HOST_RUNTIME_PATH="$runtime_path" \
  nix eval --impure --raw --expr "
    let
      flake = builtins.getFlake (toString $ROOT);
      nixosSystem = flake.inputs.nixpkgs.lib.nixosSystem;
      nixos = nixosSystem {
        system = builtins.currentSystem;
        modules = [
          $modules
        ];
      };
    in nixos.config.nixling._bundle.hostJson.jsonText
  " > "$runtime_host_path"

runtime_derived=$(jq -r '.ifNameMappings[0].derivedIfname' "$runtime_host_path")
default_derived=$(jq -r '.ifNameMappings[0].derivedIfname' "$host_path")
[ "$runtime_derived" = "$sentinel" ] || \
  fail "ifname-nix-rust-parity: runtime host-runtime.json override was not taken (expected $sentinel, got $runtime_derived)"
[ "$default_derived" != "$runtime_derived" ] || \
  fail "ifname-nix-rust-parity: runtime host-runtime.json override did not change the emitted derivedIfname"
ok "ifname-nix-rust-parity: host-json emitter prefers host-runtime.json when available"

WORKSPACE_DIR=$ROOT/packages

# Invoke the real `looks_nixling_owned` predicate via
# `cargo test -p nixling-host -- nix_emitted_ifnames_pass_looks_nixling_owned`.
# The test reads the host.json path from NIXLING_IFNAME_PARITY_HOST_JSON,
# parses `ifNameMappings[].derivedIfname`, and panics with a precise
# violation list if any name is rejected by the predicate.
log " - cargo test -p nixling-host -- nix_emitted_ifnames_pass_looks_nixling_owned"
(
  cd "$WORKSPACE_DIR"
  NIXLING_IFNAME_PARITY_HOST_JSON="$host_path" \
    CARGO_BUILD_RUSTC_WRAPPER="" \
    cargo test -p nixling-host --quiet -- \
      nix_emitted_ifnames_pass_looks_nixling_owned --nocapture
)
(
  cd "$WORKSPACE_DIR"
  NIXLING_IFNAME_PARITY_HOST_JSON="$runtime_host_path" \
    CARGO_BUILD_RUSTC_WRAPPER="" \
    cargo test -p nixling-host --quiet -- \
      nix_emitted_ifnames_pass_looks_nixling_owned --nocapture
)

ok "ifname-nix-rust-parity: $count derivedIfname values accepted by the Rust looks_nixling_owned predicate"
