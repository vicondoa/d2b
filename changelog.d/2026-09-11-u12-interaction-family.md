### Changed

- Converted the interaction/shell family to the v3 plane: new
  `InteractionDriver`/`InteractionDriverFactory`
  (`packages/d2bd/src/interaction_driver.rs`) serves the six
  display/audio/shell types (`display-wayland.d2bus.org.WaylandPolicy` /
  `WaylandSession`, `audio.d2bus.org.AudioService` / `AudioBinding`,
  `shell-terminal.d2bus.org.ShellPool` / `ShellSession`) over the
  `InteractionDriverEffects` port
  (`resource_runtime/interaction_effects.rs`). Reconcile registers the
  family's dependency/child watches once per target (R12/R17), ensures the
  desired child set through the manager child API (commit before spawn),
  retires obsolete children endpoint-first/process-last, runs the preserved
  provider effect, publishes in-memory status (R11) and requeues at the
  family's resync cadence; delete runs the provider stage before child
  retirement and stays retryable while a stage is pending. The six types
  join `V3_CONVERTED_RESOURCE_TYPES`.

### Removed

- Deleted `packages/d2bd/src/resource_runtime/interaction_provider_runtime.rs`
  and the U9 shared-provider arms (six kind variants, dispatch, effect and
  child helpers) from `resource_runtime/shared_provider_runtime.rs`, plus
  the dead U9-era child machinery in `binding_child_resource_runtime.rs`
  and the audio runtime's child-ownership surface.
