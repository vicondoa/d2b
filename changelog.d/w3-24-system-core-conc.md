### Fixed

- The system-core test-support scripted discovery port counts calls with an
  atomic instead of a tokio mutex, so a contended read can no longer silently
  report zero and the crate no longer depends on tokio's sync feature.
- The single-threaded future driver used by the hermetic system-core tests
  now asserts when a scripted future yields, failing fast instead of spinning
  forever at 100% CPU on a noop waker.