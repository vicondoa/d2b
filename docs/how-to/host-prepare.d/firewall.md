# Host firewall coexistence

This fragment is included in `docs/how-to/host-prepare.md`.

This document is the operator how-to for the `inet d2b` named
table that the privileged broker's host-prepare path reconciles (and
re-checks before every VM start). The mutating `d2b host prepare
--apply` is **not yet wired** — it returns the typed `daemon-down`
envelope (exit 1) today; `host check` and `host prepare --dry-run`
exercise the read-only path. The authoritative chain layout reference
lives at
[`../../reference/inet-d2b-chains.md`](../../reference/inet-d2b-chains.md);
the architectural rationale is in
[ADR 0013](../../adr/0013-w3-firewall-coexistence-policy.md).

## What d2b installs

Exactly one named table, `inet d2b`, with four chains:

| Chain        | Hook         | Priority | Policy   |
| ------------ | ------------ | -------- | -------- |
| `prerouting` | `prerouting` | `-150`   | `accept` |
| `forward`    | `forward`    | `-5`     | `drop`   |
| `output`     | `output`     | `-5`     | `accept` |
| `input`      | `input`     |  `-5`    | `accept` |

Every rule and chain carries `comment "d2b managed: <id>"`. D2b
NEVER allocates `raw`, `mangle`, or `nat` hooks under `inet d2b`,
and NEVER runs `nft flush ruleset`.

## What d2b does NOT touch

- Foreign tables, chains, sets, maps. The reconcile path emits a
  declarative batch for `inet d2b` only; everything else stays
  byte-for-byte intact.
- Your `iptables-save` output. If the host runs the `iptables-nft`
  compatibility shim, d2b detects it and chooses `coexist` only
  when its hook priority demonstrably wins.

## Per-distro guidance

### Fedora / RHEL / CentOS Stream (firewalld)

Default policy: **refuse**. firewalld owns the nft `filter` family
under its own zone-based abstractions; coexistence at the unprivileged
`inet d2b` priority does not survive `firewall-cmd --reload`.

To use d2b on a firewalld host, either:

1. Stop firewalld (`systemctl disable --now firewalld`) and, once
   `d2b host prepare --apply` is wired, re-run it to reconcile (it
   returns `daemon-down` (exit 1) today — use `--dry-run` to re-check); or
2. Replace firewalld with a firewall setup where d2b owns
   `inet d2b`; otherwise d2b fails closed.

### Ubuntu (ufw)

Default policy: **refuse**. ufw is implemented on top of the
`iptables-nft` shim and writes its own chains at a priority that
shadows `inet d2b`'s `forward` chain.

To use d2b on a ufw host:

1. `ufw disable` and, once `d2b host prepare --apply` is wired,
   re-run it to reconcile (it returns `daemon-down` (exit 1) today —
   use `--dry-run` to re-check); or
2. Replace ufw with a firewall setup where d2b owns `inet
   d2b`; otherwise the host check refuses.

### Mixed Docker / libvirt setups

Default policy: **require-unmanaged**. Both Docker and libvirt write
their own `filter`/`nat` chains. D2b will install `inet d2b`
alongside them but requires an explicit
`/etc/d2b/firewall.coexist-with-{docker,libvirt}.toml` marker so
the operator has acknowledged the forward-path arbitration that
follows. The host check enforces that marker, and the forward path is
verified
on every VM start via the post-apply `nft list table inet d2b -j`
re-hash; drift fails closed with `inet-d2b-drift`.

### iptables-nft compatibility shim

Default policy: **coexist**. Only safe when `iptables --version`
reports `(nf_tables)` AND no other manager is active. The pre-VM-start
hook re-reads `inet d2b`'s post-apply hash and refuses to start
VMs if a foreign rule has been inserted at a priority that would
shadow the d2b decision.

### NixOS (no manager)

Default policy: **coexist**. D2b owns `inet d2b`; the rest of
the ruleset is whatever your `networking.firewall` / `networking.nftables`
declared.

## Drift detection

Every VM start re-hashes `nft list table inet d2b -j` (with
volatile `handle`/`index` fields stripped) and compares against the
digest stored in the bundle's `host.json`. Mismatches fail closed with
`inet-d2b-drift`; remediation is to re-run
`d2b host prepare --apply` once it is wired (it returns
`daemon-down` (exit 1) today — use `--dry-run` to re-check the diff).

## USBIP firewall carve-out

When a VM is configured for USBIP passthrough,
`UsbipBindFirewallRule` adds a per-busid source-based carve-out to
`inet d2b`'s `forward` chain BEFORE the generic allow/drop.
This is **firewall-only**; the USBIP attach/detach flow is handled
separately from this firewall carve-out.

## Troubleshooting

- **`firewall-coexistence-mismatch`**: the detected manager does not
  match the bundle's declared policy. Either change the bundle (allowed
  override per the matrix above) or stop/disable the offending manager
  and, once `d2b host prepare --apply` is wired, re-run it (it
  returns `daemon-down` (exit 1) today — use `--dry-run` to re-check).
- **`nft-foreign-rule-shadows-d2b`**: a foreign hook at a priority
  ≤ `-5` is active. Inspect with `nft list ruleset` and identify the
  source.
- **`inet-d2b-drift`**: the live table no longer matches the
  bundle digest. Re-apply with `d2b host prepare --apply` once it
  is wired (it returns `daemon-down` (exit 1) today — use `--dry-run`
  to re-check); if it
  recurs immediately, a periodic process is rewriting the ruleset
  (`firewalld --reload`, `ufw reload`, custom cron, …).
