# Use console and audio controls

## Before you begin

- Join the host `d2b` group.
- Use the workload's canonical target from `d2b workload list`.
- Confirm that its audio provider reports `audio.open`,
  `audio.set-state`, and `audio.inspect`.
- Start the workload interactively. Realm audio does not support autostart.

## Inspect audio state

```text
d2b audio status <target>
```

The result reports microphone and speaker grants plus the applied enforcement
posture. A local Cloud Hypervisor workload normally reports
`host-and-guest`. Provider failures are returned per target and do not hide
successful results for other targets.

## Change microphone access

Grant access:

```text
d2b audio mic on <target>
```

Revoke access:

```text
d2b audio mic off <target>
```

Confirm the result with `d2b audio status <target>`.

## Change speaker access

```text
d2b audio speaker on <target>
d2b audio speaker off <target>
```

To revoke both directions:

```text
d2b audio off <target>
```

The provider updates bounded realm-local state under an OFD lock, updates
guest enforcement where available, and leaves the host PipeWire boundary
fail-closed if either step fails.

## Troubleshoot

**No soundcard in the guest**

Confirm that the workload has an audio provider and a Cloud Hypervisor runtime,
then cycle the workload after granting at least one direction.

**The audio process cannot connect to PipeWire**

Confirm that the host audio session is active. The bundle intentionally does
not contain a host `/run/user/<uid>` path; the allocator must resolve the active
session and lease its single PipeWire endpoint to the realm broker.

**Host playback changes when a workload starts**

Check that the d2b rules are in PipeWire `client.conf.d` and match
`d2b.mic` or `d2b.speaker` plus `media.class`. Do not place client stream rules
under WirePlumber hardware monitor sections.

## Related references

- [Realm audio resources](../reference/components-audio.md)
- [Provider capability matrix](../reference/provider-capability-matrix.md)
- [CLI contract](../reference/cli-contract.md)
