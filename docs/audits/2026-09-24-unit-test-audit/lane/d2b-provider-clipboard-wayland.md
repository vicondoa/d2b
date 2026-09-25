# d2b-provider-clipboard-wayland — unit-test audit
tests: 135 · src files: 22
net: -2 tests, -14 lines

## Findings (biggest net first)
- duplicate: `accepts_valid_maxish_open_request_line` (src/clipd_host/framing.rs:120) — covered by `accepts_valid_maxish_picker_line` (src/clipd_host/framing.rs:111). Both pin the same `bounded_line` boundary behavior — a line of exactly `max_frame_bytes` parses; the open-request test only swaps which cap constant feeds `bounded_line` (`OpenRequestFrameCaps::default().max_frame_bytes()` vs `PICKER_TO_DAEMON_MAX_FRAME_BYTES`) and asserts nothing about the cap computation, so nothing extra is pinned.
- trivial: `published_selection_echo_is_always_suppressed_once` (src/bin/d2b-clipd.rs:4021) — asserts identity function `should_suppress_published_selection_echo_state(x)` returns `x` for true/false. The helper is literally `suppress_selection_echo` (a pass-through), so the test only re-proves argument passthrough; nothing is lost if deleted.
- gap: `should_suppress_published_selection_echo` wrapper itself (src/bin/d2b-clipd.rs:3776) is never directly unit-tested — the trivial test above pins only its identity sub-helper, while the real gate (window + published + bridge selection; `None` selection → false) is exercised only in the main loop. Deleting the trivial test leaves this boundary unpinned.

## Keep
- `failed_audit_delivery_retains_the_head_for_retry` — sink failure keeps head for retry. (src/audit.rs:278)
- `acknowledged_audit_delivery_releases_capacity` — successful flush drains and frees capacity. (src/audit.rs:298)
- `bounded_reader_rejects_streams_larger_than_the_policy` — `SizeExceeded` on oversized stream. (src/fd.rs:670)
- `bounded_reader_error_codes_distinguish_item_and_batch_limits` — distinct Display strings per `FdReadError` variant. (src/fd.rs:679)
- `descriptor_permits_are_released_when_verified_ownership_drops` — pool active count returns on drop. (src/fd.rs:692)
- `attachment_metadata_rejects_bidirectional_and_network_regular_fds` — ReadWrite GuestTransfer + NetworkBacked regular rejected. (src/fd.rs:702)
- `live_socket_fd_classifies_as_socket` — live socketpair → Socket. (src/fd.rs:729)
- `fd_caps_must_leave_the_reserved_margin` — cap within margin ok, over rlimit rejected. (src/fd.rs:738)
- `held_open_attachment_times_out_instead_of_retaining_the_fd_permit` — read times out, permit released. (src/fd.rs:760)
- `gc_prunes_idle_guest_rate_buckets` — GC drops idle guest rate buckets. (src/history.rs:404)
- `history_normalizes_mime_values_before_storage_and_matching` — mime normalized on store and match. (src/history.rs:413)
- `purging_a_guest_releases_its_picker_completion_keys` — purge frees claimed completion keys. (src/history.rs:431)
- `picker_normalizes_mime_metadata_before_matching_history` — PickerRequest mime list normalized. (src/picker.rs:285)
- `fd_transfer_deadline_is_bounded_and_configured` — policy deadline bounded 4..=120. (src/policy.rs:214)
- `finalization_drains_workers_revokes_dependency_and_releases_authority` — finalize drains/releases/fences. (src/runtime.rs:392)
- `dependency_status_requires_the_canonical_display_provider` — non-canonical provider degrades. (src/controller/mod.rs:360)
- clipd_host/audit 5: fail-closed per-realm quota, metrics drop counter, JSON key set, mime bounded + parameter-preserving serialization. (src/clipd_host/audit.rs:166-239)
- clipd_host/fallback 4: arm, expected-focus ignore, unexpected-focus/disappear clears, timeout/native-selection clears. (src/clipd_host/fallback.rs:148-207)
- clipd_host/framing 3: overlong rejection at cap+1, accept-at-cap, encode/decode roundtrip. (src/clipd_host/framing.rs:97-130)
- clipd_host/host 1: host selection records focused-window guess attribution. (src/clipd_host/host.rs:124)
- clipd_host/niri 11: window/event model unknown-field tolerance, cache focus+output resolution, socket line client, variant unwrap, workspaces, overlong rejection, provider refresh/failure fallbacks. (src/clipd_host/niri.rs:505-765)
- clipd_host/notifications 4: fallback-ready/failure content + emission through notifier. (src/clipd_host/notifications.rs:145-170)
- clipd_host/picker 8: env sanitization, single-active + socketpair, timeout terminate/kill timing, cancel without blocking reap, argv ipc fd, buffered-frame decode, first-newline framing. (src/clipd_host/picker.rs:444-647)
- clipd_host/policy 3: allowlist closed set, case-insensitive secret hints, low-cardinality reason labels. (src/clipd_host/policy.rs:104-122)
- clipd_host/protocol 5: endpoint component stability, hello omits token/id, unknown-field rejection, additive candidate tolerance, realm metadata. (src/clipd_host/protocol.rs:223-297)
- clipd_host/virtual_keyboard 2: keymap content, anonymous memfd. (src/clipd_host/virtual_keyboard.rs:136-148)
- clipd_host/wayland 3: pending-offer mime accumulation + allowlist, selection-cleared event, denied-mime selection emits offer=None. (src/clipd_host/wayland.rs:658-702)
- service/mod 19: paste routes/one-use receipts, guest/host capture auth + policy + echo suppression + pruning, provider-subject rejection, dependency zone/generation fencing, picker receipt validity, cancelled picker, attachment aggregate limit. (src/service/mod.rs:1386-1787)
- bin/d2b-clipd 49: helper-thread cap, VM-focused matching/echo suppression, replay waits/fails-closed, html→plain conversion, candidate ordering (live host, newer VM, current alias, upsert aggregation, injective ids), unsafe-local authority, endpoint parsing + rejection, arg parsing, control-stream JSON/overlong/timeouts, nonblocking writers, idle reapers, listener install + umask, refresh partial writes, audit/metric flush, fd materialization, bridge frame fd handling (exact-identity, attribution, fd count, overlong, partial followup, queue caps, ctrunc), accept error/backoff/diagnostics, peer uid check. (src/bin/d2b-clipd.rs:3892-4967)
