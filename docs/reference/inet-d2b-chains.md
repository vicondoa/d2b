# `inet d2b` chain layout (reference)

Authoritative reference for the named-table layout. Source of truth
for any tool that needs to introspect or vendor
the d2b firewall surface.

> Architectural rationale and rejected alternatives live in
> [ADR 0013](../adr/0013-w3-firewall-coexistence-policy.md). The
> operator how-to is at
> [`../how-to/host-prepare.d/firewall.md`](../how-to/host-prepare.d/firewall.md).

## Table

Exactly one named table: **`inet d2b`**. The `inet` family is used
so the same rule covers IPv4 and IPv6 (d2b disables IPv6 on
links via per-link sysctl; the `inet` family hedges against future
IPv6 enablement without re-engineering the chain layout).

## Chains

| Chain         | Hook         | Type     | Priority | Policy   |
| ------------- | ------------ | -------- | -------- | -------- |
| `prerouting`  | `prerouting` | `filter` | `-150`   | `accept` |
| `forward`     | `forward`    | `filter` | `-5`     | `drop`   |
| `output`      | `output`     | `filter` | `-5`     | `accept` |
| `input`       | `input`      | `filter` | `-5`     | `accept` |

### Priority rationale

- `prerouting` at `-150` is equal to the canonical `mangle` priority.
  At this priority `inet d2b` sees packets before per-VM bridge
  forwarding decisions are taken. Equal priority to mangle is safe in
  the `inet d2b` namespace because d2b deliberately does not
  allocate a `mangle` hook here; foreign tables use other families
  and/or other priorities.
- `forward`, `output`, `input` at `-5` sit just before the canonical
  `filter` priority (`0`). This lets `inet d2b` decide allow vs
  drop before any later filter chain can re-evaluate. The default
  policy on `forward` is `drop` so cross-VM east-west isolation
  defaults closed. The `input` chain keeps policy `accept` for host
  coexistence, but USBIP is explicitly default-denied there: backend
  ports are dropped on non-loopback ingress, and TCP/3240 is dropped
  unless an active `UsbipBindFirewallRule` inserts the env-specific
  carve-out ahead of the generic drop.

### No `raw` / `mangle` / `nat` hooks

D2b intentionally allocates none of these. Adding any requires a new
ADR. Rationale is in
[ADR 0013 §"Chain layout"](../adr/0013-w3-firewall-coexistence-policy.md).

## Rule comment convention

Every rule and every chain carries a `comment` of the form:

```
comment "d2b managed: <ownership-id>"
```

`<ownership-id>` is a stable kebab-case identifier such as
`usbip-carveout-1-1.4` or `default-deny-forward`. The drift gate uses
the `d2b managed: ` prefix to distinguish d2b-owned state from
foreign rules; the foreign-rule preservation gate
(`tests/nft-foreign-rule-preservation.sh`) asserts foreign rules are
byte-stable across repeat-apply.

## Specific-before-generic ordering

Inside any chain that contains both per-flow carve-outs and a generic
allow/drop, the carve-outs MUST sort before the generic rule.
`UsbipBindFirewallRule` is the instance of this pattern:
the trusted host batch carries a generic `input` drop for TCP/3240,
and the live broker path inserts the trusted-bundle env TCP/3240
carve-out ahead of that drop for the host-side USBIP proxy listener.
The trusted host batch also drops USBIP backend ports on non-loopback
ingress so `usbipd`'s `0.0.0.0` bind is reachable only through the
host-local proxy path.

The invariant is checked by
`d2b_host::nftables::NftBatch::assert_carveout_ordering`. A
violation surfaces as `foreign-nft-rule-shadows-d2b`.

## Drift detection

The broker re-hashes the live table on every VM start (and on every
detected `nftables.service` reload) using:

```
nft list table inet d2b -j
```

The resulting JSON is canonicalized (volatile `handle` and `index`
fields stripped, object keys sorted) and hashed with SHA-256. The
digest is compared byte-for-byte against the bundle's `host.json`
`table_hash_after` and fails closed on mismatch with
`inet-d2b-drift`.

## Foreign-rule preservation guarantees

D2b NEVER calls `nft flush ruleset`. The reconcile path emits a
declarative `table inet d2b { … }` block via `nft -f -`;
everything outside that block is untouched. The fake backend gate
asserts this by seeding foreign iptables-style and nft-style rules
and verifying their byte representation is unchanged after repeated
d2b apply rounds.

## Error taxonomy

| Discriminant                          | When                                                  |
| ------------------------------------- | ----------------------------------------------------- |
| `firewall-coexistence-mismatch`       | Detected manager disagrees with bundle policy.         |
| `foreign-nft-rule-shadows-d2b`    | Foreign hook at a priority that would shadow d2b. |
| `nft-foreign-rule-flush-attempted`    | Reconcile tried to flush a foreign rule (defensive).  |
| `inet-d2b-drift`                  | Live table hash ≠ bundle digest.                      |

All four map into `d2b-core::error::Error::internal_io` via the
`NftError::to_core_error` shim; audit envelopes carry both the
kebab-case discriminant and the typed inner detail.

## Source locations

- Chain layout: `packages/d2b-host/src/nftables.rs`
  (`build_inet_d2b_chains`)
- Detector: `d2b_host::nftables::detect_firewall_manager`
- Matrix: `d2b_host::nftables::evaluate_coexistence_policy`
- Broker op: `packages/d2b-priv-broker/src/ops/nft.rs`
  (`apply_nftables`)
- USBIP carve-out: `packages/d2b-priv-broker/src/ops/usbip_firewall.rs`
  (`bind_firewall_rule`)
- Gates: `tests/nft-coexistence.sh`,
  `tests/nft-foreign-rule-preservation.sh`,
  `tests/usbip-firewall-skeleton.sh`
