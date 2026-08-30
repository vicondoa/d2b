# Provider capability matrix

**Diataxis category:** reference.

The current accepted runtime is the local Zone/Guest path. Capability
advertisement is not acceptance evidence; a Provider must still pass its
owner-local contract tests and the applicable host lane.

| Provider class | Current role | U19/U20 status |
| --- | --- | --- |
| Cloud Hypervisor Guest | Local Guest controller, Process/Endpoint/Volume children, authenticated Guest session | U20 host and KVM acceptance |
| QEMU media Guest | Optional local Provider projection | Layer-1 contract only in U19 |
| Device, audio, display, shell, volume, network Providers | Zone-scoped specialized effects | Layer-1 contract and owner-local tests |
| Azure Container Apps sandbox | Optional remote Provider-managed sandbox | Deferred until after U20; no ACA acceptance claim |

## Local Guest contract

Local Providers are addressed with typed Zone ResourceRefs and are fenced by
Guest UID, Provider generation, controller generation, and revision. They do
not expose host paths, credentials, argv, or broker handles.

```bash
d2b guest status <name> --zone <zone>
d2b provider status <name> --zone <zone>
d2b host doctor --read-only
```

## ACA deferral

The ACA adapter may retain implementation and contract documentation, but its
upstream lifecycle, exec, display, audio, and isolation behavior has not been
validated by U19 or U20. Do not use an ACA sandbox to satisfy local Guest
acceptance, and do not describe the adapter as a drop-in replacement for
Cloud Hypervisor.

See [provider-managed sandboxes](./provider-managed-sandboxes.md),
[the compatibility policy](./compatibility.md), and
[the daemon lifecycle](../explanation/daemon-lifecycle.md).
