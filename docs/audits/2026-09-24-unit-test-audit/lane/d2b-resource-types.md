# d2b-resource-types — unit-test audit
tests: 7 · src files: 10
net: -2 tests, -18 lines

## Findings (biggest net first)
- trivial: `a_child_creation_keeps_every_field` (src/child_creation.rs:51) — re-asserts its own `const CREATION` literal's pub fields verbatim (echo of visible source text); the one non-echo assert (PROCESS→"Process" conversion) is already pinned by `names_round_trip_through_the_runtime_name` (src/resource_type.rs:131). Nothing lost. (net:  ̃7 lines; CREATION const stays live — used by the custody test.)
- trivial: `custody_distinguishes_its_two_values` (src/child_creation.rs:61) — fieldless enum variants being distinct and struct-update syntax storing a field are both guaranteed by Rust semantics; nothing product-specific pinned. Nothing lost. (net:  ̃11 lines.)

## Keep
- `a_mask_without_runtime_requires_plane_registration` — pins every mask lacking RUNTIME (BUILTIN, STARTUP,, BUILTIN|STARTUP,, empty) → presence obligation (requires_plane_registration() true).
- `a_mask_with_runtime_does_not_require_plane_registration` — pins every mask with RUNTIME (RUNTIME, BUILTIN|RUNTIME,, all()) → exempt. Complementary half of the same predicate; both needed.

- `the_three_bits_combine_without_overlap` — pins the three bit constants pairwise disjoint, union=`all()`, bits=`0b111` — guards bit-value typos in the bitflags declaration.ro
- `every_well_known_type_is_distinct` — pins `WellKnownType::ALL` has no duplicate entries — guards a dup in the V3_CONVERTED_RESOURCE_TYPES authority list (cross-check: `d2b-contracts-resource::v3` tests for that list).
- `names_round_trip_through_the_runtime_name` — pins PROCESS and NIXOS_GEENERATION string literals survive `to_resource_type_name` conversion unchanged into the runtime's owned name (cross-check: `d2b-contracts-resource::v3` tests for the name list).