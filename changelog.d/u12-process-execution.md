# Process, minijail, and supervisor execution move onto the resolved plan

The Process family gains one resolved execution plan, built beside the launch
ticket rather than replacing it.

`d2b-process-conformance` gains a `plan` module: a `ProcessSubject` that pins a
committed consumer identity, typed `ProcessResourceRequest` relationship claims,
a `ProcessPlanRequest` that carries the instance and one authorized
`ExecutionPolicy` selection, a `ProcessPlanValues` projection of the broker's
own `ExecutionPlan`, and `resolve_process_plan`, which folds the two into a
`ResolvedProcessPlan` carrying the effective `AdmittedExecution`, one
`PreparedBinding` per admitted relationship, screened `ProcessLaunchArguments`,
five-way `ProcessLaunchEvidence`, and a `ProcessLaunchScope` that can only
release what its own launch prepared.

Both lifetimes now share that one policy path. A long-running `Process` and a
run-to-completion `EphemeralProcess` are classified through the contract crate's
canonical type-name constants (`ExecutionInstanceKind::of_reference`) rather than
a private copy of the vocabulary, and the kind is recorded in the admitted
execution rather than branched on.

A supplied launch argument that names a binding-selected source or destination is
refused at the screen, not dropped: the destination comes from the resolved plan,
and silently dropping a positional value would run the process with a different
meaning than the row asked for.

`d2b-contracts-resource` gains `ExecutionInstanceKind::of_resource_type` and
`of_reference`, and `d2b-core`'s test support gains an `AcceptedRelationship`
builder so a downstream fixture can build a resolving accepted graph without
restating the role vocabulary.
