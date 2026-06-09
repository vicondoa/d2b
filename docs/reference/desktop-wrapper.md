# Desktop wrapper contract

**Status:** stable.
**Owner:** the legacy `nixos-modules/cli.nix` (`vmLaunchScript` + `vmLaunchContract`) was retired in v1.0 per ADR 0015; the v1.0 launcher is the Rust CLI at `packages/nixling/src/lib.rs`, dispatched through `nixlingd` → broker.
**Test gate:** [`tests/desktop-wrapper-contract-eval.sh`](../../tests/desktop-wrapper-contract-eval.sh).
**Schema version:** `1`.

Every graphics-enabled VM (`nixling.vms.<vm>.graphics.enable = true`)
gets an auto-generated `nixling-launch-<vm>.desktop` entry installed
under `share/applications/`. The wrapper script the entry's `Exec`
line points at is the **daemon path** — it drives the VM through
`nixlingd → nixling-priv-broker → SpawnRunner`, not the legacy bash
`nixling vm start` / `microvm@<vm>.service` chain.

## Why a typed contract

The `.desktop` Exec line is part of the operator-visible UX surface,
and silent drift between "what the wrapper does" and "what the
docs / panels claim the wrapper does" is a recurring failure mode
(KDE session restore re-launches stale wrappers at login; an Exec
line that previously could have invoked a now-deleted bash codepath would
look indistinguishable from one that didn't).

The contract is exposed via the internal NixOS option
`nixling._desktopWrappers.<vm>` so the regression gate can pin every
field at eval time without scraping the rendered `.desktop` file out
of the store.

## Contract fields

| Field | Value (for VM `<vm>`) | Why pinned |
| --- | --- | --- |
| `schemaVersion` | `1` | Bumped only when this table changes. |
| `vm` | `"<vm>"` | Identity. |
| `execProgram` | `${nixling}/bin/nixling` (the Rust CLI) | The legacy bash CLI was retired in v1.0 per ADR 0015; the wrapper MUST point at the Rust binary. |
| `execArgv` | `[ "vm" "start" "<vm>" "--apply" ]` | The daemon-native lifecycle verb. Replaces `nixling vm start <vm> -d`. |
| `execEnv.NIXLING_NATIVE_ONLY` | `"1"` | In v1.0 (per ADR 0015) the daemon path is the default and the bash fallback was retired in v1.0; the env var is a no-op. The wrapper still sets it for historical traceability with pre-v1.0 desktop entries. |
| `outputMode` | `"json"` | `nixling vm start --apply --json` emits the typed envelope so failures are parseable. |
| `waitForHostCompositor` | `true` | Wrapper waits up to 30 s for the host `$WAYLAND_DISPLAY` socket before invoking the daemon. The GPU sidecar's cross-domain bind-mount target must exist when the runner starts. |
| `hostCompositorSocketEnv` | `"WAYLAND_DISPLAY"` | The env var the wrapper resolves to find the host compositor's socket under `$XDG_RUNTIME_DIR`. |
| `waitForGpuSocket` | `"/run/nixling-wlproxy/<vm>/wayland-0"` | After the daemon reports the VM up, the wrapper waits up to 30 s for the host-side Wayland filter socket when the filter is enabled. The guest `wl-cross-domain-proxy` path reaches the host compositor through this filtered socket. |
| `failureSurfaces` | `[ "notify-send" "log" ]` | Daemon failures surface as a `notify-send` desktop bubble and an appended line in the per-VM launcher log. |
| `failureLogPath` | `${XDG_STATE_HOME:-$HOME/.local/state}/nixling/launchers/<vm>.log` | Operator-readable forensic trail beyond the transient `notify-send` bubble. The daemon's `--json` stdout/stderr lands here verbatim. |
| `scriptText` | `string` | Full script body. Tests assert load-bearing substrings. |
| `scriptPath` | `/nix/store/…-nixling-launch-<vm>` | Final wrapper script the `.desktop` `Exec=` line points at. |

## Wrapper lifecycle

1. **Wait for host compositor.** Resolves
   `${XDG_RUNTIME_DIR}/${WAYLAND_DISPLAY:-wayland-0}`; bails with
   `notify-send` if absent after 30 s. Exports `WAYLAND_DISPLAY` and
   `DISPLAY` for the in-VM client.
2. **Drive the daemon.** Runs
   `NIXLING_NATIVE_ONLY=1 nixling vm start <vm> --apply --json`,
   appending stdout/stderr to `$XDG_STATE_HOME/nixling/launchers/<vm>.log`.
   On non-zero exit, parses the trailing JSON envelope with `jq`,
   extracts `errorKind` / `operationId` / `remediation`, and surfaces
   them via `notify-send`. Always points the operator at:
   - `nixling status <vm>` (per-VM state)
   - `journalctl -u nixlingd.service` (daemon log)
   - the per-VM launcher log
3. **Wait for the per-VM GPU wayland socket.** Polls
   `/run/nixling-wlproxy/<vm>/wayland-0` for up to 30 s. The daemon's
   `ssh-ready` DAG node only guarantees sshd; the filter socket (and
   through it the GPU sidecar's cross-domain channel) can race
   slightly behind on cold starts.
4. **Wait for SSH.** Probes `<sshUser>@<staticIp>` with the per-VM
   key (copied to a 0600 tempfile because the source is 0640
   `root:nixling`). Classifies failure into "host key
   mismatch" vs "not reachable" and surfaces the right remediation
   via `notify-send`.
5. **Exec Konsole.** Replaces the wrapper with a chromed Konsole
   running `ssh` to the VM. `StartupWMClass=org.kde.konsole` matches
   Konsole's fixed Wayland app_id.

## Failure envelope surfacing

The wrapper invokes the daemon with `--json` precisely so failures
produce a typed envelope on stdout (per
[`docs/reference/error-codes.md`](./error-codes.md) and
[`docs/reference/daemon-api.md`](./daemon-api.md)). The wrapper
parses, in order of preference:

| Envelope field | Used as |
| --- | --- |
| `brokerErrorKind` / `errorKind` / `kind` | `kind=` in the notify bubble. |
| `operationId` / `publicOperationId` | `op=` in the notify bubble (for cross-referencing with the daemon log / broker audit). |
| `remediation` | Body line of the bubble. |

If parsing fails (e.g. the daemon process died before emitting an
envelope), the bubble still points the operator at the launcher log
and `journalctl -u nixlingd.service`.

## What this contract does NOT cover

- The `.desktop` file's discoverability metadata (`Name`,
  `Keywords`, `Categories`, `StartupWMClass`) is intentionally out
  of scope; that's UX styling, not a lifecycle contract. See
  `nixos-modules/cli.nix` `desktopItems`.
- The in-VM session opened over SSH is not pinned here. The wrapper
  hands off to Konsole + sshd; what the operator runs once they're
  in the VM is their concern.
- Headless VMs do not get a `.desktop` wrapper at all
  (`graphics.enable = false` is filtered out).

## Drift policy

Any change to the table above must:

1. Bump `desktopWrapperContractVersion` in `nixos-modules/cli.nix`.
2. Update this doc in the same commit.
3. Update `tests/desktop-wrapper-contract-eval.sh` to assert the
   new shape.
4. Add a CHANGELOG entry under `## Unreleased`.
