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
| Per-VM audio sidecar           | `nixling-<vm>-snd.service`               |
| Per-VM TPM emulator            | `nixling-<vm>-swtpm.service`             |
| Per-VM store sync              | `nixling-<vm>-store-sync.service`        |
| Per-env USBIP proxy            | `nixling-sys-<env>-usbipd-proxy.{service,socket}` |
| Per-env USBIP backend          | `nixling-sys-<env>-usbipd-backend.service` |
| Auto-declared per-env net VM   | `nixling@sys-<env>-net.service` (workload-side `microvm@sys-<env>-net.service` backend) |
| Polkit launcher group          | `nixling-launcher` (singleton)           |

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
`_manifest` top-level key carries `manifestVersion = 1` (the first
documented/stable schema; W4 Phase 5). User-facing VM names cannot
start with `_` (eval-time assertion in
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
- `nix flake check --no-build --all-systems` — both x86_64-linux and
  aarch64-linux flake outputs eval clean.
- `tests/smoke-eval.nix` — minimal consumer-style nixosSystem on the
  builder's system (typically x86_64-linux).
- `tests/smoke-eval-aarch64.nix` — same shape, cross-evaluated on
  aarch64-linux to verify the headless workload eval graph stays
  multi-arch clean (Phase 4 W4 gate).
- `tests/assertions-eval.sh` — 10/10 eval-time-assertion regression
  tests (CIDR shape, CIDR overlap, key validation, `waylandUser`
  presence, graphics/audio platform gating, etc.).
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

## Layer 3 — reproducible `nixosTest`

Out of scope for now. Sketch is in the refactor plan for the
follow-up: a `flake.nix#checks.x86_64-linux.nixling-store` output that
boots a dummy nested host + microVM and drives the same assertions.

## Future tests (Phase 7a / v0.2.0)

- **Static lint for the `mkOption { default = …; readOnly = true; }`
  + matching `config.<…>` trio.** Spec correction #29 (the
  `nixling.manifest` default landed alongside a `config` assignment
  and `readOnly = true`, which is a silent overlay-only update
  pitfall) was caught by the W5 reviewer panel, not by tooling.
  Phase 7a will add a grep-level lint that scans every
  `nixos-modules/*.nix` for the full three-of-three trio (a
  two-of-three match — e.g. `store.nix` carrying `readOnly +
  default` on options that have **no** matching `config.<…>`
  assignment — is intentional and must NOT trip the lint).

> Per-example iteration is now part of the static gate:
> `tests/static.sh` iterates every `examples/*/flake.nix` and runs
> `nix flake check --no-build --all-systems` against each. The
> `with-entra-id` example reaches a sibling flake
> (`vicondoa/nixos-entra-id`), so its iteration step is not fully
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
