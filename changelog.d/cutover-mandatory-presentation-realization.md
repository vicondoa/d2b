### Removed

- A process launch no longer has a pre-graph presentation posture. The broker's
  spawn plan, its input, and the isolation spec carried
  `presentation: Option<PresentationRealization>`, where `None` meant "the role's
  own verified sandbox realizes its source and the broker applies no mount of
  its own". That branch is gone: a launch now always names the realization its
  trusted implementation declares. The preflight checks that make an admitted
  mount impossible to skip run unconditionally, so a launch cannot report
  itself ready while a mount it depends on was never applied.

### Changed

- The broker realizes an admitted presentation from the execution plan instead
  of leaving the destination unprepared. The realization is resolved and the
  plan's own destinations are bound by the broker before any descriptor is
  opened. This path previously existed but was never called from production,
  which left the broker's live launch reverse-parsing its mount policy out of
  the wire payload.
- The realization comes from the trusted implementation's declared capability
  rather than from the row that happens to request the launch, and it is
  recorded on the launch template the broker resolves against. A launch that
  asks for a destination its declared realization cannot place is refused with a
  named reason instead of silently skipping the mount.