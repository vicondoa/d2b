# Explanation: realm controller boundaries

> Diataxis: explanation. Conceptual model for the host-local realm controller
> metadata boundary. For the field-by-field contract, read
> [`docs/reference/realm-controller-config.md`](../reference/realm-controller-config.md).

Realm declarations now produce a private bundle artifact that describes how
host-local realm controllers are named and separated. NixOS materializes the
host-local control-plane scaffolding from that contract — principals, unit
names, socket paths, tmpfiles directories, and ACL metadata — while the
access/routing/identity runtime remains intentionally narrow.

## Why a separate controller artifact exists

The public `d2b.realms.<realm>` option tree is operator-facing. It is useful for
declaring intent, but it is not the shape a daemon, broker, or verifier should
consume at runtime. The controller artifact normalizes that intent into a small
set of deterministic facts:

- the realm id and realm path;
- reserved daemon and broker unit names;
- reserved public and broker socket paths;
- runtime, state, and audit directories;
- local users/groups intended for direct realm-socket access;
- local-root allocator metadata for host-resource requests.

Keeping those facts in one private artifact prevents later code from inventing
parallel naming rules or silently deriving different paths from the same realm.

## Direct socket access, not host byte proxying

The access model is direct realm socket authorization. A local user who
is allowed to administer a realm should connect to that realm's public Unix
socket, where `SO_PEERCRED` and socket ACLs can be checked against that realm's
policy.

The global host daemon remains the owner of the current local lifecycle socket.
It should not become a generic byte-forwarding proxy between arbitrary local
users and realm daemons. Byte proxying would blur accountability, make peer
credentials ambiguous, and encourage callers to bypass the realm's own
authorization boundary.

## Local-root resolution, not raw host mutation

Some resources are host-local by nature: cgroups, nftables partitions, host-file
partitions, interface names, and namespace boundaries. Realm brokers must not
create those resources independently. Instead, they use the local-root
allocator contract to request typed leases and receive opaque grants.

The controller artifact therefore records allocator metadata and per-realm
resource request ids. It does not turn the host daemon into a tunnel for raw
commands, raw paths, or nftables text.

## State and audit remain separate

Realm controller state and realm audit records have different jobs. State is
the controller's repairable working memory. Audit is the append-oriented
record of decisions and refusals. Mixing them would make it unclear which data
is operational authority and which data is evidence.

The metadata keeps that split visible by reserving separate state and audit
directories for each realm, plus an ephemeral runtime directory for locks and
sockets.

## Runtime boundary

Existing VM and env behavior remains in force. Defining a host-local realm
creates deterministic control-plane units, broker sockets, principals, tmpfiles
paths, and ACL entries for the realm daemon/broker scaffold. It still does not
enroll identities, start relays, advertise routes, place provider controllers,
allocate host resources through the allocator, or migrate workloads out of
`d2b.envs`. The discovery and strict tree routing contract is described in
[realm tree routing boundaries](./realm-tree-routing.md), but that contract is
metadata-only. Operators should not
treat it as live relay routing, VPN/overlay networking, SSH fallback, or any raw
tunnel facility.
