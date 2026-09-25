### Fixed

- Session record decryption reuses a scratch plaintext buffer and moves the
  payload out instead of allocating and copying on every received record.
- The outbound flush path moves each dequeued frame's bytes out of the
  scheduler instead of copying them.
- Session replay detection now uses a hash set over the replay window instead
  of a linear scan of every cached digest.