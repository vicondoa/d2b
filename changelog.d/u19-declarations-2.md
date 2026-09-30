### Changed

- The core metadata providers now describe themselves through one declaration
  per Provider instead of one per ResourceType family. `Provider/system-core`
  owns exactly `Host` and `User` per ADR-0046, so it is the single identity
  that declares both: four components (a reconciler and a hosted effects
  service per type) placed at the Host execution target, with the declared
  service and method identities read from the same `ServiceDecl` constants
  the bound descriptors carry. `d2b-provider-host` and `d2b-provider-user`
  contribute the driver descriptors and effects-service factories for their
  own halves and declare no provider identity of their own; the
  `Provider/host` and `Provider/user` artifacts they previously declared are
  gone, because a mutable `Provider` row naming an artifact with no manifest,
  no packaging chain, and no bootstrap admission behind it selects no
  implementation at all.
- The declared effects services are launchable components rather than
  in-process ones. The frozen manifest contract refuses an in-process service
  outright, so a declaration that could never be admitted by a verified
  manifest would claim a component the packaging stage cannot produce. The
  two reconcilers remain the in-process half of the bootstrap controller.
- The foundation seed can now commit `ExecutionPolicy` rows. They are
  collected after the `SeccompProfile` rows their selections resolve against
  and before the roles that may select them, their references resolve over
  the same committed set as every other seeded row, and each one is admitted
  through the same `GraphAuthority::admit_mutation` call the rest of the seed
  uses - the type is system-homed like the other rows the foundation
  commits, so a zone-local plane refuses to write one.
