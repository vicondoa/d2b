### Added

- Device-owned worker rows launch with typed launch parameters instead of a
  bare template argv. `DeviceWorkerLaunch` (`packages/d2bd/src/process_provider_runtime.rs`)
  is the closed enum over the four declared families (`Swtpm`, `SwtpmFlush`,
  `Gpu`, `Video`) with one params struct each; the Process driver derives it per
  declared row (`Process/swtpm-<device>`, `EphemeralProcess/swtpm-flush-<device>`,
  `Process/gpu-<device>`, `Process/video-<device>`) from the trusted template,
  the owning Device row and the daemon's runtime paths, and the provider
  composes the generator's argv from it (`device_worker_launch_args`). The
  Process spec stays argv-free and the declared rows stay path-free by contract;
  the parameters are derived, never authored into either.
- The one-shot outcome is published as the row's status projection
  (`{"ephemeral": {"state": "succeeded" | "failed", "code": "..."}}`), so a
  reader can tell a concluded pass from a successful one. The TPM effect port's
  flush gate reads that projection rather than the phase, which is what lets a
  failed flush fail the device path.
- `tests/host-integration/device-worker-launch.nix` is the end-to-end proof on a
  live host: the compiled zone bundle carries the declared rows and their
  digest-pinned `launchArgs` bindings, the rows reach the manager with their
  Device owners, the swtpm worker really runs with the argv the Process
  controller composed (state directory, ctrl/server sockets, socket principal)
  and binds its sockets, the flush publishes its one-shot outcome, `d2b delete
  Device/...` retires the declared rows children-first and leaves no worker, and
  the GPU rows never report Ready but end Failed with a named driver failure.

### Changed

- Every controller launch in the daemon goes through the Process controller.
  The TPM swtpm/swtpm-flush effect and the GPU/video worker read, gate and
  retire the Device Provider's declared `Process`/`EphemeralProcess` rows
  instead of spawning through the broker surface, and the VM bring-up launcher
  has no raw-broker fallthrough left: a node that is neither provider-managed
  nor the controller-owned Cloud Hypervisor runner is refused with
  `vm-start-node-not-provider-managed:<node>`.
- The grep gate over `packages/d2bd/src` is clean: no production
  `BrokerRequest::SpawnRunner` caller remains, and the only hits are test-side
  (the composition tests' fake-broker envelope readers and the `#[cfg(test)]`
  Device-TPM reconcile simulation with its fail-closed `NoManagerChildSurface`).
  The sanctioned funnel stays `BrokerProcessBackend::request_with_fds` behind
  `ProcessLaunchEffectPort`.
