### Fixed

- The toolkit harness flake in
  `a_guest_agent_enrolls_serves_and_drains_over_a_faked_vsock_transport` is
  gone: the `uhid-report` event is now queued inside `serve()` after serving
  the guest frame, so the wire order (relay, served, uhid-report) is
  deterministic instead of racing the allocator frame in `serve_enrolled`'s
  `tokio::select!`. The intermittent "guest lifecycle completes" assertion
  failure in the CI `rust-main` lane no longer reproduces.
- The host-integration guest-store disk images are now reproducible: the
  `acceptance-guest-store.img` runCommand pins `SOURCE_DATE_EPOCH` plus a
  fixed filesystem UUID and htree `hash_seed` (e2fsprogs rejects all-zero
  seeds), replacing the randomized mkfs seed that made every build differ.
  With a stable image content hash, the nixos-install closure import matches
  the recorded spec and the `state-posture-contract` and
  `runtime-cloud-hypervisor-guest-preflight` vmChecks pass reliably.
