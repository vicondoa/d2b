# d2b-session-unix - unit-test audit
tests: 25 · src files: 11
net: -0 tests, -0 lines

## Findings (biggest net first)
- gap: `ZoneBootstrapIdentity::verify` uid checks - InvalidPeerUid (expected_uid==0) and PeerUidMismatch (observed vs expected) untested anywhere in the crate (zone_admission.rs:79-95; host-socket feature builds. Bootstrap admission isia security boundary; wrong-uid or zero-uid admission must be pinned.

- gap: `CreditPool::new(0)` - ZeroLimit (credit.rs:43-45) untested; zero-limit config boundary must be rejected, else a host could build an exhausted pool silently.

- gap: `ProcessCreditLimit::derive` - BaselineExceedsLimit (baseline+reserve ≥ soft rlimit; credit.rs:272-273) and Overflow (credit.rs:264-268) untested; rlimit-derived attachment budget is a hard DoS boundary.



## Keep
- adapter.rs `unix_stream_cancelled_receive_retains_partial_framing` - pins receive-side cancel retention on UnixStream framing (partial header+body retained, resumes to full payload; distinct from vsock framing and from the send-side integration test `stream_transport_resumes_a_cancelled_partial_frame`.
- adapter.rs `disconnect_errnos_have_a_closed_transport_class` - pins EPIPE/ECONNRESET/ENOTCONN → TransportError::Disconnected via map_transport_error.

- adapter.rs `transport_observer_records_closed_labels_before_returning_failure` - pins observer records (Receive, Limit) before error is returned to caller. Unique ordering/observability contract.
- adapter.rs `inherited_first_packet_requires_credentials` / `inherited_first_packet_rejects_wrong_credentials` - pins consume_peer_credentials error paths: missing creds → ControlMismatch; wrong pid → CredentialMismatch + credentials control drained (integration tests only pin the happy-path consumption).
- descriptor.rs `first_packet_credentials_require_exact_pid_uid_and_gid` - pins verify_first_packet_credentials exact-equality rejects on pid mismatch (integration `first_packet_has_exact_directional_credentials` pins first-vs-later flag, not equality. Different code path.

- socket.rs `inherited_fd_is_rearmed_before_session_use` - pins from_inherited_fd re-arms real CLOEXEC flag on an inherited fd before use.
- socket.rs `raw_control_scanner_rejects_unknown_and_partial_headers` / `raw_control_scanner_accepts_exact_rights_shape` - complementary reject/accept pins of scan_control_layout (unknown cmsg type, partial header; exact rights shape → files:2.
- systemd.rs `activation_requires_exact_single_named_descriptor` - pins validate_environment_values exact-match accept + count/name/pid mismatches → InvalidEnvironment. Unique (subprocess test pins consume_environment instead).
- systemd.rs `prepare_listener_sets_real_inherited_flags` - pins prepare_listener sets real CLOEXEC+NONBLOCK on a real listener. Distinct from the three single-condition reject tests`.
- systemd.rs `validate_listener_rejects_non_unix_domain` / `validate_listener_rejects_non_seqpacket_type` / `validate_listener_rejects_socket_without_acceptconn` - three distinct prepare_listener rejection boundaries (domain, type,, acceptconn); each pins a condition the others do not.
- systemd.rs `activation_can_only_be_claimed_once` - pins claim_once transition Ok → AlreadyConsumed on a real AtomicBool.
.
- systemd.rs `invalid_environment_closes_advertised_descriptors` - pinned subprocess:e invalid env causes advertised fd to be closed (EBADF after close); unique process-level activation teardown test.

- vsock.rs `in_memory_vsock_adapter_is_framed_and_rejects_attachments` - pins FramedVsockTransport frame roundtrip + descriptor class NativeVsock (vsock framing tested only here).
- vsock.rs `in_memory_vsock_adapter_enforces_frame_limit_and_disconnect` - pins >64-byte payload → LimitExceeded; send after close → Disconnected.

- vsock.rs `cancelled_receive_retains_partial_header_and_body` / `cancelled_partial_send_resumes_the_same_frame` - receive- and send-side cancel/resume pins for the u32-length-prefixed vsock framing (distinct implementation from adapter.rs stream-framing twin).
- vsock.rs `full_header_zero_body_eof_is_truncated` - pins EOF after full header → Truncated,, the shoulder of the stream framing EOF behavior at the vsock boundary.
.
- vsock.rs `expected_cid_accept_discards_repeated_foreign_peers` / `foreign_peer_does_not_reset_original_accept_deadline` / `cancelling_accept_closes_foreign_peer_and_pending_listener` - three distinct accept_expected invariants: foreign-peer discard/drop-count, deadline non-reset across foreign delays,, abort-time cleanup of foreign peer. All private fn, only testable here.



route-out: `ZoneAdmissionError::ZoneInvalid` variant (zone_admission.rs:44) is declared and Display-ed but never constructed anywhere - dead error variant.