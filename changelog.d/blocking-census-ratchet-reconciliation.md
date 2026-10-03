### Changed

- The blocking-API census baseline moves on 43 counts across 13 crates, in the
  commit that carries the work that moved them. Every one of the 43 was read at
  its call site first: 43 are recorded survivors and 0 were removed. Nothing was
  lowered, no count outside these 43 lines was touched, and no allow was added
  to buy a row.
- `packages/d2b-broker` moves `std::fs::create_dir_all` to 1,
  `std::fs::remove_file` to 1, `std::fs::set_permissions` to 1,
  `std::sync::Mutex::lock` to 2, `std::sync::RwLock::write` to 4, and its
  `clippy::disallowed_methods` suppression count to 706. The audit-scratch
  directory is created by a `cfg(test)` helper that then hands the path to
  `AuditLog::open`, which requires it to exist; the unlink-and-rebind and the
  `0660` chmod are the proof that a recycled socket inode is not the socket the
  relationship owned, and the case pins the mode so the observation is about the
  grant; and the extra lock and write-guard belong to
  `TestKernelBundleResolver::empty`, which pins the process-wide bundle slot
  empty for a case whose claim is about the absence of an injected bundle. All
  16 new suppressions carry the house reason `cfg(test) helper` and sit in the
  crate's own `cfg(test)` module.
- `packages/d2b-core` moves `std::fs::read_to_string` to 1. The read is the
  row-class table this branch's own test declares as a runfile, chosen so the Nix
  eval case and the test read one committed table instead of two careful copies;
  removing it would reintroduce exactly the drift it exists to prevent.
- `packages/d2b-provider-credential` moves `lock_api::Mutex::lock` to 2 and
  `packages/d2b-provider-volume` moves `std::sync::Mutex::lock` to 2. Both are
  scripted recording doubles in `src/test_support.rs`, compiled under
  `any(test, feature = "test-support")`, and both readers are synchronous
  accessors with no executor to move the lock to.
- `packages/d2b-provider-credential-managed-identity` moves
  `std::sync::Mutex::lock` to 6. That is one line of `tests/common/mod.rs`,
  counted once per integration-test binary that declares `mod common`: the fake
  managed-identity client arming its scripted refusal.
- `packages/d2b-provider-device-security-key` moves `lock_api::Mutex::lock` to 8
  and `packages/d2b-provider-device-usbip` moves it to 6. These are production,
  not tests, and they are the rows a `d2bd`-only survey could not speak to. Both
  retain one `Option` per resource row behind a lock, and every reader and
  writer is a synchronous `fn`: `phase` reads a state enum, and `finalize` and
  `drive` take the retained value out, drive it, and put it back. The guard is
  dropped before each of them returns and none of them awaits. Converting them
  would not remove the blocking surface, it would move it: a
  `tokio::sync::Mutex` would have to be locked with `.await` inside the
  synchronous effect-phase methods and would then be held across the very drive
  call the current take-then-put-back shape exists to keep outside the guard.
- `packages/d2b-provider-guest` moves `std::sync::Mutex::lock` to 20. Sixteen
  are production: the `GuestTargetContract` admission ledger behind
  `TargetAdmissionScope` and the `ContractFencedChannel` admit helper. Each is
  one bounded critical section that clones the target and decides, and
  `ContractFencedChannel::admit` says so in its own doc comment - the decision is
  taken in its own section so no guard is held across the request round trip.
  The synchronous lock is also load-bearing for
  `impl fmt::Debug for GuestTargetService`, which reads `scope.zone()` and
  cannot await. The other four are the recording effect's recorder locks in
  `tests/graph_target_contract.rs`, each dropped at the end of its statement.
- `packages/d2b-provider-endpoint` moves `std::fs::create_dir_all`,
  `std::fs::metadata`, `std::fs::set_permissions`,
  `std::process::Command::output` and `std::sync::Mutex::lock` to 1, 1, 1, 1 and
  5. Every one of them is in `tests/endpoint_delivery.rs`: the endpoint tree is
  built in the `0700` posture a correct grant has to survive rather than in one
  that would hand listing back, `getfacl` is the lane's honest tool limit, and
  the locks are the harness's requeue and sent/refusal recorders, which the trait
  documents as synchronous so the async dispatch never takes a `std::sync`
  guard itself.
- `packages/d2b-resource-api` moves `std::fs::create_dir_all`,
  `std::fs::read_dir`, `std::fs::remove_dir_all`, `std::fs::remove_file`,
  `std::fs::set_permissions`, `std::fs::write` and
  `std::process::Command::output` to 2, 1, 2, 1, 1, 1 and 2. All of it is
  `tests/external_seals.rs`: the compile-fail seal needs a toolchain-keyed
  scratch tree, a selective `cfg(test)` rustc wrapper it writes and marks
  executable, and the cargo fingerprint directory it must clear so the seal
  cannot be satisfied by an absent compile.
- `packages/d2bd` moves 14 counts: `std::fs::read_to_string` to 2,
  `std::fs::remove_dir_all` to 1, `std::fs::remove_file` to 4,
  `std::fs::set_permissions` to 2, `std::fs::write` to 3,
  `std::io::Read::read_to_end` to 2, `std::process::Child::wait` to 3,
  `std::process::Command::spawn` to 3, `std::process::Command::status` to 1,
  `std::sync::Mutex::lock` to 31, `std::sync::RwLock::read` to 2,
  `std::sync::RwLock::write` to 2, `std::thread::JoinHandle::join` to 7 and
  `std::thread::sleep` to 2. Of the 56 distinct call sites behind those counts,
  47 are `cfg(test)` fixtures - the PipeWire stub scripts and their tempdirs,
  the pidfd table's `/dev/null` handles, the fake broker peer and its socket
  teardown, the recording gates in the authority and zone-acceptance tests. The
  9 production sites are two deliberate bounded blocking boundaries. The
  `std::process` lifecycle in `AdmittedPipeWireHostController::run` polls a
  child against a five second deadline, kills on overrun and joins its reader
  thread, and it runs on the dedicated `d2b-conn` thread the accept loop spawns
  per connection rather than on a reactor thread. The `AcceptedLimitsHolder`
  `RwLock` clones into and assigns out of the cell, and `stored` and `store` drop
  the guard inside the same function.
- `packages/xtask` moves `std::fs::read_to_string` to 15 and
  `std::fs::remove_dir_all` to 2. The two reads are the census tool reading its
  own committed baseline inside `check_against_baseline`, on the CLI-only
  synchronous path that already carries an `CLI-only path` allow; the removal is
  a generated-fixture `Drop` in that module's test module. Each is counted once
  per target that compiles it, which is why a single new call site moves a count
  by two.
- `packages/d2b-resource-runtime` moves its suppression count to 141 and
  `packages/d2b-session` to 51. All 12 new suppressions carry
  `cfg(test) helper`: nine on restart-recovery integration tests, two in the
  `cfg(test)` module of `src/spec_store.rs`, and one on the admission test that
  proves a ttrpc response read before the peer's close is still delivered.
- No row reconciled here is a lock held across an `.await`. The workspace
  clippy run the census itself performs carries
  `-W clippy::await_holding_lock -W clippy::await_holding_refcell_ref` and
  reports neither, on this tree or in any crate of it, and every production lock
  site named above was then read by hand: the admission and realization locks
  are synchronous functions whose guards drop before the function returns, the
  daemon's own blocking work runs on the dedicated `d2b-conn` thread, and the
  only helper in these crates that returns a guard - the Guest contract test
  harness - is called from a block with no await in it.