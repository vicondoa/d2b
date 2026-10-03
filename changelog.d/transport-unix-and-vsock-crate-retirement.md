### Removed

- Removed the `Provider/transport-unix` transport entirely. Nothing ever
  served it: the daemon skipped a committed Unix-transport ZoneLink in both
  composition passes, so the crate, its Provider matrix row, its ZoneLink
  settings assertions, its execution-reference authority, its dossier, and its
  binding schema all promised a transport with no serving path. The closed
  Provider matrix is now 26 rows. Test fixtures and shipped doc comments that
  used the retired identity as an example now name a live transport.

### Changed

- `ProcessRole::VsockRelay` no longer names `Provider/transport-vsock` as its
  owning Provider. The relay is a guest `socat` process the process Provider
  serves, so the role was claiming a transport Provider that never owned it.
