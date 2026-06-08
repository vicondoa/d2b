# How to migrate a nixling host from v0.4.x to v1.0

This guide is for **operators** running nixling v0.4.x (the last pre-1.0
release with the `legacy-systemd` / `daemon-experimental` /
`daemon-default` three-mode bridge) who are upgrading to v1.0 (the
daemon-only end-state). It does **not** cover fresh installs — those
follow [`headless-alpha-walkthrough.md`](./headless-alpha-walkthrough.md)
or [`install-nixos-tier1.md`](./install-nixos-tier1.md).

If you are coming from raw `microvm.nix` (no nixling at all) read
[`migrating-from-microvm.md`](./migrating-from-microvm.md) instead;
that guide stays scoped to new installs.

## What changes in v1.0

[ADR 0015](../adr/0015-daemon-only-clean-break.md) is the binding
architectural decision. The short version:

- `nixlingd.service` and `nixling-priv-broker.{service,socket}` are
  the **only** persistent root surfaces the framework declares.
- Per-VM systemd templates (`nixling@<vm>`, `microvm@<vm>`,
  `nixling-<vm>-{gpu,snd,video,swtpm,store-sync}`, the
  `microvm-{tap-interfaces,pci-devices,set-booted,virtiofsd}@`
  templates, `nixling-known-hosts-refresh@`,
  `nixling-vfsd-watchdog@.{service,timer}`,
  `nixling-otel-relay@<vm>`, and the per-env
  `nixling-sys-<env>-usbipd-{proxy,backend}.{service,socket}`) are
  deleted.
- Host singletons (`nixling-{ch-exporter,otel-host-bridge,
  net-route-preflight,audit-check}.service`,
  `nixling-audit-check.timer`, the `microvms.target` aggregator)
  are deleted; their work moved into `nixlingd` or the broker.
- The bash CLI (`nixling-legacy` / `share/nixling/cli.sh` / the
  `cli.nix` builder) is deleted. The Rust `nixling` binary is the
  only CLI surface.
- The W14c bash fallback bridge inside the Rust CLI is removed; so
  is `NIXLING_LEGACY_BASH_OPT_IN` / `NIXLING_LEGACY_CLI`.
  `NIXLING_NATIVE_ONLY` is preserved as a no-op for back-compat.
- The polkit allowlist for per-VM units is retired; `nixling-launchers`
  group membership + `SO_PEERCRED` on `public.sock` is the only
  lifecycle authorisation surface.
- The manifest contract bumps from `manifestVersion: 2` to
  `manifestVersion: 3`. There is no auto-rewriter — `ManifestV04::from_slice`
  rejects v2 bundles outright with the typed `manifest-parse-error`
  / `manifest-version-mismatch` envelope.

There is **no deprecation window**. v0.5 was skipped; the v0.4.x →
v1.0.0 boundary deletes every legacy surface in one cut. Operators
who skip this guide will see runtime `manifest-version-mismatch`
errors. (The `supervisor` option's v1.0-intended eval-time
assertion was deferred to v1.1 backlog per ADR 0015 § Decision;
v1.0 retains the option for backward-compat with consumer flakes
pinning pre-v1.0 manifests.)

## Per-phase deliverable docs

These reference docs cover the individual cut-overs in depth; this
guide cross-links them and gives operators the migration recipe.

- [ADR 0015 — Daemon-only clean break](../adr/0015-daemon-only-clean-break.md)
- [`docs/reference/host-validate.md`](../reference/host-validate.md)
  — the `nixling host validate` umbrella preflight (P5).
- [`docs/reference/cli-contract.md`](../reference/cli-contract.md)
  — the post-clean-break Rust CLI surface.
- [`docs/reference/default-switch-and-deprecation.md`](../reference/default-switch-and-deprecation.md)
  — the post-clean-break compatibility table + W18 auto-flip gate.
- [`docs/reference/privileges.md`](../reference/privileges.md) —
  daemon-only broker op catalogue + retired-unit obituary tables.
- [`docs/reference/manifest-schema.md`](../reference/manifest-schema.md)
  + [`docs/reference/manifest-schema.json`](../reference/manifest-schema.json)
  — manifest v3 contract.
- [`docs/reference/desktop-wrapper.md`](../reference/desktop-wrapper.md)
  — daemon-native `.desktop` wrapper contract (P4).
- [`docs/explanation/daemon-lifecycle.md`](../explanation/daemon-lifecycle.md)
  — daemon DAG executor, pidfd handoff, supervisor reconciliation.

## Before you begin

Take a known-good system generation:

```bash
sudo nixos-rebuild boot --flake .#myhost   # current v0.4.x generation
sudo /run/current-system/bin/switch-to-configuration boot
```

Note the generation number (`sudo nix-env -p /nix/var/nix/profiles/system --list-generations | tail`)
so you can `--rollback` if anything goes wrong.

Tag your consumer flake at the pre-migration revision:

```bash
git -C /etc/nixos tag pre-nixling-v1
git -C /etc/nixos push --tags    # if your config is in a remote repo
```

Bump the nixling input in your `flake.nix` to v1.0.0 **after** you
have applied every change in §§1–7 below. The `supervisor` option's
v1.0-intended hard removal + eval-time rejection assertion was
deferred to v1.1 backlog (per ADR 0015 § Decision); v1.0 retains
the option for backward-compat with consumer flakes pinning pre-v1.0
manifests.

## 1. Manifest v2 → v3

### Before

`vms.json` carried `_manifest.manifestVersion: 2`. The bash CLI and
the Rust daemon both accepted v2; per-VM systemd-unit reference
fields (e.g. `unitName`, `instanceName`) were still emitted.

### After

`_manifest.manifestVersion: 3`. The per-VM systemd-unit reference
fields are gone (they became meaningless once supervisor mode
shipped). `nixling_core::manifest_v04::MANIFEST_VERSION_CURRENT` is
pinned to `3`; v2 bundles are rejected with the typed
`manifest-parse-error` / `manifest-version-mismatch` envelope.

### Migration steps

The producer (`nixos-modules/manifest.nix`) already pins
`_manifestVersion = 3` on v1.0. You **must** rebuild every host
manifest from source before the daemon will accept the bundle:

```bash
sudo nixos-rebuild build --flake .#myhost
sudo cat /run/current-system/sw/share/nixling/vms.json \
  | jq '._manifest.manifestVersion'   # expect: 3
```

If you vendor the bundle to a sibling host, regenerate it on the
producer host first; never hand-edit `manifestVersion` to `3`
without a fresh derivation, because the rest of the schema also
changed.

### Validation

```bash
nixling host validate --dry-run --json | jq '.waves[] | select(.wave=="p2")'
```

Then, once §§2–7 are complete:

```bash
sudo nixling host validate --apply --wave p2
```

The Layer-1 gate `tests/host-validate-verb-eval.sh` covers schema
parity; the per-wave validator for P2 is
`tests/per-vm-state-ownership-eval.sh` +
`tests/daemon-autostart-eval.sh` + `tests/host-prep-dag-eval.sh`
(see the per-wave validator map in
[`docs/reference/host-validate.md`](../reference/host-validate.md#per-wave-validator-map)).

### Rollback

```bash
sudo nixos-rebuild switch --rollback
```

The previous generation reinstates the v2 producer. If you have
already bumped the nixling flake input, also:

```bash
git -C /etc/nixos checkout pre-nixling-v1 -- flake.nix flake.lock
sudo nixos-rebuild switch --flake .#myhost
```

## 2. Bash CLI removed (`nixling-legacy` / `share/nixling/cli.sh` / `cli.nix`)

### Before

`/run/current-system/sw/bin/nixling-legacy` was the bash entrypoint;
`/run/current-system/sw/share/nixling/cli.sh` was the script body;
`nixos-modules/cli.nix` packaged both plus every per-VM
`nixling-launch-<vm>.desktop` wrapper. Operators could call either
binary; the Rust `nixling` would shell out to bash for any verb the
daemon could not serve.

### After

The bash CLI is gone. `nixling-legacy` no longer exists in
`/run/current-system/sw/bin`. `nixos-modules/cli.nix` is deleted.
The Rust `nixling` binary is the only CLI surface and never shells
out to bash. Every retired bash verb has a typed Rust replacement;
see [`docs/reference/cli-contract.md`](../reference/cli-contract.md).

### Migration steps

Audit operator-facing scripts, cron jobs, `.desktop` files, and
helper aliases for `nixling-legacy` invocations:

```bash
sudo grep -rIn 'nixling-legacy\|share/nixling/cli\.sh' \
  /etc /home /root /var/spool 2>/dev/null | grep -v Binary
```

Replace each call with the equivalent Rust verb:

| v0.4.x (bash) | v1.0 (Rust) |
| --- | --- |
| `nixling-legacy up <vm> -d` | `nixling vm start <vm> --apply` |
| `nixling-legacy down <vm>` | `nixling vm stop <vm> --apply` |
| `nixling-legacy restart <vm>` | `nixling vm restart <vm> --apply` |
| `nixling-legacy list` | `nixling vm list` |
| `nixling-legacy status <vm>` | `nixling status <vm>` |
| `nixling-legacy audit --strict` | `nixling host doctor --apply` |
| `nixling-legacy console <vm>` | `nixling console <vm>` |

`--apply` / `--dry-run` is now a mandatory flag pair on every
mutating verb (`--apply-or-dry-run-required`, exit 78). The bash
verbs accepted neither.

The daemon-native `.desktop` wrappers shipped in P4
(`ph4-p4-desktop-wrapper`) replace the per-VM
`nixling-launch-<vm>.desktop` files `cli.nix` used to generate.
Operators do not need to regenerate them by hand; the daemon-only
wrappers ship from `nixos-modules/components/desktop-wrapper.nix`
and are installed automatically on the next `nixos-rebuild switch`.

### Validation

```bash
# Bash entrypoints must be absent on v1.0:
test ! -e /run/current-system/sw/bin/nixling-legacy
test ! -e /run/current-system/sw/share/nixling/cli.sh

# The desktop wrapper contract must be present for every graphics VM:
nixling vm list --json | jq -r '.vms[].name' | while read vm; do
  test -e "/run/current-system/sw/share/applications/nixling-launch-${vm}.desktop"
done

# Layer-2 wave validators:
sudo nixling host validate --apply --wave p4
```

Cross-reference: `tests/cli-vm-verbs-eval.sh`,
`tests/desktop-wrapper-contract-eval.sh`,
`tests/legacy-unit-denylist-eval.sh` (asserts no example's
`nixos-rebuild dry-build` output emits a retired unit name).

### Rollback

```bash
sudo nixos-rebuild switch --rollback
```

If your consumer scripts call `nixling-legacy` you cannot run them
on a v1.0 system; pin your flake input to v0.4.x until the script
audit is complete.

## 3. W14c bash fallback removed + `NIXLING_LEGACY_BASH_OPT_IN` no-op

### Before

The Rust `nixling` binary's `dispatch_mutating_verb` first tried
the daemon, then on `not-yet-implemented` or `daemon-down` shelled
out to `/run/current-system/sw/bin/nixling-legacy`. The
`NIXLING_LEGACY_BASH_OPT_IN=1` env var bypassed the daemon entirely
and went straight to bash. `NIXLING_LEGACY_CLI` /
`NIXLING_LEGACY_CLI_PATH` overrode the legacy binary path.

### After

`dispatch_mutating_verb` is daemon-only. `not-yet-implemented`
surfaces as the typed envelope (exit 78);
`daemon-down`/`Unreachable` surfaces as exit 1. The bash branches
are gone; the function still accepts `legacy_args` /
`legacy_fallback_warning` parameters for binary compatibility with
its eight call sites, but they are unused. Setting
`NIXLING_LEGACY_BASH_OPT_IN=1` has **no effect** — the env var is
silently dropped. `NIXLING_NATIVE_ONLY=1` is preserved as a
documented no-op.

### Migration steps

Audit every environment, systemd unit, cron job, and operator
shell for the retired env knobs:

```bash
sudo grep -rIn \
  -e 'NIXLING_LEGACY_BASH_OPT_IN' \
  -e 'NIXLING_LEGACY_CLI' \
  -e 'NIXLING_LEGACY_CLI_PATH' \
  /etc /home /root /var/spool 2>/dev/null | grep -v Binary
```

Remove the env-var setting. If a verb was previously kept working
only by `NIXLING_LEGACY_BASH_OPT_IN=1`, that verb is now either
shipped daemon-native (the common case — every P3/P4 lifecycle
verb landed natively) or it is a legitimate gap to file as an
issue. `NIXLING_NATIVE_ONLY=1` can stay; it does nothing in v1.0
but it is not an error.

### Validation

```bash
# Confirm the fallback is gone end-to-end:
sudo systemctl stop nixlingd.service
NIXLING_LEGACY_BASH_OPT_IN=1 NIXLING_LEGACY_CLI_PATH=/bin/false \
  nixling vm start work --apply --json
# Expected: typed daemon-down envelope, exit 1. NOT bash exec.
sudo systemctl start nixlingd.service
```

Layer-1 gate: `tests/cli-vm-verbs-eval.sh` (poison-pill case —
asserts no bash exec even with `NIXLING_LEGACY_CLI_PATH` and
`NIXLING_LEGACY_BASH_OPT_IN=1` set).

### Rollback

`NIXLING_LEGACY_BASH_OPT_IN` only works on v0.4.x or earlier.
There is no per-host knob to re-enable the fallback in v1.0; the
only rollback path is `nixos-rebuild switch --rollback` to a
generation built against v0.4.x.

## 4. Per-VM systemd templates retired

### Before

Every VM declared a constellation of root-owned systemd units:

- `nixling@<vm>.service` — lifecycle wrapper.
- `microvm@<vm>.service` — upstream microvm.nix template.
- `microvm-virtiofsd@<vm>.service`,
  `microvm-tap-interfaces@<vm>.service`,
  `microvm-pci-devices@<vm>.service`,
  `microvm-set-booted@<vm>.service` — upstream sidecars.
- `nixling-<vm>-gpu.service`, `nixling-<vm>-snd.service`,
  `nixling-<vm>-video.service`, `nixling-<vm>-swtpm.service`,
  `nixling-<vm>-store-sync.service` — per-VM nixling sidecars.
- `nixling-known-hosts-refresh@<vm>.service`,
  `nixling-vfsd-watchdog@<vm>.{service,timer}`,
  `nixling-otel-relay@<vm>.service` — auxiliary loops.

The per-VM `nixling.vms.<vm>.supervisor` option chose between
`"systemd"` (the legacy template path) and `"nixlingd"` (daemon
ownership). The `nixling-launcher` polkit allowlist permitted
operator `systemctl start/stop/restart` on every per-VM unit.

### After

There are no framework-declared per-VM systemd units. `nixlingd`
supervises every per-VM DAG in-process; runners
(cloud-hypervisor, virtiofsd, swtpm, vhost-user-sound, USBIP
attach, GPU sidecar, video sidecar, otel relay) are spawned by
the broker via `SpawnRunner` and handed back over `SCM_RIGHTS` as
pidfds. The per-VM lifecycle state lives in
`/var/lib/nixling/supervisor/state.json`.

The `nixling.vms.<vm>.supervisor` option is **retained in v1.0
source** (default `"systemd"`) for backward-compat with consumer
flakes pinning pre-v1.0 manifests; the v1.0-intended hard
removal + eval-time rejection assertion was deferred to v1.1
backlog (see [ADR 0015](../adr/0015-daemon-only-clean-break.md)
§ Decision). v1.0 consumers should declare every workload VM as
`supervisor = "nixlingd"` and enable `nixling.daemonExperimental.enable = true`.
The polkit per-VM allowlist is retired (see §6 below);
`nixling-launchers` group membership + `SO_PEERCRED` on
`public.sock` is the only lifecycle authorisation surface.

### Migration steps

In your consumer `configuration.nix` (or whichever module declares
your VMs), set every workload VM to `supervisor = "nixlingd"`
(the v1.0 daemon-only convention per ADR 0015; the option's
hard removal was deferred to v1.1 backlog so the v1.0 default
remains `"systemd"` for backward-compat):

```diff
 nixling.vms.work = {
   enable = true;
-  supervisor = "systemd";    # pre-v1.0 default
+  supervisor = "nixlingd";   # v1.0 convention
   …
 };
```

You also need `nixling.daemonExperimental.enable = true` on the host
for `supervisor = "nixlingd"` to evaluate (the assertion at
`nixos-modules/assertions.nix:252` enforces this).

If you previously kept some VMs on `supervisor = "systemd"`,
migrate them to daemon ownership **on v0.4.x first** using
[`migrate-nixos-to-daemon.md`](./migrate-nixos-to-daemon.md). That
guide moves VMs one at a time.

Audit operator scripts and runbooks for `systemctl
{start,stop,restart} nixling@<vm>.service` (and the per-VM sidecar
unit names) and replace them with the Rust verbs from §2.

```bash
sudo grep -rIn \
  -e 'nixling@' \
  -e 'nixling-[a-z0-9-]\+-\(gpu\|snd\|video\|swtpm\|store-sync\)\.service' \
  -e 'microvm@' \
  /etc /home /root /var/spool 2>/dev/null | grep -v Binary
```

After the rebuild, stop any leftover per-VM instances that the
v0.4.x activation pass may have left running:

```bash
sudo systemctl list-units --no-pager --all \
  | grep -E '^(nixling@|microvm@|nixling-.+-(gpu|snd|video|swtpm|store-sync)\.service)' \
  | awk '{print $1}' \
  | xargs -r sudo systemctl stop
```

Then start each VM through the daemon:

```bash
nixling vm list --json
sudo nixling vm start work --apply
```

### Validation

```bash
# Exactly three nixling-shaped units on the host:
systemctl list-units --no-pager --all \
  | grep -E '^(nixling|microvm)' | wc -l
# Expected: 3
#   nixlingd.service
#   nixling-priv-broker.service
#   nixling-priv-broker.socket

# Per-wave Layer-2 validator:
sudo nixling host validate --apply --wave p2
```

Layer-1 gates: `tests/legacy-unit-denylist-eval.sh` (fail-closed
on any retired unit name reappearing in a `dry-build` output),
`tests/daemon-autostart-eval.sh`, `tests/restart-policy-eval.sh`.

### Rollback

The `supervisor` option remains live in v1.0 (its hard removal
was deferred to v1.1 backlog per ADR 0015 § Decision), so an
operator can revert individual VMs from `supervisor = "nixlingd"`
back to `supervisor = "systemd"` without a flake rev rollback.
For a full rollback to v0.4.x, pin the nixling flake input back
to v0.4.x and `nixos-rebuild switch --rollback`.

## 5. Host singletons retired

### Before

Four host-singleton framework services + one aggregator target
were declared by `nixos-modules/host-*.nix`:

- `nixling-net-route-preflight.service` — kernel route table
  preflight before `nixlingd` could start.
- `nixling-audit-check.service` + `nixling-audit-check.timer` —
  periodic audit-log rotation + integrity check.
- `nixling-ch-exporter.service` — cloud-hypervisor Prometheus
  exporter on `127.0.0.1:9101`.
- `nixling-otel-host-bridge.service` — OTLP host-relay bridge for
  the observability stack.
- `microvms.target` — upstream aggregator for `microvm@<vm>`.

### After

All five surfaces are gone. Their work moved as follows:

| Retired unit | Replacement |
| --- | --- |
| `nixling-net-route-preflight.service` | `nixlingd` startup self-check + `nixling host reconcile --network --apply` (P3 `ph3-p3-net-route-degraded-mode`); typed envelope `net-route-preflight-degraded` (exit 66). |
| `nixling-audit-check.{service,timer}` | broker `ExportBrokerAudit` op + `nixling host doctor` (P3 `ph3-p3-audit-check-retire`). Doctor's `checks[]` array reports the audit-rotation health. |
| `nixling-ch-exporter.service` | `nixlingd`'s own Prometheus exposition at `127.0.0.1:9101/metrics` (P3 `ph3-p3-ch-exporter-retire`). |
| `nixling-otel-host-bridge.service` | broker `SpawnRunner{role: OtelHostBridge}` (P3 `ph3-p3-otel-host-bridge-retire`) — runs as a daemon-supervised runner, not a persistent root service. |
| `microvms.target` | retired with `microvm@<vm>`; the upstream `microvm.autostart` / `systemd.targets.microvms.wants` cascade is suppressed in `host.nix`. |

### Migration steps

These are framework-internal surfaces; consumers do not declare
them. You only need to audit external scrapers / dashboards /
alerting rules that referenced them by name:

```bash
# Prometheus scrape config — replace the ch-exporter target:
sudo grep -rIn \
  -e 'nixling-ch-exporter' \
  -e 'nixling-net-route-preflight' \
  -e 'nixling-audit-check' \
  -e 'nixling-otel-host-bridge' \
  -e 'microvms.target' \
  /etc/prometheus /etc/grafana /etc/alertmanager 2>/dev/null \
  | grep -v Binary
```

The Prometheus endpoint at `127.0.0.1:9101/metrics` is unchanged;
only the unit owning the listening socket moved. If you scraped
the metrics by host:port you do not need to change anything. If
you scraped by `systemd_unit="nixling-ch-exporter.service"` label
(via node_exporter / alloy), retarget to `nixlingd.service`.

If you ran ad-hoc `systemctl start nixling-audit-check.service`
to force an audit-log rotation, replace it with:

```bash
sudo nixling host doctor --apply --json | jq '.checks[] | select(.name=="audit-rotation")'
```

### Validation

```bash
# Singletons must be absent:
for u in nixling-net-route-preflight nixling-audit-check.service \
         nixling-audit-check.timer nixling-ch-exporter \
         nixling-otel-host-bridge; do
  test -z "$(systemctl list-unit-files --no-pager "$u" 2>/dev/null \
              | awk '$1 == "'$u'" {print}')"
done

# nixlingd's own metrics endpoint is up:
curl -fsS http://127.0.0.1:9101/metrics | head

# Per-wave Layer-2 validator covers daemon-side preflight + metrics:
sudo nixling host validate --apply --wave p3
```

Layer-1 gates: `tests/observability-eval.sh`,
`tests/daemon-metrics-eval.sh`,
`tests/legacy-unit-denylist-eval.sh`.

### Rollback

There is no in-place rollback for the host singletons — the units
no longer exist in v1.0. Pin the flake input to v0.4.x and
`nixos-rebuild switch --rollback` if you need the singleton-based
posture back.

## 6. Polkit per-VM allowlists removed

### Before

`nixos-modules/host-polkit.nix` generated polkit allowlist entries
for every per-VM systemd unit (`nixling@<vm>`,
`nixling-<vm>-{gpu,snd,swtpm,store-sync}`) and every per-env
usbipd triplet (`nixling-sys-<env>-usbipd-{proxy,backend}.{service,socket}`).
The `nixling-launcher` group plus `org.freedesktop.systemd1.manage-units`
let operators run `systemctl start/stop/restart` on those units
without a password.

A companion JS rule scoped to the per-VM
`nixling-<vm>-gpu` system user granted it start/stop/restart of
its paired `nixling-<vm>-snd.service`.

### After

`host-polkit.nix` allowlists exactly three units:
`nixlingd.service`, `nixling-priv-broker.service`,
`nixling-priv-broker.socket`. The per-VM / per-env entries and
the JS rule are gone. Per-VM lifecycle flows through
`public.sock` (`SO_PEERCRED` group check, no polkit in the path).

The `nixling-launcher` group is preserved as the privilege
boundary for daemon-singleton restarts; the **`nixling-launchers`
group** (note the plural — declared in `nixos-modules/host-users.nix`)
is the authorisation surface for the daemon socket.

### Migration steps

Audit operator-facing tooling for `systemctl` invocations against
the retired per-VM / per-env unit names (covered in §4). Make
sure every operator who currently uses
`systemctl start nixling@<vm>` is a member of `nixling-launchers`
(daemon socket access), not just `nixling-launcher` (polkit
singleton restarts):

```bash
# Audit launcher-group membership for every operator:
getent group nixling-launchers
```

Add operators to the group from your consumer config:

```nix
{ nixling.site.launcherUsers = [ "alice" "bob" ]; }
```

This is the same option you set in v0.4.x for the daemon path; no
new option is required.

### Validation

```bash
# Polkit rule file enumerates exactly the three daemon singletons:
sudo grep -E 'nixling[^"]*\.service|nixling[^"]*\.socket' \
  /etc/polkit-1/rules.d/*nixling* \
  | sort -u
# Expected lines (paths may differ slightly):
#   nixlingd.service
#   nixling-priv-broker.service
#   nixling-priv-broker.socket

# No per-VM or per-env allowlist entry remains:
sudo grep -E 'nixling@|nixling-[^"]+-(gpu|snd|swtpm|store-sync)|nixling-sys-[^"]+-usbipd' \
  /etc/polkit-1/rules.d/*nixling* \
  || echo "no per-VM polkit entries (expected on v1.0)"

# Daemon socket accept-time authz works for an operator:
sudo -u alice nixling vm list --json | jq '.vms | length'
```

Layer-1 gate: `tests/polkit-allowlist-eval.sh` (asserts the
allowlist names exactly the three daemon-only singletons).

### Rollback

Same as §4 — the polkit retirement ships in the same release as
the per-VM template deletion. There is no in-place rollback.

## 7. Final preflight + W18 default-flip

This section follows the canonical *Before / After / Migration steps /
Validation / Rollback* layout for the W18 flip itself.

### Before

- `nixling.daemonExperimental.enable` defaults to `false` even though
  every individual breaking change in §§1–6 has landed in the running
  config.
- The W18 readiness option set
  (`nixling.defaultSwitchReadiness.<wave>.{implemented,validated}`)
  carries `implemented = true` for every wave that shipped its
  Rust/daemon path, but `validated = false` until an evidence file
  exists at `<defaultFlipEvidenceDir>/<wave>.json`.

### After

- `nixling.daemonExperimental.enable` flips to `true` automatically
  on the next `nixos-rebuild switch` because every wave in the
  flip-gate subset (w4Fu, w5Fu, w6Fu, w7Fu, w8Fu, w9Fu, p0, p0Fu,
  p1, p2, p3, p4) carries `implemented + validated + evidence`.
- Operator overrides (`mkDefault false`, `mkForce false`, `mkForce
  true`) continue to win in both directions, exactly as documented
  in [`docs/reference/default-switch-and-deprecation.md`](../reference/default-switch-and-deprecation.md#auto-flip-gate).
- The framework emits exactly three persistent root-visible nixling
  systemd units: `nixlingd.service`, `nixling-priv-broker.service`,
  `nixling-priv-broker.socket`.

### Migration steps

```bash
# 1. Inventory every wave's readiness.
nixling host validate --dry-run --json \
  | jq '.waves[] | {wave, status}'

# 2. Run any per-wave Layer-2 validator that isn't `ready` yet.
#    The per-wave map lives in:
#    docs/reference/host-validate.md#per-wave-validator-map
sudo NL_LIVE=1 bash tests/minijail-validator-swtpm.sh
# … repeat for every wave you want to attest …

# 3. Write umbrella evidence records for every `ready` wave.
sudo nixling host validate --apply

# 4. nixling.daemonExperimental.enable now defaults to `true`
#    because every <wave>.json record exists with the canonical
#    schema. A second nixos-rebuild switch picks up the default
#    flip (operator overrides — explicit `= true` / `= false` —
#    still win).
sudo nixos-rebuild switch --flake .#myhost
```

### Validation

```bash
# Hard exit criterion (ADR 0015 § Verification): exactly three
# persistent nixling/microvm root-visible units.
systemctl list-units --no-pager --all \
  | grep -E '^(nixling|microvm)' | wc -l
# Expected: 3   (nixlingd.service + nixling-priv-broker.{service,socket})

# Default-flip eval gate (tests/w18-default-flip-eval.sh) asserts
# the gate honors readiness + evidence + operator override
# semantics. It is wired into tests/static.sh.
bash tests/w18-default-flip-eval.sh

# Confirm the wave-evidence schema is consistent across the live
# config + the canonical schema.
bash tests/wave-evidence-schema-eval.sh
```

If `host validate --apply` reports any `missing` wave, the verb
exits 78 and refuses to write evidence. Address the surfaced
reason and re-run; do not edit `/var/lib/nixling/validated/<wave>.json`
by hand.

### Rollback

The W18 flip is a default change driven by an evaluator predicate,
not a destructive op. Rollback is therefore three orthogonal levers:

```bash
# Option A — keep the daemon-only end-state but pin the flag
#            explicitly to false in your consumer config:
#   nixling.daemonExperimental.enable = lib.mkForce false;
# (Then `nixos-rebuild switch` to apply.)
sudo nixos-rebuild switch --flake .#myhost

# Option B — remove the evidence files; the default predicate sees
#            them missing and flips back to false on the next eval.
sudo rm -rf /var/lib/nixling/validated
sudo nixos-rebuild switch --flake .#myhost

# Option C — full v0.4.x rollback (see §8 below). Recommended only
#            if §§1–6 also need to be undone.
```

## 8. Whole-migration rollback

Every per-section rollback above is the same `nixos-rebuild switch
--rollback` recipe; the v1.0 release is a single coherent cut and
cannot be partially undone. The whole-migration rollback is:

```bash
# 1. Roll back the running NixOS generation.
sudo nixos-rebuild switch --rollback

# 2. Pin the consumer flake back to the pre-migration revision.
git -C /etc/nixos checkout pre-nixling-v1 -- flake.nix flake.lock
sudo nixos-rebuild switch --flake .#myhost

# 3. Confirm the v0.4.x supervisor surface is back.
systemctl list-units --no-pager --all \
  | grep -E '^nixling@' | head
```

If the rollback is post-incident, file an issue with the
`host validate --json` dump, the relevant `journalctl -u
nixlingd.service -u nixling-priv-broker.service` window, and the
last broker audit log under `/var/lib/nixling/audit/broker-<utc-date>.jsonl`.

## See also

- [ADR 0015 — Daemon-only clean break](../adr/0015-daemon-only-clean-break.md)
- [`docs/how-to/migrate-nixos-to-daemon.md`](./migrate-nixos-to-daemon.md)
  — per-VM `supervisor = "systemd" → "nixlingd"` move (v0.4.x only).
- [`docs/how-to/migrating-from-microvm.md`](./migrating-from-microvm.md)
  — raw microvm.nix → nixling (new installs).
- [`docs/how-to/uninstall-nixling.md`](./uninstall-nixling.md)
- [`docs/reference/host-validate.md`](../reference/host-validate.md)
- [`docs/reference/cli-contract.md`](../reference/cli-contract.md)
- [`docs/reference/default-switch-and-deprecation.md`](../reference/default-switch-and-deprecation.md)
- [`docs/reference/privileges.md`](../reference/privileges.md)
- [`docs/reference/manifest-schema.md`](../reference/manifest-schema.md)
- [`docs/explanation/default-switch-and-deprecation.md`](../explanation/default-switch-and-deprecation.md)
- [`docs/explanation/daemon-lifecycle.md`](../explanation/daemon-lifecycle.md)
