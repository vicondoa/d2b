# d2b-resource-compiler — unit-test audit
tests: 17 · src files:  ́3
net: -0 tests, -0 lines

## Findings (biggest net first)
Nothing to cut. Ship. — 17 tests checked:  each pins a distinct schema-integrity/ref-validation edge, generated-filename mapping, digest/decoder/diagnostic invariant, or Linux anchored-I/O security boundary; no two tests pin the same behavior, and no unit test duplicates the integration/contract surfaces in `tests/cli.rs` (CLI envelope/ordering/required-field rejection) or `tests/phase2.rs` (projection/worker-row binding).
- gap: `decode_ed25519_spki` rejection paths — bad PEM framing, non-ED25519 algorithm OID, or truncated key returning `None` (src/lib.rs:2311) — no test in src or tests/ pins the decoder's negative paths; only the happy ED25519 shape is tested.

- gap: `LinuxAnchoredDir::open_readable` missing-file → `LayoutError::Absent` and non-regular directory target → `LayoutError::NotRegular` (src/linux.rs:333-340, `layout_error`/`ensure_regular`) — linux.rs unit tests cover only symlink/escape/not-executable paths; no test in the crate pins thief Absent/NotRegular mappings.



## Keep
- `executable_set_digest_is_order_independent` — pins SHA-256 digest of the executable-name map is insertion-order-independent (deterministic artifact digest).
- `diagnostic_is_ascii_and_bounded` — pins `Diagnostic` message is ASCII, ≤ `MAX_DIAGNOSTIC_BYTES`, and newline-free.

- `spki_decoder_accepts_ed25519_shape` — pins `decode_ed25519_spki` parses an ED25519 SPKI PEM into the 32-byte raw key​​.

- `anchored_read_rejects_symlink_and_escape` — pins `open_readable` refuses symlinks (same-root and outward) and `..` escapes (`SymlinkRefused`/`NotBeneath`​.
- `anchored_read_checks_regular_file_and_execute_mode` — pins `open_readable` requires regular file with execute bit (`NotExecutable`), then streams content and reports mode​​.
- `anchored_entries_are_relative_to_the_open_directory` — pins `entries` returns bare relative names of the opened directory​​.


- `malformed_committed_pattern_is_schema_integrity_failure` — pins invalid JSON-Schema regex pattern → `resource-compiler-schema-integrity-failure`​​.
- `all_of_numeric_bounds_and_schema_valued_additional_properties_are_enforced` — pins `allOf` conjunction, integer min/max, and schema-valued `additionalProperties` enforcement together in one document​​.
- `every_committed_v3_schema_passes_integrity_validation` — pins every committed `docs/reference/schemas/v3` schema document passes `validate_schema_document` (≥40 files)​​.
- `qualified_resource_refs_are_local_when_schema_metadata_says_same_zone` — pins same-zone `ResourceRef` resolves against the closed identity set: known name passes, missing identity fails, cross-type qualified ref fails​​.
- `qualified_resource_types_use_generated_envelope_filenames` — pins `resource_schema_filename` mapping (dot→underscore envelope filename)​​.
- `malformed_schema_keyword_shapes_and_ref_cycles_fail_integrity_closed` — pins non-array `allOf`/`required`/`definitions`/`enum`, non-string `$ref`, invalid `x-d2b-reference-kind`, and `$ref` cycles all fail integrity-closed​​.
- `integer_bounds_keep_exact_json_integer_semantics` — pins integer `maximum` comparison preserves u64 precision beyond 2⁵³ (no f64 rounding)​​.
- `populated_ref_forms_compile_against_the_closed_identity_set` — pins all seven core/qualified ref forms (`Provider//Guest//User//Endpoint//Role//ZoneLink//AudioService…`)validate against a closed identity set​​.
- `schema_document_depth_budget_runs_before_value_validation` — pins 130-level schema nesting fails depth budget as integrity-failure before any value validation​​.
- `populated_core_and_qualified_resources_validate_together` — pins `validate_resources` accepts a mixed Provider+AudioBinding+AudioService doc set against committed schemas​​.
- `display_wayland_resources_validate_against_committed_schemas` — pins `validate_resources` accepts a full Guest/Host/User/WaylandPolicy/WaylandSession stack against committed schemas​​.