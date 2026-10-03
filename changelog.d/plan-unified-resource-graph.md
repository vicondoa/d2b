### Added

- Added the canonical confinement, callable, and authority contracts for the
  unified resource graph. The new `ExecutionPolicy` resource states what an
  execution instance may be - required isolation classes, the Linux
  capability ceiling, the restrictions it may not weaken, the identity it may
  resolve to, and the syscall filter it must load - and carries no volume,
  device, network, endpoint, credential, mount, or host-path field, so
  resource access can no longer be granted by naming a policy. Admission
  composes field-wise and refuses on conflict: a missing required class, a
  capability outside the ceiling, a weakened mandatory restriction, an
  unauthorized identity, an incompatible syscall filter, a request over the
  admitted budget ceiling, or a mandatory confinement facet the target cannot
  enforce is refused with its enforcing stage, never intersected into a
  successful but unconfined launch. A long-running `Process` and a
  run-to-completion `EphemeralProcess` pass through that one path, and
  selecting a policy is a request: without Role and RoleBinding authorization
  evidence the selection is refused even when every other rule would admit it.
- Added the narrowed `SeccompProfile` contract, which carries the syscall
  filter and its default action and nothing else. Namespace, cgroup,
  device-node, and mount authority are not syscall-filter concerns, so a
  profile row that still carries them is rejected rather than decoded with
  its authority quietly dropped.
- Added the canonical `Operation` contract, where an operation binds one
  trusted implementation - a declared provider component method or a
  provider-owned executable template - instead of an owning `Command` row.
  A caller cannot select code, and a mutable provider row cannot introduce a
  compiled privileged handler by naming an artifact.
- Added the shared authority vocabulary every admission and effect surface
  speaks: the initiating subject, a per-row desired revision that advances on
  every committed desired mutation rather than only on a spec-generation
  change, a per-Zone desired sequence, a store incarnation that is an
  identity and never an ordering, the freshness tuple an effect is fenced
  against, the stage a decision was made at, and a field-free refusal reason
  that can be logged or audited verbatim.
- Added the authorization-only Role contract, which states which resource
  verbs a subject may use and which declared operations it may call. The
  retired posture, its path grants, and its command references are rejected
  by the decoder instead of being decoded with their authority ignored.

The existing `Host`/`Guest` execution-policy fragment, the `Command`-owning
operation row, and the role posture remain in place for the production entry
point that has not switched yet; the conversion units that own those callers
switch them to these contracts, and the integrated cutover removes what they
replace. Production schema and generation output are unchanged by this entry.

### Changed

- QEMU's runner now takes its private descriptor slots from admitted graph
  relationships. The acceleration `Device`, the tap `Network`, each media
  `Volume`, and the display `Endpoint` each contribute one private slot, and
  every slot carries the `BindingKey`, `SourceReservation`, admitted right,
  and source arbitration that authorized it. A relationship whose source
  refused the right, whose access mode the Provider does not realize, whose
  committed rows have moved on, or whose lifecycle admits no new use produces
  no slot at all, and the projection is total: it returns a complete
  descriptor list or nothing.
- The QEMU runner is realized as a helper on the Guest's own reservation. It
  takes an attenuated realization leg derived from its parent relationship's
  own key, reservation, source, and right - never a second reservation, a
  competing writer, or a second device allocation - and a helper asking for a
  right its parent was not admitted for is refused before the leg is built.
- QEMU media shutdown now closes the consumer's descriptors, drops the
  runner's realization legs while the reservation is still held, stops the
  process, and only then releases the source.
- The controller-created runtime `Volume` derives the runner's storage request
  as an ordinary typed `VolumeBinding` request, so the QMP and serial sockets
  reach the runner through an admitted relationship rather than a mount the
  `Process` contract hard-coded.

### Fixed

- A Guest image that runs the target agent now delivers its own Zone's
  verified deployment graph. `serve_guest` publishes the Guest's target-local
  authority before it binds its ComponentSession listener and refuses to serve
  without one, but the host-integration `guest-shell-service` node enabled that
  agent without setting `d2b.componentSession.deploymentBootstrap`, so nothing
  materialized `/etc/d2b/deployment/deployment-bootstrap.json` and the unit
  restart-looped on `deployment bootstrap refused: deployment-bootstrap.json is
  absent, unreadable, or over the read bound`. The node now builds the document
  from the one verified-graph constructor the production `evalGuest` path uses.
- The binding-owned virtiofsd `Endpoint` no longer names a Provider as its
  consumer. `EndpointConsumerTarget` admits only `Host`, `Guest`, `Process`,
  and `EphemeralProcess` as an execution target, so a `Provider/...` subject
  refused the whole row at its delivery derivation with `endpoint-spec-invalid`
  and the status `the request names an endpoint this Zone does not own`. That
  failure is terminal, so the socket was never realized, the owning
  `VolumeBinding` never reported ready, and every Guest that mounts a shared
  Volume stayed unmounted. The endpoint publishes to nobody: the binding
  resolves the socket from its own row and the worker binds it.
- The host-integration `virtiofsd-volume-runtime` realize wait asserts the
  consumer-side destination through the presentation `VolumeBindingSpec`
  actually commits. It read a top-level `spec.mountPath`, which that contract
  has never carried since the destination moved into the closed `presentation`
  vocabulary, so the wait could not succeed against any row. It now asserts the
  committed `spec.presentation` instead, with the same destination.
- A ComponentSession no longer discards ttrpc frames it has already read off
  the wire when the session ends. `EventQueue::fail` failed every waiter and
  dropped the frames the driver had read but not yet handed out, so a peer
  that ended the session right after answering a request took the answer with
  it: the caller's `receive_ttrpc` returned `session-disconnected` instead of
  the response the peer had already written. The visible symptom was a ttrpc
  call that had been answered losing its response under load - a
  `DisplayService/Finalize` whose response the finalizing peer never
  collected, intermittently, while the peer that answered it had gone on to
  tear the session down. Frames that were read are now handed back at the
  driver's teardown and are drained by the next `receive_ttrpc` on the same
  session handle, before that call reports the session's own failure, so the
  answer survives the teardown that would have discarded it. A frame is only
  ever kept if it was actually read: nothing unread is retained, the socket
  is still closed, and no admission, revocation, or fencing changes.
- The admitted-effect and authority-publication frame gates no longer reach the
  audit worker from the reactor that read the frame. Both gates are awaited
  directly inside a task of the accept loop, where the ordinary request path
  instead hands the work to the dispatch pool and bridges it with `block_on` on
  a plain worker thread; removing the boundary's own `block_on` left the gates'
  refusal audits behind on the reactor. An audit append is a bounded channel
  send plus a reply receive with no deadline, and the privileged class
  backpressures on a full queue rather than dropping, so a caller waits for as
  long as the worker takes. The broker runs four reactor workers, so four
  stalled refusals stopped it accepting anything at all. Each of the nine
  refusal appends on those two gates now runs on the dispatch pool and is
  awaited in async time, exactly as the peer-authentication and stale-wire
  appends on the same task already were: a full pool queue is backpressure on
  the connection task rather than a parked thread. The appends themselves are
  unchanged, and no new blocking surface or suppression was introduced. The
  two gates also take the IPC admission limiter with `.lock().await` instead of
  spinning on `try_lock`, which was there because the typed path reaches the
  limiter from a synchronous dispatch worker with nothing else to do; on the
  reactor the same spin burns a worker for as long as the critical section
  takes to drain, and the reactor is the one thread every other connection's
  frame I/O needs. The typed path keeps the spin, and neither path holds the
  guard across a later await.
- The admitted-effect table case releases the process-wide kernel bundle guard
  as soon as its last read of the installed slot is done, rather than holding
  it across the two scratch-directory removals at the end of the body. That
  guard arrives from a function return rather than from a `.lock()` at the
  binding site, which is why `clippy::await_holding_lock` cannot see it; the
  hold that remains is the one the case's determinism depends on, and nothing
  after the release reads the slot.
- A second `SpecStore` handle on one database is now refused for
  cross-connection write-lock contention as the store's own typed, retryable
  `SpecStoreError::Locked`, instead of a raw `SQLITE_BUSY` escaping as an
  unclassified `Sqlite` error. The store already waits out its whole
  `busy_timeout` window internally; past it the request is refused, and that
  refusal said nothing a caller could act on, so a manager - or a test -
  holding two handles on one file could not tell "another writer is briefly
  ahead of me" from "this store is broken". `Locked` carries the same contract
  as `SpecStoreError::Busy`: the writer is alive, nothing was committed, and a
  retry succeeds once the other connection settles.
- `spec_store::tests::a_second_writer_queues_behind_the_outstanding_transaction`
  no longer asserts on which error a contended attempt happened to produce. It
  is deterministic now - no sleep, no retry loop, no concurrency - and it
  asserts the fence itself: a second handle on the same database is refused
  with the identity of the transaction holding the Zone, the refused candidate
  reserves no sequence and commits no desired row, and settling that
  transaction releases the Zone with a later sequence rather than a reused one.
  The assertion it replaced panicked on a `SQLITE_BUSY` the store produced
  correctly whenever contention on one file outlived the busy window, which is
  the handover case the store's own header names.
- `spec_store::tests::two_writer_handles_serialize_the_zone_sequence` carries
  the concurrent half of that test and asserts the writer serialisation as
  durable state. Twenty mutations from two handles on one Zone still have to
  produce twenty rows at generation 1 and twenty `authority.ensure` audit
  records, and now also exactly the sequences 1..=20 - none reused by two
  writers, none skipped - twenty distinct transaction identities, a Zone
  sequence of 20, an accepted cursor at 20, and fence refusals that name a
  transaction the test actually staged. A writer that raced another's fence
  fails on that state whatever its losing attempt returned along the way.

