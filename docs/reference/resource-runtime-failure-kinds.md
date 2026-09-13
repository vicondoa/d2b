# Driver failure kinds

Generated from `d2b_resource_runtime::error::FailureKinds::ALL` by
`render_failure_kind_reference`; the `d2b-resource-runtime` unit test
`failure_kind_reference_doc_matches_the_registry` fails when this page
drifts. Regenerate with
`cargo test -p d2b-resource-runtime --lib -- --ignored regenerate_failure_kind_reference`.

A failing driver operation reports one of these codes with its verdict
(`not-yet` defers and requeues, `refused` is terminal, `error` carries
the driver's retry class), the stage, and the compared values behind the
failure.

| code | means | likely cause |
| --- | --- | --- |
| `row-unavailable` | The manager plane could not answer a row read. | Manager RPC failure, no published plane, or an unusable payload; retry. |
| `children-draining` | Owned children have not finished their own finalize/delete pass yet. | Child-first teardown; the parent requeues and re-drives the children. |
| `target-unavailable` | The resource's realization target session is gone. | Target disconnect; the target's own reconnect drives the next attempt. |
| `driver-not-yet` | The driver operation cannot proceed yet. | The world has not reached the state the pass needs; defer and requeue. |
| `driver-refused` | The driver refused this row. | Committed input the pass cannot change; retrying cannot converge. |
| `driver-error` | An operational driver failure. | A provider effect or store call failed; retry unless terminal evidence exists. |
| `process-spec-invalid` | The durable spec did not decode as the closed Process contract. | Stored bytes are not canonical Process/EphemeralProcess JSON. |
| `process-identity-incomplete` | The row's launch identity is incomplete or invalid. | Missing or invalid zone, uid, generation, or execution binding inputs. |
| `process-provider-unsupported` | The spec selects a Provider this driver does not own. | providerRef names a Provider outside the process family this daemon serves. |
| `process-execution-unsupported` | The spec's execution target is not drivable in this daemon mode. | The row's executionRef or execution domain is not allowed here. |
| `process-template-unavailable` | The trusted bundle holds no template binding for the requested ticket. | The preflight or treated template was never materialized for this row. |
| `process-resolution-refused` | The trusted bundle refused to resolve the launch ticket before any launch. | No matching intent, or a wrong execution target, scope, or descriptor posture. |
| `process-guest-process-not-vmm` | The row is a Guest-owned process outside the guest VMM chain. | A projected preflight intent no host-minted ticket can describe. |
| `process-identity-ambiguous` | The observed process identity is ambiguous; the process is quarantined. | Several candidates matched, or the observation drifted from the ticket (R15). |
| `process-provider-effect-failed` | A provider process effect failed. | The launch, stop, wait, or finalize call returned an operational error. |
| `process-start-budget-exhausted` | The in-memory restart budget for this row is exhausted. | Repeated restarts inside the window; a spec change or daemon restart resets it. |
| `process-drain-pending` | Owned process children are still retiring. | Children must go first; the delete pass requeues and re-drives them. |
| `binding-spec-invalid` | The durable spec did not decode as the strict neutral binding contract. | Stored bytes are not canonical VolumeBinding JSON. |
| `binding-provider-unsupported` | The spec selects a Provider this driver does not own. | providerRef is outside the volume-virtiofs binding family. |
| `binding-owner-mismatch` | The declared parent Volume row is owned by a different resource. | The committed rows disagree on ownership; adopting would silently re-parent. |
| `binding-parent-unavailable` | The declared parent Volume row is not observable yet. | The parent is not committed yet, or the manager plane could not answer the read. |
| `binding-parent-spec-invalid` | The declared parent Volume row does not hold a usable Volume row. | The committed parent row's uid or stored spec does not decode as canonical Volume. |
| `binding-plan-derivation-invalid` | The worker plan could not be derived from the binding and its parent. | View rights or vcpu inputs do not satisfy the plan contract. |
| `binding-serving-effect-failed` | A provider serving effect failed. | The bind/unbind call returned an operational error; the pass retries. |
| `binding-child-mutation-failed` | A manager child ensure or delete failed. | The manager RPC failed; the manager owns the retry. |
| `volume-spec-invalid` | The durable spec did not decode as the closed Volume contract. | Stored bytes are not canonical Volume JSON. |
| `volume-provider-unsupported` | The spec selects a Provider this driver does not own. | providerRef is outside the volume-local family this daemon serves. |
| `volume-layout-effect-failed` | A provider layout effect failed. | The layout call returned an operational error; the pass retries. |
| `volume-layout-not-ready` | The provider layout report is not Ready yet. | The layout is Degraded or Pending; requeue instead of respawning the effect. |
| `volume-child-mutation-failed` | A manager child ensure or delete failed. | The manager RPC failed; the manager owns the retry. |
| `volume-child-derivation-invalid` | The derived child set does not satisfy the Volume contract. | The spec's derived children are malformed or not owned by this row. |
| `endpoint-spec-invalid` | The durable spec did not decode as the closed Endpoint contract. | Stored bytes are not canonical Endpoint JSON. |
| `endpoint-shape-unsupported` | The spec is an Endpoint shape this driver does not realize. | Class, transport, visibility, or producer ref outside the realized set. |
| `endpoint-socket-effect-failed` | A provider socket effect failed. | The socket realize/remove call returned an operational error. |
| `endpoint-drain-pending` | Owned endpoint children are still retiring. | Children must go first; the delete pass requeues and re-drives them. |
| `guest-spec-invalid` | The durable spec did not decode or names a Provider outside the row's type. | Stored bytes are not canonical Guest JSON for the selected Provider. |
| `guest-child-mutation` | A manager child ensure or delete failed. | The manager RPC failed; the manager owns the retry. |
| `guest-provider-unavailable` | The Guest Provider path is temporarily unavailable. | Provider call, manager plane, or row read failed; retry. |
| `guest-finalize-pending` | A Guest Provider teardown stage is still progressing. | Cleanup has not converged; the owner is re-entered on the next pass. |
| `core-spec-invalid` | The stored spec envelope did not decode, or the row identity is not a contract reference. | Stored bytes are not the JSON spec object the core types store. |
| `core-dependency-read-failed` | A dependency or owned-child read failed. | Manager RPC failure or a missing child row; retry. |
| `core-drain-pending` | Owned core children are still retiring. | Children must go first; the delete pass requeues and re-drives them. |
| `system-core-spec-invalid` | The durable spec did not decode as the closed Host/User contract. | Stored bytes are not canonical Host/User JSON for an admitted Provider. |
| `system-core-host-observation-failed` | The Host observation could not be read. | The provider's host status read failed; retry. |
| `system-core-user-discovery-failed` | User or group discovery failed. | The host's user database could not be read; retry. |
| `system-core-drain-pending` | Owned system-core children are still retiring. | Children must go first; the delete pass requeues and re-drives them. |
