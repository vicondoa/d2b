# Wave validation evidence schema

Canonical reference for the host-local proof files that gate
`nixling.defaultSwitchReadiness.<wave>.validated = true`. (These files
no longer drive `nixling.daemonExperimental.enable`, which is now an
obsolete always-on gate; they remain live for the per-wave `validated`
assertion and for `nixling host validate`.)

The schema is implicitly defined by the cargo-checked validator in
[`nixos-modules/options-daemon.nix`](../../nixos-modules/options-daemon.nix)
(`validationEvidencePresent`). This document is the operator-facing
mirror of that validator; the two MUST be kept in sync. The
companion JSON Schema lives at
[`wave-evidence-schema.json`](./wave-evidence-schema.json).

## File location

```
/var/lib/nixling/validated/<wave>.json
```

One file per wave. The basename (sans `.json`) MUST match the
`wave` field inside the payload, and MUST be one of the keys
declared in
[`nixos-modules/options-daemon.nix`](../../nixos-modules/options-daemon.nix)
under `readinessWaveSpecs` (see the [per-wave inventory](#per-wave-inventory)
below).

Recommended permissions: `0644 root:root`. The file is read at
eval time by Nix (under the building user), so it must be
world-readable; it carries no secrets.

## Schema

Every payload is a JSON object with three required string fields.
Additional fields are tolerated (and preserved by future
operator-verb writers for forward-compat).

| Field               | Type     | Required | Validator predicate                                                                 |
| ------------------- | -------- | -------- | ----------------------------------------------------------------------------------- |
| `wave`              | `string` | yes      | `builtins.isString payload.wave && payload.wave == <basename>`                      |
| `timestamp`         | `string` | yes      | `builtins.isString payload.timestamp && payload.timestamp != ""`                    |
| `operatorSignature` | `string` | yes      | `builtins.isString payload.operatorSignature && payload.operatorSignature != ""`    |

Failure of any predicate flips the per-wave assertion shipped from
`options-daemon.nix`:

```
nixling.defaultSwitchReadiness.<wave>.validated = true requires
/var/lib/nixling/validated/<wave>.json to exist and contain JSON
fields "wave" = "<wave>", "timestamp", and "operatorSignature".
```

### Canonical example

```json
{
  "wave": "p0",
  "timestamp": "2025-04-12T17:42:11Z",
  "operatorSignature": "alice@example"
}
```

### Field semantics

- **`wave`** — wave identifier. Must equal the file basename so a
  copied-by-mistake `p0.json` cannot satisfy `p1`.
- **`timestamp`** — when the validating smoke run completed.
  RFC 3339 / ISO-8601 UTC is the recommended shape; the
  validator enforces only non-empty `string`.
- **`operatorSignature`** — who attests to the run. Free-form
  string; typical shapes are `alice@example`,
  `ci-bot@build-host-3`, or a host fingerprint. The validator
  enforces only non-empty `string`.

Additional fields (e.g. `bundleHash`, `smokeScript`, `auditLogRefs`)
are permitted and intended for richer P5/P6 writers; the eval-time
validator ignores them.

## Per-wave inventory

Every readiness wave declared in `readinessWaveSpecs` requires an
evidence file under `/var/lib/nixling/validated/<wave>.json` before
`nixling.defaultSwitchReadiness.<wave>.validated = true` will pass
eval.

| Wave key | Implemented (shipped code)                                                                                                                                            | Validated (what the evidence file attests) — i.e. what the operator must have exercised before writing the file |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `w4Fu`   | W12/W14 headless daemon + supervisor path.                                                                                                                            | Ubuntu Tier-1 smoke + matching `broker-<utc-date>.jsonl` audit log entries.                                    |
| `w5Fu`   | W17 minijail profiles + GPU/audio/video argv generators.                                                                                                              | W20 hardware smoke (NVIDIA Quadro T1000 / virtio-snd / virtio-media) + audit log evidence. Depends on `w4Fu` validated. |
| `w6Fu`   | W13 USBIP live executors + per-busid lock.                                                                                                                            | W20 hardware smoke (YubiKey USBIP path) + USBIP audit evidence.                                                |
| `w7Fu`   | W7b store-lifecycle verbs + admin auth (`switch` / `boot` / `test` / `rollback` / `gc`).                                                                              | Switch/boot/test/rollback/gc smoke + audit log evidence.                                                       |
| `w8Fu`   | W14 keys/trust/rotate-known-host live wiring.                                                                                                                         | Keys/trust smoke + audit log evidence.                                                                         |
| `w9Fu`   | W15 host install + migrate live broker ops.                                                                                                                           | Host install/migrate smoke + audit log evidence.                                                               |
| `p0`     | P0 daemon-only foundation: broker socket-activation, bundle digest verify, canonical `/run/nixling`, `nixlingd.service restartIfChanged=false`.                       | `tests/nixlingd-startup-smoke.sh` green on this host, recorded into the evidence file.                         |
| `p0Fu`   | P0fu: cgroup delegation sequence, bundle-tampered envelope, per-artifact hash verification, `ListenSequentialPacket` socket fix.                                      | `tests/broker-cgroup-delegation-smoke.sh` green on this host.                                                  |
| `p1`     | Per-role minijail profiles + byte-parity argv generators (CH, virtiofsd, swtpm, gpu, audio, video, vsockRelay, usbip, otelHostBridge).                                | Per-role `tests/minijail-validator-<role>.sh` green + hardware smoke on the target SKUs.                       |
| `p2`     | Daemon-side host-prep + ownership matrix + `manifestVersion=4` + daemon autostart.                                                                                    | `tests/daemon-autostart-smoke.sh` + `tests/vms-json-parity.sh` + ownership-eval green.                         |
| `p3`     | Host singletons retired (net-route-preflight, audit-check, ch-exporter, otel-host-bridge, per-env usbipd) + daemon health endpoint.                                   | `tests/observability-eval.sh` + USBIP smoke + degraded-mode escape-hatch smoke green.                          |
| `p4`     | `vm start/stop/restart/list` daemon-native end-to-end; `.desktop` wrapper updated.                                                                                    | Per-VM `vm start` smoke + Wayland desktop launcher smoke green.                                                |
| `p5`     | First-run validation UX shipped (`nixling host validate --apply` + daemon auto-write on first op).                                                                    | Fresh-host bootstrap smoke green on this host.                                                                 |
| `p6`     | Legacy systemd template emission + bash CLI removed (clean break). The `nixling.vms.<vm>.supervisor` option's hard removal + eval-time rejection assertion was deferred to v1.1 backlog (see ADR 0015 § Decision); v1.0 retains the option with default `"systemd"` for backward-compat. | `tests/legacy-unit-denylist-eval.sh` + `tests/static.sh` green. |
| `p7`     | Docs blast-radius + v1.0 cut shipped.                                                                                                                                 | `tests/static.sh` + per-example flake-check green.                                                             |

> **Drift gate.** `tests/wave-evidence-schema-eval.sh` asserts every
> wave declared in `readinessWaveSpecs` has a matching `| \`<wave>\` |`
> row in the table above. Add a new wave to `readinessWaveSpecs` →
> add a row here in the same commit, or the gate fails.

Cross-dependencies enforced by additional assertions in
`options-daemon.nix`:

- `w5Fu.implemented = true` requires `w4Fu.implemented = true`
  (GPU/audio sidecars spawn through the W4-fu `SpawnRunner` broker
  exec).
- `w5Fu.validated = true` requires `w4Fu.validated = true`
  (W5-fu validation depends on the W4-fu `SpawnRunner` path already
  being validated).

## Operator workflow

The intended path from a fresh host to a wave's
`defaultSwitchReadiness.<wave>.validated = true`:

1. **Land the code.** `nixos-rebuild switch` to a nixling version
   that ships the wave's implementation (`implemented = true`
   already defaults on for the `w*Fu` waves; `p0..p7` flip in their
   own merge commits).

2. **Exercise the wave on this host.** Run the per-wave smoke
   listed in the inventory above. For `w5Fu` / `w6Fu` this is
   `tests/hardware-smoke-gpu-yubikey.sh`; for `p0` it is
   `tests/nixlingd-startup-smoke.sh`; etc.

3. **Write the evidence file.** Run:

   ```bash
   sudo nixling host validate --apply
   ```

   (P5 sibling deliverable; see [`host-validate.md`](./host-validate.md).)
   The verb composes the per-wave evidence record from the
   wave inventory and writes one
   `/var/lib/nixling/validated/<wave>.json` file per wave with
   the canonical `{wave, timestamp, operatorSignature}` payload.
   It does NOT itself run the validators — operators are expected
   to have run each wave's validator (`tests/minijail-validator-*.sh`,
   etc.) by hand or in a CI job, OR to rely on the daemon's
   opportunistic evidence-write path described below. `--dry-run`
   (the default) prints the same set of payloads without writing
   them; `--wave <wave>` narrows the run to a single wave.

   The daemon also opportunistically writes evidence on its
   first successful op for the corresponding wave (e.g. the first
   end-to-end `nixling vm start --apply` writes `p4.json`),
   bootstrapping operators who do not run `host validate`
   explicitly.

4. **Flip the readiness bit.** Add to host config:

   ```nix
   nixling.defaultSwitchReadiness.<wave>.validated = true;
   ```

5. **Rebuild.** `nixos-rebuild switch` now sees
   `defaultSwitchReadiness.<wave>.validated = true` for each
   wave whose evidence file is present, and the fail-closed eval
   assertion passes. The daemon-backed control plane is already the
   default regardless of these bits:
   `nixling.daemonExperimental.enable` is an obsolete always-on gate
   (`default = true`) and is no longer computed from wave readiness.

Setting `nixling.daemonExperimental.enable` has no current effect and
is discouraged — the daemon-only end state is always enabled. The
`validated` bits remain meaningful as host-local validation evidence,
surfaced by `nixling host validate`.

### Manual evidence writing (escape hatch)

`nixling host validate` is the supported writer. If it is
unavailable (e.g. an older daemon, a partial bootstrap, or a CI
fixture), the same file can be hand-rolled:

```bash
sudo install -d -o root -g root -m 0755 /var/lib/nixling/validated
sudo tee /var/lib/nixling/validated/p0.json > /dev/null <<'JSON'
{
  "wave": "p0",
  "timestamp": "2025-04-12T17:42:11Z",
  "operatorSignature": "alice@example"
}
JSON
sudo chmod 0644 /var/lib/nixling/validated/p0.json
```

The eval-time validator does not care who wrote the file, only
that the three fields are present and well-typed.

## See also

- [`host-validate.md`](./host-validate.md) — the
  `nixling host validate` verb (P5 sibling deliverable) that
  writes these files.
- [`default-switch-and-deprecation.md`](./default-switch-and-deprecation.md)
  — the per-wave evidence gate this evidence feeds.
- [`../explanation/default-switch-and-deprecation.md`](../explanation/default-switch-and-deprecation.md)
  — the per-wave readiness matrix and the design rationale.
- [`../how-to/hardware-smoke-walkthrough.md`](../how-to/hardware-smoke-walkthrough.md)
  — the W20 hardware smoke that writes `w5Fu.json` / `w6Fu.json`.
- [`wave-evidence-schema.json`](./wave-evidence-schema.json) —
  machine-readable JSON Schema companion to this document.
- [`../../nixos-modules/options-daemon.nix`](../../nixos-modules/options-daemon.nix)
  — `validationEvidencePresent`, the cargo-checked predicate this
  doc mirrors.
