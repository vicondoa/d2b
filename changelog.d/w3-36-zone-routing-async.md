### Fixed

- Zone enrollment replies are now written as one non-cancellable unit with
  the link FSM transition that produced them, so a serve task dropped
  mid-send closes the connection as the peer's only signal instead of
  leaving the link mid-transition with no reply on the wire.