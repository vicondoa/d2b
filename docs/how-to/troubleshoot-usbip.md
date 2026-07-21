# Troubleshoot USBIP

USBIP has three independently observable layers: declaration, allocator lease,
and active carrier. Check them in that order.

## Declaration is absent

Confirm the workload requests `usbip`, the realm is host-local, and exactly
one enabled provider has:

```nix
type = "device";
implementationId = "host-mediated";
```

Evaluation should emit one `usbip` row bound to canonical realm, workload,
provider, and role short IDs.

## Evaluation reports a conflict

Remove `security-key` from the same workload. Full-device USBIP and
ceremony-scoped FIDO mediation share one exclusive global security-key lease
and cannot be active for one workload simultaneously.

## Lease acquisition fails

Inspect the competing canonical workload target. The resource is
`device-security-key-global` with exclusive sharing, so another active claim
must release it before attachment can proceed. Do not bypass the allocator or
grant the child realm direct USB host access.

## Device is not found

Re-run provider discovery after unplugging and reconnecting the key. A USB bus
ID can change after replug. It is valid only as current operation input and
must never be copied into a path, canonical ID, or persistent declaration.

## Guest import fails

Verify the owning realm controller and broker are healthy, then inspect the
mediated attach result. Host bind/firewall work and guest import are separate
steps. Retry through the normal attach operation; do not invoke host `usbip`
commands directly because doing so bypasses lease and audit enforcement.

## Security check

Any runtime, state, lock, or socket path containing a bus ID, vendor/product
ID, HID node, or host device-node name is a contract violation. Canonical paths
contain only realm, workload, and role short IDs.
