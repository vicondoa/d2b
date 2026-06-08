#!/usr/bin/env bash
# tests/daemon-metrics-eval.sh — P3 ph3-p3-prometheus-otlp-shape:
# static parity gate between the daemon's Prometheus metric
# inventory and its canonical reference doc.
#
# Asserts:
#   1. Every metric name listed in
#      `docs/reference/daemon-metrics.md` appears in the
#      `METRIC_INVENTORY` table in `packages/nixlingd/src/metrics.rs`,
#      with a matching `MetricKind`.
#   2. The label list declared per metric in the doc matches the
#      `labels:` slice in the `MetricDescriptor`.
#   3. The two histogram bucket constants
#      (`VM_START_BUCKETS_SECONDS`,
#      `HOST_PREP_STEP_BUCKETS_SECONDS`,
#      `BROKER_REQUEST_BUCKETS_SECONDS`) carry the bucket boundaries
#      called out in the doc.
#
# The gate is pure text-grep + python; it does not build the daemon.

set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}

DOC="$ROOT/docs/reference/daemon-metrics.md"
SRC="$ROOT/packages/nixlingd/src/metrics.rs"

for f in "$DOC" "$SRC"; do
  if [ ! -f "$f" ]; then
    echo "FAIL: missing required file: $f" >&2
    exit 1
  fi
done

PY=$(command -v python3 || command -v python || true)
if [ -z "$PY" ]; then
  # Fall back to a `nix shell` python; cached in the user profile after
  # the first call so subsequent runs reuse the same store path.
  if command -v nix >/dev/null 2>&1; then
    exec nix shell nixpkgs#python3 --command bash "$0" "$@"
  fi
  echo "FAIL: python3 not on PATH and \`nix\` unavailable" >&2
  exit 1
fi

"$PY" - "$DOC" "$SRC" <<'PYEOF'
import re
import sys

doc_path, src_path = sys.argv[1], sys.argv[2]
doc = open(doc_path).read()
src = open(src_path).read()

# --- expected set, mirrored from the doc -----------------------------
expected = [
    ("nixling_daemon_vm_state", "Gauge", ["vm", "state"], None),
    (
        "nixling_daemon_vm_start_duration_seconds",
        "Histogram",
        ["vm", "outcome"],
        [0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0, 300.0],
    ),
    (
        "nixling_daemon_host_prep_step_duration_seconds",
        "Histogram",
        ["step"],
        [0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0],
    ),
    (
        "nixling_daemon_broker_request_total",
        "Counter",
        ["op", "outcome"],
        None,
    ),
    (
        "nixling_daemon_broker_request_duration_seconds",
        "Histogram",
        ["op"],
        [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0],
    ),
    ("nixling_daemon_ownership_drift_total", "Counter", ["vm"], None),
    ("nixling_daemon_ssh_host_key_drift_total", "Counter", ["vm"], None),
    ("nixling_daemon_pidfd_table_size", "Gauge", [], None),
    ("nixling_daemon_uptime_seconds", "Gauge", [], None),
]

errors = []

# --- doc parity ------------------------------------------------------
for name, kind, labels, _ in expected:
    if f"### `{name}`" not in doc:
        errors.append(f"doc missing section for metric: {name}")
        continue
    section = doc.split(f"### `{name}`", 1)[1].split("### `", 1)[0]
    want_kind = kind.lower()
    if f"**Type:** {want_kind}" not in section:
        errors.append(f"doc {name}: type line missing or wrong (expected {want_kind})")
    if labels:
        want = ", ".join(f"`{l}`" for l in labels)
        if f"**Labels:** {want}" not in section:
            errors.append(f"doc {name}: labels line missing or wrong (expected {want})")
    else:
        if "**Labels:** *(none)*" not in section:
            errors.append(f"doc {name}: expected labels = *(none)*")

# --- source parity ---------------------------------------------------
# Extract MetricDescriptor blocks.
descriptor_re = re.compile(
    r"MetricDescriptor\s*\{\s*"
    r"name:\s*\"(?P<name>[A-Za-z0-9_]+)\"\s*,\s*"
    r"kind:\s*MetricKind::(?P<kind>Counter|Gauge|Histogram)\s*,\s*"
    r"labels:\s*&\[(?P<labels>[^\]]*)\]\s*,\s*"
    r"buckets_seconds:\s*(?P<buckets>[^,]+)\s*,\s*"
    r"\}",
    re.DOTALL,
)
found = {}
for m in descriptor_re.finditer(src):
    name = m.group("name")
    kind = m.group("kind")
    labels_raw = m.group("labels").strip()
    if labels_raw:
        labels = [s.strip().strip('"') for s in labels_raw.split(",") if s.strip()]
    else:
        labels = []
    buckets_expr = m.group("buckets").strip()
    found[name] = (kind, labels, buckets_expr)

for name, kind, labels, buckets in expected:
    if name not in found:
        errors.append(f"src missing MetricDescriptor for {name}")
        continue
    got_kind, got_labels, got_buckets_expr = found[name]
    if got_kind != kind:
        errors.append(f"src {name}: kind {got_kind} != expected {kind}")
    if got_labels != labels:
        errors.append(f"src {name}: labels {got_labels} != expected {labels}")
    if buckets is None:
        if got_buckets_expr != "&[]":
            errors.append(
                f"src {name}: expected empty buckets, got {got_buckets_expr}"
            )

# Verify the three bucket constants are exactly the expected values.
def extract_const(name):
    m = re.search(
        rf"pub const {re.escape(name)}:\s*&\[f64\]\s*=\s*&\[(.*?)\];",
        src,
        re.DOTALL,
    )
    if not m:
        return None
    raw = m.group(1)
    out = []
    for tok in raw.split(","):
        tok = tok.strip()
        if not tok:
            continue
        out.append(float(tok))
    return out

for const_name, expected_vals in [
    ("VM_START_BUCKETS_SECONDS", [0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0, 300.0]),
    ("HOST_PREP_STEP_BUCKETS_SECONDS", [0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]),
    ("BROKER_REQUEST_BUCKETS_SECONDS", [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
]:
    got = extract_const(const_name)
    if got != expected_vals:
        errors.append(f"src constant {const_name} = {got} != expected {expected_vals}")

# Bucket-doc parity: spot check the doc's "Buckets (seconds):" line.
for name, _, _, buckets in expected:
    if buckets is None:
        continue
    section = doc.split(f"### `{name}`", 1)[1].split("### `", 1)[0]
    doc_buckets_re = re.search(r"\*\*Buckets \(seconds\):\*\* `([^`]+)`", section)
    if not doc_buckets_re:
        errors.append(f"doc {name}: missing buckets line")
        continue
    parsed = [float(t.strip()) for t in doc_buckets_re.group(1).split(",")]
    if parsed != buckets:
        errors.append(
            f"doc {name}: buckets line {parsed} != expected {buckets}"
        )

if errors:
    for e in errors:
        print(f"FAIL: {e}")
    sys.exit(1)

print(f"PASS: daemon-metrics inventory parity ({len(expected)} metrics)")
PYEOF
