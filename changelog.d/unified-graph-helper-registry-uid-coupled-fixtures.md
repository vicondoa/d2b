### Fixed

- The `d2bd-runtime` unsafe-local helper registry fixtures are no longer
  coupled to the uid of the machine running them. Two tests dispatched a
  launch as the runner's own uid while the launch's admission named a
  hardcoded requester, so the registry's refusal of an admission that names a
  different subject fired before the behaviour each test exists to assert.
  One test read `Err(RequesterMismatch)` where it asserted `Err(QueueFull)`,
  and the other waited forever on a helper frame the refused dispatch never
  sent. Both were green on a runner whose uid happened to equal the literal
  and red or hung on every other runner, so the suite's result depended on
  who ran it. The launch fixture now takes its requester as an argument, the
  queue-saturation test chooses its own fixed subject and reads no ambient
  identity at all, the correlated-dispatch test admits the subject the kernel
  reports for its own socket, and the helper-registration fixture takes that
  subject as a parameter instead of re-deriving it, so one subject cannot be
  split across the registry, the dispatch and the admission again.