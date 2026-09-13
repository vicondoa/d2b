### Fixed

- A session is no longer torn down when a readiness notification arrives a
  moment before the data does. The socket layer returned an empty read as if
  the peer had closed, so under scheduling pressure a live session could
  report `session-disconnected` on both sides - and the local side closed the
  socket for real. An empty read now parks and waits for the peer's next
  record; only a genuine close (or a real socket error) reports a disconnect.
- A peer's first record on a named stream can no longer fail the whole
  session with `invalid-channel`. Named-stream registration is local, so a
  peer that writes immediately after its handshake could be routed before the
  receiving side had registered the stream; the affected endpoints now
  register their streams before the driver task that routes inbound records
  exists.
