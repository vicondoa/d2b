# nixling tests

Three layers of validation, ordered cheapest-to-most-expensive.

## W2 naming expectations

Tests reference systemd unit names directly. After the W2 rename pass,
the units you should see on a host with nixling installed are:

| Resource                       | Unit name                                |
| ------------------------------ | ---------------------------------------- |
| User-facing per-VM             | `nixling@<vm>.service`                   |
| Backend (microvm.nix template) | `microvm@<vm>.service`                   |
| Per-VM virtiofsd               | `microvm-virtiofsd@<vm>.service` *(W2 rename to `nixling-<vm>-virtiofsd` deferred)* |
| Per-VM GPU sidecar             | `nixling-<vm>-gpu.service`               |
| Per-VM video sidecar           | `nixling-<vm>-video.service`             |
| Per-VM audio sidecar           | `nixling-<vm>-snd.service`               |
| Per-VM TPM emulator            | `nixling-<vm>-swtpm.service`             |
| Per-VM store sync              | `nixling-<vm>-store-sync.service`        |
| Per-env USBIP proxy            | `nixling-sys-<env>-usbipd-proxy.{service,socket}` |
| Per-env USBIP backend          | `nixling-sys-<env>-usbipd-backend.service` |
| Host observability bridge      | `nixling-otel-host-bridge.service`       |
| Host CH exporter               | `nixling-ch-exporter.service`            |
| Auto-declared per-env net VM   | `nixling@sys-<env>-net.service` (workload-side `microvm@sys-<env>-net.service` backend) |
| Polkit launcher group          | `nixling` (singleton)           |

State directories follow the matching pattern:

```
/var/lib/nixling/
  vms/<vm>/                            workload + sys VMs
    state/audio-state.json
    swtpm/
    store/, store-meta/
    host-keys/host.pub                 (Phase 2b reserved)
  keys/                                (Phase 2b reserved)
```

The per-VM manifest baked into the CLI generation lives at
`/run/current-system/sw/share/nixling/vms.json`. The reserved
`_manifest` top-level key carries `manifestVersion = 2`, and the
reserved `_observability` key carries host-wide observability metadata.
User-facing VM names cannot start with `_` (eval-time assertion in
`nixos-modules/assertions.nix`). The schema is documented in
`docs/reference/manifest-schema.{md,json}`.

## Layer 1 — `static.sh`

Pure eval / parse / dry-build. Runs in seconds. No host activation.
Catches syntax errors, missing imports, broken option types, and
per-VM closure evaluation regressions.

```bash
tests/static.sh
```

Layer-1 gates exercised (W4):

- `nix-instantiate --parse` for every framework `.nix` file.
- `shellcheck --severity=warning` for every shell script under
  `tests/` and `scripts/`.
- Heuristic `readOnly + default + config` trio lint across
  `nixos-modules/` — catches the read-only mkOption pattern from issue
  #6 before it escapes review.
- `nix flake check --no-build --all-systems` — both x86_64-linux and
  aarch64-linux flake outputs eval clean.
- Per-example flake checks pass `--no-write-lock-file`, so
  `tests/static.sh` never rewrites an example's committed `flake.lock`
  while eval-checking it.
- `tests/smoke-eval.nix` — minimal consumer-style nixosSystem on the
  builder's system (typically x86_64-linux).
- `tests/smoke-eval-aarch64.nix` — same shape, cross-evaluated on
  aarch64-linux to verify the headless workload eval graph stays
  multi-arch clean (Phase 4 W4 gate).
- `tests/smoke-eval-tpm.nix` — TPM host-surface regression gate:
  swtpm parent-dir ACL, swtpm ExecStartPre stale-session flush,
  and `nixlingMigrateOwnership` invariants.
- `tests/assertions-eval.sh` — eval-time assertion regression tests
  (CIDR shape, CIDR overlap, key validation, `waylandUser`
  presence, graphics/audio platform gating, and observability
  collision/prefix cases).
- `tests/observability-eval.sh` — 23/23 eval-time observability
  cases: defaults, auto-declaration, manifest fields, CLI-traces
  gating, and the current negative/auto-SKIP coverage.
- `tests/usbip-gating-eval.sh` — host-side USBIP gating: per-env
  usbipd services/sockets/firewall rules stay absent until both
  `site.yubikey.enable` and an enabled VM `usbip.yubikey` opt-in are
  set, and stay scoped to envs that actually have an opted-in VM.
- **Manifest contract gate** (Phase 5 W4): renders the smoke
  manifest, then runs a 5-check sequence:
  1. Manifest renders without errors.
  2. `docs/reference/manifest-schema.json` is syntactically valid JSON.
  3. Smoke manifest validates against the JSON Schema (Draft 2020-12).
  4. Every per-VM field in the manifest is documented in the schema's
     `$defs.vmEntry.required` list (catches manifest gaining a field
     without a schema update).
  5. **md ↔ json drift detection** (W4 followup): the prose Per-VM-
     entry table in `docs/reference/manifest-schema.md` and the
     schema's `properties` keys must list the same field set.
  6. `_manifest.manifestVersion` is present and `>= 1`.

Layer-1 script inventory:

| Script | Purpose |
| --- | --- |
| `tests/assertions-eval.sh` | Eval-time assertion regressions, including the observability collision/prefix cases. |
| `tests/observability-eval.sh` | Eval-time observability surface checks: defaults, auto-declared env/VM, manifest fields, CLI-traces gate, and assertion auto-SKIPs. |
| `tests/restart-policy-eval.sh` | `restartIfChanged = false` regression coverage for lifecycle services and observability host units. |
| `tests/usbip-gating-eval.sh` | Host-side USBIP gating: absent with no host+enabled-VM opt-in, present once both knobs are enabled, and scoped to the opted-in env only. |
| `tests/restart-policy-eval.sh` | `restartIfChanged = false` regression coverage for lifecycle services and observability host units. |
| `tests/restart-policy-eval.sh` | `restartIfChanged = false` regression coverage for lifecycle services plus the host, workload-guest, and obs-guest observability relay units. |
| `tests/video-sidecar-hardening-eval.sh` | Eval-time hardening gate for `nixling-<vm>-video.service` (`AF_UNIX` only, syscall filter, empty capability sets). |
| `tests/bridge-isolation-runtime.sh` | Hermetic runtime check that Linux bridge port isolation still blocks workload↔workload traffic while preserving workload↔net-VM reachability. |
| `tests/network-isolation.sh` | Optional live-host datapath checks for same-env east-west and cross-env isolation. |
| `tests/audit-forwarding.sh` | Optional live-host end-to-end check for auditd -> journald -> Alloy -> Loki delivery. |

## Layer 2 — `nixling-store.sh`

Integration tests that exercise the per-VM nix store and the
`nixling build/switch/boot/test/rollback/generations/gc` lifecycle
against the live host. Idempotent — re-run anytime. Leaves every VM
in the state it found.

Tests that need a running VM with SSH credentials (in-VM activation
checks) auto-skip when no such VM is up, printing a `SKIP` line.

```bash
# Full run (~15-60s depending on VM state)
tests/nixling-store.sh

# Smoke subset (~5-10s)
tests/nixling-store.sh --quick

# Single test
tests/nixling-store.sh --only test_closure_isolation

# List all available tests
tests/nixling-store.sh --list
```

| Test                            | What it proves                                                                 |
| ------------------------------- | ------------------------------------------------------------------------------ |
| `test_closure_isolation`        | per-VM store ≪ host /nix/store (< 30% of paths)                                |
| `test_no_host_paths_in_vm`      | host-only system closure absent from any VM's store                            |
| `test_hardlink_sharing`         | sampled regular files share inodes with /nix/store (data is shared, not copied)|
| `test_zero_data_duplication`    | every sampled regular file under store/ has nlink ≥ 2                          |
| `test_build_idempotent`         | `nixling build` twice yields identical output paths                            |
| `test_build_gc_root`            | result symlink present + recognised by nix-store --gc                          |
| `test_generations_list`         | `nixling generations` shows host-side header and (current) marker              |
| `test_host_rebuild_rehydrates`  | activation script wires the sync hook + service template installed             |
| `test_db_load_on_boot`          | (running VM) `nix-store --query --requisites` works inside the guest           |
| `test_legacy_status`            | `nixling status` baseline regression                                           |
| `test_error_missing_ssh_creds`  | helpful error when a VM lacks ssh.{user,keyPath}                               |
| `test_retention_keeps_current`  | every path in the current generation is present under store/                   |

## Layer 2 (network isolation) — `network-isolation.sh`

Optional live-host datapath checks for the Wave 3 network isolation
claims. The script looks for running workload VMs with SSH access and:

- verifies same-env east-west is blocked when bridge taps are isolated,
  including an attacker-style route-via-gateway attempt when sudo is
  available in the source VM,
- only allows a same-env reachability check for an explicitly named
  `NL_ALLOW_EASTWEST_PAIR`, using a temporary listener inside the peer VM,
- verifies cross-env workload and net-VM uplink reachability stay
  blocked against a controlled temporary listener, and
- verifies host-LAN reachability stays blocked against a temporary host
  listener plus the live LAN gateway.

Runs as a skip-if-prereqs-missing test, just like the other Layer-2
scripts.

```bash
tests/network-isolation.sh
```

## Layer 2 (audit forwarding) — `audit-forwarding.sh`

Optional live-host audit pipeline check for Wave 3. The script looks for
an audit-enabled, observability-enabled running VM plus a reachable obs
VM, verifies the default `/etc/passwd` watch is loaded, adds a
per-run nonce watch under `/run/`, triggers it, and polls Loki on the
obs VM for the resulting
`source="audit" unit="audisp-syslog" vm=<vm> env=<env>` stream.

```bash
tests/audit-forwarding.sh
```

## Layer 2 (audio) — `audio.sh`

> `tests/audio.sh` resolves the Wayland session user via the
> `NL_WAYLAND_USER` chain: explicit env var → `nix eval` against
> `nixling.site.waylandUser` on the live host config → `$SUDO_USER`
> → the invoking non-root user. UID is detected at runtime from
> `getent passwd`. Override with `NL_WAYLAND_USER=alice` if the
> auto-detection picks the wrong account.

Integration tests for the audio component
(`nixos-modules/components/audio/guest.nix` +
`nixos-modules/components/audio/host.nix`) and the host PipeWire
surface it depends on. Run from a Plasma terminal (NOT root, NOT bare
SSH — the user-systemd manager has to belong to the configured
`waylandUser`).

```bash
# Full run
tests/audio.sh

# Smoke subset (no CLI grant/revoke, no in-guest probe)
tests/audio.sh --quick

# Single test
tests/audio.sh --only test_host_has_audio_devices

# List
tests/audio.sh --list
```

| Test                                  | What it proves                                                                            |
| ------------------------------------- | ------------------------------------------------------------------------------------------ |
| `test_host_pipewire_alive`            | `pipewire.service` + `wireplumber.service` are active and `wpctl status` is reachable.    |
| `test_host_has_audio_devices`         | At least one real ALSA/v4l2/bluez5 device is visible to WirePlumber.                       |
| `test_host_has_audio_sinks_and_sources` | At least one real Sink (not just "Dummy Output") and one Source — catches the failure mode where rebuild loses ALSA visibility. |
| `test_sidecar_unit_present`           | At least one `nixling-<vm>-snd.service` per-VM system unit is registered. |
| `test_sidecar_socket_lifecycle`       | Starting `nixling-<vm>-snd.service` creates the UDS with `group=kvm mode=0660`; stop removes it (RemoveOnStop). |
| `test_cli_status_smoke`               | `nixling audio status <vm>` reports the expected fields for an audio-enabled VM.          |
| `test_cli_grant_revoke`               | `nixling audio mic on / speaker on / mic off / off` round-trip: state file transitions correctly, sidecar lifecycle follows. |
| `test_cli_rejects_audio_disabled_vm`  | Trying `nixling audio mic on <non-audio-vm>` fails with a clear error.                    |
| `test_cloud_hypervisor_capabilities`  | The CH binary actually used by the runner is **v52 or newer** (CVE-2026-45782 fixed), has `--generic-vhost-user` (audio attach), AND has `--gpu` (spectrum graphics patches present). |
| `test_guest_sees_virtio_snd`          | (auto-SKIP if no audio-enabled VM is running): SSH into the running VM and verify `/proc/asound/cards` reports the virtio-snd device. |

> **TODO (Phase 8c)**: the "Why this exists" paragraph below is a
> personal-host war story rather than user-facing documentation;
> rewrite as a generic "audio surface is fragile across CH upgrades —
> run these after any rebuild touching audio packages" note.

**Why this exists**: during the CH v50 → v52 bump in May 2026 a
`nixos-rebuild switch` once dropped the host's ALSA card visibility
(Plasma's audio mixer showed "no devices"); a manual restart of
`pipewire.service` + `wireplumber.service` recovered. The host-surface
checks above (`test_host_has_audio_*`) are the regression coverage
for that ambient breakage — run them after any rebuild that touches
audio, PipeWire packages, or the audio-host.nix module.

> **Known Layer-2 gap.** Bridge-isolation enforcement (the DHCP
> anti-spoofing posture for workload taps on `br-<env>-lan`) still has
> no runtime Layer-2 test. Verifying that workload taps remain isolated
> from each other requires a live host plus packet-level assertions
> across the bridge, so the current Layer-2 suite documents the
> expectation but does not yet automate it. The first planned Layer-3
> `nixosTest` for this area is a MAC/IP spoof attempt where one
> workload tries to impersonate a peer's DHCP reservation and send
> east-west traffic across `br-<env>-lan`; the test should prove the
> isolated taps still block that path.

## Layer 3 — reproducible `nixosTest`

Still out of scope for now.

The current runtime bridge-isolation gate is
`tests/bridge-isolation-runtime.sh`, which runs hermetically inside a
user+network namespace and is wired into `tests/static.sh`. It proves
that the Linux bridge semantics nixling relies on match the documented
threat model: the net-VM port stays reachable while workload ports stay
isolated even after a workload spoofs a peer-style MAC.

## Future tests (Phase 7a / v0.2.0)

- **USBIP live isolation `nixosTest`** (Phase 6 follow-up). The
  Layer-1 eval gate now proves host-side USBIP units, sockets, and
  firewall rules only materialize for envs with an enabled
  `usbip.yubikey` VM, but it still does not exercise live systemd
  socket materialization, iptables enforcement, or cleanup against a
  running guest. Lift that adversarial cross-env attach/isolation path
  into the eventual Phase 6 `nixosTest` suite.

- **Audit `--strict` graphics-VM running-check mock test** (Spec
  correction #38 / v0.1.6 follow-up Test-H8). The v0.1.6 fix in
  `nixos-modules/cli.nix` extended the
  `bridge_isolated_workload.<vm>` running-check from the previous
  `microvm@<vm>` probe to also accept `nixling@<vm>` or
  `nixling-<vm>-gpu` as evidence the VM is up — without this,
  graphics VMs were blanket-skipped by `nixling audit --strict`
  even when actively running. **Known gap:** this still needs a live
  host / higher-fidelity harness because the shell-application wrapper
  bakes `systemctl` in via `runtimeInputs`; a plain `PATH` stub is not
  enough to exercise the strict-audit path faithfully. Deferred to
  v0.2.0 alongside the other host-backed testing items.

> Per-example iteration is now part of the static gate:
> `tests/static.sh` iterates every `examples/*/flake.nix` and runs
> `nix flake check --no-build --all-systems` against each. The
> `with-entra-id` example reaches a sibling flake
> (`vicondoa/entrablau.nix`), so its iteration step is not fully
> hermetic — it eval-tests through the example's pinned
> `flake.lock`. That hermeticity gap is the only remaining
> example-iteration follow-up; see CHANGELOG "Known gaps".

## Daily smoke

`$FLAKE` below is the consumer's flake root (e.g. `/etc/nixos` for
the maintainer, `$HOME/projects/myhost` for someone else); `$HOST` is
the `nixosConfigurations.<name>` attribute to rebuild. Both default
to the values the framework's tooling auto-detects, so most operators
can run the snippet verbatim after pointing the env vars at their own
tree.

```bash
FLAKE=${FLAKE:-/etc/nixos}
HOST=${NL_HOST_CONFIG:-desktop}
sudo -A nixos-rebuild switch --flake "$FLAKE#$HOST" && \
  "$FLAKE"/tests/nixling-store.sh --quick && \
  "$FLAKE"/tests/audio.sh --quick
```

If both `--quick` runs are green and any failed VM was running before,
the change is safe to commit.

## Layer-2 with-sudo CI hook (v1.2fu58)

`tests/state-dir-acl-runtime.sh` is a Layer-2, root-only adversarial
test that exercises the `/var/lib/nixling` traversal ACL added in
v1.2fu58. It creates synthetic users + a throwaway state-dir, applies
the activation script's setfacl pattern, and asserts:

- A launcher-group member CAN traverse + read the per-VM SSH key.
- A non-launcher user CANNOT stat the key (no traversal).
- The traversal grant is `--x` only (launcher member cannot list the
  state-dir contents).

### Local run

Skipped silently if not root. Opt in via:

```bash
NL_RUN_LAYER2_WITH_SUDO=1 sudo -n bash tests/state-dir-acl-runtime.sh
```

### CI run

`.github/workflows/layer2-runtime-with-sudo.yml` is **manual
dispatch only** (`workflow_dispatch`) — it is NOT triggered on
`pull_request` because it uses passwordless `sudo -n` on a
self-hosted runner and running PR-controlled code under root would
be a privilege-escalation footgun (panel R9 security-1 +
networking-1). Maintainers manually dispatch the workflow against a
reviewed ref:

```bash
gh workflow run layer2-runtime-with-sudo.yml --ref <ref>
```

The `paths` list inside the workflow YAML doubles as a reviewer
checklist: PRs that touch `nixos-modules/host-activation.nix`,
`nixos-modules/host-activation.d/state-dir-acl.sh`,
`nixos-modules/host-keys.nix`, or `tests/state-dir-acl-*.sh`
should be Layer-2-dispatched after review. See
[`CONTRIBUTING.md` § "Provisioning the `nixling-sudo` self-hosted
runner"](../CONTRIBUTING.md) for runner setup.

## Phase-6 nixosTest follow-ups

Layer-3 nixosTest coverage should add these invariants:

- Cross-uid runner stop via broker fallback.
- Launcher key-path traversal without read escalation.
- Post-rename lifecycle authorization through only the `nixling` group.
- Broker `DeregisterRunnerPidfd` lifecycle with no leak after normal
  stop and after daemon crash/restart.
- Broker pidfd registry size gauge (Prometheus gauge, unlabeled,
  exported by the broker) so operators can spot registry leaks.
- Registry-size gauge converges to zero after a normal `vm stop` and
  after a daemon-crash-then-restart cycle.
- ECHILD recovery instrumentation: a distinct metric or structured-log
  dimension distinguishing `wait_terminated_with_broker_poll` ECHILD
  recovery from normal stops. The minimal structured log already ships
  as `outcome="echild-broker-recovered"`; Phase 6 adds the metric arm.
