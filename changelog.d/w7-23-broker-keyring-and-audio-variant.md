### Changed

- The USB-audit serial HMAC keyring builds as one synchronous job on the
  bounded loader probe seat instead of on the caller's executor. The
  0o700 root-owned directory prepare, the `O_NOFOLLOW | O_CLOEXEC` key
  reads, the dir-fd `openat` create with its 0o400 stamp, and the file
  and directory fsyncs all block, and a USB bind reaches the keyring
  from an async handler, so the syscall chain could park a runtime
  worker. Every check is unchanged: the same descriptor-level root-only
  validation, the same directory posture, and the same create-then-read-
  back order. A saturated seat refuses the call instead of growing
  threads.
- `AudioLastSetApplied::OfflineOnly` is renamed
  `AudioLastSetApplied::NotApplied`, and the wire-visible
  `resource.lastSetApplied` label the wayland-policy projection
  publishes is `NotApplied` with it. The variant means "no setting was
  applied in the current reconcile", which its documentation said and
  its old name did not; the projection, the wayland-policy and daemon
  test pins, and the provider ADR's enum row all move together.
