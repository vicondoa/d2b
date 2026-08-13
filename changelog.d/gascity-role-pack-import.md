### Fixed

- Fixed the Gas City contributor graph to register the d2b rig and import its
  upstream role pack at rig scope so configured agent patches resolve and
  submitted work can start, while safely materializing writable configured
  assets from the package's immutable symlink tree and exposing the managed
  Dolt identity and packaged lifecycle tools to the supervisor. The optional
  bd maintenance dog remains suspended because it has no workflow identity for
  the authenticated ACP launcher, while routed sessions use Gas City's
  standard session bead identity at that boundary. Managed rig files use the
  dedicated lifecycle-agent-check worktree group so sandboxed agents can update
  their assigned checkout and the isolated check runner can validate it without
  exposing repository contents to unrelated sidecars. Run-scoped GC roots bind
  to the durable workflow root separately from transient agent session beads.
