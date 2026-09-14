# `fix/u31-ownership-matrix-preflight.md`

### Changed

- The host-prep DAG's ownership-matrix step now runs its check daemon-side
  (`d2bd_runtime::ownership_preflight`) instead of dispatching the broker's
  stub request, so VM start actually refuses when the per-VM state subtree
  drifts from the committed ownership matrix. The daemon no longer constructs
  the typed broker request for this family.