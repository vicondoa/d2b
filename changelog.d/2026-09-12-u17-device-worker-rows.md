### Added

- The private bundle resolves Device-owned worker rows. `device_worker_posture`
  (`d2b-core::bundle_resolver`) is the one closed table of the five worker
  templates a Device Provider declares - `swtpm-socket`, `swtpm-init-flush`,
  `gpu-worker`, `gpu-render-node`, `video-worker` - carrying for each the
  broker runner role, the packaged executable, the seccomp policy class, the
  namespace classes, the user-namespace requirement, the device binds, and
  the umask. `build_device_worker_intents` mints one trusted intent per
  declared row from it, and `BundleResolver::find_device_worker_intent`
  resolves a launch through the exact declared row (role id = row name) and
  the declared template, refusing ambiguity.
- `d2b-resource-compiler` projects `append_device_tpm_worker_templates` and
  `append_device_gpu_worker_templates`: for every `Process/swtpm-<device>`,
  `EphemeralProcess/swtpm-flush-<device>`, `Process/gpu-<device>`, and
  `Process/video-<device>` row the Device Provider declares, it emits one
  non-dynamic `ProcessTemplateBinding` pinning the artifact executable and
  admitting controller-supplied launch arguments. A row whose declared
  sandbox disagrees with its template's closed posture is refused at compile
  time (`provider-device-worker-posture-mismatch`), and a template whose
  executable the artifact does not package yields no binding.
- `ProcessTemplateBinding` gained `new_with_launch_args` (declared row,
  bounded launch arguments) and `Process/swtpm-<device>`-style
  `EphemeralProcess` rows are bindable. The declared-row arm of
  `ResourceBundle::verify_process_templates` now admits a Device-owned worker
  row: the binding's owner must be the Provider the owning Device's
  `providerRef` names, the row's `executionRef` and `template` must equal the
  binding's, and the row must be a `worker` class row.

### Changed

- The supervisor's launch resolver gained the Device branch: a ticket whose
  semantic owner is a `Device` resolves through
  `find_device_worker_intent`, so a Device-owned row never falls through to
  the generic template lookup (which two Devices in one Zone share).
- The broker's typed Process metadata fence admits a Device worker only with
  its `Device` owner (and requires one), and binds its template identity to
  the declared template (`intent.profile_id`) rather than the per-row role
  id.
- The TPM and GPU Device Providers now declare the sandbox posture of their
  worker rows in Nix (`namespaceClasses`, `userNamespace`, `umask` `0007`),
  which is the public half of the same posture the private binding pins and
  the broker fences: `w1-swtpm` with a `process-principal-root` user
  namespace for the long-lived swtpm worker, `w1-swtpm` without one for the
  one-shot flush, `w1-gpu` with `kvm`/`dri`/`udmabuf` binds, `w1-gpu-render-node`
  with the broker-pre-opened render node and no bind, and `w1-video` with a
  pid namespace and the DRI bind.
