### Fixed

- The broker pins the mode of the files it exports for observability instead of
  relying on the ambient umask, so a restrictive service umask can no longer
  strip the group-read bit and silently break the readers of that channel. Two
  disk-image tests that seeded files with a mode the ambient umask could mask
  now seed deterministically and exercise their intended posture.
