# d2b-provider-display-wayland — unit-test audit
tests: 148 · src files: 19
net: -9 tests, -164 lines

## Findings (biggest net first)
- duplicate: `rate_limiter_suppresses_after_max` (wayland_proxy/diag.rs:222) — covered by `bind_denied_rate_limits_by_interface` (wayland_proxy/diag.rs:282). Both pin "6th same-key event suppressed after MAX_PER_WINDOW pass"; the keeper additionally pins the production `bind_denied` entry point and per-interface independence. The emit/custom-event path has no production consumer of its bool return.
- trivial: `create_dimensions_are_queued_for_multiple_async_creates` (wayland_proxy/dmabuf.rs:1040), `failed_create_drops_oldest_pending_dimensions` (wayland_proxy/dmabuf.rs:1074), `invalid_create_dimensions_still_reserve_queue_slot` (wayland_proxy/dmabuf.rs:1102) — three near-identical tests push/pop `pending_create_dimensions` (a `VecDeque`) directly; they assert std-library FIFO semantics, no product code is invoked. Nothing lost.
- trivial: `filtered_globals_preserve_original_global_names` (wayland_proxy/filter.rs:3213) — inserts entries into `advertised_globals`/`hidden_globals` maps and reads them back; pure field echo, no handler method is exercised. Nothing lost.
- trivial: `standard_clipboard_global_is_advertised_as_synthetic` (wayland_proxy/filter.rs:3264) — constructs an `AdvertisedGlobal`, inserts it, and asserts the fields it just wrote; the synthetic-advertisement path itself is not exercised (comment admits it needs a real wl client). Nothing lost.
- duplicate: `failed_handoff_closes_local_fd_copy` (wayland_proxy/bridge.rs:519) — covered by `successful_handoff_closes_local_fd_copy` (wayland_proxy/bridge.rs:514). `close_after_handoff` is `drop(self.fd.take()); status` — status-independent; both pin the same close+passthrough behavior.
- duplicate: `backpressured_handoff_closes_local_fd_copy` (wayland_proxy/bridge.rs:524) — covered by `successful_handoff_closes_local_fd_copy` (wayland_proxy/bridge.rs:514). Same as above; status value is irrelevant to the implementation.
- duplicate: `identity_target_is_not_overridden_by_app_id_metadata` (wayland_proxy/policy.rs:592) — covered by `app_id_plain_value_gets_prefix` (wayland_proxy/policy.rs:865). Identical setup (`FilterPolicy::build` over `work.local.d2b`/LocalVm) and identical assertion: plain app id rewritten with the identity prefix. Nothing in the body exercises app-id metadata.
- gap: StopRequest::Active immediate-termination branch (`DisplayController::finalize`, controller.rs:1308-1318) — no test uses `StopRequest::Active`; all four finalizer tests use `Requested`. It is a distinct transition (force-terminate without cleanup gating) in the safest part of the lifecycle.
- gap: volume-not-deleted branch requesting `delete_runtime_volume` (controller.rs:1327-1335) — no test reaches it; `finalizer_tracks_frontend_deletion_independently` stops at the not-all-terminal branch with `VolumeState::Present`.
- gap: `enqueue_bridge_handoff` queue-full drop path at `MAX_PENDING_BRIDGE_HANDOFFS` (wayland_proxy/filter.rs:646-658) — the 64-capacity saturation drop (handoff-queue-full) has no test; `bridge_handoff_is_queued_when_bridge_unavailable` covers only the empty-queue case.

## Keep
- wayland_proxy/policy.rs — 28 tests: default deny/allow tables (required, high-risk, clipboard-boundary, text-input, unknown, dmabuf, eglstream), warning generation per override class + stable code ordering, app-id/title prefix rewrite (plain, already-prefixed, cross-identity spoof, empty-prefix, sanitize, truncate), version caps, unknown lookup. Keeper of the app-id duplicate family.
- wayland_proxy/filter.rs — 21 tests: rail pointer translation + focus motion, clipboard source MIME cap + dead-ref scrub, bridge backoff/queue/flush/requeue/failure, error detail, registry global handling (synthetic clipboard name allocation + collision avoidance, prepare/remove/hide decisions incl. boundary + text-input), eglstream caps, bind-version cap.
- wayland_proxy/decoration.rs — 20 tests: color parse, shm-pool/buffer destroy ordering std, wrapper geometry + rail pixels, geometry expansion + overflow, label sanitization, buffer dimension scaling, viewport priority/destroy/unset, committed-size lifecycle, xdg fullscreen parse.
- wayland_proxy/dmabuf.rs — 11 tests: filter allow/deny semantics, create/immed decision + sink emission, feedback table remap + malformed, filter-name parse, bounded denial examples.
- wayland_proxy/bridge.rs — 9 tests: path layout + stable hash, explicit path/disabled config, reconnect state machine, sendmsg status mapping, SCM_RIGHTS handoff, frame encoding. Keeper of the close-family.
- wayland_proxy/diag.rs — 8 tests: per-label buckets, flush, error-detail truncate/scrub/passthrough, bind-denied limiting, bucket cardinality, label bounds. Keeper of the suppression family.
- `bin/d2b-wayland-proxy.rs` — 16 tests: CLI border defaults/flags, identity/listen/connect resolution, retired-flag rejection, first-client deadline per identity, accept-error recovery classification, accept backoff, error source chain, no-state-destructor source guard.
- process.rs — 6 tests: per-role restart budget, backoff/window evidence, launch-ticket generation fence, role-ticket consumption.
- controller.rs — 6 tests: policy snapshot consumption + generation-change relaunch, four finalize decision branches.
- runtime.rs — 3 tests: effect port + ordered cleanup, grant fence, repeated reconcile while starting.
- clipboard.rs (4), identity.rs (3), readiness.rs (2), session_children.rs (1), policy.rs (1) — pin disposition/mime-policy, identity canonicalization + collision-safe bridge component, typed readiness events, durable child intents, unknown max-version-key compile error.

No `#[ignore]`d tests; no rstest/test_case flavor attrs anywhere.
