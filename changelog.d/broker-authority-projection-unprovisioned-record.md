### Fixed

- A Zone the broker holds a durable record for but has published nothing for
  no longer reads as `Unfenced`. `open_session_locked` is what creates a
  Zone's durable record, and it created it with `PersistedPosture::Unfenced`,
  so from the session open until the first `BeginSnapshot` the Zone reported
  open ordinary admission under an initial cursor with no accepted rows and
  no store generation - and that window spans the open's two fsyncs. An
  ordinary effect dispatched inside it was admitted against authority the
  broker does not hold, which is the exact opposite of what
  `ZoneAuthorityState::Unprovisioned` documents ("a Zone is not unfenced
  before its first publication") and of what the envelope's fence exists to
  enforce. The new `PersistedPosture::Unprovisioned` is what `zone_mut` now
  creates: it reports as `Unprovisioned`, it fences ordinary admission, and a
  transfer opened from it still INSTALLS rather than being refused as a
  resynchronization, because a Zone's first document has no prior projection
  to be proved against. A restart moves it to `Reconciling` like every other
  posture that is not a replayable fence. This closes the fence as much as it
  opens it; nothing that was refused is now admitted.
- The broker's unit tests no longer read the process-wide kernel-bundle slot
  unguarded. `retired_network_family_kernels_dispatch_through_the_envelope`
  pins the resolver-dependent kernels on their fail-closed missing-intent
  refusal, so its claim is about the ABSENCE of a verified bundle - but under
  `cfg(test)` the resolver reads the injected slot in preference to the
  unused `bundle_path` the harness configures, and the spawn-kernel tests
  install a real bundle there for the whole of their own test. The case was
  therefore answered from a neighbour's fixture whenever one overlapped it,
  and observed that fixture's intent instead of the absence. It now owns the
  slot for its whole body, which is what
  `the_installed_admitted_effect_table_serves_no_operation` already did for
  the same claim.
- `RegistryTestGuard` no longer clears the kernel-bundle slot outside the lock
  that guards it. Every installer holds `TEST_KERNEL_BUNDLE_LOCK` for the
  whole of its test and restores the previous value on drop, so a clear taken
  without that lock could land in the window between one test releasing it
  and the next test acquiring it, and wipe the bundle that second test had
  installed and was relying on. The clear now takes the same lock, and the
  order stays registry-then-bundle on both sides.