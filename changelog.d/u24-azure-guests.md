# Convert the Azure Guest implementations to the admitted graph

`Provider/runtime-azure-container-apps` and
`Provider/runtime-azure-virtual-machine` each gain the two halves of a
converted Provider: an authored declaration (`declaration.rs`) that says what
the Provider is - the `Guest` type it reconciles, the Guest execution target
its controller runs at, and the presentation ceiling it realizes - and a
remote authority (`authority.rs`) that decides, before any cloud call, whether
the graph admits this Provider mutating *this* Guest in *this* cloud account
under *this* credential relationship.

The authority runs one ordered gate: the declared presentation ceiling, the
admitted Guest uid and generation, the admitted cloud target, the shared
`admit_credential_delivery` credential relationship, and the ARM audience the
admitted session carries. Every remote call in both controllers goes through
it - `AcaController::with_lease` and `AzureVmController::arm_token` are the
single paths to their control planes - so a wrong target, a reconfigured
Provider row, or a revoked credential stops a mutation instead of failing it.

Both authorities derive a deterministic cloud identity from the admitted
relationship's own Zone, Guest uid, and generation. `AcaDesiredSandbox`,
`AcaDesiredDiskImage`, and `AcaWorkloadQuery` all carry it, so the find path
and the create path address one cloud name; the Azure VM controller uses the
derived ARM idempotency token in place of its caller-supplied one. A retry
after an ambiguous response therefore reconciles the resource the first
attempt intended rather than creating a second.

Release is now a reportable fact rather than an inferred one:
`AcaReleaseEvidence` and `AzureVmReleaseEvidence` name the delete outcome,
the bootstrap-extension removal, the child-resource cleanup, and the dropped
finalizer, and a recovered `AzureVmRecoveryState` carries the admitted
identity it is fenced on, so a restart cannot resume an in-flight ARM
operation against a replaced Guest or a reconfigured Provider.

The unchanged entry points keep working: a controller with no authority bound
behaves exactly as before, and the composition root in
`d2b-provider-guest/src/effects_service.rs` still constructs both controllers
the old way.
