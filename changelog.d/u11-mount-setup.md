## broker: make Volume presentation effective under every local namespace posture

A requested mount is no longer a field a launch can populate and ignore. The
broker now carries a declared presentation realization per launch
(`PresentationRealization`), and the two KTD11 capabilities are implemented
side by side rather than approximated by one:

- `FilesystemPresentation` prepares a private mount tree FIRST and only then
  creates the requested user namespace. The admitted source is opened through
  an anchored, whole-path `RESOLVE_NO_SYMLINKS` descriptor traversal and bind-
  mounted from that descriptor at its admitted destination, inside a fresh
  `tmpfs` the child mounts on the launch's private execution root. A read-only
  relationship mounts read-only; a writable one is confined to the admitted
  view. A destination outside the private execution root is refused, so a
  presentation is never established in the host root.
- `NamespaceFirstServiceSource` keeps ADR 0021's user-namespace-first,
  zero-host-capability launch, realizes the admitted source inside the service's
  own verified sandbox, and REFUSES a Process filesystem-mount request it
  cannot realize. Skipping a requested mount is no longer a reachable success.

The child signals "my user namespace now exists" through the existing bounded
setup handshake before exec, so the parent's `uid_map`/`gid_map` writes land on
a namespace that is really there, and a child that cannot prepare its mount
tree ends the launch with the mount exit code instead of proceeding.

The launch argv is fenced: a worker addresses its admitted destination or a
declared inherited descriptor, never an independently computed host source
path.

`RunnerIsolationSpec::presentation` is not optional: every launch states the
realization it is launched under. The `spawn-process` kernel DERIVES it from
the launch row's own mount policy, so a row declaring anything the broker must
realize in a mount namespace (a mount namespace of its own, a read-only or
writable path, a device bind, a cross-domain bind, the read-only Nix closure,
or default device-node hiding) launches under `FilesystemPresentation` with
that policy actually applied, and a row declaring none of those keeps
`NamespaceFirstServiceSource` with nothing to skip. Naming the namespace-first
posture for a row that asks for a mount is what `presentation-requires-mount-
realization` refuses, so the derivation is the enforcement rather than a
hardcoded assumption.
