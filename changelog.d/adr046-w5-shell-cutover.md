### Changed

- Cut `d2b shell open`, `attach`, `list`, `status`, `detach`, and `kill` over
  qualified ShellSession Resource requests and authenticated named streams,
  including PTY I/O, resize forwarding, signal-safe detachment, restart
  recovery, and removal of the retired public shell socket protocol.
