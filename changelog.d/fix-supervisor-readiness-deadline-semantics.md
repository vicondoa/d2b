# `fix-supervisor-readiness-deadline-semantics.md`

### Fixed

- The provider supervisor no longer drops a hung effect's deadline wake when
  the bounded deadline queue is momentarily full: the deferred registration
  now rides the job and is re-attempted from the dedicated job worker once
  the deadline worker drains, so a hung effect stays bounded by its deadline
  under a production executor instead of being awaited past it forever. The
  re-attempt sender is only ever held by the job closure when a registration
  was actually deferred, so a wedged worker closure cannot keep the deadline
  channel open and stall pool teardown.
- A readiness probe that never produced a result - the blocking pool was
  saturated or the call overran the ticket deadline - now surfaces as the
  transient `launch-failed` code the driver retries, instead of
  `deadline-exceeded`, which the minijail consumer treats as the terminal
  "probe ran and found no candidate" verdict and quarantines. A busy probe
  never observed the process, so a healthy live identity is no longer
  quarantined under startup load; a probe that does run and finds no
  candidate still yields the genuine absent-candidate result.