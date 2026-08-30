# Architecture conventions

This page records the current ownership and composition rules. The binding
summary is in [`../../AGENTS.md`](../../AGENTS.md).

## Control plane

d2b declares exactly three root-visible units:

```text
d2bd.service
d2b-broker.socket
d2b-broker.service
```

`d2bd` supervises Zone runtimes and Resource lifecycle. `d2b-broker` owns
audited host mutation, runner launch, pidfd handoff, and child reaping.
Framework-owned per-Guest systemd units and host-singleton lifecycle services
are not allowed.

## Resource ownership

Nix authors:

- Zone topology and labels;
- semantic Guest, Provider, Network, Device, Volume, Credential, and Process
  inputs; and
- immutable system and Provider artifacts.

The Cloud Hypervisor Guest controller authors and reconciles its direct child
Resource graph. Specialized controllers remain the effect owners for Process,
Endpoint, Volume, Network, Device, Credential, session, and Provider work.
No controller bypasses the Resource API to spawn, mount, bind, or provision.

ResourceRefs are deterministic `Type/name` addresses. UIDs, generations,
revisions, and Provider generations fence adoption, update, stop, and delete.
Private runtime identity is derived from immutable Zone and Guest identity,
never from a name alone.

## Composition and sibling projects

The d2b repository owns the Zone control plane and host-facing Providers.
Consumer-owned Guest evaluators and external identity modules are composed at
`d2b.guestSystems.<zone>.<guest>`. Desktop companions consume only the public
CLI and daemon contracts.

```nix
d2b.guestSystems.work.work-app = {
  config.system.build.toplevel = inputs.workGuestSystem;
};
```

Do not add identity, application, or desktop-specific authority to d2b merely
because a consumer needs it. Add a Provider or sibling module only when the
contract is generic, typed, and owned by the current Zone resource plane.

## Host effects

Every host-mutable path, lock, cgroup leaf, device, socket, and ownership
marker has one named repair owner. Foreign markers fail closed. The daemon
does not sweep runtime directories, rewrite foreign nftables or
NetworkManager state, or accept caller-supplied host paths.

Runner launch uses typed broker operations and signed Provider templates.
Signals and reaping use pidfds; a persisted PID/start-time pair is only an
adoption check, never a public identity.

## Adding behavior

New behavior must choose the lowest owner that can enforce it:

1. semantic input in a Resource contract;
2. a specialized controller effect;
3. a typed broker operation for privileged host mutation; or
4. a consumer-owned Guest module or sibling flake.

Do not add a second scheduler, a per-Guest service, a static lifecycle
manifest, or a compatibility fallback. Add owner-local tests and generated
contract updates with the same change.

## References

- [`../../AGENTS.md`](../../AGENTS.md)
- [`../explanation/daemon-lifecycle.md`](../explanation/daemon-lifecycle.md)
- [`../reference/zone-control-nix.md`](../reference/zone-control-nix.md)
- [`../reference/zone-cli-contract.md`](../reference/zone-cli-contract.md)
- [`../reference/privileges.md`](../reference/privileges.md)
