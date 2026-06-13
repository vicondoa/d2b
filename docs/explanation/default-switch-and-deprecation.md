# Default switch and deprecation — historical record

> **Status: historical record.** The daemon-experimental rollout this
> page describes is closed. `nixling.daemonExperimental.enable` now
> defaults `true` and no longer flips through an evidence gate, but it
> still functionally gates the daemon control plane (setting it `false`
> reverts the host to the unsupported pre-daemon legacy state). The
> bash CLI is gone, and
> there are no framework-declared per-VM lifecycle templates. See
> [ADR 0015 — daemon-only clean break](../adr/0015-daemon-only-clean-break.md)
> for the rationale, alternatives considered, consequences, and
> rollback limits.
>
> This page is preserved because (a) historical CHANGELOG entries
> link to it, (b) the per-wave evidence-gate semantics it documents
> still gate the `nixling.defaultSwitchReadiness.<wave>.validated`
> assertion (even though they no longer drive
> `nixling.daemonExperimental.enable`), and (c) the "why we did NOT
> keep a bash coexistence path" framing is easier to read against the
> rollout shape that came before it.
>
> The active per-verb surface lives in the reference companion
> [`../reference/default-switch-and-deprecation.md`](../reference/default-switch-and-deprecation.md)
> (post-clean-break landing page) and in
> [`../reference/cli-contract.md`](../reference/cli-contract.md)
> (authoritative CLI contract).

## What was deprecated, and what replaced it

| Concept (historical) | Replaced by (current behavior) |
| --- | --- |
| `nixling.daemonExperimental.enable = false` as the shipped default | Now `default = true`. It is no longer computed from wave readiness (no longer evidence-auto-flipped), but it still functionally gates the daemon control plane — setting it `false` reverts the host to the unsupported pre-daemon legacy state. The per-wave evidence files instead gate the `nixling.defaultSwitchReadiness.<wave>.validated` assertion (see below). |
| The three-mode bridge (`default` daemon-first-with-bash-fallback / `NIXLING_NATIVE_ONLY=1` / `NIXLING_LEGACY_BASH_OPT_IN=1`) | Single daemon-native path. Both environment variables are unrecognised after the clean break. |
| The bash CLI (`scripts/nixling`, `nixos-modules/cli.nix`) shipped alongside the Rust CLI as a fallback runtime | Bash CLI deleted in the daemon-only clean break. Rust CLI is the only CLI. |
| Per-VM `nixling@<vm>.service` and `microvm@<vm>.service` templates as the lifecycle substrate | Daemon-supervised lifecycle (`nixlingd::supervisor` + per-VM DAG executor). The per-VM systemd templates are deleted in the clean break. |
| The original 30/60/90/180-day bash deprecation calendar (warning → fail-loud → binary removed) | Clean break. The clean-break framing is the deprecation: there is no warn-then-remove cadence because there is no coexistence period. |
| `nixling.vms.<vm>.supervisor` option (per-VM choice between systemd backend and daemon backend) | Removed. The option no longer exists; setting it fails eval with a typed message (`nixos-modules/assertions.nix`). Every enabled VM is daemon-supervised (see ADR 0015 § Decision). |
| ADR 0007 (bash coexistence + migration plan) | Superseded by ADR 0015. ADR 0007 remains in the tree as historical context. |

## Why a clean break instead of the original deprecation cycle

The original deprecation plan envisioned a multi-month coexistence window:
the bash CLI would warn at `+30 days`, fail loudly at `+90 days`,
and only be removed at `+180 days` (the nominal 1.0 cut). That
shape made sense while the daemon-native path was still earning
trust per-wave.

By the time the daemon-native path shipped, every operator-facing verb
already ran
daemon-native end-to-end on hosts that had passed the
flip-gate, and the bash fallback was a strict superset of the
daemon path's failure modes (it bypassed the broker's audit log,
re-introduced root-level lifecycle decisions, and could not be
sandboxed under the minijail profiles the framework ships). Maintaining a
parallel bash runtime for the deprecation window therefore meant
maintaining a *less safe* code path purely so operators could
choose it. ADR 0015 records the panel's decision that the operator
ergonomics of a "warn-then-remove" cycle were not worth the
security and audit-coverage regressions implied by carrying the
bash runtime through the deprecation window.

The trade-off explicitly accepted by ADR 0015:

- **Lost:** the ability for an operator to roll back to a known-good
  bash CLI on the same host after the clean break lands. The remediation path is
  to pin to the last pre-clean-break nixling tag (the bash runtime is still
  in that revision's tree) rather than mix bash and daemon paths
  on the same host.
- **Lost:** the `+30 / +90 / +180 day` cadence that gave external
  documentation, internal training, and consumer flake pins a
  forward-dated warning. The clean break ships with a CHANGELOG
  entry, an AGENTS.md rewrite, and ADR 0015 — no in-CLI
  deprecation warning, because there is no in-CLI legacy code path
  to warn from.
- **Kept:** every operator config knob that selects daemon vs not
  (`nixling.daemonExperimental.enable` itself, the flip gate, the
  `validated` evidence machinery). What changed is what the "off"
  side of that knob means: before the clean break it selected the bash runtime;
  after it disables the daemon-managed lifecycle bits and leaves
  the operator responsible for not invoking the daemon-native
  verbs.

## Flip-gate subset and per-wave evidence (evidence gate still live)

The daemon-experimental flip originally turned
`nixling.daemonExperimental.enable` into a
computed default that evaluated to `true` only when every wave in the
**flip-gate subset** had both readiness bits green AND a matching
evidence file on disk. That coupling is **no longer wired**:
`nixling.daemonExperimental.enable` now defaults `true` and is no
longer evidence-auto-flipped, but it still functionally gates the
daemon control plane (setting it `false` reverts the host to the
unsupported pre-daemon legacy state). The flip-gate subset is still
computed in
`nixos-modules/options-daemon.nix`, and the per-wave evidence files
are still live — but what they gate today is the per-wave
`nixling.defaultSwitchReadiness.<wave>.validated = true` eval
assertion (fail-closed without the evidence file), not the
`daemonExperimental.enable` default. The subset and evidence schema
below remain accurate for that assertion.

### Flip-gate subset

The gate iterates over a fixed subset of `defaultSwitchReadiness`
waves — the subset that has shipped by the time the flip is
considered. The full schema also carries `p5`, `p6`, `p7` records;
those are intentionally excluded from the gate because they
describe work that happens AFTER the flip itself, and requiring
them would deadlock the auto-flip.

| Wave | Capability the evidence covers | Evidence file |
| --- | --- | --- |
| `w4Fu` | headless supervisor + non-bootstrap broker path | `<defaultFlipEvidenceDir>/w4Fu.json` |
| `w5Fu` | minijail profiles + GPU/audio/video argv generators | `<defaultFlipEvidenceDir>/w5Fu.json` |
| `w6Fu` | USBIP live executors + per-busid lock | `<defaultFlipEvidenceDir>/w6Fu.json` |
| `w7Fu` | store-lifecycle verbs + admin auth | `<defaultFlipEvidenceDir>/w7Fu.json` |
| `w8Fu` | keys / trust / rotate-known-host live wiring | `<defaultFlipEvidenceDir>/w8Fu.json` |
| `w9Fu` | host install + migrate live broker ops | `<defaultFlipEvidenceDir>/w9Fu.json` |
| `p0` | daemon-only foundation (socket-activated broker, bundle hash verify, canonical `/run/nixling`) | `<defaultFlipEvidenceDir>/p0.json` |
| `p0Fu` | cgroup delegation sequence + per-artifact hash verification (foundation follow-up) | `<defaultFlipEvidenceDir>/p0Fu.json` |
| `p1` | per-role minijail profiles + byte-parity argv generators | `<defaultFlipEvidenceDir>/p1.json` |
| `p2` | daemon-side host-prep + ownership matrix + manifest version bump + daemon autostart | `<defaultFlipEvidenceDir>/p2.json` |
| `p3` | retire host singletons + daemon health endpoint | `<defaultFlipEvidenceDir>/p3.json` |
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
  out. **Semantics changed at the clean break.** Before it, this selected the
  legacy bash/systemd runtime. After it, the legacy runtime no
  longer exists; setting this to `false` simply disables the
  daemon-managed lifecycle bits and leaves the operator
  responsible for not invoking daemon-native verbs. There is no
  third runtime to fall back to.

The `mkDefault` / `mkForce` priority semantics of the underlying
`lib.mkOption` are preserved — this module declares only the
default expression, so any operator-side assignment behaves
exactly as the NixOS option-merging rules describe.

## Historical compatibility table (pre-clean-break)

The pre-clean-break contract was a per-verb matrix that recorded, for every
public CLI verb, which paths existed (bash, native daemon, native
`--apply` wired live). The table is
preserved below so historical CHANGELOG / commit
references resolve. Every row's "Bash" column reads as
**deleted** as of this revision.

| Verb | Bash (legacy) | Native | Live --apply (final) |
| --- | --- | --- | --- |
| `list` | ✅ | ✅ | ✅ (read-only) |
| `status` | ✅ | ✅ | ✅ (read-only) |
| `status --check-bridges` | ✅ | ✅ | ✅ (read-only) |
| `audit` | ✅ | ✅ | ✅ (read-only) |
| `auth status` | ✅ | ✅ | ✅ (read-only) |
| `host check` | ✅ | ✅ | ✅ (read-only) |
| `host prepare` | ✅ | ✅ | ✅ (daemon-side host-prep) |
| `host destroy` | ✅ | ✅ | ✅ (daemon-side host-prep) |
| `host doctor` | ✅ | ✅ | ✅ (read-only) |
| `host install` | — | ✅ | ✅ (daemon → broker `RunHostInstall`) |
| `vm start` | ✅ (`up`) | ✅ | ✅ (daemon-native; retired bash bridge) |
| `vm stop` | ✅ (`down`) | ✅ | ✅ (daemon-native; retired bash bridge) |
| `vm restart` | ✅ | ✅ | ✅ (daemon-native; retired bash bridge) |
| `vm list` | — | ✅ | ✅ (daemon-native; promoted from placeholder) |
| `build` | ✅ | ✅ | n/a (non-destructive) |
| `generations` | ✅ | ✅ | n/a (non-destructive) |
| `switch` | ✅ | ✅ | ✅ (broker `RunActivation`) |
| `boot` | ✅ | ✅ | ✅ (broker `RunActivation`) |
| `test` | ✅ | ✅ | ✅ (broker `RunActivation`) |
| `rollback` | ✅ | ✅ | ✅ (broker `RunActivation`) |
| `gc` | ✅ | ✅ | ✅ (broker `RunGc`; admin-auth) |
| `keys list` | ✅ | ✅ | ✅ (read-only) |
| `keys show` | ✅ | ✅ | ✅ (read-only) |
| `keys rotate` | ✅ | ✅ | ✅ (broker `RunKeysRotate`; admin-auth) |
| `rotate-known-host` | ✅ | ✅ | ✅ (broker `RunRotateKnownHost`; admin-auth) |
| `trust` | ✅ | ✅ | ✅ (broker `RunHostKeyTrust`; admin-auth) |
| `audio on\|off` | ✅ | ✅ | ✅ (daemon-native; retired bash shim) |
| `audio mic\|speaker on\|off` | ✅ | ✅ | ✅ (daemon-native; retired bash shim) |
| `audio status` | ✅ | ✅ | ✅ (daemon-native; retired bash shim) |
| `console <vm>` | ✅ | ✅ | ✅ (daemon-native; retired bash shim) |
| `debug bundle` | ✅ | ✅ | ✅ (daemon-native; retired bash shim) |
| `usb <vm>` | ✅ | ✅ | ✅ (broker `UsbipBindFirewallRule` + per-busid lock) |
| `migrate` | ✅ | ✅ | ✅ (daemon → broker `RunMigrate`) |

Legend (historical): `✅` = implemented and live in that revision;
`—` = the verb is read-only, daemon-only from
the start, or didn't apply. The "Bash" column reads as deleted
across every row.

## References

- [ADR 0015 — daemon-only clean break](../adr/0015-daemon-only-clean-break.md)
- [ADR 0007 — bash coexistence and migration (superseded)](../adr/0007-bash-coexistence-and-migration.md)
- [`../reference/default-switch-and-deprecation.md`](../reference/default-switch-and-deprecation.md)
  — post-clean-break landing page (active surface).
- [`../reference/cli-contract.md`](../reference/cli-contract.md)
  — authoritative per-verb CLI surface.
- [`../reference/wave-evidence-schema.md`](../reference/wave-evidence-schema.md)
  — JSON schema for the per-wave evidence files.
- [`../reference/host-validate.md`](../reference/host-validate.md)
  — the `nixling host validate` verb that writes those files.
- [Daemon experimental mode](daemon-experimental.md)
- [Daemon lifecycle](daemon-lifecycle.md)
