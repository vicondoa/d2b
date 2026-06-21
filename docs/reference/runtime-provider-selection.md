# Runtime provider selection

**Diataxis category:** reference.

Runtime providers describe the local VM runner family used for a
workload. They are capability declarations and routing policy, not
escape hatches around `nixlingd` or `nixling-priv-broker`.

## Default provider

Normal local nixling VMs use the Cloud Hypervisor runtime provider:

| Field | Value |
| --- | --- |
| Provider id | `local-cloud-hypervisor` |
| Provider locality | Local host |
| Runner authority | `nixlingd` DAG plus broker `SpawnRunner` |
| Supported baseline | local lifecycle through the daemon, vsock, virtiofs store/share surfaces |

The provider adapter validates the existing typed Cloud Hypervisor input
shape and delegates argv generation to the existing host-side generator.
Runtime plans remain opaque and contain only bounded provider/workload
metadata. They never serialize argv, host paths, store paths, socket paths,
fd numbers, pidfds, cgroup paths, namespace identifiers, endpoint strings,
or process output.

## Unsupported provider ids

The following ids are reserved but are not production VM runtimes:

| Provider id | Behavior |
| --- | --- |
| `crosvm` | Refuses as unsupported. crosvm is currently used for selected sidecars, not as the full VM runtime. |
| `qemu` | Refuses as unsupported. General QEMU VM runtime semantics are not implemented. |
| `qemu-media` | Refuses as a full VM runtime. It remains the dedicated media sidecar runtime documented in [qemu-media](./qemu-media.md). |
| `firecracker` | Refuses as unsupported and, before any side effect, rejects workloads requiring guest-control, virtiofs/store sync, graphics, audio, USB, or desktop surfaces. |

Unknown provider ids fail closed with `UnsupportedFeature` and a remediation
that points operators back to `local-cloud-hypervisor`.

## Capability gating

Runtime provider selection is data-driven:

- callers route only to a provider whose advertised capabilities cover the
  requested workload;
- unsupported profiles never silently fall back to Cloud Hypervisor, SSH, a
  raw command tunnel, or qemu-media;
- a missing capability returns a typed refusal before any runner side effect;
- provider/debug/error metadata is limited to provider id, capability code,
  reason, and remediation.

Features that require graphics, audio, USB, virtiofs-backed storage,
guest-control, or local GPU acceleration should assume the Cloud Hypervisor
runtime unless a future provider explicitly advertises the needed capability.

## Related component requirements

- [Graphics](./components-graphics.md) depends on the Cloud Hypervisor VM
  runner plus the crosvm GPU sidecar.
- [Video](./components-video.md) uses a media sidecar and patched Cloud
  Hypervisor vhost-user-media support.
- [Audio](./components-audio.md) uses the Cloud Hypervisor VM runner plus a
  broker-spawned sound sidecar.
- [USBIP](./components-usbip.md) is a daemon/broker lifecycle surface, not a
  generic runtime fallback.
- [qemu-media](./qemu-media.md) is a separate media workflow and is not a
  general replacement for the Cloud Hypervisor VM runtime.
