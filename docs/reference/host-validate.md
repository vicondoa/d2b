# `nixling host validate` — composite W18 readiness preflight

**Plan:** P5 ph5-p5-host-validate-verb.

`nixling host validate` is the operator-facing one-command preflight
the operator runs after `nixos-rebuild switch` and before flipping
`nixling.daemonExperimental.enable = true`. It is the umbrella verb
that produces the per-wave evidence records consumed by the W18
auto-flip gate (`nixos-modules/options-daemon.nix:validationEvidencePresent`).

## What it does

1. Iterates the W18 readiness waves in the deterministic catalog
   order (`w4Fu`, `w5Fu`, `w6Fu`, `w7Fu`, `w8Fu`, `w9Fu`, `p0`,
   `p0Fu`, `p1`, `p2`, `p3`, `p4`, `p5`, `p6`, `p7`). The catalog
   is sourced from `packages/nixling/src/host_validate.rs::WAVE_CATALOG`
   and is held byte-identical to `readinessWaveSpecs` in
   `nixos-modules/options-daemon.nix` by the Layer-1 gate
   [`tests/host-validate-verb-eval.sh`](../../tests/host-validate-verb-eval.sh).
2. For each wave, inventories the per-wave Layer-2 validator
   scripts shipped under `tests/` (e.g.
   `tests/minijail-validator-*.sh`,
   `tests/per-vm-state-ownership-eval.sh`,
   `tests/nixlingd-startup-smoke.sh`). A wave is `ready` only when
   every declared validator is present and readable on disk.
3. In `--apply` mode, writes the canonical evidence record
   `/var/lib/nixling/validated/<wave>.json` for every `ready`
   wave; the W18 gate then accepts that wave as `validated`.

The verb does NOT execute the per-wave shell validators itself
— those are Layer-2 integration tests that typically require live
host state, sudo, and external hardware (GPU, YubiKey, swtpm). The
operator runs them out-of-band; the validators that already
emit their own per-role evidence file (e.g.
`tests/minijail-validator-swtpm.sh` → `p1-swtpm.json`) continue to do
so. `host validate --apply` is the umbrella attestation that
produces the per-wave `<wave>.json` files the W18 readiness option
consumes.

## Usage

```text
nixling host validate (--dry-run | --apply)
                      [--wave <name>]
                      [--operator-signature <sig>]
                      [--evidence-dir <path>]
                      [--scripts-dir <path>]
                      [--json | --human]
```

Exactly one of `--dry-run` or `--apply` is required (the verb
returns the `--apply-or-dry-run-required` envelope, exit 78, if
neither is given — matching every other `host *` verb).

| Flag | Meaning |
| --- | --- |
| `--dry-run` | Plan only. Inventories every wave's validator presence; writes nothing. Always exits 0. |
| `--apply` | Writes evidence for every `ready` wave. Returns exit 78 if any wave is still `missing` (operator must run the listed validators first). |
| `--wave <name>` | Restrict to a single wave (validated against the catalog; unknown values surface the typed `unknown-wave` envelope, exit 78). |
| `--operator-signature <sig>` | Override the per-wave operator signature. By default the verb computes a deterministic `sha256:` digest of `hostname \| wave \| scripts_dir \| timestamp`. |
| `--evidence-dir <path>` | Override the evidence directory. Defaults to `/var/lib/nixling/validated` (the W18 path). Tests use a scratch dir; operators should never override this in production. |
| `--scripts-dir <path>` | Override the validator scripts directory. Defaults to `/run/current-system/sw/share/nixling/tests` (installed) → `./tests` (dev). Override with `NIXLING_VALIDATE_SCRIPTS_DIR`. |
| `--json` / `--human` | Render JSON (machine-consumable) vs human-readable text. JSON is default-suitable for the `nixling` JSON contract. |

## Evidence schema

Each evidence file written by `--apply` is a single JSON object on
disk with exactly the three fields the W18 gate requires:

```json
{
  "wave": "p1",
  "timestamp": "2025-11-15T10:30:00Z",
  "operatorSignature": "sha256:0123…"
}
```

The schema is enforced from the other direction by
`nixos-modules/options-daemon.nix:validationEvidencePresent`, which
rejects any record whose `wave` field does not match the basename,
or whose `timestamp` / `operatorSignature` are absent or empty.

## Per-wave validator map

| Wave | Validators (relative to `tests/`) |
| --- | --- |
| `w4Fu` | `nixlingd-startup-smoke.sh` |
| `w5Fu` | `hardware-smoke-gpu-yubikey.sh` |
| `w6Fu` | `hardware-smoke-gpu-yubikey.sh`, `usbip-state-machine-eval.sh` |
| `w7Fu` | `per-vm-state-ownership-eval.sh` |
| `w8Fu` | `ssh-host-key-preflight-eval.sh` |
| `w9Fu` | `harness-ubuntu-eval.sh` |
| `p0` | `broker-socket-activation-eval.sh`, `broker-caps-eval.sh`, `nixlingd-startup-smoke.sh` |
| `p0Fu` | `broker-bundle-path-eval.sh` |
| `p1` | `minijail-validator-{cloud-hypervisor,virtiofsd,swtpm,gpu,audio,video,vsock-relay,usbip,otel-host-bridge}.sh` |
| `p2` | `per-vm-state-ownership-eval.sh`, `daemon-autostart-eval.sh`, `host-prep-dag-eval.sh` |
| `p3` | `observability-eval.sh`, `daemon-metrics-eval.sh`, `usbip-state-machine-eval.sh` |
| `p4` | `cli-vm-verbs-eval.sh`, `desktop-wrapper-contract-eval.sh` |
| `p5` | `host-validate-verb-eval.sh` |
| `p6` | _(no per-host validator; readiness is gate-output only)_ |
| `p7` | _(no per-host validator; readiness is gate-output only)_ |

The two `(no validator)` waves report status `no-validators`. They
intentionally do not write an evidence file — the W18 readiness
gate for `p6`/`p7` is driven entirely by Layer-1 panel output
(`tests/legacy-unit-denylist-eval.sh`, `tests/static.sh` green, the
v1.0 docs blast-radius pass), not by per-host attestation.

## Exit codes

| Exit | Meaning |
| --- | --- |
| `0` | Success. Every wave the operator asked about is `ready`, `attested`, `skipped`, or `no-validators`. |
| `1` | At least one evidence write failed (typically EACCES — re-run via `sudo`). |
| `78` | Refused. Either `--dry-run`/`--apply` was missing, `--wave <name>` named an unknown wave, or `--apply` ran with at least one `missing` wave. The operator must address the surfaced reason and re-run. |

## Operator workflow

A typical first-flip workflow on a fresh host:

```bash
# 1. Activate the new closure.
sudo nixos-rebuild switch --flake .#myhost

# 2. Inventory what's ready to attest.
nixling host validate --dry-run --json | jq '.waves[] | {wave, status}'

# 3. Run any per-wave Layer-2 validators that aren't `ready` yet
#    (e.g. tests/minijail-validator-swtpm.sh requires NL_LIVE=1).
sudo NL_LIVE=1 bash tests/minijail-validator-swtpm.sh
# … repeat for every wave you want to attest …

# 4. Write the umbrella evidence records.
sudo nixling host validate --apply

# 5. Now flip the gate.
#    nixling.daemonExperimental.enable now defaults to true because
#    every <wave>.json record exists with the canonical schema.
sudo nixos-rebuild switch --flake .#myhost
```

The daemon also auto-writes an evidence record on the first
successful op of each kind (see the Critical-subsystems "Control
plane (W2+)" row in [`AGENTS.md`](../../AGENTS.md) and the W18 entry
in [`docs/reference/default-switch-and-deprecation.md`](./default-switch-and-deprecation.md)),
so a long-running host typically picks up evidence over time even
without running this verb. The verb exists so that **fresh** consumer
hosts do not hit the validation cliff between
`implementedDefault = true` and `validated = true` for waves whose
implementation has already shipped in-tree.

## Related

- [`AGENTS.md` § "Critical subsystems / Control plane (W2+)"](../../AGENTS.md) — W18 default-switch contract.
- [`docs/reference/default-switch-and-deprecation.md`](./default-switch-and-deprecation.md) — full flip timeline.
- [`docs/reference/error-codes.md`](./error-codes.md) — `--apply-or-dry-run-required` and `unknown-wave` envelopes.
- [`tests/host-validate-verb-eval.sh`](../../tests/host-validate-verb-eval.sh) — Layer-1 regression gate.
- [`packages/nixling/src/host_validate.rs`](../../packages/nixling/src/host_validate.rs) — verb implementation + per-wave catalog.
