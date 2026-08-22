### Changed

- Kept the fixed Layer-1 PR and push gate owned by protected `v3` while making
  every GitHub Actions Bazel action local and credential-free. Developer
  BuildBuddy profiles and the immutable cache/OID contract remain available
  for a future non-Actions Workflows trial.

### Security

- Preserved trusted/bootstrap checkouts, immutable PR and run metadata
  validation, PR/head cache namespaces, and the trusted control overlay while
  removing BuildBuddy secrets and remote profiles from GitHub Actions. The
  facade now rejects non-local profiles when invoked by Actions, so a workflow
  change cannot silently restore credential-bearing remote execution.
