### Security

- A serving worker's served view root is now bounded by the broker's own
  verified bundle. The `--shared-dir=` value selects an arbitrary host path
  the launch argv names, and the grant opened it to the launched principal
  with `r-x` (`rwx` for a read-write attachment) plus search on the chain
  above it. A launch could therefore name any absolute host directory and
  have the broker grant it away. The bound is the set of shared-storage roots
  the bundle declares - every storage row's `path_template` and every
  store-view farm root, which is exactly what `resolve_volume_view_root` and
  the store-view branch compose a served root from - and the view root is
  proved against it with the same `strip_prefix` proof the runner-tree grant
  applies to its broker-owned root. A root under none of them is refused by
  name, not clamped.
- A runner's `XDG_RUNTIME_DIR` / `PIPEWIRE_RUNTIME_DIR` no longer selects an
  arbitrary host path for the audio, gpu, video, wayland-proxy and
  qemu-media session ACLs. The plan's environment is wire payload, so naming
  any absolute directory there had the broker apply an access ACL to it and
  `rwx` to the PipeWire, Wayland and Pulse sockets below it. The value is
  acted on only when it IS the host session runtime directory the verified
  bundle's `site.json` declares - the `/run/user/<uid>` projection
  `nixos-modules/site-json.nix` emits from the same `d2b.site.waylandUser`
  option the session wiring uses, and the one the GPU sidecar's Wayland
  socket is already resolved from. The wayland-proxy arm's expected runtime
  directory owner is that declaration's uid as well, instead of a uid parsed
  back out of the payload string.
- A bundle that projects no Wayland session, and a site that declares none,
  leave the session directory unbound: a launch whose environment names one
  is refused by name rather than granted a directory the broker invented.
  This follows the fail-closed semantics of the swtpm identity code and the
  `vm-run-dir-socket-grant` posture fix. A launch whose environment names no
  session directory has no session grant to bound and is unaffected.
- `WAYLAND_DISPLAY` can no longer discard the session runtime directory it
  is joined onto. `Path::join` replaces its base whenever the argument is
  absolute, so an absolute `WAYLAND_DISPLAY` composed a socket path outside
  the directory the grant had just proved, and the `rwx` socket grant landed
  there. The composed socket must now be the session directory plus exactly
  one normal component; an escaping absolute path, a `..`, a nested
  subdirectory and an empty value are all refused, and both payload-named
  paths are proved before the first mutation so a refusal leaves no
  half-applied traverse grant behind. A bare display name - what every
  well-formed compositor launch uses - and an absolute spelling that lands
  back inside the session directory are unaffected.
