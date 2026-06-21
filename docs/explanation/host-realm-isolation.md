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
