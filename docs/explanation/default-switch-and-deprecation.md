# Default switch and deprecation — historical record

> **Status: historical record.** The daemon-experimental rollout this
> page describes is closed. `nixling.daemonExperimental.enable` now
> flips through the evidence gate, the bash CLI is gone, and there
> are no framework-declared per-VM lifecycle templates. See
> [ADR 0015 — daemon-only clean break](../adr/0015-daemon-only-clean-break.md)
> for the rationale, alternatives considered, consequences, and
> rollback limits.
>
> This page is preserved because (a) historical CHANGELOG entries
> link to it, (b) the auto-flip gate semantics it documents still
> drive how `nixling.daemonExperimental.enable` resolves on fresh
> consumer hosts, and (c) the "why we did NOT keep a bash
> coexistence path" framing is easier to read against the rollout
> shape that came before it.
>
> The active per-verb surface lives in the reference companion
> [`../reference/default-switch-and-deprecation.md`](../reference/default-switch-and-deprecation.md)
> (post-clean-break landing page) and in
> [`../reference/cli-contract.md`](../reference/cli-contract.md)
> (authoritative CLI contract).

## What was deprecated, and what replaced it

| Concept (historical) | Replaced by (current behavior) |
| --- | --- |
| `nixling.daemonExperimental.enable = false` as the shipped default | Auto-flip to `true` once the W18 flip-gate subset is green (see below). Explicit operator override in either direction still wins. |
| W14c three-mode bridge (`default` daemon-first-with-bash-fallback / `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1`) | Single daemon-native path. Both environment variables are unrecognised after P6. |
| The bash CLI (`scripts/nixling`, `nixos-modules/cli.nix`) shipped alongside the Rust CLI as a fallback runtime | Bash CLI deleted in P6 (`ph6-p6-cli-nix-migrations`, `ph6-remove-systemd-emission`). Rust CLI is the only CLI. |
| Per-VM `nixling@<vm>.service` and `microvm@<vm>.service` templates as the lifecycle substrate | Daemon-supervised lifecycle (`nixlingd::supervisor` + per-VM DAG executor). The per-VM systemd templates are deleted in P6. |
| `W10-fu + 30/60/90/180 days` bash deprecation calendar (warning → fail-loud → binary removed) | Clean break. The clean-break framing is the deprecation: there is no warn-then-remove cadence because there is no coexistence period. |
| `nixling.vms.<vm>.supervisor` option (per-VM choice between systemd backend and daemon backend) | Retained in v1.0 source (default `"systemd"`) for backward-compat with consumer flakes pinning pre-v1.0 manifests; the v1.0-intended hard removal + eval-time rejection assertion is **scheduled for v1.1-P2** (see ADR 0015 § Decision and the v1.1 plan). Setting `supervisor = "nixlingd"` requires `nixling.daemonExperimental.enable = true`. |
| ADR 0007 (bash coexistence + migration plan) | Superseded by ADR 0015. ADR 0007 remains in the tree as historical context. |

## Why a clean break instead of the original deprecation cycle

The original W10 plan envisioned a multi-month coexistence window:
the bash CLI would warn at `+30 days`, fail loudly at `+90 days`,
and only be removed at `+180 days` (the nominal 1.0 cut). That
shape made sense while the daemon-native path was still earning
trust per-wave.

By the end of P4 + P5 every operator-facing verb already ran
daemon-native end-to-end on hosts that had passed the W18
flip-gate, and the bash fallback was a strict superset of the
daemon path's failure modes (it bypassed the broker's audit log,
re-introduced root-level lifecycle decisions, and could not be
sandboxed under the minijail profiles W17/P1 ship). Maintaining a
parallel bash runtime for the deprecation window therefore meant
maintaining a *less safe* code path purely so operators could
choose it. ADR 0015 records the panel's decision that the operator
ergonomics of a "warn-then-remove" cycle were not worth the
security and audit-coverage regressions implied by carrying the
bash runtime past P5.

The trade-off explicitly accepted by ADR 0015:

- **Lost:** the ability for an operator to roll back to a known-good
  bash CLI on the same host after P6 lands. The remediation path is
  to pin to the last pre-P6 nixling tag (the bash runtime is still
  in that revision's tree) rather than mix bash and daemon paths
  on the same host.
- **Lost:** the `+30 / +90 / +180 day` cadence that gave external
  documentation, internal training, and consumer flake pins a
  forward-dated warning. The clean break ships with a CHANGELOG
  entry, an AGENTS.md rewrite, and ADR 0015 — no in-CLI
  deprecation warning, because there is no in-CLI legacy code path
  to warn from.
- **Kept:** every operator config knob that selects daemon vs not
  (`nixling.daemonExperimental.enable` itself, the W18 gate, the
  `validated` evidence machinery). What changed is what the "off"
  side of that knob means: pre-P6 it selected the bash runtime;
  post-P6 it disables the daemon-managed lifecycle bits and leaves
  the operator responsible for not invoking the daemon-native
  verbs.

## W18 auto-flip semantics (still live)

W18 turns `nixling.daemonExperimental.enable` into a computed
default: it evaluates to `true` only when every wave in the
**W18 flip-gate subset** has both readiness bits green AND a
matching evidence file on disk. Otherwise the default remains
`false`.

### Flip-gate subset (P5 narrowing)

The gate iterates over a fixed subset of `defaultSwitchReadiness`
waves — the subset that has shipped by the time the W18 flip is
considered. The full schema also carries `p5`, `p6`, `p7` records;
those are intentionally excluded from the gate because they
describe work that happens AFTER the flip itself, and requiring
them would deadlock the auto-flip.

| Wave | Origin | Evidence file |
| --- | --- | --- |
| `w4Fu` | W12 / W14 headless supervisor + non-bootstrap broker path | `<defaultFlipEvidenceDir>/w4Fu.json` |
| `w5Fu` | W17 minijail profiles + GPU/audio/video argv generators | `<defaultFlipEvidenceDir>/w5Fu.json` |
| `w6Fu` | W13 USBIP live executors + per-busid lock | `<defaultFlipEvidenceDir>/w6Fu.json` |
| `w7Fu` | W7b store-lifecycle verbs + admin auth | `<defaultFlipEvidenceDir>/w7Fu.json` |
| `w8Fu` | W14 keys / trust / rotate-known-host live wiring | `<defaultFlipEvidenceDir>/w8Fu.json` |
| `w9Fu` | W15 host install + migrate live broker ops (incl. W15-fu1) | `<defaultFlipEvidenceDir>/w9Fu.json` |
| `p0` | Daemon-only foundation (socket-activated broker, bundle hash verify, canonical `/run/nixling`) | `<defaultFlipEvidenceDir>/p0.json` |
| `p0Fu` | P0 follow-up: cgroup delegation sequence + per-artifact hash verification | `<defaultFlipEvidenceDir>/p0Fu.json` |
| `p1` | Per-role minijail profiles + byte-parity argv generators | `<defaultFlipEvidenceDir>/p1.json` |
| `p2` | Daemon-side host-prep + ownership matrix + `manifestVersion = 3` + daemon autostart | `<defaultFlipEvidenceDir>/p2.json` |
| `p3` | Retire host singletons + daemon health endpoint | `<defaultFlipEvidenceDir>/p3.json` |
| `p4` | `vm start/stop/restart/list` daemon-native end-to-end + desktop wrapper | `<defaultFlipEvidenceDir>/p4.json` |

`defaultFlipEvidenceDir` is the
`nixling.daemonExperimental.defaultFlipEvidenceDir` option; its
default is `/var/lib/nixling/validated`. The option is overridable
mainly for the regression test
(`tests/daemon-default-compat-eval.sh`); operator hosts should leave it
at the default.

### Per-wave gate semantics

For each wave `W` in the flip-gate subset, the gate is green iff
ALL three of:

1. `nixling.defaultSwitchReadiness.<W>.implemented = true` (the
   code has shipped in-tree);
2. `nixling.defaultSwitchReadiness.<W>.validated = true` (the
   operator has recorded host-local evidence that the wave was
   exercised successfully); and
3. `<defaultFlipEvidenceDir>/<W>.json` exists on disk and parses
   as a JSON object carrying `wave`, `timestamp`, and
   `operatorSignature` fields.

The `validated = true` ↔ evidence-file invariant is enforced as a
hard eval assertion for **every** readiness wave (not just the
flip-gate subset), so an operator cannot accidentally claim a wave
is validated without the proof artifact present. Setting
`validated = true` with `implemented = false` is likewise a hard
eval failure.

### Operator override semantics (preserved)

Operator overrides still win in both directions:

- `nixling.daemonExperimental.enable = lib.mkForce true` — opt
  into daemon mode before the flip gate is fully green. Same
  semantics as before.
- `nixling.daemonExperimental.enable = lib.mkForce false` — opt
  out. **Semantics changed post-P6.** Pre-P6 this selected the
  legacy bash/systemd runtime. Post-P6 the legacy runtime no
  longer exists; setting this to `false` simply disables the
  daemon-managed lifecycle bits and leaves the operator
  responsible for not invoking daemon-native verbs. There is no
  third runtime to fall back to.

The `mkDefault` / `mkForce` priority semantics of the underlying
`lib.mkOption` are preserved — this module declares only the
default expression, so any operator-side assignment behaves
exactly as the NixOS option-merging rules describe.

## Historical compatibility table (W10 main, pre-clean-break)

The pre-P6 contract was a per-verb matrix that recorded, for every
public CLI verb, which paths existed (bash, native daemon, native
`--apply` wired live) and which follow-up wave promoted the
native `--apply` from "typed refusal" to "live". The table is
preserved verbatim below so historical CHANGELOG / commit
references resolve. Every row's "Bash" column reads as
**deleted in P6** as of this revision.

| Verb | Bash (pre-P6) | Native | Live --apply (final) | Promoting wave |
| --- | --- | --- | --- | --- |
| `list` | ✅ | ✅ | ✅ (read-only) | — |
| `status` | ✅ | ✅ | ✅ (read-only) | — |
| `status --check-bridges` | ✅ | ✅ | ✅ (read-only) | — |
| `audit` | ✅ | ✅ | ✅ (read-only) | — |
| `auth status` | ✅ | ✅ | ✅ (read-only) | — |
| `host check` | ✅ | ✅ | ✅ (read-only) | — |
| `host prepare` | ✅ | ✅ | ✅ (P2 daemon-side host-prep) | W4-fu / P2 |
| `host destroy` | ✅ | ✅ | ✅ (P2 daemon-side host-prep) | W4-fu / P2 |
| `host doctor` | ✅ | ✅ | ✅ (read-only) | — |
| `host install` | — | ✅ (W15) | ✅ (daemon → broker `RunHostInstall`) | — |
| `vm start` | ✅ (`up`) | ✅ (W4) | ✅ (daemon-native; P4 retired bash bridge) | W4-fu → P4 |
| `vm stop` | ✅ (`down`) | ✅ (W4) | ✅ (daemon-native; P4 retired bash bridge) | W4-fu → P4 |
| `vm restart` | ✅ | ✅ (W4) | ✅ (daemon-native; P4 retired bash bridge) | W4-fu → P4 |
| `vm list` | — | ✅ (W4) | ✅ (daemon-native; P4 promoted from placeholder) | W4-fu → P4 |
| `build` | ✅ | ✅ (W7) | n/a (non-destructive) | W7b-fu |
| `generations` | ✅ | ✅ (W7) | n/a (non-destructive) | W7b-fu |
| `switch` | ✅ | ✅ (W7) | ✅ (broker `RunActivation`) | W7-fu |
| `boot` | ✅ | ✅ (W7) | ✅ (broker `RunActivation`) | W7-fu |
| `test` | ✅ | ✅ (W7) | ✅ (broker `RunActivation`) | W7-fu |
| `rollback` | ✅ | ✅ (W7) | ✅ (broker `RunActivation`) | W7-fu |
| `gc` | ✅ | ✅ (W7) | ✅ (broker `RunGc`; admin-auth) | W7c-fu |
| `keys list` | ✅ | ✅ (W8) | ✅ (read-only) | — |
| `keys show` | ✅ | ✅ (W8) | ✅ (read-only) | — |
| `keys rotate` | ✅ | ✅ (W8) | ✅ (broker `RunKeysRotate`; admin-auth) | W8-fu |
| `rotate-known-host` | ✅ | ✅ (W8) | ✅ (broker `RunRotateKnownHost`; admin-auth) | W8-fu |
| `trust` | ✅ | ✅ (W8) | ✅ (broker `RunHostKeyTrust`; admin-auth) | W8-fu |
| `audio on\|off` | ✅ | ✅ (P-phase) | ✅ (daemon-native; P6 retired bash shim) | W5-fu → P-phase |
| `audio mic\|speaker on\|off` | ✅ | ✅ (P-phase) | ✅ (daemon-native; P6 retired bash shim) | W5-fu → P-phase |
| `audio status` | ✅ | ✅ (P-phase) | ✅ (daemon-native; P6 retired bash shim) | W5-fu → P-phase |
| `console <vm>` | ✅ | ✅ (P-phase) | ✅ (daemon-native; P6 retired bash shim) | W7-fu → P-phase |
| `debug bundle` | ✅ | ✅ (P-phase) | ✅ (daemon-native; P6 retired bash shim) | W6-fu → P-phase |
| `usb <vm>` | ✅ | ✅ (W6/W13) | ✅ (broker `UsbipBindFirewallRule` + per-busid lock) | W6-fu |
| `migrate` | ✅ | ✅ (W15) | ✅ (daemon → broker `RunMigrate`) | — |

Legend (historical): `✅` = implemented and live in that revision;
`—` = no follow-up planned (verb is read-only, daemon-only from
the start, or didn't apply). The "Bash" column reads as deleted
in P6 across every row.

## References

- [ADR 0015 — daemon-only clean break](../adr/0015-daemon-only-clean-break.md)
- [ADR 0007 — bash coexistence and migration (superseded)](../adr/0007-bash-coexistence-and-migration.md)
- [`../reference/default-switch-and-deprecation.md`](../reference/default-switch-and-deprecation.md)
  — post-clean-break landing page (active surface).
- [`../reference/cli-contract.md`](../reference/cli-contract.md)
  — authoritative per-verb CLI surface.
- [`../reference/wave-evidence-schema.md`](../reference/wave-evidence-schema.md)
  — JSON schema for the W18 evidence files.
- [`../reference/host-validate.md`](../reference/host-validate.md)
  — the `nixling host validate` verb that writes those files.
- [Daemon experimental mode (W2)](daemon-experimental.md)
- [Daemon lifecycle (W4 main)](daemon-lifecycle.md)
