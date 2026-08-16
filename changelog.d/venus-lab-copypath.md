### Fixed

- Corrected a measurement in the Venus Vulkan Video lab prototype that ruled out
  the obvious repair for its GPU-copy presentation path. The record stated that
  forcing that path leaves the second image plane unimported, so nothing existed
  for the copy to find. Re-measuring shows the plane is imported. The two runs
  reached the same path by different mechanisms, one by removing a graphics
  capability preference and one by turning the surface preference off, and only
  the first suppresses the separate import. The repair is therefore unresolved
  rather than ruled out. The re-measurement also confirms the failure is no
  longer catastrophic: the copy still fails, but it no longer poisons the
  rendering context or causes submissions to be refused, and playback continues.
  The prototype is unaffected either way, since it uses the zero-copy path.
