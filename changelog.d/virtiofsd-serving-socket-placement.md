### Fixed

- A binding-owned virtiofsd worker can be launched again. Its private
  socket was derived straight into the broker's own runtime root
  (`/run/d2b/vfd-<tag>.sock`), and the broker's serving-worker ACL grant
  refuses exactly that placement: it opens the socket's PARENT directory to
  the worker principal, and it refuses a parent that IS the runtime root
  rather than grant `rwx` on a tree that also holds `priv.sock` and
  `public.sock`. Every virtiofsd launch therefore died fail-closed with
  `handler-refused` and the detail `serving worker private socket directory
  /run/d2b is outside the broker runtime directory /run/d2b`, the worker row
  stayed `Pending`, and the `Endpoint` that waits for the bind reported
  `virtiofsd socket not bound within its realize budget`.
  `d2b_provider_volume_virtiofs::derive_serving_socket_path` derives the
  frozen worker socket contract instead -
  `<runtime_root>/vms/<guest>/vol-<sha256(zone 0 volume 0 guest)[..8]>.vfd.sock`
  - the per-Guest runtime tree the verified storage contract declares as
  `path:vm-run:<guest>` for every virtiofs serving target, alongside the
  Device workers' sockets. The daemon reads that row's declared mode and
  realizes the directory before the launch ticket carries it, so the tree
  the broker grants on is the tree the contract postures.
- The daemon's serving socket effect surface no longer keeps its own copy
  of the derivation. `resource_plane_v3::serving_socket_path` and the serving
  launch composition in `process_provider_runtime` both go through the
  Provider's one function, so the `--socket-path` a worker binds and the path
  the `Endpoint`/`VolumeBinding` legs probe, present, and remove cannot be
  two different paths.