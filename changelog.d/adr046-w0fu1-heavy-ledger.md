### Changed

- The hermetic runtime-ledger gate now warm-builds before timing, collects
  repeated execution-only samples at test and crate granularity, and enforces
  a complete, comparable census: a repetition floor, non-empty scopes,
  matching per-sample repetition counts, and detection of census ids dropped
  from a run. Its cargo invocations run from the workspace directory so
  the configured compiler wrapper is discovered.
