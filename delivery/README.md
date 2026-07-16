# Delivery authorities and shared ownership

`delivery/manifest.json` remains the W4 authority and is not moved or rewritten.
W5, W6, and W7 own only:

| Wave | Authority | Implementation ownership |
| --- | --- | --- |
| W5 | `delivery/manifests/w5.json` | Runtime service and dispatch implementation, consuming the frozen service/allocator contracts. |
| W6 | `delivery/manifests/w6.json` | User, desktop, and device service implementation, consuming the frozen placement and service contracts. |
| W7 | `delivery/manifests/w7.json` | Declarative Nix, process, and resource emission against the frozen allocator API; extension of the existing provider registry only. |

`shared-contracts.json` is machine-enforced root authority. It protects the
cross-wave protobuf/generated service contracts, allocator model, workspace
dependency table and lock, delivery tooling, and foreign wave manifests. Run
from the repository root:

```console
make wave-policy-check WAVE=w5 BASE=adr0045-post-w4-contracts
```

After delivery is linearized, use the wave's immediate parent as `BASE`. A new
shared DTO, dependency, generated contract, or policy requirement returns to
`adr0045-post-w4-contracts`; it is not added on a wave branch.
