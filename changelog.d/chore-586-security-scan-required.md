### Security

- The identifier-in-log redaction scan (the ADR 0010/0028 discipline: no
  opaque correlation identifier written into a log) now runs on every
  pull request as the `security-scan` job in `pr-l1-static-fast.yml`, is
  wired into the aggregate `check` job's needs list, and its context is a
  required status check on `main` and `v3`, so a finding on changed lines
  blocks the merge path instead of passing a green pipeline.