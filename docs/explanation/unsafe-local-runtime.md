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

## Outbound helper connection

The daemon does not impersonate a user or guess a user D-Bus address. A
per-user helper connects outward to a daemon-owned socket. `SO_PEERCRED`
authenticates both the public requester and helper; the daemon routes an
operation only when the uids match exactly.

The helper queries the current systemd user-manager environment for each
operation and creates verified transient scopes. Graphical child setup removes
`DISPLAY` and replaces `WAYLAND_DISPLAY` with the proxy endpoint while
preserving `XDG_RUNTIME_DIR`. The environment posture is reported, but keys and
values are not returned or logged.

## Generic items, not application schemas

An exec item owns an id, name, icon, argv, and graphical flag. The model has no
Firefox-specific, browser-specific, or URL-specific fields. A shell item owns
presentation metadata and selects persistent-shell semantics. This keeps
provider routing common:

- local VM exec items use authenticated guest control;
- unsafe-local exec items use the same-uid helper;
- provider-managed runtimes advertise or refuse configured launch;
- shell items use the persistent-shell operation family.

Only item identity and presentation cross the public socket. Provider-private
argv is resolved from the integrity-pinned bundle.

## Restart and logout behavior

Scope identity, including systemd `InvocationID`, is the adoption authority.
Pids, process names, and broad cgroup sweeps are not. Ambiguous state is
preserved and reported degraded rather than killed.

d2b does not enable user lingering. Helper and daemon restarts are continuation
events while the user manager remains alive, but logout may end the scopes.
Status names this `user-manager-lifetime`.

