### Fixed

- The Guest, User, and VolumeBinding provider test-support doubles now record
  through the provider toolkit's shared ordered call log instead of each
  crate carrying its own recorder shape, so the recording doubles cannot
  drift apart.