# Realm controller boundaries

A host-local realm is an authority boundary, not a label on the local-root
daemon. Sharing a controller user, broker user, cgroup group, or public socket
group would let a compromise cross that boundary through ordinary Unix DAC.

d2b therefore separates four capabilities:

- the controller owns realm state and supervises workload DAGs;
- the broker owns privileged realm mutation and audit state;
- a private group permits only the controller and broker to operate within the
  delegated cgroup partition;
- a public group permits selected local users to connect to one realm.

The public group cannot mutate cgroups or broker state. The controller cannot
act as the broker, and the broker does not become a public client. Existing host
groups are applied as socket ACLs rather than nested into framework groups.

Local-root remains intentionally different. Its fixed PID1 endpoints retain the
existing `d2bd`, `root`, and `d2b` identities. This preserves the
`SO_PEERCRED` plus `d2b` lifecycle admission rule while ensuring that membership
in the local-root group does not silently authorize any child realm.

The principal modules emit declarations and ownership records only. The
allocator remains responsible for pre-binding child listeners, assigning
resources, and spawning the separate processes.
