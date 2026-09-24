# U9 d2b-resource-types
net: 0 lines, 0 deps   (Lean already. Ship.)

Checked: the whole crate's public surface for zero-caller items and for
hand-rolled copies of shared vocabulary. Every public item is multiply
consumed workspace-wide (search: `grep -rlw "<sym>" --include="*.rs" .`
minus bazel-out, one symbol per run - not the crate's own tests, which are
excluded by also dropping the crate's `src/` and the count gated on files
strictly outside `packages/d2b-resource-types/`):

- `WellKnownType` - 74+ reader files; `Cardinality` - 13; `IsolationPosture`
  - 19; `ChildCustody` - 19; `ChildCreation` - 16; `KernelCaller` - 11+
  (d2bd composition.rs, forward_rendezvous.rs, d2b-provider-toolkit seam);
  `RunnerLookup` - 3; `AllowedSources` - 44. No public item has zero
  readers outside this crate.

- No duplicate/enum-shadow declarations inside the crate. `Cardinality`
  (`provider.rs:39`) and `IsolationPosture` (`provider.rs:48`) are the
  *only* `pub enum` declarations of those names in the entire workspace
  (search: `grep -rn "pub enum Cardinality\|pub enum IsolationPosture"
  --include="*.rs" packages/`, single decl each). The `d2b_provider_toolkit::
  {Cardinality, IsolationPosture}` names used across `d2bd` are re-exports
  of these decls (`packages/d2b-provider-toolkit/src/declaration/mod.rs:12`
  imports from `d2b_resource_types`), so the plane's admission mask and the
  toolkit's declaration builder both resolve to this crate's single copy -
  no drift risk, no second home.

- `CONVERTED_TYPE_VERBS` (descriptor.rs) - one declaration; every reader
  (host, user, device, provider drivers) imports it rather than re-declaring
  a 9-verb list (search: `grep -rn "CONVERTED_TYPE_VERBS" --include="*.rs"
  packages/`, only descriptor.rs declares, all others import).

- Prior-applied surface stays settled: `WellKnownType::ALL` is projected at
  const-eval from `V3_CONVERTED_RESOURCE_TYPES` (resource_type.rs), the
  #A9 [applied] ledger item - verified present, not re-flagged.

- `src/generated/`: this crate has none; no exclusion needed.

## Consistency notes
Divergence feed for U97 (types-layer consistency). The vocabulary this crate
does NOT own but where the shared *name* reappears with non-identical variant
sets - a naming-drift / wire-shape-skew family, none byte-identical to the
canonical declaration:

- Canonical home for provider isolation posture: `packages/d2b-resource-types/
  src/provider.rs:48` (`IsolationPosture { Standard, UnsafeLocal }`), re-exported
  through `d2b-provider-toolkit`.
- `packages/d2b-contracts/src/workload.rs:27` - `IsolationPosture {
  VirtualMachine, ProviderManaged, UnsafeLocal }` (workload-runtime surface;
  distinct third variant `ProviderManaged` not present in canonical).
- `packages/d2b-provider-shell-terminal/src/host_rules.rs:7` - `IsolationPosture
  { Isolated, None }` (host-rule admission surface; distinct variants).
- `packages/d2b-contracts-resource/src/v3/host.rs:33` - `IsolationPosture {
  NoIsolation }` (v3 Host adapter surface; single no-isolation variant).

These are four different postures under one name; each lives in a crate this
one does not depend on, so the shape skew routes to U97 (shared-types
consistency consolidation), not to a per-crate deletion hereanged.

## Checked
Read all 10 source files (allowed_sources, child_creation, descriptor,
metadata, operation, provider, resource_type, service, startup, lib) and
lib.rs. Ran workspace-wide caller searches per public symbol with the
count gated to files outside this crate; ran the duplicate-declaration scan
prov [Cardinality, IsolationPosture, ChildCustody, KernelCaller, RunnerLookup]
across `packages/`; confirmed `CONVERTED_TYPE_VERBS` has a single declaration
site and all readers import it; confirmed the toolkit/session re-export seam
resolves to this crate's decls. Honored U9 ledge #A9 [applied] (const-eval
projection, fence test deleted, generated copy gone) - verified present and
settled. No prior refusals in the U9 row; nothing reopened.
