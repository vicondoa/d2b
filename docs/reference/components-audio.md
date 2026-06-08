# `nixling.vms.<vm>.audio.*`

> Reference for the `audio` component module.
> Source (host): [`nixos-modules/components/audio/host.nix`](../../nixos-modules/components/audio/host.nix)
> Source (guest): [`nixos-modules/components/audio/guest.nix`](../../nixos-modules/components/audio/guest.nix)
> CLI: `packages/nixling/src/lib.rs` (`nixling audio …`); pre-P6 bash CLI in `nixos-modules/cli.nix` was retired in P6 per ADR 0015

## What this component does

Gives a VM a virtio-snd soundcard backed by a per-VM
`vhost-device-sound` sidecar that connects to the host's PipeWire
session. On the host the sidecar appears as a PipeWire client named
`nixling-<vm>` (visible in `wpctl status`, plasma-pa, pavucontrol);
inside the guest, normal PipeWire + ALSA + PulseAudio compat stacks
work on top of the virtio-snd card.

Each VM has independent **mic** and **speaker** grants. State of
record was (pre-P6) a per-VM JSON file at
`/var/lib/nixling/vms/<vm>/state/audio-state.json`; the pre-P6
`nixling audio` bash CLI subcommand mutated it under flock. The
sidecar publishes the resolved mic/speaker state as custom
PipeWire properties (`nixling.mic`, `nixling.speaker`); a
host-side `client.conf.d` rule reads those properties and
null-routes the corresponding stream direction with
`target.object = "-1"` when it's `off`. Setting `audio.enable = true`
only enables the *capability* — in v1.0 (per ADR 0015) the
`nixling audio` Rust CLI verbs are rust-native shims that return
typed `not-yet-implemented` exit-78 envelopes; the daemon-native
audio control plane (state-file owner, flock holder, sidecar
respawn coordinator) is queued for v1.1+.
both directions default to `off` on first materialisation unless the
`allow{Mic,Speaker}ByDefault` options are flipped.

When both directions are off, the guest-side
`microvm.extraArgsScript` short-circuits and does **not** emit
`--generic-vhost-user` — the guest sees no soundcard at all.

## Options (host-side)

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.vms.<vm>.audio.enable` | bool | `false` | Enable per-VM audio capability. Incompatible with `autostart = true` (asserted at eval). Implies `hypervisor = cloud-hypervisor`. |
| `nixling.vms.<vm>.audio.allowMicByDefault` | bool | `false` | Initial value of the `mic` field when the per-VM state file is first materialised. Consulted at creation time only; subsequent edits via `nixling audio …` persisted pre-P6 (the v1.0 Rust CLI shim returns exit-78 per ADR 0015 until the daemon-native audio control plane lands in v1.1+). |
| `nixling.vms.<vm>.audio.allowSpeakerByDefault` | bool | `false` | Initial value of the `speaker` field when the per-VM state file is first materialised. Same edit-survives semantics (pre-P6; daemon-native control deferred to v1.1+). |
| `nixling.vms.<vm>.audio.users` | list of str | `[ ]` (defaults to `[ ssh.user ]` if non-null) | Guest-side usernames added to the `audio` group inside the VM. virtio-snd exposes `/dev/snd/*` as `0660 root:audio`; non-logind-active users need explicit group membership. |

Site-level dependency:

| Option | Type | Required when | Description |
|---|---|---|---|
| `nixling.site.waylandUser` | nullable str | any VM has `audio.enable = true` | The host's primary Wayland session user. Its `/run/user/<uid>/pipewire-0` socket is bind-mounted into the sidecar's private mount namespace. Eval fails clearly if unset. |

## Options (guest-side propagation)

`host.nix` propagates `audio.users` into the guest with a `mkIf`
gate (falls back to `[ ssh.user ]` if the per-VM list is empty):

```nix
(lib.mkIf vm'.audio.enable {
  nixling.audio.users =
    if vm'.audio.users != [ ]
    then vm'.audio.users
    else lib.optional (vm'.ssh.user != null) vm'.ssh.user;
})
```

The matching guest-visible option (declared in
`components/audio/guest.nix`):

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.audio.users` | list of str | `[ ]` | Guest-side usernames added to the `audio` group. Each named user gets `users.users.<u>.extraGroups += [ "audio" ]`. |

## Host-side resources created

> **v1.0 status (per [ADR 0015](../adr/0015-daemon-only-clean-break.md)):**
> the pre-P6 `nixling-<vm>-snd.service` systemd unit was retired in P6
> and respawned by the broker's `SpawnRunner` DAG under the
> `nixling.slice/<vm>/snd` cgroup leaf. The hardening shape (uid,
> caps, SupplementaryGroups, etc.) documented below is preserved
> as the minijail-profile contract the broker enforces on the
> runner spawn — the difference is the supervisor (broker pidfd
> table instead of systemd's service manager). The bullets below
> use the historical systemd unit identifier for traceability.
>
> The `nixling audio mic|speaker|status` CLI verbs in v1.0 are
> rust-native shims that return a typed `not-yet-implemented`
> envelope (exit 78 per ADR 0015); the daemon-native audio
> control plane is queued for v1.1+. The pre-P6 bash audio
> orchestration in `cli.nix` was retired in P6.

Per audio-enabled VM:

- **`nixling-<vm>-snd` system user + group**
  ([`host-users.nix`](../../nixos-modules/host-users.nix)).
- **`nixling.slice/<vm>/snd` runner** (pre-P6 `nixling-<vm>-snd.service`)
  ([`components/audio/host.nix`](../../nixos-modules/components/audio/host.nix))
  — vhost-user-sound sidecar, broker-spawned via the v1.0 daemon
  DAG (pre-P6 it was a systemd system service). Runs as
  `nixling-<vm>-snd:nixling-<vm>-snd`, `SupplementaryGroups = [ "audio" ]`.
  Started on demand by the v1.0 audio control plane (queued for
  v1.1+; pre-P6 it was spawned by `nixling audio mic|speaker on <vm>`
  through the bash CLI's audioArgsScript).
  belt-and-suspenders fallback at VM boot). Started *and* ordered
  before `nixling-<vm>-gpu.service` via the latter's `wants`/`after`.
  - `RuntimeDirectory = "nixling/vms/<vm>"`, mode 0700.
  - `BindPaths` binds `/run/user/<wayland-uid>/pipewire-0` into the
    sidecar's private runtime dir; the sidecar never sees
    `/run/user/<uid>` itself.
  - `ExecStartPre`:
    1. `setfacl -m u:nixling-<vm>-snd:rw /run/user/<uid>/pipewire-0`.
    2. `install` a per-VM copy of `vhost-device-sound` at
       `/run/nixling/vms/<vm>/nixling-<vm>` (so libpipewire's
       `init_prgname()` derives `application.name = "nixling-<vm>"`
       from `/proc/self/exe` — argv[0]/symlink tricks don't work).
    3. Read `audio-state.json` and emit `PIPEWIRE_PROPS` into
       `/run/nixling/vms/<vm>/snd.env` with `application.name`,
       `node.name`, `node.description`, `nixling.mic`,
       `nixling.speaker`.
  - `ExecStart`: `/run/nixling/vms/<vm>/nixling-<vm> --socket
    /run/nixling/vms/<vm>/snd.sock --backend pipewire`.
  - `ExecStartPost`: polls for `snd.sock` up to 30 s, then
    `setfacl -m u:nixling-<vm>-gpu:x /run/nixling/vms/<vm>` and
    `setfacl -m u:nixling-<vm>-gpu:rw .../snd.sock`. Fails the unit
    hard if the socket never materialises.
  - `Restart = "no"` — on-demand only.
- **State file** `/var/lib/nixling/vms/<vm>/state/audio-state.json`
  (mode 0640, owner `root:nixling-launcher`), initial contents
  `{"mic":"<allowMic>","speaker":"<allowSpeaker>"}`. The containing
  `state/` directory is mode 0750 `root:nixling-launcher`. ACLs
  grant `nixling-<vm>-gpu` `rx` on the directory and `r` on the
  file so the GPU sidecar can read state at VM-boot time without
  joining `nixling-launcher`.
- **Lock file** `/run/nixling/audio-<vm>.lock`, mode 0660,
  `root:nixling-launchers`. The pre-P6 bash CLI took flock on it
  before mutating the state file; in v1.0 (per ADR 0015) the
  Rust CLI shim returns exit-78 and does NOT acquire this lock.
  The v1.1+ daemon-native audio control plane will reclaim flock
  ownership on the broker side.
- **vhost-device-sound v0.3.0** vendored at
  `pkgs/vhost-device-sound/` (nixpkgs ships v0.2.0 with a known
  PipeWire-backend format-negotiation bug). Added to
  `environment.systemPackages` for ad-hoc operator debugging.

Per any audio-enabled host (emitted when at least one VM has
`audio.enable = true`):

- **PipeWire `client.conf.d/90-nixling.conf` `stream.rules`** —
  matches `nixling.mic = "off"` + `media.class =
  "Stream/Input/Audio"` (capture block) and `nixling.speaker = "off"`
  + `media.class = "Stream/Output/Audio"` (playback block); applies
  `target.object = "-1"`, `node.dont-reconnect = true`,
  `node.dont-fallback = true`, `node.linger = true`. The block fires
  only when the named direction is `off`; the other direction
  continues to auto-route via WirePlumber's normal default-target
  hook.

CLI (`nixling audio` in the v1.0 Rust CLI — currently a rust-native
shim that returns typed `not-yet-implemented` exit-78 per ADR 0015;
daemon-native audio control queued for v1.1+. Pre-P6 bash CLI in
`cli.nix` was retired in P6):

- `nixling audio mic on|off <vm>`
- `nixling audio speaker on|off <vm>`
- `nixling audio off <vm>` — shorthand for both off.
- `nixling audio status` / `nixling audio status <vm>` — reports
  current grant state per VM.

## Lifecycle (v0.1.5+)

`nixling-<vm>-snd.service` carries `restartIfChanged = false`
(matches the [graphics sidecar lifecycle policy](./components-graphics.md#lifecycle-v015)).
A `nixos-rebuild switch` updates the unit file but does NOT cycle
the running `vhost-user-sound` sidecar — vhost-user-sound's socket
connection to cloud-hypervisor cannot survive a restart, and
killing this sidecar mid-VM produces silent speakers and mic
stuck in whatever state it was in. After a rebuild, `nixling
list` flags the VM with `[pending restart]` if its `current`
closure has drifted from `booted`; apply with `nixling vm restart
<vm>` (clean down+up cycles the audio sidecar and CH together so
the socket gets re-established). See
[`docs/reference/cli-contract.md` — Pending-restart signal](./cli-contract.md#pending-restart-signal-v015).

## Guest-side resources created

In [`components/audio/guest.nix`](../../nixos-modules/components/audio/guest.nix):

- `microvm.hypervisor = "cloud-hypervisor"` (via `mkDefault`).
- `microvm.extraArgsScript = audioArgsScript` — a shell helper
  invoked by microvm.nix's runner at VM start. Reads
  `audio-state.json`; if both directions are off, emits nothing
  (no virtio-snd device at all). Otherwise:
  1. `systemctl reset-failed nixling-<vm>-snd.service` (tolerant).
  2. `systemctl start nixling-<vm>-snd.service` (tolerant).
  3. Polls for `/run/nixling/vms/<vm>/snd.sock` up to 5 s.
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
  `nixling.audio.users`.
- WirePlumber drop-in `91-nixling-virtio-snd` under
  `monitor.alsa.rules` that matches the virtio-snd PCI card and
  pins `device.profile = "pro-audio"`, `api.alsa.use-acp = false`.
  Without that, WirePlumber leaves the card in "Off" mode (no Sink
  / Source created) because virtio-snd has no ACP entry.

## Runtime invariants

- The sidecar's `application.name` in plasma-pa / `wpctl status` is
  always `nixling-<vm>` (derived from `/proc/self/exe`). Per-stream
  mute/volume in plasma-pa applies to that client.
- When both directions are `off`, no virtio-snd device is attached
  to CH and no sidecar process exists. Setting either direction on
  starts the sidecar; setting both off again does NOT teardown the
  device on the running VM (would unplug the soundcard mid-flight)
  — the WirePlumber rule null-routes the streams instead.
- Audio-enabled VMs MUST have `autostart = false`. Eval-time
  assertion in `audio/host.nix` enforces this: `microvm@<vm>.service`
  would boot the VM without starting `nixling-<vm>-gpu.service`,
  leaving no CH process for the sidecar to hand a socket to.
- The state file always lives at
  `/var/lib/nixling/vms/<vm>/state/audio-state.json`. A one-time
  activation-script migration moves the legacy
  `/var/lib/nixling/vms/<vm>/audio-state.json` location if it
  exists.

## Hardening notes

`nixling-<vm>-snd.service` is the security baseline for
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
- `ReadWritePaths = [ "/run/nixling/vms/<vm>" ]`.
- `BindPaths` exposes only `pipewire-0`, not the parent runtime
  directory.
- The per-VM binary copy at `/run/nixling/vms/<vm>/nixling-<vm>`
  is owned `root:root 0755`; the sidecar (running as
  `nixling-<vm>-snd`) gets r-x via traditional perms, never w.

`audio-state.json`:

- Lives in a root-owned non-group-writable subdir `state/` (mode
  0750 `root:nixling-launcher`). The parent
  `/var/lib/nixling/vms/<vm>/` remains `microvm:kvm 2775` so the
  CLI can take the audio lock and write temp files there, but no
  kvm-group process can unlink/replace the state file.
- File mode 0640 `root:nixling-launcher`. Read by the GPU sidecar
  via an explicit named-user ACL grant for `nixling-<vm>-gpu`.

## Common gotchas / failure modes

- **VM silent / no soundcard in `aplay -l`.** Both mic and speaker
  are off — `nixling audio status <vm>` will confirm. Toggle one on
  and restart the VM (`nixling vm stop <vm> && nixling vm start <vm>`) so
  `audioArgsScript` re-emits `--generic-vhost-user`.
- **Audible static / dropped frames on the host's own playback
  while a nixling VM is running.** Almost always a WirePlumber
  rule misplacement re-introducing the USB-headset duplex-mode bug.
  Verify the `90-nixling.conf` rule is in PipeWire's `client.conf.d/`
  (NOT WirePlumber's `wireplumber.conf.d/`), and that match keys
  are `nixling.mic` / `nixling.speaker` + `media.class`. Do not put
  the rule under `monitor.rules` or `monitor.alsa.rules` on the host
  — those match HARDWARE devices, not client streams.
- **`autostart = true` + `audio.enable = true` eval failure.**
  Intentional. Set one or the other. The sidecar lifecycle is
  bound to `nixling vm start <vm>`, which `microvm@<vm>.service` doesn't
  trigger.
- **vhost-device-sound times out waiting for snd.sock.** Most
  often a PipeWire connect failure: the sidecar dials
  `/run/user/<uid>/pipewire-0` and gives up if the operator's
  Wayland session isn't live. Start the Wayland session first.
- **Mic granted but guest can't capture.** `nixling-<vm>-snd` must
  have the host `audio` group (set via `SupplementaryGroups`) and
  the per-VM stream-rule must NOT match (because mic is "on"). If
  the guest user can't open `/dev/snd/*`, check that the user is
  in `nixling.audio.users` (or `ssh.user`, which is the default).

## See also

- [Design / threat model](../explanation/design.md)
- [Manifest schema](./manifest-schema.md) — `units.snd` field;
  `audioStateFile` path.
- [CLI contract](./cli-contract.md) — `nixling audio` subcommand.
- [`examples/graphics-workstation`](../../examples/graphics-workstation/) —
  enables audio alongside graphics + USBIP YubiKey.
