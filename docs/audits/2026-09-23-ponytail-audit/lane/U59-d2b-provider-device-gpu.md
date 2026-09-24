# U59 d2b-provider-device-gpu

net: -190 lines, -0 deps

- delete GPU effects service island - `GPU_EFFECTS_SERVICE` (ServiceDecl "gpu.d2bus.org/effects"), `inspect_gpu_response`, `GpuEffectsService`, `GpuEffectsServiceFactory` (incl. its write-only `facets` field under `#[allow(dead_code)]`) - the declared zone-plane service is never hosted: d2bd's effect-service factory map (shared_provider_effects.rs), the generated provider-registration table, and provider_lifecycle.rs all omit device-gpu, and no workspace file references the four symbols. The live half of effects_service.rs (`DeclaredWorkerGpuPort`, used at d2bd/src/shared_provider_effects.rs:2119,2583) stays. [packages/d2b-provider-device-gpu/src/effects_service.rs:46-125] (leaf)
- delete ten never-constructed `GpuAuthorityError` variants and their `code()` arms - only WrongPrincipal/PrincipalNotSeparated/ArbitrationViolation/StaleDeviceIdentity are ever constructed (authority.rs:109,178,185,205); ClaimConflict, MaxClaimsExceeded, StartupRehydrationRequired, DuplicateActiveReservation, ProcessPrincipalMismatch, PlatformMismatch, GenerationMismatch, OwnerProofMismatch, CloseUnconfirmed, Quarantined have zero construction and zero match sites workspace-wide (grep `GpuAuthorityError::<V>` across packages/*.rs + d2bd + d2b-provider-device). ClaimConflict's wire string survives via the live `GpuEffectError::AuthorityConflict`. [packages/d2b-provider-device-gpu/src/authority.rs:405-423,434-443] (leaf)
- delete three never-constructed `GpuEffectError` variants and their `code()` arms - OpenRejected, ProcessObservationUnavailable, Quarantined have zero construction sites workspace-wide; `ProbeUnavailable` and `WireContractMismatch` STAY (their codes `gpu-effect-unavailable` / `device-wire-contract-mismatch` are pinned by docs/specs/providers/ADR-046-provider-device-gpu.md:1086,1146-1147). [packages/d2b-provider-device-gpu/src/effects.rs:88,96,106,118,122,127] (leaf)
- delete `GpuProcessObservation::Ambiguous` - never constructed: the only production implementor (`DeclaredWorkerGpuPort::row_observation`) returns Matching/Missing/StaleIdentity only; the variant, its Debug arm, and the controller's quarantine match arm (controller.rs:396-399) are unreachable. [packages/d2b-provider-device-gpu/src/authority.rs:379,388 + src/controller.rs:396-399] (leaf)
- delete `GpuPhase::GpuStarting`, `VideoStarting`, `Degraded` - never constructed (controller sets Pending/GpuReady/Ready/Failed/Finalizing/Finalized/Quarantined only) and never matched; d2bd never reads GpuPhase (grep `GpuPhase` in d2bd = 0). [packages/d2b-provider-device-gpu/src/controller.rs:19,23,27] (leaf)
- delete five zero-caller `GpuProcessDeclaration` accessors - `name()`, `placement()`, `template()`, `seccomp_class()`, `user_namespace()` have no callers anywhere (grep `process().<m>()` workspace-wide); only `role()` is used (effects_service.rs:435, tests/worker_contract.rs). The `placement` field and Debug impl stay. [packages/d2b-provider-device-gpu/src/process.rs:41-77] (leaf)
- delete `exec_arg0` (gpu + video) and the lib aliases `gpu_exec_arg0`/`video_exec_arg0` - zero production callers (grep workspace-wide); only the crate's own unit tests exercise them; the daemon builds argv via `generate_gpu_argv`/`generate_video_argv` and never uses arg0. `vm_name` field stays (validated by `EmptyVmName` in the generators and populated by d2bd). [packages/d2b-provider-device-gpu/src/gpu_argv.rs:194-199, src/video_argv.rs:164-169, src/lib.rs:34,41] (leaf)

## Consistency notes

Not a types-layer crate (U59 is a provider crate); no consistency feed required.

## Reopened refusals

None: the U59 ledger rows (#S12-#S20) are all [applied] deletions; the two dossier-pinned `GpuEffectError` codes (`gpu-effect-unavailable`, `device-wire-contract-mismatch`) are honored and kept above.

## Checked

Read every file under packages/d2b-provider-device-gpu/ (src/*.rs, tests/*.rs, integration/READMEs, README.md, Cargo.toml, BUILD.bazel, root-config.schema.json, nix/*.nix). Caller verification: grep across packages/*.rs, BUILD.bazel, nixos-modules/, tests/, docs/reference/policy/, docs/specs/providers/ADR-046-provider-device-gpu.md, and tests/golden/runner-shape/ for every zero-caller claim - GPU effects service symbols (0 external refs; daemon factory map + generated provider-registrations + provider_lifecycle.rs omit device-gpu), GpuAuthorityError/GpuEffectError variant constructions (per-variant `Err(`/match-arm counts), GpuProcessObservation/GpuPhase constructions, GpuProcessDeclaration accessors, exec_arg0 aliases. Ledger rows #S12-#S20 honored (all applied, nothing to reopen); dossier-pinned error codes kept. LOC measured, not estimated: island 80, authority variants+arms 30, effect variants+arms 9, Ambiguous 5, phases 6, accessors 31, exec_arg0+aliases+tests 30.## U1 execution (2026-09-24)

All 7 findings applied. R4 re-verified at HEAD: all zero-caller/never-constructed claims hold (workspace-wide greps; `ProbeUnavailable`/`WireContractMismatch` codes kept, per dossier pin).

- Finding 1: deleted GPU effects service island from effects_service.rs (78 lines: `GPU_EFFECTS_SERVICE`, `inspect_gpu_response`, `GpuEffectsService`, `GpuEffectsServiceFactory` + its write-only `facets` field); module-header paragraph and now-unused imports`EffectResponse`/`EffectService`/`EffectServiceError`/`EffectServiceFactory`/`ServiceInvocation`/`ServiceDecl`/`ServiceMethod`/`crate::vocabulary::*` dropped. `DeclaredWorkerGpuPort`/`drive_sync`/`view_phase` stay.
- Finding 2: deleted ten never-constructed `GpuAuthorityError` variants + `code()` arms (ClaimConflict..Quarantined; keep WrongPrincipal/PrincipalNotSeparated/ArbitrationViolation/StaleDeviceIdentity.

- Finding   3: deleted three never-constructed `GpuEffectError` variants + `code()` arms (OpenRejected, ProcessObservationUnavailable,, Quarantined; keep `ProbeUnavailable` + `WireContractMismatch`.
- Finding   4: deleted `GpuProcessObservation::Ambiguous` variant + Debug arm (authority.rs) + controller quarantine match arm.
- Finding 5: deleted `GpuPhase::{GpuStarting,VideoStarting,Degraded}` variants (controller.rs;; zero construction/match sites verified.
- Finding 6: deleted five zero-caller `GpuProcessDeclaration` accessors (`name`,`placement`,`template`,`seccomp_class`,`user_namespace`;; `role()` + `placement` field + Debug impl stay.
- Finding 7: deleted `exec_arg0` (gpu_argv.rs + video_argv.rs) + their crate-local tests (`exec_arg0_matches_systemd_unit_name`×2, `exec_arg0_rejects_empty_vm_name`×1) + lib aliases `gpu_exec_arg0`/`video_exec_arg0`; vm_name-field docs de-linked,`EmptyVmName` stays (generators still emit it.

`cargo test -p d2b-provider-device-gpu`: PASS (17 unit + 4+4+5+1+1 integration; doc-tests; 0 failures..
