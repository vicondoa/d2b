# d2b-provider-network-local - unit-test audit
tests: 50 · src files: 17
net: -2 tests, -41 lines

## Findings (biggest net first)
- duplicate: `inspect_network_report_spells_the_catalog_wire_names` (src/effects_service.rs:340) - covered by `inspect_network_answers_the_trusted_bundle_report` (src/effects_service.rs:259). Both pin the 13 committed operation wire names appearing in the inspect-network report; the keeper asserts the exact full payload, ordered operations array included, while canonical-serialization behavior is separately pinned by `inspect_network_escapes_a_structural_character_in_a_trusted_value`.
- duplicate: `foreign_marker_in_target_slot_fails_closed` (src/nftables.rs:721) - covered by `firewall_target_marker_conflict_preserves_original_bytes` (tests/network_primitives.rs:100). Both pin `apply_projection` refusing a foreign marker occupying the target Network slot with `NftablesError::ForeignMarkerPreserved` and preserving the snapshot's original bytes.
- gap: `evaluate_observation` decision state machine (src/observe.rs:545, consumed at src/controller.rs:1064) - Blocked when the external authority is not ready, the `CidrConflict` error path, and the Current-vs-Requeue transitions have no test in src or tests/.
- gap: `check_network_services` `RoutesNotApplied` branch (src/routes.rs:387) - only `DnsmasqNotBound` is pinned; a Ready+dnsmasq-bound but routes_applied=false status fails nowhere.

## Keep
- `requires_declared_nixos_system` (src/artifact.rs:97) - pins Missing and TypeMismatch error paths of `resolve_net_vm_system_artifact`.
- `readback_matches_defaults` (src/bridge_port.rs:242) - `validate_readback` accepts `defaults_for` for all four tap roles.
- `drift_fails_closed_and_lists_every_difference` (src/bridge_port.rs:254) - drift reports every difference and the stable "bridge-port-flag-drift" code (strictly stronger than the integration one-flag drift check).
- `direct_east_west_requires_both_opt_ins` (src/bridge_port.rs:264) - each of the two east-west opt-in gates fails its own error.
- `provider_tap_identity_is_uid_bound_and_refuses_swapped_attachment` (src/broker.rs:1734) - tap identity derives uid-bound bridge/tap names + intent ref; swapped attachment refused pre-effect.
- `provider_rejects_swapped_tap_context_before_effects` (src/broker.rs:1796) - `validate_tap_context` rejects five distinct swapped identity fields.
- `tap_context_requires_the_live_admitted_interface_set` (src/broker.rs:1853) - missing admitted interface name and wrong generation refused (overlaps the prior test's generation swap but adds the interface-set check).
- `broker_port_requires_site_opt_in_before_dispatch` (src/broker.rs:1939) - east-west spec refused without site opt-in, no broker events.
- `broker_port_requires_host_global_nic_admission_before_dispatch` (src/broker.rs:1950) - external-NIC spec refused without host-global admission ("external-nic-authority-required"), accepted with it.
- `broker_port_rejects_swapped_network_intent_refs` (src/broker.rs:1966) - swapped projection intent ref refused.
- `broker_port_rejects_swapped_network_name_hints` (src/broker.rs:1981) - swapped route intent-ref name hint refused (distinct field from the ref-swap tests).
- `broker_port_refuses_swapped_bridge_and_stale_bundle_before_effects` (src/broker.rs:1996) - swapped bridge ref on `create_bridges`, stale bundle generation on `apply_routes` refused.
- `broker_port_refuses_mixed_network_intent_kinds_before_effects` (src/broker.rs:2024) - route slot holding a projection-kind ref refused.
- `broker_port_refuses_swapped_sysctl_hosts_and_firewall_refs_before_effects` (src/broker.rs:2038) - swapped sysctl/hosts/firewall refs refused across three verbs.
- `broker_port_rejects_unfenced_firewall_intent` (src/broker.rs:2083) - firewall intent lacking the admission fence refused.
- `broker_port_maps_host_effects_to_typed_broker_calls` (src/broker.rs:2099) - the full effect sequence dispatches eleven typed broker calls in order.
- `kernel_refusal_reasons_keep_provider_retry_and_block_states` (src/broker.rs:2154) - five kernel refusal strings classify to their provider error classes.
- `descriptor_declares_the_network_type_and_its_children` (src/driver.rs:579) - descriptor declares Network + Volume/Guest/Process children over the family effect port.
- `reconcile_commits_the_declared_children_before_the_effect` (src/driver.rs:607) - reconcile commits the three children before the typed effect runs.
- `the_decoder_yields_the_shared_envelope` (src/driver.rs:648) - decoder yields the shared envelope with the family provider ref.
- `network_spec_strips_the_envelope_fields` (src/driver.rs:661) - typed spec drops providerRef; providerRef-only spec is refused.
- `dependency_refs_are_the_attachment_execution_targets` (src/driver.rs:670) - declared dependency refs are the attachment execution targets.
- `validate_rejects_a_foreign_provider` (src/driver.rs:691) - foreign provider is a terminal validate failure.
- `the_row_keeps_the_preserved_identity` (src/driver.rs:712) - registration row preserves provider identity and resync cadence.
- `recording_effects_records_ordered_calls` (src/driver.rs:729) - reconcile-then-finalize ordering and per-verb counters (adds finalize beyond the child-commit test).
- `inspect_network_answers_the_trusted_bundle_report` (src/effects_service.rs:259) - full report payload: installed generation, host nftables, east-west ack, ordered operations inventory.
- `inspect_network_refuses_without_an_installed_generation` (src/effects_service.rs:292) - declined with the closed "inspect-network-installed-generation-unavailable" code.
- `inspect_network_operations_match_the_committed_inventory` (src/effects_service.rs:314) - catalog's lowercase operation refs in committed order (ref spellings the report test does not pin).
- `inspect_network_escapes_a_structural_character_in_a_trusted_value` (src/effects_service.rs:370) - trusted value survives canonical round-trip escaped, never interpolated.
- `ipv6_off_sequence_runs_in_order` (src/netlink.rs:390) - create-down/write/up/read ordering, no drift.
- `defense_in_depth_reapply_repairs_drift` (src/netlink.rs:418) - reapply repairs drift without create/up and checks every setting.
- `readback_drift_fails_closed_without_identity` (src/netlink.rs:432) - readback drift fails closed as `SysctlDrift`.
- `bridge_port_readback_matches_defaults` (src/netlink.rs:441) - netlink readback of bridge-port flags validates clean against defaults.
- `network_rules_reject_usbip_and_service_port` (src/nftables.rs:735) - `NetworkRule` parse refuses usbip-relay and service-port rules, accepts ct state.
- `observed_managed_projection_validates_exact_markers` (src/nftables.rs:742) - `SharedTableEntry::managed` constructor rejects foreign marker payloads.
- `projection_digest_ignores_sibling_and_foreign_churn` (src/nftables.rs:751) - owner digest stable under sibling-rule and foreign churn (sibling-rule churn not pinned by the integration digest test).
- `projection_digest_is_sha256` (src/nftables.rs:771) - digest is real SHA-256 (empty-input vector).
- `coexistence_matrix_has_all_seven_rows` (src/nftables.rs:779) - all seven firewall-manager coexistence rows accept; mismatch is refused.
- `foreign_and_uidless_objects_are_retained_as_occupancy` (src/observe.rs:453) - unmarked interfaces/routes retained as occupancy with parsed fields.
- `a_host_observation_command_that_hangs_fails_closed_within_its_budget` (src/observe.rs:490) - `run_ip_command` times out to `Backend` within budget.
- `marked_interfaces_addresses_and_routes_retain_provenance` (src/observe.rs:493) - bridge/route/cidr ownership markers survive observation parsing.
- `no_default_route_fails_closed` (src/routes.rs:463) - missing default route fails with `NoDefaultRoute`.
- `foreign_default_route_fails_closed` (src/routes.rs:471) - foreign default route fails with `ForeignDefaultRoute`.
- `dnsmasq_not_bound_fails_closed` (src/routes.rs:479) - dnsmasq not bound fails with `DnsmasqNotBound`.
- `ipv6_address_on_owned_link_fails_closed` (src/routes.rs:490) - IPv6 on an owned link fails with `Ipv6AddressPresent`.
- `host_lan_cidr_ambiguous_for_vpn` (src/routes.rs:498) - point-to-point routes land in `ambiguous`, never in the LAN CIDR set.
- `route_provenance_rejects_swapped_network_and_stale_generation` (src/routes.rs:509) - base validator's NetworkMismatch and GenerationMismatch branches (not reached by the full-provenance test).
- `full_route_provenance_rejects_swapped_zone_and_attachment_generation` (src/routes.rs:545) - provenance wrapper's zone and attachment-generation rejects plus valid pass-through.

No product-code bugs observed while reading (`route-out:` none).