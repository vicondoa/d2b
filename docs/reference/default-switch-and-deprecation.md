# Default switch and deprecation (post-clean-break)

> **Status: historical landing page.** The rollout this file used to
> describe is closed. `nixling.daemonExperimental.enable` now
> defaults to `true`, the legacy bash CLI is gone, and there are no
> framework-declared per-VM `nixling@<vm>.service` or
> `microvm@<vm>.service` templates. There is no longer a "default
> mode" vs "native-only mode" axis, and there is no multi-step
> deprecation timeline to track. See
> [ADR 0015 — daemon-only clean break](../adr/0015-daemon-only-clean-break.md)
> for the rationale, alternatives considered, and rollback limits.
>
> This page is kept at its original URL so that historical
> CHANGELOG entries, AGENTS.md references, and code comments
> (`nixos-modules/options-daemon.nix`,
> `packages/nixling/src/host_validate.rs`,
> `docs/reference/wave-evidence-schema.md`,
> `docs/reference/host-validate.md`) continue to resolve. Active
> CLI surface lives in [`cli-contract.md`](./cli-contract.md).

## What this page still covers

After the clean break, the only contract worth recording here is:

1. The **post-clean-break per-verb matrix** — every CLI verb ships
   exactly one path (daemon-native or pure Rust). The "legacy bash
   path" column collapses to a single `no` cell.
2. The **auto-flip gate** — still the mechanism that decides whether
   `nixling.daemonExperimental.enable` evaluates to `true` on a
   fresh consumer host. It is no longer a rollout valve; it is now a
   guard that refuses to flip on a host that has not recorded the
   required evidence files.
3. Cross-references to the docs that own the live surface
   (`cli-contract.md`, `wave-evidence-schema.md`, ADR 0015,
   `host-validate.md`).

Anything else that used to live on this page — the three-mode
bridge, the `NIXLING_NATIVE_ONLY` /
`NIXLING_LEGACY_BASH_OPT_IN` escape hatches, and the staged bash
warning / fail-loud / removal calendar — is gone with the clean
break.

## Post-clean-break compatibility matrix

Every public CLI verb ships exactly one path. The fallback column
exists only so that downstream readers cross-referencing older
prose can see at a glance that the answer is now uniformly
**no**[^bash-rm].

[^bash-rm]: The bash CLI binary (`scripts/nixling`,
    `nixos-modules/cli.nix`) and the per-VM
    `nixling@<vm>.service` / `microvm@<vm>.service` templates are
    gone; see [ADR 0015 § Scope](../adr/0015-daemon-only-clean-break.md).
    `vm start/stop/restart/list` now use only the daemon-native
    path, and there is no bash dispatcher fallback.

| Surface | Path today | Legacy bash path kept? | Notes |
| --- | --- | --- | --- |
| `list`, `status`, `status --check-bridges`, `audit`, `auth status`, `host check`, `host doctor` | daemon-native (read-only) | no | Read-only daemon / broker query surfaces. |
| `vm start`, `vm stop`, `vm restart`, `vm list` | daemon-native | no | Failures surface as typed daemon / broker envelopes (`daemon-down` exit 1, `not-yet-implemented` exit 78). |
| `up`, `down`, `restart` (top-level aliases) | daemon-native | no | First-class aliases for `vm start/stop/restart`. Same envelope shape. |
| `host prepare`, `host destroy` | daemon-native | no | Daemon-side host-prepare DAG. |
| `host install` | daemon-native (broker `RunHostInstall`) | no | — |
| `host validate` | daemon-native | no | Writes the per-wave evidence files this page references below. See [`host-validate.md`](./host-validate.md). |
| `build`, `generations` | pure Rust planner | no | Non-destructive, no daemon required. |
| `switch`, `boot`, `test`, `rollback`, `gc` | daemon-native (broker `RunActivation` / `RunGc`) | no | — |
| `keys list`, `keys show` | daemon-native (read-only) | no | — |
| `keys rotate`, `trust`, `rotate-known-host` | daemon-native (broker `RunKeysRotate` / `RunHostKeyTrust` / `RunRotateKnownHost`) | no | — |
| `migrate` | daemon-native (broker `RunMigrate`) | no | Dry-run analysis is local Rust; `--apply` goes through the broker. |
| `usb attach`, `usb detach`, `usb probe` | daemon-native | no | USBIP live executors via the broker; attach binds/locks the busid before applying the firewall carve-out and ensuring per-env backend/proxy runners. |
| `console`, `audio status`, `audio mic`, `audio speaker`, `audio off` | daemon-native | no | Rust CLI owns the surface; there is no bash helper fallback. |
| `debug bundle` | daemon-native | no | — |

There is no `NIXLING_NATIVE_ONLY` and no `NIXLING_LEGACY_BASH_OPT_IN`.
Both environment variables are unrecognised; setting them has no
current effect.

For the authoritative per-verb argv, exit codes, JSON shape, and
signal semantics, see [`cli-contract.md`](./cli-contract.md). For
the typed envelope catalog, see
[`error-codes.md`](./error-codes.md).

## Auto-flip gate (still live)

`nixling.daemonExperimental.enable` defaults to `true` on a fresh
consumer host only when every wave in the **flip-gate subset**
`{w4Fu, w5Fu, w6Fu, w7Fu, w8Fu, w9Fu, p0, p0Fu, p1, p2, p3, p4}`
satisfies ALL THREE of:

1. `nixling.defaultSwitchReadiness.<wave>.implemented = true`,
2. `nixling.defaultSwitchReadiness.<wave>.validated = true`, AND
3. an evidence file `<defaultFlipEvidenceDir>/<wave>.json` exists
   carrying the canonical `{wave, timestamp, operatorSignature}`
   schema — see [`wave-evidence-schema.md`](./wave-evidence-schema.md)
   for the full schema and validator.

`defaultFlipEvidenceDir` defaults to `/var/lib/nixling/validated`
and is overridable via
`nixling.daemonExperimental.defaultFlipEvidenceDir` for tests.
Waves outside the flip-gate subset (e.g. `p5`, `p6`, `p7`) do not
gate the auto-flip but do participate in `allReady` for other
consumers.

Explicit operator config still wins either way:
`lib.mkDefault false`, `lib.mkForce false`, and `lib.mkForce true`
all override the computed default unchanged. Per ADR 0015,
`lib.mkForce false` no longer selects "legacy bash/systemd mode"
because that mode no longer exists; it simply disables the
nixling-managed bits that gate on `daemonExperimental.enable` and
leaves the operator responsible for not invoking any nixling verb
that requires a running daemon.

The operator-facing one-command preflight that materialises the
evidence files is `nixling host validate --apply`; see
[`host-validate.md`](./host-validate.md).

## Cross-references

- [ADR 0015 — daemon-only clean break](../adr/0015-daemon-only-clean-break.md)
  — rationale, alternatives considered, why no compat /
  deprecation path is acceptable, rollback limits.
- [`cli-contract.md`](./cli-contract.md) — authoritative per-verb
  surface, exit codes, JSON shapes, signal semantics.
- [`error-codes.md`](./error-codes.md) — typed envelope catalog.
- [`wave-evidence-schema.md`](./wave-evidence-schema.md) — JSON
  schema for the evidence files this page's gate consumes.
- [`host-validate.md`](./host-validate.md) — the verb that writes
  the evidence files.
- [`../explanation/default-switch-and-deprecation.md`](../explanation/default-switch-and-deprecation.md)
  — historical record of the rollout shape that preceded the clean
  break.
