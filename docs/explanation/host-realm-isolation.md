# Host and realm isolation

**Diataxis category:** explanation.

The local root listens on fixed local Unix sockets and authenticates local
callers with `SO_PEERCRED`. Child host-local realms run under separate
controllers; provider-backed behavior runs through typed provider agents. The
host daemon and broker do not hold realm Relay or provider credentials.

Credential providers and their consumers are co-located in the same exact
provider-agent placement. Only opaque credential leases cross the provider
trait boundary. Relay remains reachability, never host authorization.

The host is not a global realm-policy singleton. Each controller owns its realm
decisions, while local host authorization remains `SO_PEERCRED` plus the
canonical `d2b` lifecycle group.

## Local host-resource arbitration

The host remains the only place that can arbitrate local kernel and
filesystem resources shared by multiple local realm controllers. The
local-root allocator contract defines how future realm brokers request
typed leases for bridges, taps, nftables partitions, cgroup subtrees,
host-file partitions, and namespace boundaries without becoming
independent host mutators. See
[Local-root allocator contract](../reference/local-root-allocator.md)
for the reference shape.

That contract is a foundation, not a live allocator service in the
current implementation. Today it does not change runtime host mutation.
The important design boundary is already fixed: realm brokers must not
work around the allocator with ad-hoc locks, host-file edits, or
realm-specific repair paths. Ambiguous or foreign host state is
quarantined or denied until an explicit owner reconciles or reclaims it.
