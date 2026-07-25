### Changed

- ADR 0046 spec set: aligned the illustrative examples across every resource and
  topology spec with the frozen datetime, universal-status, outcome, and
  ResourceType-name decisions so every persisted-datetime literal uses
  millisecond precision (`YYYY-MM-DDTHH:MM:SS.sssZ`), every universal envelope
  carries `status.resource` and `status.update`, retry scalars use the
  `retryAfterMs` shape, and vendor ResourceType names qualify with the
  `d2bus.org` grammar.
- ADR 0046 Host/Guest execution policy: froze a single `defaultUserRef`
  invariant across the decision register, terminology, Nix, and resource specs -
  `defaultUserRef` is required whenever `allowedDomains` contains `user`.
- ADR 0046 ZoneLink bootstrap: specified IKpsk2 with an allocator-issued
  single-use PSK for initial cross-Zone enrollment and KK for the enrolled
  session, reserving the unauthenticated NN profile for local peer-credential or
  inherited-descriptor sessions.
- ADR 0046 delivery: documented that the auto-release path cuts from the `v3`
  clean-break lineage, and aligned the release workflow and operating manual to
  trigger on `v3` instead of `main`.

### Fixed

- ADR 0046 spec docs: corrected stale current-state prose that still described
  the spec set as `Proposed` with no checked-in generator; the `spec-registry`
  and `implementation-graph` generators and their fail-closed drift gate exist
  and run.
- ADR 0046 topology: replaced the obsolete `W0`-`W10` program range with the
  binding `ADR046-W0`-`ADR046-W8` wave contract in the decision register and
  streamline spec.

### Security

- ADR 0046 release automation: the host-binary release workflow now fails closed
  when unfolded fragments remain under `changelog.d/`, so a release can no longer
  omit changelog entries or leak branch-named fragment filenames into a release
  artifact.
