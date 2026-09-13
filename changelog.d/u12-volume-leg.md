### Changed

- Retired the residual legacy U7 volume-leg runtime: the old-plane
  `VolumeBinding` shared runner
  (`SharedVolumeResourceReconciler` over `DaemonVolumeProviderEffects`, and
  its `volume-local` sibling registration) is gone, so the v3 plane's
  `VolumeDriver`/`BindingDriver` own the whole storage family.
- Folded the surviving legacy binding behavior into `BindingDriver`:
  `reconcile` derives the `VirtiofsdWorkerPlan` and carries it in the
  in-memory status as the serving authority (the plan never travels in a
  resource, KTD1; the worker Process child stays argv-free and the Process
  controller composes its launch, KTD13/U17), ensures the worker/endpoint
  children commit-before-spawn, retires owned children the derived set no
  longer names endpoint-first/process-last, registers the Volume and child
  dependency watches (R12/R17), and requeues while the child set is not yet
  current; `recover` adopts only when both child rows are current and the
  serving socket is listening; `delete` preserves the KTD6 drain gate (a
  guest mount observed before anything is deleted blocks the teardown, and
  the durable deleting mark plus the owned children stay for a retry) and
  the endpoint-first / process-last teardown, with the manager holding the
  parent row until the last child retires (F3) exactly as the old finalizer
  did. Terminal plan/view rejections keep their stable provider reason in
  the in-memory status (KTD5).
- Deleted `packages/d2bd/src/resource_runtime/volume_provider_runtime.rs`
  (the U7 kind/effect/reconciler/runner machinery), the `start_u7` /
  `stop_u7` runner lifecycle and its task/lock/readiness plumbing in
  `resource_runtime.rs`, and the `composition.rs` startup call. The
  `volume_effect_adapter.rs` module stays: the v3 plane's production volume
  effects build on it.
