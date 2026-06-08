#!/usr/bin/env bash
# tests/usbip-state-machine-eval.sh — integration gate for the typed
# per-busid USBIP state machine introduced by
# ``.
#
# The state machine itself lives in
# `packages/nixlingd/src/usbip_state_machine.rs`; the unit tests
# inside that module assert the in-process behaviour (canonical
# order, per-step failure → typed error). This script is the
# repo-level gate that:
#
#   1. Confirms the module is wired into `nixlingd::lib` (i.e. the
#      `pub mod usbip_state_machine;` declaration is present).
#   2. Confirms the source pins the canonical bring-up order
#      `modprobe → lock → withhold → firewall → backend → bind →
#      proxy` (per AGENTS.md "Critical subsystems").
#   3. Confirms `TypedError::UsbipStepFailed` is wired with the
#      pinned exit code 67 (so the public error envelope stays
#      stable across releases).
#   4. Confirms the reference documentation exists under
#      `docs/reference/usbip-state-machine.md`.
#
# The shape mirrors `tests/usbip-gating-eval.sh`: read-only,
# eval-time-only checks, no live host required.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

log()  { printf '%s %s\n' "$(date +%H:%M:%S)" "$*" >&2; }
ok()   { log "  PASS: $*"; }
fail() { log "  FAIL: $*"; exit 1; }

log "==> tests/usbip-state-machine-eval.sh"

src="$ROOT/packages/nixlingd/src/usbip_state_machine.rs"
lib="$ROOT/packages/nixlingd/src/lib.rs"
typed="$ROOT/packages/nixlingd/src/typed_error.rs"
doc="$ROOT/docs/reference/usbip-state-machine.md"

[[ -f "$src"   ]] || fail "module missing: $src"
[[ -f "$lib"   ]] || fail "lib.rs missing: $lib"
[[ -f "$typed" ]] || fail "typed_error.rs missing: $typed"
[[ -f "$doc"   ]] || fail "doc missing: $doc"
ok "every source + doc file is present"

grep -q 'pub mod usbip_state_machine;' "$lib" \
  || fail "lib.rs does not declare 'pub mod usbip_state_machine;'"
ok "lib.rs declares the module"

# Canonical step ordering pinned in the CANONICAL_STEPS const.
# Extract the seven UsbipBusidStep::* names in source order and
# compare against the canonical pin.
got=$(awk '/pub const CANONICAL_STEPS/,/\];/' "$src" \
  | grep -oE 'UsbipBusidStep::[A-Za-z]+' \
  | sed 's/UsbipBusidStep:://' \
  | tr '\n' ' ' | sed 's/ $//')
want="Modprobe Lock Withhold Firewall Backend Bind Proxy"
if [[ "$got" != "$want" ]]; then
  fail "canonical order drift: got [$got] want [$want]"
fi
ok "canonical step order pinned: $want"

# Typed error wiring: variant, kind, and exit code 67.
grep -q 'UsbipStepFailed' "$typed" \
  || fail "typed_error.rs missing UsbipStepFailed variant"
grep -q '"usbip-step-failed"' "$typed" \
  || fail "typed_error.rs missing 'usbip-step-failed' kind string"
grep -qE 'Self::UsbipStepFailed \{ \.\. \} => 67' "$typed" \
  || fail "typed_error.rs UsbipStepFailed exit code is not 67"
ok "TypedError::UsbipStepFailed wired with exit code 67"

# Doc cross-check: must name the canonical order verbatim so prose
# can't drift from the code.
grep -qF 'modprobe → lock → withhold → firewall → backend → bind → proxy' "$doc" \
  || fail "doc does not name the canonical order verbatim"
ok "docs/reference/usbip-state-machine.md names the canonical order"

log "==> tests/usbip-state-machine-eval.sh: OK"
