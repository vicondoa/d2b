# Host firewall coexistence (W3 s3)

Fragment owned by W3 scope s3. The integrator assembles this into
`docs/how-to/host-prepare.md`.

This document is the operator how-to for the `inet nixling` named
table that the privileged broker reconciles during `nixling host
prepare --apply` (and re-checks before every VM start). The
authoritative chain layout reference lives at
[`../../reference/inet-nixling-chains.md`](../../reference/inet-nixling-chains.md);
the architectural rationale is in
[ADR 0013](../../adr/0013-w3-firewall-coexistence-policy.md).

## What nixling installs

Exactly one named table, `inet nixling`, with four chains:

| Chain        | Hook         | Priority | Policy   |
| ------------ | ------------ | -------- | -------- |
| `prerouting` | `prerouting` | `-150`   | `accept` |
| `forward`    | `forward`    | `-5`     | `drop`   |
| `output`     | `output`     | `-5`     | `accept` |
| `input`      | `input`     |  `-5`    | `accept` |

Every rule and chain carries `comment "nixling managed: <id>"`. Nixling
NEVER allocates `raw`, `mangle`, or `nat` hooks under `inet nixling`,
and NEVER runs `nft flush ruleset`.

## What nixling does NOT touch

- Foreign tables, chains, sets, maps. The reconcile path emits a
  declarative batch for `inet nixling` only; everything else stays
  byte-for-byte intact.
- Your `iptables-save` output. If the host runs the `iptables-nft`
  compatibility shim, nixling detects it and chooses `coexist` only
  when its hook priority demonstrably wins.

## Per-distro guidance

### Fedora / RHEL / CentOS Stream (firewalld)

Default policy: **refuse**. firewalld owns the nft `filter` family
under its own zone-based abstractions; coexistence at the unprivileged
`inet nixling` priority does not survive `firewall-cmd --reload`.

To use nixling on a firewalld host, either:

1. Stop firewalld (`systemctl disable --now firewalld`) and re-run
   `nixling host prepare --apply`; or
2. Wait for the W3fu ADR that defines an explicit firewalld zone
   carve-out path. Until that lands, nixling fails closed.

### Ubuntu (ufw)

Default policy: **refuse**. ufw is implemented on top of the
`iptables-nft` shim and writes its own chains at a priority that
shadows `inet nixling`'s `forward` chain.

To use nixling on a ufw host:

1. `ufw disable` and re-run `nixling host prepare --apply`; or
2. (W3fu) opt into the carve-out override; until then the host check
   refuses.

### Mixed Docker / libvirt setups

Default policy: **require-unmanaged**. Both Docker and libvirt write
their own `filter`/`nat` chains. Nixling will install `inet nixling`
alongside them but requires an explicit
`/etc/nixling/firewall.coexist-with-{docker,libvirt}.toml` marker (W3
host check enforces this) so the operator has acknowledged the
forward-path arbitration that follows. The forward path is verified
on every VM start via the post-apply `nft list table inet nixling -j`
re-hash; drift fails closed with `inet-nixling-drift`.

### iptables-nft compatibility shim

Default policy: **coexist**. Only safe when `iptables --version`
reports `(nf_tables)` AND no other manager is active. The pre-VM-start
hook re-reads `inet nixling`'s post-apply hash and refuses to start
VMs if a foreign rule has been inserted at a priority that would
shadow the nixling decision.

### NixOS (no manager)

Default policy: **coexist**. Nixling owns `inet nixling`; the rest of
the ruleset is whatever your `networking.firewall` / `networking.nftables`
declared.

## Drift detection

Every VM start re-hashes `nft list table inet nixling -j` (with
volatile `handle`/`index` fields stripped) and compares against the
digest stored in the bundle's `host.json`. Mismatches fail closed with
`inet-nixling-drift`; remediation is to re-run
`nixling host prepare --apply`.

## USBIP firewall carve-out

When a VM is configured for USBIP passthrough, `UsbipBindFirewallRule`
adds a per-busid source-based carve-out to `inet nixling`'s `forward`
chain BEFORE the generic allow/drop. This is **firewall-only**; live
USBIP device routing (`UsbipBind`, `UsbipUnbind`,
`UsbipProxyReconcile`) is W6 scope and is refused at W3 with
`unknown-operation`.

## Troubleshooting

- **`firewall-coexistence-mismatch`**: the detected manager does not
  match the bundle's declared policy. Either change the bundle (allowed
  override per the matrix above) or stop/disable the offending manager
  and re-run `nixling host prepare --apply`.
- **`nft-foreign-rule-shadows-nixling`**: a foreign hook at a priority
  ≤ `-5` is active. Inspect with `nft list ruleset` and identify the
  source.
- **`inet-nixling-drift`**: the live table no longer matches the
  bundle digest. Re-apply with `nixling host prepare --apply`; if it
  recurs immediately, a periodic process is rewriting the ruleset
  (`firewalld --reload`, `ufw reload`, custom cron, …).
