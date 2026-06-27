# 0013. W3 firewall coexistence policy matrix + `inet d2b` chain layout

- Status: Accepted
- Date: 2026-06-09
- Wave: W3 (scope s3)
- Supersedes: extends ADR 0005 §"firewall coexistence" into a concrete
  7-row policy matrix.

## Context

W3 introduces the privileged `ApplyNftables` broker op and a USBIP
firewall-rule skeleton (`UsbipBindFirewallRule`). The named table
`inet d2b` must coexist with whatever firewall manager is already
present on the host — firewalld on Fedora, ufw on Ubuntu, Docker on
mixed CI / dev hosts, libvirt on virtualization hosts, the
`iptables-nft` compat shim, and "nothing" on a clean NixOS host.

Two failure modes are unacceptable:

1. **Flushing foreign rules.** Some nft tooling defaults to
   `nft flush ruleset`, which silently clears the host's firewall.
   D2b MUST never do this — operators have written rules they
   expect to keep.
2. **Silent shadowing.** Even without a flush, `inet d2b` rules can
   be ineffective if a foreign hook at a more negative priority
   evaluates first. Drift detection must catch this before any VM
   starts.

## Decision

### Chain layout (exactly four chains, no `raw`/`mangle`/`nat`)

| Chain        | Hook         | Type     | Priority | Policy   |
| ------------ | ------------ | -------- | -------- | -------- |
| `prerouting` | `prerouting` | `filter` | `-150`   | `accept` |
| `forward`    | `forward`    | `filter` | `-5`     | `drop`   |
| `output`     | `output`     | `filter` | `-5`     | `accept` |
| `input`      | `input`      | `filter` | `-5`     | `accept` |

W3 intentionally allocates NO `raw`, `mangle`, or `nat` hooks under
`inet d2b`. Rationale, against the rejected alternatives:

- *Rejected: a `mangle` hook for cross-VM packet marking.* The marking
  surface adds a second priority namespace that has to coexist with
  every distro's per-manager mangle behavior (`libvirt` marks for
  masquerading, `docker` marks for `MASQUERADE` matching). The risk of
  collision is high and the benefit (faster per-VM dispatch) is
  marginal at W3 scale.
- *Rejected: a `nat` hook for the per-env masquerade.* Net-VM
  masquerade is already handled inside the net VM via its own
  per-namespace `nat` table; pulling it into the host `inet d2b`
  table would re-introduce the dual-stack failure mode ADR 0005
  explicitly rejected.
- *Rejected: an `inet d2b` `raw` chain to disable conntrack.* W3
  does not need conntrack bypass; ADR can revisit if benchmarks warrant.

Every rule and chain carries `comment "d2b managed: <ownership-id>"`
so foreign rule preservation is mechanically grep-able and so the
drift gate can distinguish d2b-managed from foreign state.

### 7-row firewall coexistence matrix

| Detected manager              | Default policy      | Rationale                                                       |
| ----------------------------- | ------------------- | --------------------------------------------------------------- |
| `firewalld` active            | `refuse`            | nft families collide unless explicit zone carve-out             |
| `ufw` active                  | `refuse`            | iptables-nft shim shadows `inet d2b`                        |
| Docker active                 | `require-unmanaged` | Docker writes its own `filter`/`nat` chains; d2b must verify forward path |
| libvirt active                | `require-unmanaged` | libvirt nft chains can shadow bridges                            |
| `iptables-nft` compat shim    | `coexist`           | only if hook priority demonstrably wins per L2 readback test    |
| unknown manager (≥ 2 hits)    | `refuse`            | default deny                                                    |
| no manager detected           | `coexist`           | clean host                                                       |

`ApplyNftables` refuses to run unless the detector result matches the
bundle's declared `CoexistencePolicy`. The detector lives in
`d2b_host::nftables::detect_firewall_manager` and takes a typed
`DetectorProbe` populated by the broker side from the standard
shell-outs (`systemctl is-active firewalld`, `systemctl is-active
ufw`, `docker info`, `systemctl is-active libvirtd`, `iptables
--version`).

### Drift detection

Pre-VM-start and on every `nftables.service` reload (inotify on
`/var/run/nftables.lock`), the broker re-hashes the live `inet
d2b` table via `nft list table inet d2b -j` and compares
against `host.json`'s `table_hash_after`. The
`hash_inet_d2b_table` helper strips the volatile `handle` and
`index` JSON fields before hashing so kernel-assigned identifiers do
not generate spurious drift.

### USBIP firewall carve-out ordering

`UsbipBindFirewallRule` adds a source-based carve-out into the
`forward` chain BEFORE the generic allow/drop rule. The ordering
invariant is enforced by
`d2b_host::nftables::NftBatch::assert_carveout_ordering`. The
`UsbipBind`, `UsbipUnbind`, and `UsbipProxyReconcile` variants are
explicitly OUT of W3 scope and are refused with the
`unknown-operation` discriminant audited as
`defaultForUnknown: deny`.

### Implementation: NO libnftnl runtime dependency

The integrator-prep nix build environment ships `nft(8)` but does NOT
ship `libnftd2b-dev`. After panel review for the W3 prep commit, the
decision is to NOT pull the `nftnl` (or `nft-rs`) crate into the
workspace at this time. Instead, `NftBatch::render_nft_script`
produces a deterministic `nft -f -` text rendering that the broker
side feeds to the real `nft` binary at apply time. Drift detection
runs on the canonicalized JSON output of `nft list table inet d2b
-j`, so the final byte-for-byte check is still authoritative.

Revisit if W3fu or later requires netlink-level rule manipulation
(e.g. atomic transactional replace of a multi-thousand-rule chain);
at that point an ADR can pin a specific binding version and the
nixpkgs build closure can grow libnftnl. The fallback is documented
inline in `packages/d2b-host/src/nftables.rs`'s crate-level
docstring.

### Error taxonomy

The s3 `NftError` enum carries four kebab-case discriminants that map
into `d2b-core::error::Error::internal_io`:

- `foreign-nft-rule-shadows-d2b`
- `firewall-coexistence-mismatch`
- `nft-foreign-rule-flush-attempted`
- `inet-d2b-drift`

Audit log envelopes carry both the kebab discriminant and the typed
inner detail (e.g. `{ detected, declared }` for coexistence mismatch)
so operators can debug without the typed payload leaking through to
non-admin readers.

## Consequences

- New crate dep `sha2 = "0.10"` (workspace) for canonical-hashing.
  Both licenses (MIT, Apache-2.0) are already in `packages/deny.toml`'s
  allow list, so no `cargo-deny` matrix change is needed.
- The 7-row matrix becomes the source of truth for the
  `host-prepare/firewall.md` operator how-to and the
  `nft-coexistence.sh`/`nft-foreign-rule-preservation.sh` /
  `usbip-firewall-skeleton.sh` gates in `tests/static.sh`.
- Operators on firewalld or ufw hosts must explicitly opt into the
  refusal-override path (W3fu ADR will define it).
- Adding ANY new hook family (raw / mangle / nat) under `inet d2b`
  requires a new ADR, not just a code change.

## References

- plan.md §"W3 `inet d2b` chain layout" (§2492-2513)
- plan.md §"W3 firewall coexistence policy" (§2515-2530)
- plan.md §"W3 pre-merge canary matrix" — rows
  `foreign-nft-rule-preserved`, `nft-coexistence-*`,
  `usbip-firewall-skeleton`
- [docs/reference/inet-d2b-chains.md](../reference/inet-d2b-chains.md)
- [docs/how-to/host-prepare.d/firewall.md](../how-to/host-prepare.d/firewall.md)
- ADR 0005 (network, firewall, TAP model) — original `inet d2b`
  decision, extended here.
