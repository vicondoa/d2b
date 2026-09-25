### Changed

- Provider test coverage tightened across five provider crates: the
  `d2b-provider-provider` Degraded phase projection now has a direct
  `plan_observed` test, the shell-terminal supervisor runtime tests share one
  parameterized pool fixture instead of seven inline copies, and the
  zone-link, transport-vsock, and wayland-policy tests now fail with a
  per-case message (or a real assertion) instead of a bare line number or a
  tautology.