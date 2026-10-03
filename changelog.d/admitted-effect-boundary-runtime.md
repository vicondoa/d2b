### Fixed

- The broker answers an admitted privileged effect on the same reactor that
  read its frame, instead of driving the boundary through a nested
  `block_on`. An `admittedEffect` frame is served as one of the accept loop's
  tasks, and `Runtime::block_on` refuses to block a thread that is already
  driving a runtime, so a broker that reached the boundary took the
  connection's task down with it: the caller was closed with no answer and
  no typed refusal, even when the effect was refused by name a moment later.
  The boundary is async end to end - the ledger awaits a lock, and a declared
  implementation runs as a `Send` future - so it is now awaited on the
  connection's own reactor, beside the frame read and write it answers with.
  The gate order, the refusals, and the bounded dispatch pool the ordinary
  request path bridges from its plain worker threads are unchanged, and the
  blocking call was removed rather than suppressed.
