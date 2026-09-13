### Changed

- `test-host-integration` passes the host's `/dev/vhost-vsock` into the build
  sandbox, so a vmCheck may give its node the same virtio-vsock device an
  enrolled Guest receives. The QEMU the test driver uses already ships
  `vhost-vsock-pci`; only the sandbox's device namespace excluded it.

### Fixed

- The daemon can create the per-Guest subdirectory of a compiler-generated
  `store-view-<guest>` Volume. The generated store view's local-path fallback
  selected the shared state root, which is owned `root:d2bd 0750` for a reader,
  so `ZoneVolumeRootResolver::resolve_root`'s `mkdirat` failed `EACCES` and the
  Volume never resolved; the fallback now selects the daemon-owned policy root
  (`daemon-state`), the same choice the spec store made for the same reason.
- The `guest-shell-service` vmCheck now boots the Guest target agent instead of
  passing while it restart-looped. It installs a fixture ComponentSession
  bundle and key pair at the production `root:d2bd 0640` owner/mode, carries a
  real virtio-vsock device, and asserts the agent is active and logs
  `Guest ComponentSession listener bound` - and that no
  `Guest process bundle validation failed` line ever appears. The acceptance
  Guest in `runtime-cloud-hypervisor-guest-preflight` carries the same
  no-bundle-failure assertion.
- The controller-session read seam resolves its Zone plane per read instead of
  snapshotting the composition's plane table at attach time. The table is
  filled only after the per-zone loop, so the eager attach always saw an empty
  table: the daemon admitted an external Provider controller session and then
  silently dropped it (`CloseReason::RoleMismatch`) after
  `controller_context_is_current` fell back to the durable store, where a
  converted `Process` row never exists. Every external controller session now
  goes live, and a rejected admission logs at WARN with the reason.
- A converted `User` row's status is read from the manager wherever the
  durable row was read alone. Since the System-core conversion the manager is
  the only status authority for `Host`/`User` (R11), and the bundle path
  materializes every row into the durable store with a `Pending` status that
  nothing ever updates. Identity resolution refused every public request
  (`IdentityUnbound`), and the policy compiler dropped the whole RoleBinding
  whose subject was that user, so subject issuance answered `NoMatchingGrant`.
  Identity resolution overlays the manager's live phase onto the rows it
  judges; the row's uid, generation and revision stay store-authoritative.
- The policy compiler binds a subject on its committed identity, not on the
  producer's status currency. It required `phase == Ready` and
  `status.observedGeneration == metadata.generation` before binding a
  RoleBinding subject, which made every compiled grant depend on a status
  stream the compiler does not own: converted rows take their status from the
  manager (KTD3) and the durable bundle path materializes them `Pending`
  without ever updating them, so the binding was dropped at every compile -
  during boot, after the projection repair, and on the controller-session and
  controller-policy-refresh paths, which never consulted the manager at all.
  A subject row now qualifies unless it is `Deleted` or `Failed`; liveness
  stays where it is judged (identity resolution for a caller, session
  establishment for a controller). The boot-time repair that recompiled the
  projection once the manager's rows settled, and the status overlay the
  policy paths needed to satisfy the old gate, are both gone.
- `Process/virtiofsd-<guest>` is no longer projected. The Process driver
  refuses a Guest-owned row that is not `<guest>-vmm`, so the projected worker
  row retried `provider-ticket:guest-process-not-vmm` forever, held its Guest
  not-Ready, and left every dependent resource unready. The live virtiofsd
  worker is minted by the binding driver, which already owned it; the
  projection test now asserts the projection emits no guest-owned `Process`,
  and the preflight fixture bounds the acceptance Guest's owned Process set to
  exactly one.
- The manager-backed API surface reports why it is unavailable: a failed
  manager policy mirror, an absent authorization policy, an unavailable
  manager plane authorizer, and a rejected subject issuance each log at WARN
  with the underlying error instead of failing silently. The controller-session
  rejection path logs its reason instead of a `debug!`.
