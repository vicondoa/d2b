### Fixed

- The d2b-broker-composition routing-refusal test no longer pins the refusal's Display wording: its assertion now checks the refusal variant and class, so rephrasing the message text cannot fail the suite.
- A composition seam test that claimed to exercise an admitted-without-handler startup invariant leg now asserts only what the committed catalog can exercise this pass - the empty admitted set with nothing registered - so the suite documents the actual reachable behavior instead of a discarded fixture.
