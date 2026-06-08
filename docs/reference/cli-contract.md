# nixling CLI contract

**Status:** stable from v0.1.0 onward.
**Audience:** anyone reimplementing the `nixling` CLI (the Rust port
in Phase 8 is the primary motivator).
**Companion docs:**
[`manifest-schema.md`](./manifest-schema.md) is the data contract;
this document is the *behavioural* contract.

## Scope

The CLI is the imperative half of the framework. Its job: read the
JSON manifest (the Nix-evaluated declarative state), interrogate
live systemd / kernel state, and reconcile the two via well-defined
operations on systemd units, kernel network state, and SSH-driven
guest activation.

Everything in this document is what a conforming CLI MUST do.
"MUST" / "SHOULD" / "MAY" follow RFC 2119 semantics.

## Subcommand inventory

The CLI exposes the subcommands below. The shell-level `nixling
<subcmd> --help` form is OPTIONAL; only `nixling --help` is REQUIRED.

| Subcommand                | Purpose                                                                |
|---------------------------|------------------------------------------------------------------------|
| `nixling list`            | Enumerate declared VMs + capabilities.                                 |
| `nixling up <vm> [-d]`    | Bring VM up. Interactive for graphics VMs; systemd-managed otherwise.  |
| `nixling down <vm>`       | Stop VM cleanly. `--force` to stop a net VM with running workloads.    |
| `nixling status [<vm>]`   | Service / process / SSH health. Always includes a per-bridge section.  |
| `nixling status --check-bridges` | Bridge health table only. Exit non-zero if any bridge unhealthy. |
| `nixling usb <vm>`        | YubiKey USBIP attach. Ctrl-C detaches.                                 |
| `nixling console <vm>`    | Foreground serial console (headless VMs only).                         |
| `nixling audio <subcmd>`  | Per-VM mic/speaker grant/revoke.                                       |
| `nixling build <vm>`      | Build the VM's closure on the host.                                    |
| `nixling switch <vm>`     | build + sync + `switch-to-configuration switch` over SSH (live).       |
| `nixling boot <vm>`       | build + sync + bump default boot only (next start).                    |
| `nixling test <vm>`       | build + sync + live activation, don't bump default boot.                |
| `nixling rollback <vm>`   | Roll the VM's in-VM generation back one step.                          |
| `nixling generations <vm>`| List host-side + in-VM generations.                                    |
| `nixling gc <vm>`         | Reclaim old per-VM hardlinks / generations.                            |
| `nixling trust <vm>`      | Trust the VM's SSH host key (post-rotation, manual confirm).           |
| `nixling rotate-known-host <vm>` | Remove the stale host-key entry for a VM after an intentional rotation; prompts to re-run `trust`. |
| `nixling keys list`       | List nixling-managed SSH keys + fingerprints + ages.                   |
| `nixling keys show <vm>`  | Print the public key for one VM (stdout, single line).                 |
| `nixling keys rotate <vm>`| Rotate a VM's nixling-managed key. Atomic + idempotent.                |
| `nixling audit [--strict] [--human]` | One-shot security/posture audit. JSON by default; `--human` emits a human-readable summary instead (auto-enabled when stdout is a TTY). `--strict` exits non-zero if any field deviates from the post-hardening target state. |

## Lifecycle state machine

The framework treats every VM as a 4-state machine:

```
                     ┌──────────────┐
                     │   stopped    │ ─ initial state
                     └──────┬───────┘
                            │ nixling up <vm>
                            ▼
                     ┌──────────────┐
                     │   starting   │ ─ systemd Activating
                     └──────┬───────┘
                            │ (sshd reachable on staticIp:22, or
                            │  graphics VM's Konsole launched)
                            ▼
                     ┌──────────────┐
                     │    ready     │ ─ systemd Active + SSH OK
                     └──────┬───────┘
                            │ nixling down <vm>
                            ▼
                     ┌──────────────┐
                     │   stopping   │ ─ systemd Deactivating
                     └──────┬───────┘
                            │ (process exits, sockets cleaned)
                            ▼
                     ┌──────────────┐
                     │   stopped    │
                     └──────────────┘
```

The CLI MUST NOT report `ready` until both:

1. The systemd unit (`microvm@<vm>.service` for headless,
   `nixling-<vm>-konsole.service` or equivalent for graphics) is in
   the systemd `active` state, AND
2. An SSH probe to `staticIp:22` with the configured `sshUser` +
   `sshKeyPath` returns success within a configured timeout
   (default 30s).

### Required systemd operations per subcommand

| Subcommand         | systemd / kernel operations                                                                                                                                                                                                                                                  |
|--------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `up` (headless)    | `systemctl start microvm@<vm>.service` (which transitively starts the per-VM `nixling-<vm>-virtiofsd.service`, `nixling-<vm>-swtpm.service` if `tpm`, `nixling-<vm>-snd.service` if `audio`).                                                                                  |
| `up` (graphics)    | Same as headless, plus: launch a foreground Konsole window via `konsole --qwindowtitle <vm>` running `ssh -i <sshKeyPath> <sshUser>@<staticIp>`. With `-d|--detach`, disown the cloud-hypervisor child and exit; without `-d`, `wait $VM_PID` and tear down on EXIT.        |
| `down`             | `systemctl stop microvm@<vm>.service`. Also stop sidecars: `nixling-<vm>-swtpm.service`, `nixling-<vm>-snd.service`. Sweep stale sockets in `stateDir`. If `<vm>` is a net VM and any workload VM in its env is still running, refuse unless `--force` is passed.            |
| `status`           | `systemctl is-active microvm@<vm>.service`; SSH probe to `staticIp:22`; `ip link show` for each declared bridge in the manifest.                                                                                                                                              |
| `usb`              | `usbip attach -r <usbipdHostIp> -b <busid>` (inside the VM, via SSH). On EXIT/INT/TERM: `usbip detach -p <port>` (inside the VM) + release any host-side exclusive-export lock.                                                                                              |
| `audio on|off <vm>`| Atomic write of `<audioStateFile>`; `systemctl restart <audioService>`. The restart blocks until `/run/nixling/vms/<vm>/snd.sock` is recreated.                                                                                                                                |
| `switch <vm>`      | `nix build` the VM's closure → `nixling-store-sync <vm>` (hardlinks new closure into `/var/lib/nixling/<vm>/store/`) → `ssh <sshUser>@<staticIp> sudo /run/current-system/sw/bin/switch-to-configuration switch`. No host rebuild. No VM reboot.                                |
| `keys rotate <vm>` | Generate new keypair under `<keysDir>/`; archive old keys under `<keysDir>/old/<ts>/`; push new pubkey to VM's `authorized_keys`; verify with new key; update known_hosts. If any step fails, restore from archive and exit non-zero.                                       |

## Signal / Ctrl-C semantics

### `nixling up <vm>` (foreground, no `-d`)

For **headless** VMs: not the typical mode (use `-d` or just
`systemctl start`). If used, blocks until the underlying systemd unit
transitions to inactive. SIGINT / SIGTERM: send `systemctl stop
microvm@<vm>.service` then exit with the unit's last exit code.

For **graphics** VMs: blocks on `wait $VM_PID` of the
cloud-hypervisor child. SIGINT (typically from closing Konsole or
Ctrl-C in the launching terminal) triggers a `cleanup` EXIT trap that:
- Sends a clean shutdown to the VM's API socket
  (`<apiSocket>`) — for cloud-hypervisor: `vmm.shutdown`.
- `systemctl stop`s the per-VM sidecars (`swtpm`, `snd`,
  `virtiofsd`).
- Deletes the tap device.
- Removes stale sockets in `<stateDir>`.

### `nixling up <vm> -d|--detach`

Disowns the cloud-hypervisor child and exits cleanly with status `0`
once the VM has reached `ready` (or the configured timeout, whichever
comes first). Subsequent SIGINT to the parent shell does NOT affect
the running VM.

### `nixling down <vm>`

Sends `systemctl stop`. Returns when the unit transitions to inactive
or after a default 60s timeout (whichever comes first). SIGINT to
`nixling down` propagates: the underlying `systemctl stop` is
cancelled, and the CLI exits with status `130` (128 + SIGINT). The
VM may be left in `stopping` state.

### `nixling usb <vm>`

Blocks in the foreground after a successful attach. The terminal is
a clear visual indicator that the YubiKey is currently routed to the
VM. SIGINT (Ctrl-C) or SIGTERM triggers the `cleanup_usb` trap:
- `usbip detach -p <port>` inside the VM.
- Release host-side exclusive-export lock
  (`/run/nixling/usbipd.lock`).
- Exit with the trap-captured exit status.

The cleanup runs on EXIT as well, so any unexpected exit also
releases the YubiKey.

### `nixling audio …`

Non-blocking. Returns once `<audioService>` reaches `active` again
(or its restart fails). SIGINT exits with `130`; the audio-state
file write is atomic (`mktemp` + `rename(2)`), so the on-disk state
is either fully-old or fully-new — never partial.

### `nixling console <vm>`

Same semantics as `up` foreground for headless VMs — the call
unblocks when the serial console exits (typically on guest shutdown
or `~.`). SIGINT propagates to the serial-console subprocess.

### Long-running activations (`switch` / `boot` / `test`)

The host-side `nix build` step is interruptible: SIGINT stops the
build and exits `130`. The SSH-driven `switch-to-configuration`
step is NOT interruptible from the host side once it has been
issued — interrupting the CLI leaves the VM mid-activation. Recovery:
re-run `nixling switch <vm>` or `nixling rollback <vm>`.

## Audio / USBIP hot-grant behavior

### Audio

`audio.enable = true` on a VM gives the framework permission to
mediate host mic/speaker into the VM. Whether the device is
ACTUALLY routed in any given moment is controlled by the per-VM
state file at `audioStateFile`:

```json
{ "mic": "off", "speaker": "off" }
```

(defaults per `audio.allowMicByDefault` / `audio.allowSpeakerByDefault`).

CLI operations:

- `nixling audio status` — print all VMs' current `(mic, speaker)`.
  MUST support `--json` for machine consumption.
- `nixling audio mic on|off <vm>` — set `mic`. Atomic write.
- `nixling audio speaker on|off <vm>` — set `speaker`. Atomic write.
- `nixling audio on|off <vm>` — shorthand for setting both.

After every state change, the CLI MUST `systemctl restart
<audioService>`. The restart unit blocks until
`/run/nixling/vms/<vm>/snd.sock` is recreated, so the CLI doesn't
need its own readiness probe.

Safety: the CLI MUST refuse to write the audio state file if its
parent directory is a symlink or has unsafe permissions (the bash
implementation checks for `stat_d != root:root mode 0700`).

### USBIP YubiKey

YubiKey routing is exclusive — only one env may hold the device at a
time. The CLI MUST acquire an exclusive flock on
`/run/nixling/usbipd.lock` before attaching, and MUST release it on
EXIT (whether clean or via signal).

`nixling usb <vm>` workflow:

1. Verify `usbipYubikey = true` for `<vm>`.
2. Verify a Yubico device (vendor 0x1050) is plugged into the host.
3. Pick the per-env usbipd proxy via `usbipdHostIp`.
4. SSH to the VM and run `usbip attach -r <usbipdHostIp> -b <busid>`.
5. Block in foreground (the terminal is the "key is in the VM"
   indicator).
6. On EXIT/INT/TERM: SSH `usbip detach -p <port>`; release flock.

Switching envs requires explicit `nixling usb <other-vm>` — the new
attach triggers the previous attach's cleanup transitively.

## Exit codes

| Code | Meaning                                                                                                          |
|------|------------------------------------------------------------------------------------------------------------------|
| `0`  | Success.                                                                                                         |
| `1`  | Generic operational failure (a child process exited non-zero; a probe failed; an unexpected error).              |
| `2`  | Usage error: missing/invalid argument, unknown flag, unknown subcommand, unknown VM name, capability not enabled.|
| `3`  | VM unreachable over SSH (host probe failed; not a usage error — the user has to run `nixling up` first).         |
| `4`  | Health-check / readiness timeout, OR manifest schema incompatibility (CLI built for v<N>, manifest emits v>N).   |
| `5`  | Resource conflict: another `nixling` instance holds the lock; a per-env usbipd is already exporting elsewhere.   |
| `130`| Terminated by SIGINT (Ctrl-C). Standard 128 + signal-number convention.                                          |
| `143`| Terminated by SIGTERM. Standard 128 + signal-number convention.                                                  |

The current bash implementation does not use code `4` for schema
incompatibility (that case can't fire today — only one version
exists). A future Rust port MUST use `4` for both timeout AND schema
incompatibility, distinguishing via stderr message.

## Human vs JSON output

> **v0.1.0 status:** `--json` is **not implemented** in the bash CLI
> for any subcommand. The flag has been deferred to v0.2.0 (Rust CLI
> port). For machine-readable VM metadata in v0.1.0, consume the
> manifest at `/run/current-system/sw/share/nixling/vms.json`
> directly — that file is the authoritative JSON contract (versioned
> via `manifestVersion`; schema in
> [`manifest-schema.md`](./manifest-schema.md)). See "Future" below.

| Subcommand                  | Human (default)     | JSON support (v0.2.0 plan) |
|-----------------------------|---------------------|----------------------------|
| `list`                      | tabular (TTY)       | `--json` planned           |
| `status`                    | human               | `--json` planned           |
| `status --check-bridges`    | tabular             | `--json` planned           |
| `audio status`              | tabular             | `--json` planned           |
| `keys list`                 | tabular             | `--json` planned           |
| `audit`                     | JSON (default) / human (`--human`, auto on TTY) | always supports both     |
| `generations`               | tabular             | `--json` planned           |
| `up` / `down` / `usb` / `console` / `audio on/off` / `switch` / `boot` / `test` / `rollback` / `gc` / `trust` / `rotate-known-host` / `keys rotate` | human progress messages on stderr; nothing on stdout on success. | not applicable. |

`audit` is the only subcommand that emits JSON natively in v0.1.0
(it predates this contract — kept as-is for backwards compatibility).

**JSON output rules (apply to `audit` today, all subcommands in v0.2.0):**

- MUST emit a single JSON document on stdout (newline-terminated).
- MUST NOT emit progress messages or warnings on stdout when in
  `--json` mode. Use stderr.
- MUST set process exit code per the table above; the human / JSON
  bit doesn't change exit-code semantics.
- SHOULD use the same field names as `docs/reference/manifest-schema.md` for
  fields that originate from the manifest, so consumers can pipe
  one CLI's output into another tool that expects the manifest
  shape.

### Future

- **`--json` on `list` / `status` / `status --check-bridges` /
  `audio status` / `keys list` / `generations`**. Deferred to
  v0.2.0 for the Rust CLI port. The bash implementation will not
  grow these flags; the Rust port lands them as a single batch
  alongside the lifecycle FSM rewrite.

## Idempotency

The following MUST be idempotent (calling them N times has the same
effect as calling them once):

- `up` of an already-`ready` VM — exits `0`, no state change.
- `down` of an already-`stopped` VM — exits `0`, no state change.
- `audio on/off` to the current state — exits `0`, no restart
  (CLIs SHOULD check the existing state before writing to avoid
  spurious `audioService` restarts).
- `keys rotate` mid-rotation — recovers from a half-finished rotate
  (archive present but pubkey not pushed) by completing it.
- `gc` — no-op when nothing to reclaim.

## Concurrency

Two CLI invocations on the same host MAY run concurrently as long
as they touch different VMs. Operations touching the same VM
serialise via per-VM systemd unit state (a `systemctl start` while
another start is in progress no-ops correctly).

The one cross-VM resource is the YubiKey: `nixling usb` MUST hold
the `/run/nixling/usbipd.lock` flock for the duration of the attach,
so a second `nixling usb` blocks until the first releases.

## What is NOT in this contract

The following are framework-internal implementation details and MAY
change without a CLI-version bump. Consumers reimplementing the CLI
MUST NOT depend on any of these except through the manifest fields
that surface them.

### Framework-internal unit names

Only these systemd unit names ARE part of the contract (the CLI MAY
target them by name and downstream consumers MAY assume they exist):

- `nixling@<vm>.service` — the user-facing per-VM wrapper. Targeted
  by `nixling up`, `nixling down`, polkit grants, and the
  `nixling-launcher` group's allowlist.
- `microvm@<vm>.service` — the microvm.nix-template backend. The
  bring-up path for headless VMs and the implicit dependency of
  every `nixling@<vm>` wrapper. Referenced in journalctl flows and
  in the legacy `nixling list` running-detection fallback.

All OTHER unit names listed in this document — including
`nixling-<vm>-virtiofsd.service`, `nixling-<vm>-swtpm.service`,
`nixling-<vm>-snd.service`, `nixling-<vm>-gpu.service`,
`nixling-<vm>-store-sync.service`, `nixling-sys-<env>-usbipd-*` —
are framework-internal. They appear in the contract only to describe
the operations the CLI's lifecycle subcommands transitively perform;
their NAMES are not stable. The CLI MUST read service identifiers
from the manifest where possible (e.g. `audioService` field) rather
than hardcoding the templates.

### Framework-internal implementations

- **microvm.nix internal lifecycle.** The `microvm@<vm>.service`
  unit's internal ordering, ExecStart wrapper layout, env-var
  passing, and how it spawns cloud-hypervisor / crosvm /
  virtiofsd are all microvm.nix-owned. The CLI MUST NOT assume
  any particular runner is in use; the manifest's `apiSocket`,
  `gpuSocket`, and `tpmSocket` are the only socket paths it MAY
  read.
- **swtpm internals.** The wire protocol between swtpm and the
  guest kernel, the on-disk swtpm state layout under
  `<stateDir>/swtpm/`, and the host-side
  `nixling-<vm>-swtpm.service` unit's ordering against
  microvm.nix are all framework-internal. The CLI MAY observe
  `tpmSocket` and `tpm` (capability bit) from the manifest;
  beyond that it MUST NOT interact with swtpm directly.
- **virtiofsd implementation.** The virtiofsd binary used (the
  reference implementation vs. crosvm-bundled vs. cloud-
  hypervisor-bundled), its IPC details, and the per-VM
  `microvm-virtiofsd@<vm>.service` template are all implementation
  detail. Consumers MUST NOT depend on a particular virtiofsd
  flavour or unit-name layout.
- **Polkit grant specifics.** The polkit rules backing the
  `nixling-launcher` group — which unit-name patterns are
  allowlisted, which polkit actions are unlocked, the exact
  JS-rule shape — are subject to tightening with every release.
  The contract here is functional: members of `nixling-launcher`
  CAN `systemctl start/stop/restart` the units listed under
  "Framework-internal unit names" above. The mechanism is not
  promised.

### Cosmetic / observability surfaces

- The exact wording of human-readable messages on stderr.
- The exact ordering of fields in human tabular output. (JSON
  fields ARE part of the contract.)
- The internal `stateDir` layout — read paths from the manifest's
  `stateDir`, `apiSocket`, `gpuSocket`, `audioStateFile`.

## Reference implementation

`nixos-modules/cli.nix` is the current bash implementation. It is
authoritative for v0.1.0. The Rust port is expected to land in
v0.2.0 alongside `manifestVersion = 2` (if any schema changes are
needed to support it).

## Versioning

The CLI's own version is independent of `manifestVersion`. A given
CLI build declares the highest manifest version it understands; see
[`manifest-schema.md`](./manifest-schema.md#compatibility-policy)
for the version-mismatch rules.

The behavioural contract documented here SHOULD be considered
stable from `nixling v0.1.0` onward. Breaking changes to subcommand
names, exit codes, or signal semantics warrant a major-version bump.
Additive changes (new subcommands, new flags) do not.
