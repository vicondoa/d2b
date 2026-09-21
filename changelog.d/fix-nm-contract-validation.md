### Fixed

- The NetworkManager contract is now validated where it is declared: an
  unknown reload behaviour is refused before anything is written instead of
  being accepted with the reload silently skipped, and a declared owner or
  group is applied inside the atomic replace so the path is never observable
  with drifted ownership. An unresolvable principal refuses by name.
