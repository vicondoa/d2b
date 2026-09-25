# d2b-provider-guest-qemu-media - unit-test audit
tests: 4 · src files: 12
net: -1 tests, -6 lines

## Findings (biggest net first)
- duplicate: `inverted_template_and_executable_digests_fail_closed` (src/adoption.rs:88) - covered by `process_token_matches_template_digest_not_executable_digest` (src/adoption.rs:80). Both pin that a token whose digest equals the executable digest does not match when `template_digest` differs (fail-closed); the keeper also pins the positive template-digest match, so it covers strictly more behavior.
- gap: `EmptySlot` rejection of `qemu_media_hotplug_scaffold` (src/hotplug.rs:50) - the empty-slot error variant has no test anywhere in the crate; the unit test covers only `InvalidMediaRef` and `InvalidSlot`.
- gap: `verify_identity` Quarantined path (src/adoption.rs:61; decision sites src/controller/reconcile.rs:412,615) - identity-mismatch → quarantine is a security error path with no test anywhere in the crate; unit tests cover only `matches_process_token`, and integration tests only adopt matching identities.

## Keep
- `process_token_matches_template_digest_not_executable_digest` - pins `matches_process_token` consults only `template_digest`: positive template match and executable-digest token rejected.
- `qmp_scaffold_uses_only_opaque_ref_derived_ids` - pins `d2b-media-`/`d2b-usb-` id derivation and Attach QMP command order (`blockdev-add`, `device_add`).
- `qmp_scaffold_rejects_path_like_refs_and_slots` - pins `InvalidMediaRef` for path-like refs and `InvalidSlot` for `../cdrom`.
