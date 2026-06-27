# Host substrate providers

**Diataxis category:** reference.

Host substrate providers report whether a host can run d2b workloads.
They do not mutate host state. Installation, package deployment, and
network preparation remain explicit operator or broker-mediated actions.
The underlying read-only probe model is described in
[host prepare](../explanation/host-prepare.md); the advertised capability
model is the same positive-assertion model documented in
[constellation core](./constellation-core.md#capabilities).

## Current adapters

| Provider | Scope | Behavior |
| --- | --- | --- |
| `nixos-host-substrate` | NixOS hosts | Wraps the existing host-check report and advertises node capabilities only when the report has zero failures. |
| `generic-linux-host-substrate` | Generic Linux/Ubuntu hosts | Uses the same host-check report contract as a dry-run capability surface. |

Both adapters are backed by the existing `d2b-core::host_check`
findings:

- failures become typed provider allocation errors that include the first
  finding id and remediation text;
- zero-failure reports advertise the local lifecycle, vsock, and virtiofs
  substrate capabilities;
- debug output includes only provider id and strictness, never host
  topology, paths, command output, or probe details.

Future host substrate work can add prepare/install providers, but they must
remain separate from these dry-run checks and must route privileged
mutations through typed broker operations.
