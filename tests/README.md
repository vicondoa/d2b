# nixling tests

Three layers of validation, ordered cheapest-to-most-expensive.

## Naming expectations

Tests should treat `nixlingd` and the privileged broker as the only
host-visible systemd surface. Per-VM lifecycle work is represented in
the daemon process DAG and broker runner roles, not by per-VM host
units.

| Resource | Name |
| --- | --- |
| Public daemon | `nixlingd.service` |
| Privileged broker socket | `nixling-priv-broker.socket` |
| Privileged broker service | `nixling-priv-broker.service` |
| Lifecycle permission group | `nixling` |
| Per-VM runner roles | `cloud-hypervisor`, `virtiofsd`, `swtpm`, `gpu`, `video`, `audio`, `vsock-relay`, `usbip` in `processes.json` |

State directories follow the matching pattern:

```
/var/lib/nixling/
  vms/<vm>/                            workload + sys VMs
    state/audio-state.json
    swtpm/
    store/, store-meta/
    host-keys/host.pub
  keys/
```

The per-VM manifest baked into the CLI generation lives at
`/run/current-system/sw/share/nixling/vms.json`. The reserved
`_manifest` top-level key carries the current `manifestVersion`, and the
reserved `_observability` key carries host-wide observability metadata.
User-facing VM names cannot start with `_` (eval-time assertion in
`nixos-modules/assertions.nix`). The schema is documented in
`docs/reference/manifest-schema.{md,json}`.

## Test architecture — where test logic lives

The suite was rearchitected to move per-gate logic out of ad-hoc bash
into typed, fast, fail-closed homes. **When adding or changing a test,
pick the home by what it asserts — do not add a new `tests/*.sh`
gate.**

| What you assert | Home | Mechanism |
| --- | --- | --- |
| Eval-time value / config shape / eval-failure | `tests/nix-unit/cases/<gate>.nix` | declarative `{ expr; expected; }` / `{ expr; expectedError; }`, auto-discovered by `tests/nix-unit/default.nix`, gated fail-closed by `nix flake check --no-build` (the `nix-unit` check throws at eval time on any failed case or missing pin). |
| Source / doc / schema lint | `packages/nixling-contract-tests/tests/policy_*.rs` | reads the real checkout via `read_repo_file`; runs from `tests/rust-workspace-checks.sh`. |
| Daemon / broker / CLI behaviour (KVM-free) | `packages/<crate>/tests/*.rs` | native integration test via `env!("CARGO_BIN_EXE_*")`; runs in the default `cargo test --workspace`. |
| A gate that only **runs** existing `#[test]`s | `tests/golden/pinned/<gate>.txt` | cargo-pin; fail-closed presence via `tests/tools/assert-pinned-tests.sh`. |
| Generated-artifact drift (`xtask gen → git diff`) | `tests/drift-check.sh` | one consolidated runner. |
| Live-host / VM runtime (microVM boot, netns, root ACLs, devices) | `runNixOSTest` (the `test-integration` / `test-hardware` tiers) | **W4** — see "Planned runtime tests" below. |

### `.sh` files that stay permanently (infrastructure, not test cases)

The migration removes per-gate bash, but these are the test *runner*,
not test *cases* — they never become Rust/nix-unit:

- **Runners / harness:** `static.sh`, `static-fast.sh`,
  `static-fast-tier0.sh`, `static-timing.sh`, `runner.sh`, `lib.sh`,
  `preflight-disk-space.sh`, `cli-rust-native-common.sh`,
  `rust-workspace-checks.sh`.
- **Tooling:** `tests/tools/{gen-migration-ledger,assert-pinned-tests,gen-nix-unit-pins,run-layer}.sh`.
- **Meta-gates** that validate the test inventory / CI itself:
  `adr-index-coverage`, `ci-coverage`, `ci-uses-make`,
  `cli-contract-coverage`, `deliverable-gate-inventory`,
  `l3-pin-consistency`, `layer1-self-inventory`, `no-new-deferral`,
  `pr-checklist-gate`.
- The consolidated `drift-check.sh`.

The only remaining migratable `.sh` are the live-host/VM **G-tier**
gates, which leave in W4 (each flips its ledger row to
`status = "ported"`, its `.sh` is retired, and a `runNixOSTest`
successor takes over).

### `.nix` files under `tests/` — the destination, not collateral

The `.nix` files are the clean structure the migration moves *toward*;
they stay and grow:

- `tests/nix-unit/cases/*.nix` + `tests/nix-unit/default.nix` — the
  migrated eval tests. A former bash eval gate (a `.sh` that shelled
  out to `nix eval`) becomes a declarative case here.
- `tests/eval-cases/*.nix` and `tests/smoke-eval*.nix` — shared eval
  **fixtures** (consumer / VM configs) imported by the nix-unit cases and
  by `flake.checks` (e.g. `nix-unit/cases/guest-control-auth.nix` imports
  `tests/eval-cases/guest-control-auth-eval.nix`). Fixtures shared by more
  than one consumer live in `tests/eval-cases/`, not under any single
  consumer's directory.

The trade is deliberate: sloppy per-test `.sh` → a `.nix` corpus +
Rust tests. Bash shrinks to the runner/harness; the test *logic* lives
in nix-unit cases, `policy_*.rs`, `packages/<crate>/tests/*.rs`, or
cargo-pins.

## Layer 1 — `static.sh`

Pure eval / parse / dry-build. Runs in seconds. No host activation.
Catches syntax errors, missing imports, broken option types, and
per-VM closure evaluation regressions.

```bash
tests/static.sh
```

Layer-1 gates exercised:

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
  multi-arch clean.
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
- **Manifest contract gate**: renders the smoke
  manifest, then runs a 5-check sequence:
  1. Manifest renders without errors.
  2. `docs/reference/manifest-schema.json` is syntactically valid JSON.
  3. Smoke manifest validates against the JSON Schema (Draft 2020-12).
  4. Every per-VM field in the manifest is documented in the schema's
     `$defs.vmEntry.required` list (catches manifest gaining a field
     without a schema update).
  5. **md ↔ json drift detection**: the prose Per-VM-
     entry table in `docs/reference/manifest-schema.md` and the
     schema's `properties` keys must list the same field set.
  6. `_manifest.manifestVersion` is present and `>= 1`.

Layer-1 script inventory:

| Script | Purpose |
| --- | --- |
| `tests/assertions-eval.sh` | Eval-time assertion regressions, including the observability collision/prefix cases. |
| `tests/observability-eval.sh` | Eval-time observability surface checks: defaults, auto-declared env/VM, manifest fields, CLI-traces gate, and assertion auto-SKIPs. |
| `tests/restart-policy-eval.sh` | `restartIfChanged = false` regression coverage for daemon-owned lifecycle and guest/stack observability units. |
| `tests/usbip-gating-eval.sh` | Host-side USBIP gating: absent with no host+enabled-VM opt-in, present once both knobs are enabled, and scoped to the opted-in env only. |
| `tests/niri-vm-borders-eval.sh` | Opt-in niri KDL border generation: disabled by default, correct window-rule per graphics VM when enabled, per-VM color override, default color stability, and custom outputPath. |
| `tests/video-sidecar-hardening-eval.sh` | Eval-time hardening gate for the broker `SpawnRunner{role=Video}` descriptor (`AF_UNIX` only, syscall filter, empty capability sets). |
| `tests/minijail-validator-wayland-proxy.sh` | Wayland filter proxy minijail profile gate: mandatory seccomp, empty capabilities, empty device binds, dedicated runtime dir (`/run/nixling-wlproxy/<vm>`), no PipeWire/Pulse socket access; compositor access is granted to the `wlproxy` role by ACL, not by a profile bind mount. |
| `tests/network-isolation.sh` | Optional live-host datapath checks for same-env east-west and cross-env isolation. |
| `tests/audit-forwarding.sh` | Optional live-host end-to-end check for auditd -> journald -> Alloy -> Loki delivery. |

### Layer-1 gates that assume a clean CI host

A handful of Layer-1 gates are written for the clean, hermetic CI
environment (no running `nixlingd`/broker, no real
`/var/lib/nixling/daemon-state`, no real host network/posture) and are
**expected to fail when `static.sh` is run on a developer machine that
has live nixling VMs and a running daemon**. They pass in CI; treat a
failure on a live host as environment-dependent, not a regression,
unless it also reproduces in CI.

| Gate | Why it is host-dependent |
| --- | --- |
| `tests/cli-rust-native-status.sh`, `tests/cli-rust-native-host-check.sh`, `tests/cli-json-drift.sh` | `systemctl_state` shells out to the real `systemctl is-active nixlingd.service` when the unit is absent from the test fixture, and the tests do not sandbox `NIXLING_DAEMON_STATE_DIR`, so VM status reflects the real host's running VMs / `pidfd-table.json` instead of the fixture. (`cli-rust-native-list` is **retired** — its successor `packages/nixling/tests/cli_contract.rs` fixes this by pinning `nixlingd.service` in the system-state fixture and sandboxing `NIXLING_DAEMON_STATE_DIR` to an empty dir, so status is hermetic.) |
| `tests/daemon-socket-acl.sh`, `tests/daemon-version-negotiation.sh`, `tests/daemon-state-persistence.sh` | Spawn a transient test `nixlingd`; flaky/failing alongside a real running daemon on a live host. |
| `tests/cli-contract-coverage.sh` | The `host check` flag-acceptance probe treats any `rc == 2` as "flag rejected", but `host check` returns a non-zero posture/`internal-io` exit when `nft` is absent or the real host posture is imperfect. (It also has a genuine, CI-visible dispatch-table doc-drift for the merge-added `usb`/`audio` verbs — see below.) |
| `tests/examples-with-observability-eval.sh` | The example's `flake.lock` carries a mutable `path:../..` lock that `nix eval` rejects in this checkout layout. |

### Layer-1 gate with pre-existing breakage inherited from `main`

This gate fails independently of host environment (i.e. on CI too)
because of drift that predates the guest-control work and lives in the
broker-infra domain. It is **identical to `main`** and requires the
original authors' intent to fix correctly; it is not a guest-control
regression:

| Gate | Pre-existing issue |
| --- | --- |
| `tests/broker-validate-bundle.sh` | Forbids **all** `serde_json::from_str`/`from_value` under the broker `src/` to prevent duplicate bundle parsing, but the broker legitimately parses subprocess JSON (nft / `ip route` / store-view runner output) in `ops/{store_view_farm,route,tap,store_sync_*}.rs`. The over-broad assertion needs narrowing. |

## Parallel W1 unit protocol

When a parallel unit retires `tests/X.sh`, it must keep its edits
partition-local:

1. Ensure every assertion in `X.sh` has a Rust/nextest, nix-unit, or
   other declared successor.
2. Write `tests/migration-state.d/X.toml` with `status = "retired"`
   and non-empty `successor_ids = [...]`.
3. Put any pinned successor names in
   `tests/golden/pinned/<batch>.txt` and/or add the unit-owned
   successor test under `packages/nixling-contract-tests/tests/`.
4. Delete `tests/X.sh`.

Unit branches must not edit `tests/tools/gen-migration-ledger.sh`,
`tests/migration-ledger.toml`, `tests/static.sh`, `tests/static-fast.sh`,
`AGENTS.md`, the Makefile, or other units' state/pinned files. In
particular, a unit branch must never run `make ledger-regen` or commit
`tests/migration-ledger.toml`: the ledger is a generated aggregate of
every unit's `tests/migration-state.d/*.toml`, so committing it from a
unit branch reintroduces a shared-file merge conflict and breaks the
octopus integration at scale. The integrator regenerates and commits the
ledger exactly once, after merging all units. AGENTS.md
critical-subsystem doc-reference updates are applied by the integrator at
merge.

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

Optional live-host datapath checks for the network isolation
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

Optional live-host audit pipeline check. The script looks for
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
| `test_sidecar_runner_present`         | At least one audio runner is declared in the daemon process graph. |
| `test_sidecar_socket_lifecycle`       | Starting the audio runner creates the UDS with `group=kvm mode=0660`; stopping the runner removes it. |
| `test_cli_status_smoke`               | `nixling audio status <vm>` reports the expected fields for an audio-enabled VM.          |
| `test_cli_grant_revoke`               | `nixling audio mic on / speaker on / mic off / off` round-trip: state file transitions correctly, sidecar lifecycle follows. |
| `test_cli_rejects_audio_disabled_vm`  | Trying `nixling audio mic on <non-audio-vm>` fails with a clear error.                    |
| `test_cloud_hypervisor_capabilities`  | The CH binary actually used by the runner is **v52 or newer** (CVE-2026-45782 fixed), has `--generic-vhost-user` (audio attach), AND has `--gpu` (spectrum graphics patches present). |
| `test_guest_sees_virtio_snd`          | (auto-SKIP if no audio-enabled VM is running): SSH into the running VM and verify `/proc/asound/cards` reports the virtio-snd device. |

> **Maintenance note**: the "Why this exists" paragraph below is a
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

Bridge-isolation enforcement (the DHCP anti-spoofing posture for
workload taps on `br-<env>-lan`) is covered by the reproducible VM check
`vmChecks.<system>.bridge-isolation`, not by the live-host Layer-2
scripts. It creates the bridge and network namespaces as root inside a
throwaway VM, then proves workload taps remain isolated from each other
while the net-VM port stays reachable.

## Layer 3 — reproducible `nixosTest`

`tests/nixos/bridge-isolation.nix` is exposed as
`vmChecks.<system>.bridge-isolation`. It proves that the Linux bridge
semantics nixling relies on match the documented threat model: the
net-VM port stays reachable while workload ports stay isolated, including
after a workload spoofs a peer-style MAC.

## Planned runtime tests

- **USBIP live isolation `nixosTest`**. The
  Layer-1 eval gate now proves host-side USBIP units, sockets, and
  firewall rules only materialize for envs with an enabled
  `usbip.yubikey` VM, but it still does not exercise live systemd
  socket materialization, iptables enforcement, or cleanup against a
  running guest. Lift that adversarial cross-env attach/isolation path
  into the runtime `nixosTest` suite.

- **Audit `--strict` graphics-VM running-check mock test**. The
  `bridge_isolated_workload.<vm>` running-check should use daemon
  running-state evidence for graphics VMs as well as headless VMs; without
  this, graphics VMs can be blanket-skipped by `nixling audit --strict`
  even when actively running. **Known gap:** this still needs a live
  host / higher-fidelity harness because the shell-application wrapper
  bakes `systemctl` in via `runtimeInputs`; a plain `PATH` stub is not
  enough to exercise the strict-audit path faithfully.

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

## Runtime nixosTest follow-ups

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
  as `outcome="echild-broker-recovered"`; add the metric arm alongside
  the runtime regression test.
