# Two isolated realm networks

This example declares independent `work` and `personal` realms. Each owns a
LAN, an uplink, a net workload, and one local VM workload:

```text
work realm                         personal realm
10.20.0.0/24                      10.30.0.0/24
  net workload                      net workload
  work-app 10.20.0.10               personal-app 10.30.0.10
```

The local-root allocator assigns each realm's bridges, namespace veth, TAPs,
and nftables partition before delegating lease FDs to that realm's broker.
Neither child broker receives host-global network authority.

Workload TAPs are isolated by default. The net-workload TAP is the only
unisolated LAN port, so workloads can reach DHCP/DNS/NAT without reaching a
peer workload. The two realm CIDR sets are disjoint and
`d2b.hostLanCidrs` is added to both destination blocklists.

Workload addresses are assigned deterministically from each realm's normalized
workload order, starting at `.10`. Canonical realm and workload IDs participate
in MAC, TAP, and resource derivation, so the same ordinal is safe across realms.

The optional `multi-env-daemon-experimental` configuration demonstrates MTU
propagation, MSS clamping, and the double opt-in required to permit east-west
traffic in the work realm.
