refactor: async purity of the daemon control plane (U33 enforcement)

Convert the daemon control plane (d2b-broker, d2bd, d2bd-runtime, d2b-core,
d2b-resource-runtime, d2b-host, provider crates, and every crate reachable on
their execution paths) to fully async tokio code under the U33 hard ban on
blocking calls.

- Broker: async handler ABI on the abortable worker set; ops/kernel/
  live-handler subprocess, fs, and probe sites converted to tokio::process,
  tokio::fs, and tokio::time; runtime registries, audit worker, cell store,
  and trusted-context publish are tokio-sync with the documented R4
  dedicated-worker shape; per-call identity preserved on the async zbus
  surface; pipewire/wayland probes get a bounded 5s timeout.
- Daemon: composition state, rendezvous tables, resource planes, provider
  effects, and interaction/process/zone clusters converted to tokio::sync;
  `block_on_future` bridges and `spawn_blocking` eliminated (drive-sync
  bridge for genuinely synchronous boundaries); the desktop-effect admission
  cap remains the sole refusal point.
- Substrate: d2b-resource-runtime and d2bd-runtime lock tables to tokio,
  spec-store failure taxonomy split into Busy vs terminal WriterGone,
  dedicated worker pools replace spawn_blocking.
- Enforcement: workspace-root clippy disallowed-methods deny-list extended
  (parking_lot, spawn_blocking, bridge APIs, mpsc send); per-crate census
  with clippy-derived authoritative counts and a CI drift cap; async-gate
  lexical scanner covers the covered set and no longer exempts spawn_blocking
  argument bodies; every covered crate flips to deny (per-crate lint tables).
- Tests: process-global registry serialized behind one shared test guard so
  reap/spawn tests no longer race on shared state.
- CLI (d2b): the offline operator surface (doctor checks, activation staging,
  host validate, zone audit) is genuinely synchronous CLI-only code; its 24
  production blocking sites take per-fn `CLI-only path` allows (the R11
  inventory class, same treatment xtask got), 87 test-context sites take
  per-fn `cfg(test) helper` allows, and the seqpacket transport's non-blocking
  connect retry loop keeps its AsyncFd-over-non-blocking-descriptor pattern
  under a per-fn allow (no tokio seqpacket connect form exists; the CLI's
  current-thread runtime never runs daemon workers). R13: no conversion
  changed timing, ordering, or scheduling semantics - every site keeps its
  exact synchronous behavior under its per-fn allow.

The hard ban applies to production and test code; the only sanctioned
exceptions are the dedicated bounded-worker channel boundary (R4) and
documented synchronous-path boundaries, both inventoried and gated.
