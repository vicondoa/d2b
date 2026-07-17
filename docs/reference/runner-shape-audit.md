# Runner-shape audit

## Scope and method

This audit is the Cloud Hypervisor/minijail spike artifact. It does
not boot a VM: this worktree has no KVM-capable host, root, or live
network. Instead it evaluates the existing `examples/minimal` host and
builds the microvm.nix runner derivation, then inspects the generated
shell scripts.

This document is historical runner-shape evidence for the retired
microvm.nix runner. It is not the current daemon-owned virtiofsd parity
oracle. Current virtiofsd argv and share semantics are documented in
[`store-virtiofs.md`](./store-virtiofs.md) and emitted by
`nixos-modules/processes-json.nix`.

`examples/minimal` is otherwise unchanged, but today its headless VM
still defaults to microvm.nix's non-Cloud-Hypervisor backend. The
portability plan selects Cloud Hypervisor as the first daemon backend, so
all snapshots add one test-only module:

```nix
({ lib, ... }: {
  microvm.vms.corp-vm.config.microvm.hypervisor =
    lib.mkForce "cloud-hypervisor";
})
```

Generation commands:

```bash
# Sanity: root flake minimal eval gate.
nix --no-warn-dirty eval --raw \
  .#checks.x86_64-linux.eval-minimal.drvPath

# Build the Cloud Hypervisor declaredRunner used by this audit.
nix --no-warn-dirty build --impure --no-link --print-out-paths --expr \
  "$(tests/runner-shape-snapshot.sh runner_expr)"

# Regenerate committed fixtures.
bash tests/runner-shape-snapshot.sh declared-runner \
  > tests/golden/runner-shape/examples-minimal-declaredRunner.txt
bash tests/runner-shape-snapshot.sh cloud-hypervisor-argv \
  > tests/golden/runner-shape/cloud-hypervisor-argv-minimal.txt
```

The inspected runner path was:

```text
/nix/store/hjlgdb9zka7h6mgq8x63hdyiqba4mii0-microvm-cloud-hypervisor-corp-vm
```

## microvm.nix `declaredRunner` shape

For the audited headless `examples/minimal` VM (`corp-vm`) with Cloud
Hypervisor forced, `config.microvm.vms.corp-vm.config.config.microvm.declaredRunner`
resolves to the runner above. Its layout is:

- `bin/microvm-run` — shell wrapper that removes stale CH/notify
  sockets, optionally starts a `socat` systemd-notify relay, sets
  `runtime_args=`, and `exec`s Cloud Hypervisor.
- `bin/virtiofsd-run` — shell wrapper that execs supervisord:
  `/nix/store/qal8sp237c6rdxljvm9k1i2xsnl1wz3n-python3.13-supervisor-4.3.0/bin/supervisord --configuration /nix/store/7mfww0f3h8z4m83dvgqyqwbgydr69bf6-corp-vm-virtiofsd-supervisord.conf "$@"`.
- `bin/tap-up` — deletes any stale `work-l10`, creates a TAP as user
  `microvm` with `vnet_hdr`, then brings it up.
- `bin/tap-down` — deletes `work-l10`.
- `share/microvm/hypervisor` — `cloud-hypervisor`.
- `share/microvm/tap-interfaces` — `work-l10`.
- `share/microvm/tap-flags` — `vnet_hdr`.
- `share/microvm/vsock-cid` — `10914385`.

The virtiofsd supervisord config at
`/nix/store/7mfww0f3h8z4m83dvgqyqwbgydr69bf6-corp-vm-virtiofsd-supervisord.conf`
contains one event listener and four virtiofsd programs:

- `virtiofsd-ro-store`: `--socket-path=corp-vm-virtiofs-ro-store.sock`,
  `--socket-group=kvm`, `--shared-dir=/nix/store`.
- `virtiofsd-d2b-meta`: `--shared-dir=/var/lib/d2b/r/<realm-id>/w/<workload-id>/store-view/meta`.
- `virtiofsd-d2b-hkeys`: `--shared-dir=/var/lib/d2b/r/<realm-id>/w/<workload-id>/keys/host`.
- `virtiofsd-d2b-ssh-host`: `--shared-dir=/var/lib/d2b/r/<realm-id>/w/<workload-id>/keys/sshd`.

Each virtiofsd wrapper conditionally adds `--rlimit-nofile 1048576` when
running as uid 0, then passes `--thread-pool-size \`nproc\``,
`--posix-acl --xattr`, `--cache=auto`, and
`--inode-file-handles=prefer`.

The actual Cloud Hypervisor exec line in `bin/microvm-run:24` is
snapshotted in `tests/golden/runner-shape/cloud-hypervisor-argv-minimal.txt`.
At this revision it is:

```bash
exec -a "microvm@corp-vm" /nix/store/5dp5ya1q03ab3indxnd7x3pwixifw5rn-cloud-hypervisor-52.0/bin/cloud-hypervisor --cpus 'boot=1' --watchdog --kernel /nix/store/6p1aazl39927kp22ajw4h8bqa6j5g4vz-linux-6.18.31-dev/vmlinux --initramfs /nix/store/qdrg2rycwnqw7b5m69v12pizvf3p19yr-initrd-linux-6.18.31/initrd --cmdline 'earlyprintk=ttyS0 console=ttyS0 reboot=t panic=-1 8250.nr_uarts=1 root=fstab loglevel=4 lsm=landlock,yama,bpf init=/nix/store/5ycspc2h3zhl9qiq2axsc1hvirr5pm02-nixos-system-corp-vm-26.05pre-git/init regInfo=/nix/store/ldfmwp9xh6av69d5bvz7j898m6kqlgzm-closure-info/registration' --seccomp true --memory 'shared=on,size=512M' --platform 'oem_strings=[io.systemd.credential:vmm.notify_socket=vsock-stream:2:8888]' --console null --serial tty --vsock 'cid=10914385,socket=notify.vsock' --fs 'socket=corp-vm-virtiofs-ro-store.sock,tag=ro-store' 'socket=corp-vm-virtiofs-d2b-meta.sock,tag=d2b-meta' 'socket=corp-vm-virtiofs-d2b-hkeys.sock,tag=d2b-hkeys' 'socket=corp-vm-virtiofs-d2b-ssh-host.sock,tag=d2b-ssh-host' --api-socket corp-vm.sock --net 'mac=02:76:53:AE:57:0A,tap=work-l10'   ${runtime_args:-}
```

## Cloud Hypervisor argv inventory

The headless runner emits these flags today:

| Flag | Current value | Source / note |
| --- | --- | --- |
| `exec -a` | `microvm@corp-vm` | microvm.nix runner process name. |
| binary | `cloud-hypervisor-52.0/bin/cloud-hypervisor` | `microvm.cloud-hypervisor.package`; graphics VMs override this to the vendored Spectrum CH package. |
| `--cpus` | `boot=1` | microvm CPU defaults. |
| `--watchdog` | present | microvm.nix CH runner default. |
| `--kernel` | guest `vmlinux` store path | guest NixOS kernel. |
| `--initramfs` | guest initrd store path | guest NixOS initrd. |
| `--cmdline` | earlyprintk/console/reboot/root/loglevel/LSM/init/regInfo | microvm.nix boot command line plus guest system and closure registration paths. |
| `--seccomp` | `true` | CH runner default. |
| `--memory` | `shared=on,size=512M` | microvm memory default; shared memory is required by virtiofs. |
| `--platform` | `oem_strings=[io.systemd.credential:vmm.notify_socket=vsock-stream:2:8888]` | systemd notify compatibility over vsock. |
| `--console` | `null` | headless VM console policy. |
| `--serial` | `tty` | headless serial console policy. |
| `--vsock` | `cid=10914385,socket=notify.vsock` | `microvm.vsock.cid`; host.nix gives non-observability VMs a fallback CID for CH notify. Observability VMs append their own `microvm.cloud-hypervisor.extraArgs` `--vsock socket=...`. |
| `--fs` | four sockets/tags: `ro-store`, `d2b-meta`, `d2b-hkeys`, `d2b-ssh-host` | `microvm.shares`; d2b's store and host-key modules materialize these virtiofs shares. |
| `--api-socket` | `corp-vm.sock` | CH runner default. The daemon keeps API sockets always enabled but daemon-only. |
| `--net` | allocator-derived MAC and TAP id | The normalized realm network row for the canonical workload id. |
| `${runtime_args:-}` | empty for headless minimal | Extension point used by audio/graphics shapes; empty in this audit. |

Not present for the audited headless VM, but present when features enable
them: `--gpu` from graphics, `--tpm` from `microvm.cloud-hypervisor.extraArgs`
in the TPM component, `--vhost-user-media` from the video component, audio
`--user-device`/generic-vhost-user arguments through `runtime_args`, and
additional explicit `microvm.cloud-hypervisor.extraArgs`.

## Sidecar process tree under microvm.nix today

- Headless minimal: `microvm@corp-vm` runs `bin/microvm-run`; the
  separate `microvm-virtiofsd@corp-vm` path runs `bin/virtiofsd-run`,
  which supervises four virtiofsd workers. `tap-up`/`tap-down` are
  lifecycle helpers rather than long-lived sidecars. A `socat` notify
  relay is forked only when `NOTIFY_SOCKET` is set.
- TPM-enabled VMs: d2b's TPM component adds a per-VM swtpm service
  and passes the CH TPM socket via `microvm.cloud-hypervisor.extraArgs`.
- Graphics workstation: the graphics CH runner starts `crosvm device gpu`
  inline, then CH connects to it with `--gpu socket=...`. D2b also
  has host-side video and audio sidecars for the graphics/audio example;
  video contributes `--vhost-user-media`, and audio contributes runtime
  CH args through the generated audio args script.
- Observability-enabled VMs: host-side observability relay/listener roles
  are separate d2b services; the guest CH argv gets a vsock socket
  from `microvm.cloud-hypervisor.extraArgs`.

## Inline-crosvm-gpu spawn note

The graphics runner inspected at
`/nix/store/k0w9y5n5l1wnykpax09fhswkgv6rpgwb-microvm-cloud-hypervisor-corp-desktop/bin/microvm-run`
forks `crosvm device gpu` before the CH `exec`:

- `bin/microvm-run:18`: removes `corp-desktop-gpu.sock`.
- `bin/microvm-run:19-23`: runs
  `/nix/store/rfw2rn9875py1l34wfr45wnlkphbgj5n-crosvm/bin/crosvm device gpu \
  --socket corp-desktop-gpu.sock --wayland-sock $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY \
  --params '{"context-types":"virgl:virgl2:cross-domain","displays":[{"hidden":true}],"egl":true,"vulkan":true}' &`.
- `bin/microvm-run:24-26`: waits for the GPU socket.
- `bin/microvm-run:34`: execs CH with `--gpu 'socket=corp-desktop-gpu.sock'`.

That inline fork is the central blocker for using declaredRunner as
the daemon payload: the daemon cannot assign role-specific uid/gid,
capability, cgroup, mount, and seccomp/minijail policy to CH and GPU
separately when one shell script forks both.

## Runner-shape options

### A. Wrap declaredRunner as a black-box shell in one union minijail profile

Rejected. A union profile would have to include the combined privileges
of CH, virtiofsd readiness, crosvm GPU, audio/video helpers, swtpm paths,
TAP handling, notify relay, and future observability roles. That violates
the requirements for per-role minijail profiles, per-role uid/capability
sets, cgroup leaves, readiness predicates, and runtime oracles.

### B. Generate d2b-owned CH argv from evaluated microvm/d2b config

Preferred. D2b should evaluate the existing Nix module graph, then
serialize enough runner data for `d2bd` to launch each role itself.
Inputs are:

- `microvm.interfaces` for MAC/TAP/network argv;
- `microvm.shares` for virtiofs sockets/tags and virtiofsd roles;
- `microvm.vsock` and `microvm.cloud-hypervisor.extraArgs` for notify,
  observability, TPM, video/audio, and other backend-specific flags;
- realm workload/provider options and allocator-derived resources, graphics,
  audio, video, TPM, USBIP, audit, observability, state roots, and
  lifecycle policy;
- the manifest bundle (`bundle.json`, `host.json`, `processes.json`,
  `privileges.json`) as the stable daemon input.

The parity oracle is: for headless VMs, d2b-generated CH argv must
match the declaredRunner argv snapshotted here except for explicitly
documented divergences. Known expected divergences are daemon-owned API
socket placement/permissions, daemon-owned vsock CID allocation, and any
TAP fd-passing shape selected by the TAP ADR.

### C. Patch microvm.nix's CH runner to skip inline crosvm-gpu spawn

Deferred. This remains a fallback if option B hits unforeseen complexity.
It would reduce the graphics blocker but still leaves d2b with
a shell-runner ABI and less direct control over role supervision.

## Decision

Choose option B: generate d2b-owned Cloud Hypervisor argv from the
evaluated microvm/d2b config and keep `declaredRunner` only as a
parity oracle during the transition.

## Parity oracle contract

`tests/golden/runner-shape/` contains:

- `examples-minimal-declaredRunner.txt`: the built declaredRunner store
  path for `examples/minimal` plus the CH-forcing module.
- `cloud-hypervisor-argv-minimal.txt`: the exact `exec -a ...
  cloud-hypervisor ...` line extracted from that runner's
  `bin/microvm-run`.

`tests/runner-shape-snapshot.sh` regenerates both fixtures by evaluating
and building the same expression, then diffs committed goldens. A changed
runner store path or CH argv is a failure, not a skip. The test skips
only when the example flake cannot be evaluated or built at all in the
current environment, and logs that as a TODO-style skip.

A future microvm.nix, nixpkgs, component, or d2b option change that
alters runner shape must update this audit, explain the intended drift,
and refresh the fixtures in the same commit. The CH argv fixture will be
lifted into a hard build-time parity gate by comparing daemon-generated
argv against declaredRunner argv for headless VMs before allowing the
new supervisor path to replace the shell runner.
