# Product coverage checklist

**Feature**: [spec.md](../spec.md) | **Plan**: [plan.md](../plan.md)

This checklist confirms that the restored implementation artifacts cover the product
surfaces. It is a requirements aid, not a delivery or review gate.

## Resource plane

- [x] Resource declaration, schema, compilation, digest, routing, watch replay, and live
  delivery are covered.
- [x] Controller registration, effect idempotency, durable revision handling, cleanup, and
  per-Zone readiness are covered.
- [x] Provider descriptors, typed EffectPorts, broker mediation, and failure isolation are
  covered for the resource families.
- [x] The four Network/Host east-west combinations and their default-deny behavior are covered.

## Security and durability

- [x] Registrar admission uses accepted-socket evidence and a connection-scoped pidfd.
- [x] Policy bootstrap is private, one-shot, exact-revision, and non-reconstructable.
- [x] Privileged mutations have transactionally committed authoritative audit rows.
- [x] Pending export, replay binding, operation inspection, retention, and redaction behavior
  are covered, including delete responses and restart.
- [x] Host-generation handoff pins target and apply identities, transfers coordinator ownership
  exactly once, and fails closed on peer, executable, intent, or rollback mismatch.

## Cutover and release

- [x] Recovery-point evidence is bound to the exact candidate and restore instructions and is
  validated before irreversible mutation.
- [x] Superseded capabilities have removal proofs and successor behavior where promised.
- [x] Companion contracts, capability parity, explicit retirement, changelog, and version
  consistency are covered.
- [x] Conditional container, host, live, hardware, and performance checks are named where
  the changed component needs them.

## Traceability

- [x] Each functional requirement points to an implementation area and focused validation.
- [x] Each success criterion has measurable evidence.
- [x] Architectural ADR references are retained only where they explain existing code or
  trust boundaries.
- [x] Each requirement identifies the implementation and evidence needed to verify its
  contract.
