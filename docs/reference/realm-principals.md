# Realm principals

Each enabled host-local child realm receives four identities derived only from
its canonical 20-character realm ID:

| Purpose | Name |
| --- | --- |
| Controller system user and primary group | `d2bd-r-<realm-id>` |
| Broker system user and primary group | `d2bbr-r-<realm-id>` |
| Internal cgroup delegation group | `d2bcg-r-<realm-id>` |
| Public socket access group | `d2b-r-<realm-id>` |

The controller and broker are distinct system users. Their only shared
supplementary group is the internal cgroup group. Neither service identity is a
member of the public group. The public group grants connection access, not
cgroup or filesystem mutation authority.

The local-root instance is the sole exception. Its controller remains `d2bd`,
its broker remains `root`, and its public socket group remains `d2b`. It has no
realm-derived internal cgroup group.

## Local admission

`d2b.site.launcherUsers` receive only the `d2b` local-root lifecycle group.
Users listed by a child realm, plus site administrators, receive that realm's
public group. Membership in `d2b` does not implicitly grant a child realm, and
child-realm access does not grant local-root lifecycle authority. Existing
groups listed by a realm are represented as explicit public-socket ACL entries;
groups are never nested.

## Socket principals

The allocator pre-binds child sockets and passes listener file descriptors:

- `/run/d2b/r/<realm-id>/public.sock` is owned by the controller and grouped by
  the public access group.
- `/run/d2b/r/<realm-id>/broker.sock` is owned by the broker and grouped by the
  controller's primary group.

Both sockets use mode `0660`. The internal cgroup group never owns the public
socket, and the public group never owns the broker socket.

## Resource ownership

Declarative access rows assign the controller its state, controller, and cache
trees; assign the broker its broker and audit trees; and assign the internal
group the realm runtime root. Public principals receive only search access to
that root and connection access to the public socket. Dynamic paths and socket
nodes remain allocator or broker creations; these rows do not create or repair
them.
