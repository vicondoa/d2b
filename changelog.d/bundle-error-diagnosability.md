# `bundle-error-diagnosability.md`

### Changed

- The Network intent source's daemon-supplied loader now journals the typed
  bundle-load error at the point where a failed load collapses into no
  intent, instead of discarding it. The fail-closed posture is unchanged -
  a tampered, missing, or unreadable trusted bundle still yields no intent
  and the kernel broker still refuses with its generic closed code - but
  the root cause now reaches the daemon journal: a tampered bundle logs
  `BundleTampered` with its reason, and a missing or unreadable bundle logs
  the `InternalIo` detail, so bundle-problem refusals are diagnosable again.