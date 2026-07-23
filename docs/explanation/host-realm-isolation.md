# Host and realm isolation

**Diataxis category:** explanation.

The host remains the local lifecycle authority. It listens on local Unix
sockets, authenticates local callers with `SO_PEERCRED`, and starts local
gateway VMs. It does not hold realm relay/provider credentials and does not
open realm relay sessions.

Gateway-backed realms move remote reachability into a gateway guest. The
gateway guest owns the sealed credential envelope, unseals Relay/provider
credentials, and opens the transport sessions for that realm. This keeps
relay identity from becoming local host authorization and keeps work and
personal realms separated by topology instead of by host-side conditionals.

Remote-management transports follow the same trust boundary: they run from a
gateway guest or a separately reviewed guest-owned design, never as a
host-side realm relay exception.

The host is not a global realm-policy singleton. Gateway guests own
remote/provider realm policy evaluation, while local host authorization remains
`SO_PEERCRED` plus the canonical `d2b` lifecycle group.

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
