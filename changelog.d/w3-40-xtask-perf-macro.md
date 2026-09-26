### Changed

- xtask commands resolve the repository root once per process instead of
  re-scanning environment variables and parent directories on every call, and
  schema generation mutates each schema document in place instead of cloning
  the full document before serializing.
- xtask redaction and proto generation no longer allocate per-line or
  per-field strings while scanning build logs and proto sources, and the
  test-only crash-hook guard is defined once instead of three times.