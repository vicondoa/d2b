# d2b-core - unit-test audit
tests: 93 · src files: 33
net: -0 tests, -0 lines

## Findings (biggest net first)
- gap: kernel-seat refusal paths (`kernel_seat.rs` - `run()`/`admit()` return `KernelRefusal::Busy` on saturation and `Unavailable` on dead/absent workers, incl. the panicking-job → `Unavailable` reply-drop path) - zero tests anywhere in the crate (src or tests/); the seat is consumed by three provider crates, so a silent admission-semantics regression ships untested.
- gap: static-invariants validators (`static_invariants.rs` - `world_readable_field_leaks`, `path_bearing_key_violations`, `is_broad_cap_violation`, `undeclared_writable_paths`) - module doc promises "the original positive/negative cases preserved as unit tests"; no test fn exists in the file and nothing in src/tests/ exercises the validators (only xtask consumes them). Doc promise is broken.
- cross-check: `if_name_accepts_safe_linux_names` (src/host.rs:547) and `if_name_rejects_invalid_names` (src/host.rs:553) test `d2b_contracts::v3::IfName` (`IfName::new` == `parse`), whose Empty/TooLong/InvalidCharacter rejections are pinned verbatim by `linux_limit_alphabet_and_prefix_reservation_fail_closed` (packages/d2b-contracts/src/v3/ifname.rs:498) and whose acceptance path is exercised at ifname.rs:517. C2 should resolve; both lanes keep until then.

## Keep
- `build_sysctl_intents_emits_bridge_nf_triplet` - sysctl intent set carries the bridge-nf call triplet.
- `host_runtime_carries_persisted_nft_hash` - persisted `tableHashAfterApply` reaches `host_runtime().nft_applied_hash`.
- `production_bundle_does_not_load_legacy_env_nft_projection` - production load path emits no legacy env bridge/route/sysctl/projection intents.
- `qemu_media_runner_wins_exact_writable_path_owner_score` - exact-path owner scoring beats host-reconcile.
- `provider_controller_is_not_resolved_from_process_dag` - `ResolvedRunnerIntent::from_process_node` refuses ProviderController nodes (leaf guard).
- `provider_controller_intent_uses_private_bundle_metadata_exactly` - private-bundle controller intent: role, cgroup subtree, binary path, no mount/pid/net/user namespaces; negative fence on wrong owner/host/template/kind. Note: its "excluded from processes.json intents" assertion runs against an empty processes DAG, so it is vacuous - the leaf guard above is the real pin.
- `device_worker_posture_names_the_in_namespace_socket_ids` - launch_ids are user-NS-scoped (0,0) or host-principal per posture.
- `device_worker_posture_table_is_closed` - full posture table (role/binary/seccomp/ns/device-binds/umask) plus retired-template and wrong-provider refusals.
- `device_worker_posture_nvidia_video_carries_the_nvidia_nodes` - nvidia video = plain video + the three reviewed NVIDIA nodes, distinct template.
- `serving_worker_condition_agrees_with_the_broker_spelling` - core predicate agrees with the broker's resolved-intent spelling over a 5-case table.
- `device_worker_intents_use_the_declared_row_and_template` - device rows mint per-row intents; template/row-type/host fences; flush has no user NS.
- `loads_zone_native_bundle_index_without_legacy_artifacts` - v3 index loads under verify policy, resolves declared site artifact.
- `site_artifact_loads_declared_and_conventional_and_refuses_malformed` - declared sitePath loads; conventional scan removed; absent → unbound; malformed → manifest-parse-error.
- `zone_native_host_artifact_declares_nm_unmanaged_contract` - declared host artifact loads the NM-unmanaged contract and its intent row.
- `zone_native_host_artifact_absent_keeps_empty_model_fail_closed` - absent host artifact → empty model → empty-path intent fails closed.
- `zone_native_host_artifact_tamper_is_refused_by_hash_policy` - mismatched artifact hash → bundle-tampered.
- `zone_native_bundle_index_rejects_artifact_paths_outside_bundle_root` - traversal/absolute artifact refs rejected.
- `zone_native_index_supplies_sealed_runtime_topology` - sealed topology loads root + parent map.
- `host_reconcile_and_store_preflight_emit_executable_vm_start_intents` - vm-start intents: host-reconcile prepare actions; store preflight readiness-only; no executable prerequisites.
- `resolves_macvtap_intents_from_process_contract` - macvtap intent from process network contract + trusted runner fallback.
- `network_tap_intent_ref_binds_full_provenance_and_role` - tap intent id binds zone/network/attachment uids + generations + bundle gen; bridge ≠ tap ifnames.
- `v3_tap_resolution_ignores_legacy_env_and_manifest_names` - tap resolution is UID-authoritative; legacy env mutation changes nothing.
- `uplink_bridge_address_derivation_matches_gateway_host_for_30` - /30 bridge = host octet 1, route gateway = host octet 2.
- `uplink_bridge_address_derivation_refuses_malformed_cidr` - six malformed CIDR shapes → None.
- `uplink_bridge_address_derivation_refuses_gateway_outside_bridge_subnet` - /31, /32, host-octet overflow → None.
- `resolved_uplink_bridge_intent_carries_derived_address_and_lan_none` - LAN bridge addressless; uplink bridge + route ride the derived /30.
- `resource_network_intents_use_uid_derived_kernel_names` - same-named networks across zones get distinct bridge/route kernel names.
- `resolved_network_effects_bind_complete_provenance_and_markers` - bridge/route/marker intents carry provenance + ownership markers.
- `usbip_firewall_intent_targets_uplink_not_lan_bridge` - rule targets uplink ifname, scoped to net-VM source identity.
- `usbip_firewall_intent_uses_user_visible_uplink_ifname` - user-visible ifname mapping wins over derived name in rule body.
- `usbip_firewall_intent_fails_closed_without_uplink_source_validation` - unsafe/unvalidated uplink emits no usbip firewall intent.
- `host_nft_script_drops_usbip_input_without_runtime_carveout` - host nft script drops backend + proxy ports on non-loopback ingress.
- `role_device_classes_gpu_matches_p1_matrix` / `role_device_classes_audio_matches_p1_matrix` / `role_device_classes_virtiofsd_matches_p1_matrix` / `role_device_classes_qemu_media_declares_kvm_only` - four distinct per-role device matrices (GPU excludes vfio; qemu-media kvm-only, no vhost-net/tun).
- `video_runner_has_no_stock_crosvm_legacy_fallback` - video fails closed when processes.json omits the patched crosvm binary/argv.
- `cloud_hypervisor_requires_a_closed_runner_specification` - incomplete CH spec detected; closed spec accepted.
- `catalog_guest_vmm_intent_is_zone_and_descriptor_bound` - guest VMM intent from catalog: role/vm/execution ref, zone uid, template match, store-view paths.
- `catalog_same_named_guests_keep_store_views_runners_and_cgroups_distinct` - same-named guests in two zones stay distinct in store views, intents, cgroups, uids.
- `swtpm_user_namespace_propagates_to_resolved_intent` - swtpm user-NS spec + zero host caps + umask 7 through resolve_runner_node.
- `gpu_render_node_user_namespace_propagates_to_resolved_intent` - gpu-render-node user-NS spec + seccomp_policy_ref for broker pre-open.
- `audio_user_namespace_propagates_to_resolved_intent` - audio user-NS + namespaces.net propagation (Tier 2 AF_NETLINK contract).
- `zone_native_index_accepts_optional_storage_path` - storagePath optional in v3 index.
- remaining 49 tests pin: manifest_v04 version-boundary matrix (v5 obsolete / v6 compat window with default lifecycle / v7 current / v99 future), name-key mismatch, reserved-key fail-closed, shell defaultName shape, golden byte-identical compact round-trip + networking-fixture field round-trip; console_ring boundaries (empty/EOF reads, offset reads, overflow drop + larger-than-capacity, wrap-around linearity + two-segment copy, slow-client fast-forward, past-end None, zero-capacity panic, chunked accumulation); base64 RFC 4648 vectors, arbitrary-byte round-trip, malformed/padding/non-canonical rejection; host serde contracts (deny-unknown, optional qemuMedia, legacy bridge-port defaults, arp-ignore default, usbip-lock round-trip); host_w3 wire formats (IfNameMapping, kebab-case policy, deny-unknown, RouteIntent/SysctlIntent); storage_lifecycle issue-variant casing, forward-compatible report, issue-kind dedup/order; privileges w1 matrix + deny-unknown; site socket validate/round-trip + malformed refusal; storage duplicate-id rejection; sync OFD cloexec enforcement.
