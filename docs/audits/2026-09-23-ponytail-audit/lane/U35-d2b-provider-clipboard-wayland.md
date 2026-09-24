# U35 d2b-provider-clipboard-wayland
Lean already. Ship.

Swept all 13,487 LOC (workspace census, `packages/…/src/**` + `tests/` + `integration/` + `nix/`): every declared pub symbol has a live workspace consumer already verified here —
- `src/audit.rs` ClipboardAuditSink/ClipboardAuditEvent: daemon + daemon test + d2bd impl at `d2bd/src/interaction_composition.rs:4792`
- `fd.rs` classify/validate/read-bounded vocabulary: daemon bin + daemon tests + the daemon bin's bounded reads (`read_bounded`/`read_owned_fd_bounded`, bin's own call chains)
- `src/picker.rs` PickerAuthority family: daemon + controller runner; the separate `clipd_host/picker.rs` picker family is the daemon host's distinct admission gate and prior-shrunk ledger row 73/74 already consolidated the dead half
- history vocabulary (ClipboardHistory/HistoryEntry/history window): daemon + daemon history tests
- runtime + controller runner contract: d2bd consumes via the runner contract
- `tests/*` never-run + duplicate clusters: ledger rows 73–79 apply; controller-array shrink refused (crate-layout policy pins source path, daemon uses runner contract); MIME-policy triplication reported not-consolidated — both refused rows honored without new evidence.

No hand-rolled stdlib UUID/UUIDv4 rendering found in-crate after the shared `ResourceUid::from_bytes` migration.
