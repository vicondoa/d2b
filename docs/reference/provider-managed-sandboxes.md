# Provider-managed sandboxes

**Diataxis category:** reference.

A provider-managed sandbox is a workload whose lifecycle is owned by a cloud
provider rather than a host-local `d2b-priv-broker`. The canonical d2b 2.0
model represents that lifecycle with a runtime provider. Provider calls use the
same bounded provider DTOs whether the implementation is trusted in-process or
served by an authenticated provider agent.

Azure Container Apps (ACA) is the implemented provider-managed runtime. It is
implemented by `d2b-provider-runtime-azure-container-apps`; Azure Relay is a
separate transport provider implemented by
`d2b-provider-transport-azure-relay`.

## Public configuration boundary

`d2b.gateways` and its nested gateway/ACA sandbox fields are removed. A
non-empty legacy declaration is rejected at evaluation with a migration error.
No supported Nix option auto-declares a gateway guest, installs a gateway
credential envelope, or turns nested ACA coordinates into a live provider.

Current realm declarations record provider intent under
`d2b.realms.<realm>.providers`:

```nix
d2b.realms.work = {
  placement = "provider-agent";
  placementProvider = "aca";

  providers.aca = {
    kind = "aca";
    placement = "provider-agent";
    capabilityRefs = [ "runtime" ];
    configRef = "work-aca";
  };
};
```

These fields are non-secret metadata. In the current Nix schema they do not
instantiate an ACA provider agent or enroll credentials. Keep provider
coordinates and secrets out of Nix until the provider-agent composition and
enrollment surface is available.

## Provider placement and credentials

ACA runtime providers must use canonical `provider-agent` placement. The
runtime and its cloud credential provider must be co-located in the exact same
authenticated provider-agent placement. Credential acquisition returns only an
opaque, operation-scoped `CredentialLease`; credential bytes, environment
variables, files, and file descriptors cannot cross the provider contract.

The ACA provider has no ambient Azure CLI, environment-variable, broker,
daemon, endpoint, command, or legacy-provider fallback. Its provider-specific
SDK behavior is supplied behind an agent-local control port.

Azure Relay follows the same placement rule. Endpoint coordinates and
credentials remain behind the injected, co-located relay control port.
Connect and listen use distinct opaque credential leases. Relay establishes
reachability only; relay authentication never grants d2b authorization.

## ACA runtime capabilities

The ACA runtime descriptor must advertise exactly the methods implemented by
the canonical provider:

| Method | Behavior |
| --- | --- |
| `runtime.plan` | Produces a bounded plan for the configured ACA workload. |
| `runtime.ensure` | Ensures the configured disk image and sandbox exist. |
| `runtime.start` | Starts or resumes the bound sandbox. |
| `runtime.stop` | Stops the bound sandbox idempotently. |
| `runtime.inspect` | Returns bounded lifecycle observation. |
| `runtime.adopt` | Adopts only an exact, unambiguous provider resource binding. |
| `runtime.destroy` | Destroys the bound sandbox through the provider control port. |

The provider does not expose broker operations, raw SDK payloads, arbitrary
exec, SSH, raw guest-control, vsock, file copy, generic tunnels, device
hotplug, host cgroup authority, or host filesystem authority. Those are not
inferred from provider-managed isolation and do not gain fallbacks.

Display, audio, console, persistent shell, storage, network, device, and
observability are separate provider authorities. They require their own
registered provider and positive capabilities; the ACA runtime provider does
not acquire them by implication.

## Azure Relay transport capabilities

The canonical Azure Relay transport provider can positively advertise:

- `transport.connect`;
- `transport.listen`;
- `transport.revoke-binding`;
- `transport.inspect`, including adoption through its control port.

It consumes an already-configured opaque transport binding and rendezvous id.
It does not accept endpoint URLs, credential material, or free-form transport
configuration through operation inputs.

## Availability

The host production provider registry currently activates mapped local runtime
providers and local observability providers. ACA runtime and Azure Relay are
constructed only by the generic provider-agent composer with agent-local
control ports and co-located credential providers; the current public Nix
surface does not compose or launch that agent.

Therefore realm provider metadata is not a deployment recipe. Do not recreate
the removed gateway options, hand-write `/etc/d2b/gateway.json`, or run the
legacy gateway enrollment helpers as a substitute. Until a supported
provider-agent composition surface is available, an ACA/Relay declaration is
planning metadata and must not be presented as an operational deployment.

## Diagnostics and failure posture

Provider errors use closed provider failure kinds, bounded remediation, retry
classes, and redacted diagnostics. Logs, errors, audit, and `Debug` output must
not expose:

- provider, resource, workload, lease, or operation identifiers;
- endpoints, subscription or resource-group values, image references, or
  provider payloads;
- credentials, bearer tokens, SDK responses, command bytes, stdout, or stderr;
- host paths, agent socket paths, or environment values.

Descriptor, placement, capability, configuration-digest, and credential
co-location mismatches fail closed before provider operations are admitted.
Provider ambiguity is quarantined; it is never repaired by selecting an
unconfigured fallback.

## Related references

- [Provider contract v2](./provider-contract-v2.md)
- [d2b 2.0 provider implementations](./v2-provider-implementations.md)
- [Realm option schema](./realm-options.md)
- [Azure Relay transport](./transport-azure-relay.md)
- [Provider capability matrix](./provider-capability-matrix.md)
- [Remote full-host nodes](./remote-full-host-nodes.md)
