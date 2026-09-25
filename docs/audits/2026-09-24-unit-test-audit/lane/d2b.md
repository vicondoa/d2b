# d2b — unit-test audit
tests: 148 · src files: 27
net: -2 tests, -13 lines

## Findings (biggest net first)
- duplicate: `seccomp_field_parse_disabled` (src/doctor.rs:2288) — covered by `seccomp_field_parse_bpf` (src/doctor.rs:2279). Both pin `parse_proc_status_field` extracting the `Seccomp:` value from a fake /proc status string; the 0-vs-2 value is echoed input, no extra parser path or boundary.
- duplicate: `nstgid_single_parse` (src/doctor.rs:2360) — covered by `nstgid_nested_parse` (src/doctor.rs:2352). Both pin `parse_proc_status_field` extracting the `NStgid:` value; the 1-vs-2 token count is echoed input, same extraction path.
- gap: `FdStateGuard::enter` raw-mode setup failure path (src/exec_client.rs:982) — `enter_raw()?` must propagate Err without touching flags/termios; only the no-op, nonblock-failure, and drop-restore paths are tested. A regression that half-applies raw mode on the error path would pass the suite.
- gap: `CliSocket::send_frame` deadline path (src/context.rs:514) — a stalled send must surface as bounded `TimedOut` like `recv_frame`; only the receive deadline is tested (`cli_socket_reports_a_stalled_peer_as_a_bounded_deadline`).

## Keep
- activation.rs: `guest_config_names_are_bounded_before_path_construction` — guest-name validation rejects empty/`../`/typed refs before path construction. `config_approve_publishes_exact_bytes_and_consumes_staging` — approve writes exact bytes, consumes staging. `config_approve_digest_mismatch_keeps_staging_and_target` — digest mismatch refuses and preserves both files.
- complete.rs: `all_builtin_completions_are_bounded_and_shell_safe` — completion output bounded, shell-escaping strips `;rm`, no CR.
- context.rs: remaining 24 tests pin the seqpacket transport contract (one datagram per frame, oversized/ancillary rejection, stalled-peer deadline, closed-peer errno), resource-ref default types, deadline caps (900s/10s), UTF-8 byte ceiling, spec-size bound, injected-context envelope/zone/attach-verb wiring, and the CliAttachStream family (idempotent close, cancel-on-drop, partial-write retry, EOF delivery, resize-shape safety, stalled round-trip deadline, redaction, call-policy idempotency, root-listener selection, no-isolation summary).
- debug.rs: remaining 10 tests pin compose/render behaviors: failure-path expansion, settled-subtree collapse, owner-absent rooting, cycle termination, unready-naming, unreachable-zone abort, degraded-read naming, empty report, named-subtree form, composite revision marking.
- dispatch.rs: remaining 7 tests pin ModernCli parse surfaces: zone+global flags, positional debug zone routing, typed list requirement, v2-alias/realm rejection, manifest command surface, built-in registry vs projection carriers, audit frame wire shape.
- doctor.rs: remaining 40 tests pin URL parsing (port/path/default/https-reject), signoz health URL, exit-code ladder (0/1/2), summary rendering, pidfd loose parsing, kernel-module-matrix statuses, autostart statuses, storage-lifecycle statuses + redaction + remediation, otel/usbipd runners, seccomp/pre-ns/broker-reap checks, bridge name collection.
- endpoint.rs: 4 tests pin ref locality, projection field dropping, forbidden locator keys, unknown-provider redaction.
- exec.rs: `attach_rejects_non_ephemeral_resources_with_the_existing_exit_code` — non-ephemeral attach refused with exit 2 and typed message. `json_tty_attach_is_refused_before_transport_access` — json+tty refused pre-transport.
- exec_client.rs: remaining 25 tests pin the exec FSM (stdin close policy, wait polling, protocol-failure fail-closed for out-of-range/missing/abnormal terminal states, partial/zero/backpressured writes, stream separation, tty stderr merge, sigwinch/signal mapping, FdStateGuard restore paths, exit-code table incl. clamping and 70-vs-old-generation disambiguation, redaction of guest bytes and static remediation, named-stream mapping).
- guest.rs: `guest_lifecycle_requests_use_typed_methods` — lifecycle payload shape. `unsafe_local_entries_never_appear_in_guest_lists` — unsafe-local filtering.
- host_validate.rs: remaining 9 tests pin sha256 FIPS vectors, ISO8601 formatting, wave catalog sync, dry-run/apply evidence writing, missing-validator refusal (exit 78), operator signature pass-through.
- provider.rs: `registry_rejects_provider_collisions_without_dispatch_fallback` — projection collision classes. `projection_text_is_single_line_and_bounded` — sanitization bound.
- resource.rs: `list_limits_and_selectors_are_bounded` — list payload bounds.
- share.rs: `share_output_never_leaks_private_shapes` — forbidden share keys.
- shell.rs: `json_open_creates_without_opening_a_terminal_stream` — JSON open uses Create with attach=false. `watch_stops_only_for_terminal_shell_states` — terminal-state vocabulary.
- zone.rs: `topology_projection_rejects_child_local_link_fields` — forbidden topology fields.
- zone_audit.rs: remaining 7 tests pin validator break/redaction behavior, credential-shaped opaque-id rejection, route-component validation, chain-boundary range start, normalized v2 envelope acceptance, debug-surface redaction, segment-name closure.
- zone_doctor.rs: remaining 4 tests pin exit-code/otel warning, closed phase vocabulary, isolation-check presence rules, no-isolation warning shape.
- zone_support_bundle.rs: `quarantine_is_partial_and_status_contains_no_name_or_spec` — quarantine bundle partial marker and name/spec redaction.

## Cross-check (for C2)
- cross-check: doctor.rs kernel-module-matrix family (3 tests) vs tests/host_doctor_contract.rs (`host_doctor_clean_kernel_module_report_passes`, `host_doctor_missing_required_kernel_module_fails_exit_2`) — clean/required-missing statuses re-pinned end-to-end; optional→warn not seen in tests/.
- cross-check: doctor.rs autostart family (3 tests) vs tests/host_doctor_contract.rs (`host_doctor_autostart_failed_outcome_fails_exit_2`, `host_doctor_autostart_degraded_outcome_warns`) — failed/degraded re-pinned; pass case not seen.
- cross-check: doctor.rs storage-lifecycle clean + exit-code trio + summary pair vs tests/host_doctor_contract.rs and tests/cli_json_contract.rs — clean→pass and exit-code ladder re-pinned end-to-end.
- cross-check: dispatch.rs parser tests (7) vs tests/cli_contract_coverage.rs — top-level registry, global flags, parser probes, and retired-namespace rejection re-pinned against the published v3 reference.
- cross-check: zone_doctor.rs `all_ready_is_zero_and_otel_absence_is_a_warning` vs tests/zone_doctor_contract.rs — zone_phase Ready, otel-sink-reachable warn, no broker_ready re-pinned end-to-end.