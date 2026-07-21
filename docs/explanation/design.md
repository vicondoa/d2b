# Realm-native design

d2b is an opinionated NixOS framework for desktop workloads in microVMs. The
host is trusted; workloads are grouped into realms that own their controller,
broker, provider set, network boundary, storage, process DAGs, and audit state.

## Control plane

PID1 owns only four unsuffixed local-root units:

- `d2bd.socket`
- `d2bd.service`
- `d2b-priv-broker.socket`
- `d2b-priv-broker.service`

The local-root allocator pre-binds public and broker listeners for each child
host-local realm. It parent-spawns one controller and one broker into their
declared cgroup leaves and returns pidfds for supervision. Child realm
processes are not systemd units and do not receive socket-activation
descriptors.

Each realm controller supervises its workload DAGs. There are no per-realm or
per-workload systemd templates. A normal local-root controller restart is a
continuation event: the controller proves child identity and adopts fresh
pidfds before considering cleanup.

## Declarative model

A consumer declares:

```nix
d2b.realms.work = {
  placement = "host-local";
  allowedUsers = [ "alice" ];
  network = {
    mode = "declared";
    lanSubnet = "10.20.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };
  providers.runtime = {
    type = "runtime";
    implementationId = "cloud-hypervisor";
  };
  workloads.desktop = {
    provider = "runtime";
    config = { ... }: {
      networking.hostName = "desktop";
    };
  };
};
```

Evaluation derives bounded canonical realm, workload, provider, role, and
resource identifiers. Human names remain presentation metadata. Generated
runtime paths and ownership records use canonical identifiers.

## Provider and privilege boundaries

Runtime, network, storage, device, display, audio, observability, and other
authorities are explicit provider axes. Provider registry rows bind canonical
provider identities to opaque intent IDs in the integrity-pinned private
bundle. Provider IDs authorize nothing by themselves.

Controllers are unprivileged. Realm-confined host mutations go through that
realm's broker. Host-global allocation remains a closed operation of the
local-root broker. A single host-global broker carrying realm tags would not
provide the required authority boundary.

The local lifecycle authorization surface is `SO_PEERCRED` plus membership in
the `d2b` group. Relay or provider identity never becomes local administrator
authority.

## Network isolation

A declared realm network owns its bridge, TAP, address, route, firewall, and
allocator rows. The host has no address on a workload LAN. Workload TAPs are
isolated unless the realm explicitly opts into east-west communication.
Foreign nftables, NetworkManager, and hosts-file state is preserved through
ownership markers and fail-closed coexistence checks.

Separate trust domains use separate realms and bridges. Realm relay credentials
and remote registries belong in a dedicated gateway workload, never in the host
daemon or broker.

## Storage and restart safety

Persistent and boot-scoped paths are generated from normalized resource rows.
The broker resolves opaque storage IDs through anchored, fd-relative path
walking. Locks use OFD semantics with `O_CLOEXEC`, an explicit total order, and
explicit descriptor transfer only.

Workload store views contain only the declared closure. Hardlink farms require
the d2b state root and `/nix/store` to share a filesystem. Ambiguous ownership
after restart degrades or quarantines the affected realm or workload; daemon
ledgers are diagnostics, never repair authority.

TPM state is identity-bound and persistent. Missing or replaced previously
provisioned state fails closed instead of silently creating a new device
identity.

## Mediated desktop resources

Graphics, Wayland, audio, video, TPM, USB, and security-key access is expressed
as provider/resource/role rows. The broker passes only declared descriptors and
device allowlists to dedicated role principals. Workload processes never gain
ambient access to the host compositor, PipeWire socket, device tree, or another
realm's resources.

## Bundle boundary

Bundle version 12 retains schema version `v2`. Private artifacts under
`/etc/d2b` are installed `root:d2bd` mode `0640` and covered by bundle
integrity hashes. Public-safe launcher metadata is argv-free and served through
the authorized daemon API. Configured argv remains only in private workload
intent.

The public version-7 workload manifest remains a frozen compatibility contract.
Its dynamic keys are canonical workload IDs and its path fields point into the
realm-native layout. New control-plane authority comes from the private bundle,
not from the compatibility projection.

## Threat model

d2b assumes one trusted human and one trusted NixOS host. It reduces accidental
and workload-originated access to host resources; it is not a defense against a
compromised host kernel, compositor, root account, or malicious local user with
the same host identity. Secret material and command output must not enter public
metadata, telemetry labels, logs, or bundle diagnostics.
