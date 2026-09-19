# Over-engineering audit record

This page records what the over-engineering audits of the provider families and
the shared runtime found, and what became of every finding. It exists because
the verdicts lived only in the auditing session: a reader could see the code
that changed, but not which findings were applied, which were refused, and why
the refused ones were refused. The refusals are the part a diff cannot show.

## What was audited, and when

Five read-only audits ran over the restructure's provider families - policy and
the identity/control-plane crates, guest and workload lifecycle, process and
activation/credential/telemetry, the shell/volume/transport/device/audio/display
crates, and the shared runtime, toolkit, and contract layer. A sixth read-only
audit covered the daemon's composition modules for resource logic that belongs
in a provider, in a declaration, or nowhere. The audits read revision
`218b7eb46`; the accepted improvements landed on this branch after it, and the
daemon composition deletions landed last. The tree state this record was
verified against is `515cbf610`.

The audit reports are session artifacts, not files in this tree. Every row
below therefore carries evidence a reader can check from the repository: a
path and line at the head named above, a commit sha, and for deletions the
commit that carries the removal. Line estimates in the reports are not
repeated as facts here except where a commit's own diff confirms them.

## How to check a row

```console
$ git -C .worktrees/provider-per-crate log --oneline 218b7eb46..515cbf610
$ git -C .worktrees/provider-per-crate show --stat <sha>
$ git -C .worktrees/provider-per-crate show <sha> -- <path>
```

A verdict means:

- **applied** - the finding's deletion or simplification is in the tree, and
  the commit named carries it.
- **partial** - part of the finding landed; the surviving part is named.
- **refused** - the target is deliberately kept, with the reason recorded in
  the tree, in the commit message, or in the dossier the finding argues
  against.
- **not applied** - the finding was outside the improvements the pass actually
  ran; the target is unchanged and no refusal reason was recorded per item.

Per-crate compile and test results are recorded in the lane reports and commit
messages; this record re-verified tree state, commit contents, and cited
paths, not the gates.

## Counts

| Family | Findings | Applied | Partial | Refused | Not applied | Net lines removed (lane-reported) |
| --- | --- | --- | --- | --- | --- | --- |
| policy / identity / control plane | 10 | 4 | 4 | 2 | 0 | ~7,758 |
| guest / workload lifecycle | 101 | 69 | 5 | 25 | 2 | ~7,625 |
| process / activation / credential / telemetry | 15 | 6 | 4 | 5 | 0 | ~4,840 |
| shell / volume / transport / device / audio / display | 88 | 56 | 9 | 21 | 2 | ~25,524 |
| shared runtime / toolkit / contracts | 28 | 4 | 4 | 1 | 19 | ~3,859 |
| **Total** | **242** | **139** | **26** | **54** | **23** | **~49,600** |

Three commits carry content another lane staged (the worktree index is shared):
`baf1f7a0c` carries the shared-crate deletions under a policy-family message,
`ec1ed0099` carries the process-systemd deletions under the toolkit message,
and `b3c9814b6` carries qemu-media and supervisor deletions. The content is
intact; only the attribution is crossed, so a row's commit may not be the
commit whose subject names it.

## Policy / identity / control-plane family

Ten findings over the crates that declare the policy and control-plane resource
types, plus the shared declaration vocabulary the fixes created. Carried by
`c67997af1`, `0a521e8a9`, `fe0156015`, `3fe045406`, `eb6a9a684`, `6cb07fa8b`,
`d1fd570dc`, `232ecd287`, `e5a6e091a`, `6baf671cf`, `093f39e5a`, `872121bf4`.

| # | Finding | Verdict | Commit(s) | Evidence |
| --- | --- | --- | --- | --- |
| 1 | Ten name-substituted copies of one metadata driver | applied | `c67997af1 0a521e8a9 fe0156015 3fe045406 eb6a9a684 6cb07fa8b d1fd570dc` | `packages/d2b-resource-runtime/src/metadata.rs` (535 lines) is the one driver; `packages/d2b-resource-types/src/metadata.rs:39` builds each type's declaration. The ten crates' `src/driver.rs` are 20-22 lines each (278 at the audit revision); the shared driver serves the eleven declaration-only types. |
| 2 | Ten copies of the same registration test | applied | `same as 1` | `packages/d2b-resource-types/src/metadata.rs:72` `assert_metadata_registration`; each crate's `tests/registration.rs` is now 13-17 lines that call it. The per-crate file stays because the crate-layout policy requires the path. |
| 3 | A second ZoneLink enrollment-and-session state machine inside the provider crate | partial | `232ecd287` | Only the callerless half left: `ZoneLinkKeyPolicy::new` and the MIN/MAX cryptoperiod bounds (-45 lines). The machine itself remains in `packages/d2b-provider-zone-link/src/zone_links.rs`; no refusal reason is recorded in the tree beyond the pass boundary (the merge needs `d2b-bus` and `d2bd`). |
| 4 | system-core modules with no caller in the tree | applied | `e5a6e091a 6baf671cf` | `packages/d2b-provider-system-core/src/` now holds only `error.rs`, `host.rs`, `lib.rs`, `ownership.rs`, `testing.rs`, `user.rs`; 17 files / -1,624 lines in `e5a6e091a`. |
| 5 | Duplicate Host/User handler-status emitter and its readiness checker | partial | `e5a6e091a` | The system-core copy (`handler_status.rs`) is gone with finding 4. The zone-side copy stays: `packages/d2b-provider-zone/src/zone_status.rs:7` `emit_handler_status` is called by the live `SystemCoreStatusEmitter` (`zone_status.rs:149`), constructed at `packages/d2bd/src/resource_runtime.rs:3216,3616,4957`. The report's claim that neither is called in production does not hold. |
| 6 | Single-product wrappers in the Provider and config-nixos crates | partial | `093f39e5a` | `ProviderDriverStatus` is a struct (`packages/d2b-provider-provider/src/driver.rs:152`). `ProviderDriverArgs` (driver.rs:220) and `decode_document` (`config-nixos/src/service.rs:332`) stay; `ConfigNixosClient` has a live caller at `packages/d2bd/src/composition.rs:11508`, refuting the report's no-caller claim. |
| 7 | The uid-to-UUIDv4 renderer hand-rolled in eleven places | partial | `6d6bd67d7 8667ed18c` | `ResourceUid::from_bytes` at `packages/d2b-contracts/src/identity.rs:621`; three in-scope call sites migrated (`wayland-policy/src/interaction.rs`, `volume-binding/src/driver.rs`, `volume/src/driver.rs`). Twelve call sites remain in `d2bd`, `d2bd-runtime`, `d2b-broker`, and provider crates. |
| 8 | `emit_handler_status` duplicating the emitter's own mandatory pair | refused | `-` | Same live-caller evidence as finding 5; deleting the helper would change the status a live emitter publishes. No commit; `packages/d2b-provider-zone/src/zone_status.rs:101-152` is unchanged. |
| 9 | `integration/*.rs` and README scaffolds in ten metadata crates | refused | `-` | Kept: `packages/xtask/src/provider_crate_policy.rs` requires an `integration/*.rs` for every crate not on the README-only ratchet. Example at `packages/d2b-provider-role-binding/integration/`. |
| 10 | Unused `d2b-contracts-resource` dependency in seven crates | applied | `fe0156015 3fe045406 eb6a9a684 6cb07fa8b d1fd570dc` | `role-binding`, `operation`, `quota`, `emergency-policy`, `resource-import`, `resource-export`, `seccomp-profile`: zero references in `Cargo.toml` and BUILD dep lists at HEAD. |

**What was kept, and why**

- The full ZoneLink enrollment-machine merge (finding 3) is refused: the merge
  spans `d2b-bus` and `d2bd`, which the pass did not own. Only the callerless
  half - `ZoneLinkKeyPolicy::new` and its frozen MIN/MAX bounds - was removed
  (`232ecd287`). The child-side planner in
  `packages/d2b-provider-zone-link/src/zone_links.rs` still owns the durable
  record, route-admission dedup, and cursor adoption, which the bus machine
  does not model.
- The zone-side handler-status helper (findings 5 and 8) is kept: the audit
  called it callerless, but the live `SystemCoreStatusEmitter` calls it, and
  that emitter is constructed by the daemon at
  `packages/d2bd/src/resource_runtime.rs:3216,3616,4957`.
- The `integration/*.rs` scaffolds (finding 9) are policy-required paths: the
  crate-layout check in `packages/xtask/src/provider_crate_policy.rs` demands
  one for every provider crate not on its README-only ratchet.
- `ConfigNixosClient` (part of finding 6) has a live caller at
  `packages/d2bd/src/composition.rs:11508`; see Discrepancies.


## Guest / workload-lifecycle family

One hundred and one findings over eight crates: Cloud Hypervisor, qemu-media,
the guest family driver, the supervisor, azure-container-apps,
azure-virtual-machine, endpoint, and four cross-crate classes. Carried by
`aa1aa13b1`, `92bbee807`, `4422fe6ab`, `4bd85b3d5`, `366791c80`,
`521f70589`, `7814f7bc1`, `de531b9bb`, `34822957a`; the lane reports a net
-7,625 lines across 95 files.

| # | Finding | Verdict | Commit(s) | Evidence |
| --- | --- | --- | --- | --- |
| 1 | CH: guest_local.rs seed/watch/session DTO family and GuestLocalController | applied | `92bbee807` | `packages/d2b-provider-guest-cloud-hypervisor/src/guest_local.rs` shrank 1,803 -> 180 lines (the `GUEST_SEED_RESOURCE_TYPES` const and `GuestControlEndpoint` stay); the DTO/controller halves and 10 tests are gone. |
| 2 | CH: adoption.rs near-verbatim copy of the qemu-media file | partial | `92bbee807` | The copy, its integration test file, and the property test are gone; `ProcessAdoptionStatus` (16 lines) stays because `packages/d2bd/src/resource_runtime.rs` reads it. File is 143 -> 16 lines. |
| 3 | CH: health_check_test.rs rebuilds the same wiring 10 times | applied | `92bbee807` | `tests/health_check_test.rs` is deleted; the remaining suites keep their coverage. |
| 4 | CH: identical test harness copy-pasted across test binaries | partial | `92bbee807` | `tests/common/mod.rs` now exists and is shared by `controller.rs` and `reconcile_state_machine_test.rs`; `redaction_test.rs` keeps its own fixture. |
| 5 | CH: map_wire_commit_response + committed_child_from_wire + owner_ref_from_canonical_json have zero callers | applied | `92bbee807` | Deleted from `src/identity.rs` with the lib re-export. |
| 6 | CH: CloudHypervisorGuestSettings, ConsoleType, valid_token unconstructed | applied | `92bbee807` | Deleted from `src/config.rs` and `src/lib.rs`; `CloudHypervisorConfig` stays. |
| 7 | CH: fd10 bootstrap handshake re-implemented locally | refused | `-` | The toolkit's fd10 entry points the finding points at are private; the only public entry is a different supervised flow. `controller_session.rs` keeps its handshake and assignment-stream loop. |
| 8 | CH: nix/tests/default.nix repeats evalModules boilerplate in 10 cases | applied | `92bbee807` | Replaced by one `mkCase` helper; the eight cases were proved byte-identical to the previous JSON before deletion. |
| 9 | CH: BUILD.bazel repeats the same test tail for 9 rust_test targets | not applied | `4422fe6ab` | The first attempt defined a function in a BUILD file, which Bazel rejects at analysis time and which dropped a crate dep; it was reverted and the targets restored inline. The commit keeps the targets explicit; the line saving did not materialize. |
| 10 | CH: AttachmentRef and BootstrapGraph.attachments are write-only | refused | `-` | The deletion needs an edit at `packages/d2bd/src/resource_runtime.rs:6207` (another lane's file); the field and every `Vec::new()` call site survive. |
| 11 | CH: tests/state_status_test.rs spends 5 rows on the same is_exact chain | refused | `-` | The five rows exercise distinct conjuncts of the chain; shrinking them would drop coverage, so they stay. |
| 12 | CH: validate_commit_response duplicates map_commit_response | applied | `92bbee807` | `controller.rs` now calls `identity::map_commit_response`. Disclosed log-only difference: a strict-subset commit response can now emit the pre-existing warn line; both old arms returned the same pending outcome. |
| 13 | CH: redaction_test.rs builds the same descriptor twice and carries its own verifier | applied | `92bbee807` | Fixture shared. |
| 14 | CH: two single-variant error enums | applied | `92bbee807` | `CloudHypervisorConfigError::Invalid` and `BootstrapGraphError::InvalidReference` are gone. |
| 15 | CH: valid_digest duplicated in two modules | applied | `92bbee807` | Both copies deleted in favor of the schema fingerprint parse. |
| 16 | CH: nix self-assert (guestSetupDescriptors == projection of itself) | applied | `92bbee807` | `projectedPrivateDescriptors` and the assertion are gone from `nix/default.nix`. |
| 17 | CH: pending_after_batch only forwards args | applied | `92bbee807` | Inlined at the five call sites. |
| 18 | CH: GuestSessionEvidenceProbe trait has no implementation | applied | `92bbee807` | Deleted from `src/health.rs` and `src/lib.rs`. |
| 19 | CH: session_generation_is_fresh read only by a test | applied | `92bbee807` | Deleted from `src/shutdown.rs`. |
| 20 | CH: CLOUD_HYPERVISOR_IMPLEMENTATION_ID, CONTROLLER_BINARY have no reader | applied | `92bbee807` | Deleted from `src/lib.rs`. |
| 21 | qemu: whole qemu_argv module called only by its own tests | applied | `4bd85b3d5` | `src/qemu_argv.rs` deleted (257 lines) with its re-exports. |
| 22 | qemu: entire media-watch subsystem has no production caller | applied | `4bd85b3d5` | `src/controller/media_watch.rs` deleted; d2bd derives `media_ready` from dependency phases. |
| 23 | qemu: RuntimeState observation map and constant methods | applied | `4bd85b3d5` | `src/state.rs` deleted; the invariant stays in the descriptor. |
| 24 | qemu: TapLaunchRouter/NetworkLaunchEvent/TapAttachment record an effect nobody replays | applied | `4bd85b3d5` | Deleted with `src/controller/network.rs`. |
| 25 | qemu: hand-written Deserialize + 14 default fns for GuestProviderSpecSettings | refused | `-` | The hand-written deserializer is the live admission gate (d2bd parses and discards, so a derive would drop validation); `types/guest.rs` keeps it. |
| 26 | qemu: display.rs has zero production callers | applied | `4bd85b3d5` | `src/controller/display.rs` deleted; display stays a plain Endpoint ref in d2bd. |
| 27 | qemu: status surface has no production path | applied | `4bd85b3d5` | `src/controller/status.rs` and the status constructors/deserializer deleted; d2bd publishes status through its own sink. |
| 28 | qemu: QMP methods no production path invokes | applied | `4bd85b3d5` | Dead methods and their only-producer variants deleted from `src/qmp/mod.rs`. Disclosed log-only difference: `negotiate` propagates the transport's own error instead of collapsing it to `GreetingTimeout`. |
| 29 | qemu: HotplugOperation/HotplugResult never constructed | applied | `4bd85b3d5` | `src/controller/hotplug.rs` deleted; `QmpSession` is used directly. |
| 30 | qemu: nix/projection.nix providerAssertions re-validates what live modules enforce | refused | `-` | The premise is refuted: the assertions are evaluated as a module and asserted by the crate's own Nix case; they stay. |
| 31 | qemu: HostGlobalAuthorityIndex + AuthorityReservation test-only second index | applied | `4bd85b3d5` | Deleted from `src/controller/device_watch.rs`; the vocabulary types stay. |
| 32 | qemu: three dead ControllerConfigProjection fields | applied | `4bd85b3d5` | Write-only fields removed from `src/config.rs`. |
| 33 | qemu: ProviderConfig hand-rolls Deserialize and a Wire copy | refused | `-` | Same live admission-gate reason as finding 25. |
| 34 | qemu: descriptor.rs contract exercised only by its own test | applied | `4bd85b3d5` | `src/descriptor.rs` deleted. |
| 35 | qemu: repr-field validation triplicated; QmpCommand::name() has no caller | applied | `4bd85b3d5` | One token validator; `name()` deleted. |
| 36 | qemu: QmpHealth tracker never probed outside tests | applied | `4bd85b3d5` | Deleted from `src/qmp/mod.rs`. |
| 37 | qemu: RuntimeVolumeSpec::validate rebuilds literals new_with_provider just built | applied | `4bd85b3d5` | One shared helper in `src/controller/volume.rs`. |
| 38 | qemu: nix/projection.nix guestPatchForZone emits a patch nobody inspects | refused | `-` | The premise is refuted: `guestPatchesByZone` is applied by `nixos-modules/bundle-zones.nix` and asserted by the crate's own Nix case; it stays. |
| 39 | qemu: GuestSpecError variants never constructed | applied | `4bd85b3d5` | Dead variants deleted from `src/types/guest.rs`. |
| 40 | qemu: ProviderDescriptor compatibility alias has no namer | applied | `4bd85b3d5` | Deleted. |
| 41 | qemu: command journal evicts with Vec::remove(0) | applied | `4bd85b3d5` | Now `VecDeque::pop_front` in `src/qmp/mod.rs`. |
| 42 | Guest: the shared-provider driver re-rolled instead of implementing SharedProviderFamily | refused | `-` | Framework-side work: the toolkit lacks three hooks this family needs (a provider_spec and a status_sink on the effect request, a children-converged-to-Pending gate, and a per-kind recover-evidence hook). `packages/d2b-provider-guest/src/driver.rs` still implements the runtime driver directly and does not depend on the toolkit. |
| 43 | Guest: GuestTargetEffect port and dispatch have no production implementor | refused | `-` | The only consumer of the target-local dispatch is `packages/d2bd/tests/guest_target_service.rs` (another lane's file); the trait and dispatch stay until the first target-local effect registers. |
| 44 | Guest: byte-identical re-rolls of toolkit helpers | refused | `-` | Reusing the toolkit's `decode_metadata`/`resource_uid`/`key_ref` changes the guest crate's public error type, which a live daemon file consumes; the 20 local lines stay and the dedup rides the refused fold. |
| 45 | Guest: never-called public items | applied | `aa1aa13b1` | `HOST_REF`, `GuestEffectOutcome::projection`, `GuestDriverStatus::phase`, `GuestSpecEnvelope::raw`, `GuestDriver::controller_generation` deleted. |
| 46 | Guest: UnavailableGuestDriverEffects port adapter | applied | `aa1aa13b1` | Deleted; the in-file test already had `ScriptedEffects`. |
| 47 | Guest: GuestRegistration.controller_ref and GuestKind::controller_ref() have no production reader | applied | `aa1aa13b1` | Deleted with the self-assert. |
| 48 | Guest: hand-written Debug impls that render only the type name | applied | `aa1aa13b1` | Deleted. |
| 49 | Guest: integration/guest_family.rs scenario declaration no target compiles | refused | `-` | Kept: the crate-layout policy requires an `integration/*.rs` for every crate not on the README-only ratchet, and this crate is not on it. |
| 50 | Guest: write-only fields (deleting, raw bytes copy) | applied | `aa1aa13b1` | Deleted. |
| 51 | Guest: GuestChildSurface is a one-implementation trait | refused | `-` | Making the request field concrete hits invariance on `ContextChildSurface`'s `Mutex<&mut ResourceContext>`; it is a restructure, not a deletion. |
| 52 | Guest: module-scope #![allow(dead_code)] | applied | `aa1aa13b1` | Removed with the two dead test helpers it hid. |
| 53 | supervisor: hand-rolled blocking executor (16 threads, deadline thread, custom waker) | refused | `-` | The deadline cannot move out of the crate: d2bd awaits launch with no timeout, tokio is dev-only here, and the conformance code would change. `src/adapter.rs` keeps the executor. |
| 54 | supervisor: hand-rolled seqpacket broker transport | refused | `-` | The d2bd-runtime transport is not semantically interchangeable (no deadlines, different error mapping) and provider crates may not depend on it. `src/broker.rs:2239-2428` survives. |
| 55 | supervisor: generic systemd seam with exactly one production implementation | refused | `-` | d2bd names the generic type at `packages/d2bd/src/process_provider_runtime.rs:59`; the seam stays. |
| 56 | supervisor: three write-only quarantine sets/fields | applied | `de531b9bb` | Deleted; ambiguity is already reported through `ProcessConformanceError::AdoptionAmbiguous`. |
| 57 | supervisor: check_user_manager chain has no caller | applied | `de531b9bb` | Trait default, forwarder, and override deleted. |
| 58 | supervisor: test-only constructors on the production backend | partial | `34822957a` | `new` and `with_socket` deleted from both backends; `with_socket_and_role` stays because its only caller is `tests/production_adapter.rs`. |
| 59 | supervisor: BrokerSystemdEffectOwner::{new,with_socket} have zero callers | applied | `34822957a` | Deleted. |
| 60 | supervisor: a test scrapes d2b-broker's source to keep an error-kind string honest | refused | `-` | No exported constant exists for the broker error kind and `d2b-contracts-broker` is out of lane; the include_str scrape and cross-crate compile_data stay. |
| 61 | supervisor: typed-identity projection hand-copied into six wire requests | applied | `34822957a` | One shared emit helper. |
| 62 | supervisor: identical bounded pending-observation ledger implemented twice | not applied | `-` | Deferred for a single writer: the dedup spans `src/broker.rs` and `src/systemd.rs`; both files were free by the end of the pass but the two copies remain. |
| 63 | supervisor: observe and probe bodies differ by one line | applied | `de531b9bb` | Shared implementation. |
| 64 | supervisor: ProviderSupervisor::wait_identity has no caller | applied | `de531b9bb` | Deleted. |
| 65 | supervisor: comment-only integration files no target compiles | refused | `-` | Kept: the crate-layout policy ratchet requires the paths. |
| 66 | supervisor: wait_pidfd_exit/wait_pidfd_observer are the same poll loop twice | refused | `-` | Measured against the finding: the shared part is nine lines and the observer needs the elapsed-vs-unreadiness split, so the saving does not exist. |
| 67 | supervisor: SystemdIdentityContext 2-field wrapper | applied | `de531b9bb` | Folded into `SystemdInvocationIdentity::new`. |
| 68 | supervisor: BrokerFrame wraps its fd table in a Mutex though single-owner | refused | `-` | Removing the `Mutex` needs a `&mut self` call and `src/systemd.rs` calls `take_fd` on an immutable frame. |
| 69 | ACA: deployment_service.rs has no production caller | applied | `366791c80` | `src/deployment_service.rs` deleted (451 lines) with its exports and test; `nix/default.nix` no longer projects an unrunnable service. |
| 70 | ACA: seven hand-written Deserialize blocks mirroring Raw twins | applied | `366791c80` | Replaced by try-from plumbing; `deny_unknown_fields` stays on the raw shapes. |
| 71 | ACA: dead public API in effects.rs | applied | `366791c80` | Deleted: lease-cleanup const, credential-scope validator, required_operations, the retry_after feature, BoxAcaFuture, implementation id. |
| 72 | ACA: config/profile re-validation three times per admission | applied | `366791c80` | Validated once. |
| 73 | ACA: dead controller members | applied | `366791c80` | Deleted. |
| 74 | ACA: unused deps | applied | `366791c80` | `d2b-contracts` dropped; `serde_json` moved to dev-dependencies; tokio feature trimmed. |
| 75 | ACA: nix/default.nix processFor template param redundant | applied | `366791c80` | Param dropped. |
| 76 | AVM: owned-VM check copy-pasted five times | applied | `521f70589` | One `verify_owned_vm` helper. Disclosed log-only difference: the checks now log resource group plus a stage field. |
| 77 | AVM: untouched accessors | partial | `521f70589` | `as_str` and `retryable` deleted; `PskExtensionPayload::{len,is_empty}` are the field's only readers and removing them trips `dead_code`. |
| 78 | AVM: BootstrapPskDelivery is a one-variant enum carried as config | refused | `-` | d2bd constructs `BootstrapPskDelivery::VmExtension` at `packages/d2bd/src/guest_effects.rs:1697`; it stays. |
| 79 | AVM: write-only state (tag_digest, vm_delete_confirmed) | applied | `521f70589` | Deleted. |
| 80 | AVM: bootstrap_svc.rs is a module for a 3-state wrapper | applied | `521f70589` | Folded into `bootstrap.rs`. |
| 81 | AVM: validate_credential_scope has zero call sites | applied | `521f70589` | Deleted. |
| 82 | AVM: idempotency.rs is a module for one 20-line fn | applied | `521f70589` | Moved next to its call sites. |
| 83 | AVM: AzureVmConfig tenant_id/client_id never read | refused | `-` | They are `deny_unknown_fields` wire fields; deleting them is a wire change. |
| 84 | AVM: AzureVmStatus::operation_digest never observed | applied | `521f70589` | Deleted. |
| 85 | AVM: serde_json is a normal dep but only tests use it | applied | `521f70589` | Moved to dev-dependencies. |
| 86 | endpoint: DeadManager + NullRequeue doubles duplicate the workspace's | refused | `-` | The runtime's recording doubles are `#[cfg(test)] pub(crate)` and cannot be used across crates; the local doubles stay. |
| 87 | endpoint: EndpointRealization variant identity consumed nowhere in production | refused | `-` | The change needs an edit at `packages/d2bd/src/resource_plane_v3.rs:1858` (another lane's file). |
| 88 | endpoint: three near-duplicate EndpointSpec test builders | applied | `7814f7bc1` | One parameterized builder; the dead one removed. |
| 89 | endpoint: EndpointDriverArgs.zone never read | refused | `-` | Needs an edit at the daemon construction site; the field stays. |
| 90 | endpoint: factory_registers_only_the_endpoint_resource_type re-asserts registration | applied | `7814f7bc1` | Deleted. |
| 91 | endpoint: EndpointSpecEnvelope wraps one live field | applied | `7814f7bc1` | Decoded straight to the canonical object. |
| 92 | endpoint: EndpointDriverErrorKind::class() has unreachable arms | applied | `7814f7bc1` | Arms removed. |
| 93 | endpoint: tautological assertions on the test's own fake vocabulary | applied | `7814f7bc1` | Deleted. |
| 94 | endpoint: ENDPOINT_TYPE_NAME duplicates the WellKnownType const | applied | `7814f7bc1` | Now derived. |
| 95 | endpoint: EndpointDriver::error ignores self | applied | `7814f7bc1` | Free function. |
| 96 | endpoint: #[derive(Clone)] on EndpointDriver unused | applied | `7814f7bc1` | Dropped. |
| 97 | endpoint: tokio time feature declared but unused | applied | `7814f7bc1` | Feature dropped. |
| 98 | cross-crate: unproduced metric/audit vocabulary across five crates | applied | `92bbee807 4bd85b3d5 366791c80 521f70589 de531b9bb` | The label-set modules were deleted with their re-exports: supervisor `metrics.rs`/`tracing.rs` (416 lines, swept into b3c9814b6 by the shared index), qemu-media `audit.rs` + telemetry span half, azure-vm `telemetry.rs`/`audit.rs`, ACA `metrics.rs`/`audit.rs`, cloud-hypervisor `metrics.rs`/`audit.rs`. |
| 99 | cross-crate: runner-contract structs read only by their own tests | applied | `92bbee807 4bd85b3d5 366791c80 521f70589` | The `*RunnerContract` struct/accessors/constructors and the tests that only pinned them were deleted; the `FINALIZER`/`REPAIR_INTERVAL_SECS` constants stay. |
| 100 | cross-crate: clock seam triplicated | partial | `521f70589` | The azure-virtual-machine half applied (it now depends on the toolkit clock seam; the crate did not already depend on the toolkit as the report assumed). The ACA half is refused: `d2bd/tests/cloud_composition.rs` imports and implements `AcaClock`. |
| 101 | cross-crate: /proc/<pid>/stat field-22 readers copied per crate | refused | `-` | No shared parser is reachable from the supervisor, and the local one classifies Z/X as gone; the copy stays. |

**What was kept, and why**

- The fold of the guest driver onto the toolkit's `SharedProviderFamily`
  (finding 42) is refused as framework-side work: the toolkit lacks the three
  hooks the family needs (a `provider_spec` and a `status_sink` on the effect
  request, a children-converged gate that can downgrade Ready to Pending, and
  a per-kind recover-evidence hook). The driver stays a direct
  `ResourceDriver` implementation and does not depend on the toolkit.
- The hand-written deserializers in qemu-media (findings 25 and 33) are kept
  as the live admission gate; a derive would drop validation because the
  daemon parses and discards.
- The two Nix findings (30, 38) are refuted on inspection: the projection
  assertions are evaluated as a module and the zone patch is applied by
  `nixos-modules/bundle-zones.nix`.
- Findings that remain for other lanes' files: Cloud Hypervisor 10 (needs
  `packages/d2bd/src/resource_runtime.rs:6207`), guest 43 (needs the daemon's
  target-service test), endpoint 87 and 89 (need daemon edits), AVM 78 (d2bd
  constructs the variant), ACA half of 100 (d2bd implements the clock).
- Coverage-preserving refusals: the five state-status rows (11), the pidfd
  poll-loop pair measured as nine shared lines with a needed split (66), the
  single-owner frame mutex that has no `&mut` call site (68), and the local
  `/proc` parser whose Z/X classification is load-bearing (101).
- Cloud Hypervisor 9 is recorded as not applied: the BUILD shrink defined a
  function in a BUILD file, which Bazel rejects at analysis time, and was
  reverted in `4422fe6ab` with the targets restored inline.


## Process / activation / credential / telemetry family

Fifteen findings over twelve crates: the process family and its two
realizations, activation, command, the three credential realizers and the base
driver, the telemetry pair, and the otel crate. Carried by `ec1ed0099`,
`27d3d0699`, `743f3a789`, `b7f0898e2`, `a93c573d1`,
`f8f0f60e4`, `2377666ff`, `11c5e5626`, `472e2f7cf`, `679dda409`,
`aeaf256d1`, `d37581f02`, `5ee5d3156`.

| # | Finding | Verdict | Commit(s) | Evidence |
| --- | --- | --- | --- | --- |
| 1 | The three Credential realization crates were one provider written three times | applied | `ec1ed0099 27d3d0699 743f3a789 b7f0898e2` | One shared module `packages/d2b-provider-toolkit/src/credential.rs` keyed on `CredentialProviderKind` (exports `authorized_service_record`, `credential_frame`, `dispatch_blocking`); the three crates keep their own binaries, provider refs, and process-level canary tests. Toolkit commit +190 lines; the three crates' audit/telemetry/controller copies deleted. |
| 2 | Every credential op implemented twice per crate (sync copies of the async bodies) | applied | `27d3d0699 743f3a789 b7f0898e2` | Sync twins deleted (managed-identity -480, entra -474, secret-service -501); each synchronous dispatch is a fail-fast guard plus one `dispatch_blocking` call, and the async half is unchanged. The three lifecycle tests that pin the fail-fast answer pass unchanged. |
| 3 | secret-service's production-dead halves | partial | `a93c573d1, b7f0898e2` | Deleted: `controller_binary_entrypoint`, never-constructed `SecretServicePortError::{Missing,Denied}`, duplicate `invariant_error()`, and the two suites that asserted other crates' behavior. Kept: the session-capability authority is live (`src/lib.rs:1290-1360` `authorize_session_for_user_locked` issues and consumes it), and the disconnect/finalize/drain lifecycle plus controller projections are declared by the crate README and the provider dossier. |
| 4 | systemd's dead modules | partial | `ec1ed0099` | Deleted: `guest_exec.rs` (614 lines, re-declared types `TtySize`/`AttachRequest` live in the shell-terminal crate), `manifest.rs` (29), and `adoption.rs` (31, the dead identity check). Kept: the modules the provider dossier's required layout names that remain live surface. Note: the dossier still names `src/adoption.rs` at `docs/specs/providers/ADR-046-provider-system-systemd.md:1295,1351,1468`, and no dossier edit landed in the pass. |
| 5 | The `EffectPortAdapter` layer in both process provider crates is never constructed | refused | `-` | Kept as the providers' declared effect-port boundary: the systemd dossier names `src/effect_port.rs` as the destination (`ADR-046-provider-system-systemd.md:1295`), the minijail dossier makes `MinijailProcessEffectPort` the sole spawn path (`ADR-046-provider-system-minijail.md:138,586-596`). `minijail/src/effect_port.rs` and `systemd/src/effect_port.rs` survive. |
| 6 | otel: 1,368 lines of Nix byte-copies of the live module graph | applied | `aeaf256d1 d37581f02 5ee5d3156` | Deleted `nix/stack.nix` (741), the stale `nix/host.nix` and `nix/guest.nix` forks, the crate aggregation entry, and the two dangling build inputs. `packages/d2b-provider-observability-otel/nix/` now holds only `projection.nix` and `tests`. |
| 7 | otel: unwired surface | refused | `-` | The provider dossier declares the realization plan (`ADR-046-provider-observability-otel.md` section 18 names `collector_bin`/`emitter_socket`/`ingress_policy`/`controller`/`service`/`binding` as destinations); nothing beyond the Nix fork was deleted. The audit itself flagged this half as a routing decision. |
| 8 | activation-nixos: three orphan subsystems | applied | `11c5e5626` | Deleted `runner.rs` (151), `diagnostics/` (110), `manifest.rs` (32), the unreachable `RetentionPlan` surface (52), `tests/runner.rs` (86), and its BUILD target (14). Live generation retention stays in Nix (`retainedGenerations`). |
| 9 | managed-identity 'backward-compatible' shadow types | applied | `27d3d0699` | `ManagedIdentityTelemetry{Operation,Outcome,Frame}` and `ManagedIdentityAudit{Operation,Outcome,Record}` deleted (~230 lines); the canary test rebuilt on the shared contract types with its assertions kept. |
| 10 | telemetry pair: mirrored drivers and self-testing tests | partial | `679dda409` | Deleted both `finalize` overrides that re-ran what the erased boundary already does, the registration asserts comparing descriptor fields to the constants they are built from, and the dead accessors (-184 net in the commit). Kept: collapsing the pair into one generic driver needs `d2b-resource-runtime` test support reachable across crates, and `DEPENDENCY_READINESS_PROVEN` is the published readiness gate the crate README records. |
| 11 | The process family depends on both of its own Providers to read two `&str`s | refused | `-` | The premise is refuted: `PROVIDER_REF` has eight non-test users across seven files, and the family driver reading the providers' own exported names is the correct layering. No lines or dependency edges removed. |
| 12 | minijail shims with no production caller | applied | `472e2f7cf` | Deleted `manifest.rs`, `sandbox_compiler.rs`, `user_ns.rs`, `ephemeral.rs`, `effect_result.rs`, `finalize.rs`, the `reconcile` dispatcher and its enums, `PlatformGate::new_for_test`, and `adoption::validate_candidate`, plus the tests that pinned only deleted code (-405 net in the commit, +5/-405). |
| 13 | command: the controller-family template | refused | `-` | Deferred: the duplicated fences and verb lists live in a shared engine that another lane was actively restructuring during the pass, so a cut would have collided. The declaration-only half of this template was in fact removed with the shared metadata driver (policy finding 1); the controller-family fence dedup remains open. |
| 14 | credential base crate: test-only and write-only surface | partial | `f8f0f60e4 2377666ff` | Deleted `CredentialDriverStatus::{phase,outcome_code}` and the test that re-asserted the ready status through the retired accessor. Kept: `CONTROLLER_PROVIDER_*_ANNOTATION` is written into live status annotations (`driver.rs:550-552`), `CredentialRevocationEvidence` has a production reader, and `integration/` is policy-required. |
| 15 | Systemic duplications worth one fix each | refused | `-` | Consolidating the `NullRequeue`/recording doubles and the duplicated `resource_uid` requires test support from `d2b-resource-runtime` reachable by provider crates, which is a cross-crate change owned elsewhere; the otel copy sits inside the refused surface. One fix did land: the shared `ResourceUid::from_bytes` constructor (shared finding A2/A9 commits and `6d6bd67d7`). |

**What was kept, and why**

- The credential realization work landed in the thin-descriptor shape the pass
  agreed: the three crates stay separate binaries with separate provider
  identities, and the shared use lives in
  `packages/d2b-provider-toolkit/src/credential.rs`.
- The `EffectPortAdapter` layer (finding 5) is the declared effect-port
  boundary named by both process dossiers; it stays.
- The otel unwired surface (finding 7) is dossier-planned work; only the Nix
  fork was removed.
- The sync collapse kept the fail-fast contention semantics exactly: each
  synchronous entry point is a guard followed by the blocking bridge over the
  async body, and the tests that pin the fail-fast answer were not relaxed.
- One genuine defect was found and fixed while landing the collapse: a bare
  first poll panicked outside a Tokio runtime, so the bridge now enters a
  current-thread runtime (`f8f0f60e4`), with a guard test that arms a timer
  while polling.

**A report and the tree disagreed, and the tree is recorded**

Finding 4 says the systemd modules named by the provider dossier's required
layout stay. `src/adoption.rs` is in that list and is gone (deleted in
`ec1ed0099`, 31 lines, a dead identity-check helper). No dossier edit landed,
so `docs/specs/providers/ADR-046-provider-system-systemd.md:1295,1351,1468`
still names the deleted module. The other dossier-named modules -
`controller.rs`, `launch.rs`, `effect_port.rs`, `sandbox.rs`, `drain.rs`,
`audit.rs`, `metrics.rs`, `error.rs`, `lifecycle.rs` - remain.


## Shell / volume / transport / device / audio / display family

Eighty-eight findings over the shell, volume, transport, device, audio,
display, clipboard, notification, host, and user crates, plus the build and
packaging surface. Carried by `b35250cf0`, `8667ed18c`, `31a119277`,
`62b577c08`, `54b7948f2`, `14929e51a`, `f99db5a95`, `589c5d20e`,
`04257b6c9`, `01de0fd71`; the lane reports +611/-26,135 (net -25,524) across
214 files. The per-crate detail rows in the report are folded into the
findings below where the lane's record covered them.

| # | Finding | Verdict | Commit(s) | Evidence |
| --- | --- | --- | --- | --- |
| 1 | Cross-cutting: hand-rolled UUID rendering, 15+ copies | applied | `6d6bd67d7 8667ed18c` | `ResourceUid::from_bytes` at `packages/d2b-contracts/src/identity.rs:621`; three in-scope call sites migrated (`wayland-policy/src/interaction.rs`, `volume-binding/src/driver.rs`, `volume/src/driver.rs`); twelve remain in `d2b-broker`, `d2b-provider-credential`, `-guest`, `-process`, `-provider`, `d2bd-runtime`, and `d2bd`. |
| 2 | Cross-cutting: six copies of the same 9-verb list | applied | `8667ed18c` | `CONVERTED_TYPE_VERBS` in `packages/d2b-resource-types/src/descriptor.rs`; `DEVICE_VERBS`, `HOST_VERBS`, `USER_VERBS`, `VOLUME_VERBS`, `BINDING_VERBS` migrated. `TELEMETRY_BINDING_VERBS` left outside the family and reported. |
| 3 | Cross-cutting: per-type declaration boilerplate in the interaction family (approx. 250 lines claimed) | refused | `-` | Measured about 200 lines across six crates, of which only about 70 is safely removable; the `*_spec_decoder()` wrappers are consumed by each crate's registration test and the `*Driver/*Factory` aliases are public surface. |
| 4 | Cross-cutting: dead per-type constants no caller reads | partial | `8667ed18c` | Six `*_CONTROLLER_REF` constants and their six re-export arms deleted. The paired `*_RESYNC` constants are refused: each is the return value of its own `InteractionType::resync()`, called by the engine at `d2b-provider-wayland-policy/src/interaction.rs:795`. |
| 5 | Cross-cutting: spec_ref duplicated in four crates | refused | `-` | No importable shared pointer-ref parser exists in scope: the interaction engine exposes only `key_ref`/`owned_child_ensure`/`resource_uid`, and the toolkit helper is metadata-specific and off-limits. The four copies stay. |
| 6 | Cross-cutting: host and user drivers are ~60% the same file | refused | `-` | Measured 922 (host) vs 845 (user) lines, 281 differing after subject normalization; the effect ports, observation report types, failure kinds, and providerRef fence differ, and the only shared home (the toolkit) was off-limits. A host-to-user edge would be a new cross-family dependency. |
| 7 | transport-unix has no caller in the repo | refused | `-` | Declared provider: `packages/xtask/src/provider_crate_policy.rs:275-279` pins `src/portal.rs` and `tests/transport.rs` with the dossier path; `nixos-modules/provider-runtime-contracts.nix:213` lists `Provider/transport-unix`; the committed binding schema exists. Nothing deleted. |
| 8 | transport-vsock: ~2,000 lines with zero workspace dependents | refused | `-` | Same declaration class as finding 7: policy matrix row, runtime provider row with settings assertions, dossier, and committed schema all name it. Zero in-tree callers remain the reason it looks scaffolded, but it is a declared artifact. |
| 9 | transport-azure-relay: ~580 lines of unused surface | applied | `f99db5a95` | Sealed-credential write half, `RelayTransportService` and handles, `src/reconnect.rs` with its backoff loop, the duplicate `mint_sas`, the unbound-acquire default, and `src/audit.rs`/`src/metrics.rs` deleted. `GatewayGuestCredentialSource` kept: both variants are constructed by live or declared constructors (`packages/d2bd/src/composition.rs:4422`). |
| 10 | Volume: DesiredBindingChild carries five fields nobody reads | applied | `8667ed18c` | `DesiredBindingChild` is now `{name, spec}`; the five unread fields and the unused import are gone. |
| 11 | Volume: volume-binding uid_hex and resource_uid side by side | partial | `8667ed18c` | `resource_uid` migrated to the shared constructor; `uid_hex` refused because it renders plain 32-hex inside failure diagnostic text (`driver.rs:504`), not a UUID, so replacing it would change an operator-visible detail. |
| 12 | GPU: host-global authority index and restart-recovery machinery | applied | `31a119277` | Deleted the authority index/recovery/adoption machinery, the legacy effect port and controller upgrade path, `src/probe.rs`, `src/status.rs`+`audit.rs`+`telemetry.rs`, `src/production.rs`, `src/arbitration.rs`, `src/descriptor.rs` with the worker forwarders, and `src/wire.rs` with its snapshot test. `GpuController::adopt_lifecycle` kept because a policy-pinned test exercises it; `nix/default.nix` left to its owning lane. |
| 13 | GPU: legacy effect path and upgrade/runner-contract machinery | applied | `31a119277` | Deleted the authority index/recovery/adoption machinery, the legacy effect port and controller upgrade path, `src/probe.rs`, `src/status.rs`+`audit.rs`+`telemetry.rs`, `src/production.rs`, `src/arbitration.rs`, `src/descriptor.rs` with the worker forwarders, and `src/wire.rs` with its snapshot test. `GpuController::adopt_lifecycle` kept because a policy-pinned test exercises it; `nix/default.nix` left to its owning lane. |
| 14 | GPU: src/probe.rs nothing probes DRM through | applied | `31a119277` | Deleted the authority index/recovery/adoption machinery, the legacy effect port and controller upgrade path, `src/probe.rs`, `src/status.rs`+`audit.rs`+`telemetry.rs`, `src/production.rs`, `src/arbitration.rs`, `src/descriptor.rs` with the worker forwarders, and `src/wire.rs` with its snapshot test. `GpuController::adopt_lifecycle` kept because a policy-pinned test exercises it; `nix/default.nix` left to its owning lane. |
| 15 | GPU: status/audit/telemetry trio authored by d2bd | applied | `31a119277` | Deleted the authority index/recovery/adoption machinery, the legacy effect port and controller upgrade path, `src/probe.rs`, `src/status.rs`+`audit.rs`+`telemetry.rs`, `src/production.rs`, `src/arbitration.rs`, `src/descriptor.rs` with the worker forwarders, and `src/wire.rs` with its snapshot test. `GpuController::adopt_lifecycle` kept because a policy-pinned test exercises it; `nix/default.nix` left to its owning lane. |
| 16 | GPU: src/production.rs never constructed | applied | `31a119277` | Deleted the authority index/recovery/adoption machinery, the legacy effect port and controller upgrade path, `src/probe.rs`, `src/status.rs`+`audit.rs`+`telemetry.rs`, `src/production.rs`, `src/arbitration.rs`, `src/descriptor.rs` with the worker forwarders, and `src/wire.rs` with its snapshot test. `GpuController::adopt_lifecycle` kept because a policy-pinned test exercises it; `nix/default.nix` left to its owning lane. |
| 17 | GPU: src/arbitration.rs second claim arbiter | applied | `31a119277` | Deleted the authority index/recovery/adoption machinery, the legacy effect port and controller upgrade path, `src/probe.rs`, `src/status.rs`+`audit.rs`+`telemetry.rs`, `src/production.rs`, `src/arbitration.rs`, `src/descriptor.rs` with the worker forwarders, and `src/wire.rs` with its snapshot test. `GpuController::adopt_lifecycle` kept because a policy-pinned test exercises it; `nix/default.nix` left to its owning lane. |
| 18 | GPU: worker specs carrying unread fields and forwarders | applied | `31a119277` | Deleted the authority index/recovery/adoption machinery, the legacy effect port and controller upgrade path, `src/probe.rs`, `src/status.rs`+`audit.rs`+`telemetry.rs`, `src/production.rs`, `src/arbitration.rs`, `src/descriptor.rs` with the worker forwarders, and `src/wire.rs` with its snapshot test. `GpuController::adopt_lifecycle` kept because a policy-pinned test exercises it; `nix/default.nix` left to its owning lane. |
| 19 | GPU: duplicate wire snapshot | applied | `31a119277` | Deleted the authority index/recovery/adoption machinery, the legacy effect port and controller upgrade path, `src/probe.rs`, `src/status.rs`+`audit.rs`+`telemetry.rs`, `src/production.rs`, `src/arbitration.rs`, `src/descriptor.rs` with the worker forwarders, and `src/wire.rs` with its snapshot test. `GpuController::adopt_lifecycle` kept because a policy-pinned test exercises it; `nix/default.nix` left to its owning lane. |
| 20 | GPU: nix/default.nix row merge already supplies defaults | applied | `31a119277` | Deleted the authority index/recovery/adoption machinery, the legacy effect port and controller upgrade path, `src/probe.rs`, `src/status.rs`+`audit.rs`+`telemetry.rs`, `src/production.rs`, `src/arbitration.rs`, `src/descriptor.rs` with the worker forwarders, and `src/wire.rs` with its snapshot test. `GpuController::adopt_lifecycle` kept because a policy-pinned test exercises it; `nix/default.nix` left to its owning lane. |
| 21 | security-key: semantic descriptor machinery | applied | `04257b6c9` | Deleted `src/descriptor.rs`, the Binding-admission branch, `src/session_ring.rs` with its pushes, `src/effect_port.rs` with `observe_inventory`, `src/cid.rs` with the blocking framers and zero-referent command consts, and the unread vsock/lease constants with their self-asserting shim. Ring capacity stays admitted through the pre-existing error; framing tests were re-pointed to the surviving async framers. |
| 22 | security-key: Binding-admission branch with zero daemon callers | applied | `04257b6c9` | Deleted `src/descriptor.rs`, the Binding-admission branch, `src/session_ring.rs` with its pushes, `src/effect_port.rs` with `observe_inventory`, `src/cid.rs` with the blocking framers and zero-referent command consts, and the unread vsock/lease constants with their self-asserting shim. Ring capacity stays admitted through the pre-existing error; framing tests were re-pointed to the surviving async framers. |
| 23 | security-key: src/session_ring.rs and its pushes | applied | `04257b6c9` | Deleted `src/descriptor.rs`, the Binding-admission branch, `src/session_ring.rs` with its pushes, `src/effect_port.rs` with `observe_inventory`, `src/cid.rs` with the blocking framers and zero-referent command consts, and the unread vsock/lease constants with their self-asserting shim. Ring capacity stays admitted through the pre-existing error; framing tests were re-pointed to the surviving async framers. |
| 24 | security-key: src/effect_port.rs with no implementor | applied | `04257b6c9` | Deleted `src/descriptor.rs`, the Binding-admission branch, `src/session_ring.rs` with its pushes, `src/effect_port.rs` with `observe_inventory`, `src/cid.rs` with the blocking framers and zero-referent command consts, and the unread vsock/lease constants with their self-asserting shim. Ring capacity stays admitted through the pre-existing error; framing tests were re-pointed to the surviving async framers. |
| 25 | security-key: second CID translator and relay leftovers | applied | `04257b6c9` | Deleted `src/descriptor.rs`, the Binding-admission branch, `src/session_ring.rs` with its pushes, `src/effect_port.rs` with `observe_inventory`, `src/cid.rs` with the blocking framers and zero-referent command consts, and the unread vsock/lease constants with their self-asserting shim. Ring capacity stays admitted through the pre-existing error; framing tests were re-pointed to the surviving async framers. |
| 26 | security-key: constants with no production referent | applied | `04257b6c9` | Deleted `src/descriptor.rs`, the Binding-admission branch, `src/session_ring.rs` with its pushes, `src/effect_port.rs` with `observe_inventory`, `src/cid.rs` with the blocking framers and zero-referent command consts, and the unread vsock/lease constants with their self-asserting shim. Ring capacity stays admitted through the pre-existing error; framing tests were re-pointed to the surviving async framers. |
| 27 | display: labs/window-chrome/proxy copy | refused | `-` | Not deleted: `labs/window-chrome/proxy` is a standalone workspace (its own Cargo.toml, not built by the root workspace) whose disposition ADR 0047 owns; the pass left it. |
| 28 | display: legacy border-decoration renderer | applied | `62b577c08` | Deleted the legacy border-decoration renderer and the unreachable subsurface tracking, `src/wayland_proxy_argv.rs`, `nix/ui-colors.nix`+`nix/niri-vm-borders.nix`, `src/metrics.rs`+`src/audit.rs`+`tests/redaction.rs`, `src/portal.rs`, the duplicated proxy-readiness protocol, `src/descriptor.rs`, the bundle half of `PrincipalPool`, and the stale `ProxyProcessTemplate` bincs. The global catalog and `debug_logging` are refused (a Nix assertion pins the catalog; the field is pinned wire). `labs/window-chrome/proxy` left in place (ADR 0047). |
| 29 | display: src/wayland_proxy_argv.rs has no caller | applied | `62b577c08` | Deleted the legacy border-decoration renderer and the unreachable subsurface tracking, `src/wayland_proxy_argv.rs`, `nix/ui-colors.nix`+`nix/niri-vm-borders.nix`, `src/metrics.rs`+`src/audit.rs`+`tests/redaction.rs`, `src/portal.rs`, the duplicated proxy-readiness protocol, `src/descriptor.rs`, the bundle half of `PrincipalPool`, and the stale `ProxyProcessTemplate` bincs. The global catalog and `debug_logging` are refused (a Nix assertion pins the catalog; the field is pinned wire). `labs/window-chrome/proxy` left in place (ADR 0047). |
| 30 | display: nix/ui-colors.nix + nix/niri-vm-borders.nix unimported | applied | `62b577c08` | Deleted the legacy border-decoration renderer and the unreachable subsurface tracking, `src/wayland_proxy_argv.rs`, `nix/ui-colors.nix`+`nix/niri-vm-borders.nix`, `src/metrics.rs`+`src/audit.rs`+`tests/redaction.rs`, `src/portal.rs`, the duplicated proxy-readiness protocol, `src/descriptor.rs`, the bundle half of `PrincipalPool`, and the stale `ProxyProcessTemplate` bincs. The global catalog and `debug_logging` are refused (a Nix assertion pins the catalog; the field is pinned wire). `labs/window-chrome/proxy` left in place (ADR 0047). |
| 31 | display: src/metrics.rs + src/audit.rs built only by tests | applied | `62b577c08` | Deleted the legacy border-decoration renderer and the unreachable subsurface tracking, `src/wayland_proxy_argv.rs`, `nix/ui-colors.nix`+`nix/niri-vm-borders.nix`, `src/metrics.rs`+`src/audit.rs`+`tests/redaction.rs`, `src/portal.rs`, the duplicated proxy-readiness protocol, `src/descriptor.rs`, the bundle half of `PrincipalPool`, and the stale `ProxyProcessTemplate` bincs. The global catalog and `debug_logging` are refused (a Nix assertion pins the catalog; the field is pinned wire). `labs/window-chrome/proxy` left in place (ADR 0047). |
| 32 | display: src/portal.rs has no component | applied | `62b577c08` | Deleted the legacy border-decoration renderer and the unreachable subsurface tracking, `src/wayland_proxy_argv.rs`, `nix/ui-colors.nix`+`nix/niri-vm-borders.nix`, `src/metrics.rs`+`src/audit.rs`+`tests/redaction.rs`, `src/portal.rs`, the duplicated proxy-readiness protocol, `src/descriptor.rs`, the bundle half of `PrincipalPool`, and the stale `ProxyProcessTemplate` bincs. The global catalog and `debug_logging` are refused (a Nix assertion pins the catalog; the field is pinned wire). `labs/window-chrome/proxy` left in place (ADR 0047). |
| 33 | display: duplicated proxy-readiness protocol | applied | `62b577c08` | Deleted the legacy border-decoration renderer and the unreachable subsurface tracking, `src/wayland_proxy_argv.rs`, `nix/ui-colors.nix`+`nix/niri-vm-borders.nix`, `src/metrics.rs`+`src/audit.rs`+`tests/redaction.rs`, `src/portal.rs`, the duplicated proxy-readiness protocol, `src/descriptor.rs`, the bundle half of `PrincipalPool`, and the stale `ProxyProcessTemplate` bincs. The global catalog and `debug_logging` are refused (a Nix assertion pins the catalog; the field is pinned wire). `labs/window-chrome/proxy` left in place (ADR 0047). |
| 34 | display: src/descriptor.rs duplicates lib.rs literals | applied | `62b577c08` | Deleted the legacy border-decoration renderer and the unreachable subsurface tracking, `src/wayland_proxy_argv.rs`, `nix/ui-colors.nix`+`nix/niri-vm-borders.nix`, `src/metrics.rs`+`src/audit.rs`+`tests/redaction.rs`, `src/portal.rs`, the duplicated proxy-readiness protocol, `src/descriptor.rs`, the bundle half of `PrincipalPool`, and the stale `ProxyProcessTemplate` bincs. The global catalog and `debug_logging` are refused (a Nix assertion pins the catalog; the field is pinned wire). `labs/window-chrome/proxy` left in place (ADR 0047). |
| 35 | display: bundle half of PrincipalPool unreachable | applied | `62b577c08` | Deleted the legacy border-decoration renderer and the unreachable subsurface tracking, `src/wayland_proxy_argv.rs`, `nix/ui-colors.nix`+`nix/niri-vm-borders.nix`, `src/metrics.rs`+`src/audit.rs`+`tests/redaction.rs`, `src/portal.rs`, the duplicated proxy-readiness protocol, `src/descriptor.rs`, the bundle half of `PrincipalPool`, and the stale `ProxyProcessTemplate` bincs. The global catalog and `debug_logging` are refused (a Nix assertion pins the catalog; the field is pinned wire). `labs/window-chrome/proxy` left in place (ADR 0047). |
| 36 | display: ProxyProcessTemplate + stale binary consts | applied | `62b577c08` | Deleted the legacy border-decoration renderer and the unreachable subsurface tracking, `src/wayland_proxy_argv.rs`, `nix/ui-colors.nix`+`nix/niri-vm-borders.nix`, `src/metrics.rs`+`src/audit.rs`+`tests/redaction.rs`, `src/portal.rs`, the duplicated proxy-readiness protocol, `src/descriptor.rs`, the bundle half of `PrincipalPool`, and the stale `ProxyProcessTemplate` bincs. The global catalog and `debug_logging` are refused (a Nix assertion pins the catalog; the field is pinned wire). `labs/window-chrome/proxy` left in place (ADR 0047). |
| 37 | display: hand-written Wayland global catalog; projection assertions; debug_logging; lib.rs export block | partial | `62b577c08` | The hand-written Wayland global catalog, `FilterInput::debug_logging`, and the lib.rs re-export block are refused: a Nix assertion pins the catalog and the field is a pinned wire field. The projection-assertion duplication the finding names was not separately recorded as landed. |
| 38 | shell-terminal: src/process_lifecycle.rs second ProcessProvider impl | applied | `54b7948f2` | Deleted `src/process_lifecycle.rs` with its conformance test (the systemd crate ships the same profile), `src/process_templates.rs` with its test, and the `ShellRunnerContract` constant holder; `SHELL_REPAIR_INTERVAL_SECS` kept (read by two crates). `InMemoryShellAuthority` refused: real behavior with live coverage (every method forwards to the ledger; production composes the ledger directly). |
| 39 | shell-terminal: InMemoryShellAuthority forwards to the ledger | refused | `-` | Refused: `InMemoryShellAuthority` is real behavior with live coverage (every method forwards to the ledger; production composes the ledger directly), so deleting it would remove an exercised implementation. |
| 40 | shell-terminal: src/process_templates.rs read only by its own test | applied | `54b7948f2` | Deleted `src/process_lifecycle.rs` with its conformance test (the systemd crate ships the same profile), `src/process_templates.rs` with its test, and the `ShellRunnerContract` constant holder; `SHELL_REPAIR_INTERVAL_SECS` kept (read by two crates). `InMemoryShellAuthority` refused: real behavior with live coverage (every method forwards to the ledger; production composes the ledger directly). |
| 41 | shell-terminal: *RunnerContract constant-holder pattern | applied | `54b7948f2` | Deleted `src/process_lifecycle.rs` with its conformance test (the systemd crate ships the same profile), `src/process_templates.rs` with its test, and the `ShellRunnerContract` constant holder; `SHELL_REPAIR_INTERVAL_SECS` kept (read by two crates). `InMemoryShellAuthority` refused: real behavior with live coverage (every method forwards to the ledger; production composes the ledger directly). |
| 42 | audio: orphaned legacy nix/host.nix + nix/guest.nix | applied | `54b7948f2` | Deleted `nix/host.nix`+`nix/guest.nix`, `src/audio_argv.rs`, the `src/audio_policy.rs` shim (the crate re-exports the contract module directly), and the `AudioRunnerContract` holder. `AudioMediator` defaults, `AudioReadiness`, and the in-src fake refused: a test implementor depends on the default, the daemon reads the readiness type, and another crate's tests consume the fake. |
| 43 | audio: src/audio_argv.rs duplicate argv builder | applied | `54b7948f2` | Deleted `nix/host.nix`+`nix/guest.nix`, `src/audio_argv.rs`, the `src/audio_policy.rs` shim (the crate re-exports the contract module directly), and the `AudioRunnerContract` holder. `AudioMediator` defaults, `AudioReadiness`, and the in-src fake refused: a test implementor depends on the default, the daemon reads the readiness type, and another crate's tests consume the fake. |
| 44 | audio: AudioMediator compat defaults, AudioReadiness, FakeAudioMediator | refused | `-` | Refused: a test implementor depends on the mediator default, `AudioReadiness` is read by the daemon, and the in-src fake is consumed by another crate's tests. |
| 45 | audio: src/audio_policy.rs re-export shim | applied | `54b7948f2` | Deleted `nix/host.nix`+`nix/guest.nix`, `src/audio_argv.rs`, the `src/audio_policy.rs` shim (the crate re-exports the contract module directly), and the `AudioRunnerContract` holder. `AudioMediator` defaults, `AudioReadiness`, and the in-src fake refused: a test implementor depends on the default, the daemon reads the readiness type, and another crate's tests consume the fake. |
| 46 | notification: the whole security_key/ island | applied | `54b7948f2` | Deleted the whole `security_key/` island (state machine, waybar renderer, second nonce store, second notification model, ceremony events), `src/bin/d2b-sk-waybar-helper.rs` with `nix/site.nix` and packaging, the runtime's eight unused twin entry points, the re-hardcoded collector tables, and two tautological test files. `NotificationRunnerContract` refused: the daemon's composition test calls it. |
| 47 | notification: d2b-sk-waybar-helper bin + nix/site.nix + packaging | applied | `54b7948f2` | Deleted the whole `security_key/` island (state machine, waybar renderer, second nonce store, second notification model, ceremony events), `src/bin/d2b-sk-waybar-helper.rs` with `nix/site.nix` and packaging, the runtime's eight unused twin entry points, the re-hardcoded collector tables, and two tautological test files. `NotificationRunnerContract` refused: the daemon's composition test calls it. |
| 48 | notification: NotificationRuntime's eight unused twin entry points | applied | `54b7948f2` | Deleted the whole `security_key/` island (state machine, waybar renderer, second nonce store, second notification model, ceremony events), `src/bin/d2b-sk-waybar-helper.rs` with `nix/site.nix` and packaging, the runtime's eight unused twin entry points, the re-hardcoded collector tables, and two tautological test files. `NotificationRunnerContract` refused: the daemon's composition test calls it. |
| 49 | notification: collector-field tables re-hardcoded | applied | `54b7948f2` | Deleted the whole `security_key/` island (state machine, waybar renderer, second nonce store, second notification model, ceremony events), `src/bin/d2b-sk-waybar-helper.rs` with `nix/site.nix` and packaging, the runtime's eight unused twin entry points, the re-hardcoded collector tables, and two tautological test files. `NotificationRunnerContract` refused: the daemon's composition test calls it. |
| 50 | notification: tautological tests | applied | `54b7948f2` | Deleted the whole `security_key/` island (state machine, waybar renderer, second nonce store, second notification model, ceremony events), `src/bin/d2b-sk-waybar-helper.rs` with `nix/site.nix` and packaging, the runtime's eight unused twin entry points, the re-hardcoded collector tables, and two tautological test files. `NotificationRunnerContract` refused: the daemon's composition test calls it. |
| 51 | volume-local: audit catalog + otel catalog + test | applied | `14929e51a` | Deleted the audit and otel catalogs with their test, the migration/relocation/sealing/snapshot planners with five test files, `src/path.rs` with its test, the store-view validators and swtpm-volume policy, the test-only ACL planner, the dead effect-port half, the unread schema, and the unused zone-session dependency. The orphaned `nix/store.nix`/`nix/sync-json.nix` are refused as declared (loaded by `bazel/checks/nix/BUILD.bazel`). The inline quota duplicate was removed while `quota::admit_quota` stays. |
| 52 | volume-local: migration/relocation/sealing state machines | applied | `14929e51a` | Deleted the audit and otel catalogs with their test, the migration/relocation/sealing/snapshot planners with five test files, `src/path.rs` with its test, the store-view validators and swtpm-volume policy, the test-only ACL planner, the dead effect-port half, the unread schema, and the unused zone-session dependency. The orphaned `nix/store.nix`/`nix/sync-json.nix` are refused as declared (loaded by `bazel/checks/nix/BUILD.bazel`). The inline quota duplicate was removed while `quota::admit_quota` stays. |
| 53 | volume-local: orphaned nix/store.nix + nix/sync-json.nix | refused | `-` | Refused: both files are loaded by `bazel/checks/nix/BUILD.bazel:55-56` as part of the storage-volume eval surface. |
| 54 | volume-local: snapshot policy/catalog planner | applied | `14929e51a` | Deleted the audit and otel catalogs with their test, the migration/relocation/sealing/snapshot planners with five test files, `src/path.rs` with its test, the store-view validators and swtpm-volume policy, the test-only ACL planner, the dead effect-port half, the unread schema, and the unused zone-session dependency. The orphaned `nix/store.nix`/`nix/sync-json.nix` are refused as declared (loaded by `bazel/checks/nix/BUILD.bazel`). The inline quota duplicate was removed while `quota::admit_quota` stays. |
| 55 | volume-local: src/path.rs opaque path proofs + tests | applied | `14929e51a` | Deleted the audit and otel catalogs with their test, the migration/relocation/sealing/snapshot planners with five test files, `src/path.rs` with its test, the store-view validators and swtpm-volume policy, the test-only ACL planner, the dead effect-port half, the unread schema, and the unused zone-session dependency. The orphaned `nix/store.nix`/`nix/sync-json.nix` are refused as declared (loaded by `bazel/checks/nix/BUILD.bazel`). The inline quota duplicate was removed while `quota::admit_quota` stays. |
| 56 | volume-local: store-view validators, swtpm volume policy, standalone quota planner | applied | `14929e51a` | Deleted the audit and otel catalogs with their test, the migration/relocation/sealing/snapshot planners with five test files, `src/path.rs` with its test, the store-view validators and swtpm-volume policy, the test-only ACL planner, the dead effect-port half, the unread schema, and the unused zone-session dependency. The orphaned `nix/store.nix`/`nix/sync-json.nix` are refused as declared (loaded by `bazel/checks/nix/BUILD.bazel`). The inline quota duplicate was removed while `quota::admit_quota` stays. |
| 57 | volume-local: src/effect_port.rs second vocabulary | partial | `14929e51a` | The dead half of `src/effect_port.rs` was removed (216 -> 100 lines); the surviving file re-exports the contract effect-port vocabulary (`src/effect_port.rs:12`), so the whole-file deletion the finding proposed did not happen. |
| 58 | volume-local: duplicate Volume validator inside the zone compiler | not applied | `-` | Not applied: the duplicate validator remains (`packages/d2b-provider-volume-local/nix/resources-zones-volumes.nix:11` keeps its own `modePattern`, already drifted from `resources-volume.nix:23`), and no commit in the pass touched the crate's Nix directory. |
| 59 | volume-local: JSON round-trip decode of typed contract values | applied | `14929e51a` | Deleted the audit and otel catalogs with their test, the migration/relocation/sealing/snapshot planners with five test files, `src/path.rs` with its test, the store-view validators and swtpm-volume policy, the test-only ACL planner, the dead effect-port half, the unread schema, and the unused zone-session dependency. The orphaned `nix/store.nix`/`nix/sync-json.nix` are refused as declared (loaded by `bazel/checks/nix/BUILD.bazel`). The inline quota duplicate was removed while `quota::admit_quota` stays. |
| 60 | volume-local: four competing marker-phase vocabularies; test-only ACL planner; unread schema | partial | `14929e51a` | Removed: the test-only ACL planner (`src/acl.rs` 342 -> 241) and the unread `root-config.schema.json`; the readiness vocabulary file is gone. The marker-phase vocabulary consolidation was not separately verified; `src/marker.rs` remains. |
| 61 | volume-local: volume scout could not see the driver crates | not applied | `-` | Method note in the report, not an action: the scout could not see the driver crates, and the duplications it points at are recorded under findings 1, 2, 5, and 6. |
| 62 | volume-virtiofs: in-crate reconciler + readiness classifier + testing | partial | `14929e51a` | Deleted `src/readiness.rs` (99) and `src/user_ns.rs` (181); `src/controller.rs` (361 lines, the in-crate reconciler) and `src/testing.rs` (333) are untouched, and `tests/lifecycle.rs` uses the testing fixtures, so the reconciler-and-testing half of the finding stays. |
| 63 | volume-virtiofs: third argv renderer + socket-path generator + dead helpers | applied | `14929e51a` | Deleted `src/virtiofsd_argv.rs` (247 lines, the third argv renderer), `src/user_ns.rs` (181), and the dead worker helpers (`src/worker.rs` 455 -> 198); `src/socket_path.rs` is now the 4-line `MAX_SOCKET_PATH_BYTES` constant. The daemon composes argv itself (`process_provider_runtime.rs`) and mirrors the socket path derive. |
| 64 | volume-virtiofs: src/port.rs + unread schema | partial | `14929e51a` | Deleted the unread `root-config.schema.json`; `src/port.rs` (139) remains and is used by the controller and the lifecycle test. `src/socket_path.rs` shrank 165 -> 4 (only `MAX_SOCKET_PATH_BYTES` stays). |
| 65 | usbip: the v2 reconcile model inside src/reconcile_state.rs | applied | `b35250cf0` | `src/reconcile_state.rs` shrank 5,520 -> 454 lines; the degraded-reason cluster the daemon projects stays. The reference page was corrected in `254b1ac49` after the deletion left it naming the old path. |
| 66 | usbip: src/state_machine.rs and src/usbip_argv.rs | partial | `b35250cf0` | `src/usbip_argv.rs` (748 lines) deleted; `src/state_machine.rs` refused: `docs/reference/usbip-state-machine.md:7,245` pins it as the canonical step-ordering artifact and the provider dossier adapts its ordering into the future reconciler. |
| 67 | usbip: parallel Service effect path (firewall.rs + controller.rs) | refused | `-` | Refused: the provider dossier requires `tests/controller_state_machine.rs`, `tests/wrong_zone_and_redaction.rs`, and `effect_port_contract.rs` against `UsbipEffectPort`, and those tests exist and run. `src/firewall.rs` and the controller path stay. |
| 68 | usbip: Binding lifecycle half + production.rs forwarding impl | refused | `-` | Refused: the v3 rewrite plan names `BindingLifecycle` as the attach seam no production path constructs yet - declared, not dead. |
| 69 | usbip: arbitration.rs + never-constructed declaration modules + BusId re-wrap + token duplicate | refused | `-` | Refused: dossier-declared tests and rows (`arbitration_conflict.rs`, the Process/EphemeralProcess declarations, and the physical-usb-backing token shared with security-key). |
| 70 | Forward-only nixos-modules wrappers (usbip, tpm) | refused | `-` | Refused: `nixos-modules/**` is outside the lane's write scope. |
| 71 | tpm: the semantic ticket layer, state tokens, runner half, migration helpers | applied | `31a119277` | Deleted the semantic ticket layer with its tests, the dead runner half, migration helpers, `from_status`/`status` with `src/status.rs`, and the `swtpm_argv` `exec_arg0*` flexibility. `StateDirIntent`/tokens refused (the daemon references them), the `SwtpmArgvInput` fields refused (the daemon constructs them field-by-field), and the test-support target kept (two BUILD files depend on the name). |
| 72 | tpm: swtpm_argv dead flexibility; from_status/status test-only | applied | `31a119277` | Deleted the semantic ticket layer with its tests, the dead runner half, migration helpers, `from_status`/`status` with `src/status.rs`, and the `swtpm_argv` `exec_arg0*` flexibility. `StateDirIntent`/tokens refused (the daemon references them), the `SwtpmArgvInput` fields refused (the daemon constructs them field-by-field), and the test-support target kept (two BUILD files depend on the name). |
| 73 | clipboard: d2b-clip-debug bin + packaging | applied | `62b577c08` | Deleted `src/bin/d2b-clip-debug.rs` with its packaging, the duplicated `src/clipd_host/fd.rs`, the test-only controller/rbac/descriptor cluster, the annotated-dead reserved items, the dead picker/service shells, and the never-run and duplicate tests. The controller module shrink is refused (the crate-layout policy pins its source path and the daemon uses the runner contract); the MIME-policy triplication was reported, not consolidated. |
| 74 | clipboard: src/clipd_host/fd.rs copy of src/fd.rs | applied | `62b577c08` | Deleted `src/bin/d2b-clip-debug.rs` with its packaging, the duplicated `src/clipd_host/fd.rs`, the test-only controller/rbac/descriptor cluster, the annotated-dead reserved items, the dead picker/service shells, and the never-run and duplicate tests. The controller module shrink is refused (the crate-layout policy pins its source path and the daemon uses the runner contract); the MIME-policy triplication was reported, not consolidated. |
| 75 | clipboard: test-only controller/rbac/descriptor cluster | applied | `62b577c08` | Deleted `src/bin/d2b-clip-debug.rs` with its packaging, the duplicated `src/clipd_host/fd.rs`, the test-only controller/rbac/descriptor cluster, the annotated-dead reserved items, the dead picker/service shells, and the never-run and duplicate tests. The controller module shrink is refused (the crate-layout policy pins its source path and the daemon uses the runner contract); the MIME-policy triplication was reported, not consolidated. |
| 76 | clipboard: annotated-dead reserved items | applied | `62b577c08` | Deleted `src/bin/d2b-clip-debug.rs` with its packaging, the duplicated `src/clipd_host/fd.rs`, the test-only controller/rbac/descriptor cluster, the annotated-dead reserved items, the dead picker/service shells, and the never-run and duplicate tests. The controller module shrink is refused (the crate-layout policy pins its source path and the daemon uses the runner contract); the MIME-policy triplication was reported, not consolidated. |
| 77 | clipboard: session-typed runtime admits | applied | `62b577c08` | Deleted `src/bin/d2b-clip-debug.rs` with its packaging, the duplicated `src/clipd_host/fd.rs`, the test-only controller/rbac/descriptor cluster, the annotated-dead reserved items, the dead picker/service shells, and the never-run and duplicate tests. The controller module shrink is refused (the crate-layout policy pins its source path and the daemon uses the runner contract); the MIME-policy triplication was reported, not consolidated. |
| 78 | clipboard: never-run and duplicate tests | applied | `62b577c08` | Deleted `src/bin/d2b-clip-debug.rs` with its packaging, the duplicated `src/clipd_host/fd.rs`, the test-only controller/rbac/descriptor cluster, the annotated-dead reserved items, the dead picker/service shells, and the never-run and duplicate tests. The controller module shrink is refused (the crate-layout policy pins its source path and the daemon uses the runner contract); the MIME-policy triplication was reported, not consolidated. |
| 79 | clipboard: MIME policy triplication; dead files | partial | `62b577c08` | The dead files were removed (`src/picker_session/` and `src/service/{audit,metrics}.rs` are gone, leaving `service/mod.rs`), but the MIME-policy triplication was reported, not consolidated. |
| 80 | Build: every rust_test re-lists its crate's dep set (126 targets in 25 crates) | refused | `-` | Not attempted: repo-wide build and packaging changes across other lanes' crates and the shared Bazel/Nix surface during a concurrent wave. The findings were reported with their measured numbers and remain open. |
| 81 | Build: 21-line cargo_workspace_sources filegroup cloned into 93 BUILD files | refused | `-` | Not attempted: repo-wide build and packaging changes across other lanes' crates and the shared Bazel/Nix surface during a concurrent wave. The findings were reported with their measured numbers and remain open. |
| 82 | Build: hand-rolled Nix option-stub preamble 16x/11 files | refused | `-` | Not attempted: repo-wide build and packaging changes across other lanes' crates and the shared Bazel/Nix surface during a concurrent wave. The findings were reported with their measured numbers and remain open. |
| 83 | Build: provider projection helpers copy-pasted per crate | refused | `-` | Not attempted: repo-wide build and packaging changes across other lanes' crates and the shared Bazel/Nix surface during a concurrent wave. The findings were reported with their measured numbers and remain open. |
| 84 | Build: 18 nix_surface_test registrations restating derivable fields | refused | `-` | Not attempted: repo-wide build and packaging changes across other lanes' crates and the shared Bazel/Nix surface during a concurrent wave. The findings were reported with their measured numbers and remain open. |
| 85 | Build: flake.nix enumerates 82 cp -r lines duplicating the fixture manifest | refused | `-` | Not attempted: repo-wide build and packaging changes across other lanes' crates and the shared Bazel/Nix surface during a concurrent wave. The findings were reported with their measured numbers and remain open. |
| 86 | Build: tautological Nix cases, duplicate validators, hand-enumerated exports_files | refused | `-` | Not attempted: repo-wide build and packaging changes across other lanes' crates and the shared Bazel/Nix surface during a concurrent wave. The findings were reported with their measured numbers and remain open. |
| 87 | Build: third-party versions re-typed per crate; dead root dep rtnetlink | applied | `01de0fd71` | The unused `rtnetlink = "0.14"` workspace dependency and its stale justification comment removed; not present in `Cargo.lock`, no consumer. |
| 88 | Build: 4 of 10 cargo_workspace_sources glob entries permanently empty | refused | `-` | Not attempted with the rest of the build surface. |

**What was kept, and why**

- `d2b-provider-transport-unix` and `d2b-provider-transport-vsock` (findings
  7 and 8) are refused: both are declared providers. The crate-layout policy
  pins their source and test paths
  (`packages/xtask/src/provider_crate_policy.rs:275-293`), the runtime
  provider rows list `Provider/transport-unix` and `Provider/transport-vsock`
  with their settings assertions
  (`nixos-modules/provider-runtime-contracts.nix:213-215`), their committed
  schemas and dossiers exist, and the protocol freeze names them as initial
  providers. Nothing was deleted in either crate.
- The USBIP refusals (findings 66-69) keep the dossier- and plan-declared
  surface: `state_machine.rs` is the canonical step-ordering artifact the
  reference page and the provider dossier adapt from, the `UsbipEffectPort`
  tests are dossier-required, `BindingLifecycle` is named by the v3 plan as
  the attach seam no production path constructs yet, and the arbitration and
  process/worker declarations are dossier-declared rows.
- `volume-local`'s orphaned Nix files (finding 53) are refused as declared:
  the storage-volume eval surface in `bazel/checks/nix/BUILD.bazel` loads
  both.
- The tpm state-intent tokens and `swtpm_argv` input fields (findings 71, 72)
  are kept because the daemon references and constructs them.
- The display Wayland global catalog and `debug_logging` field (finding 37)
  are kept because a Nix assertion pins the catalog and the field is a pinned
  wire field; the clipboard controller module shrink (finding 73-79 group) is
  kept because the crate-layout policy pins its source path and the daemon
  uses the runner contract.
- Refused outright: findings 3, 5, 6, 27, 39, 44, 67-70 and 80-86 plus 88.
  The interaction boilerplate measured smaller than reported and its wrappers
  are public surface; the shared homes for `spec_ref` and the host/user
  driver are in the toolkit (off-limits mid-restructure); the notification ack
  path and the audio defaults are exercised by live tests; the build/packaging
  consolidation is a repo-wide change across lanes that the pass did not
  attempt.
- Partially applied: findings 4, 11, 37, 57, 60, 62, 64, 66 and 79. Finding
  58 was not attempted: the duplicate zone-compiler Volume validator still
  ships with a drifted `modePattern`.


## Shared runtime, toolkit, and contract layer

Twenty-eight findings over twelve crates: the resource runtime, types, API,
compiler, and client; the provider toolkit and registry; the controller
session library; and the contract crates. Carried by `c67997af1`,
`0a521e8a9`, `baf1f7a0c` (content staged by this family, committed under a
policy-family message), `c125d191c`, and `6d6bd67d7`.

| # | Finding | Verdict | Commit(s) | Evidence |
| --- | --- | --- | --- | --- |
| A1 | 11 provider crates hand-roll the same 45-line declaration-only ResourceDriver | applied | `c67997af1 0a521e8a9` | One driver in `packages/d2b-resource-runtime/src/metadata.rs` (535 lines) and one declaration builder + shared registration assertion in `packages/d2b-resource-types/src/metadata.rs`; the eleven type crates' `driver.rs` are 20-22 lines. |
| A2 | Generated resource-type catalog, its generator, drift test, and sync test | applied | `baf1f7a0c` | `packages/d2b-contracts/src/generated/resource_type_catalog.rs`, its `mod.rs`, `xtask/src/gen_resource_type_catalog.rs`, the drift target, and the `.bzl` entry are gone; both plane cross-checks read `d2b_contracts::identity::V3_CONVERTED_RESOURCE_TYPES`. About 251 lines. |
| A3 | d2b-core runtime.rs duplicates 8 runtime DTOs | applied | `baf1f7a0c` | `packages/d2b-core/src/runtime.rs` re-exports the eight names from `d2b-contracts`; the field-for-field duplicate declarations and 15 call sites were updated (-215). |
| A4 | 6 one-line compat shim modules in d2b-core plus a MODULE_NAME const | not applied | `-` | `error.rs`, `contract_id.rs`, `configured_argv.rs`, `privileges_w3.rs`, `workload_identity.rs`, `unsafe_local_workloads.rs` still exist at HEAD; the pass's applied-work list excluded this item. |
| A5 | 12 MODULE_NAME consts and the smoke_tests module asserting them against their own filenames | not applied | `-` | The consts and the smoke test remain in `packages/d2b-resource-runtime/src/*.rs` and `lib.rs`. |
| A6 | 20 hand-written redaction Debug impls where `redacted_debug!` exists | not applied | `-` | Hand-written impls remain in `d2b-resource-api` and `d2b-core-controller` (e.g. `packages/d2b-resource-api/src/authz.rs`); the exported macro is unused there. |
| A7 | 92 BUILD.bazel files repeat the cargo_workspace_sources filegroup and all-tests suite | not applied | `-` | No emitting macro exists in the tree (no `cargo_crate` in `bazel/`); the clones remain. |
| A8 | d2b-core/src/base64_codec.rs hand-rolls RFC 4648 padding rules | not applied | `-` | `packages/d2b-core/src/base64_codec.rs` is present; no wrapper onto the workspace `base64` dependency landed. |
| A9 | Three hand-maintained copies of the converted-resource-type list and two sync tests | applied | `baf1f7a0c` | `WellKnownType::ALL` is projected at const-eval from `d2b_contracts_resource::v3::V3_CONVERTED_RESOURCE_TYPES` (`packages/d2b-resource-types/src/resource_type.rs:103`); the fence test is deleted and the generated copy is gone with A2. |
| B1 | Five d2b-resource-api modules with zero callers plus the bus-side WatchSink impl | partial | `baf1f7a0c` | Deleted: `metrics.rs`, `zone_service.rs`, `quota_gate.rs`, `emergency_gate.rs` (807 lines) and the crate's telemetry dependency. Kept: `watch.rs` and the bus-side impl; deleting the impl cascades through the tested watch-delivery credit path (`OutgoingStream::send_wait`, `send_and_wait_ack`, `StreamBridge::acknowledged_bytes`). |
| B2 | d2b-provider installation/share_adapter/forwarding and the dispatcher half of agent.rs | partial | `baf1f7a0c` | Deleted: `installation.rs`, `share_adapter.rs`, `forwarding.rs` (1,134 lines, -1,290 with tests) and three dependency edges. Kept: the `agent.rs` dispatcher half, because the toolkit's `FakeProvider` implements `ProviderAgentService` and is exported from the framework the pass keeps. |
| B3 | The toolkit's unconsumed framework half | refused | `-` | Declared-but-unwired work the provider lanes are instantiating; the toolkit's live paths are the base the migration lands on. The zero-impl codec traits and zero-caller entry points remain in `packages/d2b-provider-toolkit/src/{server,operations,plane,base,testing}`. |
| B4 | d2b-contracts dead/compat surface | partial | `baf1f7a0c` | Deleted: `usbip_effect_port.rs` (509 lines), `provider_effects/mod.rs`, `auth_wire.rs`. Kept: the `target.rs` node-era compat types (their only external references are in `d2b-realm-core`, whose disposition is planned separately) and two test-only identity wrappers the owning lane holds. |
| B5 | d2b-contracts-zone-session skeleton limbs | partial | `baf1f7a0c` | Deleted: `src/v3/generation_bundle.rs` (608) with its test file (171), its mod lines, its BUILD target, and its suite entry. Not applied: the dead half of `services.rs` and the six `*StatusResource` projections (~1,000 lines) stay. |
| B6 | d2b-resource-runtime peripheral dead paths | partial | `bfaf2091e` | Deleted: `ManagerCall` and `ChannelManagerEndpoint` (`context.rs`), with the five tests that drove them reworked onto a local endpoint stub and their assertions preserved. Kept, with the callers that justify them: the audit-log history read path (five in-crate tests use it as their audit observation mechanism) and the identity re-export module (ten in-crate modules plus nine provider-side crates import identity types through it). |
| B7 | d2b-resource-compiler's compile_provider_artifact alias | applied | `bfaf2091e` | Deleted the alias and its doc comment; the canonical `compile_artifact` it forwarded to stays and keeps its tests. |
| B8 | BindingChildReconciler | applied | `bfaf2091e` | Deleted the type, its `lib.rs` re-export, and the five tests plus one helper whose only subject it was; tests exercising surviving functions in that module stay. |
| B9 | Assorted one-purpose public surface (aliases, reachability enum, limits consts) | not applied | `-` | Still present in `d2b-resource-client`, `d2b-resource-api`, and `d2b-contracts-resource`. |
| C1 | GuestControlEndpoint declared twice, byte-identically | not applied | `-` | Both copies remain: `packages/d2b-resource-client/src/zone_client.rs:129` and `packages/d2b-provider-guest-cloud-hypervisor/src/guest_local.rs:49`. |
| C2 | d2b-core-controller authority.rs parallel NIC machinery and test-only constructors | not applied | `-` | Still present. |
| C3 | controller_assignment.rs hand-rolled JSON codec | not applied | `-` | Still present. |
| C4 | Contract-crate macro/boilerplate consolidation (76 Wire deserialize blocks, identity macros, facade copies) | not applied | `-` | Still present. |
| C5 | StoreErrorKind second taxonomy and its two translation tables | not applied | `-` | `packages/d2b-contracts-resource/src/v3/operations/error.rs:93` still declares the enum beside `ResourceErrorKind`. |
| C6 | d2b-provider-guest driver hand-rolls what shared_provider.rs ships | not applied | `-` | Refused with the guest family's fold (guest finding 42): framework-side work. |
| C7 | d2b-resource-compiler hand-rolls a JSON-Schema validator | not applied | `-` | The walkers and secret-shape scanners remain in `packages/d2b-resource-compiler/src/`. |
| C8 | The same type declared in several crates (TerminalSize, clocks, CancellationToken, UID/GID rows, IfNameMapping, ZoneLinkLimits, ControllerSessionBinding) | not applied | `-` | Consolidations cross live API surfaces owned by other lanes; not attempted in this pass. |
| C9 | Forwarding-only functions (~880 single-statement forwarders) | not applied | `-` | Not attempted. |
| C10 | Test mass that buys nothing | not applied | `-` | Not attempted beyond the per-family deletions listed in the other sections. |

**What was kept, and why**

- The toolkit's unconsumed framework half (B3) is refused as
  declared-but-unwired work: the provider lanes are instantiating it, and no
  piece of it is superseded by a duplicate elsewhere. The zero-impl codec
  traits and zero-caller entry points named in the report remain in
  `packages/d2b-provider-toolkit/src/`.
- The bus-side watch sink and `d2b-resource-api/src/watch.rs` (B1) are refused:
  deleting them cascades through `OutgoingStream::send_wait`,
  `send_and_wait_ack`, and `StreamBridge::acknowledged_bytes`, which are the
  writer end of the tested watch-delivery credit path
  (`bounded_watch_delivery_waits_for_transport_credit`) and the receive-path
  ack accounting.
- The `d2b-provider` agent dispatcher half (B2) is refused: the toolkit's
  `FakeProvider` implements `ProviderAgentService` and is exported from the
  framework the pass keeps.
- The `target.rs` node-era compat types (B4) are kept because every external
  reference is a re-export in `d2b-realm-core`, whose disposition is scheduled
  separately; editing it to satisfy a deletion here would cross lanes.

**Gaps this family's record names**

Findings A4, A5, A6, A7, A8, B6, B7, B8, B9 and the whole C tier were verified
as still present at the head but were not in the pass's applied-work list, so
they carry no refusal reason per item. The consolidation work among them
touches crates and live API surfaces other lanes held during the pass; the
same reason was recorded for the resources family's build and packaging
findings. They are the largest honest gap in this record: the code is
unchanged, and the decision to leave it is a scope statement, not a verdict on
the finding.


## Daemon composition audit

A read-only audit of the daemon's composition modules classified 123 pockets:
78 keep with a stated reason, 32 move to the declaring provider or a shared
helper, and 13 delete as residue, with the typed-broker construction class
counted separately (67 census sites). The effect-port split decided most
verdicts: adapter halves retire with their typed arm, executor halves - the
privileged effect, the descriptor handoff, the trusted resolution against the
broker's own bundle copy - stay in the daemon by design.

The delete half landed in `5bbd1d861` (6 files, +101/-1881): the
`cfg(test)`-only network and legacy-TPM simulations, the generic `Reconcile`
refusal arm, `resolve_volume_storage_ref`, `parse_committed_network_spec` and
the fifteen callerless admin-default broker wrappers, the legacy
security-key reconcile dispatch with its relay-target locator, and the USBIP
child-port trait with its fail-open stub. `b20c9ec37` separately deleted the
retired socket-intent resolution (-73).

**Kept because the delete verdict did not survive the caller census**
(the commit message carries each reason):

- `credential_backend_runtime.rs`'s `cfg(test)` lease registry and
  `ProductionGuestCredentialBackendSupervisor::fail_closed` - live test
  infrastructure; deleting them removes coverage, not residue.
- The USBIP CLI verb tail `usbip_guest_import_unavailable` - reachable from
  the live Device USB dispatch.
- `security_key_effect_port::RelayTarget` (the field type of the kept effect
  port) and `resource_runtime::security_key_device_is_admitted` (retained with
  an explicit allow and reason as the trusted admission the kept port
  consumes).

**Withdrawn instruction:** the plan ordered deleting `packages/d2b-realm-core`
as orphaned; the re-verified tree showed the standalone
`labs/window-chrome/proxy` prototype declares it by path and imports
`WorkloadProviderKind` from it, so the tree stays (`40d562887`).

**A staged deletion that did not belong to its commit:** the bundle manifest
module's deletion reached the tree inside a commit meant for one enrollment
test and left the tree unbuildable; it was restored (`19a59ed55`) so the
owning change can carry its own callers.

The 32 move pockets wait on the per-family migration of their typed call
sites; the 78 kept pockets have their reasons at the site, and two async-purity
seats the sweep named are recorded as deliberate keeps in `c93da2e53`.


## Refusals that overruled the audits

The audits were wrong in several places, and the pass refused findings rather
than deleting live behavior. The refusals cluster into the classes below; each
one is a decision a diff cannot show.

**A live caller.** The policy audit called `ConfigNixosClient` callerless; the
daemon calls it at `packages/d2bd/src/composition.rs:11508`. The same audit
called the zone handler-status emitter callerless; the live
`SystemCoreStatusEmitter` calls it (`packages/d2b-provider-zone/src/zone_status.rs:149`,
constructed at `packages/d2bd/src/resource_runtime.rs:3216`). The process audit
said the process family reads two `&str`s from its own Providers through
`check_provider`; `PROVIDER_REF` has eight non-test users across seven files.
The guest audit said the azure-virtual-machine crate already used the toolkit
clock seam; it did not, and the dependency had to be added. The composition
audit's lease-registry and fail-closed delete verdicts did not survive the
caller census (both are live test infrastructure).

**A declared artifact or dossier requirement.** The two process effect-port
adapters are the declared boundary in both process dossiers. The otel ingress,
emitter, agent, config, and metrics surface is dossier-planned work. The
transport-unix and transport-vsock crates are named by the policy matrix,
runtime provider rows, dossiers, committed schemas, and the protocol freeze -
"no in-tree caller" does not make a declared provider crate dead. The USBIP
state machine, effect-port tests, and binding lifecycle are dossier- and
plan-declared. The metadata crates' `integration/*.rs` scaffolds are required
by the crate-layout policy.

**A credit-path dependency.** The bus-side watch sink and the resource API's
`watch.rs` looked orphaned, but deleting the sink cascades through the tested
watch-delivery credit path (`send_wait`, `send_and_wait_ack`,
`acknowledged_bytes`), so the deletion was reverted and both stay.

**A certificate that the "dead" code was load-bearing.** The secret-service
session-capability authority was reported as having no issuer in `src`; the
issuer and its consumers are live in the same file
(`packages/d2b-provider-credential-secret-service/src/lib.rs:1290-1360`), and
both dispatch paths route through it. `CredentialRevocationEvidence` has a
production reader. The qemu-media hand-written deserializers are the live
admission gate. The usbip `state_machine.rs` is the ordering source a
reference page and a dossier still adapt from.

**Coverage and measured no-savings.** Findings whose deletion would remove
coverage or whose saving does not exist were refused with the measurement: the
five state-status table rows exercise distinct conjuncts, the pidfd poll-loop
pair shares nine lines and needs its split, the single-owner frame mutex has
no `&mut` call site, and the supervisor's local `/proc` parser classifies
zombie and dead states the shared parser the audit pointed at does not.


## Discrepancies between the reports and the tree

Recorded because the audit reports are not in the tree and a reader deserves to
know where their claims did not hold at `515cbf610`:

1. `ConfigNixosClient` has a live caller (`packages/d2bd/src/composition.rs:11508`);
   the policy report says it has none.
2. The zone handler-status emitter is live through `SystemCoreStatusEmitter`
   (`packages/d2bd/src/resource_runtime.rs:3216,3616,4957`); the policy report
   says neither copy is called in production.
3. `PROVIDER_REF` has eight non-test users across seven files; the process
   report says the family reads it only in `check_provider`.
4. Azure-virtual-machine did not already depend on `d2b-provider-toolkit`; the
   guest report's clock-seam replacement assumed it did. The edge was added
   (+1/-1 net crate edges in the family).
5. qemu-media's Nix premises (report findings 30 and 38) are refuted: the
   projection assertions are evaluated as a module and the zone patch is
   applied and asserted; both stay.
6. The systemd provider dossier still names `src/adoption.rs`
   (`docs/specs/providers/ADR-046-provider-system-systemd.md:1295,1351,1468`)
   while the module is deleted and no dossier edit landed. The applying lane's
   own report lists that module as refused under the dossier guard, which
   cannot be reconciled with the commit; the tree is recorded here.
7. The policy report counts ten metadata crates; the shared driver that landed
   serves eleven declaration-only types (the `Command` type came through the
   process family's template finding). The difference is a reporting scope
   artifact, not a missed deletion.
8. Commit attribution is crossed on three commits (see Counts); a row's
   carrying commit may have a subject from another lane.
9. The reports themselves warn that their line anchors drifted while siblings
   edited; all evidence in this record is at `515cbf610`, not at the audit
   revision.


## What this record could not reconstruct

Named rather than papered over:

- **The shared family's untaken set.** Findings A4-A8, B6-B9, and C1-C10 are
  present at the head but were outside the pass's applied-work list. No
  per-item refusal reason was recorded; the group reason is scope during a
  concurrent wave.
- **The resources family's build and packaging block.** Findings 80-86 and 88
  were reported with numbers and not attempted; no per-item decision exists
  beyond the same scope reason. Finding 87 landed.
- **The composition audit's move half.** The 32 move pockets have no landed or
  refused record in the tree; they wait on per-family call-site migration, and
  the per-pocket list lives in the session artifact, not here.
- **Per-crate detail bullets.** The process and resources reports carry
  sub-findings below the ranked set; those the applying lane's record covered
  are folded into the tables. A few sub-items (for example the activation
  crate's duplicated ordinal parse, the telemetry pair's phase constants, and
  the minijail/systemd identity-binding set declared three times) have no
  per-item outcome recorded and no commit naming them; they are open.
- **The audit totals as originally estimated.** The reports predicted larger
  nets than the lanes realized in several families (resources: about -34,700
  predicted against -25,524 landed). The difference is mostly the untaken
  build/packaging block and the refusals above, and it is expected; the
  numbers are not comparable line-for-line because the lanes report their own
  commit sets.
- **Gate results.** This record does not re-run any gate. The lane reports
  carry per-crate `cargo check` and test results and name the points where
  `make check` was red from other lanes' in-flight state; those claims were
  not re-verified here.
