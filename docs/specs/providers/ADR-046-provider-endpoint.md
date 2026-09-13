# ADR 0046 Provider dossier: endpoint

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-endpoint` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 1 |
| Normative | Yes |
| Owners | `d2b-provider-endpoint` crate |
| Depends on | `ADR-046-provider-model-and-packaging`, `ADR-046-resources-zone-control`, `ADR-046-resources-host-guest-process-user` |

---

## Overview

`d2b-provider-endpoint` is the crate root for the `Endpoint` ResourceType. It
moves the Endpoint driver out of the daemon: the driver, its factory, its spec
decoder, its effect port, and the `DriverDescriptor` the v3 resource plane
registers the type by all live here, and `d2bd` retains only the production
effect implementation behind the port.

The family admits exactly the endpoint shapes the plane realizes:

| Shape | Producer | Realization |
| --- | --- | --- |
| virtiofsd socket (`EndpointClass::Service`, transport unix, purpose `virtiofsd`, `recycle-with-producer`) | the serving worker Process child the binding owns | socket realized by the host effect port |
| guest-runtime control (`EndpointClass::Control`, opaque carriage, provider visibility, purposes `ch-api` and `guest-control`) | the guest's VMM Process for `ch-api`, the Guest for `guest-control` | the subject row's committed VMM Process being live; the nested VMM carries both rendezvous, so the daemon creates no socket |
| device-worker socket (opaque carriage, owner visibility, host-local, purposes `swtpm-tpm-socket` and `swtpm-control-socket`) | the swtpm worker Process | the producer worker row's `Ready` status; one launch composes both sockets and the daemon creates nothing |

Anything outside that closed set is refused at validate with
`endpoint-shape-unsupported`, preserving the previous admission behavior.

## Pending contract

- The `Endpoint` broker operations cut over in a later unit; the declaration
  carries no operation rows yet.
- The per-provider purpose derivation is inverted into
  `EndpointPurposeVocabulary`, implemented in the daemon over the declaring
  provider crates (the Cloud Hypervisor provider's child roles and the Device
  TPM Provider's declared purposes). A declaration change in either provider
  moves the admitted set without editing this crate.

## Evidence

| Gate | Signal |
| --- | --- |
| Registration | `packages/d2b-provider-endpoint/tests/registration.rs` registers the declaration through the provider registry and asserts the served decoder, factory, and allowed-source mask |
| Driver verbs | colocated unit tests over a scripted effect port (validate/recover/reconcile/finalize/delete, ordering, idempotence) |
| Daemon cutover | `packages/d2bd/src/endpoint_driver.rs` deleted; the type reaches the plane only through the registry |
