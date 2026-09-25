# d2b-provider-volume-local - unit-test audit
tests: 37 · src files: 22
net: -2 tests, -30 lines

Note: census counts 40; 3 of those are `#[test]` mentions inside comments in
src/diagnostics/storage_lifecycle.rs (lines 107, 121, 258), not test fns. Real
surface: 37 `#[test]` fns, no `#[tokio::test]`, no `#[ignore]`.

## Findings (biggest net first)
- duplicate: `resolve_refuses_grants_wider_than_the_group_class` (src/layout.rs:419) - covered by `grants_wider_than_the_group_class_are_refused` (src/acl.rs:179). Both pin the same security gate: a declaration whose ACL grants exceed the declared mode's group class is refused as `InvalidSpec`. `EntryRequest::resolve` delegates to `AclBinding::from_rendered` (src/layout.rs:84, src/acl.rs:92), so the resolve-level test adds only wrapper plumbing; the acl.rs keeper pins strictly more (both accessAcl AND defaultAcl variants, vs resolve's single accessAcl case). Deleting also orphans helper `volume_uid` (src/layout.rs:407, 4 lines).
- duplicate: `replacement_aware_quota_rejects_overage` (src/atomic.rs:487) - covered by `quota_soft_check_accounts_for_replaced_bytes_and_rejects_overage` (tests/volume_local.rs:25). Both call the same exported `check_soft_quota` and pin the same behavior: replacement-aware soft quota admits within limit and rejects overage with `AtomicWriteError::QuotaExceeded`. The integration test covers strictly more (3 cases incl. `(0, 0, 1, 0)` vs the unit's 2).

## Keep
- `grants_wider_than_the_group_class_are_refused` - pins access+default ACL group-class overage refusal at the binding layer.
- `tpm_state_volume_grants_fit_the_group_class` - pins the shipped TPM state Volume declaration (0770, 4 grants) still decodes.
- `marker_root_fd_supplies_marker_file_ownership` - marker owner/group derived from marker-root fd stat.
- `component_validation_rejects_traversal_names` - component names reject "", ".", "..", "/", "\\", NUL; accepts real names.
- `declared_acls_are_applied_to_the_owned_entry` - real POSIX ACL xattr bytes written, mask mirrored into mode 0770.
- `acl_application_fails_closed_on_unresolved_principals` - unresolvable principal fails instead of silent skip.
- `durable_sequence_handles_short_writes_in_order` - commit_document write/sync/replace/parent-sync call order and receipt phase.
- `parent_sync_failure_is_ambiguous_and_never_removes_the_replaced_temp` - parent sync failure → `CommitAmbiguous`, temp never removed.
- `every_virtiofs_attachment_becomes_a_stable_owned_intent` - one owned intent per virtiofs attachment, stable name, mount path.
- `reordering_attachments_never_churns_binding_names` - attachment reorder does not churn intent names.
- `the_named_view_is_part_of_the_binding_identity` - view change yields a different binding name.
- `virtio_blk_attachments_do_not_create_filesystem_bindings` - virtio-blk produces no filesystem intents.
- `readback_evidence_requires_exact_bytes_and_metadata` - exact readback matches; tampered byte → `EffectFailed`.
- `unsafe_names_and_metadata_fail_before_materialization` - ContentFile rejects traversal/absolute/unicode-slash/colon names and out-of-range modes.
- `guest_local_domain_rejects_host_and_other_guest_sources` - same-guest accepted, Host/other-guest rejected (strictly more than tests/volume_local.rs::cross_domain_volume_access_is_rejected, which lacks the acceptance branch).
- `every_code_is_unique_and_matches_the_frozen_grammar` - 29 error codes unique and grammar-conformant.
- `entry_digests_are_stable_distinct_and_redacted` - digest stable/distinct/64-hex, Debug and serde redacted.
- `the_root_handle_is_opaque_in_diagnostics` - `VolumeRootHandle` Debug redacted.
- `marker_outliving_a_missing_root_fails_closed` - provisioned marker + missing root → `PreviouslyProvisionedStateMissing`.
- `correct_owner_empty_replacement_fails_identity_check` - inode change → `RootReplaced`.
- `root_identity_diagnostics_do_not_expose_device_or_inode` - `VolumeRootIdentity` Debug redacted.
- `tampered_marker_schema_binding_fails_closed` - tampered schema version → `MarkerInvalid`.
- `hard_quota_requires_an_enforceable_filesystem` - hard quota + unenforceable → `QuotaUnenforceable`; enforceable admits (integration layout_conformance.rs:388 covers only the failure branch).
- `policy_catalog_matches_opaque_id_class_and_volume_kind` - catalog validate: match ok, kind mismatch, empty catalog not-found.
- `block_images_require_a_byte_ceiling_and_virtio_blk` - block-image with virtiofs attachment fails spec parse.
- `tmpfs_limits_render_to_kernel_options` - tmpfs quota renders to `size=`/`nr_inodes=` mount options.
- `nix_closure_uses_an_artifact_binding_instead_of_a_source_policy` - nix-closure validates with empty catalog and keeps its artifact id.
- `block_image_plan_keeps_the_declared_byte_ceiling_opaque` - BlockImagePlan preserves max_bytes/format/preallocate.
- `source_policy_catalog_debug_does_not_publish_opaque_ids` - catalog Debug redacted.
- `phase_folding_keeps_the_more_severe_phase` - `LayoutPhase::worse` folding incl. tie.
- `reports_clean_when_every_zone_carries_its_storage_row` - clean startup report, storage contract present, path count.
- `reports_missing_storage_row_for_a_zone_without_one` - missing row → degraded with `MissingStorageContract`.
- `startup_report_carries_diagnostics_without_pidfd_authority_or_raw_paths` - serialized report leaks no pidfd/raw paths.
- `storage_classifiers_fall_back_to_unclassified_for_unknown_details` - unknown reason → Unclassified; offending-id extraction.
- `bundle_resolver_unavailable_report_is_degraded_and_schema_current` - unavailable report: degraded, schema v2, single issue.

No gaps: every product error path/boundary in this crate's src has a test in src or tests/.