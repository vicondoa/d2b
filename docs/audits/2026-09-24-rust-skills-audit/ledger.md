# Rust skills remediation ledger

Baseline: branch `refactor-rust-skills-remediation`, base commit `147a536a0` (the audit baseline `v3` @ `6ebdd4cec` plus the audit corpus commit). Authority: `docs/plans/2026-09-24-002-refactor-rust-skills-remediation-plan.md` (R1-R14, KTD1-KTD11). Corpus: `README.md` and `lane/`.

One row per finding id. `outcome` is filled by the owning wave when it disposes of the row: `applied`, `applied-variant` (the claim or the stated fix needed a minimal correction or a recorded deviation), `skipped-stale` (the claim does not hold at HEAD; no change made), `already-fixed` (the stated fix is already present in the tree), `escalated` (moved to the owning wave named in `escalation`), `policy-confirmed` (recorded no-op citing its policy), `needs-contract` (deferred to the contract-adjacent wave), `reclassified` (severity or verdict changed on re-verification, reason recorded), `declined` (the claim holds but no correct minimal change lands - the pinned toolchain rejects the rewrite, or the fix would break a contract; the evidence is recorded and the code stays as it is), and `not-started` (the batch's budget ended before the row; it is carried into a follow-up dispatch, never counted as done).

Corpus caveat: the audit read its sources through a tool path that rewrites long digit runs, so at least one row (RS-0916) quotes a literal that exists nowhere in the tree or in git history. Every row's true state is re-verified at apply time (R3) and the ledger records the corrected finding; the corpus row text is left as the audit wrote it. `anchor` is the apply-time anchor when the wave re-located it; the seed anchor comes from the corpus row. Empty cells mean the row is not yet disposed.

Every id must appear exactly once and end `applied`, `already-fixed`, `policy-confirmed`, or `needs-contract` at close-out; `escalated` rows carry the escalation history and their final outcome (R1, KTD1).

## Baseline record (gate set at the untouched head)

Measured at `147a536a0` in a dedicated gates worktree before any wave-0 fix landed. The whole gate set is green at the baseline, so a wave's bar is to stay green rather than to improve a red gate; any red a wave introduces is its own.

| gate | command | result at baseline | attribution |
| --- | --- | --- | --- |
| security scan | `D2B_SCAN_BASE_SHA=6ebdd4cec tests/tools/security-scan.sh` | pass (clean) | none |
| blocking census | `make check-census` | pass (no crate above its committed baseline) | none |
| Layer-1 aggregate | `make check` | pass (988 of 988 tests) | none |
| host integration | `make test-host-integration` | pass (11 of 11 vmChecks) | Attic closure-upload warning only, non-fatal |

## Wave gates

Each wave closes on the same gate set, run on the wave's integrated head in the gates worktree. `base` is the commit the scan measures changed lines against.

| wave | head | security scan | census | Layer-1 aggregate | host integration | notes |
| --- | --- | --- | --- | --- | --- | --- |
| U1 | `11bbfe41a` | pass | pass | pass (988 of 988 tests) | pass (11 of 11 vmChecks) | First attempt flaked on the load-sensitive `daemon_state_persistence` kill-during-startup race (passes standalone, not an audit row); the retry is green. The head carries the refreshed async-gate inventory for the broker line shifts. |

## Findings (965 rows)

| id | lens | cluster | sev | audit verdict | blast/effort | outcome | wave | commit | anchor | reason or policy citation | escalation |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `RS-0037` | `idiom` | `d2b` | medium | actionable | leaf | applied | U2 | d4e4604e4 | `packages/d2b/src/dispatch.rs` | all_known_subcommands derived from parser via modern_cli_subcommands minus PROJECTION_COMMANDS; test updated; mutation (re-add up) fails updated assertion |  |
| `RS-0031` | `idiom` | `d2b` | low | actionable | leaf | applied | U2 | 0a4f73a1f | `packages/d2b/src/doctor.rs` | PidfdEntries::state_detail() added; five detail matches replaced |  |
| `RS-0036` | `idiom` | `d2b` | low | actionable | leaf | applied | U2 | 0a4f73a1f | `packages/d2b/src/zone_doctor.rs` | summarize uses DoctorSummary::default() |  |
| `RS-0032` | `idiom` | `d2b` | low | actionable | leaf | applied-variant | U2 | 0a4f73a1f | `packages/d2b/src/resource.rs` | typed/typed_noun consume TypedResourceArgs by value; 7 dispatch sites pass args.clone() because match binds by reference (deviation recorded |  |
| `RS-0033` | `idiom` | `d2b` | low | actionable | leaf | applied | U2 | 0a4f73a1f | `packages/d2b/src/zone_audit.rs` | valid_hash deleted; call sites now use valid_digest |  |
| `RS-0034` | `idiom` | `d2b` | low | actionable | leaf | applied | U2 | 0a4f73a1f | `packages/d2b/src/zone_audit.rs` | nested verify_chain() shared by v1/v2 validation paths |  |
| `RS-0035` | `idiom` | `d2b` | low | actionable | leaf | applied | U2 | 0a4f73a1f | `packages/d2b/src/zone_audit.rs` | validate_fields(class, fields, fn) merged; thin wrappers keep both validators |  |
| `RS-0001` | `idiom` | `d2b-audit` | medium | actionable | leaf |  |  |  | `packages/d2b-audit/src/export.rs:255, packages/d2b-audit/src/segment.rs:980, packages/d2b-` |  |  |
| `RS-0002` | `idiom` | `d2b-audit` | low | actionable | leaf |  |  |  | `packages/d2b-audit/src/export.rs:103` |  |  |
| `RS-0003` | `idiom` | `d2b-audit` | low | actionable | leaf |  |  |  | `packages/d2b-audit/src/sink.rs:393, packages/d2b-audit/src/sink.rs:403` |  |  |
| `RS-0011` | `idiom` | `d2b-broker` | medium | actionable | leaf | applied | U2 | c155578ca | `packages/d2b-broker/src/ops/device_worker.rs` | find_resource_row helper drives row_owner_ref/device_guest_owner/tpm_devices_of_guest |  |
| `RS-0006` | `idiom` | `d2b-broker` | low | actionable | leaf | applied | U2 | 99d7247ee | `packages/d2b-broker/src/runtime.rs` | parse_common_flags helper extracted; parse_probe_flags now takes Vec<String>; error strings preserved |  |
| `RS-0007` | `idiom` | `d2b-broker` | low | actionable | leaf | applied | U2 | 99d7247ee | `packages/d2b-broker/src/runtime.rs` | push loop replaced with filter_map; foreign lock Err early return preserved; merged with RS-0006 in same commit |  |
| `RS-0008` | `idiom` | `d2b-broker` | low | actionable | leaf | applied | U2 | d2096735c | `packages/d2b-broker/src/sys.rs` | format_errno reversal now iter_mut/zip without intermediate allocation |  |
| `RS-0009` | `idiom` | `d2b-broker` | low | actionable | leaf | applied | U2 | 4234afe18 | `packages/d2b-broker/src/ops/exec_reconcile.rs` | seven hand-copied absolute-path checks factored into require_absolute; error wording normalized; no test asserts old strings |  |
| `RS-0010` | `idiom` | `d2b-broker` | low | actionable | leaf | applied | U2 | 5a2fa07c7 | `packages/d2b-broker/src/ops/store_view_farm.rs` | run_store_helper and store_helper_failure extracted; both namespaced builders share them |  |
| `RS-0012` | `idiom` | `d2b-broker` | low | actionable | leaf | applied | U2 | ac44605b7 | `packages/d2b-broker/src/envelope/mod.rs` | open/publish/init_trusted_context_store gated #[cfg(test)]; only in-crate test callers existed; Drop persist path left ungated |  |
| `RS-0004` | `idiom` | `d2b-broker-composition` | low | actionable | leaf |  |  |  | `packages/d2b-broker-composition/src/dependency_surface.rs:136` |  |  |
| `RS-0005` | `idiom` | `d2b-broker-composition` | low | actionable | leaf |  |  |  | `packages/d2b-broker-composition/src/seam.rs:270, packages/d2b-broker-composition/src/seam.` |  |  |
| `RS-0013` | `idiom` | `d2b-bus` | low | actionable | leaf | applied | U2 | fbf92683a | `packages/d2b-bus/src/router.rs` | resolve_for_service uses filter_map+next() over collect-then-index; bus check+tests passed |  |
| `RS-0014` | `idiom` | `d2b-bus` | low | actionable | leaf | applied | U2 | fbf92683a | `packages/d2b-bus/src/operations.rs` | abort_destination uses drain(..).partition and aborts drained handles; tests passed |  |
| `RS-0015` | `idiom` | `d2b-contracts-broker` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-broker/src/kernel_client.rs:225-227` |  |  |
| `RS-0016` | `idiom` | `d2b-contracts-broker` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-broker/src/host_generation.rs:162-167, packages/d2b-contracts-broke` |  |  |
| `RS-0017` | `idiom` | `d2b-contracts-broker` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-broker/src/broker_wire.rs:2904-2912, packages/d2b-contracts-broker/` |  |  |
| `RS-0018` | `idiom` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/credential.rs:362, packages/d2b-contracts-provider/` |  |  |
| `RS-0020` | `idiom` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/semantic_services/child_resources.rs:573, packages/` |  |  |
| `RS-0019` | `idiom` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/provider.rs:2374` |  |  |
| `RS-0022` | `idiom` | `d2b-contracts-resource` | medium | actionable | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/volume.rs:1210-1213, packages/d2b-contracts-resourc` |  |  |
| `RS-0021` | `idiom` | `d2b-contracts-resource` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/network.rs:597, packages/d2b-contracts-resource/src` |  |  |
| `RS-0023` | `idiom` | `d2b-contracts-resource` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/resource.rs:621-626` |  |  |
| `RS-0024` | `idiom` | `d2b-contracts-resource` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/volume.rs:1218-1223` |  |  |
| `RS-0025` | `idiom` | `d2b-contracts-zone-session` | low | actionable | leaf |  |  |  | `src/v3/component_session.rs:1935, src/v3/component_session.rs:1976` |  |  |
| `RS-0027` | `idiom` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:1805, packages/d2b-core/src/bundle_resolver.rs:19` |  |  |
| `RS-0030` | `idiom` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/static_invariants.rs:162, packages/d2b-core/src/static_invariants.rs` |  |  |
| `RS-0028` | `idiom` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:5746` |  |  |
| `RS-0029` | `idiom` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:2434, packages/d2b-core/src/processes.rs:180` |  |  |
| `RS-0026` | `idiom` | `d2b-core-controller` | low | actionable | leaf |  |  |  | `authority.rs:2189-2192, authority.rs:2542-2545` |  |  |
| `RS-0040` | `idiom` | `d2b-provider-clipboard-wayland` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/clipd_host/policy.rs:3-16, packages/d2b-provid` |  |  |
| `RS-0039` | `idiom` | `d2b-provider-clipboard-wayland` | medium | actionable | leaf |  |  |  | `src/bin/d2b-clipd.rs:2812, src/policy.rs:12` |  |  |
| `RS-0041` | `idiom` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/picker.rs:11-18, packages/d2b-provider-clipboa` |  |  |
| `RS-0038` | `idiom` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `src/bin/d2b-clipd.rs:1131` |  |  |
| `RS-0042` | `idiom` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/clipd_host/fallback.rs:40-46, packages/d2b-pro` |  |  |
| `RS-0043` | `idiom` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/clipd_host/policy.rs:22-42, packages/d2b-provi` |  |  |
| `RS-0044` | `idiom` | `d2b-provider-config-nixos` | low | actionable | leaf |  |  |  | `packages/d2b-provider-config-nixos/src/ttrpc.rs:379-386, packages/d2b-provider-config-nixo` |  |  |
| `RS-0045` | `idiom` | `d2b-provider-credential-entra` | medium | actionable | family |  |  |  | `packages/d2b-provider-credential-entra/src/lib.rs:1164, packages/d2b-provider-credential-e` |  |  |
| `RS-0046` | `idiom` | `d2b-provider-credential-secret-service` | low | actionable | leaf |  |  |  | `packages/d2b-provider-credential-secret-service/src/lib.rs:446, packages/d2b-provider-cred` |  |  |
| `RS-0047` | `idiom` | `d2b-provider-device-gpu` | low | actionable | leaf |  |  |  | `packages/d2b-provider-device-gpu/src/authority.rs:401, packages/d2b-provider-device-gpu/sr` |  |  |
| `RS-0048` | `idiom` | `d2b-provider-device-gpu` | low | actionable | leaf |  |  |  | `packages/d2b-provider-device-gpu/src/effects_service.rs:193` |  |  |
| `RS-0049` | `idiom` | `d2b-provider-device-gpu` | low | actionable | leaf |  |  |  | `packages/d2b-provider-device-gpu/src/authority.rs:17, packages/d2b-provider-device-gpu/src` |  |  |
| `RS-0050` | `idiom` | `d2b-provider-device-security-key` | low | actionable | leaf |  |  |  | `packages/d2b-provider-device-security-key/src/driver.rs:380-390` |  |  |
| `RS-0051` | `idiom` | `d2b-provider-device-tpm` | low | actionable | leaf |  |  |  | `swtpm_argv.rs:160, lib.rs:63, lib.rs:65` |  |  |
| `RS-0052` | `idiom` | `d2b-provider-device-usbip` | low | actionable | leaf |  |  |  | `driver.rs:267-281` |  |  |
| `RS-0059` | `idiom` | `d2b-provider-display-wayland` | medium | actionable | leaf |  |  |  | `src/controller.rs:1442, src/spec.rs:385` |  |  |
| `RS-0053` | `idiom` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:620, packages/d2b-provid` |  |  |
| `RS-0058` | `idiom` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `src/controller.rs:464, src/process.rs:402, src/process.rs:676` |  |  |
| `RS-0054` | `idiom` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:1272, packages/d2b-provi` |  |  |
| `RS-0055` | `idiom` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-display-wayland/src/wayland_proxy/dmabuf.rs:790, packages/d2b-provid` |  |  |
| `RS-0056` | `idiom` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-display-wayland/src/wayland_proxy/decoration.rs:150` |  |  |
| `RS-0057` | `idiom` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-display-wayland/src/wayland_proxy/dmabuf.rs:820, packages/d2b-provid` |  |  |
| `RS-0061` | `idiom` | `d2b-provider-endpoint` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-endpoint/src/effects_service.rs:50-60, packages/d2b-provider-endpoin` |  |  |
| `RS-0060` | `idiom` | `d2b-provider-endpoint` | low | actionable | leaf |  |  |  | `packages/d2b-provider-endpoint/src/endpoint.rs:395-398` |  |  |
| `RS-0062` | `idiom` | `d2b-provider-guest-azure-virtual-machine` | low | actionable | leaf |  |  |  | `src/bootstrap.rs:138, src/bootstrap.rs:124` |  |  |
| `RS-0063` | `idiom` | `d2b-provider-guest-cloud-hypervisor` | low | actionable | leaf |  |  |  | `controller.rs:1712, controller.rs:1744` |  |  |
| `RS-0064` | `idiom` | `d2b-provider-guest-cloud-hypervisor` | low | actionable | leaf |  |  |  | `controller.rs:1722, controller.rs:1860, controller.rs:2042` |  |  |
| `RS-0065` | `idiom` | `d2b-provider-guest-cloud-hypervisor` | low | actionable | leaf |  |  |  | `shutdown.rs:487-494, shutdown.rs:666-673` |  |  |
| `RS-0066` | `idiom` | `d2b-provider-guest-cloud-hypervisor` | low | actionable | leaf |  |  |  | `bootstrap_graph.rs:131-139, bootstrap_graph.rs:417-422` |  |  |
| `RS-0067` | `idiom` | `d2b-provider-guest-qemu-media` | low | actionable | leaf |  |  |  | `packages/d2b-provider-guest-qemu-media/src/config.rs:93, packages/d2b-provider-guest-qemu-` |  |  |
| `RS-0068` | `idiom` | `d2b-provider-host` | low | actionable | leaf |  |  |  | `packages/d2b-provider-host/src/test_support.rs:185` |  |  |
| `RS-0069` | `idiom` | `d2b-provider-network-local` | low | actionable | leaf |  |  |  | `src/controller.rs:298-302` |  |  |
| `RS-0070` | `idiom` | `d2b-provider-network-local` | low | actionable | leaf |  |  |  | `src/driver.rs:377-393` |  |  |
| `RS-0071` | `idiom` | `d2b-provider-notification-desktop` | low | actionable | leaf |  |  |  | `packages/d2b-provider-notification-desktop/src/controller.rs:660-675` |  |  |
| `RS-0072` | `idiom` | `d2b-provider-notification-desktop` | low | actionable | leaf |  |  |  | `packages/d2b-provider-notification-desktop/src/descriptor.rs:43-44, packages/d2b-provider-` |  |  |
| `RS-0073` | `idiom` | `d2b-provider-process-systemd` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process-systemd/src/effects_service.rs:98, packages/d2b-provider-pro` |  |  |
| `RS-0074` | `idiom` | `d2b-provider-seccomp-profile` | low | actionable | leaf |  |  |  | `packages/d2b-provider-seccomp-profile/src/seccomp_profile.rs:43, packages/d2b-provider-sec` |  |  |
| `RS-0075` | `idiom` | `d2b-provider-supervisor` | low | actionable | leaf |  |  |  | `packages/d2b-provider-supervisor/src/systemd.rs:840` |  |  |
| `RS-0076` | `idiom` | `d2b-provider-supervisor` | low | actionable | leaf |  |  |  | `packages/d2b-provider-supervisor/src/broker.rs:1104-1130, packages/d2b-provider-supervisor` |  |  |
| `RS-0077` | `idiom` | `d2b-provider-system-core` | low | actionable | leaf |  |  |  | `src/user.rs:75, src/user.rs:76` |  |  |
| `RS-0078` | `idiom` | `d2b-provider-system-core` | low | actionable | leaf |  |  |  | `src/user.rs:232` |  |  |
| `RS-0079` | `idiom` | `d2b-provider-toolkit` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-toolkit/src/base/fd10.rs:926-941, packages/d2b-provider-toolkit/src/` |  |  |
| `RS-0080` | `idiom` | `d2b-provider-toolkit` | low | actionable | leaf |  |  |  | `packages/d2b-provider-toolkit/src/base/fd10.rs:545-597, packages/d2b-provider-toolkit/src/` |  |  |
| `RS-0081` | `idiom` | `d2b-provider-transport-azure-relay` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/credential_client.rs:236-239, packages/d2b` |  |  |
| `RS-0082` | `idiom` | `d2b-provider-transport-azure-relay` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/auth.rs:249-253` |  |  |
| `RS-0083` | `idiom` | `d2b-provider-transport-vsock` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-vsock/src/auth.rs:221-224` |  |  |
| `RS-0084` | `idiom` | `d2b-provider-volume` | low | actionable | leaf |  |  |  | `driver.rs:607-609` |  |  |
| `RS-0085` | `idiom` | `d2b-provider-volume-local` | low | actionable | leaf |  |  |  | `src/status.rs:33-38` |  |  |
| `RS-0086` | `idiom` | `d2b-provider-zone-link` | medium | actionable | family |  |  |  | `packages/d2b-provider-zone-link/src/zone_links.rs:60, packages/d2b-provider-zone-link/src/` |  |  |
| `RS-0087` | `idiom` | `d2b-resource-api` | low | actionable | leaf |  |  |  | `packages/d2b-resource-api/src/authz.rs:106, packages/d2b-resource-api/src/authz.rs:300, pa` |  |  |
| `RS-0088` | `idiom` | `d2b-resource-api` | low | actionable | leaf |  |  |  | `packages/d2b-resource-api/src/manager_backend/tests.rs:1045, packages/d2b-resource-api/src` |  |  |
| `RS-0089` | `idiom` | `d2b-resource-client` | low | actionable | leaf |  |  |  | `packages/d2b-resource-client/src/zone_client.rs:914, packages/d2b-resource-client/src/proc` |  |  |
| `RS-0090` | `idiom` | `d2b-resource-client` | low | actionable | leaf |  |  |  | `packages/d2b-resource-client/src/zone_client.rs:194, packages/d2b-resource-client/src/zone` |  |  |
| `RS-0091` | `idiom` | `d2b-resource-compiler` | medium | actionable | leaf |  |  |  | `packages/d2b-resource-compiler/src/lib.rs:2401, packages/d2b-resource-compiler/src/lib.rs:` |  |  |
| `RS-0092` | `idiom` | `d2b-resource-compiler` | low | actionable | leaf |  |  |  | `packages/d2b-resource-compiler/src/lib.rs:2402` |  |  |
| `RS-0093` | `idiom` | `d2b-resource-compiler` | low | actionable | leaf |  |  |  | `packages/d2b-resource-compiler/src/lib.rs:1724` |  |  |
| `RS-0094` | `idiom` | `d2b-resource-compiler` | low | actionable | leaf |  |  |  | `packages/d2b-resource-compiler/src/lib.rs:1913` |  |  |
| `RS-0095` | `idiom` | `d2b-resource-runtime` | low | actionable | leaf | applied | U2 | 4cbf82851 | `manager.rs` | derive Default on three unit structs; new() const kept |  |
| `RS-0097` | `idiom` | `d2b-resource-runtime` | low | actionable | leaf | declined | U2 |  | `target.rs` | sort_by_key and sort_by_cached_key both rejected by rustc 1.97 (lifetime may not live long enough; closure returns (&str,&str,&str) borrowing the element); kept sort_by |  |
| `RS-0096` | `idiom` | `d2b-resource-runtime` | low | actionable | leaf | applied | U2 | 4cbf82851 | `manager.rs` | shared free manager_rpc transport; both endpoints route through it |  |
| `RS-0098` | `idiom` | `d2b-resource-runtime` | low | actionable | leaf | applied | U2 | 69d4b615f | `target.rs` | derive Default on TargetDirectory |  |
| `RS-0099` | `idiom` | `d2b-resource-runtime` | low | actionable | leaf | applied | U2 | fb5ed9466 | `spec_store.rs` | impl FromStr for ResourceProvenance; store parses via str::parse |  |
| `RS-0100` | `idiom` | `d2b-session` | low | actionable | leaf |  |  |  | `engine.rs:1802, engine.rs:1803, engine.rs:1807` |  |  |
| `RS-0101` | `idiom` | `d2b-session` | low | actionable | leaf |  |  |  | `admission.rs:1593, admission.rs:956, admission.rs:958` |  |  |
| `RS-0102` | `idiom` | `d2b-sk-frontend` | low | actionable | leaf |  |  |  | `packages/d2b-sk-frontend/src/config.rs:178` |  |  |
| `RS-0103` | `idiom` | `d2b-unsafe-local-helper` | low | actionable | leaf |  |  |  | `packages/d2b-unsafe-local-helper/src/systemd.rs:260, packages/d2b-unsafe-local-helper/src/` |  |  |
| `RS-0104` | `idiom` | `d2b-zone-routing` | low | actionable | leaf |  |  |  | `packages/d2b-zone-routing/src/resolver.rs:144` |  |  |
| `RS-0105` | `idiom` | `d2b-zone-routing` | low | actionable | leaf |  |  |  | `packages/d2b-zone-routing/src/service.rs:311` |  |  |
| `RS-0106` | `idiom` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/resource_runtime.rs:9168, packages/d2bd/src/resource_runtime.rs:4555, pa` |  |  |
| `RS-0108` | `idiom` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:14699, packages/d2bd/src/composition.rs:14710, packages/d` |  |  |
| `RS-0109` | `idiom` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:20450-20461, packages/d2bd/src/composition.rs:20486-20498` |  |  |
| `RS-0111` | `idiom` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/provider_lifecycle.rs:78, packages/d2bd/src/resource_plane_v3.rs:2226` |  |  |
| `RS-0112` | `idiom` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/effect_service_actors.rs:268, packages/d2bd/src/effect_service_actors.rs` |  |  |
| `RS-0113` | `idiom` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/forward_rendezvous.rs:672, packages/d2bd/src/shared_provider_effects.rs:` |  |  |
| `RS-0107` | `idiom` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/resource_runtime.rs:6896` |  |  |
| `RS-0110` | `idiom` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:21306-21319, packages/d2bd/src/composition.rs:21291` |  |  |
| `RS-0114` | `idiom` | `d2bd-runtime` | low | actionable | leaf | applied | U2 | c44ccbd0b | `packages/d2bd-runtime/src/autostart.rs` | build_autostart_plan uses iterator partition into the two sorted Vec halves |  |
| `RS-0115` | `idiom` | `d2bd-runtime` | low | actionable | leaf | applied | U2 | c44ccbd0b | `packages/d2bd-runtime/src/unsafe_local_helper.rs` | fd extraction loops are filter_map+flatten collects |  |
| `RS-0116` | `idiom` | `d2bd-runtime` | low | actionable | leaf | applied | U2 | c44ccbd0b | `packages/d2bd-runtime/src/guest_mode.rs` | monotonic_tick deduped into runtime_util (LazyLock per repo std; lazy init preserved} |  |
| `RS-0117` | `idiom` | `d2bd-runtime` | low | actionable | leaf | applied | U2 | c44ccbd0b | `packages/d2bd-runtime/src/console_session.rs` | ConsoleSessionTable derives Default; manual impl deleted |  |
| `RS-0118` | `idiom` | `xtask` | medium | actionable | leaf | applied | U2 | 80037234d | `gen_layer_catalogs.rs` | string_array deleted; ten call sites rerouted through string_slice |  |
| `RS-0120` | `idiom` | `xtask` | medium | actionable | leaf | applied-variant | U2 | e5f3b7f25 | packages/xtask/src/main.rs | fields/variants extracted from the syn AST; type text sliced from source via a once-per-file line index so the emitted doc stays byte-identical (quote is not a direct dep and ToTokens spacing would change the generated doc; normalize_ws retained for byte-identical rendering) |  |
| `RS-0123` | `idiom` | `xtask` | low | actionable | leaf | applied | U2 | 7eef023f9 | `resource_type_authority.rs` | three statements reindented |  |
| `RS-0124` | `idiom` | `xtask` | low | actionable | leaf | applied | U2 | fc0353eb1 | `nix_inventories.rs` | applied-variant: generic S: AsRef<str> + Display standard params (caller with Vec<String> cannot feed &[&str]); call site passes STANDARD_RESOURCE_TYPES.as_slice() |  |
| `RS-0119` | `idiom` | `xtask` | low | actionable | leaf | applied | U2 | a8bc3bb0b | packages/xtask/src/gen_layer_catalogs.rs, packages/xtask/src/main.rs | ten surface_catalog blocks and the two protobuf redaction templates now raw strings; emitted text verified byte-identical (gen-layer-catalogs --check passes) |  |
| `RS-0121` | `idiom` | `xtask` | low | actionable | leaf | applied | U2 | bbd40b6fd | `main.rs` | dead corrupted sanitizer strip line deleted; marker appears nowhere in generated files |  |
| `RS-0122` | `idiom` | `xtask` | low | actionable | leaf | applied | U2 | 5f04f2119 | `provider_crate_policy.rs` | dead close-block reset replaced with scan end at closing brace |  |
| `RS-0964` | `own` | `X3-cross-crate-duplication` | medium | actionable | family |  |  |  | `packages/d2bd/src/resource_plane_v3.rs:3227, packages/d2b-resource-runtime/src/target.rs:4` |  |  |
| `RS-0150` | `own` | `d2b` | low | actionable | leaf | applied | U2 | d4e4604e4 | `packages/d2b/src/dispatch.rs` | cursor/page_token/reference moved into calls;call-site reassignment unchanged |  |
| `RS-0151` | `own` | `d2b` | low | actionable | leaf | applied | U2 | d4e4604e4 | `packages/d2b/src/dispatch.rs` | try_parse_from consumes raw_args by value (sole caller, never reused) |  |
| `RS-0152` | `own` | `d2b` | low | actionable | leaf | applied | U2 | d4e4604e4 | `packages/d2b/src/dispatch.rs` | host_error_envelope takes impl Into<String>;;&format! results move in directly |  |
| `RS-0149` | `own` | `d2b` | low | actionable | leaf | applied | U2 | 0a4f73a1f | `packages/d2b/src/doctor.rs` | json! literal clones dropped (schema_version, issue_kinds, issues) |  |
| `RS-0125` | `own` | `d2b-audit` | low | actionable | leaf |  |  |  | `packages/d2b-audit/src/operation.rs:79` |  |  |
| `RS-0129` | `own` | `d2b-broker` | low | actionable | leaf | applied | U2 | c3ac2bb59 | `packages/d2b-broker/src/ops/pidfd.rs` | redundant payload.argv.clone removed; impl param renamed _payload |  |
| `RS-0131` | `own` | `d2b-broker` | low | actionable | leaf | applied | U2 | b09261be0 | `packages/d2b-broker/src/ops/media.rs` | unwrap_or_else(/_/ vec![record]) in enroll; merged with RS-0630 docs in same commit |  |
| `RS-0130` | `own` | `d2b-broker` | low | actionable | leaf | applied | U2 | ac44605b7 | `packages/d2b-broker/src/envelope/mod.rs` | Bootstrap reply shrunk to Result<(),>; state.clone removal; merged with RS-0012/RS-0628 in same commit |  |
| `RS-0126` | `own` | `d2b-broker-composition` | low | actionable | leaf |  |  |  | `packages/d2b-broker-composition/src/dependency_surface.rs:251, packages/d2b-broker-composi` |  |  |
| `RS-0127` | `own` | `d2b-broker-composition` | low | actionable | leaf |  |  |  | `packages/d2b-broker-composition/src/dependency_surface.rs:272` |  |  |
| `RS-0128` | `own` | `d2b-broker-composition` | low | actionable | leaf |  |  |  | `packages/d2b-broker-composition/src/dependency_surface.rs:340, packages/d2b-broker-composi` |  |  |
| `RS-0132` | `own` | `d2b-bus` | low | actionable | leaf | applied | U2 | fbf92683a | `packages/d2b-bus/src/router.rs` | ScopedCommitTransport::validate added; authorization_request validates borrowed data instead of cloning |  |
| `RS-0133` | `own` | `d2b-bus` | low | actionable | leaf | applied | U2 | fbf92683a | `packages/d2b-bus/src/session/prologue.rs` | of_subject hashes &str slices via hash_resource_ref; digest byte-identical (full bus suite passed |  |
| `RS-0134` | `own` | `d2b-bus` | low | actionable | leaf | applied | U2 | fbf92683a | `packages/d2b-bus/src/session/contract.rs` | private verify_body() shared by verify()/revalidate(); clone removed |  |
| `RS-0137` | `own` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/telemetry_policy.rs:497, packages/d2b-contracts-pro` |  |  |
| `RS-0135` | `own` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/provider.rs:2497, packages/d2b-contracts-provider/s` |  |  |
| `RS-0138` | `own` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/credential_controller.rs:1591, packages/d2b-contrac` |  |  |
| `RS-0136` | `own` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/provider.rs:2438, packages/d2b-contracts-provider/s` |  |  |
| `RS-0139` | `own` | `d2b-contracts-resource` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/volume_state.rs:138` |  |  |
| `RS-0142` | `own` | `d2b-contracts-zone-session` | low | actionable | leaf |  |  |  | `zone_routing.rs:883` |  |  |
| `RS-0140` | `own` | `d2b-contracts-zone-session` | low | actionable | family |  |  |  | `src/v3/component_session.rs:456, src/v3/component_session.rs:473, src/v3/component_session` |  |  |
| `RS-0143` | `own` | `d2b-contracts-zone-session` | low | actionable | leaf |  |  |  | `resource_bundle.rs:944, resource_bundle.rs:149` |  |  |
| `RS-0141` | `own` | `d2b-contracts-zone-session` | low | actionable | family |  |  |  | `src/v3/resource_export.rs:545, src/v3/resource_export.rs:546` |  |  |
| `RS-0144` | `own` | `d2b-contracts-zone-session` | low | actionable | leaf |  |  |  | `services.rs:216` |  |  |
| `RS-0148` | `own` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/static_invariants.rs:201` |  |  |
| `RS-0147` | `own` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:1973, packages/d2b-core/src/bundle_resolver.rs:33` |  |  |
| `RS-0145` | `own` | `d2b-core-controller` | low | actionable | leaf |  |  |  | `owner_reconcile.rs:1072-1075` |  |  |
| `RS-0146` | `own` | `d2b-core-controller` | low | actionable | leaf |  |  |  | `authority.rs:2803, authority.rs:2806` |  |  |
| `RS-0153` | `own` | `d2b-process-conformance` | low | actionable | leaf |  |  |  | `packages/d2b-process-conformance/src/ticket.rs:557, packages/d2b-process-conformance/src/t` |  |  |
| `RS-0154` | `own` | `d2b-provider` | low | actionable | leaf |  |  |  | `packages/d2b-provider/src/agent.rs:290, packages/d2b-provider/src/agent.rs:299-304` |  |  |
| `RS-0155` | `own` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/clipd_host/wayland.rs:257-263` |  |  |
| `RS-0156` | `own` | `d2b-provider-config-nixos` | low | actionable | leaf |  |  |  | `packages/d2b-provider-config-nixos/src/ttrpc.rs:111, packages/d2b-provider-config-nixos/sr` |  |  |
| `RS-0157` | `own` | `d2b-provider-credential` | low | actionable | leaf |  |  |  | `packages/d2b-provider-credential/src/driver.rs:342, packages/d2b-provider-credential/src/t` |  |  |
| `RS-0158` | `own` | `d2b-provider-device-gpu` | low | actionable | leaf |  |  |  | `packages/d2b-provider-device-gpu/src/controller.rs:272, packages/d2b-provider-device-gpu/s` |  |  |
| `RS-0159` | `own` | `d2b-provider-device-tpm` | low | actionable | leaf |  |  |  | `resource_controller.rs:233, resource_controller.rs:247, effects_service.rs:280, effects_se` |  |  |
| `RS-0160` | `own` | `d2b-provider-device-usbip` | low | actionable | leaf |  |  |  | `broker.rs:194-208, broker.rs:218-232, broker.rs:313-321, broker.rs:333-341` |  |  |
| `RS-0161` | `own` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-display-wayland/src/wayland_proxy/policy.rs:186, packages/d2b-provid` |  |  |
| `RS-0162` | `own` | `d2b-provider-guest` | low | actionable | leaf |  |  |  | `packages/d2b-provider-guest/src/driver.rs:981` |  |  |
| `RS-0163` | `own` | `d2b-provider-guest` | low | actionable | leaf |  |  |  | `packages/d2b-provider-guest/src/effects_service.rs:243, packages/d2b-provider-guest/src/ef` |  |  |
| `RS-0164` | `own` | `d2b-provider-guest-azure-container-apps` | low | actionable | leaf |  |  |  | `src/controller.rs:156-157` |  |  |
| `RS-0165` | `own` | `d2b-provider-guest-azure-container-apps` | low | actionable | leaf |  |  |  | `src/controller.rs:500-501` |  |  |
| `RS-0166` | `own` | `d2b-provider-guest-azure-container-apps` | low | actionable | leaf |  |  |  | `src/controller.rs:527` |  |  |
| `RS-0167` | `own` | `d2b-provider-guest-azure-container-apps` | low | actionable | leaf |  |  |  | `src/controller.rs:417-419, src/controller.rs:469-471` |  |  |
| `RS-0168` | `own` | `d2b-provider-guest-azure-container-apps` | low | actionable | leaf |  |  |  | `src/controller.rs:892, src/controller.rs:903` |  |  |
| `RS-0169` | `own` | `d2b-provider-guest-azure-container-apps` | low | actionable | leaf |  |  |  | `src/effects.rs:450-464` |  |  |
| `RS-0170` | `own` | `d2b-provider-guest-azure-virtual-machine` | medium | actionable | leaf |  |  |  | `src/controller/mod.rs:791, src/bootstrap.rs:42` |  |  |
| `RS-0171` | `own` | `d2b-provider-guest-azure-virtual-machine` | low | actionable | leaf |  |  |  | `src/controller/mod.rs:640, src/controller/mod.rs:764` |  |  |
| `RS-0172` | `own` | `d2b-provider-guest-azure-virtual-machine` | low | actionable | leaf |  |  |  | `src/controller/mod.rs:852` |  |  |
| `RS-0173` | `own` | `d2b-provider-guest-azure-virtual-machine` | low | actionable | leaf |  |  |  | `src/controller/mod.rs:1046` |  |  |
| `RS-0174` | `own` | `d2b-provider-guest-qemu-media` | low | actionable | leaf |  |  |  | `packages/d2b-provider-guest-qemu-media/src/qmp/mod.rs:266` |  |  |
| `RS-0175` | `own` | `d2b-provider-host` | low | actionable | leaf |  |  |  | `packages/d2b-provider-host/src/driver.rs:263, packages/d2b-provider-host/src/driver.rs:266` |  |  |
| `RS-0176` | `own` | `d2b-provider-network-local` | low | actionable | leaf |  |  |  | `src/controller.rs:416-417` |  |  |
| `RS-0177` | `own` | `d2b-provider-notification-desktop` | low | actionable | leaf |  |  |  | `packages/d2b-provider-notification-desktop/src/controller.rs:1033, packages/d2b-provider-n` |  |  |
| `RS-0178` | `own` | `d2b-provider-notification-desktop` | low | actionable | leaf |  |  |  | `packages/d2b-provider-notification-desktop/src/lifecycle.rs:338, packages/d2b-provider-not` |  |  |
| `RS-0179` | `own` | `d2b-provider-observability-otel` | low | actionable | leaf |  |  |  | `agent.rs:255, agent.rs:256, agent.rs:283, agent.rs:285` |  |  |
| `RS-0180` | `own` | `d2b-provider-process` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process/src/driver.rs:1988, packages/d2b-provider-process/src/driver` |  |  |
| `RS-0181` | `own` | `d2b-provider-process` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process/src/operations.rs:1354, packages/d2b-provider-process/src/op` |  |  |
| `RS-0182` | `own` | `d2b-provider-provider` | low | actionable | leaf |  |  |  | `src/driver.rs:355, src/driver.rs:478, src/driver.rs:522` |  |  |
| `RS-0183` | `own` | `d2b-provider-provider` | low | actionable | leaf |  |  |  | `src/driver.rs:360, src/driver.rs:435` |  |  |
| `RS-0184` | `own` | `d2b-provider-shell-terminal` | low | actionable | leaf |  |  |  | `src/service/supervisor.rs:599, src/service/supervisor.rs:601` |  |  |
| `RS-0185` | `own` | `d2b-provider-system-core` | low | actionable | leaf |  |  |  | `src/host.rs:466, src/host.rs:467` |  |  |
| `RS-0187` | `own` | `d2b-provider-toolkit` | low | actionable | family |  |  |  | `packages/d2b-provider-toolkit/src/server/adapter.rs:331-334` |  |  |
| `RS-0188` | `own` | `d2b-provider-toolkit` | low | actionable | family |  |  |  | `packages/d2b-provider-toolkit/src/server/adapter.rs:305, packages/d2b-provider-toolkit/src` |  |  |
| `RS-0186` | `own` | `d2b-provider-toolkit` | low | actionable | leaf |  |  |  | `packages/d2b-provider-toolkit/src/base/runtime.rs:530-537` |  |  |
| `RS-0189` | `own` | `d2b-provider-toolkit` | low | actionable | leaf |  |  |  | `packages/d2b-provider-toolkit/src/shared_provider.rs:771` |  |  |
| `RS-0190` | `own` | `d2b-provider-transport-azure-relay` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/credential_client.rs:190-198, packages/d2b` |  |  |
| `RS-0191` | `own` | `d2b-provider-transport-azure-relay` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/guest_credential.rs:267-277, packages/d2b-` |  |  |
| `RS-0192` | `own` | `d2b-provider-user` | low | actionable | leaf |  |  |  | `packages/d2b-provider-user/src/effects_service.rs:179, packages/d2b-provider-user/src/effe` |  |  |
| `RS-0193` | `own` | `d2b-provider-volume` | low | actionable | leaf |  |  |  | `driver.rs:337` |  |  |
| `RS-0194` | `own` | `d2b-provider-volume` | low | actionable | wide |  |  |  | `driver.rs:380, d2b-provider-volume-local/src/bindings.rs:80-81, d2bd/src/resource_runtime.` |  |  |
| `RS-0195` | `own` | `d2b-provider-volume-binding` | low | actionable | leaf |  |  |  | `packages/d2b-provider-volume-binding/src/row_readers.rs:38-44` |  |  |
| `RS-0196` | `own` | `d2b-provider-zone-link` | low | actionable | leaf |  |  |  | `packages/d2b-provider-zone-link/src/zone_links.rs:1692, packages/d2b-provider-zone-link/sr` |  |  |
| `RS-0197` | `own` | `d2b-provider-zone-link` | low | actionable | leaf |  |  |  | `packages/d2b-provider-zone-link/src/zone_links.rs:1526` |  |  |
| `RS-0198` | `own` | `d2b-resource-api` | low | actionable | leaf |  |  |  | `adapter.rs:425, client.rs:110, service.rs:852` |  |  |
| `RS-0199` | `own` | `d2b-resource-api` | low | actionable | leaf |  |  |  | `packages/d2b-resource-api/src/admission.rs:338, packages/d2b-resource-api/src/admission.rs` |  |  |
| `RS-0200` | `own` | `d2b-resource-client` | low | actionable | family |  |  |  | `packages/d2b-resource-client/src/target.rs:155, packages/d2b-resource-client/src/target.rs` |  |  |
| `RS-0202` | `own` | `d2b-resource-compiler` | medium | actionable | leaf |  |  |  | `packages/d2b-resource-compiler/src/linux.rs:93, packages/d2b-resource-compiler/src/linux.r` |  |  |
| `RS-0201` | `own` | `d2b-resource-compiler` | low | actionable | leaf |  |  |  | `packages/d2b-resource-compiler/src/main.rs:1148, packages/d2b-resource-compiler/src/main.r` |  |  |
| `RS-0203` | `own` | `d2b-resource-compiler` | low | actionable | leaf |  |  |  | `packages/d2b-resource-compiler/src/main.rs:1467, packages/d2b-resource-compiler/src/main.r` |  |  |
| `RS-0204` | `own` | `d2b-resource-compiler` | low | actionable | leaf |  |  |  | `packages/d2b-resource-compiler/src/main.rs:269, packages/d2b-resource-compiler/src/main.rs` |  |  |
| `RS-0205` | `own` | `d2b-resource-compiler` | low | actionable | leaf |  |  |  | `packages/d2b-resource-compiler/src/main.rs:704` |  |  |
| `RS-0206` | `own` | `d2b-resource-runtime` | low | actionable | leaf | applied | U2 | 4cbf82851 | `manager.rs` | observed_status filters before clone |  |
| `RS-0209` | `own` | `d2b-resource-runtime` | low | actionable | leaf | applied | U2 | fb5ed9466 | `spec_store.rs` | insert_new takes StoredDesiredResource by value; ensure passes by move |  |
| `RS-0207` | `own` | `d2b-resource-runtime` | low | actionable | leaf | applied | U2 | 0e6061c1e | `resource.rs` | pre_start moves row into state; clones only for ResourceContext::new |  |
| `RS-0210` | `own` | `d2b-resource-runtime` | low | actionable | leaf | applied | U2 | fb5ed9466 | `spec_store.rs` | list binds borrowed selector str forms |  |
| `RS-0208` | `own` | `d2b-resource-runtime` | low | actionable | leaf | declined | U2 |  | `metadata.rs` | ctx.spec::<Value>() returns Result<&Value,_>; Ok(spec) is E0308; the clone is required by the API |  |
| `RS-0211` | `own` | `d2b-resource-runtime` | low | actionable | leaf | applied | U2 | 1f9423d9a | `target.rs` | assign moves assignment into map and clones once for return |  |
| `RS-0212` | `own` | `d2b-resource-runtime` | low | actionable | leaf | applied | U2 | 75d64d039 | `guest_target.rs` | json_object moves member values; 17 call sites updated |  |
| `RS-0213` | `own` | `d2b-session` | low | actionable | family |  |  |  | `engine.rs:689, admission.rs:593` |  |  |
| `RS-0214` | `own` | `d2b-sk-frontend` | low | actionable | leaf |  |  |  | `packages/d2b-sk-frontend/src/main.rs:55, packages/d2b-sk-frontend/src/config.rs:83` |  |  |
| `RS-0215` | `own` | `d2b-zone-routing` | low | actionable | leaf |  |  |  | `packages/d2b-zone-routing/src/engine.rs:345, packages/d2b-zone-routing/src/engine.rs:346` |  |  |
| `RS-0221` | `own` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/interaction_composition.rs:4343, packages/d2bd/src/interaction_compositi` |  |  |
| `RS-0222` | `own` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/process_provider_runtime.rs:4057` |  |  |
| `RS-0216` | `own` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/resource_runtime.rs:359` |  |  |
| `RS-0218` | `own` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:20404` |  |  |
| `RS-0217` | `own` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/resource_runtime.rs:181, packages/d2bd/src/resource_runtime.rs:4625, pac` |  |  |
| `RS-0219` | `own` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:21091` |  |  |
| `RS-0223` | `own` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/forward_rendezvous.rs:456, packages/d2bd/src/forward_rendezvous.rs:458-4` |  |  |
| `RS-0220` | `own` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:20638` |  |  |
| `RS-0224` | `own` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/shared_provider_effects.rs:690, packages/d2bd/src/shared_provider_effect` |  |  |
| `RS-0225` | `own` | `d2bd-runtime` | low | actionable | leaf | applied | U2 | c44ccbd0b | `packages/d2bd-runtime/src/supervisor/dag.rs` | run_split matches &state, binds reason by ref,and moves state into api_ready afterwards |  |
| `RS-0226` | `own` | `d2bd-runtime` | low | actionable | leaf | applied-variant | U2 | c44ccbd0b | `packages/d2bd-runtime/src/unsafe_local_helper.rs` | complete_pending takes &str; two call sites pass as_str; third kept to_string because E0505 forbids borrow+move of result in one call (deviation) |  |
| `RS-0227` | `own` | `d2bd-runtime` | low | actionable | leaf | applied | U2 | c44ccbd0b | `packages/d2bd-runtime/src/console_session.rs` | Borrow<str> implemented; five map lookups/removes resolve without String alloc |  |
| `RS-0228` | `own` | `d2bd-runtime` | low | actionable | leaf | applied-variant | U2 | c44ccbd0b | `packages/d2bd-runtime/src/daemon_audit.rs` | write_event* and enqueue take DaemonEvent by value, drop clone; caller migration in d2bd/src/composition.rs left to W1Daemon/orchestrator (cross-crate} |  |
| `RS-0231` | `own` | `xtask` | low | actionable | leaf | applied | U2 | 5f04f2119 | `provider_crate_policy.rs` | check_members takes &[WorkspaceMember]; caller clones dropped; second caller borrows result |  |
| `RS-0236` | `own` | `xtask` | low | actionable | leaf | declined | U2 |  | `production_closure.rs` | compute_* take ContextSpec by value today; ComputedContext struct owns spec; changing to &ContextSpec forces internal clones at the struct literals (= no net clone removal; E0308 evidence); reverted |  |
| `RS-0238` | `own` | `xtask` | low | actionable | leaf | applied | U2 | 16e0b183d | packages/xtask/src/blocking_census.rs | CensusBaseline built by consuming each CrateCensus; --baseline check driven from the written map via a shared check_against_baseline helper; 18 census tests pass |  |
| `RS-0232` | `own` | `xtask` | low | actionable | leaf | applied | U2 | 5f04f2119 | `provider_crate_policy.rs` | family-knowledge exempt set keys &str pairs; probe via as_str |  |
| `RS-0237` | `own` | `xtask` | low | actionable | leaf | applied | U2 | be3e0744b | `production_closure.rs` | duplicate approval clone binding removed; single clone at with_approval call |  |
| `RS-0234` | `own` | `xtask` | low | actionable | leaf | applied | U2 | 44a90ab5e | `provider_packaging.rs` | nix_string_list generic over AsRef<str>; eight to_owned closures deleted |  |
| `RS-0233` | `own` | `xtask` | low | actionable | leaf | applied | U2 | f01a683c7 | `gen_broker_operations.rs` | profile_catalog returns Vec<&str> via as_deref; string_list items are &str |  |
| `RS-0235` | `own` | `xtask` | low | actionable | leaf | applied | U2 | 02238dbdd | `semantic_service_schemas.rs` | resource_ref_schema takes &str/&[&str]; five call sites pass borrowed forms |  |
| `RS-0229` | `own` | `xtask` | low | actionable | leaf | applied | U2 | 5f04f2119 | `provider_crate_policy.rs` | mem::take on the mut slot before in-place edit |  |
| `RS-0230` | `own` | `xtask` | low | actionable | leaf | applied | U2 | 5f04f2119 | `provider_crate_policy.rs` | two ratchet probes key borrowed strs via signal fields |  |
| `RS-0954` | `type` | `X2-generated-boundary` | medium | actionable | family |  |  |  | `packages/xtask/src/gen_broker_operations.rs:979-987, packages/xtask/src/gen_broker_operati` |  |  |
| `RS-0955` | `type` | `X2-generated-boundary` | medium | actionable | leaf |  |  |  | `packages/xtask/src/gen_broker_operations.rs:949, packages/d2b-broker/src/generated/broker_` |  |  |
| `RS-0956` | `type` | `X2-generated-boundary` | low | actionable | family |  |  |  | `packages/xtask/src/gen_broker_operations.rs:853-858, packages/xtask/src/gen_broker_operati` |  |  |
| `RS-0957` | `type` | `X2-generated-boundary` | low | actionable | leaf |  |  |  | `packages/xtask/src/gen_broker_operations.rs:891, packages/d2b-core/src/generated/broker_op` |  |  |
| `RS-0962` | `type` | `X3-cross-crate-duplication` | high | actionable | family | escalated | U1 |  | `packages/d2b-provider-wayland-policy/src/interaction.rs:428, packages/d2b-provider-volume-` | wave 0 applied the panicking constructor member (RS-0516); the five remaining provider-crate zone/key_ref member sites are the family wave's | U5 |
| `RS-0263` | `type` | `d2b` | low | actionable | leaf |  |  |  | `context.rs:713, context.rs:2751, context.rs:801` |  |  |
| `RS-0264` | `type` | `d2b` | low | actionable | leaf |  |  |  | `packages/d2b/src/exec.rs:90, packages/d2b/src/exec.rs:345, packages/d2b/src/endpoint.rs:34` |  |  |
| `RS-0239` | `type` | `d2b-audit` | medium | actionable | wide |  |  |  | `packages/d2b-audit/src/evidence_chain.rs:50, packages/d2b-audit/src/evidence_chain.rs:115,` |  |  |
| `RS-0242` | `type` | `d2b-broker` | medium | actionable | leaf |  |  |  | `packages/d2b-broker/src/ops/media.rs:889-892` |  |  |
| `RS-0240` | `type` | `d2b-broker` | low | needs-contract | leaf |  |  |  | `packages/d2b-broker/src/ops/storage_contract.rs:24, packages/d2b-broker/src/ops/storage_co` |  |  |
| `RS-0241` | `type` | `d2b-broker` | low | needs-contract | leaf |  |  |  | `packages/d2b-broker/src/live_handlers.rs:291, packages/d2b-broker/src/live_handlers.rs:362` |  |  |
| `RS-0245` | `type` | `d2b-bus` | medium | actionable | leaf |  |  |  | `packages/d2b-bus/src/session/zone_link.rs:111-117` |  |  |
| `RS-0243` | `type` | `d2b-bus` | low | actionable | leaf |  |  |  | `packages/d2b-bus/src/router.rs:218-219, packages/d2b-bus/src/router.rs:298-337` |  |  |
| `RS-0244` | `type` | `d2b-bus` | low | actionable | leaf |  |  |  | `packages/d2b-bus/src/router.rs:1445-1446, packages/d2b-bus/src/router.rs:1667-1674` |  |  |
| `RS-0246` | `type` | `d2b-contracts-broker` | medium | actionable | leaf |  |  |  | `packages/d2b-contracts-broker/src/host_generation.rs:226-232, packages/d2b-contracts-broke` |  |  |
| `RS-0247` | `type` | `d2b-contracts-broker` | medium | actionable | leaf |  |  |  | `packages/d2b-contracts-broker/src/broker_wire.rs:2805, packages/d2b-contracts-broker/src/b` |  |  |
| `RS-0248` | `type` | `d2b-contracts-control` | medium | needs-contract | wide |  |  |  | `public_wire.rs:2166, public_wire.rs:2203` |  |  |
| `RS-0249` | `type` | `d2b-contracts-control` | low | needs-contract | wide |  |  |  | `cli_output.rs:99, cli_output.rs:123, public_wire.rs:2634, public_wire.rs:2672` |  |  |
| `RS-0250` | `type` | `d2b-contracts-control` | low | needs-contract | wide |  |  |  | `public_wire.rs:316, public_wire.rs:311` |  |  |
| `RS-0253` | `type` | `d2b-contracts-provider` | medium | actionable | family |  |  |  | `packages/d2b-contracts-provider/src/v3/semantic_services/child_resources.rs:92, packages/d` |  |  |
| `RS-0251` | `type` | `d2b-contracts-provider` | low | actionable | family |  |  |  | `packages/d2b-contracts-provider/src/v3/provider.rs:1366, packages/d2b-contracts-provider/s` |  |  |
| `RS-0252` | `type` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/provider.rs:1333, packages/d2b-contracts-provider/s` |  |  |
| `RS-0256` | `type` | `d2b-contracts-resource` | medium | needs-contract | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/activation_nixos.rs:285, packages/d2b-contracts-res` |  |  |
| `RS-0255` | `type` | `d2b-contracts-resource` | medium | actionable | family |  |  |  | `packages/d2b-contracts-resource/src/v3/operations/mod.rs:71, packages/d2b-contracts-resour` |  |  |
| `RS-0257` | `type` | `d2b-contracts-resource` | medium | needs-contract | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/activation_nixos.rs:46-47, packages/d2b-contracts-r` |  |  |
| `RS-0254` | `type` | `d2b-contracts-resource` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/operations/seal.rs:57, packages/d2b-contracts-resou` |  |  |
| `RS-0258` | `type` | `d2b-contracts-zone-session` | low | actionable | leaf |  |  |  | `role_binding.rs:179-182, role_binding.rs:149-162` |  |  |
| `RS-0259` | `type` | `d2b-controller-toolkit` | low | actionable | family |  |  |  | `packages/d2b-controller-toolkit/src/context.rs:17-18, packages/d2b-controller-toolkit/src/` |  |  |
| `RS-0261` | `type` | `d2b-core` | medium | actionable | leaf |  |  |  | `packages/d2b-core/src/manifest_v04.rs:313` |  |  |
| `RS-0260` | `type` | `d2b-core-controller` | low | actionable | leaf |  |  |  | `authority.rs:985-988` |  |  |
| `RS-0262` | `type` | `d2b-host` | medium | actionable | family |  |  |  | `packages/d2b-host/src/nftables.rs:609, packages/d2b-broker/src/ops/media.rs:194` |  |  |
| `RS-0265` | `type` | `d2b-process-conformance` | low | actionable | leaf |  |  |  | `packages/d2b-process-conformance/src/ticket.rs:387, packages/d2b-process-conformance/src/t` |  |  |
| `RS-0266` | `type` | `d2b-provider-audio-pipewire` | low | actionable | leaf |  |  |  | `src/resource_type.rs:64-70, src/resource_type.rs:117-125, src/resource_type.rs:206-231, sr` |  |  |
| `RS-0267` | `type` | `d2b-provider-audio-pipewire` | low | actionable | leaf |  |  |  | `src/controller.rs:589-598, src/controller.rs:602-608, src/controller.rs:199` |  |  |
| `RS-0268` | `type` | `d2b-provider-clipboard-wayland` | medium | actionable | leaf |  |  |  | `src/bin/d2b-clipd.rs:140, src/bin/d2b-clipd.rs:142` |  |  |
| `RS-0269` | `type` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/controller/mod.rs:13-41, packages/d2b-provider` |  |  |
| `RS-0270` | `type` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/picker.rs:258-264, packages/d2b-provider-clipb` |  |  |
| `RS-0271` | `type` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/picker.rs:247, packages/d2b-provider-clipboard` |  |  |
| `RS-0272` | `type` | `d2b-provider-config-nixos` | low | actionable | leaf |  |  |  | `packages/d2b-provider-config-nixos/src/service.rs:31-34, packages/d2b-provider-config-nixo` |  |  |
| `RS-0273` | `type` | `d2b-provider-credential` | low | actionable | leaf |  |  |  | `packages/d2b-provider-credential/src/driver.rs:295, packages/d2b-provider-credential/src/d` |  |  |
| `RS-0274` | `type` | `d2b-provider-credential-managed-identity` | medium | actionable | leaf |  |  |  | `controller.rs:85-89, tests/binding.rs:1214-1234` |  |  |
| `RS-0275` | `type` | `d2b-provider-device-gpu` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-device-gpu/src/controller.rs:79, packages/d2b-provider-device-gpu/sr` |  |  |
| `RS-0276` | `type` | `d2b-provider-guest` | medium | actionable | family |  |  |  | `packages/d2b-provider-guest/src/driver.rs:709, packages/d2b-provider-guest/src/driver.rs:7` |  |  |
| `RS-0277` | `type` | `d2b-provider-guest-azure-container-apps` | medium | actionable | leaf |  |  |  | `src/effects.rs:394-406` |  |  |
| `RS-0278` | `type` | `d2b-provider-guest-azure-virtual-machine` | medium | actionable | leaf |  |  |  | `src/controller/mod.rs:278, src/controller/mod.rs:186` |  |  |
| `RS-0279` | `type` | `d2b-provider-guest-azure-virtual-machine` | medium | actionable | leaf |  |  |  | `src/bootstrap.rs:65, src/bootstrap.rs:82` |  |  |
| `RS-0280` | `type` | `d2b-provider-guest-azure-virtual-machine` | low | actionable | leaf |  |  |  | `src/controller/mod.rs:82, src/controller/mod.rs:982` |  |  |
| `RS-0281` | `type` | `d2b-provider-guest-cloud-hypervisor` | medium | actionable | leaf |  |  |  | `identity.rs:857-865, controller.rs:1808-1818` |  |  |
| `RS-0282` | `type` | `d2b-provider-guest-cloud-hypervisor` | medium | needs-contract | leaf |  |  |  | `config.rs:20, config.rs:53` |  |  |
| `RS-0283` | `type` | `d2b-provider-guest-cloud-hypervisor` | medium | actionable | leaf |  |  |  | `bootstrap_graph.rs:142-176, controller.rs:662-670` |  |  |
| `RS-0284` | `type` | `d2b-provider-guest-qemu-media` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-guest-qemu-media/src/types/guest.rs:417, packages/d2b-contracts-reso` |  |  |
| `RS-0285` | `type` | `d2b-provider-guest-qemu-media` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-guest-qemu-media/src/config.rs:89, packages/d2b-provider-guest-qemu-` |  |  |
| `RS-0286` | `type` | `d2b-provider-notification-desktop` | low | policy-confirmed | leaf |  |  |  | `packages/d2b-provider-notification-desktop/src/controller.rs:22-27, docs/explanation/over-` |  |  |
| `RS-0287` | `type` | `d2b-provider-observability-otel` | low | actionable | leaf |  |  |  | `agent.rs:56, agent.rs:265, agent.rs:297` |  |  |
| `RS-0288` | `type` | `d2b-provider-process-minijail` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process-minijail/src/lib.rs:151, packages/d2b-provider-process-minij` |  |  |
| `RS-0289` | `type` | `d2b-provider-process-systemd` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process-systemd/src/lifecycle.rs:78, packages/d2b-provider-process-s` |  |  |
| `RS-0290` | `type` | `d2b-provider-process-systemd` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process-systemd/src/metrics.rs:7, packages/d2b-provider-process-syst` |  |  |
| `RS-0291` | `type` | `d2b-provider-telemetry-binding` | low | actionable | leaf |  |  |  | `packages/d2b-provider-telemetry-binding/src/driver.rs:168, packages/d2b-provider-telemetry` |  |  |
| `RS-0292` | `type` | `d2b-provider-telemetry-service` | low | actionable | leaf |  |  |  | `packages/d2b-provider-telemetry-service/src/driver.rs:119-125, packages/d2b-provider-telem` |  |  |
| `RS-0293` | `type` | `d2b-provider-transport-azure-relay` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/guest_zone_link.rs:149-150, packages/d2b-p` |  |  |
| `RS-0294` | `type` | `d2b-provider-volume-local` | low | actionable | leaf |  |  |  | `src/identity.rs:72-88, src/identity.rs:146-150` |  |  |
| `RS-0295` | `type` | `d2b-resource-api` | low | actionable | leaf |  |  |  | `adapter.rs:124` |  |  |
| `RS-0296` | `type` | `d2b-resource-client` | low | actionable | leaf |  |  |  | `packages/d2b-resource-client/src/call.rs:168, packages/d2b-resource-client/src/dispatch.rs` |  |  |
| `RS-0297` | `type` | `d2b-session` | low | actionable | leaf |  |  |  | `scheduler.rs:18-21, scheduler.rs:66-68` |  |  |
| `RS-0298` | `type` | `d2b-session` | low | actionable | leaf |  |  |  | `engine.rs:1702, engine.rs:1295, engine.rs:31` |  |  |
| `RS-0299` | `type` | `d2b-unsafe-local-helper` | low | actionable | leaf |  |  |  | `packages/d2b-unsafe-local-helper/src/runtime.rs:155, packages/d2b-unsafe-local-helper/src/` |  |  |
| `RS-0303` | `type` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:18659, packages/d2bd/src/composition.rs:18608` |  |  |
| `RS-0305` | `type` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/shared_provider_effects.rs:1299, packages/d2bd/src/shared_provider_effec` |  |  |
| `RS-0302` | `type` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:16152, packages/d2bd/src/composition.rs:16155, packages/d` |  |  |
| `RS-0300` | `type` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/resource_runtime.rs:3041, packages/d2bd/src/resource_runtime.rs:3230, pa` |  |  |
| `RS-0301` | `type` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/resource_runtime.rs:1014, packages/d2bd/src/resource_runtime.rs:7614` |  |  |
| `RS-0304` | `type` | `d2bd` | low | needs-contract | leaf |  |  |  | `packages/d2bd/src/composition.rs:20146, packages/d2bd/src/composition.rs:20280, packages/d` |  |  |
| `RS-0307` | `type` | `d2bd-runtime` | medium | actionable | leaf |  |  |  | `resource_operator_activation.rs:129-182, resource_operator_activation.rs:163-177` |  |  |
| `RS-0309` | `type` | `d2bd-runtime` | medium | actionable | family |  |  |  | `packages/d2bd-runtime/src/daemon_audit.rs:194, packages/d2bd/src/composition.rs:19361` |  |  |
| `RS-0310` | `type` | `d2bd-runtime` | medium | actionable | family |  |  |  | `packages/d2bd-runtime/src/wire_response_helpers.rs:99, packages/d2bd-runtime/src/wire_resp` |  |  |
| `RS-0308` | `type` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `admission.rs:107-108, admission.rs:66, admission.rs:83` |  |  |
| `RS-0306` | `type` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `component_session_vsock.rs:32-36` |  |  |
| `RS-0311` | `type` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `packages/d2bd-runtime/src/typed_shell_targets.rs:13, packages/d2bd-runtime/src/typed_shell` |  |  |
| `RS-0314` | `type` | `xtask` | medium | actionable | leaf |  |  |  | `packages/xtask/src/inventory.rs:230, packages/xtask/src/delivery/model.rs:557` |  |  |
| `RS-0313` | `type` | `xtask` | low | actionable | leaf |  |  |  | `packages/xtask/src/provider_crate_policy.rs:5104, packages/xtask/src/provider_crate_policy` |  |  |
| `RS-0315` | `type` | `xtask` | low | needs-contract | leaf |  |  |  | `packages/xtask/src/production_closure.rs:107, packages/xtask/src/production_closure.rs:660` |  |  |
| `RS-0312` | `type` | `xtask` | low | actionable | leaf |  |  |  | `packages/xtask/src/gen_layer_catalogs.rs:288, packages/xtask/src/gen_layer_catalogs.rs:515` |  |  |
| `RS-0952` | `api` | `X2-generated-boundary` | medium | actionable | leaf |  |  |  | `packages/d2b-resource-api/src/generated/mod.rs:3-4, packages/xtask/src/main.rs:355-370, pa` |  |  |
| `RS-0953` | `api` | `X2-generated-boundary` | low | actionable | leaf |  |  |  | `packages/d2b-audit/src/lib.rs:7, packages/xtask/src/gen_layer_catalogs.rs:725, packages/d2` |  |  |
| `RS-0965` | `api` | `X3-cross-crate-duplication` | low | actionable | family |  |  |  | `packages/d2b-provider-device-usbip/src/lib.rs:24, packages/d2b-provider-seccomp-profile/sr` |  |  |
| `RS-0352` | `api` | `d2b` | medium | actionable | leaf |  |  |  | `packages/d2b/src/host_generation.rs:7, packages/d2b/src/host_generation.rs:17, packages/d2` |  |  |
| `RS-0353` | `api` | `d2b` | low | actionable | leaf |  |  |  | `zone_support_bundle.rs:19, zone_support_bundle.rs:28, zone_support_bundle.rs:99, zone_supp` |  |  |
| `RS-0354` | `api` | `d2b` | low | actionable | leaf |  |  |  | `packages/d2b/src/lib.rs:25, packages/d2b/src/lib.rs:41, packages/d2b/src/doctor.rs:62, pac` |  |  |
| `RS-0316` | `api` | `d2b-audit` | medium | actionable | leaf |  |  |  | `packages/d2b-audit/src/lib.rs:16, packages/d2b-audit/src/lib.rs:30, packages/d2b-audit/src` |  |  |
| `RS-0319` | `api` | `d2b-broker` | medium | actionable | leaf |  |  |  | `packages/d2b-broker/src/ops/sysctl.rs:16, packages/d2b-broker/src/ops/sysctl.rs:84` |  |  |
| `RS-0320` | `api` | `d2b-broker` | medium | actionable | leaf |  |  |  | `packages/d2b-broker/src/ops/route.rs:19` |  |  |
| `RS-0318` | `api` | `d2b-broker` | medium | actionable | leaf |  |  |  | `packages/d2b-broker/src/ops/gpu.rs:13, packages/d2b-broker/src/ops/gpu.rs:24, packages/d2b` |  |  |
| `RS-0322` | `api` | `d2b-broker` | medium | policy-confirmed | leaf |  |  |  | `src/lib.rs:45, src/ops/mod.rs:20-94` |  |  |
| `RS-0317` | `api` | `d2b-broker` | low | actionable | leaf |  |  |  | `packages/d2b-broker/src/ops/usbip_lock.rs:93, packages/d2b-broker/src/live_handlers.rs:467` |  |  |
| `RS-0321` | `api` | `d2b-broker` | low | actionable | leaf |  |  |  | `packages/d2b-broker/src/ops/cgroup.rs:126-129, packages/d2b-broker/src/ops/cgroup.rs:343` |  |  |
| `RS-0323` | `api` | `d2b-broker` | low | actionable | leaf |  |  |  | `packages/d2b-broker/src/kernel_ops.rs:99-100` |  |  |
| `RS-0324` | `api` | `d2b-bus` | medium | actionable | leaf |  |  |  | `packages/d2b-bus/src/router.rs:1208, packages/d2b-bus/src/router.rs:1224, packages/d2b-bus` |  |  |
| `RS-0325` | `api` | `d2b-bus` | medium | actionable | leaf |  |  |  | `packages/d2b-bus/src/authorization.rs:75, packages/d2b-bus/src/router.rs:1415-1416` |  |  |
| `RS-0326` | `api` | `d2b-bus` | medium | actionable | leaf |  |  |  | `packages/d2b-bus/src/lib.rs:34, packages/d2b-bus/src/session/mod.rs:89-97` |  |  |
| `RS-0327` | `api` | `d2b-contracts` | medium | actionable | leaf |  |  |  | `packages/d2b-contracts/src/types.rs:96-113, packages/d2b-contracts/src/types.rs:114` |  |  |
| `RS-0328` | `api` | `d2b-contracts-broker` | medium | needs-contract | wide |  |  |  | `packages/d2b-contracts-broker/src/broker_wire.rs:2862-2869, packages/d2b-broker/src/runtim` |  |  |
| `RS-0329` | `api` | `d2b-contracts-broker` | low | actionable | family |  |  |  | `packages/d2b-contracts-broker/src/broker_wire.rs:13, packages/d2b-contracts-broker/src/lib` |  |  |
| `RS-0330` | `api` | `d2b-contracts-control` | low | actionable | leaf |  |  |  | `cli_output.rs:241, cli_output.rs:276` |  |  |
| `RS-0331` | `api` | `d2b-contracts-control` | low | actionable | leaf |  |  |  | `cli_output.rs:6` |  |  |
| `RS-0332` | `api` | `d2b-contracts-control` | low | needs-contract | wide |  |  |  | `public_wire.rs:2677, docs/reference/schemas/v1/wire-protocol.json:127` |  |  |
| `RS-0333` | `api` | `d2b-contracts-control` | low | actionable | leaf |  |  |  | `unsafe_local_wire.rs:105, unsafe_local_wire.rs:171` |  |  |
| `RS-0334` | `api` | `d2b-contracts-provider` | low | actionable | family |  |  |  | `packages/d2b-contracts-provider/src/v3/provider.rs:568, packages/d2b-contracts-resource/sr` |  |  |
| `RS-0335` | `api` | `d2b-contracts-resource` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/network.rs:956, packages/d2b-contracts-resource/src` |  |  |
| `RS-0336` | `api` | `d2b-contracts-resource` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/identity.rs:269-270` |  |  |
| `RS-0337` | `api` | `d2b-contracts-zone-session` | low | actionable | leaf |  |  |  | `src/v3/resource_export.rs:97, src/v3/resource_export.rs:156, src/v3/resource_import.rs:86,` |  |  |
| `RS-0338` | `api` | `d2b-contracts-zone-session` | low | actionable | leaf |  |  |  | `emergency_policy.rs:142, emergency_policy.rs:170` |  |  |
| `RS-0339` | `api` | `d2b-contracts-zone-session` | low | actionable | leaf |  |  |  | `zone.rs:74` |  |  |
| `RS-0348` | `api` | `d2b-core` | medium | actionable | wide |  |  |  | `packages/d2b-core/src/error.rs:1, packages/d2b-core/src/contract_id.rs:1, packages/d2b-cor` |  |  |
| `RS-0345` | `api` | `d2b-core` | medium | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:620, packages/d2b-core/src/bundle_resolver.rs:641` |  |  |
| `RS-0349` | `api` | `d2b-core` | medium | actionable | wide |  |  |  | `packages/d2b-core/src/static_invariants.rs:3, packages/d2b-core/src/static_invariants.rs:1` |  |  |
| `RS-0346` | `api` | `d2b-core` | medium | actionable | wide |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:107, packages/d2b-core/src/bundle_resolver.rs:109` |  |  |
| `RS-0350` | `api` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/privileges.rs:741` |  |  |
| `RS-0347` | `api` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:2461, packages/d2b-core/src/bundle_resolver.rs:24` |  |  |
| `RS-0340` | `api` | `d2b-core-controller` | medium | actionable | leaf |  |  |  | `packages/d2b-core-controller/src/binding_children.rs:173, packages/d2b-core-controller/src` |  |  |
| `RS-0341` | `api` | `d2b-core-controller` | medium | actionable | leaf |  |  |  | `packages/d2b-core-controller/src/controller_assignment.rs:2707, packages/d2b-core-controll` |  |  |
| `RS-0342` | `api` | `d2b-core-controller` | medium | actionable | leaf |  |  |  | `packages/d2b-core-controller/src/coordinator.rs:252, packages/d2b-core-controller/src/coor` |  |  |
| `RS-0343` | `api` | `d2b-core-controller` | medium | actionable | leaf |  |  |  | `authority.rs:80-128, authority.rs:167-211, authority.rs:258-335, authority.rs:1732` |  |  |
| `RS-0344` | `api` | `d2b-core-controller` | low | actionable | leaf |  |  |  | `owner_reconcile.rs:579, owner_reconcile.rs:600, owner_reconcile.rs:697, owner_reconcile.rs` |  |  |
| `RS-0351` | `api` | `d2b-host` | low | actionable | leaf |  |  |  | `packages/d2b-host/src/host_prep_dag.rs:85` |  |  |
| `RS-0355` | `api` | `d2b-process-conformance` | low | actionable | leaf |  |  |  | `packages/d2b-process-conformance/src/lib.rs:52, packages/d2b-process-conformance/src/lib.r` |  |  |
| `RS-0356` | `api` | `d2b-process-conformance` | low | actionable | leaf |  |  |  | `packages/d2b-process-conformance/src/process_provider.rs:6, packages/d2b-process-conforman` |  |  |
| `RS-0357` | `api` | `d2b-process-conformance` | low | actionable | family |  |  |  | `packages/d2b-process-conformance/src/lib.rs:38, packages/d2b-process-conformance/src/testi` |  |  |
| `RS-0358` | `api` | `d2b-process-conformance` | low | actionable | leaf |  |  |  | `packages/d2b-process-conformance/src/sandbox.rs:18, packages/d2b-process-conformance/src/s` |  |  |
| `RS-0359` | `api` | `d2b-provider-activation-nixos` | low | actionable | leaf |  |  |  | `packages/d2b-provider-activation-nixos/src/driver.rs:426, packages/d2b-provider-activation` |  |  |
| `RS-0360` | `api` | `d2b-provider-activation-nixos` | low | actionable | leaf |  |  |  | `packages/d2b-provider-activation-nixos/src/controller.rs:14, packages/d2b-provider-activat` |  |  |
| `RS-0361` | `api` | `d2b-provider-audio-pipewire` | low | actionable | leaf |  |  |  | `src/authority.rs:157-171, src/controller.rs:395-403` |  |  |
| `RS-0362` | `api` | `d2b-provider-audio-pipewire` | low | actionable | leaf |  |  |  | `src/controller.rs:746-748, src/lib.rs:32` |  |  |
| `RS-0363` | `api` | `d2b-provider-audio-pipewire` | low | needs-contract | family |  |  |  | `src/controller.rs:124-133, packages/d2b-provider-wayland-policy/src/audio_registry.rs:117-` |  |  |
| `RS-0364` | `api` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/service/mod.rs:1289, packages/d2b-provider-cli` |  |  |
| `RS-0365` | `api` | `d2b-provider-config-nixos` | low | actionable | wide |  |  |  | `packages/d2b-provider-config-nixos/src/service.rs:296-298, packages/d2b-provider-config-ni` |  |  |
| `RS-0366` | `api` | `d2b-provider-credential-entra` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-credential-entra/src/lib.rs:102, packages/d2b-provider-credential-en` |  |  |
| `RS-0367` | `api` | `d2b-provider-credential-entra` | low | actionable | leaf |  |  |  | `packages/d2b-provider-credential-entra/src/lib.rs:446, packages/d2b-provider-credential-en` |  |  |
| `RS-0368` | `api` | `d2b-provider-credential-managed-identity` | low | actionable | leaf |  |  |  | `lib.rs:612-618` |  |  |
| `RS-0369` | `api` | `d2b-provider-device` | medium | actionable | family |  |  |  | `packages/d2b-provider-device/src/driver.rs:128-143, packages/d2bd/src/shared_provider_effe` |  |  |
| `RS-0370` | `api` | `d2b-provider-device-gpu` | low | actionable | leaf |  |  |  | `packages/d2b-provider-device-gpu/src/lib.rs:12, packages/d2b-provider-device-gpu/src/lib.r` |  |  |
| `RS-0371` | `api` | `d2b-provider-device-gpu` | low | actionable | leaf |  |  |  | `packages/d2b-provider-device-gpu/src/effects_service.rs:465, packages/d2b-provider-device-` |  |  |
| `RS-0372` | `api` | `d2b-provider-device-security-key` | low | actionable | leaf |  |  |  | `packages/d2b-provider-device-security-key/src/lib.rs:16-17, packages/d2b-provider-device-s` |  |  |
| `RS-0373` | `api` | `d2b-provider-device-tpm` | medium | actionable | leaf |  |  |  | `state.rs:6, state.rs:29, state.rs:51, state.rs:73` |  |  |
| `RS-0374` | `api` | `d2b-provider-device-tpm` | low | actionable | leaf |  |  |  | `effects_service.rs:53, effects_service.rs:103, effects_service.rs:114` |  |  |
| `RS-0375` | `api` | `d2b-provider-device-tpm` | low | actionable | leaf |  |  |  | `effects_service.rs:341` |  |  |
| `RS-0376` | `api` | `d2b-provider-device-tpm` | low | actionable | leaf |  |  |  | `migration.rs:5, lib.rs:24` |  |  |
| `RS-0377` | `api` | `d2b-provider-device-usbip` | medium | actionable | leaf |  |  |  | `lib.rs:24, lib.rs:61-65` |  |  |
| `RS-0378` | `api` | `d2b-provider-device-usbip` | medium | actionable | leaf |  |  |  | `broker.rs:131-133` |  |  |
| `RS-0381` | `api` | `d2b-provider-display-wayland` | medium | actionable | leaf |  |  |  | `src/controller.rs:702, src/controller.rs:1379, src/lib.rs:20` |  |  |
| `RS-0379` | `api` | `d2b-provider-display-wayland` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-display-wayland/src/lib.rs:14` |  |  |
| `RS-0382` | `api` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `src/controller.rs:580` |  |  |
| `RS-0380` | `api` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-display-wayland/src/wayland_proxy/mod.rs:12` |  |  |
| `RS-0383` | `api` | `d2b-provider-guest-azure-container-apps` | low | actionable | leaf |  |  |  | `src/lib.rs:14, src/effects.rs:8-9` |  |  |
| `RS-0384` | `api` | `d2b-provider-guest-azure-virtual-machine` | low | actionable | family |  |  |  | `src/controller/mod.rs:608, src/controller/mod.rs:418, src/controller/mod.rs:748` |  |  |
| `RS-0385` | `api` | `d2b-provider-guest-azure-virtual-machine` | low | actionable | leaf |  |  |  | `src/controller/mod.rs:211, packages/d2b-provider-guest/src/effects_service.rs:1186` |  |  |
| `RS-0386` | `api` | `d2b-provider-guest-cloud-hypervisor` | medium | actionable | leaf |  |  |  | `controller.rs:2140, controller.rs:2148-2151, controller.rs:2876-2895` |  |  |
| `RS-0387` | `api` | `d2b-provider-guest-cloud-hypervisor` | medium | actionable | leaf |  |  |  | `controller.rs:1361-1376, controller.rs:1907-1909` |  |  |
| `RS-0390` | `api` | `d2b-provider-guest-cloud-hypervisor` | medium | actionable | family |  |  |  | `guest_local.rs:49-166, packages/d2b-resource-client/src/zone_client.rs:129-256` |  |  |
| `RS-0388` | `api` | `d2b-provider-guest-cloud-hypervisor` | low | actionable | leaf |  |  |  | `identity.rs:554-556, tests/controller.rs:206` |  |  |
| `RS-0389` | `api` | `d2b-provider-guest-cloud-hypervisor` | low | actionable | leaf |  |  |  | `shutdown.rs:576-578` |  |  |
| `RS-0391` | `api` | `d2b-provider-guest-qemu-media` | low | actionable | leaf |  |  |  | `packages/d2b-provider-guest-qemu-media/src/qmp/mod.rs:129, packages/d2b-provider-guest-qem` |  |  |
| `RS-0392` | `api` | `d2b-provider-host` | low | actionable | leaf |  |  |  | `packages/d2b-provider-host/src/driver.rs:84, packages/d2b-provider-host/src/driver.rs:111,` |  |  |
| `RS-0393` | `api` | `d2b-provider-network-local` | medium | actionable | leaf |  |  |  | `src/routes.rs:244-279, src/routes.rs:264-265` |  |  |
| `RS-0394` | `api` | `d2b-provider-notification-desktop` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-notification-desktop/src/controller.rs:1019, packages/d2b-provider-n` |  |  |
| `RS-0395` | `api` | `d2b-provider-notification-desktop` | low | actionable | leaf |  |  |  | `packages/d2b-provider-notification-desktop/src/controller.rs:110, packages/d2b-provider-no` |  |  |
| `RS-0396` | `api` | `d2b-provider-notification-desktop` | low | actionable | leaf |  |  |  | `packages/d2b-provider-notification-desktop/src/stream_admission.rs:1-3, packages/d2b-provi` |  |  |
| `RS-0397` | `api` | `d2b-provider-process-systemd` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process-systemd/src/lib.rs:28, packages/d2b-provider-process-systemd` |  |  |
| `RS-0398` | `api` | `d2b-provider-process-systemd` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process-systemd/src/lifecycle.rs:55, packages/d2b-provider-process-s` |  |  |
| `RS-0399` | `api` | `d2b-provider-process-systemd` | low | policy-confirmed | leaf |  |  |  | `packages/d2b-provider-process-systemd/src/lib.rs:22, packages/d2b-provider-process-systemd` |  |  |
| `RS-0400` | `api` | `d2b-provider-provider` | low | actionable | leaf |  |  |  | `src/driver.rs:215, src/driver.rs:229` |  |  |
| `RS-0401` | `api` | `d2b-provider-provider` | low | actionable | leaf |  |  |  | `src/providers.rs:121, src/lib.rs:19` |  |  |
| `RS-0402` | `api` | `d2b-provider-quota` | low | actionable | family |  |  |  | `packages/d2b-provider-quota/Cargo.toml:17, packages/d2b-provider-quota/src/lib.rs:21` |  |  |
| `RS-0403` | `api` | `d2b-provider-resource-export` | low | actionable | family |  |  |  | `packages/d2b-provider-resource-export/Cargo.toml:17, packages/d2b-provider-resource-export` |  |  |
| `RS-0404` | `api` | `d2b-provider-resource-import` | low | actionable | family |  |  |  | `packages/d2b-provider-resource-import/Cargo.toml:17, packages/d2b-provider-resource-import` |  |  |
| `RS-0405` | `api` | `d2b-provider-role` | low | actionable | family |  |  |  | `packages/d2b-provider-role/Cargo.toml:17, packages/d2b-provider-role/src/lib.rs:16` |  |  |
| `RS-0406` | `api` | `d2b-provider-seccomp-profile` | low | actionable | family |  |  |  | `packages/d2b-provider-seccomp-profile/src/lib.rs:19, packages/d2b-provider-seccomp-profile` |  |  |
| `RS-0407` | `api` | `d2b-provider-supervisor` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-supervisor/src/broker.rs:997, packages/d2bd/src/process_provider_run` |  |  |
| `RS-0408` | `api` | `d2b-provider-supervisor` | medium | actionable | family |  |  |  | `packages/d2b-provider-supervisor/src/broker.rs:951-955, packages/d2b-provider-supervisor/s` |  |  |
| `RS-0409` | `api` | `d2b-provider-system-core` | medium | actionable | leaf |  |  |  | `src/lib.rs:41, src/testing.rs:23` |  |  |
| `RS-0413` | `api` | `d2b-provider-system-core` | medium | needs-contract | wide |  |  |  | `src/host.rs:389, src/host.rs:13` |  |  |
| `RS-0410` | `api` | `d2b-provider-system-core` | low | actionable | leaf |  |  |  | `src/lib.rs:40, src/ownership.rs:42` |  |  |
| `RS-0411` | `api` | `d2b-provider-system-core` | low | actionable | leaf |  |  |  | `src/host.rs:419, src/host.rs:134` |  |  |
| `RS-0412` | `api` | `d2b-provider-system-core` | low | actionable | leaf |  |  |  | `src/lib.rs:70` |  |  |
| `RS-0414` | `api` | `d2b-provider-toolkit` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-toolkit/src/base/fd10.rs:888, packages/d2b-provider-toolkit/src/base` |  |  |
| `RS-0415` | `api` | `d2b-provider-toolkit` | low | actionable | leaf |  |  |  | `packages/d2b-provider-toolkit/src/server/service.rs:297-299, packages/d2b-provider-toolkit` |  |  |
| `RS-0416` | `api` | `d2b-provider-toolkit` | low | actionable | family |  |  |  | `packages/d2b-provider-toolkit/src/shared_provider.rs:525-531` |  |  |
| `RS-0417` | `api` | `d2b-provider-toolkit` | low | actionable | family |  |  |  | `packages/d2b-provider-toolkit/src/testing/mod.rs:530-532` |  |  |
| `RS-0418` | `api` | `d2b-provider-transport-azure-relay` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/credential_client.rs:491-497, packages/d2b` |  |  |
| `RS-0419` | `api` | `d2b-provider-transport-azure-relay` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/credential_client.rs:343-347, packages/d2b` |  |  |
| `RS-0420` | `api` | `d2b-provider-volume` | medium | actionable | wide |  |  |  | `driver.rs:246-250, driver.rs:282, d2bd/src/resource_plane_v3.rs:2975, tests/registration.r` |  |  |
| `RS-0421` | `api` | `d2b-provider-volume-local` | medium | actionable | family |  |  |  | `src/lib.rs:82, src/testing.rs:1-402, packages/d2b-provider-volume-local/Cargo.toml:1` |  |  |
| `RS-0422` | `api` | `d2b-provider-zone` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-zone/src/lib.rs:9-10, packages/d2b-provider-zone/src/zone_status.rs:` |  |  |
| `RS-0423` | `api` | `d2b-provider-zone-link` | low | actionable | leaf |  |  |  | `packages/d2b-provider-zone-link/src/zone_links.rs:1783, packages/d2b-provider-zone-link/sr` |  |  |
| `RS-0424` | `api` | `d2b-provider-zone-link` | low | actionable | leaf |  |  |  | `packages/d2b-provider-zone-link/src/zonelink.rs:281` |  |  |
| `RS-0425` | `api` | `d2b-provider-zone-link` | low | actionable | leaf |  |  |  | `packages/d2b-provider-zone-link/src/zonelink.rs:178` |  |  |
| `RS-0426` | `api` | `d2b-resource-api` | low | actionable | leaf |  |  |  | `adapter.rs:251-256, adapter.rs:1208-1211` |  |  |
| `RS-0427` | `api` | `d2b-resource-api` | low | actionable | leaf |  |  |  | `client.rs:98, service.rs:837` |  |  |
| `RS-0428` | `api` | `d2b-resource-api` | low | actionable | leaf |  |  |  | `manager_backend.rs:81, manager_backend.rs:208, manager_backend.rs:227` |  |  |
| `RS-0429` | `api` | `d2b-resource-client` | medium | actionable | leaf |  |  |  | `packages/d2b-resource-client/src/zone_client.rs:83, packages/d2b-resource-client/src/zone_` |  |  |
| `RS-0430` | `api` | `d2b-resource-runtime` | medium | actionable | family |  |  |  | `packages/d2b-resource-runtime/src/context.rs:364-366` |  |  |
| `RS-0431` | `api` | `d2b-resource-runtime` | medium | actionable | leaf |  |  |  | `packages/d2b-resource-runtime/src/target.rs:491-493` |  |  |
| `RS-0432` | `api` | `d2b-resource-runtime` | low | actionable | leaf |  |  |  | `packages/d2b-resource-runtime/src/guest_target.rs:460-462` |  |  |
| `RS-0433` | `api` | `d2b-resource-types` | medium | actionable | family |  |  |  | `packages/d2b-resource-types/src/lib.rs:30, packages/d2b-resource-types/src/metadata.rs:72,` |  |  |
| `RS-0434` | `api` | `d2b-session` | low | actionable | leaf |  |  |  | `fragmentation.rs:10-13` |  |  |
| `RS-0435` | `api` | `d2b-session` | low | actionable | leaf |  |  |  | `engine.rs:166, engine.rs:262, engine.rs:416, engine.rs:356` |  |  |
| `RS-0436` | `api` | `d2b-session-unix` | medium | actionable | family |  |  |  | `packages/d2b-session-unix/src/socket.rs:176` |  |  |
| `RS-0437` | `api` | `d2b-sk-frontend` | low | actionable | leaf |  |  |  | `packages/d2b-sk-frontend/src/lib.rs:22, packages/d2b-sk-frontend/src/lib.rs:27, packages/d` |  |  |
| `RS-0438` | `api` | `d2b-unsafe-local-helper` | low | actionable | leaf |  |  |  | `packages/d2b-unsafe-local-helper/src/runtime.rs:573, packages/d2b-unsafe-local-helper/src/` |  |  |
| `RS-0442` | `api` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:398, packages/d2bd/src/composition.rs:400, packages/d2bd/` |  |  |
| `RS-0441` | `api` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/resource_plane_v3.rs:3227, packages/d2bd/src/resource_plane_v3.rs:3295, ` |  |  |
| `RS-0439` | `api` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/audio_host_controller.rs:59, packages/d2bd/src/audio_host_controller.rs:` |  |  |
| `RS-0440` | `api` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/audio_host_controller.rs:68, packages/d2bd/src/audio_host_controller.rs:` |  |  |
| `RS-0443` | `api` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/provider_registry.rs:48, packages/d2bd/src/provider_registry.rs:288` |  |  |
| `RS-0444` | `api` | `d2bd-runtime` | medium | actionable | family |  |  |  | `shell_backend.rs:52-53` |  |  |
| `RS-0446` | `api` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `packages/d2bd-runtime/src/exec_session.rs:900` |  |  |
| `RS-0445` | `api` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `public_read_model.rs:51-53` |  |  |
| `RS-0447` | `api` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `packages/d2bd-runtime/src/console_session.rs:118` |  |  |
| `RS-0448` | `api` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `packages/d2bd-runtime/src/console_session.rs:341, packages/d2bd-runtime/src/console_sessio` |  |  |
| `RS-0449` | `api` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `packages/d2bd-runtime/src/console_session.rs:54` |  |  |
| `RS-0450` | `api` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `packages/d2bd-runtime/src/console_session.rs:66, packages/d2bd-runtime/src/console_session` |  |  |
| `RS-0963` | `err` | `X3-cross-crate-duplication` | medium | needs-contract | wide |  |  |  | `packages/d2b-broker/src/ops/usbip_lock.rs:47, packages/d2b-broker/src/ops/hosts.rs:122, pa` |  |  |
| `RS-0477` | `err` | `d2b` | medium | actionable | leaf |  |  |  | `packages/d2b/src/host.rs:200, packages/d2b/src/resource.rs:916, packages/d2b/src/lib.rs:44` |  |  |
| `RS-0478` | `err` | `d2b` | medium | needs-contract | leaf |  |  |  | `packages/d2b/src/host.rs:274, packages/d2b/src/host.rs:303, packages/d2b/src/host.rs:332, ` |  |  |
| `RS-0451` | `err` | `d2b-audit` | medium | actionable | leaf |  |  |  | `packages/d2b-audit/src/export.rs:230` |  |  |
| `RS-0452` | `err` | `d2b-audit` | medium | actionable | leaf |  |  |  | `packages/d2b-audit/src/segment.rs:694, packages/d2b-audit/src/segment.rs:628, packages/d2b` |  |  |
| `RS-0455` | `err` | `d2b-broker` | high | actionable | wide | applied-variant | U1 | 092b0f3d3 | packages/d2b-broker/src/runtime.rs (from_request) | claim re-verified unreachable at HEAD (join digests are computed before parse); applied anyway so the broker yields the typed protocol refusal like the daemon and the sibling from_request_with_join - mutation-verified with the join returning raw strings, no wire change |  |
| `RS-0454` | `err` | `d2b-broker` | medium | actionable | leaf |  |  |  | `packages/d2b-broker/src/ops/usbip_lock.rs:47, packages/d2b-broker/src/ops/usbip_lock.rs:84` |  |  |
| `RS-0457` | `err` | `d2b-broker` | medium | actionable | leaf |  |  |  | `packages/d2b-broker/src/ops/hosts.rs:122, packages/d2b-broker/src/ops/hosts.rs:149, packag` |  |  |
| `RS-0456` | `err` | `d2b-broker` | low | actionable | leaf |  |  |  | `packages/d2b-broker/src/state_cells.rs:348, packages/d2b-broker/src/state_cells.rs:360` |  |  |
| `RS-0458` | `err` | `d2b-broker` | low | actionable | leaf |  |  |  | `src/ops/exec_reconcile.rs:1238-1265` |  |  |
| `RS-0459` | `err` | `d2b-broker` | low | actionable | leaf |  |  |  | `src/ops/device_worker.rs:262-284, src/ops/device_worker.rs:149, src/ops/live_handlers.rs:2` |  |  |
| `RS-0453` | `err` | `d2b-broker-composition` | low | actionable | leaf |  |  |  | `packages/d2b-broker-composition/src/seam.rs:198, packages/d2b-broker-composition/src/depen` |  |  |
| `RS-0460` | `err` | `d2b-bus` | medium | actionable | leaf |  |  |  | `packages/d2b-bus/src/wire.rs:49-56` |  |  |
| `RS-0461` | `err` | `d2b-contracts` | medium | actionable | leaf |  |  |  | `packages/d2b-contracts/src/configured_argv.rs:15, packages/d2b-contracts/src/launcher.rs:2` |  |  |
| `RS-0462` | `err` | `d2b-contracts-control` | low | actionable | leaf |  |  |  | `public_wire.rs:1297` |  |  |
| `RS-0463` | `err` | `d2b-contracts-provider` | medium | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/provider_registry.rs:186, packages/d2b-contracts-pr` |  |  |
| `RS-0465` | `err` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/credential_controller.rs:90` |  |  |
| `RS-0466` | `err` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/credential_controller.rs:1508, packages/d2b-contrac` |  |  |
| `RS-0464` | `err` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/provider_registry.rs:189, packages/d2b-contracts-pr` |  |  |
| `RS-0467` | `err` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/credential_controller.rs:834, packages/d2b-contract` |  |  |
| `RS-0468` | `err` | `d2b-contracts-resource` | medium | actionable | family |  |  |  | `packages/d2b-contracts-resource/src/v3/operations/error.rs:93, packages/d2b-contracts-reso` |  |  |
| `RS-0469` | `err` | `d2b-contracts-zone-session` | medium | actionable | leaf |  |  |  | `zone_session.rs:297, zone_session.rs:332` |  |  |
| `RS-0471` | `err` | `d2b-core` | medium | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:2625, packages/d2b-broker/src/runtime.rs:6405` |  |  |
| `RS-0475` | `err` | `d2b-core` | medium | actionable | family |  |  |  | `packages/d2b-core/src/storage.rs:357, packages/d2b-core/src/sync.rs:136, packages/d2b-core` |  |  |
| `RS-0472` | `err` | `d2b-core` | medium | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:3368, packages/d2b-core/src/bundle_resolver.rs:19` |  |  |
| `RS-0476` | `err` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/site.rs:42` |  |  |
| `RS-0473` | `err` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:1405, packages/d2b-core/src/bundle_resolver.rs:14` |  |  |
| `RS-0474` | `err` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:5730` |  |  |
| `RS-0470` | `err` | `d2b-core-controller` | low | actionable | leaf |  |  |  | `authority.rs:412` |  |  |
| `RS-0479` | `err` | `d2b-process-conformance` | low | actionable | leaf |  |  |  | `packages/d2b-process-conformance/src/launch_identity.rs:147` |  |  |
| `RS-0480` | `err` | `d2b-provider-activation-nixos` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-activation-nixos/src/controller.rs:119, packages/d2b-provider-activa` |  |  |
| `RS-0481` | `err` | `d2b-provider-audio-pipewire` | medium | actionable | leaf |  |  |  | `src/authority.rs:53-54, src/authority.rs:144-145` |  |  |
| `RS-0482` | `err` | `d2b-provider-audio-pipewire` | low | actionable | leaf |  |  |  | `src/state.rs:114-123, src/controller.rs:155-160` |  |  |
| `RS-0483` | `err` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `src/bin/d2b-clipd.rs:3550, src/bin/d2b-clipd.rs:1754, src/bin/d2b-clipd.rs:2863` |  |  |
| `RS-0484` | `err` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `src/bin/d2b-clipd.rs:127, src/bin/d2b-clipd.rs:411, src/bin/d2b-clipd.rs:247` |  |  |
| `RS-0485` | `err` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/history.rs:137-147, packages/d2b-provider-clip` |  |  |
| `RS-0486` | `err` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/clipd_host/picker.rs:262, packages/d2b-provide` |  |  |
| `RS-0487` | `err` | `d2b-provider-config-nixos` | low | actionable | leaf |  |  |  | `packages/d2b-provider-config-nixos/src/ttrpc.rs:372-376, packages/d2b-provider-config-nixo` |  |  |
| `RS-0488` | `err` | `d2b-provider-credential-managed-identity` | low | actionable | leaf |  |  |  | `lib.rs:1261-1262` |  |  |
| `RS-0489` | `err` | `d2b-provider-device-tpm` | medium | actionable | leaf |  |  |  | `runner.rs:43, swtpm_argv.rs:104, lib.rs:35, tests/conformance.rs:11` |  |  |
| `RS-0490` | `err` | `d2b-provider-device-usbip` | medium | actionable | leaf |  |  |  | `state_machine.rs:378-386` |  |  |
| `RS-0491` | `err` | `d2b-provider-display-wayland` | medium | actionable | leaf |  |  |  | `src/controller.rs:740, src/controller.rs:741` |  |  |
| `RS-0493` | `err` | `d2b-provider-display-wayland` | medium | actionable | leaf |  |  |  | `src/process.rs:335, src/process.rs:346, src/process.rs:825, src/process.rs:886` |  |  |
| `RS-0492` | `err` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `src/spec.rs:30, src/spec.rs:43` |  |  |
| `RS-0494` | `err` | `d2b-provider-host` | low | actionable | leaf |  |  |  | `packages/d2b-provider-host/src/driver.rs:197, packages/d2b-provider-host/src/effects_servi` |  |  |
| `RS-0495` | `err` | `d2b-provider-host` | low | actionable | leaf |  |  |  | `packages/d2b-provider-host/src/driver.rs:129, packages/d2b-provider-host/src/driver.rs:99` |  |  |
| `RS-0496` | `err` | `d2b-provider-network-local` | low | actionable | leaf |  |  |  | `src/nftables.rs:588` |  |  |
| `RS-0497` | `err` | `d2b-provider-notification-desktop` | medium | actionable | family |  |  |  | `packages/d2b-provider-notification-desktop/src/guest_source.rs:18-21, packages/d2b-provide` |  |  |
| `RS-0498` | `err` | `d2b-provider-notification-desktop` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-notification-desktop/src/host_sink.rs:185, packages/d2b-provider-not` |  |  |
| `RS-0499` | `err` | `d2b-provider-observability-otel` | low | actionable | leaf |  |  |  | `ingress_policy.rs:647` |  |  |
| `RS-0500` | `err` | `d2b-provider-process` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process/src/driver.rs:1018-1030, packages/d2b-provider-process/src/d` |  |  |
| `RS-0501` | `err` | `d2b-provider-process-systemd` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process-systemd/src/error.rs:5, packages/d2b-provider-process-system` |  |  |
| `RS-0502` | `err` | `d2b-provider-supervisor` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-supervisor/src/adapter.rs:419-421` |  |  |
| `RS-0503` | `err` | `d2b-provider-supervisor` | low | actionable | leaf |  |  |  | `packages/d2b-provider-supervisor/src/adapter.rs:888-890` |  |  |
| `RS-0504` | `err` | `d2b-provider-telemetry-service` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-telemetry-service/src/driver.rs:304` |  |  |
| `RS-0505` | `err` | `d2b-provider-telemetry-service` | low | actionable | leaf |  |  |  | `packages/d2b-provider-telemetry-service/src/driver.rs:386` |  |  |
| `RS-0506` | `err` | `d2b-provider-test-controller` | low | actionable | leaf |  |  |  | `packages/d2b-provider-test-controller/src/main.rs:186-187, packages/d2b-provider-test-cont` |  |  |
| `RS-0507` | `err` | `d2b-provider-toolkit` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-toolkit/src/operations/envelope.rs:429-433, packages/d2b-provider-to` |  |  |
| `RS-0508` | `err` | `d2b-provider-toolkit` | medium | actionable | family |  |  |  | `packages/d2b-provider-toolkit/src/shared_provider.rs:615, packages/d2b-provider-toolkit/sr` |  |  |
| `RS-0510` | `err` | `d2b-provider-toolkit` | medium | actionable | family |  |  |  | `packages/d2b-provider-toolkit/src/testing/fixture.rs:181-192` |  |  |
| `RS-0511` | `err` | `d2b-provider-toolkit` | medium | actionable | family |  |  |  | `packages/d2b-provider-toolkit/src/testing/fakes.rs:275-277, packages/d2b-provider-toolkit/` |  |  |
| `RS-0509` | `err` | `d2b-provider-toolkit` | low | actionable | family |  |  |  | `packages/d2b-provider-toolkit/src/shared_provider.rs:405-408, packages/d2b-resource-runtim` |  |  |
| `RS-0512` | `err` | `d2b-provider-transport-azure-relay` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/guest_zone_link.rs:69-78, packages/d2b-pro` |  |  |
| `RS-0513` | `err` | `d2b-provider-transport-azure-relay` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/guest_zone_link.rs:26-30, packages/d2b-pro` |  |  |
| `RS-0514` | `err` | `d2b-provider-user` | low | actionable | leaf |  |  |  | `packages/d2b-provider-user/src/driver.rs:118-124, packages/d2b-contracts/src/failure_kinds` |  |  |
| `RS-0515` | `err` | `d2b-provider-volume-binding` | medium | actionable | family |  |  |  | `packages/d2b-provider-volume-binding/src/driver.rs:344, packages/d2b-provider-volume-bindi` |  |  |
| `RS-0516` | `err` | `d2b-provider-wayland-policy` | high | actionable | family | applied | U1 | 5776b3cc5 | packages/d2b-provider-wayland-policy/src/interaction.rs, tests/engine.rs | driver constructor returns a typed SpecInvalid refusal; the class's remaining member sites stay with the family wave | U5 (driver-args class member key_ref; RS-0962) |
| `RS-0517` | `err` | `d2b-provider-wayland-session` | low | actionable | leaf |  |  |  | `packages/d2b-provider-wayland-session/src/wayland_session.rs:73` |  |  |
| `RS-0518` | `err` | `d2b-resource-api` | low | actionable | leaf |  |  |  | `service.rs:867-868` |  |  |
| `RS-0519` | `err` | `d2b-resource-client` | low | actionable | leaf |  |  |  | `packages/d2b-resource-client/src/call.rs:281, packages/d2b-resource-client/src/call.rs:309` |  |  |
| `RS-0520` | `err` | `d2b-resource-runtime` | medium | actionable | leaf |  |  |  | `packages/d2b-resource-runtime/src/error.rs:773, packages/d2b-resource-runtime/src/manager.` |  |  |
| `RS-0521` | `err` | `d2b-resource-runtime` | medium | actionable | leaf |  |  |  | `packages/d2b-resource-runtime/src/spec_store.rs:553, packages/d2b-resource-runtime/src/spe` |  |  |
| `RS-0522` | `err` | `d2b-session` | medium | actionable | leaf |  |  |  | `transport.rs:170, transport.rs:178, transport.rs:185, transport.rs:173` |  |  |
| `RS-0523` | `err` | `d2b-session` | medium | actionable | leaf |  |  |  | `client.rs:216, client.rs:224, client.rs:237` |  |  |
| `RS-0524` | `err` | `d2b-telemetry` | medium | actionable | leaf |  |  |  | `packages/d2b-telemetry/src/emitter.rs:201-206, packages/d2b-telemetry/src/emitter.rs:93` |  |  |
| `RS-0525` | `err` | `d2b-telemetry` | low | actionable | leaf |  |  |  | `packages/d2b-telemetry/src/session_metrics_sink.rs:73, packages/d2b-telemetry/src/session_` |  |  |
| `RS-0526` | `err` | `d2b-unsafe-local-helper` | medium | actionable | leaf |  |  |  | `packages/d2b-unsafe-local-helper/src/systemd.rs:350, packages/d2b-unsafe-local-helper/src/` |  |  |
| `RS-0531` | `err` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:8140, packages/d2bd-runtime/src/workload_dispatch.rs:104,` |  |  |
| `RS-0532` | `err` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/interaction_composition.rs:5365, packages/d2bd/src/interaction_compositi` |  |  |
| `RS-0533` | `err` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/resource_plane_v3.rs:2646, packages/d2bd/src/resource_plane_v3.rs:3016, ` |  |  |
| `RS-0535` | `err` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/effect_service_actors.rs:551, packages/d2bd/src/effect_service_actors.rs` |  |  |
| `RS-0527` | `err` | `d2bd` | medium | actionable | family |  |  |  | `packages/d2bd/src/resource_runtime.rs:381, packages/d2bd/src/resource_runtime.rs:4511` |  |  |
| `RS-0529` | `err` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:22793-22797, packages/d2b-contracts-control/src/public_wi` |  |  |
| `RS-0530` | `err` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:20185-20190` |  |  |
| `RS-0537` | `err` | `d2bd` | medium | needs-contract | wide |  |  |  | `packages/d2bd/src/audio_dispatch.rs:487-495, packages/d2bd/src/audio_dispatch.rs:594-602, ` |  |  |
| `RS-0534` | `err` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/resource_plane_v3.rs:1956` |  |  |
| `RS-0536` | `err` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/process_provider_runtime.rs:3436` |  |  |
| `RS-0528` | `err` | `d2bd` | low | actionable | family |  |  |  | `packages/d2bd/src/composition.rs:13894, packages/d2bd/src/composition.rs:14127, packages/d` |  |  |
| `RS-0538` | `err` | `d2bd-runtime` | high | actionable | leaf | applied-variant | U1 | ebb3831b1 | packages/d2bd-runtime/src/broker_transport.rs, packages/d2bd/src/composition.rs | claim re-verified unreachable at HEAD (digests canonicalized before parse); applied anyway to remove the latent expect and mirror the sibling typed refusal - callers updated |  |
| `RS-0539` | `err` | `d2bd-runtime` | medium | actionable | leaf |  |  |  | `wire.rs:529-536` |  |  |
| `RS-0540` | `err` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `packages/d2bd-runtime/src/exec_session.rs:939` |  |  |
| `RS-0541` | `err` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `packages/d2bd-runtime/src/console_session.rs:132` |  |  |
| `RS-0542` | `err` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `packages/d2bd-runtime/src/daemon_version.rs:77, packages/d2bd-runtime/src/daemon_version.r` |  |  |
| `RS-0543` | `err` | `xtask` | medium | actionable | leaf |  |  |  | `packages/xtask/src/delivery/recovery.rs:384, packages/xtask/src/delivery/recovery.rs:1610,` |  |  |
| `RS-0961` | `serde` | `X3-cross-crate-duplication` | medium | actionable | family |  |  |  | `packages/d2b-contracts-zone-session/src/v3/zone_routing.rs:558, packages/d2b-contracts-zon` |  |  |
| `RS-0545` | `serde` | `d2b-broker` | low | actionable | leaf |  |  |  | `packages/d2b-broker/src/state_cells.rs:337, packages/d2b-broker/src/state_cells.rs:924` |  |  |
| `RS-0544` | `serde` | `d2b-broker-composition` | low | actionable | leaf |  |  |  | `packages/d2b-broker-composition/src/dependency_surface.rs:308, packages/d2b-broker-composi` |  |  |
| `RS-0546` | `serde` | `d2b-contracts` | medium | needs-contract | wide |  |  |  | `packages/d2b-contracts/src/audit_wire.rs:27-36` |  |  |
| `RS-0547` | `serde` | `d2b-contracts-broker` | medium | needs-contract | wide |  |  |  | `packages/d2b-contracts-broker/src/broker_wire.rs:1836-1861, packages/d2b-contracts-broker/` |  |  |
| `RS-0548` | `serde` | `d2b-contracts-control` | medium | actionable | leaf |  |  |  | `unsafe_local_wire.rs:118, unsafe_local_wire.rs:176, public_wire.rs:2228` |  |  |
| `RS-0549` | `serde` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/telemetry_frame.rs:76, packages/d2b-contracts-provi` |  |  |
| `RS-0550` | `serde` | `d2b-contracts-resource` | medium | actionable | wide |  |  |  | `packages/d2b-contracts-resource/src/v3/error.rs:175, packages/d2b-contracts-resource/src/v` |  |  |
| `RS-0551` | `serde` | `d2b-contracts-resource` | medium | actionable | family |  |  |  | `packages/d2b-contracts-resource/src/v3/payload_schema.rs:30, packages/d2b-contracts-resour` |  |  |
| `RS-0552` | `serde` | `d2b-contracts-zone-session` | medium | actionable | family |  |  |  | `zone_routing.rs:558, zone_routing.rs:624, zone_routing.rs:818, zone_routing.rs:932` |  |  |
| `RS-0556` | `serde` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/storage_lifecycle.rs:49, packages/d2b-core/src/storage_lifecycle.rs:` |  |  |
| `RS-0554` | `serde` | `d2b-core` | low | needs-contract | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:209, packages/d2b-core/src/bundle_resolver.rs:175` |  |  |
| `RS-0555` | `serde` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:183, packages/d2b-core/src/bundle_resolver.rs:187` |  |  |
| `RS-0553` | `serde` | `d2b-core-controller` | medium | actionable | wide |  |  |  | `packages/d2b-core-controller/src/controller_assignment.rs:596, packages/d2b-core-controlle` |  |  |
| `RS-0557` | `serde` | `d2b-host` | low | actionable | family |  |  |  | `packages/d2b-host/src/nftables.rs:229` |  |  |
| `RS-0558` | `serde` | `d2b-process-conformance` | low | actionable | leaf |  |  |  | `packages/d2b-process-conformance/src/status.rs:125, packages/d2b-process-conformance/src/t` |  |  |
| `RS-0559` | `serde` | `d2b-process-conformance` | low | actionable | leaf |  |  |  | `packages/d2b-process-conformance/src/terminal.rs:39, packages/d2b-process-conformance/src/` |  |  |
| `RS-0560` | `serde` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/clipd_host/audit.rs:13, packages/d2b-provider-` |  |  |
| `RS-0561` | `serde` | `d2b-provider-command` | medium | needs-contract | leaf |  |  |  | `packages/d2b-provider-command/src/command.rs:64-80, packages/d2b-provider-command/src/comm` |  |  |
| `RS-0562` | `serde` | `d2b-provider-device-gpu` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-device-gpu/src/gpu_argv.rs:77, packages/d2b-provider-device-gpu/src/` |  |  |
| `RS-0563` | `serde` | `d2b-provider-display-wayland` | medium | actionable | family |  |  |  | `packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:817, packages/d2b-provid` |  |  |
| `RS-0564` | `serde` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `src/spec.rs:230, src/spec.rs:233, src/spec.rs:263` |  |  |
| `RS-0565` | `serde` | `d2b-provider-endpoint` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-endpoint/src/endpoint.rs:112-120, packages/d2b-provider-endpoint/src` |  |  |
| `RS-0566` | `serde` | `d2b-provider-network-local` | medium | actionable | leaf |  |  |  | `src/driver.rs:282-289, src/driver.rs:190` |  |  |
| `RS-0567` | `serde` | `d2b-provider-network-local` | low | actionable | leaf |  |  |  | `src/broker.rs:1432, src/broker.rs:1453, src/operations.rs:408, src/operations.rs:429` |  |  |
| `RS-0568` | `serde` | `d2b-provider-supervisor` | medium | needs-contract | leaf |  |  |  | `packages/d2b-provider-supervisor/src/broker.rs:1563-1603, packages/d2b-broker/src/generate` |  |  |
| `RS-0569` | `serde` | `d2b-provider-transport-azure-relay` | medium | needs-contract | family |  |  |  | `packages/d2b-provider-transport-azure-relay/src/transport_settings.rs:9-15, packages/d2b-p` |  |  |
| `RS-0570` | `serde` | `d2b-provider-transport-azure-relay` | low | policy-confirmed | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/guest_credential.rs:245-264` |  |  |
| `RS-0571` | `serde` | `d2b-provider-transport-vsock` | medium | needs-contract | wide |  |  |  | `packages/d2b-provider-transport-vsock/src/settings.rs:16-27, packages/d2b-provider-transpo` |  |  |
| `RS-0572` | `serde` | `d2b-provider-volume-local` | medium | actionable | leaf |  |  |  | `src/content.rs:38-39, src/content.rs:106-107, src/content.rs:203-204, src/content.rs:239-2` |  |  |
| `RS-0573` | `serde` | `d2b-provider-wayland-policy` | medium | actionable | family |  |  |  | `packages/d2b-provider-wayland-policy/src/interaction.rs:241-243, packages/d2b-provider-way` |  |  |
| `RS-0574` | `serde` | `d2b-resource-api` | medium | actionable | leaf |  |  |  | `manager_backend.rs:744-745` |  |  |
| `RS-0576` | `serde` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:4196, packages/d2bd/src/composition.rs:4205` |  |  |
| `RS-0575` | `serde` | `d2bd` | low | needs-contract | leaf |  |  |  | `packages/d2bd/src/composition.rs:20144, packages/d2bd/src/composition.rs:20275, packages/d` |  |  |
| `RS-0577` | `serde` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/forward_rendezvous.rs:979-981` |  |  |
| `RS-0578` | `serde` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `wire.rs:265-353` |  |  |
| `RS-0579` | `serde` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `packages/d2bd-runtime/src/ch_api.rs:79` |  |  |
| `RS-0580` | `serde` | `xtask` | medium | actionable | leaf |  |  |  | `packages/xtask/src/service_catalog.rs:22, packages/xtask/src/provider_registration_authori` |  |  |
| `RS-0582` | `serde` | `xtask` | low | actionable | leaf |  |  |  | `packages/xtask/src/delivery/mod.rs:46, packages/xtask/src/delivery/model.rs:111` |  |  |
| `RS-0581` | `serde` | `xtask` | low | actionable | leaf |  |  |  | `packages/xtask/src/resource_type_authority.rs:179, packages/xtask/src/resource_type_author` |  |  |
| `RS-0583` | `obs` | `d2b-broker` | low | actionable | leaf |  |  |  | `packages/d2b-broker/src/live_handlers.rs:2532, packages/d2b-broker/src/live_handlers.rs:25` |  |  |
| `RS-0584` | `obs` | `d2b-core` | medium | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:1097` |  |  |
| `RS-0585` | `obs` | `d2b-provider-activation-nixos` | low | actionable | leaf |  |  |  | `packages/d2b-provider-activation-nixos/src/controller.rs:569, packages/d2b-provider-activa` |  |  |
| `RS-0586` | `obs` | `d2b-provider-clipboard-wayland` | medium | actionable | leaf |  |  |  | `src/bin/d2b-clipd.rs:206, src/bin/d2b-clipd.rs:1627, src/runtime.rs:100` |  |  |
| `RS-0587` | `obs` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/clipd_host/host.rs:64-68, packages/d2b-provide` |  |  |
| `RS-0588` | `obs` | `d2b-provider-credential-secret-service` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-credential-secret-service/src/service.rs:860` |  |  |
| `RS-0589` | `obs` | `d2b-provider-guest-azure-virtual-machine` | low | actionable | leaf |  |  |  | `src/controller/mod.rs:346, src/controller/mod.rs:425` |  |  |
| `RS-0590` | `obs` | `d2b-provider-guest-qemu-media` | low | actionable | leaf |  |  |  | `packages/d2b-provider-guest-qemu-media/src/controller/reconcile.rs:352, packages/d2b-provi` |  |  |
| `RS-0591` | `obs` | `d2b-provider-process-systemd` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process-systemd/src/lib.rs:139, packages/d2b-provider-process-system` |  |  |
| `RS-0592` | `obs` | `d2b-provider-test-controller` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-test-controller/src/main.rs:33-47, packages/d2b-provider-test-contro` |  |  |
| `RS-0593` | `obs` | `d2b-provider-test-controller` | low | actionable | leaf |  |  |  | `packages/d2b-provider-test-controller/src/main.rs:164, packages/d2b-provider-test-controll` |  |  |
| `RS-0594` | `obs` | `d2b-provider-toolkit` | low | actionable | leaf |  |  |  | `packages/d2b-provider-toolkit/src/base/guest.rs:494-498, packages/d2b-provider-toolkit/src` |  |  |
| `RS-0595` | `obs` | `d2b-provider-toolkit` | low | actionable | family |  |  |  | `packages/d2b-provider-toolkit/src/server/session.rs:135, packages/d2b-provider-toolkit/src` |  |  |
| `RS-0596` | `obs` | `d2b-provider-transport-azure-relay` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/guest_credential.rs:406-408, packages/d2b-` |  |  |
| `RS-0597` | `obs` | `d2b-provider-transport-unix` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-unix/src/portal.rs:217-220, packages/d2b-provider-transpor` |  |  |
| `RS-0598` | `obs` | `d2b-provider-transport-vsock` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-vsock/src/service.rs:392, packages/d2b-provider-transport-` |  |  |
| `RS-0599` | `obs` | `d2b-provider-transport-vsock` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-vsock/src/service.rs:735, packages/d2b-provider-transport-` |  |  |
| `RS-0600` | `obs` | `d2b-resource-api` | low | actionable | leaf |  |  |  | `service.rs:1992` |  |  |
| `RS-0601` | `obs` | `d2b-unsafe-local-helper` | low | actionable | leaf |  |  |  | `packages/d2b-unsafe-local-helper/src/protocol.rs:188` |  |  |
| `RS-0605` | `obs` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:4081, packages/d2bd/src/composition.rs:4099, packages/d2b` |  |  |
| `RS-0606` | `obs` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:4487, packages/d2bd/src/composition.rs:4538, packages/d2b` |  |  |
| `RS-0607` | `obs` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/provider_lifecycle.rs:1085` |  |  |
| `RS-0602` | `obs` | `d2bd` | low | actionable | leaf |  |  | `instrument` = 0), so the events lose the underlying error: most are inside `map_err` closu` |  |  |
| `RS-0603` | `obs` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:14781` |  |  |
| `RS-0604` | `obs` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:15195, packages/d2bd/src/composition.rs:15221` |  |  |
| `RS-0608` | `obs` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/forward_rendezvous.rs:1250` |  |  |
| `RS-0609` | `obs` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `runtime_process.rs:450, runtime_process.rs:464, runtime_process.rs:472, runtime_process.rs` |  |  |
| `RS-0610` | `obs` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `ssh_host_key_preflight.rs:305, resource_runtime_support.rs:679-680` |  |  |
| `RS-0611` | `obs` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `packages/d2bd-runtime/src/readiness.rs:327` |  |  |
| `RS-0612` | `obs` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `packages/d2bd-runtime/src/pidfs_probe.rs:129, packages/d2bd-runtime/src/pidfs_probe.rs:132` |  |  |
| `RS-0613` | `obs` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `packages/d2bd-runtime/src/console_session.rs:450` |  |  |
| `RS-0659` | `docs` | `d2b` | low | actionable | leaf | applied | U2 | 4784e6cda | `packages/d2b/src/exec_client.rs` | one-line docs for expect_start/expect_detached_create/list/logs/status/kill |  |
| `RS-0658` | `docs` | `d2b` | low | actionable | leaf | applied | U2 | 4784e6cda | `packages/d2b/src/doctor.rs` | docs for doctor/validate/CLI surface incl. crate-level doc |  |
| `RS-0614` | `docs` | `d2b-audit` | low | actionable | leaf |  |  |  | `packages/d2b-audit/src/export.rs:74, packages/d2b-audit/src/record_types.rs:428, packages/` |  |  |
| `RS-0624` | `docs` | `d2b-broker` | medium | actionable | leaf | applied | U2 | 93ced15b2 | `packages/d2b-broker/src/fd_passing.rs` | doc comments added per row |  |
| `RS-0630` | `docs` | `d2b-broker` | medium | actionable | leaf | applied | U2 | b09261be0 | `packages/d2b-broker/src/ops/media.rs` | MediaOpError variants, outcome structs/fields, eight pub ops fns documented with # Errors |  |
| `RS-0625` | `docs` | `d2b-broker` | medium | actionable | leaf | applied | U2 | d2096735c | `packages/d2b-broker/src/sys.rs` | doc comments added; merged with RS-0008/RS-0626 in same commit |  |
| `RS-0631` | `docs` | `d2b-broker` | medium | actionable | leaf | applied | U2 | ba9873712 | `packages/d2b-broker/src/protocol.rs` | MAX_FRAME_SIZE and connect/bind/send_json_frame/recv_json_frame documented |  |
| `RS-0632` | `docs` | `d2b-broker` | medium | actionable | leaf | applied | U2 | 6b709ea9a | `packages/d2b-broker/src/ops/state_dir.rs` | DirKind, PrepareDirRequest/fields, PrepareDirAudit, ReplaceOrCreateResult, prepare_dir and live helpers documented with # Errors |  |
| `RS-0621` | `docs` | `d2b-broker` | medium | actionable | leaf | applied | U2 | 1643cd532 | `packages/d2b-broker/src/audit.rs` | field docs on AuditDropSummary/AuditEntry; contract docs on AuditLog::open/audit_drop_summary |  |
| `RS-0616` | `docs` | `d2b-broker` | medium | actionable | family |  |  |  | `packages/d2b-broker/src/runtime.rs:1, packages/d2b-broker/src/runtime.rs:324, packages/d2b` |  |  |
| `RS-0622` | `docs` | `d2b-broker` | medium | actionable | leaf | applied | U2 | 7ac3d8cdf | `packages/d2b-broker/src/ops/host_generation_handoff.rs` | doc comments added per row |  |
| `RS-0623` | `docs` | `d2b-broker` | medium | actionable | leaf | applied | U2 | 025b075a8 | `packages/d2b-broker/src/ops/route.rs` | doc comments added per row |  |
| `RS-0615` | `docs` | `d2b-broker` | low | actionable | leaf | applied | U2 | 34f355adf | `packages/d2b-broker/src/ops/usbip_lock.rs` | doc comments added per row |  |
| `RS-0617` | `docs` | `d2b-broker` | low | actionable | leaf | applied | U2 | 4bfa5fd51 | `packages/d2b-broker/src/ops/usbip_host.rs` | doc comments added per row |  |
| `RS-0626` | `docs` | `d2b-broker` | low | actionable | leaf | applied | U2 | d2096735c | `packages/d2b-broker/src/sys.rs` | doc comments added; merged with RS-0008/RS-0625 in same commit |  |
| `RS-0618` | `docs` | `d2b-broker` | low | actionable | leaf | applied | U2 | 7cf9b6478 | `packages/d2b-broker/src/ops/sysctl.rs` | doc comments added per row |  |
| `RS-0619` | `docs` | `d2b-broker` | low | actionable | leaf | applied | U2 | 380e073d5 | `packages/d2b-broker/src/ops/storage_contract.rs` | doc comments added per row |  |
| `RS-0620` | `docs` | `d2b-broker` | low | actionable | leaf | applied | U2 | e507a1e71 | `packages/d2b-broker/src/ops/mod.rs` | doc comments added per row |  |
| `RS-0627` | `docs` | `d2b-broker` | low | actionable | leaf | applied | U2 | 5a2fa07c7 | `packages/d2b-broker/src/ops/store_view_farm.rs` | journal sentence trimmed and # Errors block added; merged with RS-0010 in same commit |  |
| `RS-0628` | `docs` | `d2b-broker` | low | actionable | leaf | applied | U2 | ac44605b7 | `packages/d2b-broker/src/envelope/mod.rs` | # Errors blocks on call/call_with_fds/call_nested_with_fds naming ENVELOPE_REFUSALS vocabulary; merged |  |
| `RS-0629` | `docs` | `d2b-broker` | low | actionable | leaf | applied | U2 | 098cdc0e5 | `packages/d2b-broker/src/ops/nm.rs` | apply_with_reload/remove_with_reload docs rewritten as plain contracts |  |
| `RS-0633` | `docs` | `d2b-bus` | medium | actionable | leaf | applied | U2 | fbf92683a | `packages/d2b-bus/src/router.rs` | docs for BusEvent/BusFailureReason variants,and BusObserver methods |  |
| `RS-0637` | `docs` | `d2b-bus` | medium | actionable | leaf | applied | U2 | fbf92683a | `packages/d2b-bus/src/operations.rs` | Cancellation docs: minted by bus, one attempt, is_cancelled |  |
| `RS-0634` | `docs` | `d2b-bus` | low | actionable | leaf | applied | U2 | fbf92683a | `packages/d2b-bus/src/router.rs` | one-line docs for DEFAULT_MAX_ROUTES_PER_SESSION,and DEFAULT_MAX_TOTAL_ROUTES |  |
| `RS-0635` | `docs` | `d2b-bus` | low | actionable | leaf | applied | U2 | fbf92683a,6355caebb | `packages/d2b-bus/src/router.rs` | docs for install body, authz error class, session-failure accessors, as_str wire labels |  |
| `RS-0636` | `docs` | `d2b-bus` | low | actionable | leaf | applied | U2 | fbf92683a,6355caebb | `packages/d2b-bus/src/router.rs` | # Errors on BusIngress::invoke, ZoneRegistrar::register_component_session,and BusEndpoint::invoke |  |
| `RS-0638` | `docs` | `d2b-bus` | low | actionable | leaf | applied | U2 | fbf92683a,6355caebb | `packages/d2b-bus/src/streams.rs` | # Errors on StreamName::parse, OperationId::parse, ZoneBoundPolicyIdentity::digest,and ZoneEndpointPolicy::lower |  |
| `RS-0639` | `docs` | `d2b-contracts-broker` | medium | actionable | leaf |  |  |  | `packages/d2b-contracts-broker/src/host_generation.rs:47, packages/d2b-contracts-broker/src` |  |  |
| `RS-0640` | `docs` | `d2b-contracts-broker` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-broker/src/broker_wire.rs:246, packages/d2b-contracts-broker/src/br` |  |  |
| `RS-0641` | `docs` | `d2b-contracts-control` | medium | actionable | leaf |  |  |  | `cli_output.rs:10, cli_output.rs:14, cli_output.rs:49, cli_output.rs:130` |  |  |
| `RS-0642` | `docs` | `d2b-contracts-control` | medium | actionable | leaf |  |  |  | `public_wire.rs:278, public_wire.rs:293, public_wire.rs:2451, public_wire.rs:2495` |  |  |
| `RS-0643` | `docs` | `d2b-contracts-control` | medium | actionable | leaf |  |  |  | `unsafe_local_wire.rs:15, unsafe_local_wire.rs:21, unsafe_local_wire.rs:24, unsafe_local_wi` |  |  |
| `RS-0644` | `docs` | `d2b-contracts-control` | low | actionable | leaf |  |  |  | `terminal_wire.rs:12, terminal_wire.rs:19, terminal_wire.rs:26, terminal_wire.rs:105` |  |  |
| `RS-0646` | `docs` | `d2b-contracts-provider` | medium | actionable | family |  |  |  | `packages/d2b-contracts-provider/src/v3/credential_controller.rs:155, packages/d2b-contract` |  |  |
| `RS-0645` | `docs` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/provider.rs:250, packages/d2b-contracts-provider/sr` |  |  |
| `RS-0649` | `docs` | `d2b-contracts-resource` | medium | actionable | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/artifact.rs:5, packages/d2b-contracts-resource/src/` |  |  |
| `RS-0650` | `docs` | `d2b-contracts-resource` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/execution_policy.rs:923, packages/d2b-contracts-res` |  |  |
| `RS-0647` | `docs` | `d2b-contracts-resource` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/limits.rs:3, packages/d2b-contracts-resource/src/v3` |  |  |
| `RS-0648` | `docs` | `d2b-contracts-resource` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/operations/error.rs:21, packages/d2b-contracts-reso` |  |  |
| `RS-0651` | `docs` | `d2b-contracts-zone-session` | medium | actionable | wide |  |  |  | `src/v3/component_session.rs:27, src/v3/component_session.rs:51, src/v3/component_session.r` |  |  |
| `RS-0655` | `docs` | `d2b-core` | medium | actionable | leaf |  |  |  | `packages/d2b-core/src/manifest_v04.rs:1, packages/d2b-core/src/manifest_v04.rs:31, package` |  |  |
| `RS-0654` | `docs` | `d2b-core` | medium | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:2917, packages/d2b-core/src/bundle_resolver.rs:29` |  |  |
| `RS-0652` | `docs` | `d2b-core-controller` | low | actionable | leaf |  |  |  | `packages/d2b-core-controller/src/main.rs:33-56, packages/d2b-core-controller/src/controlle` |  |  |
| `RS-0653` | `docs` | `d2b-core-controller` | low | actionable | leaf |  |  |  | `migration.rs:54, migration.rs:58` |  |  |
| `RS-0656` | `docs` | `d2b-host` | low | actionable | leaf |  |  |  | `packages/d2b-host/src/bridge_port.rs:127, packages/d2b-host/src/host_generation.rs:103, pa` |  |  |
| `RS-0657` | `docs` | `d2b-host` | low | actionable | leaf |  |  |  | `packages/d2b-host/src/cgroup.rs:53, packages/d2b-host/src/cgroup.rs:71, packages/d2b-host/` |  |  |
| `RS-0660` | `docs` | `d2b-process-conformance` | low | actionable | leaf |  |  |  | `packages/d2b-process-conformance/src/terminal.rs:51, packages/d2b-process-conformance/src/` |  |  |
| `RS-0661` | `docs` | `d2b-provider` | medium | actionable | leaf |  |  |  | `packages/d2b-provider/src/agent.rs:270, packages/d2b-provider/src/descriptor.rs:232, packa` |  |  |
| `RS-0662` | `docs` | `d2b-provider-activation-nixos` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-activation-nixos/src/controller.rs:711, packages/d2b-provider-activa` |  |  |
| `RS-0663` | `docs` | `d2b-provider-audio-binding` | low | actionable | leaf |  |  |  | `packages/d2b-provider-audio-binding/src/audio_binding.rs:56, packages/d2b-provider-audio-b` |  |  |
| `RS-0664` | `docs` | `d2b-provider-audio-pipewire` | medium | actionable | leaf |  |  |  | `src/state.rs:81-84, src/lib.rs:9-10` |  |  |
| `RS-0665` | `docs` | `d2b-provider-audio-pipewire` | low | actionable | leaf |  |  |  | `src/controller.rs:21, src/controller.rs:209, src/controller.rs:212` |  |  |
| `RS-0666` | `docs` | `d2b-provider-clipboard-wayland` | medium | actionable | leaf |  |  |  | `src/fd.rs:549, src/policy.rs:81, src/audit.rs:200, src/runtime.rs:97` |  |  |
| `RS-0667` | `docs` | `d2b-provider-clipboard-wayland` | medium | actionable | family |  |  |  | `src/audit.rs:172, src/audit.rs:174` |  |  |
| `RS-0668` | `docs` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/history.rs:112-115` |  |  |
| `RS-0669` | `docs` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/clipd_host/niri.rs:136, packages/d2b-provider-` |  |  |
| `RS-0670` | `docs` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/clipd_host/framing.rs:28-29, packages/d2b-prov` |  |  |
| `RS-0671` | `docs` | `d2b-provider-command` | low | actionable | leaf |  |  |  | `packages/d2b-provider-command/src/command.rs:38, packages/d2b-provider-command/src/command` |  |  |
| `RS-0672` | `docs` | `d2b-provider-config-nixos` | low | actionable | leaf |  |  |  | `packages/d2b-provider-config-nixos/src/controller.rs:45-64, packages/d2b-provider-config-n` |  |  |
| `RS-0673` | `docs` | `d2b-provider-credential` | low | actionable | leaf |  |  |  | `packages/d2b-provider-credential/src/session.rs:132, packages/d2b-provider-credential/src/` |  |  |
| `RS-0674` | `docs` | `d2b-provider-credential-entra` | low | actionable | leaf |  |  |  | `packages/d2b-provider-credential-entra/src/controller.rs:47, packages/d2b-provider-credent` |  |  |
| `RS-0675` | `docs` | `d2b-provider-credential-managed-identity` | low | actionable | leaf |  |  |  | `lib.rs:449, lib.rs:514, lib.rs:592, lib.rs:796` |  |  |
| `RS-0676` | `docs` | `d2b-provider-credential-secret-service` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-credential-secret-service/src/lib.rs:523, packages/d2b-provider-cred` |  |  |
| `RS-0677` | `docs` | `d2b-provider-credential-secret-service` | low | actionable | leaf |  |  |  | `packages/d2b-provider-credential-secret-service/src/service.rs:660, packages/d2b-provider-` |  |  |
| `RS-0678` | `docs` | `d2b-provider-device-gpu` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-device-gpu/src/controller.rs:172, packages/d2b-provider-device-gpu/s` |  |  |
| `RS-0679` | `docs` | `d2b-provider-device-gpu` | low | actionable | leaf |  |  |  | `packages/d2b-provider-device-gpu/src/gpu_argv.rs:24, packages/d2b-provider-device-gpu/src/` |  |  |
| `RS-0681` | `docs` | `d2b-provider-device-security-key` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-device-security-key/src/lease.rs:150-303, packages/d2b-provider-devi` |  |  |
| `RS-0680` | `docs` | `d2b-provider-device-security-key` | low | actionable | leaf |  |  |  | `packages/d2b-provider-device-security-key/src/relay.rs:7, packages/d2b-provider-device-sec` |  |  |
| `RS-0682` | `docs` | `d2b-provider-device-tpm` | low | actionable | leaf |  |  |  | `resources.rs:53, swtpm_argv.rs:130, resource_controller.rs:132, resource_controller.rs:190` |  |  |
| `RS-0683` | `docs` | `d2b-provider-device-tpm` | low | actionable | leaf |  |  |  | `swtpm_argv.rs:39` |  |  |
| `RS-0684` | `docs` | `d2b-provider-device-usbip` | medium | actionable | leaf |  |  |  | `reconcile_state.rs:6, state_machine.rs:61, lib.rs:9` |  |  |
| `RS-0685` | `docs` | `d2b-provider-device-usbip` | medium | actionable | leaf |  |  |  | `arbitration.rs:76, busid.rs:12, broker.rs:61, controller.rs:210` |  |  |
| `RS-0687` | `docs` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `src/policy.rs:114, src/policy.rs:119` |  |  |
| `RS-0686` | `docs` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-display-wayland/src/wayland_proxy/bridge.rs:37, packages/d2b-provide` |  |  |
| `RS-0688` | `docs` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `src/spec.rs:117, src/spec.rs:286, src/policy.rs:247, src/controller.rs:759` |  |  |
| `RS-0689` | `docs` | `d2b-provider-guest` | low | actionable | leaf |  |  |  | `packages/d2b-provider-guest/src/facets.rs:63, packages/d2b-provider-guest/src/target_contr` |  |  |
| `RS-0690` | `docs` | `d2b-provider-guest-azure-container-apps` | low | actionable | leaf |  |  |  | `src/lib.rs:7, src/effects.rs:813-882` |  |  |
| `RS-0691` | `docs` | `d2b-provider-guest-azure-virtual-machine` | medium | actionable | leaf |  |  |  | `src/controller/mod.rs:333, src/controller/mod.rs:446` |  |  |
| `RS-0692` | `docs` | `d2b-provider-guest-azure-virtual-machine` | low | actionable | leaf |  |  |  | `src/bootstrap.rs:25` |  |  |
| `RS-0693` | `docs` | `d2b-provider-guest-cloud-hypervisor` | low | actionable | leaf |  |  |  | `descriptor.rs:423-429, identity.rs:590-634` |  |  |
| `RS-0694` | `docs` | `d2b-provider-guest-qemu-media` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-guest-qemu-media/src/controller/device_watch.rs:82, packages/d2b-pro` |  |  |
| `RS-0695` | `docs` | `d2b-provider-notification-desktop` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-notification-desktop/src/action_nonce.rs:84-89, packages/d2b-provide` |  |  |
| `RS-0696` | `docs` | `d2b-provider-notification-desktop` | low | actionable | leaf |  |  |  | `packages/d2b-provider-notification-desktop/src/runtime.rs:88-89, packages/d2b-provider-not` |  |  |
| `RS-0697` | `docs` | `d2b-provider-observability-otel` | medium | actionable | leaf |  |  |  | `lib.rs:13, lib.rs:14, lib.rs:15` |  |  |
| `RS-0698` | `docs` | `d2b-provider-observability-otel` | medium | actionable | leaf |  |  |  | `agent.rs:216, config.rs:147, controller.rs:100, emitter_socket.rs:131` |  |  |
| `RS-0699` | `docs` | `d2b-provider-operation` | low | actionable | leaf |  |  |  | `packages/d2b-provider-operation/src/operation.rs:138, packages/d2b-provider-operation/src/` |  |  |
| `RS-0700` | `docs` | `d2b-provider-process` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process/src/backend.rs:256, packages/d2b-provider-process/src/launch` |  |  |
| `RS-0701` | `docs` | `d2b-provider-process-minijail` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process-minijail/src/launch.rs:33, packages/d2b-provider-process-min` |  |  |
| `RS-0702` | `docs` | `d2b-provider-process-systemd` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-process-systemd/src/lifecycle.rs:33, packages/d2b-provider-process-s` |  |  |
| `RS-0703` | `docs` | `d2b-provider-provider` | low | actionable | leaf |  |  |  | `src/providers.rs:72, src/providers.rs:79` |  |  |
| `RS-0704` | `docs` | `d2b-provider-role` | low | actionable | leaf |  |  |  | `packages/d2b-provider-role/src/lib.rs:1, packages/d2b-provider-role/src/rbac.rs:11` |  |  |
| `RS-0705` | `docs` | `d2b-provider-seccomp-profile` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-seccomp-profile/src/seccomp_profile.rs:31, packages/d2b-provider-sec` |  |  |
| `RS-0706` | `docs` | `d2b-provider-shell-terminal` | medium | actionable | leaf |  |  |  | `src/authz.rs:76, src/service/controller.rs:134, src/service/supervisor.rs:88, src/session/` |  |  |
| `RS-0707` | `docs` | `d2b-provider-system-core` | medium | actionable | leaf |  |  |  | `src/host.rs:121, src/host.rs:161, src/user.rs:241` |  |  |
| `RS-0708` | `docs` | `d2b-provider-toolkit` | low | actionable | leaf |  |  |  | `packages/d2b-provider-toolkit/src/base/runtime.rs:248-249, packages/d2b-provider-toolkit/s` |  |  |
| `RS-0709` | `docs` | `d2b-provider-toolkit` | low | actionable | leaf |  |  |  | `packages/d2b-provider-toolkit/src/testing/conformance.rs:267, packages/d2b-provider-toolki` |  |  |
| `RS-0710` | `docs` | `d2b-provider-transport-azure-relay` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/auth.rs:229-246, packages/d2b-provider-tra` |  |  |
| `RS-0711` | `docs` | `d2b-provider-transport-unix` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-unix/src/portal.rs:197-201, packages/d2b-provider-transpor` |  |  |
| `RS-0712` | `docs` | `d2b-provider-transport-vsock` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-vsock/src/auth.rs:59, packages/d2b-provider-transport-vsoc` |  |  |
| `RS-0713` | `docs` | `d2b-provider-volume-binding` | low | actionable | leaf |  |  |  | `packages/d2b-provider-volume-binding/src/facets.rs:52, packages/d2b-provider-volume-bindin` |  |  |
| `RS-0714` | `docs` | `d2b-provider-volume-local` | low | actionable | leaf |  |  |  | `src/content.rs:118-125, src/controller.rs:148-154, src/layout.rs:54-57, src/views.rs:88-92` |  |  |
| `RS-0715` | `docs` | `d2b-provider-zone` | low | actionable | leaf |  |  |  | `packages/d2b-provider-zone/src/zone_status.rs:110-114` |  |  |
| `RS-0716` | `docs` | `d2b-provider-zone-link` | low | actionable | leaf |  |  |  | `packages/d2b-provider-zone-link/src/zone_links.rs:267, packages/d2b-provider-zone-link/src` |  |  |
| `RS-0718` | `docs` | `d2b-resource-api` | medium | actionable | leaf |  |  |  | `packages/d2b-resource-api/src/authz.rs:630, packages/d2b-resource-api/src/authz.rs:864, pa` |  |  |
| `RS-0717` | `docs` | `d2b-resource-api` | medium | actionable | leaf |  |  |  | `service.rs:198, store.rs:39, adapter.rs:71, manager_backend.rs:625` |  |  |
| `RS-0719` | `docs` | `d2b-resource-client` | low | actionable | leaf |  |  |  | `packages/d2b-resource-client/src/call.rs:87, packages/d2b-resource-client/src/dispatch.rs:` |  |  |
| `RS-0720` | `docs` | `d2b-resource-runtime` | medium | actionable | leaf | applied | U2 | 4cbf82851 | `manager.rs` | # Errors on all 14 ResourceManagerClient pub methods |  |
| `RS-0721` | `docs` | `d2b-resource-runtime` | low | actionable | leaf | applied | U2 | 4cbf82851 | `manager.rs` | ResourceManagerArgs.store/providers documented |  |
| `RS-0722` | `docs` | `d2b-resource-runtime` | low | actionable | leaf | applied | U2 | 4cbf82851 | `manager.rs` | MODULE_NAME docs on five modules |  |
| `RS-0723` | `docs` | `d2b-resource-runtime` | low | actionable | leaf | applied | U2 | 75d64d039 | `guest_target.rs` | TargetControlAssignment five methods documented |  |
| `RS-0724` | `docs` | `d2b-resource-runtime` | low | actionable | leaf | applied | U2 | 69d4b615f | `target.rs` | doc contracts on target/spec/identity accessors |  |
| `RS-0725` | `docs` | `d2b-resource-types` | low | actionable | leaf |  |  |  | `packages/d2b-resource-types/src/operation.rs:64, packages/d2b-resource-types/src/operation` |  |  |
| `RS-0726` | `docs` | `d2b-session` | medium | actionable | leaf |  |  |  | `handshake.rs:25, handshake.rs:111, handshake.rs:155, handshake.rs:239` |  |  |
| `RS-0728` | `docs` | `d2b-session` | medium | actionable | leaf |  |  |  | `lifecycle.rs:38, lifecycle.rs:81, lifecycle.rs:147, record.rs:62` |  |  |
| `RS-0727` | `docs` | `d2b-session` | low | actionable | leaf |  |  |  | `operation.rs:207` |  |  |
| `RS-0729` | `docs` | `d2b-session` | low | actionable | leaf |  |  |  | `admission.rs:1182, admission.rs:1186, admission.rs:1190, admission.rs:1194` |  |  |
| `RS-0730` | `docs` | `d2b-session` | low | actionable | leaf |  |  |  | `transport.rs:208, transport.rs:212` |  |  |
| `RS-0731` | `docs` | `d2b-session-unix` | medium | actionable | leaf |  |  |  | `packages/d2b-session-unix/src/socket.rs:190, packages/d2b-session-unix/src/adapter.rs:351,` |  |  |
| `RS-0732` | `docs` | `d2b-session-unix` | low | actionable | leaf |  |  |  | `packages/d2b-session-unix/src/socket.rs:202, packages/d2b-session-unix/src/socket.rs:210, ` |  |  |
| `RS-0733` | `docs` | `d2b-telemetry` | low | actionable | leaf |  |  |  | `packages/d2b-telemetry/src/emitter.rs:236, packages/d2b-telemetry/src/audit_hash.rs:23` |  |  |
| `RS-0734` | `docs` | `d2b-zone-routing` | low | actionable | leaf |  |  |  | `packages/d2b-zone-routing/src/resolver.rs:80, packages/d2b-zone-routing/src/service.rs:208` |  |  |
| `RS-0736` | `docs` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:3455, packages/d2bd/src/composition.rs:4828` |  |  |
| `RS-0741` | `docs` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/audio_dispatch.rs:372` |  |  |
| `RS-0737` | `docs` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/composition.rs:438` |  |  |
| `RS-0738` | `docs` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/resource_plane_v3.rs:292, packages/d2bd/src/resource_plane_v3.rs:1770, p` |  |  |
| `RS-0739` | `docs` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/lib.rs:1, packages/d2bd/src/composition.rs:1` |  |  |
| `RS-0735` | `docs` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/resource_runtime.rs:8294, packages/d2bd/src/resource_runtime.rs:8390, pa` |  |  |
| `RS-0740` | `docs` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/provider_effects.rs:91, packages/d2bd/src/provider_effects.rs:711, packa` |  |  |
| `RS-0742` | `docs` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/forward_rendezvous.rs:1046-1049, packages/d2bd/src/forward_rendezvous.rs` |  |  |
| `RS-0743` | `docs` | `d2bd-runtime` | medium | actionable | leaf | applied | U2 | c4b29ded7 | `packages/d2bd-runtime/src/json_io.rs` | resolve_bundle_artifact_path and load_manifest documented (+# Errors) |  |
| `RS-0747` | `docs` | `d2bd-runtime` | medium | actionable | leaf | applied | U2 | c44ccbd0b | `packages/d2bd-runtime/src/unsafe_local_helper.rs` | module doc plus consts, enums, HelperRegistry struct,and its 7 pub methods documented |  |
| `RS-0744` | `docs` | `d2bd-runtime` | medium | actionable | leaf | applied | U2 | c4b29ded7 | `packages/d2bd-runtime/src/vm_start_support.rs` | VmStartNodeMode enum, vm_start_node_mode, tracked_role_id,and store-view resolver documented |  |
| `RS-0748` | `docs` | `d2bd-runtime` | medium | actionable | leaf | applied | U2 | c4b29ded7 | `packages/d2bd-runtime/src/exec_session.rs` | exec-session DTO fields documented (ExecStartSpec, ExecSessionInfo, Established, WorkerSpawn) |  |
| `RS-0749` | `docs` | `d2bd-runtime` | medium | actionable | leaf | applied | U2 | c4b29ded7 | `packages/d2bd-runtime/src/readiness.rs` | six readiness predicates/functions documented (+# Errors);async twin was already documented |  |
| `RS-0745` | `docs` | `d2bd-runtime` | medium | actionable | leaf | applied | U2 | c4b29ded7 | `packages/d2bd-runtime/src/broker_transport.rs` | 5 broker-transport helpers documented;default_audit_join_context has no panic post-wave0 (re-verified) |  |
| `RS-0746` | `docs` | `d2bd-runtime` | low | actionable | leaf | applied | U2 | c4b29ded7 | `packages/d2bd-runtime/src/ssh_host_key_preflight.rs` | workflow tokens dropped from doc and trace comment; 0440-with-ACL why kept |  |
| `RS-0750` | `docs` | `d2bd-runtime` | low | actionable | leaf | applied | U2 | c4b29ded7 | `packages/d2bd-runtime/src/ch_api.rs` | consts with provenance, ChApiError variants, ChVmInfo fields,and both entry fns documented |  |
| `RS-0751` | `docs` | `d2bd-runtime` | low | actionable | leaf | applied | U2 | c4b29ded7 | `packages/d2bd-runtime/src/target_runtime.rs` | AdmissionBudget/AdmissionPermit/ProviderDeployment accessors documented incl. release idempotence |  |
| `RS-0755` | `docs` | `xtask` | medium | actionable | leaf | applied | U2 | a18cc6eb1 | packages/xtask/src/blocking_census.rs, packages/xtask/src/delivery/evidence.rs, packages/xtask/src/delivery/seal.rs | per-field docs added to DeniedApi, CensusBaseline.crates, OutputDigest, EvidenceRecord, SealedLane, SealedValidation, SealRecord |  |
| `RS-0756` | `docs` | `xtask` | low | actionable | leaf | applied-variant | U2 | a18cc6eb1 | packages/xtask/src/changelog.rs, packages/xtask/src/delivery/evidence.rs, packages/xtask/src/delivery/seal.rs | Errors sections added to parse_fragment, EvidenceLane::parse, EvidenceRecord::validate, SealRecord::validate; async_gate::scan_source returns ScanOutcome not Result, so no Errors section applies there |  |
| `RS-0754` | `docs` | `xtask` | low | actionable | leaf | applied | U2 | a18cc6eb1 | packages/xtask/src/delivery/snapshot.rs, packages/xtask/src/delivery/command.rs | one-line docs added to WaveSnapshot digests/program/wave, WaveCommand as_str/parse/required_options/optional_options, WorkflowOutput ok/with_digests, WorkflowCommandHelp, and the CliOptions accessors |  |
| `RS-0752` | `docs` | `xtask` | low | actionable | leaf | applied | U2 | 6668d84dd | `provider_crate_policy.rs` | four doubled parens and whiche typo fixed; the audit's trailing \. doc lines do not exist at HEAD (grep zero), so that component is stale |  |
| `RS-0753` | `docs` | `xtask` | low | actionable | leaf | applied | U2 | bbd40b6fd | `main.rs` | doc comments above today_utc_iso8601 and civil_from_days naming the Hinnant algorithm, constants, and epoch fallback |  |
| `RS-0771` | `perf` | `d2b` | medium | actionable | leaf |  |  |  | `context.rs:570, context.rs:538` |  |  |
| `RS-0757` | `perf` | `d2b-audit` | low | actionable | leaf |  |  |  | `packages/d2b-audit/src/sink.rs:386, packages/d2b-audit/src/segment.rs:970` |  |  |
| `RS-0762` | `perf` | `d2b-broker` | medium | actionable | wide |  |  |  | `packages/d2b-broker/src/protocol.rs:86, packages/d2b-broker/src/protocol.rs:125` |  |  |
| `RS-0759` | `perf` | `d2b-broker` | low | actionable | leaf |  |  |  | `packages/d2b-broker/src/ops/nft.rs:784-790` |  |  |
| `RS-0760` | `perf` | `d2b-broker` | low | actionable | leaf |  |  |  | `packages/d2b-broker/src/ops/cgroup.rs:341, packages/d2b-broker/src/ops/cgroup.rs:368` |  |  |
| `RS-0758` | `perf` | `d2b-broker` | low | actionable | leaf |  |  |  | `packages/d2b-broker/src/state_cells.rs:470, packages/d2b-broker/src/state_cells.rs:488, pa` |  |  |
| `RS-0761` | `perf` | `d2b-broker` | low | actionable | leaf |  |  |  | `src/ops/store_view_posture.rs:194-271, src/ops/store_view_posture.rs:110-120, src/ops/stor` |  |  |
| `RS-0763` | `perf` | `d2b-bus` | low | actionable | leaf |  |  |  | `packages/d2b-bus/src/router.rs:4228-4235` |  |  |
| `RS-0764` | `perf` | `d2b-bus` | low | actionable | leaf |  |  |  | `packages/d2b-bus/src/streams.rs:642-658` |  |  |
| `RS-0765` | `perf` | `d2b-contracts-resource` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-resource/src/v3/resource_status.rs:636, packages/d2b-contracts-reso` |  |  |
| `RS-0766` | `perf` | `d2b-contracts-zone-session` | low | actionable | leaf |  |  |  | `resource_bundle.rs:382` |  |  |
| `RS-0767` | `perf` | `d2b-core` | medium | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:1636, packages/d2b-core/src/bundle_resolver.rs:16` |  |  |
| `RS-0768` | `perf` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:3717, packages/d2b-core/src/bundle_resolver.rs:37` |  |  |
| `RS-0769` | `perf` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/src/bundle_resolver.rs:1609, packages/d2b-core/src/bundle_resolver.rs:16` |  |  |
| `RS-0770` | `perf` | `d2b-host` | low | actionable | leaf |  |  |  | `packages/d2b-host/src/nftables.rs:46, packages/d2b-host/src/hardlink_farm.rs:628` |  |  |
| `RS-0772` | `perf` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `src/bin/d2b-clipd.rs:757, src/bin/d2b-clipd.rs:1043, src/bin/d2b-clipd.rs:1833, src/bin/d2` |  |  |
| `RS-0773` | `perf` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/clipd_host/niri.rs:140-159` |  |  |
| `RS-0774` | `perf` | `d2b-provider-config-nixos` | low | actionable | leaf |  |  |  | `packages/d2b-provider-config-nixos/src/controller.rs:298, packages/d2b-provider-config-nix` |  |  |
| `RS-0775` | `perf` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `src/session_children.rs:316, src/session_children.rs:317` |  |  |
| `RS-0776` | `perf` | `d2b-provider-guest` | low | actionable | leaf |  |  |  | `packages/d2b-provider-guest/src/driver.rs:863, packages/d2b-provider-guest/src/driver.rs:8` |  |  |
| `RS-0777` | `perf` | `d2b-provider-guest-cloud-hypervisor` | low | actionable | leaf |  |  |  | `shutdown.rs:505-513` |  |  |
| `RS-0778` | `perf` | `d2b-provider-guest-qemu-media` | low | actionable | leaf |  |  |  | `packages/d2b-provider-guest-qemu-media/src/controller/process_builder.rs:239` |  |  |
| `RS-0779` | `perf` | `d2b-provider-network-local` | low | actionable | leaf |  |  |  | `src/nftables.rs:266-268` |  |  |
| `RS-0780` | `perf` | `d2b-provider-network-local` | low | actionable | leaf |  |  |  | `src/observe.rs:305` |  |  |
| `RS-0781` | `perf` | `d2b-provider-notification-desktop` | low | actionable | leaf |  |  |  | `packages/d2b-provider-notification-desktop/src/host_sink.rs:265, packages/d2b-provider-not` |  |  |
| `RS-0782` | `perf` | `d2b-provider-observability-otel` | low | actionable | leaf |  |  |  | `emitter_socket.rs:139` |  |  |
| `RS-0783` | `perf` | `d2b-provider-observability-otel` | low | actionable | leaf |  |  |  | `ingress_policy.rs:203, ingress_policy.rs:368` |  |  |
| `RS-0784` | `perf` | `d2b-provider-observability-otel` | low | actionable | leaf |  |  |  | `metric_policy.rs:44` |  |  |
| `RS-0785` | `perf` | `d2b-provider-process-systemd` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process-systemd/src/operations.rs:417, packages/d2b-provider-process` |  |  |
| `RS-0786` | `perf` | `d2b-provider-toolkit` | medium | actionable | family |  |  |  | `packages/d2b-provider-toolkit/src/shared_provider.rs:944, packages/d2b-provider-toolkit/sr` |  |  |
| `RS-0787` | `perf` | `d2b-provider-transport-azure-relay` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/relay_transport.rs:706-714, packages/d2b-p` |  |  |
| `RS-0788` | `perf` | `d2b-provider-transport-azure-relay` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/guest_credential.rs:617-618` |  |  |
| `RS-0789` | `perf` | `d2b-provider-volume` | low | actionable | leaf |  |  |  | `driver.rs:437-440, driver.rs:614-617` |  |  |
| `RS-0791` | `perf` | `d2b-resource-api` | medium | actionable | leaf |  |  |  | `manager_backend.rs:1006` |  |  |
| `RS-0792` | `perf` | `d2b-resource-api` | medium | actionable | family |  |  |  | `manager_backend.rs:1081-1103, manager_backend.rs:1090` |  |  |
| `RS-0793` | `perf` | `d2b-resource-api` | low | actionable | leaf |  |  |  | `packages/d2b-resource-api/src/authz.rs:457, packages/d2b-resource-api/src/authz.rs:458` |  |  |
| `RS-0790` | `perf` | `d2b-resource-api` | low | actionable | leaf |  |  |  | `manager_backend.rs:495, manager_backend.rs:449-455` |  |  |
| `RS-0794` | `perf` | `d2b-resource-runtime` | low | actionable | leaf |  |  |  | `packages/d2b-resource-runtime/src/manager.rs:1390` |  |  |
| `RS-0795` | `perf` | `d2b-resource-runtime` | low | actionable | leaf |  |  |  | `packages/d2b-resource-runtime/src/guest_target.rs:178, packages/d2b-resource-runtime/src/g` |  |  |
| `RS-0796` | `perf` | `d2b-session` | low | actionable | leaf |  |  |  | `record.rs:125, record.rs:147` |  |  |
| `RS-0797` | `perf` | `d2b-session` | low | actionable | leaf |  |  |  | `engine.rs:1354, engine.rs:1362, scheduler.rs:72` |  |  |
| `RS-0798` | `perf` | `d2b-session` | low | actionable | leaf |  |  |  | `record.rs:120, record.rs:146` |  |  |
| `RS-0799` | `perf` | `d2b-session-unix` | low | actionable | leaf |  |  |  | `packages/d2b-session-unix/src/socket.rs:256, packages/d2b-session-unix/src/socket.rs:306, ` |  |  |
| `RS-0800` | `perf` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/resource_runtime.rs:2360, packages/d2bd/src/resource_runtime.rs:2489, pa` |  |  |
| `RS-0801` | `perf` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/process_provider_runtime.rs:333` |  |  |
| `RS-0802` | `perf` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/forward_rendezvous.rs:1356, packages/d2bd/src/forward_rendezvous.rs:1476` |  |  |
| `RS-0803` | `perf` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/shared_provider_effects.rs:1014-1016, packages/d2bd/src/audio_dispatch.r` |  |  |
| `RS-0804` | `perf` | `d2bd-runtime` | low | actionable | family |  |  |  | `public_read_model.rs:117-118` |  |  |
| `RS-0808` | `perf` | `xtask` | low | actionable | leaf |  |  |  | `packages/xtask/src/bazel_evidence.rs:397, packages/xtask/src/bazel_evidence.rs:399, packag` |  |  |
| `RS-0805` | `perf` | `xtask` | low | actionable | leaf |  |  |  | `packages/xtask/src/main.rs:431` |  |  |
| `RS-0806` | `perf` | `xtask` | low | actionable | leaf |  |  |  | `packages/xtask/src/main.rs:582` |  |  |
| `RS-0807` | `perf` | `xtask` | low | actionable | leaf |  |  |  | `packages/xtask/src/main.rs:972` |  |  |
| `RS-0960` | `conc` | `X3-cross-crate-duplication` | medium | policy-confirmed | wide |  |  |  | `clippy.toml:82, packages/xtask/data/blocking-census-baseline.json:13, packages/d2b-provide` |  |  |
| `RS-0809` | `conc` | `d2b-broker` | low | actionable | leaf |  |  |  | `src/envelope/mod.rs:1146, src/envelope/mod.rs:2144` |  |  |
| `RS-0810` | `conc` | `d2b-bus` | low | actionable | leaf |  |  |  | `packages/d2b-bus/src/registry.rs:522-523, packages/d2b-bus/src/registry.rs:573-582` |  |  |
| `RS-0811` | `conc` | `d2b-contracts-provider` | low | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/credential/service.rs:955, packages/d2b-contracts-p` |  |  |
| `RS-0812` | `conc` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `src/fd.rs:545, src/fd.rs:600, src/bin/d2b-clipd.rs:74, src/bin/d2b-clipd.rs:96` |  |  |
| `RS-0813` | `conc` | `d2b-provider-credential` | medium | policy-confirmed | leaf |  |  |  | `packages/d2b-provider-credential/src/test_support.rs:18, packages/d2b-provider-credential/` |  |  |
| `RS-0814` | `conc` | `d2b-provider-device-gpu` | medium | policy-confirmed | family |  |  |  | `packages/d2b-provider-device-gpu/src/effects_service.rs:79, packages/d2b-provider-device-g` |  |  |
| `RS-0815` | `conc` | `d2b-provider-device-security-key` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-device-security-key/src/relay_service.rs:129, packages/d2b-provider-` |  |  |
| `RS-0816` | `conc` | `d2b-provider-device-usbip` | low | policy-confirmed | leaf |  |  |  | `test_support.rs:25, test_support.rs:35, Cargo.toml:30` |  |  |
| `RS-0817` | `conc` | `d2b-provider-guest` | medium | policy-confirmed | family |  |  |  | `packages/d2b-provider-guest/src/driver.rs:502, packages/d2b-provider-guest/Cargo.toml:29, ` |  |  |
| `RS-0818` | `conc` | `d2b-provider-guest` | medium | policy-confirmed | leaf |  |  |  | `packages/d2b-provider-guest/src/test_support.rs:59, packages/d2b-provider-guest/src/test_s` |  |  |
| `RS-0819` | `conc` | `d2b-provider-process` | medium | policy-confirmed | leaf |  |  |  | `packages/d2b-provider-process/src/driver.rs:680, packages/d2b-provider-process/src/driver.` |  |  |
| `RS-0820` | `conc` | `d2b-provider-process` | low | actionable | leaf |  |  |  | `packages/d2b-provider-process/src/driver.rs:438, packages/d2b-provider-process/src/driver.` |  |  |
| `RS-0821` | `conc` | `d2b-provider-system-core` | low | actionable | leaf |  |  |  | `src/testing.rs:43, src/testing.rs:79` |  |  |
| `RS-0822` | `conc` | `d2b-provider-toolkit` | low | actionable | leaf |  |  |  | `packages/d2b-provider-toolkit/src/operations/envelope.rs:487` |  |  |
| `RS-0823` | `conc` | `d2b-provider-transport-unix` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-unix/src/portal.rs:18, packages/d2b-provider-transport-uni` |  |  |
| `RS-0824` | `conc` | `d2b-provider-user` | medium | policy-confirmed | leaf |  |  |  | `packages/d2b-provider-user/src/test_support.rs:41-42, packages/d2b-provider-user/src/test_` |  |  |
| `RS-0825` | `conc` | `d2b-provider-user` | low | actionable | leaf |  |  |  | `packages/d2b-provider-user/src/test_support.rs:77, packages/d2b-provider-user/src/test_sup` |  |  |
| `RS-0826` | `conc` | `d2b-provider-volume-binding` | low | actionable | leaf |  |  |  | `packages/d2b-provider-volume-binding/Cargo.toml:29-31, packages/d2b-provider-volume-bindin` |  |  |
| `RS-0827` | `conc` | `d2b-resource-client` | low | actionable | leaf |  |  |  | `packages/d2b-resource-client/src/zone_client.rs:510, packages/d2b-resource-client/src/zone` |  |  |
| `RS-0828` | `conc` | `d2b-resource-runtime` | low | actionable | leaf |  |  |  | `packages/d2b-resource-runtime/src/resource.rs:247` |  |  |
| `RS-0829` | `conc` | `d2b-resource-runtime` | low | actionable | leaf |  |  |  | `packages/d2b-resource-runtime/Cargo.toml:33, packages/d2b-resource-runtime/src/context.rs:` |  |  |
| `RS-0830` | `conc` | `d2b-session` | low | actionable | leaf |  |  |  | `admission.rs:756, admission.rs:1666, driver.rs:33` |  |  |
| `RS-0831` | `conc` | `d2b-unsafe-local-helper` | low | actionable | leaf |  |  |  | `packages/d2b-unsafe-local-helper/src/protocol.rs:166, packages/d2b-unsafe-local-helper/src` |  |  |
| `RS-0832` | `conc` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/effect_service_actors.rs:174, packages/d2bd/src/effect_service_actors.rs` |  |  |
| `RS-0833` | `conc` | `d2bd` | low | actionable | leaf |  |  |  | `packages/d2bd/src/forward_rendezvous.rs:332, packages/d2bd/src/forward_rendezvous.rs:414` |  |  |
| `RS-0835` | `conc` | `d2bd-runtime` | medium | policy-confirmed | leaf |  |  |  | `packages/d2bd-runtime/src/unsafe_local_helper.rs:17, packages/d2bd-runtime/Cargo.toml:34` |  |  |
| `RS-0836` | `conc` | `d2bd-runtime` | medium | policy-confirmed | leaf |  |  |  | `packages/d2bd-runtime/src/concurrency.rs:163, packages/d2bd-runtime/src/concurrency.rs:189` |  |  |
| `RS-0834` | `conc` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `resource_runtime_support.rs:158, resource_runtime_support.rs:162, resource_runtime_support` |  |  |
| `RS-0837` | `async` | `d2b-broker` | high | actionable | wide | applied | U1 | 9b64eaa27 | packages/d2b-broker/src/runtime.rs (reap fn; kernel_ops.rs callers) | bounded WNOHANG reap poll replaces the blocking waitid; orphaned helpers deleted |  |
| `RS-0841` | `async` | `d2b-broker` | high | actionable | leaf | applied | U1 | 25dfa3aee | packages/d2b-broker/src/sys.rs, packages/d2b-broker/src/ops/swtpm_dir.rs | setfacl shellout moved behind an async wrapper on a bounded worker |  |
| `RS-0842` | `async` | `d2b-broker` | high | actionable | wide | applied | U1 | a6d8fb022 | packages/d2b-broker/src/ops/media.rs | nss group lookup hoisted to a LazyLock, off the per-write path |  |
| `RS-0840` | `async` | `d2b-broker` | high | actionable | leaf | applied | U1 | f4f09c74c | packages/d2b-broker/src/ops/host_generation_handoff.rs | flock wait moved to a bounded worker (sanctioned allow reason) |  |
| `RS-0839` | `async` | `d2b-broker` | medium | actionable | leaf |  |  |  | `packages/d2b-broker/src/live_handlers.rs:1614, packages/d2b-broker/src/live_handlers.rs:18` |  |  |
| `RS-0838` | `async` | `d2b-broker` | medium | policy-confirmed | wide |  |  |  | `packages/d2b-broker/src/runtime.rs:7729, packages/d2b-broker/src/runtime.rs:7654, packages` |  |  |
| `RS-0843` | `async` | `d2b-process-conformance` | low | actionable | family |  |  |  | `packages/d2b-process-conformance/src/port.rs:99, packages/d2b-process-conformance/src/port` |  |  |
| `RS-0844` | `async` | `d2b-provider` | medium | actionable | leaf |  |  |  | `packages/d2b-provider/src/agent.rs:316-324` |  |  |
| `RS-0845` | `async` | `d2b-provider-credential-secret-service` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-credential-secret-service/src/lib.rs:1323, packages/d2b-provider-cre` |  |  |
| `RS-0846` | `async` | `d2b-provider-device-tpm` | medium | actionable | family |  |  |  | `effects_service.rs:412, effects_service.rs:467, effects_service.rs:518, effects_service.rs` |  |  |
| `RS-0847` | `async` | `d2b-provider-device-tpm` | low | actionable | leaf |  |  |  | `effects_service.rs:361, effects_service.rs:384, effects_service.rs:698` |  |  |
| `RS-0848` | `async` | `d2b-provider-network-local` | low | actionable | leaf |  |  |  | `src/observe.rs:255-260` |  |  |
| `RS-0849` | `async` | `d2b-provider-system-core` | low | actionable | leaf |  |  |  | `src/testing.rs:30, src/testing.rs:19` |  |  |
| `RS-0850` | `async` | `d2b-provider-transport-azure-relay` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/src/relay_transport.rs:823-833` |  |  |
| `RS-0851` | `async` | `d2b-provider-user` | high | policy-confirmed | leaf | policy-confirmed | U1 |  | `packages/d2b-provider-user/src/probe.rs:48, packages/d2b-provider-user/src/probe.rs:63, pa` | recorded no-op (KTD8/R14): the deliberate bounded NSS probe is the crate's documented contract - packages/d2b-provider-user/README.md:50-55, src/probe.rs:1-5; audit cluster README.md:2806 |  |
| `RS-0852` | `async` | `d2b-zone-routing` | medium | actionable | leaf |  |  |  | `packages/d2b-zone-routing/src/serving.rs:195, packages/d2b-zone-routing/src/serving.rs:207` |  |  |
| `RS-0853` | `async` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/interaction_composition.rs:5518-5530` |  |  |
| `RS-0854` | `async` | `d2bd` | medium | actionable | leaf |  |  |  | `packages/d2bd/src/shared_provider_effects.rs:316-324, packages/d2bd/src/shared_provider_ef` |  |  |
| `RS-0855` | `async` | `d2bd-runtime` | low | actionable | family |  |  |  | `packages/d2bd-runtime/src/console_session.rs:33, packages/d2bd-runtime/src/console_session` |  |  |
| `RS-0858` | `unsafe` | `d2b-broker` | medium | actionable | leaf |  |  |  | `packages/d2b-broker/src/sys.rs:593, packages/d2b-broker/src/sys.rs:615, packages/d2b-broke` |  |  |
| `RS-0857` | `unsafe` | `d2b-broker` | low | actionable | leaf |  |  |  | `packages/d2b-broker/src/ops/disk_init.rs:673, packages/d2b-broker/src/ops/disk_init.rs:661` |  |  |
| `RS-0856` | `unsafe` | `d2b-broker-fixture-syscall-surface` | medium | actionable | leaf |  |  |  | `packages/d2b-broker-fixture-syscall-surface/src/lib.rs:25-32` |  |  |
| `RS-0859` | `unsafe` | `d2b-host-activation-helper` | medium | actionable | leaf |  |  |  | `packages/d2b-host-activation-helper/src/main.rs:96, packages/d2b-host-activation-helper/sr` |  |  |
| `RS-0860` | `macro` | `d2b-provider-display-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-display-wayland/src/wayland_proxy/policy.rs:420` |  |  |
| `RS-0861` | `macro` | `d2b-resource-api` | low | actionable | leaf |  |  |  | `service.rs:2245-2267` |  |  |
| `RS-0862` | `macro` | `d2b-session` | low | actionable | leaf |  |  |  | `admission.rs:625, admission.rs:641, admission.rs:642, admission.rs:643` |  |  |
| `RS-0863` | `macro` | `xtask` | low | actionable | leaf |  |  |  | `packages/xtask/src/changelog.rs:812, packages/xtask/src/changelog.rs:886, packages/xtask/s` |  |  |
| `RS-0958` | `test` | `X3-cross-crate-duplication` | medium | actionable | family |  |  |  | `packages/d2b-provider-credential/src/test_support.rs:18, packages/d2b-provider-guest/src/t` |  |  |
| `RS-0959` | `test` | `X3-cross-crate-duplication` | low | actionable | family |  |  |  | `packages/d2b-provider-quota/Cargo.toml:17, packages/d2b-provider-resource-export/Cargo.tom` |  |  |
| `RS-0880` | `test` | `d2b` | medium | actionable | leaf |  |  |  | `packages/d2b/src/exec.rs:227-239` |  |  |
| `RS-0881` | `test` | `d2b` | low | actionable | leaf |  |  |  | `packages/d2b/src/exec.rs:383-397` |  |  |
| `RS-0866` | `test` | `d2b-broker` | low | actionable | leaf |  |  |  | `packages/d2b-broker/tests/pidfd_handoff_scm_rights.rs:90` |  |  |
| `RS-0864` | `test` | `d2b-broker-composition` | low | actionable | leaf |  |  |  | `packages/d2b-broker-composition/src/seam.rs:523` |  |  |
| `RS-0865` | `test` | `d2b-broker-composition` | low | actionable | leaf |  |  |  | `packages/d2b-broker-composition/src/seam.rs:731, packages/d2b-broker-composition/src/seam.` |  |  |
| `RS-0867` | `test` | `d2b-bus` | high | actionable | leaf | applied | U1 | 01e8edab3 | packages/d2b-bus/src/metrics.rs | test now drives BusMetrics::emit over every closed label domain; mutation-verified |  |
| `RS-0868` | `test` | `d2b-bus` | medium | actionable | leaf |  |  |  | `packages/d2b-bus/src/session_seam_tests.rs:1623-1625, packages/d2b-bus/src/session_seam_te` |  |  |
| `RS-0869` | `test` | `d2b-bus` | low | actionable | leaf |  |  |  | `packages/d2b-bus/src/operations.rs:1057-1060` |  |  |
| `RS-0870` | `test` | `d2b-contracts-control` | medium | actionable | leaf |  |  |  | `public_wire.rs:167, public_wire.rs:175` |  |  |
| `RS-0871` | `test` | `d2b-contracts-provider` | medium | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/credential/service.rs:1071, packages/d2b-contracts-` |  |  |
| `RS-0872` | `test` | `d2b-contracts-provider` | medium | actionable | leaf |  |  |  | `packages/d2b-contracts-provider/src/v3/credential_controller.rs:691, packages/d2b-contract` |  |  |
| `RS-0873` | `test` | `d2b-contracts-zone-session` | medium | actionable | leaf |  |  |  | `src/v3/component_session.rs:2445, src/v3/component_session.rs:1829, src/v3/component_sessi` |  |  |
| `RS-0874` | `test` | `d2b-contracts-zone-session` | medium | actionable | leaf |  |  |  | `emergency_policy.rs:236, emergency_policy.rs:112` |  |  |
| `RS-0877` | `test` | `d2b-core` | low | actionable | leaf |  |  |  | `packages/d2b-core/tests/bundle_resolver_tamper.rs:149` |  |  |
| `RS-0875` | `test` | `d2b-core-controller` | medium | actionable | leaf |  |  |  | `authority.rs:1824, authority.rs:1968, authority.rs:1899` |  |  |
| `RS-0876` | `test` | `d2b-core-controller` | medium | actionable | leaf |  |  |  | `authority_persistence.rs:246-320` |  |  |
| `RS-0878` | `test` | `d2b-host` | medium | actionable | family |  |  |  | `packages/d2b-host/src/nftables.rs:245` |  |  |
| `RS-0879` | `test` | `d2b-host` | low | actionable | leaf |  |  |  | `packages/d2b-host/src/bin/d2b-activation-helper.rs:792` |  |  |
| `RS-0882` | `test` | `d2b-provider-activation-nixos` | low | actionable | leaf |  |  |  | `packages/d2b-provider-activation-nixos/tests/reconcile.rs:439, packages/d2b-provider-activ` |  |  |
| `RS-0883` | `test` | `d2b-provider-audio-pipewire` | low | actionable | leaf |  |  |  | `tests/authority.rs:26-31, src/authority.rs:236-241` |  |  |
| `RS-0884` | `test` | `d2b-provider-audio-pipewire` | low | actionable | leaf |  |  |  | `tests/mediator.rs:13-24` |  |  |
| `RS-0885` | `test` | `d2b-provider-clipboard-wayland` | medium | actionable | leaf |  |  |  | `src/bin/d2b-clipd.rs:4021, src/bin/d2b-clipd.rs:3787` |  |  |
| `RS-0886` | `test` | `d2b-provider-clipboard-wayland` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/history.rs:150-172, packages/d2b-provider-clip` |  |  |
| `RS-0887` | `test` | `d2b-provider-clipboard-wayland` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/controller/mod.rs:85-141, packages/d2b-provide` |  |  |
| `RS-0888` | `test` | `d2b-provider-clipboard-wayland` | low | actionable | leaf |  |  |  | `packages/d2b-provider-clipboard-wayland/src/clipd_host/fallback.rs:78-83, packages/d2b-pro` |  |  |
| `RS-0889` | `test` | `d2b-provider-config-nixos` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-config-nixos/src/service.rs:70-90, packages/d2b-provider-config-nixo` |  |  |
| `RS-0890` | `test` | `d2b-provider-config-nixos` | low | actionable | leaf |  |  |  | `packages/d2b-provider-config-nixos/src/service.rs:20-21, packages/d2b-provider-config-nixo` |  |  |
| `RS-0891` | `test` | `d2b-provider-credential-entra` | low | actionable | leaf |  |  |  | `packages/d2b-provider-credential-entra/src/lib.rs:1361, packages/d2b-provider-credential-e` |  |  |
| `RS-0892` | `test` | `d2b-provider-credential-managed-identity` | low | actionable | leaf |  |  |  | `tests/conformance.rs:77-82, tests/topology.rs:33-38, tests/topology.rs:68-76` |  |  |
| `RS-0893` | `test` | `d2b-provider-credential-secret-service` | low | actionable | leaf |  |  |  | `packages/d2b-provider-credential-secret-service/tests/faults.rs:25, packages/d2b-provider-` |  |  |
| `RS-0894` | `test` | `d2b-provider-device-gpu` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-device-gpu/src/authority.rs:168, packages/d2b-provider-device-gpu/te` |  |  |
| `RS-0896` | `test` | `d2b-provider-device-usbip` | medium | actionable | leaf |  |  |  | `tests/arbitration_conflict.rs:8-19, src/arbitration.rs:81-84, src/arbitration.rs:113-115, ` |  |  |
| `RS-0897` | `test` | `d2b-provider-device-usbip` | medium | actionable | leaf |  |  |  | `src/reconcile_state.rs:51-308, src/state_machine.rs:98-100` |  |  |
| `RS-0895` | `test` | `d2b-provider-device-usbip` | low | actionable | leaf |  |  |  | `tests/conformance.rs:63-66` |  |  |
| `RS-0898` | `test` | `d2b-provider-display-wayland` | high | actionable | leaf | applied | U1 | 3b964169f | packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs | registry-handler tests assert advertised-global outcomes; mutation-verified |  |
| `RS-0900` | `test` | `d2b-provider-guest-azure-container-apps` | medium | actionable | leaf |  |  |  | `src/controller.rs:264-266, tests/provider_lifecycle.rs:210-225` |  |  |
| `RS-0899` | `test` | `d2b-provider-guest-azure-container-apps` | low | actionable | leaf |  |  |  | `tests/provider_lifecycle.rs:536` |  |  |
| `RS-0901` | `test` | `d2b-provider-guest-azure-virtual-machine` | low | actionable | leaf |  |  |  | `tests/error_redaction.rs:17` |  |  |
| `RS-0902` | `test` | `d2b-provider-guest-cloud-hypervisor` | medium | actionable | leaf |  |  |  | `finalize_ordering_test.rs:286` |  |  |
| `RS-0903` | `test` | `d2b-provider-guest-qemu-media` | low | actionable | leaf |  |  |  | `packages/d2b-provider-guest-qemu-media/tests/lifecycle.rs:132, packages/d2b-provider-guest` |  |  |
| `RS-0904` | `test` | `d2b-provider-observability-otel` | medium | actionable | leaf |  |  |  | `metric_policy.rs:145, metric_policy.rs:150` |  |  |
| `RS-0905` | `test` | `d2b-provider-provider` | medium | actionable | leaf |  |  |  | `src/providers.rs:206, src/driver.rs:1147` |  |  |
| `RS-0906` | `test` | `d2b-provider-shell-terminal` | low | actionable | leaf |  |  |  | `tests/supervisor_runtime.rs:17, tests/supervisor_runtime.rs:81, tests/supervisor_runtime.r` |  |  |
| `RS-0907` | `test` | `d2b-provider-supervisor` | low | policy-confirmed | leaf |  |  |  | `packages/d2b-provider-supervisor/src/broker.rs:2040-2043` |  |  |
| `RS-0908` | `test` | `d2b-provider-system-core` | low | actionable | leaf |  |  |  | `tests/host_reconciliation.rs:204, tests/host_reconciliation.rs:230` |  |  |
| `RS-0909` | `test` | `d2b-provider-transport-vsock` | low | actionable | leaf |  |  |  | `packages/d2b-provider-transport-vsock/tests/observe.rs:14-15` |  |  |
| `RS-0910` | `test` | `d2b-provider-user` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-user/src/driver.rs:789-848, packages/d2b-provider-user/src/driver.rs` |  |  |
| `RS-0911` | `test` | `d2b-provider-wayland-policy` | low | actionable | leaf |  |  |  | `packages/d2b-provider-wayland-policy/tests/registration.rs:93` |  |  |
| `RS-0912` | `test` | `d2b-provider-zone-link` | low | actionable | leaf |  |  |  | `packages/d2b-provider-zone-link/src/zone_links.rs:2519, packages/d2b-provider-zone-link/sr` |  |  |
| `RS-0914` | `test` | `d2b-resource-api` | medium | actionable | leaf |  |  |  | `packages/d2b-resource-api/src/manager_backend/tests.rs:1459, packages/d2b-resource-api/src` |  |  |
| `RS-0913` | `test` | `d2b-resource-api` | low | actionable | leaf |  |  |  | `service.rs:3377` |  |  |
| `RS-0915` | `test` | `d2b-resource-client` | low | actionable | leaf |  |  |  | `packages/d2b-resource-client/src/process_attach.rs:515, packages/d2b-resource-client/src/p` |  |  |
| `RS-0916` | `test` | `d2b-resource-runtime` | high | actionable | leaf | applied-variant | U1 | 8b191fe39 | packages/d2b-resource-runtime/src/revision.rs (display test) | claim corrected: the committed line was a tautological bare-epoch assertion (not an assertion that cannot pass); the audit's quoted literal is a tool-output redaction artifact, absent from the file and from git history; the row's own fix text applied by deleting the redundant assertion |  |
| `RS-0917` | `test` | `d2b-resource-runtime` | low | actionable | leaf |  |  |  | `packages/d2b-resource-runtime/src/revision.rs:182` |  |  |
| `RS-0918` | `test` | `d2b-resource-runtime` | low | actionable | leaf |  |  |  | `packages/d2b-resource-runtime/src/lib.rs:66` |  |  |
| `RS-0919` | `test` | `d2b-session` | low | actionable | leaf |  |  |  | `tests/admission.rs:78, tests/admission.rs:96, tests/admission.rs:139` |  |  |
| `RS-0920` | `test` | `d2b-sk-frontend` | medium | actionable | leaf |  |  |  | `packages/d2b-sk-frontend/src/uhid.rs:175, packages/d2b-sk-frontend/src/uhid.rs:186` |  |  |
| `RS-0921` | `test` | `d2b-telemetry` | low | actionable | leaf |  |  |  | `packages/d2b-telemetry/src/meter_registry.rs:176-180` |  |  |
| `RS-0922` | `test` | `d2b-unsafe-local-helper` | low | actionable | leaf |  |  |  | `packages/d2b-unsafe-local-helper/src/runtime.rs:1566-1576, packages/d2b-unsafe-local-helpe` |  |  |
| `RS-0923` | `test` | `d2b-zone-routing` | low | actionable | leaf |  |  |  | `packages/d2b-zone-routing/src/engine.rs:3333` |  |  |
| `RS-0924` | `test` | `d2b-zone-routing` | low | actionable | leaf |  |  |  | `packages/d2b-zone-routing/src/router.rs:517` |  |  |
| `RS-0925` | `test` | `d2bd-runtime` | high | actionable | leaf | applied | U1 | bea8fa96d | packages/d2bd-runtime/src/runtime_process.rs | sd_notify tests assert observable tracing outcomes; two mutations verified |  |
| `RS-0926` | `test` | `d2bd-runtime` | low | actionable | leaf |  |  |  | `packages/d2bd-runtime/src/daemon_audit.rs:2367` |  |  |
| `RS-0928` | `test` | `xtask` | low | actionable | leaf |  |  |  | `packages/xtask/src/delivery/command.rs:1059, packages/xtask/src/delivery/command.rs:1079` |  |  |
| `RS-0927` | `test` | `xtask` | low | actionable | leaf |  |  |  | `packages/xtask/src/gen_layer_catalogs.rs:705` |  |  |
| `RS-0933` | `supply` | `X1-supply-chain` | medium | actionable | leaf |  |  |  | `packages/d2b-session/Cargo.toml:17, packages/d2b-session/BUILD.bazel:28` |  |  |
| `RS-0934` | `supply` | `X1-supply-chain` | medium | actionable | leaf |  |  |  | `packages/d2b-session/Cargo.toml:19, packages/d2b-session/BUILD.bazel:31` |  |  |
| `RS-0935` | `supply` | `X1-supply-chain` | medium | actionable | leaf |  |  |  | `packages/d2b-session/Cargo.toml:44` |  |  |
| `RS-0936` | `supply` | `X1-supply-chain` | medium | actionable | leaf |  |  |  | `packages/d2b-telemetry/Cargo.toml:14` |  |  |
| `RS-0937` | `supply` | `X1-supply-chain` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-quota/Cargo.toml:24` |  |  |
| `RS-0938` | `supply` | `X1-supply-chain` | medium | actionable | leaf |  |  |  | `packages/d2b-bus/Cargo.toml:41` |  |  |
| `RS-0939` | `supply` | `X1-supply-chain` | medium | actionable | leaf |  |  |  | `dependencies` |  |  |
| `RS-0940` | `supply` | `X1-supply-chain` | medium | actionable | leaf |  |  |  | `dependencies` |  |  |
| `RS-0941` | `supply` | `X1-supply-chain` | medium | actionable | leaf |  |  |  | `dependencies` |  |  |
| `RS-0942` | `supply` | `X1-supply-chain` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-device-gpu/Cargo.toml:20` |  |  |
| `RS-0943` | `supply` | `X1-supply-chain` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-device-gpu/Cargo.toml:25, packages/d2b-provider-device-gpu/BUILD.baz` |  |  |
| `RS-0944` | `supply` | `X1-supply-chain` | medium | actionable | leaf |  |  |  | `packages/d2b/Cargo.toml:23, packages/d2b/BUILD.bazel:55` |  |  |
| `RS-0946` | `supply` | `X1-supply-chain` | medium | actionable | wide |  |  |  | `deny.toml:2, packages/d2b-broker/Cargo.toml:53, packages/d2bd-runtime/Cargo.toml:33` |  |  |
| `RS-0947` | `supply` | `X1-supply-chain` | medium | actionable | wide |  |  |  | `Cargo.toml:202, deny.toml:2` |  |  |
| `RS-0948` | `supply` | `X1-supply-chain` | medium | actionable | leaf |  |  |  | `packages/d2b-provider-transport-azure-relay/Cargo.toml:34, packages/d2b-provider-transport` |  |  |
| `RS-0950` | `supply` | `X1-supply-chain` | medium | actionable | wide |  |  |  | `packages/Cargo.guest.lock:1, flake.nix:389` |  |  |
| `RS-0945` | `supply` | `X1-supply-chain` | low | actionable | wide |  |  |  | `Cargo.toml:184-229, packages/d2b-broker/Cargo.toml:60, packages/d2b-broker/Cargo.toml:63, ` |  |  |
| `RS-0949` | `supply` | `X1-supply-chain` | low | actionable | wide |  |  |  | `deny.toml:2` |  |  |
| `RS-0951` | `supply` | `X1-supply-chain` | low | actionable | wide |  |  |  | `deny.toml:21` |  |  |
| `RS-0929` | `supply` | `d2b-provider-audio-pipewire` | low | actionable | leaf |  |  |  | `Cargo.toml:24, Cargo.toml:25` |  |  |
| `RS-0930` | `supply` | `d2b-provider-device-gpu` | low | actionable | leaf |  |  |  | `packages/d2b-provider-device-gpu/Cargo.toml:20, packages/d2b-provider-device-gpu/Cargo.tom` |  |  |
| `RS-0931` | `supply` | `d2b-provider-guest-azure-container-apps` | low | actionable | leaf |  |  |  | `Cargo.toml:21` |  |  |
| `RS-0932` | `supply` | `d2b-provider-quota` | low | actionable | leaf |  |  |  | `dependencies` |  |  |
