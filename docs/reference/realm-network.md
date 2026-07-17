# Realm network resource contract

`d2b.realms.<realm>.network.mode = "declared"` emits one deterministic
realm-network plan. It does not create host links during Nix evaluation and it
does not give a child broker host-global network authority.

## Identity and allocation

Canonical realm IDs seed every opaque network, bridge, veth, TAP, policy, and
provider identifier. Linux interface names use the bounded
`d2b-<role><8-hex>` form and remain within `IFNAMSIZ-1`.

Global claims are appended to `allocator.json.resourceRequests` using only the
frozen resource kinds:

- `bridge` for the LAN and uplink bridges;
- `veth-pair` for the realm namespace boundary;
- `tap` for the net workload and each local VM workload;
- `nftables-partition` for the realm's partition of `inet d2b`.

The existing realm `namespace-boundary` request remains authoritative.
Requests have a total phase/ordinal acquisition order. A child broker receives
only allocator-approved leases and FDs and performs only the listed
namespace-local address, DHCP, DNS, NAT, filter, and MSS operations.

## Isolation and routing

Workload TAPs are isolated by default. The net-workload LAN TAP remains
unisolated so it can serve every workload. Enabling east-west traffic requires
both `network.lan.allowEastWest` and
`d2b.site.allowUnsafeEastWest`.

Each plan carries the configured MTU on bridges, veths, and TAPs. MSS clamping
is an explicit policy row. IPv6 is disabled on every managed link with RA and
autoconfiguration disabled.

The net guest keeps the forced `10-eth-dhcp` neutralizer: its catch-all match is
replaced with the impossible MAC `00:00:00:00:00:00`, while the uplink and LAN
interfaces use explicit MAC matches.

## nftables ownership

Every nftables chain and rule row carries:

```text
comment "d2b managed: r-<canonical-realm-id>"
```

The shared table is never flushed. A missing marker, a foreign marker in the
realm partition, or ambiguous observed ownership preserves foreign state and
fails closed. Chain names include the realm ownership ID, so two realm
partitions never claim the same chain.

## Host network-manager coexistence

NetworkManager reconciliation is limited to the marked block in
`/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf`:

```text
# d2b-managed begin
# d2b-managed end
```

Foreign lines are byte-preserved. A managed d2b interface reports
`nm-managed-foreign-conflict`. systemd-networkd coexistence is detection-only:
the operator must provide a configured-unmanaged rule for the `d2b-` prefix;
d2b does not overwrite foreign networkd configuration.

## Provider binding

`provider-registry-v2-extensions/network.nix` emits one `network` binding per
declared realm for `d2b-provider-network-local-realm`. The binding contains
only canonical IDs, an allocator lease reference, resource-set/policy IDs, the
net-workload role ID, and a generation. It contains no interface policy text,
host path, endpoint, credential, argv, or secret.
