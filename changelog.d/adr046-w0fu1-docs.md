### Changed

- ADR 0046 spec set: aligned the illustrative examples across every resource,
  topology, and Provider-dossier spec with the frozen datetime, universal-status,
  outcome, and ResourceType-name decisions so every persisted-datetime literal
  uses millisecond precision (`YYYY-MM-DDTHH:MM:SS.sssZ`), every universal
  envelope carries `status.resource` and `status.update`, retry scalars use the
  `retryAfterMs` shape, and vendor ResourceType names qualify with the
  `d2bus.org` grammar.
- ADR 0046 Host/Guest execution policy: froze a single `defaultUserRef`
  invariant across the decision register, terminology, Nix, and resource specs -
  `defaultUserRef` is required whenever `allowedDomains` contains `user`.
- ADR 0046 ZoneLink bootstrap: specified IKpsk2 with an allocator-issued
  single-use PSK for initial cross-Zone enrollment and KK for the enrolled
  session, reserving the unauthenticated NN profile for local peer-credential or
  inherited-descriptor sessions.
### Fixed

- ADR 0046 spec docs: corrected stale current-state prose that still described
  the spec set as `Proposed`.
- ADR 0046 topology: replaced the obsolete `W0`-`W10` program range with the
  current topology terminology in the decision register.

### Security

- ADR 0046 release automation: the host-binary release workflow now fails closed
  when unfolded fragments remain under `changelog.d/`, so a release can no
  longer omit changelog entries.
