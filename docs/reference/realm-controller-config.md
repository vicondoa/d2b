# Realm controller configuration

**Diataxis category:** reference.

`/etc/d2b/realm-controllers.json` is private, non-secret configuration for
host-local child realms. It projects the normalized realm index, generated
principals, allocator resource references, listener paths, and controller and
broker identities into the existing versioned bundle contract.

## Deterministic child identity

For a canonical child realm ID `<realm-id>`, the emitted identity is:

| Surface | Value |
| --- | --- |
| Controller user/group | `d2bd-r-<realm-id>` |
| Broker user/group | `d2bbr-r-<realm-id>` |
| Internal cgroup group | `d2bcg-r-<realm-id>` |
| Public access group | `d2b-r-<realm-id>` |
| Public listener | `/run/d2b/r/<realm-id>/public.sock` |
| Broker listener | `/run/d2b/r/<realm-id>/broker.sock` |

Controller and broker identities, listener rows, launch rows, and resource
references are separate. Rows are sorted by canonical realm path and exclude
the local-root instance and non-host-local realms.

## Declarative launch records

The allocator input contains one controller launch record and one broker launch
record per child realm. Each record names:

- its exact principal and internal supplementary group;
- its pre-bound public or broker listener reference;
- its direct `controller/` or `broker/` cgroup leaf;
- dedicated user, mount, network, IPC, PID, and cgroup namespace references;
- bounded resource references and the non-secret identity configuration path;
- the local-root broker as spawn authority and local-root controller as
  supervision owner.

The records explicitly deny self-binding and `SD_LISTEN_FDS`. They are
declarative: no process is started while evaluating or installing the bundle.

## Cgroup and ownership records

The generated cgroup tree is rooted at:

```text
/sys/fs/cgroup/d2b.slice/r-<realm-id>/
  controller/
  broker/
  workloads/
    w-<workload-id>/
      <role-id>/
```

The realm root, `workloads/`, and each workload root are process-free.
Controllers and brokers start directly in their role leaves; workload
processes may appear only in generated role leaves. Ownership rows keep the
public access group separate from the internal cgroup group and assign repair
authority to the declared broker boundary.

## Bundle projection

The frozen `realm-controllers.json` schema retains historical `serviceName`,
`socketUnitName`, and `serviceUnitName` field names. For child realms these
fields carry deterministic launch/listener record IDs, not PID1 unit names.
`materializedService` and `materializedSocket` are always `false`, and
`noSystemdUnitsMaterialized` is `true`.

The allocator block lists resource request IDs from `/etc/d2b/allocator.json`.
It is a resolver input, not a lease grant or host-mutation capability.

## Runtime boundary

Nix does not bind the child listeners, spawn either child, open pidfds,
allocate or execute leases, adopt a generation, or supervise a process. Those
operations belong to the local-root runtime. No per-realm or per-workload
systemd unit is emitted.
