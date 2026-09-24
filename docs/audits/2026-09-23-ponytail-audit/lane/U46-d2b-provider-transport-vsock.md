# U46 d2b-provider-transport-vsock

net: -697 lines, -1 fixture/BUILD arm set

## Findings

1. **Delete `src/relay_argv.rs` (621 lines: 259 production + 362 test) plus its
   `lib.rs` re-export arm (lib.rs:39-41) and the golden fixture + BUILD arms that
   exist only to pin it.** `generate_vsock_relay_argv` / `VsockRelayArgvInput` /
   `SocatEndpoint::VsockListen/VsockConnect` / `exec_arg0` / `VsockRelayArgvError`
   have zero callers anywhere in the workspace (swept `packages/*/src`, `tests/`,
   `integration/`, `nixos-modules/`, `nix/`, benches, and all `BUILD.bazel`
   compile_data; only hits are this crate's own unit tests + this crate's own
   committed golden `tests/golden/runner-shape/vsock-relay-argv-minimal.txt`).
   The dossier's socat/socat-relay-argv rows (ADR-046 dossier rows 955-964,
   1068-1076) all name the LEGACY `packages/d2b-host/src/vsock_relay_argv.rs`
   socat path as `delete-after-cutover` superseded by the observability-otel
   Provider native vsock relay; they do not name this crate's `src/relay_argv.rs`
   as a destination. Same declaration class as the applied cross-crate relay-argv
   socat deletions (ADR 046 reuse disposition: delete-after-cutover). Delete the
   module + lib.rs re-export arm + the golden fixture +
   `packages/d2b-provider-transport-vsock/BUILD.bazel` compile_data/compile-data
   arms + root `BUILD.bazel` exports_files arm. [-625] (leaf→family via fixture) (relay_argv.rs)

2. **Delete `src/metrics.rs` (36) + `src/audit.rs` (30) with their two `lib.rs`
   re-export arms (lib.rs:20, 35).** Unproduced metric/audit vocabulary:
   `TransportMetricLabels/Operation/Outcome` and
   `TransportAuditEvent/Operation/Outcome` are never constructed or consumed
   anywhere in the workspace (zero caller sweep over `packages/*/src`, `tests/`,
   `integration/`, `benches/`, `nixos-modules/`; the only references are the
   defining modules + this crate's own lib.rs re-export arms; no dossier row
   names `src/metrics.rs`/`src/audit.rs` as a destination). Same class as the
   applied cross-crate metric/audit vocabulary deletions (supervisor, qemu-media,
   azure-vm, ACA, cloud-hypervisor — this is the sixth application of the
   established finding): unproduced metric/audit vocabulary is deleted; the
   crate's live `VsockEffectPort` metric/audit surface is untouched
   (framing/service/errors stay). [-68] (leaf) (metrics.rs, audit.rs)

3. **Delete `service.rs:155` private `const PROVIDER_REF`** — byte-identical
   duplicate of `lib.rs:54 pub const PROVIDER_REF` (same
   `"Provider/transport-vsock"` value); the two in-crate users (service.rs:444,
   986) should use `crate::PROVIDER_REF`. The dossier dossier pins the crate-root
   `PROVIDER_REF` provider-identity constant, so lib.rs's public arm stays;
   only the service.rs private duplicate goes. [-2] (leaf) (service.rs)

4. **Delete `OpaqueEndpointId::from_core` (service.rs:53) + `OpaqueBindingId::from_core`
   (service.rs:92)** — byte-identical aliases of `parse` (both `Self::parse`)
   with zero callers in or out of crate (workspace-wide sweep; only the
   definitions + the lib.rs re-export arm). [-8] (leaf) (service.rs)

## Consistency notes

None due — transport-vsock is a runtime/transport Provider crate (not a
contracts/types crate); no types-lane vocabulary feed due.

## Reopened refusals

None — this crate's U46 refusal ledger row (#S8 whole-crate declaration) is
honored: the crate, its `framing.rs`/`errors.rs`/`service.rs`/`bridge.rs`
dossier destinations and their tests stay; no caller verification contradicted.
Reopened rows: none (no dossier-named socat-relay destination in this crate —
the dossier pins relay argv only at the legacy `d2b-host` path marked
delete-after-cutover, which is a legacy-crate deletion, not an in-crate surface).

## Checked

Verified zero-caller claims workspace-wide for all four leaf deletions:
swept `generate_vsock_relay_argv`/`VsockRelayArgvInput`/`SocatEndpoint::Vsock*`,
`TransportMetric*`/`TransportAudit*` constructors, `PROVIDER_REF` duplicates, and
`from_core` across `packages/*/src`, `tests/`, `integration/`, `benches/`,
`nixos-modules/`, `nix/`, and every `BUILD.bazel` compile_data/compile-data arm;
confirmed the only hits are in-crate defs + re-export arms + the crate's own
committed golden fixture. Read lib.rs, service.rs, relay_argv.rs, metrics.rs,
audit.rs, errors.rs, framing.rs, bridge.rs, limits.rs, settings.rs, topology.rs,
auth.rs, state_volume.rs and the dossier
(`docs/specs/providers/ADR-046-provider-transport-vsock.md`); confirmed the
dossier pins only framing/errors/service/bridge as owned destinations and names
no in-crate relay argv/metrics/audit surface. LOC measured: relay_argv.rs 621
(259 prod + 362 test), metrics.rs 36, audit.rs 30, from_core x2 = 8.
