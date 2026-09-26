### Changed

- d2b-broker: the USBIP busid lock error taxonomy no longer flattens
  `io::Error` into a string. `UsbipLockError::Io` now carries the source
  `io::Error` (`#[source]`), and a lock path with no parent directory is
  reported as its own `PathSafetyViolation` variant. The rendered refusal
  text is unchanged, so the broker error envelope is byte-identical.
- d2b-broker: `acquire_lock` drops its unused `daemon_uid` parameter; the
  lock record keeps using the broker's own uid and the daemon gid.
- d2b-broker: `guest_socket_directory` returns a typed `GuestSocketError`
  instead of a `&'static str` code, with the same stable refusal strings
  ("not-a-plain-name", "not-anchored", "outside-runtime-root") surfaced
  through the launch-failure envelope.