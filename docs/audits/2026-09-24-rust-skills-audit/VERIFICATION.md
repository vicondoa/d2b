# VERIFICATION - rust skills audit (plan Step 5, independent)

Baseline: branch `v3` @ `6ebdd4cec`  -  Date: 2026-09-24  -  Verifier: separate
clean-context agent (no consolidation state trusted; every number recomputed).

**Method.** All mechanical checks run from the repo root with
`python3 .scratch/rust-skills-audit-verify.py` (read-only; imports
`.scratch/parse_lanes.py` and `.scratch/consolidate.py` for parsing and
consolidation, recomputing their outputs rather than trusting the report).
The script parses `U1-constraints.md` sections (a) and (e), the root
`Cargo.toml` member list, all 110 lane files, and `README.md` sections 1, 2,
4, 5, 6; it resolves every anchor against the working tree at the baseline OID
and prints each cited line's content. Anchor plausibility (the cited line
range relates to the claim) was judged by the verifier from the printed line
contents for all 568 checked anchors, with direct reads for every `sev=high`
anchor and for the ambiguous basename resolutions. The check script output
below is abbreviated; the full transcript is reproducible by re-running the
script.

## check 1: lane completeness - pass

Command: `python3 .scratch/rust-skills-audit-verify.py` (check 1 section).

```
expected lane ids: 110  actual lane files: 110
missing: []
extra: []
check 1: PASS
```

The expected set is expanded from the U1 section (a) lane map (`d2bd-p1`..`-p8`
style ranges, single-part rows, `tail-1`..`tail-6`) plus the three cross-cutting
lanes `X1-supply-chain`, `X2-generated-boundary`, `X3-cross-crate-duplication`.
Every mapped lane id has exactly one file in `lane/`; no extra files. 110 = 107
crate lanes + 3 cross lanes, as the report states.

## check 2: crate coverage - pass

```
members: 94  covered crates: 94
crates with >1 owner group: []
uncovered: []
stray (not members): []
partition summary: 17 multi-part crates, 50 whole-crate lanes, 27 tail crates (94 = 94 members)
check 2: PASS
```

All 94 `packages/*` members of the root `Cargo.toml` appear exactly once across
the lane files (tail lanes counted per `## <crate>` section; part lanes counted
per crate). No member is uncovered, no lane covers a non-member, and no crate is
claimed by two different lane owners. The partition matches the README method
paragraph: 51 part lanes over 17 crates, 50 whole-crate lanes, 6 tail lanes
covering 27 crates (17 + 50 + 27 = 94).

## check 3: schema conformance - pass

Command: `python3 .scratch/rust-skills-audit-verify.py` (check 3 section),
which imports `.scratch/parse_lanes.py` and parses all 110 lane files.

```
lane files parsed: 110  findings: 1022
schema violations: 0
findings missing evidence: []
findings missing anchors: []
check 3: PASS
```

Zero violations of the lane-file grammar (finding rows matching the exact field
pattern `- <lane-id>#<k> sev=... blast=... effort=... verdict=... - what - fix: ... - [path:line, ...]`
with a following `evidence:` line and >=1 well-formed `[path:line]` anchor;
local id matches lane id; coverage lines well-formed). Every one of the 1022
raw finding rows carries an evidence line and at least one anchor.

## check 4: coverage matrix completeness - pass

```
matrix rows: 94 (expect 94)  cols: 16
cells: N/A=449 clean=504 numeric=461 X1=90
bad N/A cells (u1!=0 or findings!=0): []
bad clean cells (u1==0 or findings!=0): []
bad numeric cells (count mismatch): []
zero-u1 numeric set == README-named 8 cells: True
check 4: PASS
```

README section 5 has all 94 crate rows x 16 lens columns. Every `N/A` cell
(449 of them) corresponds to a zero seed-hit row for that crate in U1 section
(e) AND zero findings; every `clean` cell has nonzero U1 seed mass and zero
findings; every numeric cell equals the consolidated finding count for that
(crate, lens). The eight numeric cells whose U1 pre-scan row is zero
(`d2b-controller-toolkit`/type, `d2b-provider-credential-entra`/idiom,
`d2b-provider-credential-secret-service`/idiom,
`d2b-provider-guest-azure-container-apps`/type,
`d2b-provider-seccomp-profile`/idiom, `d2b-provider-system-core`/idiom,
`d2b-provider-volume`/idiom, `d2b-provider-wayland-session`/err - lanes read
deeper than the single-pass pre-scan) match exactly the set the README's
coverage note names. Spot-checks against U1 section (e) (10 `N/A` cells:
d2b/ffi=0, d2b/macro=0, d2b-audit/async=0, d2b-audit/ffi=0,
d2b-broker-composition/type=0, d2b-broker-composition/conc=0,
d2b-broker-composition/ffi=0, d2b-broker-fixture-handlers/idiom=0,
d2b-broker-fixture-handlers/type=0, d2b-broker-fixture-handlers/err=0;
10 `clean` cells: d2b/serde=194, d2b/obs=33, d2b/conc=44, d2b/async=77,
d2b/unsafe=4, d2b-audit/serde=84, d2b-audit/obs=5, d2b-audit/conc=18,
d2b-audit/unsafe=1, d2b-audit/macro=1) all agree with the U1 seed matrix.

## check 5: anchor existence - pass

```
high findings: 13 (raw high: 14)
medium/low rows: 1008; every-5th sample: 201
anchors checked: 568  failures: 0
check 5 (mechanical): PASS
```

All 13 consolidated `sev=high` findings (every anchor) and a deterministic 20%
sample of medium/low rows - every 5th row of the 1008 medium/low raw findings
sorted by (lens, crate, lane-file line) - were resolved and re-read. 568
anchors total; every anchor resolves to an existing file whose line count is
>= the cited line, and the cited line/range plausibly relates to the claim
(judged from the printed line contents; the 13 high anchors and all ambiguous
basename resolutions were additionally read directly). Resolution followed the
rule: repo-relative first, then crate-relative via `packages/<crate>/`, then
`packages/<crate>/src/` and `packages/<crate>/tests/`, then a basename search
under `packages/<crate>/` preferring `src/` (e.g. `admission.rs:1182` ->
`packages/d2b-session/src/admission.rs:1182`; `public_wire.rs:167` ->
`packages/d2b-contracts-control/src/public_wire.rs:167`; `controller.rs:1808`
-> `packages/d2b-provider-guest-cloud-hypervisor/src/controller.rs:1808`).

The raw corpus carries 14 high rows; the 14th (`tail-1#4`, unsafe,
d2b-host-activation-helper, `walk_dir` fdopendir handle leak) was merged into
`tail-1#3` by consolidation (recorded merge, overlapping anchors) - a merge,
not a demotion, consistent with README section 7's "no demotions" record. Its
anchors (`packages/d2b-host-activation-helper/src/main.rs:194,218,222,260,265`)
all exist and relate to the claim.

Deterministic sample list (201 rows; every 5th medium/low row in
(lens, crate, line) order; first anchor shown):

```
d2b-audit#7 [api] d2b-audit packages/d2b-audit/src/lib.rs:16
d2b-broker-p5#2 [api] d2b-broker-p5 packages/d2b-broker/src/ops/cgroup.rs:126-129
d2b-bus-p2#5 [api] d2b-bus-p2 packages/d2b-bus/src/lib.rs:34
d2b-contracts-control#5 [api] d2b-contracts-control cli_output.rs:6
d2b-contracts-resource-p2#6 [api] d2b-contracts-resource-p2 packages/d2b-contracts-resource/src/v3/identity.rs:269-270
d2b-core-controller-p1#2 [api] d2b-core-controller-p1 packages/d2b-core-controller/src/controller_assignment.rs:2707
d2b-core-p1#5 [api] d2b-core-p1 packages/d2b-core/src/bundle_resolver.rs:620
d2b-core-p2#6 [api] d2b-core-p2 packages/d2b-core/src/privileges.rs:741
d2b-process-conformance#3 [api] d2b-process-conformance packages/d2b-process-conformance/src/lib.rs:52
d2b-provider-activation-nixos#2 [api] d2b-provider-activation-nixos packages/d2b-provider-activation-nixos/src/controller.rs:14
d2b-provider-config-nixos#5 [api] d2b-provider-config-nixos packages/d2b-provider-config-nixos/src/service.rs:296-298
d2b-provider-device-gpu#6 [api] d2b-provider-device-gpu packages/d2b-provider-device-gpu/src/lib.rs:12
d2b-provider-device-tpm#5 [api] d2b-provider-device-tpm effects_service.rs:341
d2b-provider-display-wayland-p1#8 [api] d2b-provider-display-wayland-p1 packages/d2b-provider-display-wayland/src/wayland_proxy/mod.rs:12
d2b-provider-guest-azure-virtual-machine#10 [api] d2b-provider-guest-azure-virtual-machine src/controller/mod.rs:211
d2b-provider-guest-cloud-hypervisor#14 [api] d2b-provider-guest-cloud-hypervisor guest_local.rs:49-166
d2b-provider-notification-desktop#7 [api] d2b-provider-notification-desktop packages/d2b-provider-notification-desktop/src/controller.rs:110
d2b-provider-provider#3 [api] d2b-provider-provider src/driver.rs:215
tail-3#8 [api] d2b-provider-role packages/d2b-provider-role/Cargo.toml:17
d2b-provider-system-core#5 [api] d2b-provider-system-core src/lib.rs:40
d2b-provider-toolkit-p2#4 [api] d2b-provider-toolkit-p2 packages/d2b-provider-toolkit/src/server/service.rs:297-299
d2b-provider-volume#4 [api] d2b-provider-volume driver.rs:246-250
d2b-provider-zone-link#6 [api] d2b-provider-zone-link packages/d2b-provider-zone-link/src/zonelink.rs:178
d2b-resource-runtime-p2#8 [api] d2b-resource-runtime-p2 packages/d2b-resource-runtime/src/context.rs:364-366
d2b-session-p2#5 [api] d2b-session-p2 engine.rs:166
d2bd-p2#5 [api] d2bd-p2 packages/d2bd/src/audio_host_controller.rs:68
d2bd-runtime-p1#5 [api] d2bd-runtime-p1 public_read_model.rs:51-53
d2bd-runtime-p4#11 [api] d2bd-runtime-p4 packages/d2bd-runtime/src/console_session.rs:66
d2b-provider-credential-secret-service#5 [async] d2b-provider-credential-secret-service packages/d2b-provider-credential-secret-service/src/lib.rs:1323
d2b-provider-system-core#11 [async] d2b-provider-system-core src/testing.rs:30
d2bd-runtime-p4#20 [async] d2bd-runtime-p4 packages/d2bd-runtime/src/console_session.rs:33
d2b-provider-clipboard-wayland-p1#12 [conc] d2b-provider-clipboard-wayland-p1 src/fd.rs:545
d2b-provider-guest#2 [conc] d2b-provider-guest packages/d2b-provider-guest/src/driver.rs:502
d2b-provider-toolkit-p1#8 [conc] d2b-provider-toolkit-p1 packages/d2b-provider-toolkit/src/operations/envelope.rs:487
d2b-resource-client#8 [conc] d2b-resource-client packages/d2b-resource-client/src/zone_client.rs:510
d2bd-p7#9 [conc] d2bd-p7 packages/d2bd/src/effect_service_actors.rs:174
d2b-audit#10 [docs] d2b-audit packages/d2b-audit/src/export.rs:74
d2b-broker-p3#9 [docs] d2b-broker-p3 packages/d2b-broker/src/ops/sysctl.rs:32
d2b-broker-p4#5 [docs] d2b-broker-p4 packages/d2b-broker/src/ops/route.rs:29
d2b-broker-p6#11 [docs] d2b-broker-p6 src/envelope/mod.rs:1118
d2b-bus-p1#7 [docs] d2b-bus-p1 packages/d2b-bus/src/router.rs:1126
d2b-bus-p2#8 [docs] d2b-bus-p2 packages/d2b-bus/src/streams.rs:39-40
d2b-contracts-control#10 [docs] d2b-contracts-control cli_output.rs:10
d2b-contracts-provider-p2#9 [docs] d2b-contracts-provider-p2 packages/d2b-contracts-provider/src/v3/credential_controller.rs:155
d2b-contracts-zone-session-p1#5 [docs] d2b-contracts-zone-session-p1 src/v3/component_session.rs:27
d2b-core-controller-p2#8 [docs] d2b-core-controller-p2 authority_persistence.rs:50
d2b-core-p2#10 [docs] d2b-core-p2 packages/d2b-core/src/manifest_v04.rs:1
d2b-p2#11 [docs] d2b-p2 packages/d2b/src/doctor.rs:91
d2b-provider-activation-nixos#6 [docs] d2b-provider-activation-nixos packages/d2b-provider-activation-nixos/src/controller.rs:118
d2b-provider-clipboard-wayland-p1#9 [docs] d2b-provider-clipboard-wayland-p1 src/fd.rs:549
tail-2#2 [docs] d2b-provider-command packages/d2b-provider-command/src/command.rs:38
d2b-provider-credential-secret-service#3 [docs] d2b-provider-credential-secret-service packages/d2b-provider-credential-secret-service/src/lib.rs:523
d2b-provider-device-security-key#4 [docs] d2b-provider-device-security-key packages/d2b-provider-device-security-key/src/lease.rs:150-303
d2b-provider-display-wayland-p1#10 [docs] d2b-provider-display-wayland-p1 packages/d2b-provider-display-wayland/src/lib.rs:14
d2b-provider-guest-azure-container-apps#9 [docs] d2b-provider-guest-azure-container-apps src/lib.rs:7
d2b-provider-guest-cloud-hypervisor#15 [docs] d2b-provider-guest-cloud-hypervisor descriptor.rs:423-429
d2b-provider-notification-desktop#12 [docs] d2b-provider-notification-desktop packages/d2b-provider-notification-desktop/src/runtime.rs:88-89
tail-3#2 [docs] d2b-provider-process-minijail packages/d2b-provider-process-minijail/src/launch.rs:33
tail-3#7 [docs] d2b-provider-role packages/d2b-provider-role/src/lib.rs:1
d2b-provider-toolkit-p1#7 [docs] d2b-provider-toolkit-p1 packages/d2b-provider-toolkit/src/base/runtime.rs:248-249
d2b-provider-volume-binding#3 [docs] d2b-provider-volume-binding packages/d2b-provider-volume-binding/src/facets.rs:52
d2b-resource-api-p1#9 [docs] d2b-resource-api-p1 service.rs:198
d2b-resource-runtime-p1#8 [docs] d2b-resource-runtime-p1 packages/d2b-resource-runtime/src/manager.rs:850
d2b-session-p1#3 [docs] d2b-session-p1 handshake.rs:25
d2b-session-p2#10 [docs] d2b-session-p2 admission.rs:1182
d2b-unsafe-local-helper#7 [docs] d2b-unsafe-local-helper packages/d2b-unsafe-local-helper/src/lib.rs:1-4
d2bd-p4#6 [docs] d2bd-p4 packages/d2bd/src/composition.rs:438
d2bd-p8#13 [docs] d2bd-p8 packages/d2bd/src/forward_rendezvous.rs:1046-1049
d2bd-runtime-p2#7 [docs] d2bd-runtime-p2 runtime_capability.rs:47-64
d2bd-runtime-p4#16 [docs] d2bd-runtime-p4 packages/d2bd-runtime/src/wire_response_helpers.rs:7
xtask-p3#6 [docs] xtask-p3 packages/xtask/src/delivery/snapshot.rs:87
d2b-audit#9 [err] d2b-audit packages/d2b-audit/src/segment.rs:694
d2b-broker-p6#8 [err] d2b-broker-p6 src/ops/exec_reconcile.rs:1238-1265
d2b-contracts-provider-p1#8 [err] d2b-contracts-provider-p1 packages/d2b-contracts-provider/src/v3/provider_registry.rs:186
d2b-contracts-resource-p1#6 [err] d2b-contracts-resource-p1 packages/d2b-contracts-resource/src/v3/operations/error.rs:93
d2b-core-p1#10 [err] d2b-core-p1 packages/d2b-core/src/bundle_resolver.rs:1405
d2b-p2#10 [err] d2b-p2 packages/d2b/src/host.rs:274
d2b-provider-clipboard-wayland-p1#5 [err] d2b-provider-clipboard-wayland-p1 src/bin/d2b-clipd.rs:3550
d2b-provider-credential-managed-identity#3 [err] d2b-provider-credential-managed-identity lib.rs:1261-1262
d2b-provider-display-wayland-p2#7 [err] d2b-provider-display-wayland-p2 src/process.rs:335
d2b-provider-notification-desktop#10 [err] d2b-provider-notification-desktop packages/d2b-provider-notification-desktop/src/host_sink.rs:185
d2b-provider-supervisor#6 [err] d2b-provider-supervisor packages/d2b-provider-supervisor/src/adapter.rs:888-890
d2b-provider-toolkit-p2#7 [err] d2b-provider-toolkit-p2 packages/d2b-provider-toolkit/src/shared_provider.rs:615
d2b-provider-transport-azure-relay#9 [err] d2b-provider-transport-azure-relay packages/d2b-provider-transport-azure-relay/src/guest_zone_link.rs:26-30
d2b-resource-client#6 [err] d2b-resource-client packages/d2b-resource-client/src/call.rs:281
d2b-telemetry#1 [err] d2b-telemetry packages/d2b-telemetry/src/emitter.rs:201-206
d2bd-p3#1 [err] d2bd-p3 packages/d2bd/src/composition.rs:22793-22797
d2bd-p6#4 [err] d2bd-p6 packages/d2bd/src/resource_plane_v3.rs:1956
d2bd-runtime-p3#4 [err] d2bd-runtime-p3 packages/d2bd-runtime/src/exec_session.rs:939
d2b-audit#2 [idiom] d2b-audit packages/d2b-audit/src/export.rs:103
d2b-broker-p2#1 [idiom] d2b-broker-p2 packages/d2b-broker/src/runtime.rs:10132
d2b-broker-p6#4 [idiom] d2b-broker-p6 src/ops/device_worker.rs:312-335
d2b-contracts-broker#1 [idiom] d2b-contracts-broker packages/d2b-contracts-broker/src/kernel_client.rs:225-227
d2b-contracts-provider-p2#1 [idiom] d2b-contracts-provider-p2 packages/d2b-contracts-provider/src/v3/semantic_services/child_resources.rs:573
d2b-contracts-zone-session-p1#1 [idiom] d2b-contracts-zone-session-p1 src/v3/component_session.rs:1935
d2b-core-p2#1 [idiom] d2b-core-p2 packages/d2b-core/src/static_invariants.rs:162
d2b-p2#5 [idiom] d2b-p2 packages/d2b/src/zone_audit.rs:591
d2b-provider-clipboard-wayland-p1#3 [idiom] d2b-provider-clipboard-wayland-p1 src/bin/d2b-clipd.rs:1131
d2b-provider-clipboard-wayland-p2#4 [idiom] d2b-provider-clipboard-wayland-p2 packages/d2b-provider-clipboard-wayland/src/clipd_host/fallback.rs:40-46
d2b-provider-device-gpu#1 [idiom] d2b-provider-device-gpu packages/d2b-provider-device-gpu/src/authority.rs:401
d2b-provider-device-usbip#1 [idiom] d2b-provider-device-usbip driver.rs:267-281
d2b-provider-display-wayland-p1#5 [idiom] d2b-provider-display-wayland-p1 packages/d2b-provider-display-wayland/src/wayland_proxy/dmabuf.rs:820
d2b-provider-guest-azure-virtual-machine#1 [idiom] d2b-provider-guest-azure-virtual-machine src/bootstrap.rs:138
d2b-provider-guest-cloud-hypervisor#13 [idiom] d2b-provider-guest-cloud-hypervisor guest_local.rs:113-121
d2b-provider-notification-desktop#1 [idiom] d2b-provider-notification-desktop packages/d2b-provider-notification-desktop/src/controller.rs:660-675
d2b-provider-supervisor#2 [idiom] d2b-provider-supervisor packages/d2b-provider-supervisor/src/broker.rs:1104-1130
d2b-provider-transport-azure-relay#1 [idiom] d2b-provider-transport-azure-relay packages/d2b-provider-transport-azure-relay/src/credential_client.rs:236-239
d2b-provider-zone-link#1 [idiom] d2b-provider-zone-link packages/d2b-provider-zone-link/src/zone_links.rs:60
d2b-resource-compiler#1 [idiom] d2b-resource-compiler packages/d2b-resource-compiler/src/lib.rs:2401
d2b-resource-runtime-p1#2 [idiom] d2b-resource-runtime-p1 packages/d2b-resource-runtime/src/manager.rs:1419
d2b-session-p2#2 [idiom] d2b-session-p2 admission.rs:1593
d2b-zone-routing#2 [idiom] d2b-zone-routing packages/d2b-zone-routing/src/service.rs:311
d2bd-p3#9 [idiom] d2bd-p3 packages/d2bd/src/composition.rs:21306-21319
d2bd-p8#3 [idiom] d2bd-p8 packages/d2bd/src/provider_registry.rs:527-536
d2bd-runtime-p4#2 [idiom] d2bd-runtime-p4 packages/d2bd-runtime/src/console_session.rs:162
xtask-p1#5 [idiom] xtask-p1 packages/xtask/src/provider_crate_policy.rs:6662
d2b-resource-api-p1#14 [macro] d2b-resource-api-p1 service.rs:2245-2267
d2b-provider-activation-nixos#4 [obs] d2b-provider-activation-nixos packages/d2b-provider-activation-nixos/src/controller.rs:569
d2b-provider-guest-azure-virtual-machine#11 [obs] d2b-provider-guest-azure-virtual-machine src/controller/mod.rs:346
d2b-provider-toolkit-p1#6 [obs] d2b-provider-toolkit-p1 packages/d2b-provider-toolkit/src/base/guest.rs:494-498
d2b-provider-transport-vsock#3 [obs] d2b-provider-transport-vsock packages/d2b-provider-transport-vsock/src/service.rs:735
d2bd-p2#8 [obs] d2bd-p2 packages/d2bd/src/composition.rs:15195
d2bd-runtime-p2#5 [obs] d2bd-runtime-p2 runtime_process.rs:450
X3-cross-crate-duplication#7 [own] X3-cross-crate-duplication packages/d2bd/src/resource_plane_v3.rs:3227
d2b-broker-p3#1 [own] d2b-broker-p3 packages/d2b-broker/src/ops/pidfd.rs:210
d2b-bus-p2#3 [own] d2b-bus-p2 packages/d2b-bus/src/session/contract.rs:1046-1056
d2b-contracts-resource-p1#2 [own] d2b-contracts-resource-p1 packages/d2b-contracts-resource/src/v3/volume_state.rs:138
d2b-contracts-zone-session-p2#3 [own] d2b-contracts-zone-session-p2 services.rs:216
d2b-p2#6 [own] d2b-p2 packages/d2b/src/doctor.rs:1062
d2b-provider#1 [own] d2b-provider packages/d2b-provider/src/agent.rs:290
d2b-provider-device-gpu#4 [own] d2b-provider-device-gpu packages/d2b-provider-device-gpu/src/controller.rs:272
d2b-provider-guest#5 [own] d2b-provider-guest packages/d2b-provider-guest/src/effects_service.rs:243
d2b-provider-guest-azure-container-apps#5 [own] d2b-provider-guest-azure-container-apps src/controller.rs:892
d2b-provider-guest-azure-virtual-machine#5 [own] d2b-provider-guest-azure-virtual-machine src/controller/mod.rs:1046
d2b-provider-notification-desktop#4 [own] d2b-provider-notification-desktop packages/d2b-provider-notification-desktop/src/lifecycle.rs:338
d2b-provider-provider#2 [own] d2b-provider-provider src/driver.rs:360
d2b-provider-toolkit-p2#2 [own] d2b-provider-toolkit-p2 packages/d2b-provider-toolkit/src/server/adapter.rs:305
d2b-provider-volume#2 [own] d2b-provider-volume driver.rs:337
d2b-resource-api-p1#1 [own] d2b-resource-api-p1 adapter.rs:425
d2b-resource-compiler#7 [own] d2b-resource-compiler packages/d2b-resource-compiler/src/main.rs:1467
d2b-resource-runtime-p1#5 [own] d2b-resource-runtime-p1 packages/d2b-resource-runtime/src/metadata.rs:191
d2b-session-p2#3 [own] d2b-session-p2 engine.rs:689
d2bd-p3#4 [own] d2bd-p3 packages/d2bd/src/composition.rs:20404
d2bd-p8#5 [own] d2bd-p8 packages/d2bd/src/forward_rendezvous.rs:456
d2bd-runtime-p4#4 [own] d2bd-runtime-p4 packages/d2bd-runtime/src/daemon_audit.rs:976
xtask-p2#3 [own] xtask-p2 packages/xtask/src/gen_broker_operations.rs:844
xtask-p5#2 [own] xtask-p5 packages/xtask/src/blocking_census.rs:1270
d2b-broker-p6#13 [perf] d2b-broker-p6 src/ops/store_view_posture.rs:194-271
d2b-contracts-zone-session-p2#10 [perf] d2b-contracts-zone-session-p2 resource_bundle.rs:382
d2b-p1#5 [perf] d2b-p1 context.rs:570
d2b-provider-guest#6 [perf] d2b-provider-guest packages/d2b-provider-guest/src/driver.rs:863
d2b-provider-notification-desktop#13 [perf] d2b-provider-notification-desktop packages/d2b-provider-notification-desktop/src/host_sink.rs:265
d2b-provider-toolkit-p2#13 [perf] d2b-provider-toolkit-p2 packages/d2b-provider-toolkit/src/shared_provider.rs:944
d2b-resource-api-p1#12 [perf] d2b-resource-api-p1 manager_backend.rs:1006
d2b-session-p2#12 [perf] d2b-session-p2 record.rs:125
d2bd-p7#8 [perf] d2bd-p7 packages/d2bd/src/process_provider_runtime.rs:333
xtask-p1#12 [perf] xtask-p1 packages/xtask/src/main.rs:431
d2b-broker-composition#7 [serde] d2b-broker-composition packages/d2b-broker-composition/src/dependency_surface.rs:308
d2b-contracts-provider-p2#8 [serde] d2b-contracts-provider-p2 packages/d2b-contracts-provider/src/v3/telemetry_frame.rs:76
d2b-core-p1#12 [serde] d2b-core-p1 packages/d2b-core/src/bundle_resolver.rs:209
d2b-process-conformance#9 [serde] d2b-process-conformance packages/d2b-process-conformance/src/terminal.rs:39
d2b-provider-display-wayland-p1#9 [serde] d2b-provider-display-wayland-p1 packages/d2b-provider-display-wayland/src/wayland_proxy/filter.rs:817
d2b-provider-supervisor#7 [serde] d2b-provider-supervisor packages/d2b-provider-supervisor/src/broker.rs:1563-1603
d2b-provider-wayland-policy#2 [serde] d2b-provider-wayland-policy packages/d2b-provider-wayland-policy/src/interaction.rs:241-243
d2bd-runtime-p2#4 [serde] d2bd-runtime-p2 wire.rs:265-353
X1-supply-chain#1 [supply] X1-supply-chain packages/d2b-session/Cargo.toml:17
X1-supply-chain#6 [supply] X1-supply-chain packages/d2b-bus/Cargo.toml:41
X1-supply-chain#11 [supply] X1-supply-chain packages/d2b-provider-device-gpu/Cargo.toml:25
X1-supply-chain#16 [supply] X1-supply-chain packages/d2b-provider-transport-azure-relay/Cargo.toml:34
d2b-provider-device-gpu#13 [supply] d2b-provider-device-gpu packages/d2b-provider-device-gpu/Cargo.toml:20
d2b-broker-composition#9 [test] d2b-broker-composition packages/d2b-broker-composition/src/seam.rs:523
d2b-contracts-control#14 [test] d2b-contracts-control public_wire.rs:167
d2b-core-controller-p2#11 [test] d2b-core-controller-p2 authority.rs:1824
d2b-host#8 [test] d2b-host packages/d2b-host/src/bin/d2b-activation-helper.rs:792
d2b-provider-audio-pipewire#12 [test] d2b-provider-audio-pipewire tests/mediator.rs:13-24
d2b-provider-config-nixos#9 [test] d2b-provider-config-nixos packages/d2b-provider-config-nixos/src/service.rs:70-90
d2b-provider-device-gpu#12 [test] d2b-provider-device-gpu packages/d2b-provider-device-gpu/src/authority.rs:168
d2b-provider-display-wayland-p2#12 [test] d2b-provider-display-wayland-p2 src/controller.rs:1481
d2b-provider-guest-cloud-hypervisor#5 [test] d2b-provider-guest-cloud-hypervisor finalize_ordering_test.rs:286
d2b-provider-supervisor#9 [test] d2b-provider-supervisor packages/d2b-provider-supervisor/src/broker.rs:2040-2043
d2b-provider-zone-link#8 [test] d2b-provider-zone-link packages/d2b-provider-zone-link/src/zone_links.rs:2519
d2b-resource-runtime-p1#13 [test] d2b-resource-runtime-p1 packages/d2b-resource-runtime/src/revision.rs:182
d2b-telemetry#5 [test] d2b-telemetry packages/d2b-telemetry/src/meter_registry.rs:176-180
xtask-p1#15 [test] xtask-p1 packages/xtask/src/gen_layer_catalogs.rs:705
X2-generated-boundary#6 [type] X2-generated-boundary packages/xtask/src/gen_broker_operations.rs:891
d2b-bus-p1#3 [type] d2b-bus-p1 packages/d2b-bus/src/router.rs:218-219
d2b-contracts-broker#5 [type] d2b-contracts-broker packages/d2b-contracts-broker/src/broker_wire.rs:2805
d2b-contracts-provider-p1#6 [type] d2b-contracts-provider-p1 packages/d2b-contracts-provider/src/v3/provider.rs:1333
d2b-contracts-resource-p2#5 [type] d2b-contracts-resource-p2 packages/d2b-contracts-resource/src/v3/activation_nixos.rs:46-47
d2b-host#1 [type] d2b-host packages/d2b-host/src/nftables.rs:609
d2b-provider-audio-pipewire#1 [type] d2b-provider-audio-pipewire src/resource_type.rs:64-70
d2b-provider-clipboard-wayland-p2#9 [type] d2b-provider-clipboard-wayland-p2 packages/d2b-provider-clipboard-wayland/src/picker.rs:247
d2b-provider-guest#1 [type] d2b-provider-guest packages/d2b-provider-guest/src/driver.rs:709
d2b-provider-guest-cloud-hypervisor#10 [type] d2b-provider-guest-cloud-hypervisor identity.rs:857-865
d2b-provider-notification-desktop#5 [type] d2b-provider-notification-desktop packages/d2b-provider-notification-desktop/src/controller.rs:22-27
tail-4#4 [type] d2b-provider-telemetry-binding packages/d2b-provider-telemetry-binding/src/driver.rs:168
d2b-resource-client#4 [type] d2b-resource-client packages/d2b-resource-client/src/call.rs:168
d2bd-p1#8 [type] d2bd-p1 packages/d2bd/src/resource_runtime.rs:1014
d2bd-runtime-p1#3 [type] d2bd-runtime-p1 component_session_vsock.rs:32-36
d2bd-runtime-p4#7 [type] d2bd-runtime-p4 packages/d2bd-runtime/src/typed_shell_targets.rs:13
tail-1#1 [unsafe] d2b-broker-fixture-syscall-surface packages/d2b-broker-fixture-syscall-surface/src/lib.rs:25-32
```

## check 6: count reconciliation - pass (after consolidator fix; initial run failed)

Initial run (before the consolidator's fix):

```
raw lane rows: 1022  consolidated rows: 965  merges: 57
965 + 57 == 1022: True
consolidated by sev: {'medium': 295, 'low': 657, 'high': 13}
consolidated by verdict: {'actionable': 923, 'needs-contract': 25, 'policy-confirmed': 17}
exec summary by sev: {'high': 13, 'medium': 295, 'low': 657}
exec summary by verdict: {'actionable': 923, 'needs-contract': 25, 'policy-confirmed': 17}
exec summary lens table: matches consolidated by_lens for all 16 lenses (idiom 124, own 115,
  type 82, api 138, err 94, serde 40, obs 31, docs 143, perf 52, conc 29, async 19,
  unsafe 4, ffi 0, macro 4, test 67, supply 23)
section 2 rows per lens: total 947; section 4 rows per lane: total 33
findings with a full row in section 2 or 4: 961 of 965
rows displayed in BOTH section 2 and section 4 (X1 overlap): 19
findings with NO full row in sections 2/4: ['RS-0929', 'RS-0930', 'RS-0931', 'RS-0932']
check 6: FAIL
```

The three headline numbers reconciled exactly (1022 raw = 965 report + 57
merges), and the executive summary's severity split (13 high / 295 medium /
657 low), verdict split (923 actionable / 25 needs-contract / 17
policy-confirmed), and all 16 per-lens totals equaled the recomputed
consolidated counts. The initial failure was the per-lens section display:
section 2's `supply` section showed 19 rows while the executive summary counts
23 supply findings. Four crate-lane supply findings - `RS-0929`
(`d2b-provider-audio-pipewire#13`), `RS-0930` (`d2b-provider-device-gpu#13`),
`RS-0931` (`d2b-provider-guest-azure-container-apps#13`), `RS-0932`
(`tail-3#4`) - had no full row (what/fix/anchors) anywhere in README sections
2 or 4; they appeared only in the section 3 per-crate index and the section 6
remediation clusters. The other 14 rows absent from section 2 were the
cross-cutting findings (RS-0952..RS-0965), which section 4 displays by design;
the 19 X1 rows were displayed in both section 2's supply section and section 4
(a double display, not a loss). Root cause: the render path restricted the
supply section to X1-supply-chain rows and omitted X2/X3 rows from their lens
sections, dropping crate-lane supply rows.

Re-run after the consolidator's fix (renderer corrected: section 2 now renders
every finding under its lens, grouped by crate, with cross-cutting lanes
grouped under their lane id; section 4 keeps the lane-oriented list):

```
raw lane rows: 1022  consolidated rows: 965  merges: 57
965 + 57 == 1022: True
exec summary by sev: {'high': 13, 'medium': 295, 'low': 657}   (matches consolidated)
exec summary by verdict: {'actionable': 923, 'needs-contract': 25, 'policy-confirmed': 17}  (matches)
exec summary lens table: matches consolidated by_lens for all 16 lenses (ffi 0 included)
section 2 rows per lens: idiom 124, own 115, type 82, api 138, err 94, serde 40, obs 31,
  docs 143, perf 52, conc 29, async 19, unsafe 4, ffi 0, macro 4, test 67, supply 23
  (total 965; every per-lens count equals the executive-summary count)
section 4 rows per lane: X1 19, X2 6, X3 8 (total 33)
findings with a full row in section 2 or 4: 965 of 965
rows displayed in BOTH section 2 and section 4 (cross-lane overlap): 33
findings with NO full row in sections 2/4: []
check 6: PASS
```

`RS-0929`, `RS-0930`, `RS-0931`, `RS-0932` are now present as full rows in the
section 2 `supply` section under their crates (`d2b-provider-audio-pipewire`,
`d2b-provider-device-gpu`, `d2b-provider-guest-azure-container-apps`,
`d2b-provider-quota`), and the X2/X3 findings appear in their lens sections.
All 965 findings now have a full row in sections 2/4, and the per-lens section
counts equal the executive-summary counts for all 16 lens labels.

## check 7: writes confined - pass

```
porcelain entries: 2
  ?? docs/audits/2026-09-24-rust-skills-audit/
  ?? docs/plans/2026-09-22-001-chore-post-plan-cleanup-wave-plan.md
outside allowed paths: []
check 7: PASS
```

`git status --porcelain` lists only the audit deliverable directory and the
pre-existing untracked plan file `docs/plans/2026-09-22-001-chore-post-plan-cleanup-wave-plan.md`
(not ours, left untouched). No tracked file is modified; `.scratch/` is
gitignored (its contents are audit tooling, allowed by the plan). No source,
policy, or gate file changed during the audit.

## check 8: README faithfulness spot-check - pass

Rows 100, 300, 500, 700, 900 of the findings corpus (`RS-0100`, `RS-0300`,
`RS-0500`, `RS-0700`, `RS-0900`) were located in the README and their fields
compared field-by-field with the corresponding lane rows (severity, crate,
what, fix, first-four anchors, verdict, lane reference):

```
RS-0100 (d2b-session-p2 d2b-session-p2#1): OK
RS-0300 (d2bd-p1 d2bd-p1#7): OK
RS-0500 (d2b-provider-process d2b-provider-process#3): OK
RS-0700 (d2b-provider-process d2b-provider-process#4): OK
RS-0900 (d2b-provider-guest-azure-container-apps d2b-provider-guest-azure-container-apps#12): OK
check 8: PASS
```

## check 9: data quality - parenthesis artifacts (informational)

Metric (as measured by the consolidator): share of lines with unbalanced
parentheses after stripping regex-escaped parens, over lines containing parens
(after the same strip). Recomputation per lane file:

```
unbalanced-line numerators match consolidator's list for all 9 files: True
denominator differences (parent, mine): d2b-provider-guest-qemu-media (49, 50),
  d2b-resource-compiler (49, 48), d2b-provider-guest (45, 43),
  d2b-provider-volume (41, 37), d2b-provider-credential-managed-identity (36, 37),
  d2b-contracts (39, 37), d2b-broker-p6 (52, 51); the other two files match exactly
files >25% artifact share (my recomputation): d2b-broker-p6 (13/51), d2b-contracts
  (10/37), d2b-provider-credential-managed-identity (11/37), d2b-provider-guest
  (20/43), d2b-provider-guest-qemu-media (40/50), d2b-provider-shell-terminal
  (17/37), d2b-provider-volume (14/37), d2b-resource-compiler (33/48), d2bd-p5
  (12/34), xtask-p5 (9/35)
```

The unbalanced-line numerators match the consolidator's nine-file list exactly
(40/33/17/20/12/14/11/10/13). The denominators differ by at most 4 lines per
file (attributable to the line-scope definition and/or pre-repair file state);
under my recomputation `xtask-p5` (9/35 = 25.7%) and `d2b-broker-p6` (13/51 =
25.5%) sit just above the 25% threshold while the consolidator's measurement
places broker-p6 at exactly 25% (13/52). This class is prose/punctuation only -
paths, line numbers, and counts are intact - and is not a schema violation.

## Mismatches

None. The single mismatch found on the initial run (check 6: four crate-lane
supply findings - `RS-0929`, `RS-0930`, `RS-0931`, `RS-0932` - missing full
rows from README section 2's `supply` section) was reported to the
consolidator, fixed in the renderer, and re-verified: all four rows are now
present and every count reconciles (see check 6 and the re-run section below).

## Re-run after consolidator fix (checks 3, 5, 6)

Initial run: check 3 PASS, check 5 PASS, check 6 FAIL (four supply rows missing
from section 2; exact ids `RS-0929`..`RS-0932`). Re-run results after the
consolidator's renderer fix:

```
check 3: PASS   (110 files, 1022 findings, 0 schema violations)
check 5: PASS   (13 high + 201 sampled rows; 568 anchors; 0 failures)
check 6: PASS   (1022 = 965 + 57; severity 13/295/657; verdicts 923/25/17;
                 section 2 total 965; per-lens counts equal the exec summary
                 for all 16 lenses; no finding without a full row)
```

Both passes recorded: checks 3 and 5 were unaffected by the fix (no lane row,
count, or anchor changed) and passed on both runs; check 6 failed on the
initial run and passes after the fix.

## Pass 3 (final README: verbatim pipe restoration)

The consolidator made two further README-only changes after pass 2: (1) a
renderer fix - section 2/4 bullet rows previously normalized `|` to `/` in
`what`/`fix` text, corrupting closure pipes inside inline code (e.g.
`map(|resource_type| ...)` rendered as `map(/resource_type/ ...)`); bullet
rows now carry lane text verbatim, and only the section 1 summary table
escapes `|` as `\|` (21 closure snippets restored); (2) README section 7 now
carries the verification summary and the data-quality note gained the
`xtask-p5` (9/35) mention and a renderer note about verbatim pipes. No count,
matrix cell, finding id, or anchor changed.

Re-run of the full script at the final README:

```
check 1: PASS   check 2: PASS   check 3: PASS   check 4: PASS
check 5: PASS   check 6: PASS   check 7: PASS   check 8: PASS
(568 anchors checked, 0 failures; section 2 total 965; per-lens counts equal
 the exec summary for all 16 lenses; no finding without a full row)
```

Checks 3, 5, 6, 8 all pass at the final README. Additional full-corpus
faithfulness run (beyond check 8's five-row spot check): all 998 finding rows
in README sections 2 and 4 were parsed and compared field-by-field against the
consolidated findings - severity, grouping label, `what` (verbatim), `fix`
(verbatim), first-four anchors, verdict, and lane reference - with zero
mismatches, and all 965 unique RS ids appear in section 2. This confirms the
final README differs from the pass-2 state only in the verbatim row-text
restoration and prose (section 7), with no count, cell, id, or anchor change.
Both earlier passes remain valid: pass 1 (check 6 FAIL, four supply rows
missing) and pass 2 (check 6 PASS after the renderer fix) are recorded above
unchanged.

## Data-quality note

Two artifact classes were observed in the lane corpus, both confined to prose
and both verified not to affect any structured field:

1. **Numeral-spelling and punctuation artifacts** from the lane-writing path
   (e.g. `two finding(s)`, `twelve/eleven/zero/zero` coverage phrasing, a
   dropped or duplicated `)` in inline snippets). The structured fields
   (lens, severity, blast, effort, verdict, anchors, local ids) were parsed
   and verified independently of this prose: check 3 found zero schema
   violations across all 110 files, and check 8 confirmed the README rows
   reproduce the lane rows' structured fields exactly.
2. **Parenthesis imbalance** (drop/duplication, no fact loss): see check 9.
   Per-file incidence of unbalanced-paren lines (after stripping regex-escaped
   parens) is highest in `d2b-provider-guest-qemu-media` (40/50), `d2b-resource-compiler`
   (33/48), `d2b-provider-shell-terminal` (17/37), `d2b-provider-guest` (20/43),
   `d2bd-p5` (12/34), `d2b-provider-volume` (14/37), `d2b-provider-credential-managed-identity`
   (11/37), `d2b-contracts` (10/37), `d2b-broker-p6` (13/51); the consolidator's
   measured list (40/49, 33/49, 17/37, 20/45, 12/34, 14/41, 11/36, 10/39,
   13/52) agrees on every numerator. One mechanical repair was applied to
   `d2b-provider-guest-qemu-media.md` only (space-after-paren form); no other
   lane file was rewritten for this class.

The known-and-accepted deviations from the plan's lane-file contract (lane
prose artifacts; non-15-line coverage blocks in five single-crate lanes and
the tail/X lanes; tail lanes carrying one `## <crate>` section per crate) were
observed as documented and do not affect the checks above.