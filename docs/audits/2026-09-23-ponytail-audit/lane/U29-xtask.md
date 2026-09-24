# U29 xtask

- delete: on-crate durable-ledger island in `delivery/recovery.rs:620-1727`
  (1,108 lines): `FrozenClosure`, `FrozenClosure`/`FrozenClosure` digest
  twins, `TerminalFailureReason`, `DurableDeliveryState`, `DeliveryBinding`,
  `BindingRequestRecord`, `TerminalFailureRecord`, `MergeAttemptRecord`,
  `PostMergeReconciliationRecord`, `PostMergeSealRecord`,
  `FinalizationRecord`, `CloseRecord`, [`DeliveryLedger`] and its ten
  leading-heavy relatives, plus the island's own digest store-path/host-identity/
  operator-subject/recovery-locator/restore-instructions helpers at
  recovery.rs:620-1727. The whole island has **zero production callers**: the
  only reachable path, `RecoveryImport` → `RecoveryImportRequest` →
  `import_attestation` (command.rs:527, recovery.rs:1728-1883), constructs
  none of these types and calls none of the helpers. Nothing else in the
  workspace references any island symbol (verified: single-file island; all
  symbols count 0 refs across every other `.rs`, `.bazel`, `.nix`, `.json`,
  and `.md` in the repo). Tests that only pin the island go with it;
  `validate_at`/`RecoveryValidation`/`canonical_bytes` and the live storage
  guard on `BINDING_REQUEST_FILE` (storage.rs:478) stay. `.bzl`/BUILD
  surfaces untouched. (xtask is CLI-only; the module carries
  `#![allow(dead_code, unused_imports)]` at delivery/mod.rs:1-2, so the
  compiler's own dead-code gate cannot see the island.) [packages/xtask/src/delivery/recovery.rs:620-1727,1884+]

## Consistency notes
  
  (xtask is not a contracts/types crate; wire-shape and duplicate-type gates
  do not apply. No entity here duplicates a committed shared type — the
  island is self-contained hypothetical wire that nothing serializes.)

## Reopened refusals
  
  (refused stays refused; no new evidence)

## Checked
  
  Read packages/xtask/src/delivery/{mod.rs,command.rs,recovery.rs,
  evidence.rs,model.rs,eligibility.rs,snapshot.rs,storage.rs,
  seal.rs,history_proof.rs} symbol-by-symbol; run (1803) → import_attestation
  (1824-1883) touches only RecoveryImportRequest/CandidateDir/StateRoot/
  RecoveryAttestation/RecoveryBinding/RecoveryValidation. Ledger symbol
  census across all 620-1727 symbols: workspace-wide zero refs in source,
  BUILD.bazel, nixos-modules, tests, and docs. Confirmed the only production
  reference point (`BINDING_REQUEST_FILE` storage guard) is a separate
  constant kept. Prior xtask ledger #A2 (generated catalog) honored.
