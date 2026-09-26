### Fixed

- Console drainer tasks for VM console sessions now run on the daemon's own
  tokio runtime instead of a second runtime started from library code that was
  never shut down.
- Public list/status polling no longer copies the whole cached read-model
  frame on every request; the cached frame is shared by reference and copied
  only when the response is actually served.
- Mutating-verb responses are matched against the typed outcome vocabulary
  instead of raw wire strings, so unknown outcomes fail closed instead of
  being treated as a known state.