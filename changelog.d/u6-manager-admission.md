### Added

- New-graph mutation admission and canonical relation indexing (U6, KTD2-KTD4;
  R2-R8, R16-R18, R35). `d2b-core::resource_authority` is the one pure graph
  admission evaluator: it depends only on the contract layers, decides every
  mutation for its initiating subject so a privileged transport cannot widen a
  grant (AE15), admits a binding only against a prior accepted source decision,
  and admits bootstrap work only for the exact verified deployment root.
  `d2b-resource-runtime::relations` derives six distinct typed relation indexes
  - ownership, consumption, implementation, placement, authorization, and
  dependency/observation - from committed desired rows alone, so a restart
  rebuilds the identical index and no separately authored dependency list
  exists to drift from the rows. The manager gains authenticated mutation
  entry points that carry a typed subject instead of rendered text, require the
  source controller's own authenticated authority to create a source-owned
  binding, and normalize each consumer slot against the derived index before
  any mutation. The daemon composition adds `GraphMutationAdmission`, the
  production-shaped adapter U34 installs; the unchanged production entry point
  keeps its current admission until that atomic cutover.