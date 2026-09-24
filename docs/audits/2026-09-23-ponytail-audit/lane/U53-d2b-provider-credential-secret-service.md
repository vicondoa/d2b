# U53 d2b-provider-credential-secret-service
net: -50 lines, -0 deps

- shrink deadline/absolute-unix-ms helper trio hand-rolled identically across the three credential family crates - `ABSOLUTE_UNIX_MS_THRESHOLD` + `now_unix_ms` + `is_absolute_unix_ms` + `operation_deadline` + `deadline_remaining` (lib.rs:49,1501-1527,1596-1603; ~35 lines) are byte-identical to entra (lib.rs:49,1165-1212) and managed-identity (lib.rs:60,938-952,1124-1130); the PR1-shared module `d2b-provider-toolkit/src/credential.rs` owns the credential family's common machinery and already exposes the toolkit `Clock` trait + `DeterministicClock`, but has no deadline/absolute-unix-ms helper. Move the trio (threshold const + now/absolute/deadline/remaining fns) into the shared toolkit credential module and call it from all three crates, keeping the toolbox pinned `now_unix_ms` trait method. [packages/d2b-provider-credential-secret-service/src/lib.rs:49,1501-1527,1596-1603] (family)
- shrink reject_process_environment_credential_chain env-scan body duplicated across the three credential family crates - lib.rs:52-66 (~15 lines) is the same `std::env::vars_os` ambient-SDK-env scan + `reject_ambient_credential_chain` contract call as entra (lib.rs:52-65) and managed-identity (lib.rs:52-66,63-76); the shared scan already lives in `d2b-contracts-provider::v3::credential_controller::reject_ambient_credential_chain`, so the per-crate copy reduces to a one-line `reject_process_environment_credential_chain` wrapper. Move the env-scan to the toolkit credential family module (which owns the ambient-chain machinery per the dossier §4 Declared-provider family surface). [packages/d2b-provider-credential-secret-service/src/lib.rs:52-66] (family)

## Consistency notes
- `SecretServiceOwner` single-variant enum + `PROVIDER_KIND`/`BACKEND_REF`/`PROVIDER_REF`/`MAX_LOCAL_LEASES`/`PROVIDER_REVOKE_FINALIZER` - dossier-pinned provider identity surface (dossier §4, dossier ID rows); kept as the canonical home of the declared-provider constants (entra/managed-identity keep their own twin const sets per the family payout; canonical home = this dossier, mirrored by ADR-046 docs).
- `SecretServiceController`/`SecretServiceStatusProjection`/telemetry.rs/audit.rs/controller.rs - dossier-pinned controller projection surface (dossier §6, §7 rows); kept.

## Checked
README.md (106 lines) - rules pinned, zero-secret-bytes invariant, audit/telemetry closings, directive blocks §8 (Noise_KK session end-to-end token delivery: zero secrets in status/store/audit/telemetry). src/ full surface read (lib.rs 2192, service.rs 910, controller.rs 245 + controller.rs #[path] audit/telemetry, main.rs): lib.rs deadline helpers (ABSOLUTE_UNIX_MS_THRESHOLD at lib.rs:49, now_unix_ms/is_absolute_unix_ms at 49,1501-1527, operation_deadline at 1596-1603), reject_process_environment_credential_chain (52-66), placement/placement.rs, session.rs credential helper trio (now_unix_ms, is_absolute_unix_ms, operation_deadline, deadline_remaining ~35 lines), controller.rs reconcile/observe/finalize/drain handler guards pinned by dossier §6; audit.rs (69) + telemetry.rs (50) thin toolkit wrappers (shared `d2b-provider-toolkit/src/credential.rs` home `authorized_service_record`/`credential_frame`); faults.rs/canary.rs/delivery.rs/session.rs/lifecycle.rs/placement.rs + tests (session.rs lifecycle/session; delivery.rs; faults.rs; placement.rs; canary.rs; conformance.rs; faults.rs; lifecycle.rs; canary.rs; main.rs) - all exercises dossier-conformance surface. Caller/usage verification done via workspace-wide grep of `ABSOLUTE_UNIX_MS_THRESHOLD`, `now_unix_ms`, `is_absolute_unix_ms`, `operation_deadline`, `deadline_remaining`, `reject_process_environment_credential_chain`, `reject_ambient_credential_chain` across `packages/`, `nixos-modules/`, `docs/reference/policy/`, `docs/specs/providers/` - the deadline helper trio is aggregated family-wide (this crate + entra lib.rs:1165-1212 + managed-identity lib.rs:938-952,1124-1130); the only handler the trio feeds (operation_deadline → poll_port stacks) is live forwarded from `run_from_fd10` (dossier §1 provider entry). Integration trees under `integration/` are dossier-ratchet/lab-scope. No zero-caller claim is made without a workspace search; every kept item is a live external reader, utility-test subject, or dossier-pinned declared surface.

## Reopened refusals
None. U1 refusal ledger for this crate has no refused rows applicable here.

## U5 execution (2026-09-24)

- Finding 1 (deadline/absolute-unix-ms trio): applied. Trio moved verbatim into
  `d2b-provider-toolkit/src/credential.rs` (`ABSOLUTE_UNIX_MS_THRESHOLD`,
  `now_unix_ms`, `is_absolute_unix_ms`, `operation_deadline`, `deadline_remaining`);
  this crate's lib.rs helper methods deleted and every call site (service.rs,
  ensure_unlocked_async, tests) re-pointed to the toolkit module. R4 note: at HEAD
  the trio is byte-identical only between secret-service and managed-identity;
  entra's `operation_deadline` delegates to its own `time_bound_instant` (checked_sub
  semantics, no zero-duration rejection) - see U54 note.
- Finding 2 (env-scan): applied. `reject_process_environment_credential_chain`
  env-scan moved into the toolkit credential module; this crate's public wrapper is
  now a one-line mapping call (public error surface unchanged).
