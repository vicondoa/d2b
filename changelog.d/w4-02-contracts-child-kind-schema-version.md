### Fixed

- `BindingChildRequest::process` and `process_for_user` now take the restricted `ProcessChildKind`, so an Endpoint can no longer be passed to a process constructor and rejected at runtime.
- `SchemaVersion` now exposes `major()` and `minor()` component accessors, and state-schema admission reads them directly instead of re-parsing the canonical version string.