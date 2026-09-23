### Security

- The identifier-in-log redaction scan (the ADR 0010/0028 discipline: no
  opaque correlation identifier written into a log) now runs on every
  pull request to `v3` as the `security-scan` job in
  `pr-l1-static-fast.yml`, is wired into the aggregate `check` job's
  needs list, and its context is a required status check on `main` and
  `v3`, so a finding on changed lines blocks the merge path instead of
  passing a green pipeline.
- The deterministic rule evaluates the whole log-macro invocation
  (buffered from the opening line to its closing `);`), so a multi-line
  record that references a pinned identifier on a continuation line is a
  finding. The four in-tree records this surfaced (broker
  `OwnershipMatrixCheck` refusal, credential revocation confirm/unconfirm,
  gateway route admission denial) were redacted at source: the opaque
  identifier no longer appears in the log record, matching the ADR
  0010/0028 redaction discipline.
- The changed-line scan diffs the merge base against the working tree
  (uncommitted lines are visible locally), keeps rename/copy statuses in
  the diff, rejects an all-zeros base explicitly, and exits 2 when the
  diff itself fails - a scan that could not run is never reported clean.