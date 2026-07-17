# Why unsafe-local is a provider

**Diataxis category:** explanation.

Some desktop actions are processes on the host rather than guests or remote
provider workloads. Treating each application as a special case would duplicate
launcher, status, shell, Wayland, and clipboard behavior. d2b instead models the
host-user runtime as an explicit provider and models applications as generic
configured launcher items.

## Explicitly unsafe

The name `unsafe-local` is a security statement. A process runs with the normal
permissions of the authenticated host user. A transient user systemd scope
gives d2b an exact lifecycle handle, but it does not isolate the process from
other processes of the same uid. Likewise, a Wayland proxy can add realm
identity rails and mediate d2b clipboard attribution, but a malicious same-uid
process can use the user's ordinary compositor access.

This is why the realm policy defaults to deny, status carries a closed
`unsafe-local` posture, and desktop clients must display rather than soften the
warning.

## Authenticated per-user runtime

The user manager owns the fixed `d2b-runtime-systemd-user.socket`. Its service
accepts only the frozen `runtime-systemd-user` ComponentSession purpose and
`d2b.runtime.systemd-user.v2` service package. The authenticated Unix peer uid,
service-process uid, and current uid must be the same non-root uid. No request
field can select an execution identity, and no host process impersonates a user
or guesses a user D-Bus address.

Each operation is bound to the negotiated session generation, authenticated
realm/workload scope, absolute deadline, idempotency key, and private
configuration digest. Configured argv is resolved behind the service boundary;
it is never supplied by a public request. The old helper generation hello,
heartbeat, JSON frames, and helper socket are not compatibility paths.

The runtime queries the current systemd user-manager environment for each
operation and creates verified transient scopes. Malformed or oversized manager
environments fail closed. Child setup clears inherited environment and copies
the complete manager environment. Environment keys and values are not returned,
logged, or included in diagnostics.

## Graphical readiness and clipboard authority

Graphical launch requires an authenticated handle from the frozen Wayland
control service. Child setup removes `DISPLAY` and installs only the
service-issued `WAYLAND_DISPLAY`. If Wayland control cannot open a display, the
runtime fails the operation; it never retries against the compositor or starts a
host-shell wrapper. A failed process start closes the newly issued display
handle.

The clipboard bridge attributes an unsafe-local endpoint from that canonical
target and provider identity. Window app ids and titles remain presentation
metadata and cannot authorize a transfer. Host, VM, and unsafe-local offers are
discovery-only until the picker sends `Select`; `d2b-clipd` remains the sole
component that publishes selected data or fulfills a transfer file descriptor.

## Generic items, not application schemas

An exec item owns an id, name, icon, argv, and graphical flag. The model has no
Firefox-specific, browser-specific, or URL-specific fields. A shell item owns
presentation metadata and selects persistent-shell semantics. This keeps
provider routing common:

- local VM exec items use authenticated guest control;
- unsafe-local exec items use the same-uid systemd user runtime;
- provider-managed runtimes advertise or refuse configured launch;
- shell items use the persistent-shell operation family.

Only item identity and presentation cross the public socket. Provider-private
argv is resolved from the integrity-pinned bundle.

## Restart and logout behavior

Scope identity, including systemd `InvocationID` and the exact scope cgroup
leaf, is the adoption authority. Pids, process names, and broad cgroup sweeps are
not. Ambiguous state is preserved and reported degraded rather than killed.

d2b does not enable user lingering. Runtime-agent and controller reconnects are
continuation events while the user manager remains alive, but logout may end
the scopes. Status names this `user-manager-lifetime`.
