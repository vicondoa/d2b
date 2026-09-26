# d2b-sk-frontend - unit-test audit
tests: 13 · src files: 6
net: -0 tests, -0 lines

## Findings (biggest net first)
No duplicate or trivial tests: all 13 pin distinct byte-layout fields, parsing branches, or redaction behavior of private builders/helpers that no other test (unit, integration, or sibling crate) can reach - the tested fns are crate-private and the crate has no `tests/` dir.

- gap: `Config::from_env` fail-closed validation (config.rs:69-127) - missing/invalid env vars, `D2B_SK_GUEST_ZONE == D2B_SK_PARENT_ZONE` rejection, non-direct-child edge rejection, nonzero reconnect-generation rejection all untested; only the two leaf helpers (`digest_value`, `zone_path`) are pinned.
- gap: `UhidDevice::read_event` event dispatch (uhid.rs:175-214) - Output/GetReport/Lifecycle/Other type mapping, short-header `UnexpectedEof`, clean-EOF `None` all untested; only `parse_output_report` (via the two output tests) is pinned.
- gap: `build_get_report_reply_error` layout (uhid.rs:337-347) - reply type 10, id echo, err=EPIPE, size=0 fields untested, unlike every other builder in the file.

## Keep
- `digests_are_lower_case_hex_and_never_zero` (config.rs:197) - pins `digest_value`: 64 lower-case hex decodes to 32 bytes; upper/non-hex, short, and all-zero digests refused.
- `zone_paths_are_label_paths_most_specific_first` (config.rs:212) - pins `zone_path`: `/`-separated labels kept most-specific-first; empty and upper-case labels refused.
- `create2_event_length` (uhid.rs:357) - pins create2 event total size (4 + packed payload).
- `create2_event_type_field` (uhid.rs:366) - pins create2 type field = UHID_CREATE2 (11).
- `create2_descriptor_length_field` (uhid.rs:373) - pins rd_size field at offset 260 = HID descriptor byte length.
- `create2_identity_fields_are_aligned` (uhid.rs:381) - pins bus/vendor/product at packed-layout offsets with BUS_USB/FIDO_VENDOR_ID/FIDO_PRODUCT_ID.
- `create2_descriptor_data_matches` (uhid.rs:407) - pins descriptor bytes copied verbatim into rd_data at documented offset.
- `input2_event_length` (uhid.rs:417) - pins input2 event total size (4 + 2 + 4096).
- `input2_event_type_field` (uhid.rs:425) - pins input2 type field = UHID_INPUT2 (12).
- `input2_event_payload_preserved` (uhid.rs:433) - pins 64-byte report copied unchanged at offset 6.
- `output_report_preserves_plain_64_byte_report` (uhid.rs:444) - pins `parse_output_report` default branch: size=64 payload copied with no stripping.
- `output_report_strips_zero_report_id_prefix` (uhid.rs:460) - pins `parse_output_report` special branch: size=65 with leading zero strips the report-id prefix and shifts the report.
- `output_report_debug_redacts_ctaphid_bytes` (uhid.rs:477) - pins `UhidEvent::Output` Debug redaction (`<redacted>`, no data bytes leaked).
