# TPM device resource

Declare `tpm` in a realm workload's capability list and configure one
`host-mediated` device provider in that host-local realm:

```nix
d2b.realms.work = {
  path = "work.local-root";
  providers.runtime = {
    type = "runtime";
    implementationId = "cloud-hypervisor";
  };
  providers.devices = {
    type = "device";
    implementationId = "host-mediated";
  };
  workloads.desktop = {
    provider = "runtime";
    launcher.capabilities = [ "tpm" ];
  };
};
```

## Contract

Evaluation emits a `tpm` device row with:

- canonical realm, workload, provider, and `swtpm` role short IDs;
- capability `tpm2-stateful`;
- an exclusive `device-tpm-<workload-id>` allocator lease;
- state resource ID `workload/<workload-id>/tpm`;
- FD-only, realm-local broker mediation.

The control socket is:

```text
/run/d2b/r/<realm-id>/w/<workload-id>/roles/<swtpm-role-id>/tpm.sock
```

Cloud Hypervisor receives that canonical socket. The guest sees a TPM CRB
device and provisions the standard storage root key.

## Persistence and recovery

TPM state is workload identity. Never delete, replace, or copy it between
workloads: doing so can look like device tampering and invalidate bound
credentials. Provisioning and recovery remain broker-owned and fail closed
when previously provisioned state is missing or has the wrong identity.

The physical storage location is resolved from the opaque state resource ID.
Callers must not construct it from the realm label, workload label, TPM
contents, or host device metadata.
