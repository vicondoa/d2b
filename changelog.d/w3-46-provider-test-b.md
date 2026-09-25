### Fixed

- The audio pipewire provider's authority tests now assert the saturated mix
  level is capped at 100 even when bounded consumer levels sum past the cap,
  and the mediator tests assert a failed projection set leaves the grant and
  level unchanged.
- The managed-identity provider conformance and topology tests now report
  which method, binding, or permission variant failed instead of sharing a
  bare line number.
