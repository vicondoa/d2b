# d2b-session - unit-test audit
tests: 28 · src files: 22
net: -5 tests, -92 lines

## Findings (biggest net first)
- duplicate: `cancellation_propagates_delivery_failure_after_local_cleanup` (src/admission.rs:1944) - covered by `cancellation_schedules_local_completion_before_delivery` (src/admission.rs:1926). Both pin `SessionCancellationHandle::cancel` invoking local completion exactly once; the error variant adds only `unwrap_err` + code check on the same mock path, and the failure-propagation contract is already pinned end-to-end by `unpolled_cancellation_on_real_driver_reclaims_request_for_reuse` and `failed_cancellation_delivery_on_real_driver_reclaims_request_for_reuse` (tests/admission.rs:47,106).
- duplicate: `frame_type_rejects_trailing_bytes_and_oversize_bodies` (src/client.rs:243) - covered by `frame_validation_is_exact_and_bounded` (src/server.rs:564). Both pin the same ttrpc frame contract: exact header+body length, trailing byte rejected, oversize body rejected. (client.rs does not import server.rs; the two are kept in sync only by convention.)
- trivial: `canonical_member_has_one_slash_and_two_identifiers` (src/operation.rs:337) - asserts `OperationMember::method` accepts one canonical form and rejects six malformed strings; the same identifier grammar is already enforced by the same parse path exercised in `diagnostics_are_exact_and_resolve_closed_verbs` (src/operation.rs:352). Nothing lost.
- trivial: `guest_seed_operation_is_exactly_commit_batch` (src/operation.rs:367) - a two-case boolean check of `is_guest_resource_commit_batch` (true for CommitBatch, false for Get); the true branch is already pinned by `diagnostics_are_exact_and_resolve_closed_verbs` (src/operation.rs:352) and the false branch by the `AuditService/Inspect` rejection. Nothing lost.
- trivial: `expected_handshake_failures_have_specific_closed_reasons` (src/metrics.rs:96) - six hand-picked rows of a total function; the mapping is already pinned exhaustively by `display_exposes_closed_class_and_generic_remediation` (src/error.rs:307), which covers class, remediation, and the closed reason string per code. Nothing lost.
- gap: `SessionError::new` never panics on any `SessionErrorCode` (src/error.rs, `SessionError::new`/`class`/`remediation`/`reason_for_error` total functions) - a single new code added to any of the four exhaustive matches can panic or misclassify at runtime; no test iterates `SessionErrorCode::ALL` or asserts totality. Existing tests sample only a handful of codes.
- gap: `OperationMember::method` rejects identifiers containing `?` (src/operation.rs:337) - the invalid list covers it, but the product grammar `valid_identifier` (src/operation.rs) also admits `/`-adjacent and empty segments only by the slash split; a boundary case such as a single-char identifier or a member with three segments is unpinned. (Minor; the canonical-form test already covers the main shape.)

## Keep
- `cancellation_schedules_local_completion_before_delivery` - pins `SessionCancellationHandle::cancel` runs local completion exactly once before delivery.
- `active_request_limit_backpressures_and_terminal_removal_releases_capacity` - pins `RequestRegistry` limit backpressure, terminal removal releasing capacity, and re-registration after `cancel_all`.
- `read_frame_preserves_exact_ttrpc_boundaries` - pins `read_frame` splitting a coalesced duplex stream into exact ttrpc frames.
- `writer_closes_transport_before_reporting_packet_failure` - pins writer ordering: transport close happens before the failure is reported.
- `revocation_waits_for_writer_admission_before_returning` - pins `cancel_and_wait` blocking until the admitted writer acknowledges.
- `generation_revocation_rejects_queued_control_after_admitted_request` - pins generation revocation rejecting queued control with `Cancelled`, recording the `RejectedRecord` metric, and admitting only the pre-revocation packet.
- `post_protection_error_closes_writer_before_reply` - pins `complete_after_write` closing the writer before the reply is delivered on post-protection error.
- `unbatched_writes_preserve_the_cancellation_slot` - pins unbatched writes keeping the cancellation slot reserved.
- `driver_transport_enqueues_a_logical_packet_batch_atomically` - pins batch writes queuing as one atomic logical unit.
- `queue_exhaustion_aborts_the_writer_through_priority_control` - pins full-queue backpressure aborting the writer via priority control.
- `cancelled_immediate_receiver_restores_queued_event` - pins a cancelled immediate receiver restoring the queued event.
- `event_queue_capacity_is_measured_in_bytes` - pins byte-based `EventQueue` capacity and backpressure.
- `cancelled_receives_do_not_consume_waiter_capacity` - pins cancelled waiters not consuming `EventQueue` waiter capacity.
- `named_stream_waiters_are_delivered_only_their_stream_events` - pins per-stream waiter delivery isolation.
- `closed_waiters_on_one_stream_do_not_consume_another_streams_capacity` - pins closed waiters on one stream being reaped without consuming another stream's capacity.
- `display_exposes_closed_class_and_generic_remediation` - pins `SessionError` closed class, remediation, and display string across code families.
- `diagnostics_are_exact_and_resolve_closed_verbs` - pins diagnostic operations resolving to closed verbs and non-diagnostic/rejected operations erroring.
- `frame_validation_is_exact_and_bounded` - pins server-side ttrpc frame validation (exact length, trailing byte rejected).
- `request_correlation_binds_generation_and_stream` - pins `ttrpc_request_id` binding generation and stream.
- `stream_rewrite_preserves_payload_and_changes_correlation` - pins `rewrite_ttrpc_stream_id` preserving payload while changing correlation.
- `service_cancellation_reaches_handler_and_suppresses_late_response` - pins service cancellation reaching the handler, suppressing late responses, and completing locally.
- `stream_cancellation_reaches_handler_suppresses_response_and_cleans_up` - pins stream cancellation reaching the handler, suppressing the response, and cleaning up the registry.
- `fragments_and_streams_share_one_retained_receive_budget` - pins aggregate named-stream receive budget shared across streams with backpressure and credit release.
