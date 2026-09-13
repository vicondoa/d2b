### Changed

- One canonical `LaunchIdentity` (`d2b-process-conformance`) replaces the
  partial, per-layer Process launch identity. The value carries the semantic
  owner ref and UID, the exact execution target, the cross-target Guest
  selector, the broker VM scope, and the legacy runner role, and it is
  validated at construction: an incomplete row fails once, naming the missing
  input (`launch-identity-missing-target-ref`,
  `launch-identity-invalid-execution-ref`, ...).
- One resolver (`resolve_launch_identity`, `d2bd::process_resource_runtime`)
  produces that value from the durable row (`owner ref -> owner uid ->
  execution target -> target ref -> vm/role`). The Guest-owned guest-runtime
  target derivation (`guest_runtime_process_matches`) is now a rule inside
  the resolver instead of a call-site special case in the Process driver.
- The launch ticket carries the resolved identity and consumes it (owner,
  target, VM scope, binding-worker split) instead of re-deriving
  `target_vm_name`; the bundle process-DAG ticket path names its own VM
  through the same value. The broker's identity fence
  (`BundleBackedLaunchResolver::resolve_intent`) reads
  `ticket.launch_identity()` for the launch VM, the legacy role, and the
  serving-worker split instead of matching `(execution_ref, target_ref)`.
- The Process driver's adopt/probe identity (`ProcessResourceIdentity`) now
  embeds the resolved `LaunchIdentity`; the legacy `ProcessResourceRuntime`
  context resolves the same value for unconverted rows, and the guest-side
  Cloud Hypervisor probes resolve through the same resolver at ticket time.
- An incomplete identity is refused as `process-identity-incomplete` at the
  driver boundary (a terminal, named failure) instead of surfacing as a
  fence rejection one field at a time.
