# Realm audio resources

Realm audio gives a local Cloud Hypervisor workload a virtio-snd device backed
by one `vhost-device-sound --backend pipewire` process. The owning realm
controller supervises the process by pidfd. No per-workload systemd unit is
created.

## Declarative rows

For each enabled audio workload, `realm-audio-rows.nix` emits:

- one audio role process, placed directly in the workload's role cgroup leaf;
- one vhost-user Unix endpoint under
  `/run/d2b/r/<realm-id>/w/<workload-id>/sockets/audio.sock`;
- bounded audio policy state under
  `/var/lib/d2b/r/<realm-id>/w/<workload-id>/audio/audio-state.json`;
- one boot-scoped OFD lock and one process-scoped mediation directory; and
- one lease for the active host audio session's PipeWire endpoint.

The workload must explicitly bind same-realm, host-local `runtime` and `audio`
providers using the `cloud-hypervisor` and `pipewire-vhost-user`
implementations. Emission fails closed when either normalized binding is
missing or disagrees with its provider.

Every realm, workload, and role path component is a canonical short ID.
Human-configured realm and workload names do not enter these rows.

The state file is capped at 128 bytes and is atomically replaced under its OFD
lock. Missing or malformed state resolves to microphone and speaker both off.
The configured initial grants apply only when the broker first creates the
state file.

## PipeWire boundary

The bundle records an opaque `active-host-audio-session` lease, not an ambient
`/run/user/<uid>` path, user ID, display name, or host socket. At runtime the
allocator resolves the active session, and the realm broker exposes only its
`pipewire-0` endpoint inside the audio role's private mediation directory. The
role cannot see the ambient runtime directory.

The process receives `PIPEWIRE_RUNTIME_DIR` and `XDG_RUNTIME_DIR` pointing at
that private directory. It publishes only canonical workload-ID labels. The
audio provider resolves `d2b.mic` and `d2b.speaker` from the bounded state row;
invalid state is fail-closed.

Host PipeWire stream rules preserve direction isolation:

- a microphone stream with `d2b.mic = "off"` is routed to `-1`;
- a playback stream with `d2b.speaker = "off"` is routed to `-1`; and
- an explicitly configured input target is applied only when the microphone
  grant is on.

The rules belong in PipeWire `client.conf.d`, not WirePlumber hardware monitor
rules.

## Provider fragment

The audio provider fragment uses implementation `pipewire-vhost-user`, is
placed in the owning realm controller, and advertises only:

- `audio.open`
- `audio.set-state`
- `audio.inspect`
- `audio.adopt`
- `audio.close`

Its binding contains opaque process, endpoint, storage, and lease IDs. It does
not contain host PipeWire paths.

## Guest configuration

`components/audio/guest.nix` enables `snd_virtio`, PipeWire ALSA/Pulse
compatibility, rtkit, and the `pro-audio` WirePlumber profile. Users listed in
`d2b.audio.users` join the guest `audio` group. The realm process composer owns
the Cloud Hypervisor vhost-user attachment; the guest module does not discover
or poll host paths.

## Lifecycle invariants

- Audio requires a Cloud Hypervisor role and interactive workload start.
- The audio process starts after its PipeWire lease and before Cloud
  Hypervisor.
- The vhost-user process restarts only as part of a workload cycle.
- State and locks survive controller restart; the process and mediation
  directory are adopted or removed only with live-owner proof.
- Ambient host endpoints never appear in bundle metadata.
