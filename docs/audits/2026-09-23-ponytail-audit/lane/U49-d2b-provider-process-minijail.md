# U49 d2b-provider-process-minijail

net: -6 lines, -2 deps   (Lean srce: 499-line lib + 44-line launch + 20-line
adoption; every public item has a live caller - d2bd composition at
`packages/d2bd/src/process_provider_runtime.rs:925` builds the provider
through `with_platform_gate`, and the three conformance tests pin
PlatformGate admission, execution-parent neutrality, and stale adoption.)

- deps drop `d2b-contracts-zone-session` - declared in Cargo.toml (zone-session) and in BUILD.bazel lib/conformance/execution_parents/platform_gate deps rows, but the crate never imports the zone plane (zero `zone_session`/`ZoneSession` matches across `src/` + all three `tests/` files - caller verified crate-local). The zone-session surface lives only in the sibling systemd family crate; remove the dep line from Cargo.toml and the zone-session dep rows from BUILD.bazel (5 rows). [packages/d2b-provider-process-minijail/Cargo.toml + BUILD.bazel] (leaf|family)
- deps drop `serde = { workspace = true }` - `MinijailProcessProvider` derives only `Debug`; no `Serialize`/`Deserialize`/`#[serde` anywhere in src or tests (crate-local reference search: zero matches). serde reaches this crate only through `all_crate_deps(cargo_only=True)` in BUILD.bazel, so the single Cargo.toml line is the only row to delete; bazel drops it automatically via the cargo_only arm. [packages/d2b-provider-process-minijail/Cargo.toml] (leaf)
- shrink crate-local `launch.rs` duplicate-names - `launch.rs` re-hardcodes `"Provider/system-minijail"` and `"system-minijail"` even though the crate root already exports `PROVIDER_NAME`/`PROVIDER_REF` (`src/lib.rs:44-47`). Precedent in this family: SERVING_WORKER_TEMPLATE/PROVIDER_REF are taken from the declaring crates, not restated (ADR 0046 section 3). Replace the two literals in `validate_launch_ticket` with the root consts; no behavior change. [packages/d2b-provider-process-minijail/src/launch.rs] (leaf)

## Consistency notes

Process provider family (d2b-provider-process-minijail + d2b-provider-process-systemd): both crates declare `d2b-contracts-zone-session` in Cargo.toml + BUILD.bazel, but only the systemd crate actually consumes the zone plane (its `component_session`/`Guest` wire surface). Either both crates drop it, or the minijail family keeps a documented exemption - do not leave one-sided wire in a sibling that the family policy scans. Canonical home for the wire strings: `Provider/system-minijail` is pinned by the golden test `declared_process_templates_require_the_system_minijail_provider` at `resource_bundle.rs` in d2b-contracts-zone-session (frozen wire, not movable knowledge).

## Checked

Read every file: `src/lib.rs` (full 499), `src/launch.rs` (44), `src/adoption.rs` (20), `tests/conformance.rs`, `tests/execution_parents.rs`, `tests/platform_gate.rs`, `integration/README.md`, `BUILD.bazel`, `Cargo.toml`. Workspace-wide caller search: `with_platform_gate` → live in `d2bd/src/process_provider_runtime.rs:925`; `PROVIDER_NAME`/`PROVIDER_REF` → live in d2bd + tests; `PlatformGate::from_observed` → live in `with_platform_gate` and `tests/platform_gate.rs`. Crate-local reference searches for `serde` and `zone_session` returned zero matches across src + tests. The two applied ledger rows (#PR5 EffectPortAdapter already deleted, #PR12 shims/sandbox-compiler/manifest already deleted -405) both verified gone; no new evidence reopens them. `adoption.rs` `is_stale_candidate` is called from `MinijailProcessProvider::adopt` (live), and `launch::PlatformGate` is read by `readiness_phase` - both kept. No finding touches a wire string, serde wire, or the frozen Provider/system-minijail reference.
## U1 execution (2026-09-24)

All 3 findings applied (finding 3 partial - one literal already const'd at HEAD).

- Findings 1+2: dropped `d2b-contracts-zone-session` (Cargo.toml + all 5 BUILD.bazel deps rows) and `serde = { workspace = true }` (Cargo.toml; bazel auto-drops via cargo_only arm. R4 re-verified: zero `zone_session`/`ZoneSession`/`serde` code matches across src/ + tests/.
- Finding 3: replaced `ticket.provider_ref()... != "Provider/system-minijail"` in `validate_launch_ticket` (launch.rs:51) with `crate::PROVIDER_REF`. The bare name half of the claim is stale at HEAD - `launch.rs:50` already uses the crate-root `PROVIDER_NAME` const, so only one hardcoded literal remained. (lib.rs:160 carries the same `"Provider/system-minijail"` literal in an assignment check - not lane-named, out of scope, left.)

`cargo test -p d2b-provider-process-minijail`: PASS (22 tests + doc-tests; 0 failures).
