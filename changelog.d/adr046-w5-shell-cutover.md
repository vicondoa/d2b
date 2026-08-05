### Changed

- Cut `d2b shell open`, `attach`, `list`, `status`, `detach`, and `kill` over
  qualified ShellSession Resource requests and authenticated named streams,
  including partial-write-safe PTY I/O, resize and EOF forwarding, JSON
  create-without-attach, multi-target restart recovery, signal-safe
  detachment, and removal of the retired public shell socket protocol.
