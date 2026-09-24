# U60 d2b-provider-device-security-key
Lean already. Ship.
- Checked: workspace-wide caller search (grep `spawn_accept_loop|run_connection|bind_accept_socket|authenticate_peer|AsyncHidrawDevice|HidrawDevice|CidTranslator|CtaphidPacket|build_*_packet|parse_ctaphid_report|SkSessionTable` across `packages/`, `nixos-modules/`, `docs/reference/policy/`, `tests/`) - the only production consumers of this crate are d2bd (`SkSessionTable::default` + `stop_vm`, `SecurityKeyController::child_resources`, `SecurityKeyEffectsServiceFactory`, `SecurityKeyEffectFacets`, `security_key_descriptors`, `security_key_process_name`, vocabulary/ref constants) and d2b-provider-device (descriptor/dependency surface). The CTAPHID relay framing/accept machinery has zero production callers - but the dossier ADR046-security-key-002/030 pin the relay extraction to this crate (`src/relay.rs`/`relay_service.rs`; crate README declares the relay lives there), matching the refused declared-provider-with-zero-callers class (transport-unix/transport-vsock precedent). No new evidence that the pinned blocker changed: the relay stays.

## Refusal ledger
- #S5 [refused, no change] spec_ref duplicated (no importable shared pointer-ref parser; four copies stay) - the toolchain-codec lane retains ownership
- #S21-026 [applied in prior pass] descriptor.rs/session_ring.rs/effect_port.rs/cid.rs + relay leftovers already deleted

## Consistency notes
Types-plane crate: relay_service.rs/relay.rs/lease.rs/vocabulary.rs/effects_service.rs are the sole type definitions for the family's surface; no duplicate type definitions or wire-shape skew found. SkSessionTable declared once (relay_service.rs) and consumed by d2bd composition; no sibling copy.

## Checked
Read all crate sources (driver.rs 683 elided-ranges + lib.rs re-export graph, authority.rs, controller.rs, effects_service.rs, facets.rs, lease.rs, process.rs, relay.rs, relay_service.rs, vocabulary.rs), tests/ (lease_state_machine.rs, exact_authority.rs, guest_frontend_process.rs, mutual_exclusion.rs, redaction.rs), integration/provider_lifecycle.rs, BUILD.bazel, Cargo.toml, nix/ files, README.md; dossier ADR-046-provider-device-security-key.md. Workspace-wide caller verification via grep over packages/ + d2bd + d2b-provider-device + nixos-modules. No dead surface beyond the dossier-pinned relay; handled ledger items honored.
