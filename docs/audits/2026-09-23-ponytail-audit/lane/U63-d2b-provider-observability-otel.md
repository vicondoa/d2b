# U63 d2b-provider-observability-otel
Lean already. Both in-lane applies already landed; the rest of the crate is the
previously-refused unwired surface, and this pass found no new evidence to
reopen any refusal. Ship.

- <tag>unwrap</tag> No new findings - the crate's entire non-`TelemetryBindingController` public
  surface (`agent.rs`, `config.rs` credential rejection, `emitter_socket.rs`,
  `ingress_policy.rs`, `metric_policy.rs`, `metrics.rs`, plus
  `TelemetryServiceController`, `TelemetryComponentSession`, `TelemetryStream*`,
  and the Binding controller's session/admission half) sits inside the
  dossier-declared realization plan (ADR-046 section 18) that #PR7 refused.
  Living wiring is exactly
  `TelemetryBindingController::child_resources` consumed by
  `packages/d2b-provider-telemetry-binding/src/driver.rs:356` (import at
  driver.rs:45); grep of every other symbol across the workspace
  (`grep -rln --include=*.rs "TelemetryServiceController\|TelemetryComponentSession\|TelemetryStreamAdmission\|EmitterSocket\|IngressPolicyGate\|validate_resource_attributes" packages/`) returns no external
  reference. Caller-verification method: `grep -rln` / `grep -rn` over
  `packages/**/*.rs`, `BUILD.bazel` deps, and `tests/` - no out-of-crate hits
  besides telemetry-binding's single `child_resources` call.
  [packages/d2b-provider-observability-otel/src/*.rs] (leaf)

net: 0 lines, 0 deps

## Consistency notes
(Otel is not a types/contracts crate; U2-U11 duplicate-type/macro-review lane
applies only to contracts crates. Not applicable here.)

## Reopened refusals
- #PR7 (unwired surface) - not reopened. Dossier ADR-046 section 18 still
  declares the realization plan; the one wired seam (`child_resources`
  through telemetry-binding) is unchanged and the non-wired surface still has
  zero workspace consumers.
- #PR15 (systemic duplications - NullRequeue/recording doubles, duplicated
  `resource_uid`, otel descriptor copy) - not reopened. Still owned
  cross-crate in d2b-resource-runtime test support; `ResourceUid::from_bytes`
  fix already landed; no new caller or deletion evidence since the refusal.

## Checked
Read all of `src/{agent,config,controller,emitter_socket,ingress_policy,metric_policy,metrics}.rs`,
`lib.rs`, `Cargo.toml`, `BUILD.bazel`, `README.md`, `nix/projection.nix`, and
both `tests/` targets. Ran workspace-wide symbol searches for every exported
name (caller class 2): the only in-tree consumer is
`packages/d2b-provider-telemetry-binding` which imports
`d2b_provider_observability_otel::TelemetryBindingController` and calls
`child_resources` (driver.rs:45,356). Cross-checked `BUILD.bazel` deps of d2bd,
telemetry-binding, and d2b-provider-telemetry-service - all reference the crate
only as a dependency, matching the wiring. The crate's `d2b_provider_observability_otel_test_support`
is consumed by telemetry-binding's BUILD (line 42) and tests. No dead private
helpers with in-crate-auditable zero callers beyond the refused surface; all
crate-internal helpers (parse_token, parse_closed_token, parse_outcome,
redact_identity, prune_expired, canonical_series_key, series accounting) have
live in-crate callers or test coverage. Ledger items honored: #PR6 (Nix fork
deletion) was already applied and remains gone (`nix/` holds only projection.nix
and tests); #PR7 and #PR15 refused and stay refused. Declared-provider caveat:
this crate's surface is the ADR-046 dossier realization plan, and per refusal
class 2 no zero-caller deletion was made. net 0.
