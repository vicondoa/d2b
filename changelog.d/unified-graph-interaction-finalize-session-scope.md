### Fixed

- A `DisplayService/Finalize` request no longer tears down every session
  registered with the Zone's interaction composition. The request was
  dispatching the daemon's whole-runtime shutdown routine
  (`finalize_async`, whose contract is "finalize all runtimes and explicitly
  revoke every registered ingress"), so one service asking to be finalized
  revoked the display, clipboard, picker, and notification sessions alike.
  Any peer that pipelined a command behind its Finalize on a multiplexed
  connection lost that command: its session was revoked mid-flight and the
  driver failed the unanswered command with `session-disconnected`. The
  teardown is now scoped to the session that sent the request through the
  composition's own per-session removal path, so a second session's in-flight
  work survives another session's Finalize and the display runtime plus its
  dependents are released only when the finalizing session was the last
  display session. Daemon shutdown keeps its whole-Zone scope.
- The `DisplayService/Finalize` teardown no longer rests on a flush it never
  performed. The response frame is on the transport before that branch runs:
  `send_component_response` awaits the session writer, which reports
  completion only after the batch carrying the frame has been written, and the
  release then takes the registrar's writer-acknowledged revocation. The
  one-millisecond delay that sat in front of it was never that ordering - it
  was a grace for the peer, because ending the session closes its socket and a
  peer's driver ends its loop on that EOF. The peer losing the response it had
  already been sent is fixed in the session driver itself, in
  `changelog.d/plan-unified-resource-graph.md`; the delay stays at one
  millisecond and is now documented as what it is rather than as a flush.
  Separately, the interaction tests no longer assert the peer's post-close
  bookkeeping for the one request whose contract is to end the session, because
  that call races the operation under test; those call sites assert the
  deterministic outcome instead, that the response was delivered and that
  session is gone.
