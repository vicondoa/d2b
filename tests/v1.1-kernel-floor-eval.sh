#!/usr/bin/env bash
# v1.1 invariant gate (paired with the daemon's runtime pidfs
# self-probe in `packages/nixlingd/src/startup.rs`): assert the
# consumer's NixOS config declares a `boot.kernelPackages` whose
# version meets the v1.1 floor of >= 6.9.
#
# Static eval gate — best-effort detection of the easy case
# (operator's flake declares a < 6.9 kernel via
# `boot.kernelPackages = pkgs.linuxPackages_<X.Y>` with an
# explicit version). The runtime pidfs self-probe in the
# daemon catches the harder cases (custom kernel build at >= 6.9
# that strips pidfs out).
#
# At v1.1-rc1, this gate is a passthrough — there is no consumer-
# side configuration fixture available in CI to assert against,
# and the v1.0 floor (>= 6.6) is already documented per ADR 0008.
# The gate body lands as the v1.1 invariant placeholder; the
# enforce path runs against an operator-provided fixture at
# v1.1-final (per the v1.1 plan's release-readiness rerun).
set -euo pipefail

HERE=$(cd -- "$(dirname -- "$0")" >/dev/null 2>&1 && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

# Assert the ADR 0008 file declares the v1.1 floor.
adr="$ROOT/docs/adr/0008-supported-platforms-and-rejected-targets.md"
if [ ! -f "$adr" ]; then
  printf 'v1.1-kernel-floor-eval: FAIL — %s missing\n' "$adr" >&2
  exit 1
fi

if ! grep -q -E '(>=\s*6\.9|≥\s*6\.9|6\.9\+|kernel-floor uplift)' "$adr"; then
  printf 'v1.1-kernel-floor-eval: FAIL — ADR 0008 must declare the v1.1 >=6.9 kernel floor\n' >&2
  exit 1
fi

# Assert the migration guide cross-links the kernel floor as
# Prerequisite #1.
guide="$ROOT/docs/how-to/migrate-nixling-v1-0-to-v1-1.md"
if [ ! -f "$guide" ]; then
  printf 'v1.1-kernel-floor-eval: FAIL — %s missing\n' "$guide" >&2
  exit 1
fi
if ! grep -q -E 'kernel\s*≥?\s*6\.9|kernel\s*>=\s*6\.9' "$guide"; then
  printf 'v1.1-kernel-floor-eval: FAIL — migration guide must mention the v1.1 kernel-floor prerequisite\n' >&2
  exit 1
fi

printf 'v1.1-kernel-floor-eval: PASS (ADR 0008 + migration guide declare the v1.1 floor; runtime pidfs probe is the defense-in-depth)\n'
