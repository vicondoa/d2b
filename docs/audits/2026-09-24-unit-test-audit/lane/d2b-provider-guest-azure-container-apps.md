# d2b-provider-guest-azure-container-apps - unit-test audit
tests: 1 · src files: 3
net: -0 tests, -0 lines

## Findings (biggest net first)
Nothing to cut. Ship. Checked the single unit test against the crate's own `tests/provider_lifecycle.rs` (14 controller-reconcile integration tests) and the constructor/deserialization bounds in `effects.rs` - the one unit test pins a boundary the integration suite never touches (serde `try_from` revalidation), so it stays.
- gap: `AcaRuntimeConfig::new` plan-ttl and operation-capacity bounds (`plan_ttl_ms` 0/>300_000 → `InvalidPlanTtl`, capacity 0/>1024 → `InvalidOperationCapacity`) have no test (effects.rs:346-356) - unit test only exercises the cpu bound via deserialization, integration tests only use valid values.
- gap: `AcaSandboxProfile::new` `auto_suspend_secs` (60..=86_400) and `AcaMemoryMib` (512..=16_384 step 256) bounds have no test (effects.rs:118-128, 191-197) - same class of constructor boundary the cpu bound gets tested for.
- gap: `AcaProviderConfig::new` resource-type checks on `gateway_execution_ref`/`control_credential_ref`/`pull_credential_ref`/`network_ref` have no test (effects.rs:406-422) - mis-typed refs would slip through until runtime.

## Keep
- `runtime_config_deserialization_revalidates_constructor_bounds` - pins that `serde_json::from_str::<AcaRuntimeConfig>` accepts a valid camelCase doc and rejects out-of-bounds `cpu` (251 < 250..=4000 step-250), proving the `#[serde(try_from)]` path revalidates constructor bounds instead of bypassing them.