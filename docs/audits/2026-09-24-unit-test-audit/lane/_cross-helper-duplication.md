# Cross-crate test-helper duplication (C1) - unit-test audit
net: -646 lines of duplicated helpers

## Findings (biggest net first)
- duplicate: `RecordingManager` (12 src copies, 464 ln) - canonical home `d2b-provider-toolkit::testing::fakes` (add a `RecordingManagerEndpoint` fake; ADR-046 designates this module as the every-Provider doubles home, and its `block_on` already absorbed the same class of drift). Recording `ManagerEndpoint` double - call log + owned-row set for ensure_child/remove_child/list_owned/watch - re-derived per crate for the same trait:
  - d2b-provider-volume-binding/src/driver.rs:1137-1208 (72)
  - d2b-provider-guest/src/driver.rs:1533-1602 (70)
  - d2b-provider-provider/src/driver.rs:755-819 (65)
  - d2b-provider-telemetry-binding/src/driver.rs:673-721 (49)
  - d2b-provider-activation-nixos/src/driver.rs:989-1020 (32)
  - d2b-provider-host/src/driver.rs:514-545 (32)
  - d2b-provider-user/src/driver.rs:468-499 (32)
  - d2b-provider-credential/src/driver.rs:1072-1100 (29)
  - d2b-provider-telemetry-service/src/driver.rs:464-492 (29)
  - d2b-provider-volume/src/driver.rs:769-789 (21)
  - d2b-provider-toolkit/src/shared_provider.rs:1126-1145 (20)
  - d2b-provider-network-local/src/driver.rs:422-434 (13)
  - plus one integration copy: d2b-provider-wayland-policy/tests/engine.rs:95-127.
  d2b-resource-runtime::context::test_support already holds `DeadManager`/`NullRequeue` but is `#[cfg(test)] pub(crate)` - unreachable cross-crate, so the toolkit is the home. Net -392 (464 - largest copy 72 as shared-impl base).

- duplicate: `RecordingRequeue` (10 src copies, 148 ln) - canonical home `d2b-provider-toolkit::testing::fakes` (same module as above). Recording `RequeueScheduler` double - schedule/cancel into a Mutex<Vec<...>>:
  - d2b-provider-process/src/driver.rs:2470-2498 (29)
  - d2b-provider-user/src/driver.rs:562-581 (20)
  - d2b-provider-toolkit/src/shared_provider.rs:1217-1233 (17)
  - d2b-provider-host/src/driver.rs:608-622 (15)
  - d2b-provider-volume-binding/src/driver.rs:1314-1328 (15)
  - d2b-provider-network-local/src/driver.rs:502-515 (14)
  - d2b-provider-guest/src/driver.rs:1760-1770 (11)
  - d2b-provider-provider/src/driver.rs:899-907 (9)
  - d2b-provider-telemetry-binding/src/driver.rs:838-846 (9)
  - d2b-provider-telemetry-service/src/driver.rs:588-596 (9)
  - plus one integration copy: d2b-provider-device/tests/device_family.rs:76.
  Net -119 (148 - largest copy 29).

- duplicate: `block_on` (6 src copies, 66 ln) - canonical home `d2b-provider-toolkit::testing::block_on` (src/testing/mod.rs:78-88, "this is that driver, once"). Noop-waker single-thread future driver, byte-similar; stragglers after the toolkit absorbed the family:
  - d2b-provider-network-local/src/broker.rs:1655-1665 (11)
  - d2b-provider-system-core/src/testing.rs:23-33 (11)
  - d2b-provider-volume-local/src/testing.rs:29-39 (11)
  - d2b-provider-volume-virtiofs/src/testing.rs:23-33 (11)
  - d2b-process-conformance/src/testing.rs:33-43 (11)
  - d2bd/src/composition.rs:30082-30092 (11)
  - plus 5 integration copies: d2b-core/tests/loader_worker.rs:41-51, d2b-core/tests/loader_worker_panic.rs:25-35, d2b-provider-device-tpm/tests/resource_controller.rs:224-236, d2b-provider-network-local/tests/reconcile.rs:64-74, d2bd/tests/zone_provider_acceptance.rs:68-85.
  Note: d2b-core, d2b-process-conformance, d2b-provider-device-tpm, d2b-provider-volume-local, d2b-provider-volume-virtiofs do not depend on d2b-provider-toolkit today; switching needs the dep. Net -55 (66 - canonical 11).

- duplicate: scratch-root resolution `test_scratch_root`/`test_root`/`writable_manifest_dir` (7 src copies, 59 ln, 5 crates) - canonical home `d2b-core::test_support` (feature-gated module already consumed cross-crate by d2bd-runtime via `RoleProfileBuilder`). TEST_TMPDIR-priority writable scratch dir; d2b-broker/d2bd-runtime/d2bd shapes are byte-identical, d2b-core's adds a `test_name` param:
  - d2b-broker/src/lib.rs:60-67 (8)
  - d2bd-runtime/src/lib.rs:60-68 (9)
  - d2bd/src/composition.rs:376-384 (9)
  - d2b-core/src/bundle_resolver.rs:5834-5845 `test_root` (12)
  - d2b-audit/src/export.rs:303-309, src/segment.rs:1078-1084, src/sink.rs:461-467 `writable_manifest_dir` (7 × 3 = 21; the triple is intra-crate - the d2b-audit lane flags it too).
  Note: d2b-audit does not depend on d2b-core; the shared home needs a dep or the audit triple stays local. Net -47 (59 - keeper 12).

- duplicate: `sample_zone_native_host_json`/`sample_v3_host_contract_json` (2 src copies, 66 ln) - covering helper `d2b-core/src/bundle_resolver.rs:7022-7054::sample_zone_native_host_json`; canonical home `d2b-core::test_support`. Byte-identical v3 host-contract JSON (schemaVersion/site/nftables/networkManager/hostsFile/kernelModules/fdOwnership/cloudHypervisorCapabilities), differing only in `serde_json::json!` vs `json!` prefix:
  - d2bd/src/composition.rs:28266-28298 `sample_v3_host_contract_json` (33)
  Net -33.

## Keep
- Per-crate `test_support.rs` `RecordingEffects`/`RecordingRuntime` doubles (credential, device, device-usbip, device-security-key, guest, host, network-local, notification-desktop, provider, activation-nixos) - each implements a distinct effect/runtime trait; already feature-gated shared homes consumed by d2bd plane tests; shapes similar but not unifiable.
- `recording_facets` facet-set builders (~10 crates) - one-line wrappers over per-crate facet types; fine.
- Row builders (`test_row`, `generation_row`, `provider_row`, `guest_row`, `owned_row`, `binding_row`, `row_with`) - same name family only; each carries crate-specific keys, uid constants, and spec shapes.
- `fixture()`/`context()` ResourceContext builders (15 crates) - crate-specific row/decoder/Fixture wiring dominates; only the ~6-line `mpsc::unbounded_channel()` pair + `ResourceContext::new(...)` core repeats - revisit as a toolkit helper once `RecordingManager` lands there.
- `fixtures` modules (d2b-bus session, d2b-process-conformance, d2b-provider-system-core, d2b-provider-volume-local, d2b-provider-volume-virtiofs, d2b-resource-client, xtask delivery) - per-crate canonical fixture sets; fine.
- `type Log = Arc<Mutex<Vec<String>>>` aliases (credential test_support.rs:30, network-local driver.rs:420, toolkit shared_provider.rs:1087, wayland-policy test_support.rs:127) - 1-line aliases; nothing gained by sharing.
- d2b-core::test_support builders (`RoleProfileBuilder`, `ResolvedRunnerIntentBuilder`) - already the shared home; fine.
