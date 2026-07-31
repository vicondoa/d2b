### Added

- The `system-core` bootstrap Provider now reconciles `Host` and `User`
  resources. A user-only Host always reports the no-isolation posture and
  its fixed message, a Host with any other execution policy reports no
  posture at all, and a status that tries to set or suppress either field is
  rejected rather than merged. Local User discovery reports whether the
  machine resolves a declared identity, and reports a declared group
  membership that did not verify as drift rather than as readiness.
- `system-core` refuses every resource type outside `Host` and `User`,
  including `Process`, `EphemeralProcess`, `Volume`, `Network`, `Device`,
  `Credential`, and any semantic type. The boundary is an allowlist, so a
  type nothing has claimed yet is refused too.

### Changed

- The `system-systemd` and `system-minijail` Process Providers now treat a
  Guest execution parent exactly as they treat a Host: a process launched
  under either reports the same status apart from the execution reference,
  and a user-domain launch requires the same exact user identity under both.
- Public status from the system Providers carries no user name, home
  directory, shell, numeric identity, unit name, cgroup, or path. A user
  resource's declared operating-system username is never restated in its
  status.
