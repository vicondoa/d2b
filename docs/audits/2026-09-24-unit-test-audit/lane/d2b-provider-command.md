# d2b-provider-command - unit-test audit
tests: 4 · src files: 3
net: -0 tests, -0 lines

## Findings
- gap: over-bound argv and length ceilings (src/command.rs:199, lines 40/91) - no test pins empty or >64-slot argv (InvalidArgvSlot) or the 4096-byte exec/slot ceilings; the only exec/slot rejection tested is shape (relative path, empty, NUL, malformed braces). Feeds the same error variants, but the boundary itself is unpinned.
- Nothing to cut. Ship. Checked all 4 fns for intra-crate duplication and against tests/registration.rs (which pins driver-descriptor registration, a different surface); each test owns a complementary validation behavior.

## Keep
- `a_placeholder_slot_draws_from_a_declared_parameter` - pins the positive wiring: a `{slot}` parses to its declared parameter name, literal slots have no placeholder, params().declares() resolves a declared key.
- `an_undeclared_placeholder_is_refused` - pins CommandSpec::new rejecting a placeholder naming an undeclared parameter with exactly `UndefinedPlaceholder`.
- `partial_braces_and_foreign_roles_are_refused` - pins slot-level parse refusal of malformed brace shapes (prefix/suffix/invalid property name), role-ref type check (non-Role rejected), and CommandExec refusal of relative/empty/NUL paths.
- `the_wire_shape_round_trips_and_refuses_unknown_fields` - pins serde roundtrip through canonical_json_bytes back to an equal spec, and wire rejection of an unknown field.
