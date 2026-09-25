# d2b-provider-device-gpu — unit-test audit
tests: 17 · src files: 13
net: -4 tests, -36 lines

## Findings (biggest net first)
- duplicate: `daemon_input_pins_cross_domain_and_wayland_bind` (src/gpu_argv.rs:269) — covered by `daemon_input_snapshot_line` (src/gpu_argv.rs:257). Both render the same `daemon_input()` fixture; the snapshot pins the full argv line byte-exact against `tests/golden/runner-shape/gpu-argv-minimal.txt` (whose header documents these very flags), so the four substring `contains` assertions pin a strict subset of the same line.
- duplicate: `context_type_string_round_trip` (src/gpu_argv.rs:397) — covered by `daemon_input_snapshot_line` (src/gpu_argv.rs:257). The golden's `--params {"context-types":"virgl:virgl2:cross-domain",...}` renders `as_str()` for all three variants verbatim, so the mapping pairs pin nothing the snapshot doesn't already pin byte-exact.
- duplicate: `audit_parity_minimal` (src/video_argv.rs:175) — covered by `audit_parity_snapshot_line` (src/video_argv.rs:266). Same `audit_input()` fixture; the snapshot pins the full argv line plus the wire-contract line against `tests/golden/runner-shape/video-argv-minimal.txt`, and every `audit_parity_minimal` assertion (`device video-decoder`, `--socket-path ...`, `--backend vaapi`) is a substring of that golden line.
- duplicate: `backend_string_round_trip` (src/video_argv.rs:238) — covered by `audit_parity_snapshot_line` (src/video_argv.rs:266). The `Vaapi → "vaapi"` mapping is pinned verbatim by the golden's `--backend vaapi`; the single-variant round trip adds nothing.

## Keep
- `daemon_input_snapshot_line` — pins the daemon-input gpu argv line byte-exact to the runner-shape golden (cross-domain, wayland bind, params JSON).
- `audit_parity_minimal` (gpu_argv) — pins audit-input gpu argv rendering ("device gpu", socket, wayland, full params JSON); no golden covers the audit fixture.
- `rejects_one_invalid_field_at_a_time` (gpu_argv) — pins 6 typed rejection vectors (relative crosvm path, empty vm name / socket / wayland sock / context types / displays).
- `extra_args_appended_in_order` — pins extra args appended verbatim at argv tail.
- `params_renders_multi_display` — pins two-display params JSON rendering.
- `params_renders_subset_context_types` — pins single-type (virgl2) context-types rendering; distinct input from the golden fixture.
- `params_omits_egl_when_false` — pins `"egl":false` rendering.
- `context_type_string_is_json_safe` — pins charset invariant of all `as_str()` outputs (fails closed for future variants feeding the manual `format!` params JSON).
- `rejects_one_invalid_field_at_a_time` (video_argv) — pins 4 typed rejection vectors (relative/empty crosvm path, empty vm name, empty socket path).
- `rejects_unknown_extra_args_field` — pins `deny_unknown_fields` on `VideoArgvInput` deserialization.
- `audit_parity_snapshot_line` — pins video argv + wire-contract snapshot byte-exact to golden (CH-patch parity).
- `worker_observation_is_gated_on_the_declared_rows_phase` — pins Ready→Matching, non-Ready statuses→Missing, replaced-uid→StaleIdentity for `DeclaredWorkerGpuPort::row_observation`.
- `declared_worker_rows_accept_both_video_postures` — pins per-role declared-row template resolution, foreign-posture refusal, and undecodable-spec refusal.

No gaps: rejection vectors cover every `GpuArgvError`/`VideoArgvError` path; worker-observation phase gating and template resolution are pinned; lifecycle behavior is covered by the crate's integration tests (`tests/worker_contract.rs`, `tests/authority_lifecycle.rs`, etc.).