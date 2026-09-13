### Added

- U17 launcher-sweep documentation at the two remaining direct broker
  `SpawnRunner` sites (the TPM swtpm/swtpm-flush spawn and the GPU/video
  worker spawn) while they existed. At the time of the sweep the v3 bundle
  carried no launch intent for the providers' typed templates
  (`swtpm-socket`, `swtpm-init-flush`, `gpu-worker`, `gpu-render-node`,
  `video-worker`): the resource compiler projected worker templates for
  virtiofsd and the managed-identity agent only, and the generic Process
  ticket path's template matcher knew only the legacy role names. The
  projection, its resolver posture table, and the supervisor/broker fences
  landed in the follow-up slice (`2026-09-12-u17-device-worker-rows.md`), and
  the port slice (`2026-09-12-u17-port-conversions.md`) converted both sites;
  no direct daemon spawn remains, and the launch-parameterization gap those
  ports still have is recorded there and in the plan's U17 status.

### Changed

- The VM bring-up launcher (`VmStartRunner`) never spawns through the raw
  broker surface. Its `SpawnRunner` fallthrough is removed and any node that
  is neither provider-managed nor the controller-owned Cloud Hypervisor
  runner is refused with `vm-start-node-not-provider-managed:<node>`. The
  fallthrough was unreachable by live nodes: both `NodeRunner` entry points
  already filter the guest-owned and durable-wayland node classes (a
  guest-owned process is launched by its Guest, a durable-wayland node stays
  readiness-only), so provider-managed sidecars keep launching through
  `ProductionProcessProviders::launch_node` and the daemon no longer holds a
  second, legacy launch path for them.
- The USBIP attach child port (`SharedRunnerUsbipChildren`) documents why it
  stays fail-closed: its attach seams belong to `BindingLifecycle`, which no
  production path constructs (it exists only in the USBIP provider's tests).
  The v3 Binding realizes its attach through the declared child resources -
  the Guest `Process/...guest-proxy` plus its Endpoint - committed as
  owner-scoped child rows of the Binding and launched by the Process
  controller, so no attach spawn is faked here.

### Removed

- The dead per-env usbipd spawner (`run_usbipd_perenv_autostart` and
  `BrokerPerEnvUsbipdSpawner`) is deleted from the daemon: it had no
  callers, and per-env usbipd runners are attach-owned rather than
  startup-owned. The `d2bd-runtime` spec module it used stays with its own
  tests.
- The legacy VM-start launch machinery freed by the fallthrough removal:
  `VmRunnerLaunch::Legacy`, `register_node_pidfd`, the surviving
  `cleanup_vm_start_registration` helper, and the `VmStartRunner` fields
  (`lifecycle_authorization`, `workload_identity`, `network_tap_context`)
  that only the raw `SpawnRunner` request read.
