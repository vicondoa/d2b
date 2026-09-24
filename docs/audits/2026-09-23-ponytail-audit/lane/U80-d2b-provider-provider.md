# U80 d2b-provider-provider
net: 0 lines, 0 deps

- #P6 [partial] — the struct half is applied at HEAD: `ProviderDriverStatus` is now a struct (driver.rs:152, with ProviderDriverStatusBorrow accessor at :258) instead of the previous status-bundle; refused half stays — `ProviderDriverArgs` + `decode_document` are the live spec-admission surface (10 workspace callers of ProviderDriverArgs, decode_document exercised by the composition decode path), and `ConfigNixosClient` has a live caller at d2bd/src/composition.rs:11508. Refused on live-caller evidence, same class as the prior process-family refusals (#G5/#S5 no-importable-shared-in-scope and live admission gates).

## Consistency notes
(Not applicable: U80 is a provider crate, not a types-layer/contracts crate.)

## Reopened refusals
(None reopened: #P6's refused half was a live-surface refusal, not a ledger [refused] row; no refusal was re-opened.)

## Checked
Read all 6 src/ files (driver.rs 1194, drivers.rs 1034, lib.rs 1527, linux.rs 680, project.rs 1978, test_support.rs 16 = 3709 lines; main.rs driver 258 + lib driver 258) + integration/license.rs + tests/registration.rs + registration_driver.rs + driver.rs. Verified ProviderDriverStatus struct at driver.rs:152 plus ProviderDriverStatusBorrow:258; verified ProviderDriverArgs + decode_document + ConfigNixosClient live caller at d2bd/src/composition.rs:11508 (workspace-wide rg, 10 ProviderDriverArgs callers / 3 decode_document points, 6 ConfigNixosClient call sites). #P6 struct half applied; refused half stays. No new findings. [packages/d2b-provider-provider/] (leaf)
