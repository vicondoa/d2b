# `d2b-provider-system-core` hermetic tests

Cargo integration, ResourceType and controller conformance, fault,
redaction, schema, and fake-port tests live here.

- `ownership.rs` - the Host and User allowlist, and the refusal of every
  ResourceType the specification denies this Provider.
- `host_reconciliation.rs` - Host status, the non-negotiable user-only
  no-isolation posture, the rejection of an operator-supplied posture field,
  and Host status redaction.
- `user_discovery.rs` - local User discovery over a scripted effect port:
  discovered, absent, drifted, and unverified outcomes, and User status
  redaction.

Every case here is hermetic. Nothing resolves a local account, reads an
account database, or otherwise touches the machine the tests run on.
