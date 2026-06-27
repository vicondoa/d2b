# `d2b.vms.<vm>.audio.*`

> Reference for the `audio` component module.
> Source (host): [`nixos-modules/components/audio/host.nix`](../../nixos-modules/components/audio/host.nix)
> Source (guest): [`nixos-modules/components/audio/guest.nix`](../../nixos-modules/components/audio/guest.nix)
> CLI: `packages/d2b/src/lib.rs` (`d2b audio …`); there is no bash helper for this surface.

## What this component does

Gives a VM a virtio-snd soundcard backed by a per-VM
`vhost-device-sound` sidecar that connects to the host's PipeWire
session. On the host the sidecar appears as a PipeWire client named
`d2b-<vm>` (visible in `wpctl status`, plasma-pa, pavucontrol);
inside the guest, normal PipeWire + ALSA + PulseAudio compat stacks
work on top of the virtio-snd card.
Audio-capable VMs use the Cloud Hypervisor runtime provider plus the
broker-spawned sound sidecar; see
[runtime provider selection](./runtime-provider-selection.md) for runtime
provider capability boundaries.
Audio remains separate from display; see
[display and virtual I/O capabilities](./display-io-capabilities.md).

Console and audio are provider-capability-aware daemon surfaces; the
per-provider capability matrix and enforcement model are documented in
[provider capability matrix](./provider-capability-matrix.md) and
[ADR 0041](../adr/0041-console-and-audio-controls.md). The matrix covers
Cloud Hypervisor NixOS VMs (vhost-user-sound + guestd enforcement),
qemu-media targets (host/qemu subset only; guest enforcement unsupported),
and ACA sandboxes (remote guestd policy only; no local host mutations).

Each VM has independent **mic** and **speaker** grants. The
host-side state file is `/var/lib/d2b/vms/<vm>/state/audio-state.json`.
The sidecar publishes the resolved mic/speaker state as custom
PipeWire properties (`d2b.mic`, `d2b.speaker`); a
host-side `client.conf.d` rule reads those properties and
null-routes the corresponding stream direction with
`target.object = "-1"` when it's `off`. Setting `audio.enable = true`
only enables the *capability* — the current `d2b audio` Rust
CLI verbs return typed `not-yet-implemented` exit-78 envelopes, so
there is no daemon-native audio control plane yet. Both directions
default to `off` on first materialisation unless the
`allow{Mic,Speaker}ByDefault` options are flipped.

When both directions are off, the guest-side
`microvm.extraArgsScript` short-circuits and does **not** emit
`--generic-vhost-user` — the guest sees no soundcard at all.

## Options (host-side)

| Option | Type | Default | Description |
|---|---|---|---|
| `d2b.vms.<vm>.audio.enable` | bool | `false` | Enable per-VM audio capability. Incompatible with `autostart = true` (asserted at eval). Implies `hypervisor = cloud-hypervisor`. |
| `d2b.vms.<vm>.audio.allowMicByDefault` | bool | `false` | Initial value of the `mic` field when the per-VM state file is first materialised. Consulted at creation time only. |
| `d2b.vms.<vm>.audio.allowSpeakerByDefault` | bool | `false` | Initial value of the `speaker` field when the per-VM state file is first materialised. Consulted at creation time only. |
| `d2b.vms.<vm>.audio.users` | list of str | `[ ]` (defaults to `[ ssh.user ]` if non-null) | Guest-side usernames added to the `audio` group inside the VM. virtio-snd exposes `/dev/snd/*` as `0660 root:audio`; non-logind-active users need explicit group membership. |

Site-level dependency:

| Option | Type | Required when | Description |
|---|---|---|---|
| `d2b.site.waylandUser` | nullable str | any VM has `audio.enable = true` | The host's primary Wayland session user. Its `/run/user/<uid>/pipewire-0` socket is bind-mounted into the sidecar's private mount namespace. Eval fails clearly if unset. |
| `d2b.site.audio.inputTargetNode` | nullable str | optional | PipeWire `node.name` to force VM microphone streams to when `d2b.mic = "on"`. Leave `null` to let WirePlumber auto-select the default source. Useful on hosts where capture clients are not auto-linked reliably. |

## Options (guest-side propagation)

`host.nix` propagates `audio.users` into the guest with a `mkIf`
gate (falls back to `[ ssh.user ]` if the per-VM list is empty):

```nix
(lib.mkIf vm'.audio.enable {
  d2b.audio.users =
    if vm'.audio.users != [ ]
    then vm'.audio.users
    else lib.optional (vm'.ssh.user != null) vm'.ssh.user;
})
```

The matching guest-visible option (declared in
`components/audio/guest.nix`):

| Option | Type | Default | Description |
|---|---|---|---|
| `d2b.audio.users` | list of str | `[ ]` | Guest-side usernames added to the `audio` group. Each named user gets `users.users.<u>.extraGroups += [ "audio" ]`. |

## Host-side resources created

> There is no `d2b-<vm>-snd.service` systemd unit. The broker
> spawns the audio runner under `d2b.slice/<vm>/snd`, and the
> hardening shape documented below is enforced as the runner
> contract. The `d2b audio mic|speaker|status` CLI verbs are
> rust-native shims that currently return a typed
> `not-yet-implemented` envelope.

Per audio-enabled VM:

- **`d2b-<vm>-snd` system user + group**
  ([`host-users.nix`](../../nixos-modules/host-users.nix)).
- **`d2b.slice/<vm>/snd` runner**
  ([`components/audio/host.nix`](../../nixos-modules/components/audio/host.nix))
  — vhost-user-sound sidecar, broker-spawned via the daemon DAG.
  Runs as `d2b-<vm>-snd:d2b-<vm>-snd`,
  `SupplementaryGroups = [ "audio" ]`. The runner is started on
  demand, with a belt-and-suspenders fallback at VM boot, and is
  ordered before the GPU runner.
  - `RuntimeDirectory = "d2b/vms/<vm>"`, mode 0700.
  - `BindPaths` binds `/run/user/<wayland-uid>/pipewire-0` into the
    sidecar's private runtime dir; the sidecar never sees
    `/run/user/<uid>` itself.
  - The broker-spawned runner environment sets
    `PIPEWIRE_RUNTIME_DIR=/run/user/<uid>`,
    `XDG_RUNTIME_DIR=/run/user/<uid>`, and `PIPEWIRE_PROPS` with
    `application.name = "d2b-<vm>"`, `node.name =
    "d2b-<vm>"`, and `node.description = "d2b <vm>"`.
    This gives plasma-pa / pavucontrol a per-VM host application name
    without executing a mutable runtime-dir binary.
  - `ExecStart`: `vhost-device-sound --socket
    /run/d2b/vms/<vm>/snd.sock --backend pipewire` with argv[0]
    set to `d2b-<vm>-snd`.
  - `ExecStartPost`: polls for `snd.sock` up to 30 s, then
    `setfacl -m u:d2b-<vm>-gpu:x /run/d2b/vms/<vm>` and
    `setfacl -m u:d2b-<vm>-gpu:rw .../snd.sock`. Fails the unit
    hard if the socket never materialises.
  - `Restart = "no"` — on-demand only.
- **State file** `/var/lib/d2b/vms/<vm>/state/audio-state.json`
  (mode 0640, owner `root:d2b`), initial contents
  `{"mic":"<allowMic>","speaker":"<allowSpeaker>"}`. The containing
  `state/` directory is mode 0750 `root:d2b`. ACLs
  grant `d2b-<vm>-gpu` `rx` on the directory and `r` on the
  file so the GPU sidecar can read state at VM-boot time without
  joining `d2b`.
- **Lock file** `/run/d2b/audio-<vm>.lock`, mode 0660,
  `root:d2b`. The current Rust CLI shim returns
  exit-78 and does not acquire this lock because the daemon-native
  audio control plane is not yet available.
- **`vhost-device-sound`** vendored at
  `pkgs/vhost-device-sound/` because the nixpkgs version has a known
  PipeWire-backend format-negotiation bug. Added to
  `environment.systemPackages` for ad-hoc operator debugging.

Per any audio-enabled host (emitted when at least one VM has
`audio.enable = true`):

- **PipeWire `client.conf.d/90-d2b.conf` `stream.rules`** stamps
  early block properties for disabled directions.
- The broker stamps final `PIPEWIRE_PROPS` at audio-runner spawn time
  from `audio-state.json`: `application.name`, `node.name`,
  `node.description`, `d2b.vm`, `d2b.mic`, and
  `d2b.speaker`. When `d2b.site.audio.inputTargetNode` is
  set and mic is `on`, the broker also adds `target.object` so the
  vhost-device-sound capture stream links to that host source at
  creation time. When no explicit input target is configured and the
  direction is `on`, WirePlumber's normal default-target hook selects
  the host source.

CLI (`d2b audio` in the Rust CLI — currently a rust-native shim
that returns typed `not-yet-implemented` exit-78; there is no bash
helper and no daemon-native audio control plane yet):

- `d2b audio mic on|off <vm>`
- `d2b audio speaker on|off <vm>`
- `d2b audio off <vm>` — shorthand for both off.
- `d2b audio status` / `d2b audio status <vm>` — reports
  current grant state per VM.

## Lifecycle

`d2b-<vm>-snd.service` carries `restartIfChanged = false`
(matches the [graphics sidecar lifecycle policy](./components-graphics.md#lifecycle-v015)).
A `nixos-rebuild switch` updates the unit file but does NOT cycle
the running `vhost-user-sound` sidecar — vhost-user-sound's socket
connection to cloud-hypervisor cannot survive a restart, and
killing this sidecar mid-VM produces silent speakers and mic
stuck in whatever state it was in. After a rebuild, `d2b
list` flags the VM with `[pending restart]` if its `current`
closure has drifted from `booted`; apply with `d2b vm restart
<vm> --apply` (clean down+up cycles the audio sidecar and CH together so
the socket gets re-established). See
[`docs/reference/cli-contract.md` — Pending-restart signal](./cli-contract.md#pending-restart-signal-v015).

## Guest-side resources created

In [`components/audio/guest.nix`](../../nixos-modules/components/audio/guest.nix):

- `microvm.hypervisor = "cloud-hypervisor"` (via `mkDefault`).
- `microvm.extraArgsScript = audioArgsScript` — a shell helper
  invoked by microvm.nix's runner at VM start. Reads
  `audio-state.json`; if both directions are off, emits nothing
  (no virtio-snd device at all). Otherwise:
  1. `systemctl reset-failed d2b-<vm>-snd.service` (tolerant).
  2. `systemctl start d2b-<vm>-snd.service` (tolerant).
  3. Polls for `/run/d2b/vms/<vm>/snd.sock` up to 5 s.
  4. Prints `--generic-vhost-user socket=...,virtio_id=25,
     queue_sizes=[64,64,64,64]`. virtio_id 25 = "sound" per the
     virtio spec; queue_sizes is a 4-element list matching
     vhost-device-sound's ctrl + event + tx + rx queues.
- `boot.kernelModules = [ "snd_virtio" ]`.
- `services.pulseaudio.enable = lib.mkForce false`.
- `security.rtkit.enable = true`.
- `services.pipewire.enable = true` with `alsa.enable`,
  `alsa.support32Bit`, `pulse.enable`.
- `environment.systemPackages` += `pipewire`, `wireplumber`,
  `alsa-utils`.
- `users.users.<u>.extraGroups += [ "audio" ]` for each user in
  `d2b.audio.users`.
- WirePlumber drop-in `91-d2b-virtio-snd` under
  `monitor.alsa.rules` that matches the virtio-snd PCI card and
  pins `device.profile = "pro-audio"`, `api.alsa.use-acp = false`.
  Without that, WirePlumber leaves the card in "Off" mode (no Sink
  / Source created) because virtio-snd has no ACP entry.

## Runtime invariants

- The sidecar's `application.name` in plasma-pa / `wpctl status` is
  always `d2b-<vm>` (set through `PIPEWIRE_PROPS`). Per-stream
  mute/volume in plasma-pa applies to that client.
- When both directions are `off`, no virtio-snd device is attached
  to CH and no sidecar process exists. Setting either direction on
  starts the sidecar; setting both off again does NOT teardown the
  device on the running VM (would unplug the soundcard mid-flight)
  — the WirePlumber rule null-routes the streams instead.
- Audio-enabled VMs MUST have `autostart = false`. Eval-time
  assertion in `audio/host.nix` enforces this so the daemon does not
  start a graphics/audio VM without an operator Wayland session.
- The state file always lives at
  `/var/lib/d2b/vms/<vm>/state/audio-state.json`. A one-time
  activation-script migration moves the legacy
  `/var/lib/d2b/vms/<vm>/audio-state.json` location if it
  exists.

## Hardening notes

`d2b-<vm>-snd.service` is the security baseline for
sidecar-as-system-service. Compared to the GPU sidecar template:

- `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`,
  `PrivateTmp`, `PrivateDevices`, `ProtectKernelTunables/Modules`,
  `ProtectControlGroups`, `ProtectClock`, `ProtectHostname`,
  `ProtectProc=invisible`, `LockPersonality`, `RestrictNamespaces`,
  `SystemCallArchitectures=native`, `SystemCallFilter =
  [ "@system-service" ]`, `UMask = "0077"`.
- **`MemoryDenyWriteExecute = true`** — unlike the GPU sidecar,
  vhost-device-sound has no JIT, so MDWE is on. This is the visible
  delta from the graphics-sidecar template.
- `RestrictAddressFamilies = [ "AF_UNIX" "AF_NETLINK" ]` —
  vhost-user is AF_UNIX, libpipewire uses AF_NETLINK for some
  introspection.
- **`RestrictRealtime = false`** with `LimitRTPRIO = 95`,
  `LimitNICE = -19`, `LimitMEMLOCK = 4194304`. libpipewire elevates
  its mixing thread to SCHED_FIFO; blocking that produces dropped
  frames and audible static on the host's own playback.
- `ReadWritePaths = [ "/run/d2b/vms/<vm>" ]`.
- `BindPaths` exposes only `pipewire-0`, not the parent runtime
  directory.
- The sidecar executable stays in the immutable Nix store. The
  per-VM host-visible name is provided through `PIPEWIRE_PROPS`,
  avoiding executable copies in the writable runtime directory.

`audio-state.json`:

- Lives in a root-owned non-group-writable subdir `state/` (mode
  0750 `root:d2b`). The parent
  `/var/lib/d2b/vms/<vm>/` remains `microvm:kvm 2775` so the
  CLI can take the audio lock and write temp files there, but no
  kvm-group process can unlink/replace the state file.
- File mode 0640 `root:d2b`. Read by the GPU sidecar
  via an explicit named-user ACL grant for `d2b-<vm>-gpu`.

## Common gotchas / failure modes

- **VM silent / no soundcard in `aplay -l`.** Both mic and speaker
  are off — `d2b audio status <vm>` will confirm. Toggle one on
  and restart the VM (`d2b vm stop <vm> --apply && d2b vm start <vm> --apply`) so
  `audioArgsScript` re-emits `--generic-vhost-user`.
- **Audible static / dropped frames on the host's own playback
  while a d2b VM is running.** Almost always a WirePlumber
  rule misplacement re-introducing the USB-headset duplex-mode bug.
  Verify the `90-d2b.conf` rule is in PipeWire's `client.conf.d/`
  (NOT WirePlumber's `wireplumber.conf.d/`), and that match keys
  are `d2b.mic` / `d2b.speaker` + `media.class`. Do not put
  the rule under `monitor.rules` or `monitor.alsa.rules` on the host
  — those match HARDWARE devices, not client streams.
- **`autostart = true` + `audio.enable = true` eval failure.**
  Intentional. Set one or the other. The sidecar lifecycle is
  bound to `d2b vm start <vm> --apply` from an operator session.
- **vhost-device-sound times out waiting for snd.sock.** Most
  often a PipeWire connect failure: the sidecar dials
  `/run/user/<uid>/pipewire-0` and gives up if the operator's
  Wayland session isn't live. Start the Wayland session first.
- **Mic granted but guest can't capture.** `d2b-<vm>-snd` must
  have the host `audio` group (set via `SupplementaryGroups`) and
  the per-VM stream-rule must NOT match (because mic is "on"). If
  the guest user can't open `/dev/snd/*`, check that the user is
  in `d2b.audio.users` (or `ssh.user`, which is the default).

## See also

- [Design / threat model](../explanation/design.md)
- [Provider capability matrix](./provider-capability-matrix.md) — per-provider
  console and audio capability boundaries (Cloud Hypervisor, qemu-media, ACA).
- [ADR 0041](../adr/0041-console-and-audio-controls.md) — binding design for
  provider-capability-aware console and audio.
- [Manifest schema](./manifest-schema.md) — `units.snd` field;
  `audioStateFile` path.
- [CLI contract](./cli-contract.md) — `d2b audio` subcommand.
- [`examples/graphics-workstation`](../../examples/graphics-workstation/) —
  enables audio alongside graphics + USBIP YubiKey.
