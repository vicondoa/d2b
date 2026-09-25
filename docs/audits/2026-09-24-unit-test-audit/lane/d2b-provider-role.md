# d2b-provider-role - unit-test audit
tests: 2 · src files: 3 (lib.rs, rbac.rs, driver.rs)
net: -0 tests, -0 lines

## Findings
- gap: expiry boundary of `PositiveDecisionCache` (insert_allow/contains at rbac.rs:76-95) - the test named `positives_expire_...` inserts an entry expiring at tick 10 but only asserts at tick 9 (inside the window) and a revision change; it never asserts containment is false at/after the expiry tick, so the `expires_at_tick > now_tick` boundary is unpinned anywhere in the crate.
- gap: bounded capacity eviction of `PositiveDecisionCache` (insert_allow at rbac.rs:91-94) - the `entries.len() >= max_entries` rejection of a new key (and the `max_entries == 0` no-op at rbac.rs:89) is untested by either unit test or tests/registration.rs.

## Keep
- `authorization_cache_debug_redacts_every_protected_field` - pins `AuthorizationCacheKey: Debug` redacts subject ref/uid/digest (sentinel-marked) while `PositiveDecisionCache: Debug` formats count + poisoned state without locking.
- `positives_expire_and_revision_changes_invalidate_immediately` - pins a positive entry is contained within its expiry window and a changed `PolicyRevisionSet` invalidates it immediately.
