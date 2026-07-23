# ADR 0046 security and threat model

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-security-and-threat-model` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-session`, `d2b-bus`, Zone runtime/core-controller, `d2b-priv-broker`, every Provider dossier owner |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-resource-store-redb`, `ADR-046-componentsession-and-bus`, `ADR-046-zone-routing`, `ADR-046-resources-zone-control`, `ADR-046-core-controllers`, `ADR-046-provider-model-and-packaging`, `ADR-046-provider-state`, `ADR-046-components-processes-and-sandbox`, `ADR-046-primitive-resource-composition`, `ADR-046-resources-host-guest-process-user`, `ADR-046-resources-volume`, `ADR-046-resources-network`, `ADR-046-resources-device`, `ADR-046-resources-credential`, `ADR-046-nix-configuration`, `ADR-046-cli-and-operations`, `ADR-046-telemetry-audit-and-support`, `ADR-046-current-code-migration-map`, `ADR-046-decision-register`, and all 27 `docs/specs/providers/ADR-046-provider-*.md` dossiers |
| Supersedes | None. This spec is a new cross-cutting synthesis. `SECURITY.md` and `docs/explanation/design.md` remain the disclosure policy and pre-v3 (v1/v2) threat model for the protected `v3` baseline and are not superseded; historical main-branch ADR 0045 (`a1cc0b2d^:docs/adr/0045-provider-and-transport-framework.md`, "d2b 2.0 provider and transport framework") and ADR 0032/0034 remain non-ancestry reuse sources whose useful invariants this spec adapts under D001/D041 |

This spec is the normative closing security and threat-model contract for the
ADR 0046 d2b 3.0 Provider control plane. It does not introduce new
ResourceTypes, Providers, or wire contracts. It collects, cross-references, and
makes independently testable every security invariant that the other
ADR-046 specs and the 27 Provider dossiers already state, and it resolves the
remaining cross-cutting threat-model questions (asset inventory, attacker
classes, per-ResourceType/per-Provider-family threat matrix, reset boundary,
incident response) that no single resource or Provider spec owns end to end.
Every normative statement below cites the owning spec/dossier and, where the
owning spec has not yet stated a control explicitly, this spec states it and
assigns it to the owning spec's future implementation surface through a
`ADR046-security-*` validation work item defined in
[Implementation work items](#implementation-work-items).

## Table of contents

1. [Purpose, scope, and normative precedence](#purpose-scope-and-normative-precedence)
2. [Assets and trust boundaries](#assets-and-trust-boundaries)
3. [Attacker classes](#attacker-classes)
4. [Minimal core vs. semantic Provider authority](#minimal-core-vs-semantic-provider-authority)
5. [Signed package/manifest/schema/config trust and publisher roots](#signed-packagemanifestschemaconfig-trust-and-publisher-roots)
6. [Component identities and separate Processes](#component-identities-and-separate-processes)
7. [ComponentSession Noise boundaries and binding](#componentsession-noise-boundaries-and-binding)
8. [d2b-bus routing, RBAC, and authorization](#d2b-bus-routing-rbac-and-authorization)
9. [Exact recipient confidentiality](#exact-recipient-confidentiality)
10. [ZoneLink: no FD/resource grants, transport carriage-only](#zonelink-no-fdresource-grants-transport-carriage-only)
11. [Gateway Guest custody](#gateway-guest-custody)
12. [Injected EffectPort boundary and the privileged broker](#injected-effectport-boundary-and-the-privileged-broker)
13. [Prohibited Provider discovery/ambient authority](#prohibited-provider-discoveryambient-authority)
14. [Process least privilege: controller/service/worker](#process-least-privilege-controllerserviceworker)
15. [LaunchTicket integrity](#launchticket-integrity)
16. [User-domain Host: no isolation](#user-domain-host-no-isolation)
17. [Volume security](#volume-security)
18. [Guest-local vs. host-backed-guest custody](#guest-local-vs-host-backed-guest-custody)
19. [Credential security: end-to-end delivery and zeroization](#credential-security-end-to-end-delivery-and-zeroization)
20. [Content secrecy: clipboard, terminal, CTAP, notification](#content-secrecy-clipboard-terminal-ctap-notification)
21. [Audit vs. OTEL: redaction, cardinality, durability](#audit-vs-otel-redaction-cardinality-durability)
22. [Lifecycle security: update, revocation, finalizer, restart, adoption, quarantine](#lifecycle-security-update-revocation-finalizer-restart-adoption-quarantine)
23. [Availability and DoS controls](#availability-and-dos-controls)
24. [Incident response and support bundle](#incident-response-and-support-bundle)
25. [Reset boundary](#reset-boundary)
26. [Per-ResourceType threat matrix](#per-resourcetype-threat-matrix)
27. [Per-Provider-family threat matrix](#per-provider-family-threat-matrix)
28. [Forbidden designs](#forbidden-designs)
29. [Residual risks and explicit non-goals](#residual-risks-and-explicit-non-goals)
30. [Current-code fit](#current-code-fit)
31. [Implementation work items](#implementation-work-items)

## Purpose, scope, and normative precedence

This spec is normative wherever it states a control not already stated by an
owning spec, and it is a cross-reference (never an override) wherever an
owning spec has already stated the control. Per `docs/specs/README.md`, "no
spec may silently override another spec." Where this spec and an owning spec
disagree, the owning spec's ResourceType/Provider-specific text is
authoritative for that ResourceType/Provider, and this spec's cross-cutting
statement is corrected to match in the same revision that discovers the
conflict; no such conflict exists as of this baseline (see
[Current-code fit](#current-code-fit)).

Scope: the v3 Zone resource/control plane described by the parent ADR and its
spec set — Zone runtime, redb resource store, ComponentSession/d2b-bus,
core-controller, every standard ResourceType (D035), the frozen initial
Provider catalog (D043-D049), Nix authoring/activation, and the CLI. Out of
scope: pre-v3 (v1/v2) daemon/broker/bash surfaces already covered by
`SECURITY.md` and `docs/explanation/design.md` (retained, not superseded);
upstream `nixpkgs`/`cloud-hypervisor`/`crosvm`/`swtpm` vulnerabilities; physical
attacks defeating disk encryption/TPM-bound unlock; CPU side channels; Nix
store supply-chain attacks upstream of the pinned artifact catalog (all four
remain explicitly out of scope per `SECURITY.md` and are not reintroduced
here).

## Assets and trust boundaries

### Assets

| Asset class | Concrete instances | Owning spec |
| --- | --- | --- |
| Zone resource store | Per-Zone embedded redb database, revision log, RBAC index, reverse-owner index | `ADR-046-resource-store-redb` |
| ComponentSession key material | Zone/ZoneLink Noise static keys, ephemeral session keys, bootstrap PSKs, session nonces/replay caches | `ADR-046-componentsession-and-bus`, `ADR-046-zone-routing` |
| Credential bytes | Secret-service secrets, Entra tokens, managed-identity tokens, relay SAS/Entra credentials, TPM-sealed systemd credentials | `ADR-046-resources-credential`, credential/transport Provider dossiers |
| Volume state | Layout entries, ACLs, identity markers, sealed state (TPM NVRAM, provider component state, store-view hardlink farm) | `ADR-046-resources-volume`, `ADR-046-provider-state` |
| Process identity | pidfd, cgroup leaf, LaunchTicket, InvocationID | `ADR-046-resources-host-guest-process-user`, `ADR-046-components-processes-and-sandbox` |
| Provider signed packages | Artifact catalog entries (digest/manifestDigest/configSchemaDigest/signatureId/trustEpoch/conformanceAttestationDigest) | `ADR-046-provider-model-and-packaging`, `ADR-046-resources-zone-control` |
| Audit segments | Append-only hash-chained JSONL, privileged/standard/best-effort records | `ADR-046-telemetry-audit-and-support` |
| Gateway Guest custody | Cloud/relay credentials, remote registries, per-realm audit held only inside a Guest execution context | `ADR-046-resources-host-guest-process-user`, runtime/transport/credential Provider dossiers |
| Host substrate | Physical/local execution context, `/dev/kvm`, TPM/USBIP/GPU/security-key hardware, host network namespace | `ADR-046-resources-host-guest-process-user`, `ADR-046-resources-device`, `ADR-046-resources-network` |
| Interactive content | Clipboard bytes, terminal/PTY bytes, CTAP payloads, notification bodies | interaction Provider dossiers (`display-wayland`, `clipboard-wayland`, `shell-terminal`, `device-security-key`, `notification-desktop`) |
| Zone configuration | Nix-emitted resource bundle, catalog, schema fingerprints | `ADR-046-nix-configuration` |

### Trust boundaries

```text
Nix build/eval (offline, hermetic)
  -> signed artifact catalog + Zone resource bundle (integrity-pinned)
    -> Zone runtime activation (verifies bundle/catalog digests)
      -> Zone resource store (redb; one writer; RBAC-gated)
        -> core-controller (system-core, system-minijail: fixed bootstrap)
          -> Provider processes (controller/service/worker; EffectPort-only)
            -> privileged broker (sole privileged executor; allocator leases)
              -> Host kernel / Guest VMM / cloud control plane
        -> ComponentSession/d2b-bus (Noise NN/KK/IKpsk2; RBAC per hop)
          -> ZoneLink (parent Zone <-> child Zone; carriage-only, no FD/path/credential)
            -> Gateway Guest (relay/cloud credential custody; never the Host)
```

Six named boundaries, each independently enforced and independently
testable:

1. **Nix build to runtime.** Everything the Zone runtime trusts at activation
   (resource bundle, artifact catalog, ResourceTypeSchema/Provider settings
   schema fingerprints) is produced by a hermetic, deterministic, offline Nix
   build and is integrity-pinned by digest before activation. See
   [Signed package/manifest/schema/config trust](#signed-packagemanifestschemaconfig-trust-and-publisher-roots).
2. **Zone resource store boundary.** Only the Zone runtime's redb coordinator
   reads/writes the store. Providers/controllers reach it only through
   ComponentSession/d2b-bus and the native RBAC engine — never a direct
   handle, path, or ambient socket (D011, D006).
3. **Component/process boundary.** Every controller, service, and worker is a
   distinct process/UID pair with least-privilege sandboxing; privileged
   effects cross this boundary only through an injected `EffectPort` trait
   (D077). See [Injected EffectPort boundary](#injected-effectport-boundary-and-the-privileged-broker).
4. **d2b-bus/ComponentSession boundary.** All control-plane traffic —
   local, user, guest, remote, and nested — is authenticated Noise traffic
   authorized by the native Role/RoleBinding engine; a payload cannot
   self-assert a role (D054, D040). See
   [ComponentSession Noise boundaries](#componentsession-noise-boundaries-and-binding)
   and [d2b-bus routing, RBAC, and authorization](#d2b-bus-routing-rbac-and-authorization).
5. **ZoneLink boundary.** Cross-Zone traffic is capability-ceilinged,
   RBAC-relayed, and structurally excludes FDs, credential bytes, and host
   paths (D016, D081). See
   [ZoneLink: no FD/resource grants, transport carriage-only](#zonelink-no-fdresource-grants-transport-carriage-only).
6. **Gateway Guest boundary.** Realm/cloud/relay credentials, remote node
   registries, and realm audit for any non-host-resident Zone live inside a
   Guest execution context, never the Host (ADR 0032, carried into D043 Guest
   runtime Providers). See [Gateway Guest custody](#gateway-guest-custody).

## Attacker classes

Every subsequent section maps its controls back to one or more of these eight
classes by name. Prevention, detection, and recovery controls are cited per
class; "N/A" means the class cannot reach that asset by construction.

### AC1: Malicious or compromised Provider package/process

A Provider crate, once installed, is untrusted code running with
declared-but-bounded authority. AC1 covers a malicious publisher, a
compromised build pipeline, or a Provider process compromised at runtime
(e.g. via a parsing bug in its semantic logic).

- **Prevention:** signed `PackageIdentity` (digest/manifestDigest/
  configSchemaDigest/signatureId/trustEpoch/conformanceAttestationDigest,
  RZC lines 479-527, 671-683); component/RBAC bounds (D073); `EffectPort`-only
  privileged access (D077); no broker client import (RZC lines 1621-1628 per
  Provider dossier); canonical Provider spec is exactly `{ artifactId; config
  }` with all other properties resolved from the signed manifest (D075).
- **Detection:** build-time signature chain verification (RZC lines
  2731-2735); runtime trust/conformance re-validation at activation;
  `PackageTrusted` condition (`trusted|revoked|expired-epoch|
  attestation-failed|conformance-failed`, RZC lines 671-683, 1753-1778).
- **Recovery:** quarantine (not deletion) on trust/conformance failure — all
  component Processes stopped, exported ResourceTypes withdrawn, state
  Volumes preserved for incident investigation (RZC lines 1752-1897,
  `provider-quarantine-on-trust-failure`, RZC line 2983).

### AC2: Compromised Guest

A compromised VM/sandbox/cloud Guest is assumed to have full control of its
own execution context but no ambient authority beyond what its Guest
resource, attached Volumes/Networks/Devices, and ComponentSession
authorization grant.

- **Prevention:** guest-agent network capabilities (CAP_NET_ADMIN/
  BIND_SERVICE/RAW) confined to the guest network namespace and verified
  absent from the host netns effective set at broker spawn time (INV-NET-009);
  east-west isolation default off (INV-NET-003); Volume mount views scoped per
  attachment; ComponentSession KK enrollment required for any resource-plane
  access from inside the Guest.
- **Detection:** capability presence in host netns → immediate Process
  quarantine (INV-NET-009); pidfd/InvocationID re-verification on controller
  restart catches an impersonating re-attach.
- **Recovery:** Guest quarantine (`Degraded`, no broad kill); Volume/Network/
  Device detach through normal finalizer teardown; ZoneLink revocation if the
  Guest was itself hosting a child Zone gateway.

### AC3: Compromised Host or local same-UID malicious process

A compromised Host process, or any process sharing a UID with a d2b
component, is explicitly **not** contained by d2b — this matches
`SECURITY.md`'s existing "does NOT defend against ... multi-user trust on a
single host" and the historical "unlocked Secret Service is ambient to
processes with the same uid" invariant. AC3 is scoped to what remains true
even under this assumption.

- **Prevention:** every non-host-resident credential/registry/audit lives in
  a Guest (gateway custody, §11), so a compromised Host does not
  automatically yield cloud/relay credentials; per-component sandboxing
  (namespaces, zero ambient capabilities) limits a compromised same-UID
  Process to its own declared Volume views and device attachments, not the
  whole Host.
- **Detection:** `no_isolation: true` audit records make a user-domain
  same-UID Host's reduced guarantee explicit and non-suppressible in status/
  CLI/audit (D067; see [User-domain Host: no isolation](#user-domain-host-no-isolation)).
- **Recovery:** none beyond the explicit warning — this is a documented
  residual risk (see [Residual risks](#residual-risks-and-explicit-non-goals)),
  matching `SECURITY.md`'s unsafe-local threat-model carve-out.

### AC4: Remote relay / cross-Zone peer

A relay (Azure Relay or equivalent) or a remote Zone reachable through a
ZoneLink is treated as untrusted transport carriage, not an authorization
input.

- **Prevention:** relay/transport auth material never maps to a local Role
  (`transport-azure-relay` invariant, lines 389-404: "a relay token proving
  the Listen SAS claim does not grant Admin"); ZoneLink structurally excludes
  FDs/credentials/paths (§10); capability ceilings propagate monotonically
  downward and a child cannot advertise beyond its parent's grant (ZR lines
  2081-2098).
- **Detection:** `childStaticKeyFingerprint` re-verified on every reconnect
  (ZR lines 554-580); route advertisement replay window and signature check
  (ZR lines 1835-1895); malformed/exhausted hop count rejected at the
  source bus (ZR lines 1880-1895).
- **Recovery:** ZoneLink revocation sets the capability ceiling to empty,
  withdraws routes, and closes existing streams with `zone-link-revoked`;
  already-committed child-Zone operations are not rolled back (ZR lines
  1835-1895).

### AC5: Forged or tampered Nix artifact / configuration bundle

A forged or downgraded Zone resource bundle, artifact catalog, or Provider
package attempts to make the Zone runtime activate untrusted state.

- **Prevention:** bundle SHA-256 digest computed at eval time and verified
  before any Create/UpdateSpec (`ADR-046-resources-credential` lines
  1069-1082); `catalogSha256` binds the artifact catalog to the bundle
  (`ADR-046-nix-configuration` lines 192-202); schema fingerprints
  (`resourceTypeSchemaDigests`, `providerSchemaFingerprints`) verified before
  any resource of that type/Provider activates (`ADR-046-nix-configuration`
  lines 126-137).
- **Detection:** any mismatch aborts the **entire** bundle
  (`config-bundle-integrity-failed`, `bundle-schema-mismatch`,
  `config-catalog-mismatch`) — never a partial activation.
- **Recovery:** the prior activated generation remains live; generation
  counter rejects replay/downgrade (bundle `generation` must be strictly
  greater than `store_meta.active_configuration_revision`, RZC lines
  2753-2754).

### AC6: Local same-UID peer abusing ComponentSession/d2b-bus

A local process with a legitimate Unix-socket connection attempts to gain
authority beyond what its authenticated peer credentials grant (role
self-assertion, wildcard permission claims, replayed handshake).

- **Prevention:** `AuthenticatedSubjectContext.subjectRef` is derived only
  from kernel `SO_PEERCRED`/enrolled static-key identity, never from the
  request payload (D054; ZR lines 1526-1555); explicit wildcards
  (`resourceNames: ["*"]`) permitted only for core-controller-generated
  Roles (RZC lines 868-997); RoleBinding `spec.roleRef` immutable after
  creation.
- **Detection:** `provider-wildcard-permission-restricted` rejects any
  non-bootstrap wildcard claim (RZC line 2985); denial decisions are never
  cached as allow (RZC lines 868-997).
- **Recovery:** RoleBinding deletion is one atomic redb transaction (index
  removal + `Deleted` revision + row removal) — no observable intermediate
  state a racing caller could exploit (RZC lines 1062-1260).

### AC7: Privileged broker abuse (compromised Provider requesting broker action)

A compromised or buggy Provider process attempts to obtain privileged host
mutation beyond its declared authority by abusing the injected `EffectPort`.

- **Prevention:** the broker is the sole privileged executor; it re-derives
  every privileged parameter from allocator-approved leases and the trusted
  bundle, never from caller-supplied raw parameters (`d2b-priv-broker`
  invariant, `SECURITY.md` "Host-prepare trust-boundary delta"; D077); a
  Provider crate imports no broker service/client/DTO (compile-time-audited
  per Provider dossier, e.g. `ADR-046-provider-system-minijail.md` lines
  1621-1628).
- **Detection:** `MinijailProcessEffectPort`/`VolumeEffectPort`/
  `NetworkEffectPort`/`AzureEffectPort` calls are all typed, opaque-ID,
  bounded operations; any operation outside the closed enum is a compile
  error, not a runtime check to bypass.
- **Recovery:** compromise of a Provider process cannot escalate to
  arbitrary host mutation beyond the declared broker-op enum (carried
  forward from `SECURITY.md`'s existing daemon-compromise invariant).

### AC8: State tamper and resource/API DoS

An attacker with some authorized access attempts to corrupt persistent state
(Volume identity markers, redb rows) or exhaust shared resources (connection
slots, attachment credits, audit writes) to deny service to other Zone
tenants.

- **Prevention:** Volume identity markers are tamper-evident
  (`st_dev`/`st_ino` + schema digest, `ADR-046-provider-state` lines
  301-321); redb single-writer with expected-revision preconditions rejects
  stale/conflicting writes (D005); quota/backpressure ceilings bound every
  session/attachment/queue resource (§23).
- **Detection:** marker `missing`/`replaced` status transitions the Volume
  to `Failed` rather than silently re-provisioning; `d2b_api_admission_rejected_total`
  counter tracks rejection reasons (`auth|quota|conflict|invalid|schema`).
- **Recovery:** quarantine-not-kill for ambiguous process/Volume identity
  (§22); privileged audit records are never dropped under load, even during
  a DoS (`ADR-046-telemetry-audit-and-support` durability classes, §21).

## Minimal core vs. semantic Provider authority

The Zone core (redb store, RBAC engine, ComponentSession/d2b-bus, and the two
fixed bootstrap controllers `Provider/system-core` and
`Provider/system-minijail`) is the only code trusted with ambient authority.
Every other Provider — including all four Guest runtimes, both Volume
Providers, and every interaction/credential/transport Provider — is a
semantic Provider: it decides *what* should happen and calls an injected,
narrowly typed `EffectPort` trait to make it happen; it never decides *how*
to reach the kernel/network/filesystem/cloud API directly (D077).

| Property | Minimal core | Semantic Provider |
| --- | --- | --- |
| Broker/allocator access | Yes (core-controller + broker are the sole privileged executors) | No — never imports a broker client/DTO (D077) |
| Bootstrap exception | `Provider/system-core` and `Provider/system-minijail` are the two fixed, non-configurable bootstrap Providers (D036, D051); embedded in the Zone runtime binary, pre-created before any other resource, and still subject to full trust/package/conformance validation (RZC lines 754-777, "not exempt from trust checks") | None — every other Provider is installed as an ordinary `Provider/<name>` resource before use (D016) |
| Own Process bootstrap | The fixed core-controller process launches every other Provider's static controller/service Processes and any *declared* optional per-component state Volumes (D078; a component declares a state Volume only under the storage-need test, D087) | A Provider controller never bootstraps its own Process; it may create authorized dynamic child Process/EphemeralProcess/primitive resources after it exists |
| Status authority | Computes aggregate Provider status from component/dependency/process health (D085) | Writes status only for the ResourceTypes/fields it is explicitly authorized to own; never self-declares its own aggregate `Provider` resource status |
| RBAC authority | Native Role/RoleBinding engine, wildcard Roles restricted to core-controller-generated Roles only | Cannot claim a wildcard permission (`resourceNames: ["*"]`/empty `executionRefs`) without explicit review (RZC lines 868-997, `provider-wildcard-permission-restricted`) |

Authoring `system-core` or `system-minijail` as an operator-declared Provider
in Nix fails at eval time with `"system-core and system-minijail are
bootstrap-only providers and cannot be hand-authored"` (RZC lines 754-777,
2350-2354). This closes the obvious "shadow the bootstrap Provider" attack:
an attacker cannot register a second, attacker-controlled
`Provider/system-core`.

## Signed package/manifest/schema/config trust and publisher roots

Every `PackageIdentity` field is resolved at Nix build time from the artifact
catalog, never operator-authored inline in `spec` (`ADR-046-resources-zone-control`
lines 479-527, 671-683):

| Field | Requirement |
| --- | --- |
| `digest` | Required, non-empty SHA-256 of the artifact |
| `manifestDigest` | Required, non-empty SHA-256 of the manifest file |
| `configSchemaDigest` | Required, non-empty SHA-256 of the config schema file |
| `signatureId` | Required, non-empty; links to the publisher's trust root |
| `trustEpoch` | Required; must not be revoked |
| `conformanceAttestationDigest` | Required SHA-256; must be present in the known attestation store |
| `revocationRef` | `null` or a stable revocation-check token |

Verification happens twice, at two independent points, so a compromised
build host cannot alone forge a trusted artifact:

1. **Build time** (resource compiler / `xtask gen-schemas`-class tooling):
   artifact manifest signature chain verified against the installed trust
   store; `conformanceAttestationDigest` verified present in the known
   attestation store; `configSchemaDigest` and `manifestDigest` verified
   against the actual derivation output — any mismatch is a build failure,
   not a runtime warning (RZC lines 2731-2735).
2. **Runtime** (Zone activation): the activation controller re-verifies
   `catalogSha256` and all schema fingerprints before applying any bundle
   (`ADR-046-nix-configuration` lines 126-137, 192-202).

`spec.config` for every Provider/ResourceType is restricted at both layers:
no credential bytes, no raw host paths, no PIDs/process arguments, no ambient
authority values (RZC lines 479-527). A field annotated `credentialRef: true`
in the signed Provider settings schema accepts **only** a
`Credential/[a-z][a-z0-9-]*` ResourceRef string; a raw inline value is a
build-time `inline-secret-in-settings`/`credential-value-must-be-ref` error
(`ADR-046-resources-credential` lines 929-930, 954-957; RZC lines 479-527).
`$credentialRef` is the only permitted `$`-prefixed key in config JSON — any
other `$`-prefixed key is rejected at build time (RZC line 2396). A
heuristic `contains_sensitive_shape` lint additionally scans every string
value for PEM headers, bearer prefixes, AWS-key shapes, and long base64/hex
runs at Nix eval time, failing the build under `--strict-secrets`
(`ADR-046-resources-credential` lines 929-930; RZC lines 2408-2411).

Trust failure and conformance failure are handled identically: the Provider
is **quarantined**, not deleted. `quarantined = true`; all component
Processes are stopped; exported ResourceTypes are withdrawn; state Volumes
are preserved for incident investigation (RZC lines 1752-1897). See
[Lifecycle security](#lifecycle-security-update-revocation-finalizer-restart-adoption-quarantine).

Store paths and raw Nix closure metadata are catalog-private implementation
data. They never appear in a public ResourceSpec, status, audit record, log
line, metric label, or span attribute (`ADR-046-nix-configuration` lines
178-181; RZC lines 2566-2569, 3075). Build tests
`nix-build-artifact-store-path-absent-from-bundle` and
`nix-build-artifact-store-path-absent-from-config` enforce this at every
build (RZC line 3057).

## Component identities and separate Processes

A Provider is one independently buildable, signed crate/package (D012), but
it may declare up to 8 controller components, 8 service components, and 32
worker process templates (D073); each is compiled to its own `binaryRef` —
a key into the package's `executableDigests` table binding that component to
one exact, digest-identified binary (`ADR-046-resources-zone-control` lines
754-777). `allowedDomains` (`system`/`user`) on each component prevents a
user-domain component from claiming system-domain execution privileges.

`Provider/system-core` and `Provider/system-minijail` share the same
core-controller process boundary at bootstrap, but they use **distinct
authenticated subjects and closed RBAC grants**; after bootstrap, both are
ordinary RBAC subjects like any other component (RZC lines 1564-1627).
Bootstrap authorization is a compiled, non-extensible narrow policy: only
these two exact subjects receive the closed initial recovery/config
publication verbs, it cannot be widened by operator config, and
`bootstrap-supersession-atomic` guarantees no window where both bootstrap
policy and stored RBAC are simultaneously active once
`IndexBuilt=True` (D052; RZC lines 1564-1627, line 3030).

Controllers own their watch/coalescing queues and retry decisions
independently; workers have **no** `ResourceClient`, no d2b-bus/dependency
portal, no Credential access, no CLI, no broker, and no child-spawn
authority — every resource, FD, and config value they need is inherited
through their `LaunchTicket` (D078). This is the concrete mechanism behind
[Process least privilege](#process-least-privilege-controllerserviceworker).

## ComponentSession Noise boundaries and binding

ComponentSession (`ADR-046-componentsession-and-bus`) is the sole
control-plane transport authentication layer. Three Noise profiles, all
`25519·ChaChaPoly·SHA256`, are the closed set:

| Profile | Usage | Forbidden fallback |
| --- | --- | --- |
| `Nn` (ephemeral/ephemeral) | Bootstrap and intra-Zone local sessions where static keys are unavailable | Never used for sensitive credential delivery (D056) |
| `Kk` (static/static) | Enrolled peer sessions — ZoneLink, credential delivery, guest control | Both sides must already have enrolled static keys; no downgrade to `Nn` |
| `IKpsk2` (initiator-known/responder-known + PSK) | One-time bootstrap using an allocator-issued single-use PSK | PSK is `Secret32` (zeroizing), checked single-use at handshake time |

There is no `none`, local plaintext, HMAC, long-lived guest PSK, or weaker
retry. Noise ephemeral keys, cipher state, nonces, sockets, and session keys
are never persisted or adopted across reconnect (adapted from historical
main ADR 0045 lines 1389-1391; re-affirmed structurally in
`ADR-046-zone-routing` lines 96-124).

**Prologue and offer binding.** `prologue = preface‖canonical-offer`: the
16-byte network-order preface (`PREFACE_MAGIC = b"D2BCS2\r\n"`,
`COMPONENT_SESSION_MAJOR = 2`, `COMPONENT_SESSION_MINOR = 0`) and the
canonical-serialized `HandshakeOffer` (bounded
`MAX_HANDSHAKE_OFFER_BYTES = 16384`, canonical length 148 bytes) are
concatenated and bound into the Noise prologue. Any tampering with the offer
diverges the transcript and fails the handshake (ZR lines 96-124). The offer
carries exactly one endpoint-policy allowed purpose, initiator role,
responder role, service package, schema fingerprint, Noise profile, limit
profile, and attachment policy — there is no semver range, ignored feature
flag, codec fallback, or lower limit selected after failure.

**Generation/revision binding.** The handshake `INIT` payload contains a
SHA-256 commitment to the current generation/revision; the responder
verifies this before completing `ACCEPT` and fails closed on any mismatch
(ZR lines 96-124). For ZoneLink specifically, `spec.childStaticKeyFingerprint`
(sha256-hex) pins the child's expected static public key and is re-verified
on **every** reconnect; a child presenting a different key is refused before
any resource exchange. `childZoneUid` is recorded on first successful
connection and checked on every reconnect; a UID change resets the cursor to
revision 0 rather than silently continuing (ZR lines 554-580). Reconnect
always performs a new handshake and increments the session generation;
calls retry only under the provider/operation idempotency contract, and
streams resume only through an explicit generation and cursor contract
(historical main ADR 0045 lines 1574-1576, carried forward unchanged).

**Record protection.** AEAD encryption with a directional sequence number; a
1024-entry replay cache per direction; `MAX_PROTECTED_CIPHERTEXT_BYTES =
65535`; `MAX_LOGICAL_MESSAGE_BYTES = 1_048_576` (1 MiB). Handshake deadlines
are `LOCAL_HANDSHAKE_DEADLINE_MS = 5000`, `REMOTE_HANDSHAKE_DEADLINE_MS =
15000` (ZR lines 37-55, 96-124; RZC line 3568).

**Directional local peer credentials.** The acceptor reads kernel
`SO_PEERCRED` from the accepted socket before the preface. The connecting
initiator authenticates the responder endpoint from an expected fixed or
broker-generated path under the integrity-checked endpoint contract, anchored
parent ownership/mode, matching pre-/post-connect path device/inode/type
observations, and the expected system/user socket unit and activation
owner. An established ComponentSession endpoint is never transferable by
`SCM_RIGHTS`, pidfd duplication, inheritance, or broker handoff (historical
main ADR 0045 lines 1394-1449, carried forward as `d2b-bus` invariants).

`GUEST_SESSION_CREDENTIAL_*` constants and `GuestBootstrapPsk`/
`GuestSessionCredentialV1` from the historical realm-era ADR 0045 model are
explicitly excluded from v3 (ZR lines 37-55): v3 has no long-lived shared
guest-control HMAC token; every guest session is an enrolled `Kk` (or
one-time `IKpsk2`) ComponentSession like any other Zone peer.

## d2b-bus routing, RBAC, and authorization

ComponentSession authentication and native resource authorization share one
identity: `AuthenticatedSubjectContext.subjectRef` is set from the KK-enrolled
parent static key identity (or the local `SO_PEERCRED` peer for `Nn`
sessions), **never** from any field in the forwarded request payload (D054;
RZC lines 868-997, 1062-1260). This is the concrete mechanism behind "no
self-asserted role": a component cannot claim `Admin`, a different
principal, or a wider capability merely by writing it into a request.

**Closed verb sets.** Resource verbs: `get, list, watch, create, update-spec,
update-status, update-metadata, update-finalizers, delete`. Session verbs:
`connect, invoke, open-stream, attach, cancel, observe`. All verb tokens
outside this closed set are rejected with `role-unknown-verb-rejected`
(RZC lines 3422-3430, adapted from `verb_requires_admin()` in the baseline
`d2bd/src/admission.rs`).

**Per-hop RBAC.** For every intermediate Zone hop in a ZoneLink chain, the
forwarding d2b-bus performs a `relay` verb check against the local RBAC index
before forwarding the frame — a Zone never grants authority beyond its own
Role/RoleBinding evaluation, and forwarded calls are re-authorized at each
hop (ZR lines 1526-1555, 1574-1601). This closes the historical "one relay
hop is authorized, therefore every downstream hop is trusted" gap.

**Wildcard restriction.** Explicit wildcard (`resourceNames: ["*"]`) is
permitted only for core-controller-generated Roles. Operator-authored or
Provider Roles with a wildcard are rejected at admission with
`resource-schema-invalid` (RZC lines 868-997); `provider-wildcard-permission-restricted`
is the corresponding conformance test (RZC line 2985). An empty
`resourceNames: []` for a non-core Role means "all names of this
ResourceType" and is a distinct, narrower grant than `["*"]`.

**RoleBinding immutability and atomic deletion.** `spec.roleRef` is immutable
after creation. Deletion is one atomic redb write transaction: RBAC index
entry removal, `Deleted` revision event, and resource row removal all commit
together — there is no observable intermediate state a racing authorization
check could exploit (RZC lines 1062-1260). A subject UID change invalidates
existing sessions via `SubjectIdentityChanged`, and existing positive
authorization-decision caches are invalidated; **denial decisions are never
cached as allow** (RZC lines 868-997).

**Scope narrowing.** `scopeNarrowing` may only restrict, never grant beyond
the referenced Role; a `scopeNarrowing` verb absent from the Role is rejected
with `rolebinding-scope-exceeds-role-rejected` (RZC lines 1062-1260).

**Status write authority.** Only the core-controller handler may
`update-status` for any Zone control resource (`zone-control-status-owner-only`,
RZC line 3039) — a Provider cannot forge its own aggregate health.

## Exact recipient confidentiality

Every dedicated-recipient authorization check in the resource/session model
enforces exact-match recipient identity, never a class or wildcard match:

- **Credential `consumerRef`.** The bus admission check verifies that the
  requesting session's authenticated subject matches the Credential's
  `consumerRef` exactly; a mismatch closes the session with
  `authorization-denied` (`ADR-046-resources-credential` lines 378-383,
  480-491). `operationClasses` on `CredentialSpec` further narrows the
  allowed operation set per consumer; a request outside that set is rejected
  the same way.
- **externalPrincipalSelector.** Bounded at 512 bytes canonical JSON and
  restricted to opaque enrollment digests — it may not contain credential
  bytes, so an external-principal RoleBinding cannot be used to smuggle
  secret material into the RBAC index (RZC lines 1062-1260).
- **Sensitive KK delivery.** Credential Providers may deliver raw token
  bytes or `SignChallenge` signatures only over a dedicated end-to-end Noise
  `Kk` ComponentSession to the exact enrolled consumer Provider/component
  named by the Credential's signed descriptor/RBAC grant (D055, D056, D068);
  see [Credential security](#credential-security-end-to-end-delivery-and-zeroization).

The common failure mode this prevents is a "close enough" recipient match —
for example a Credential intended for one Provider component being readable
by a sibling component of the same Provider, or by any subject holding a
generic "read credentials" Role. Exact `consumerRef`/session-subject matching
closes both.

## ZoneLink: no FD/resource grants, transport carriage-only

Every resource belongs to exactly one Zone (D016). A parent Zone represents a
child with a local `ZoneLink/<name>` resource and accesses the child's
resources only through the child's own Zone API — a parent never mounts or
mirrors the child store, and ordinary resource references never cross Zones
(D017).

**Structural, not policy, enforcement.** No FD, credential, or host path is
forwarded across a ZoneLink. This is enforced as a **structural
serialization-boundary failure**, not a runtime policy decision that could be
misconfigured away (ZR lines 1779-1801):

- **File descriptors:** `SCM_RIGHTS` is local-Unix-socket only. Every
  ZoneLink transport (vsock, Azure Relay, etc.) carries no FDs. Attempting to
  attach an FD to a ZoneLink frame fails with
  `attachment-not-permitted-over-zone-link` (ZR line 2065).
  `transport-vsock`'s `TransportDescriptor.attachment_support = false`
  (INV-VSOCK-003) and `transport-unix`'s `attachments_enabled=false` for
  `route_class=zone-link` (`attachment-policy-conflict`) both structurally
  enforce the same invariant at the transport layer.
- **Credential bytes:** no token, PSK, private key, bearer token, enrollment
  secret, or credential-lease byte may appear in any forwarded frame.
- **Filesystem paths:** no filesystem path, socket path, device path, or Nix
  store path in routing metadata. `ZoneRoutePath` carries only Zone tree
  edges and a session generation — no transport socket information or
  credential.
- **Process identities:** no PIDs, pidfds, or broker operations propagate
  across a Zone boundary.

**Transport carriage-only.** Transport Providers (`transport-unix`,
`transport-vsock`, `transport-azure-relay`) never own ZoneLink state (D081).
The core ZoneLink handler alone reads/writes/finalizes `ZoneLink` and owns
Noise/session/reconnect/route/idempotency/intent state; it calls typed
`OpenTransport`/`CloseTransport`/`ObserveTransport` on the installed
Transport Provider, which returns only opaque `OwnedTransport`/byte-stream
handles and observations. A Transport Provider crate does not call the raw
transport syscalls itself (e.g. `transport-vsock` does not depend on
`tokio-vsock`; `AF_VSOCK` `socket`/`connect`/`bind`/`accept` live exclusively
in the core `LiveVsockEffectPort` adapter, INV-VSOCK-004).

**Additional structural rules:**

- Cross-Zone `ownerRef` fails admission with `resource-ref-invalid`;
  `ResolveRef` cannot cross Zone boundaries (RZC lines 479-527).
- A `CommitBatch` spanning multiple Zones is rejected before forwarding.
- Capability ceilings propagate monotonically downward; a child cannot
  advertise capabilities beyond what its parent allocated
  (`zone-advertisement-namespace-violation`, ZR lines 2081-2098, line 2070).
- A parent Zone never receives a database handle, credential, token, or
  cross-Zone `ResourceRef` from a child; the parent only learns child state
  from authenticated responses (ZR lines 2081-2098).
- Route advertisements are signed and expiring (max 7200 s, max 64 routes per
  advertisement); replay is rejected on a
  `(zone, principal, node, operation_kind, idempotency_key)` 6-tuple within a
  15-minute dedup / 60-minute tombstone window (ZR lines 1835-1895).
- Hop count is enforced at the **source** bus, not the destination
  (`maxHops` default 8, max 16); a malformed hop count that claims more hops
  than physically arrived is `malformed-hop-count`, and counter exhaustion is
  `hop-limit-exceeded` (ZR lines 1880-1895).

## Gateway Guest custody

A Zone that is not host-resident reaches its remote/cloud/relay surface only
through a Guest execution context; the Host never holds that Zone's relay,
provider, or remote-node credentials, registries, or audit (adapted from
ADR 0032's gateway-VM invariant, carried into the v3 Guest runtime Provider
family via D043 and D050's Host/Guest split).

- **Admission-enforced placement.** Every cloud/relay-facing component's
  `executionRef`/`gatewayExecutionRef` must resolve to a `Guest/<name>`, never
  a `Host/<name>`; the admission controller rejects a Process with
  `executionRef: Host/*` for any ACA, Azure VM, or Azure Relay component at
  Nix eval time and at runtime (`ADR-046-provider-runtime-azure-container-apps.md`
  lines 1100, 1186-1201, 1288; `ADR-046-provider-transport-azure-relay.md`
  lines 428-431).
  `Provider.spec.config.executionRef`/`gatewayExecutionRef` absent or
  Host-pointing is a hard rejection, not a warning.
- **No Host-domain subject has any Role/RoleBinding for the cloud/relay
  Provider** — the RBAC grant itself does not exist for Host subjects, so
  there is no privilege to escalate even if a Host process were compromised
  (ACA dossier, "No Host-level Azure transport; ZoneLink from gateway Guest
  only").
- **Credential acquisition stays inside the Guest.** Token bytes are
  acquired per reconcile tick inside the gateway Guest's credential-consuming
  component, held only in zeroizing memory, and zeroized immediately after
  the cloud/relay API call; they never cross to the Host Zone controller,
  never appear in any resource spec/status/audit/OTEL/log/environment
  variable (`ADR-046-provider-transport-azure-relay.md` lines 416-424,
  461-478; `ADR-046-provider-runtime-azure-virtual-machine.md` lines
  1363-1367).
- **No cross-Zone credential minting.** No parent-minted token is delivered
  to a child Zone; each Zone's gateway component independently acquires its
  own credentials (`ADR-046-provider-transport-azure-relay.md` lines
  454-459).
- **Relay/managed-identity/Entra auth material never maps to a local Role.**
  A relay token proving a `Listen` SAS claim does not grant `Admin`; a
  managed-identity token proving a `Send` Entra claim does not grant Zone
  resource API access beyond what the KK-enrolled child static key
  authorizes (ADR 0032; `ADR-046-provider-transport-azure-relay.md` lines
  389-404). This is the same invariant as AC4's "relay identity is not local
  auth."
- **Bootstrap PSK single-use, sealed at rest.** Where a Guest bootstraps a
  cloud VM via `IKpsk2` (Azure VM `bootstrap-svc`), the PSK is sealed
  ciphertext in a Volume — plaintext is held only in the controller process
  address space during the delivery window and zeroized immediately after
  (`ADR-046-provider-runtime-azure-virtual-machine.md` lines 412, 432, 447,
  450-454). Replay of a consumed PSK is rejected.
- **Degraded, not silently local, on Guest unavailability.** If the gateway
  Guest is unreachable, the Provider transitions to `Degraded` with a typed
  reason (`gateway-guest-unavailable`); there is no fallback to a Host
  process (ACA dossier line 1218).

## Injected EffectPort boundary and the privileged broker

Every semantic Provider composes behavior by creating owned primitive
resources and by calling one injected, narrowly typed, async `EffectPort`
trait per domain (D077). No Provider process imports the broker, receives a
broker socket/DTO, or directly opens a host path/device/systemd socket, or
performs a privileged mutation — including the primitive Process/Volume/
Network/Device Providers themselves.

| Domain | EffectPort | Sole privileged executor | Enforcement |
| --- | --- | --- | --- |
| Process | `MinijailProcessEffectPort` | `Provider/system-minijail` controller + `d2b-priv-broker` `SpawnRunner` | Compile-time dependency audit: the Provider crate imports no `d2b.broker.v3` service/client/DTO (`ADR-046-provider-system-minijail.md` lines 1621-1628) |
| Process (systemd) | `SystemdProcessEffectPort` | `Provider/system-systemd` controller via D-Bus transient unit API | Controller never connects to the systemd D-Bus socket directly and never calls `systemctl` as a subprocess |
| Volume | `VolumeEffectPort` | `Provider/volume-local` controller + broker `ProvisionLayoutEntry`/`RepairLayoutEntry`/`CleanupLayoutEntry`/`RotateSealingKey`/`PrepareSwtpmDir` | "The controller process holds no claim that grants access to raw host paths" (`ADR-046-provider-volume-local.md` lines 1739-1776) |
| Network | `NetworkEffectPort` | `Provider/network-local` controller + broker `CreateBridge`/`DeclareTap`/`ApplyNftables`/`ApplySysctls` | "The controller holds no broker role and no `network-admin` capability" (`ADR-046-provider-network-local.md` lines 1680-1682) |
| Device (vsock) | `VsockEffectPort` | Zone runtime `LiveVsockEffectPort` | `tokio-vsock` is not a dependency of `transport-vsock` (INV-VSOCK-004) |
| Cloud (ARM/ACA) | `AzureEffectPort` | The cloud runtime Provider's own controller, confined to the gateway Guest | All calls non-blocking; `AzureOperationHandle` is opaque, max 256 bytes |

The broker remains the sole privileged executor and independent audit owner
(`ADR-046-provider-model-and-packaging`, D077). It re-derives every
privileged parameter from allocator-approved leases and the trusted bundle,
never from a caller-supplied raw parameter — carrying forward
`SECURITY.md`'s existing "compromise of `d2bd` cannot escalate to arbitrary
host mutation beyond the declared broker enum variants" invariant into the
Zone/Provider model. Opaque `LaunchTicket`/lease IDs are passed across the
`EffectPort`; the Provider never receives raw broker parameters or a broker
wire message.

## Prohibited Provider discovery/ambient authority

No Provider process discovers or opens a broker client, host path, device,
systemd unit, or compositor socket by ambient lookup. Every dossier states
this as an explicit "MUST NOT"/"never" invariant for its domain:

| Surface | Explicit prohibition | Source |
| --- | --- | --- |
| Broker client | Provider crate imports no broker service/client/DTO; compile-time dependency audit | `ADR-046-provider-system-minijail.md` lines 1621-1628; every EffectPort-consuming dossier |
| Host path | `sourcePolicyId` is opaque; "the controller itself never opens the host path directly and never sees the raw path" | `ADR-046-resources-volume.md` lines 794-799 |
| Device node | "No Device Provider receives a blanket device-path grant, a raw socket address, or an ambient host capability" | `ADR-046-resources-device.md` lines 320-322 |
| Security-key hidraw | "The broker opens the hidraw node exclusively from the trusted bundle device table; it never accepts a caller-supplied path"; relay process "MUST NOT open any device node itself" | `ADR-046-resources-device.md` lines 328-329, 776-779 |
| systemd D-Bus | "The controller MUST NOT connect to the systemd D-Bus socket directly ... MUST NOT call `systemctl` CLI as a subprocess" | `ADR-046-resources-host-guest-process-user.md` §6.3 (system-systemd) |
| Compositor socket | "The `Provider/display-wayland` status resource never publishes a D-Bus address or socket path. No ambient environment lookup or `/run/user/<uid>/bus` default path is used as a fallback" | `ADR-046-provider-notification-desktop.md` lines 1292-1293; `ADR-046-provider-display-wayland.md` (compositor session) |
| D-Bus session bus | No `$DBUS_SESSION_BUS_ADDRESS` lookup, no `/run/user/<uid>/bus` fallback; D-Bus FD delivered only via ComponentSession attachment at a declared in-jail number | `ADR-046-provider-notification-desktop.md` lines 1290-1295; `ADR-046-provider-audio-pipewire.md` |
| vsock AF_VSOCK syscalls | `tokio-vsock` is not a dependency; `socket`/`connect`/`bind`/`accept` live only in the core effect adapter | `ADR-046-provider-transport-vsock.md` INV-VSOCK-004, lines 212-220 |
| Nix daemon socket | "No ambient Nix daemon socket access; activation helper uses sealed closure from artifact catalog exclusively" | `ADR-046-provider-activation-nixos.md` (technical_details) |
| ARM/cloud credential | "No ambient credential fallback (no `AZURE_CLIENT_ID`, `MSI_ENDPOINT`, SDK env chain)" | `ADR-046-provider-runtime-azure-virtual-machine.md` line 428 |

Every FD a Provider process needs is delivered at a declared in-jail number
by its `LaunchTicket` before the process's first instruction — never
discovered by path, environment variable, or ambient socket lookup. This is
the concrete mechanism the table above enforces; see
[LaunchTicket integrity](#launchticket-integrity).

## Process least privilege: controller/service/worker

Every component is one of three narrowly bounded classes (D078, D073):

| Class | Authority | Bound |
| --- | --- | --- |
| Controller | Owns its watch/coalescing queue and retry decisions; may create authorized dynamic child Process/EphemeralProcess/primitive/vendor resources | Max 8 controllers per Provider |
| Service | Exports ttrpc methods; may send typed internal requests to its controller for worker help | Max 8 services per Provider |
| Worker | Ephemeral execution; **no** `ResourceClient`, no d2b-bus/dependency portal, no Credential access, no CLI, no broker, no child-spawn authority — everything is inherited via `LaunchTicket` | Max 32 worker templates per Provider |

A narrowly declared worker may own its exact workload child only when that
child is the worker's manifest-fixed data-plane purpose under explicit
descriptor policy (e.g. a shell-supervisor worker owning its shell) — this is
the one, explicitly scoped exception to "workers have no child-spawn
authority" (D078).

Sandbox defaults (`ADR-046-resources-host-guest-process-user`, D079):

- `noNewPrivileges: true` by default.
- `startRoot: false` for user-domain Processes; any user-domain Process
  requesting `startRoot: true` is rejected at `validateSpec`.
- `readOnlyRoot: true` by default.
- `capabilityClasses: [sys-admin]` requires an explicit carve-out in the
  Provider descriptor's `allowedCapabilityClasses`; without it, the request
  is rejected.
- `seccompClass: allow-all` requires the same explicit carve-out.
- `UserNamespaceSpec.mappingClass: process-principal-root` is the sole
  frozen value (D079): in-namespace UID/GID 0 maps to the host UID/GID of the
  Process's resolved `User/<name>` principal; the numeric host UID/GID is
  **never** in the public ResourceSpec, status, audit payload, or API
  surface — resolution happens exclusively inside the private
  effect-adapter state at launch time.

**Nine pidfd non-exportability invariants** (`ADR-046-resources-host-guest-process-user`
lines 764-794, 1280-1290), verbatim:

1. Pidfd is never stored in the resource store.
2. Pidfd is never sent over d2b-bus, ComponentSession, or any ttrpc call.
3. Pidfd is never written to any log, metric, audit record, or status field.
4. Pidfd is never inferred from status by a caller.
5. No API method accepts a pidfd from a caller.
6. Every controller restart acquires a fresh pidfd through re-verification —
   a pidfd is never persisted across restarts.
7. An ambiguous pidfd identity (suspected PID reuse) results in quarantine,
   never a broad kill of all processes in a cgroup.
8. A controller crash while holding a pidfd must be detectable; orphan
   recovery uses PID re-verification from the cgroup leaf, not the stale
   pidfd.
9. Pidfd is closed (not just leaked) before the holding controller restarts.

`Provider/system-systemd` binds process identity to
`InvocationID`+cgroup+`MainPID`/start-time rather than the pidfd primitive
(D022); `Type=forking` is forbidden (daemonizing prevents pidfd-based
identity verification), and the unit name alone must never be treated as
process identity — identity requires verification against the unit's main
PID (`ADR-046-resources-host-guest-process-user` lines 803, 818).

A Process ResourceSpec must not contain: raw numeric UID/GID/supplementary
groups, a raw cgroup path, a raw socket address/file path, raw seccomp BPF
bytecode, a raw capability bitmask, a raw minijail argument string, a raw
systemd unit property fragment, a raw broker operation name/parameter, or an
environment variable containing a credential/token/secret byte
(`ADR-046-resources-host-guest-process-user` lines 1291-1303). Violating this
is a synchronous `spec-security-violation` rejection at admission, not a
later runtime check.

## LaunchTicket integrity

The `LaunchTicket` is the cryptographically bound, single-use launch
authorization every Process/EphemeralProcess Provider issues to its
privileged executor (`ADR-046-provider-system-minijail.md` §7.4,
`ADR-046-components-processes-and-sandbox`). It is bound to:

- the Process/EphemeralProcess `ResourceRef`;
- the resolved principal UID (never the numeric value itself — an opaque
  resolved identity);
- the spec generation;
- the `CompiledSandboxPlan` digest (namespace/capability/seccomp
  compilation output);
- the cgroup placement digest;
- the mount table digest;
- the inherited FD table.

**Descriptor identity and validation.**

- FDs are delivered at a declared in-jail number, installed before the
  process's first instruction; `CLOEXEC` is cleared only at installation,
  never held open across an unrelated exec.
- The compiled sandbox plan digest bound into the `LaunchTicket` is
  re-verified by the broker at exec time; any change between ticket issue
  and exec fails the spawn closed — this prevents a TOCTOU window between
  the Provider deciding to launch and the broker actually launching
  (`ADR-046-provider-system-minijail.md` §22 Invariant 6, lines 1593-1596).
- The pidfd is obtained atomically from `clone3(CLONE_PIDFD |
  CLONE_INTO_CGROUP)`: no window exists between clone and pidfd acquisition
  where a PID could be reused, and the process is placed in its cgroup leaf
  before any instruction executes — no window exists for cgroup escape
  (§22 Invariants 2-3, lines 1567-1575).
- Duplicate kernel objects (same `st_dev`/`st_ino`) presented as two FDs in
  one packet are rejected and cleaned up (`transport-unix`
  `duplicate-kernel-object`).
- Pidfd identity for adoption additionally requires live launch evidence via
  `/proc/<pid>/fdinfo`, a same-kernel-object check, and a double-read race
  guard on `/proc/self/fdinfo/<fd>` (`pidfd-double-read-race-guard-detects-pid-reuse`,
  `ADR-046-resources-zone-control.md` lines 3553-3559).

**Credit/quota.** `LaunchTicket` issuance is bounded per Zone (max 64
concurrent in-flight `LaunchTicket`s for `system-minijail`,
`ADR-046-provider-system-minijail.md` §17 line 1290); `startDeadline` is
range-checked 1 s-3600 s and TTL-enforced by a controller ticker so an
expired ticket cannot be exec'd late.

**Sealed memfd / attachment credit.** All memfd attachments require all four
seals (`F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_SEAL`); partial
seals are rejected with `sealed-memfd-partial-seal-rejected`. Attachment
credit is scoped and bounded (`Packet` 32, `Request` 64, `Operation` 128,
`Session` 256, `Process` 2048, `Host` 8192), each scope reserving emergency
headroom so a saturated data path cannot starve control-plane FDs
(`ADR-046-zone-routing.md` lines 617-641; `ADR-046-resources-zone-control.md`
lines 3553-3559).

## User-domain Host: no isolation

Current `unsafe-local` becomes a user-only `Host` under `Provider/system-core`
in v3, not a separate Provider (D042). It is `defaultDomain=user`,
`allowedDomains=[user]`, `defaultUserRef` set, and
`providerSettings.isolationPosture = "none"`. This is a genuine reduction in
guarantee — a Process running there executes as the operator's own host UID
with **no VM or Provider-managed isolation boundary** — and the spec
requires the reduction to be impossible to hide.

`isolationPosture = "none"` is mandatory and non-suppressible across three
independent surfaces simultaneously (`ADR-046-resources-host-guest-process-user`
lines 1270-1278; `ADR-046-nix-configuration` lines 719-764;
`ADR-046-cli-and-operations` lines 685-703):

1. **Status.** `status.isolationPosture = "none"` is always present in
   `--json`; a `[no isolation]` annotation appears in `--human` table rows.
2. **CLI/UI.** `d2b shell open` and `d2b exec run` emit
   `warning: no isolation boundary — this process runs as your host user` to
   stderr before attaching, unconditionally. **This warning has no
   suppression flag** — there is no `--quiet`/`--no-warn` escape.
3. **Audit.** Every `ProcessEffect` record (launch, stop, adopt, quarantine)
   for a Process under this Host carries `no_isolation=true` as a fixed
   closed audit label.

`no_isolation` is audit-only. It **must not** appear as an OTEL metric label,
span attribute, or structured log field, in either direction (D067;
`ADR-046-telemetry-audit-and-support`) — this prevents the isolation
weakness itself from becoming a high-cardinality or externally-queryable
telemetry dimension while keeping it fully auditable.

"A missing or silent posture field is a correctness violation. Reviewers
MUST reject any diff that adds a code path serving a Host/unsafe-local
resource without propagating the isolation posture through all three
surfaces" (`ADR-046-resources-host-guest-process-user` lines 1270-1278).

**Bidirectional rejection of `null` posture.** A Host with
`defaultDomain=user`, `allowedDomains=[user]`, `defaultUserRef` set, and
`isolationPosture=null` is rejected at both `validateSpec` and Nix eval time
— the null/absent value may not be used to evade the explicit declaration
(`ADR-046-resources-host-guest-process-user` line ~391;
`ADR-046-nix-configuration` line ~1806). Symmetrically, a system-domain
Process (`domain: system`) targeting a user-only Host
(`executionRef: Host/host-unsafe-local`) is a Nix eval error
(`ADR-046-nix-configuration` lines 748-764).

User reconcile itself must not hold OS-user credentials or perform
authentication/login — it is purely observational (NSS lookup, home-stat,
group query); numeric UID/GID appearing in status is diagnostic-only and is
never an authorization input (`ADR-046-resources-host-guest-process-user`
lines 1184-1186, 1232-1233). Authorization always uses the canonical Zone
`User/<name>` subject reference.

## Volume security

Volume (`ADR-046-resources-volume`) is the one ResourceType covering files,
directories, ACLs, views, and mount lifecycle (D032). Every security
property below is enforced by `volume-local`/`volume-virtiofs` through the
`VolumeEffectPort`, never by the calling controller directly.

**`sourcePolicyId`: opaque, never a raw path.** `source.settings.sourcePolicyId`
is an opaque bounded string referencing an entry in `volume-local`'s private
`config.allowedHostPaths` policy catalog (D082). The raw path is resolved
exclusively inside the private effect-adapter/broker path and handed to the
caller only as an opaque FD; it never appears in public status, audit
records, CLI output, or telemetry (`ADR-046-resources-volume` lines 143,
390-406). Resolving `sourcePolicyId` requires the
`volume-local/source-policy-resolve` permission, granted only to
`ProviderSupervisor` on behalf of `Provider/volume-local` — never the
controller process itself.

**ACL: typed `User/<name>` refs only.** ACL principals are always typed
`User/<name>` ResourceRefs in the same Zone — no numeric UID/GID form is
accepted (`ADR-046-resources-volume` lines 204-214;
`ADR-046-nix-configuration` lines 991-996). `foreignChildPolicy: fail` sets a
`ForeignAclViolation` condition when the broker finds directory children not
covered by a declared ACL entry, providing tamper detection for controlled
subtrees.

**View and `no-follow`.** `noFollow: true` is the default on every
`LayoutEntry`; symlink traversal is disabled by default and only explicit
`symlink`-type entries with `noFollow: false` may traverse (line 179,
802-805). Anchored relative paths are enforced: no absolute path, no `..`,
no drive letter, no null byte, no Unicode path-separator homoglyph. A worker
Process with a narrow view fails with `volume-view-rights-exceeded` if it
requests a right absent from its view declaration (`ADR-046-provider-state`
lines 437-461).

**Identity marker.** Every Volume has an identity marker — a regular file
under the broker-maintained root recording `(st_dev, st_ino)`, `schemaId`,
`schemaVersion`, and a tamper-evident HMAC digest (`ADR-046-provider-state`
lines 301-321). Fail-closed rules: a missing marker after provisioning sets
`markerStatus: missing` and the Volume transitions to `Failed` — the broker
**never** silently re-provisions; an `st_ino` mismatch sets
`markerStatus: replaced` and also fails closed. Neither condition
auto-recovers; only an operator can remediate.

**Quota.** `enforcement: hard` fails Volume creation if the backing
filesystem cannot enforce byte/inode limits; for `tmpfs` sources,
`quota.maxBytes`/`quota.maxInodes` are required and kernel-enforced as mount
options (`ADR-046-resources-volume` lines 333-352). `quotaBytes: 0` is
rejected with `component-quota-zero` (D035 catalog invariant,
`ADR-046-resources-zone-control` lines 1319-1326).

**Sensitivity classes and single-writer enforcement.**

| Class | Access |
| --- | --- |
| `private` | Single-process only — no sharing between processes |
| `internal` | Same-Provider multi-component sharing permitted |
| `shared-read` | Cross-Provider read-only sharing permitted |

At most one attachment may use `access: read-write` at any time; the
controller rejects a second read-write attachment while one is active
(`ADR-046-resources-volume` lines 730-735).

**Destruction sequence.** Volume destruction is an ordered, resumable-once
sequence: (1) shred sealing key material if sealed, (2) fd-relative
`unlinkat` to remove layout, (3) `fsync` the parent directory, (4) remove the
identity marker, (5) remove the root directory, (6) commit finalizer removal
(D084; `ADR-046-provider-state` lines 513-529). Any abort between steps 2-5
leaves a partially removed Volume with a valid marker, which is
**quarantined, not silently re-provisioned**.

**ProviderStateSet per-component isolation (D076, D087).** `ProviderStateSet`
is the optional, logical, query-time grouping of the *declared* Volume
resources owned by a Provider (`ProviderStateSet(zone, provider-name)`); it is
never a ResourceType or a stored row, and it is empty for a Provider that
declares no state Volume. Bounded non-secret operational state belongs in the
owning resource's `status` subresource and the core Operation ledger by default
(D087). A component declares a state Volume only when a payload passes the
storage-need test (secret/sensitive private recovery data; large/binary/file
content; private data unsafe for status readers; or bounded-but-revision-
unsuitable data with a demonstrated recovery need). Core `ProviderDeployment`
creates one private, framework-created Volume per *declared* `stateNamespaces`
entry from the signed state declarations, **before** launching that component's
Process; a stateless component declares no namespace and receives no Volume,
and there is no empty identity-only Volume. These declared state Volumes use the
canonical full Volume schema (extended with
`stateSchema`/`persistenceClass`/`sensitivityClass`/quota/sealing fields) and
`User/<name>` layout principals drawn from bounded, Nix-preprovisioned pools.
Each component mounts only its own declared view; there is **no cross-component
or cross-Provider sharing** and no separate non-Volume "compartment" concept.
For declared component state Volumes, `persistenceClass: persistent` is
required — `ephemeral`, `cache`, and `config` are rejected with
`component-persistence-class-forbidden`.

**Status confidentiality is RBAC, not secret storage (D087).** Resource
`status` is a redacted, RBAC-readable observation surface. It MUST NOT contain
secrets, raw tokens/keys/PSKs, authority-conferring credential handles, private
endpoint/path/argv/environment/PID/unit data, terminal/clipboard/CTAP bytes,
raw cloud error bodies, large binary blobs, or unbounded/churn-heavy content
(`ADR-046-resource-object-model` § Status prohibitions). Confidentiality of the
bounded non-secret observations status does carry is provided by the status
subresource's RBAC read authorization; status is never used to store a secret
whose exposure would depend on redaction alone.

**Three-layer status shape (D088).** All three status layers — the universal
`ResourceStatus` base, the ResourceType-common `status.resource`, and the
optional Provider-specific `status.provider` — are redacted and non-secret. The
`status.provider.details` extension schema is signed into and registered with
the Provider package and is versioned; the resource store validates every
`status.provider` write against that registered schema with strict unknown-field
denial and per-layer size/cardinality bounds, rejecting an unregistered or
version-mismatched extension (`status-provider-schema-invalid`) or one that
duplicates a universal/`status.resource` field (`status-provider-overlap`). The
signed, versioned, strict, bounded extension surface prevents an implementation
from smuggling secret, unbounded, or authority-conferring data into status under
a private field, and keeps cross-provider consumers on the provider-neutral
base-only projection (universal base + `status.resource`) so no consumer parses
another Provider's opaque details.

**Three-layer spec shape (D089).** Desired `spec` is symmetric: the universal
envelope, the ResourceType base spec at `spec.*` (including `spec.providerRef`),
and the optional canonical `spec.provider = { schemaId, schemaVersion, settings }`
extension. The `spec.provider.settings` schema is signed into and registered with
the Provider package, versioned/digested, and validated against `spec.providerRef`
at Nix build and API admission with strict unknown-field denial and spec bounds;
it may not shadow a base field (`spec-provider-shadow`) or use an
unregistered/version-mismatched schema (`spec-provider-schema-invalid`). This
prevents an implementation from smuggling unbounded, secret-shaped, or
authority-shaped desired data into spec under a private field, or from silently
reinterpreting, renaming, or weakening a base field; a Provider that cannot honor
an optional base capability must say so through its signed capability matrix and
the provider-neutral `unsupported-capability` result rather than degrading a base
field. Generic tooling authors only the base spec, so no operator or generic
controller depends on a Provider's opaque `settings`.

**Expedited reconcile (D090).** Only authorized UX mutations and core (and the
admin `resource reconcile` action) may set `waitForReconcile`; an unauthorized
request is rejected with `expedited-not-authorized`. The expedited priority lane
is quota-bounded and fair so it cannot starve ordinary reconciles or serve as a
DoS amplifier. A controller performs no external effect, finalizer release, or
status mutation until Core's typed `CommittedRevisionProof` arrives, so a
never-committed mutation causes no effect; and a durable commit is authoritative
regardless of whether the expedited pass later fails, so the API cannot be
tricked into reporting an uncommitted mutation as durable or an uncommitted
status as persisted (`statusPersistence: pending`).

**Currency and disruptive upgrade (D091).** The `status.update` currency object
carries only bounded, non-secret observed/target generation/digest IDs and
bounded/truncated owned/dependency refs — never secret material, raw artifact
paths, or unbounded collections. Disruptive changes require an explicit,
authorized `resource upgrade ... --apply`; a controller must report
`UpgradeRequired` rather than silently disrupt a running workload, and the
dependency-aware planner drains dependents before recycling so there is no
surprise disruption. Upgrades preserve TPM identity and durable/state/secret
Volumes (`preserveState`); `Replace` of a resource-row identity is used only
when explicitly required and planned with ownership/state transfer, and full
destructive factory reset remains a separate authorized path.

**Endpoint resource (D092).** A stable endpoint is the `Endpoint` ResourceType,
never a raw locator in spec/status/CLI: the base carries only closed
class/transport/locality/purpose values and bounded fingerprints, and status
carries no path/address/CID/port/fd/credential. Core/ProviderSupervisor resolves
an `Endpoint` to a live transport/FD only through the EffectPort/LaunchTicket
path under authorization; an unauthorized resolve is denied with a typed error
and returns no locator. Promoting endpoints to resources brings independent
RBAC, audit, ownership, and dependency edges to what were opaque IDs; the frozen
permitted-opaque set (pidfd, fd index, per-session named stream/`OwnedTransport`
handle, `operationId`, digests) stays internal and non-locator by the
`ADR-046-resource-object-model` promotion test.

Two invariants close the specific attacks Volume state is most exposed to:

- **TPM Volume never re-provisioned.** After the swtpm provisioning marker
  exists, a missing or replaced swtpm directory is a hard failure
  (`previously-provisioned-swtpm-state-missing`); the controller never
  silently creates a new empty TPM directory, which would look like a clean
  TPM to any IdP relying on attestation continuity (`ADR-046-resources-volume`
  lines 817-819, 628-665).
- **Store-view isolation.** virtiofsd serving `access: read-only` for the
  per-Guest Nix store ALWAYS uses `store-view/live` as `--shared-dir`, never
  the host's `/nix/store`; the `share.source == "/nix/store"` string in
  `processes-json.nix`-equivalent config is an eval-time sentinel only —
  runtime virtiofsd always serves the isolated hardlink-farm subtree
  (`ADR-046-resources-volume` lines 820-822). The virtiofsd `--shared-dir`
  argv is delivered as a `/proc/self/fd/<N>` inherited-FD path, never a
  literal host path string.

## Guest-local vs. host-backed-guest custody

By default, a Volume is **guest-local**: the Host never holds bytes,
dirfds, or identity markers for it (`ADR-046-provider-state` lines
159-272). `host-backed-guest` placement requires `hostCustodyPermitted:
true` in the Volume's **signed descriptor**; without it, the placement fails
with `placement-host-custody-violation`. Credential, audit, remote-node, and
cloud-control schemas *require* `guest-local`; attempting `host-backed-guest`
for one of these classes fails with `guest-local-required`. This is the
Volume-layer enforcement of the same custody boundary described in
[Gateway Guest custody](#gateway-guest-custody).

**No bootstrap state Volume (D086, superseded by D087).** The fixed bootstrap
components — the first `volume-local` controller instance on each execution
target, and (where present) `system-core` and `system-minijail` — keep their
bounded non-secret operational state in resource `status` and the core
Operation ledger and declare **no** state Volume. Because no component requires
a state Volume before a `volume-local` instance is Ready, there is no bootstrap
state-Volume cycle, no per-execution-target local bootstrap storage mechanism,
and no bootstrap-storage exception. There is no hidden bootstrap store: a fixed
bootstrap component reaches Ready by adopting running processes and re-deriving
its observed state from `status`, the core Operation ledger, and independent
external observation (cgroup-leaf scanning, fresh pidfds, marker reverification
against external reality).

A Guest still bootstraps its own primitive controllers (including a Guest-local
`volume-local` instance) independently of the Host: the Guest-local instance
uses only Guest-local primitives, never a leaked parent-Host dirfd or other
Host-local resource handle, and reaches Ready from Guest-local primitives and
its own status alone.

`volume-local` remains the single owner of the Volume ResourceType's layout/
spec/ownership fields; `volume-virtiofs` is a separate attachment-
implementation Provider that never writes Volume layout/spec/ownership
fields — it writes only its own owned virtiofsd Process children and the
per-attachment status entries it is authorized for (D083). Two controllers
never write the same Volume resource row.

## Credential security: end-to-end delivery and zeroization

**Zero-secret invariant.** Credential bytes MUST NOT appear in any resource
spec, resource status, resource store row, WAL entry, audit record, OTEL
span attribute, OTEL metric label, log line, or inter-process bus DTO
(`ADR-046-resources-credential` lines 26-55). This is unconditional: every
DTO used on d2b-bus, every audit record, every OTEL export, and every log
line is excluded from carrying secret bytes by construction, not by
best-effort filtering. Violating it is a correctness defect, not a
configuration issue.

**Noise KK is required; NN is forbidden for delivery.** d2b-bus authorizes
the route and forwards opaque Noise records for credential delivery; it
never terminates the delivery channel, decrypts message content, inspects
ciphertext, or stores delivery channel records (lines 398-401).
`Noise_KK_25519_ChaChaPoly_SHA256` is the required profile; `Noise_NN_*` is
explicitly forbidden for this purpose because NN does not authenticate the
responder (lines 405-407; D056). Every delivery session enforces seven
requirements (lines 440-476):

1. Only keys enrolled at credential registration are accepted.
2. Replay protection: a monotonically increasing sequence number per
   consumer; replay closes the session immediately.
3. Output size is bounded by `maxTokenBytes`; exceeding it closes the
   session.
4. Zeroizing buffers (`zeroize::Zeroize`) are mandatory; allocating a
   non-zeroizing type for token bytes is a defect.
5. `Debug` is hand-written and redacted; `#[derive(Debug)]` is forbidden on
   any type touching key material or token bytes.
6. No auto-retry on an ambiguous outcome — a delivery-channel failure or
   missing acknowledgment leaves state unchanged rather than assuming
   success and retrying.
7. Immediate close and zeroize after delivery — no state is retained.

`SignChallenge` signatures use the same sensitive KK delivery channel as
tokens (D068): the outer response carries only outcome metadata, and the
signature bytes themselves remain end-to-end protected and opaque to every
intermediary.

**Interactive vs. unattended credential distinction.** `credential-secret-service`
is user-domain only (interactive desktop secret-service backend);
`credential-managed-identity` is system-domain only (unattended/machine
IMDS-backed managed identity); a Credential resource of the wrong domain for
either type is rejected at spec validation (`ADR-046-resources-credential`
lines 1440-1443, 1563-1567). `credential-entra` may be user- or
system-domain depending on the requesting Process's domain (lines
1671-1674).

**No plaintext-at-rest guarantee is claimed by d2b.** The zero-secret
invariant, zeroizing delivery buffers, and immediate post-delivery zeroize
are the guarantees d2b makes. The backing secret store (OS keyring, IMDS
endpoint, Entra token cache) is managed by the underlying OS/cloud provider
and its at-rest security is out of scope, matching the existing
`SECURITY.md` scoping for upstream components.

**Bytes-in-motion redaction.** Excluded from audit records: token bytes, key
material, password hashes, bearer strings, provider diagnostics, host paths,
connection strings, audience literals, tenant/subscription/client IDs,
endpoint URIs, and Noise/session key material (lines 699-702). Excluded from
OTEL: token bytes, audience literals, provider diagnostics, host paths,
resource IDs, tenant/subscription IDs, endpoint URIs, and any correlation ID
embedding a secret shape (lines 730-732). Error messages are bounded to 240
UTF-8 characters, stripped of control characters, and must not contain
token bytes, URLs, UUIDs, provider diagnostics, host paths, or connection
strings (lines 681-683).

**RBAC.** `use-credential` is required and checked by d2b-bus before
forwarding delivery; `operationClasses` on `CredentialSpec` narrows the
allowed operation set per consumer; `admin-credential` is required to
create/delete a Credential resource, operator-only (lines 318-336).

## Content secrecy: clipboard, terminal, CTAP, notification

A closed set of asset classes is explicitly, per-dossier, "never in any
surface" — never audit, OTEL, Debug output, error messages, or structured
logs — regardless of general redaction rules elsewhere. This table collects
every such class with its exact source quote:

| Sensitive asset | Provider | Explicit "never in any surface" statement | Source |
| --- | --- | --- | --- |
| CTAP payloads, PINs, CBOR assertions, FIDO credential material | `device-security-key` | "No CTAP payloads, PINs, CBOR assertions, or any FIDO credential material appear in any log, audit record, OTEL span attribute, metric label, or error message" | `ADR-046-provider-device-security-key.md` (Invariant I-7) |
| Clipboard content bytes | `clipboard-wayland` | "clipboard bytes only as SCM_RIGHTS attachment FDs — NEVER in method arguments, stream frames, status, audit, or traces" | `ADR-046-provider-clipboard-wayland.md` (Invariant 1) |
| Terminal/PTY bytes, argv, cwd, environment | `shell-terminal` | "No terminal bytes, argv, cwd, environment, paths, PIDs, unit names, usernames, session names, socket paths, or opaque handles may appear in Debug, audit, metrics, or span attributes" | `ADR-046-provider-shell-terminal.md` (SR-8) |
| Notification summary, body, action text, icon ref | `notification-desktop` | "Notification summary, body, action text, and icon ref never appear in any audit field" | `ADR-046-provider-notification-desktop.md` §10.1 |
| TPM NVRAM content | `device-tpm` | "No path, PID, NVRAM content in any audit payload (sensitivity: private)" | `ADR-046-provider-device-tpm.md` |
| IMDS token bytes, IMDS URLs | `credential-managed-identity` | "token bytes, IMDS URLs, and IMDS response fragments excluded from all audit records and OTEL" | `ADR-046-provider-credential-managed-identity.md` |
| Entra token bytes, key material, tenant IDs | `credential-entra` | "No token bytes, key material, tenant IDs, authority URLs, or MSAL cache entries in any audit record" | `ADR-046-provider-credential-entra.md` |
| Secret-service secret bytes | `credential-secret-service` | "Zero-secret-bytes: no secret bytes cross the `Oo7SecretServicePort` boundary" | `ADR-046-provider-credential-secret-service.md` (Invariant 1) |
| PipeWire node IDs, socket paths, gain/volume levels | `audio-pipewire` | "AudioMediator does not export PipeWire node IDs (never in spec/status/d2b-bus/audit/OTEL)"; no level/gain labels | `ADR-046-provider-audio-pipewire.md` (Invariant 8) |
| Compositor socket paths, window titles, app-ids, DnD content | `display-wayland` | "must not contain: compositor socket paths; user or session identities beyond subject_digest; window titles or app-id values; clipboard payloads or DnD content; raw argv; process PIDs, pidfds, or unit names; Wayland protocol message bodies" | `ADR-046-provider-display-wayland.md` |
| Raw busid, USB vendor/product/serial | `device-usbip` | "Raw busid MUST NOT appear in spec, status, audit, OTEL, or `ListBusIds` response" | `ADR-046-provider-device-usbip.md` |
| GPU device node paths | `device-gpu` | "No device path on public wire"; audit excludes raw GPU device paths | `ADR-046-provider-device-gpu.md` |
| hidraw / UHID device path | `device-security-key` | "No path on any public surface (spec, status, audit, OTEL, error messages)" (I-2) | `ADR-046-provider-device-security-key.md` |
| Credential bytes forwarded through OTEL | `observability-otel` | "Credential bytes are never held, routed, or processed by this Provider; all auth material stays inside the transport Provider's scope" | `ADR-046-provider-observability-otel.md` |

**Replay/nonce protections for interactive content actions.** Where content
delivery has an associated user-facing "act on this" callback (notification
action buttons), replay is bounded by a single-use nonce, not by relying on
content secrecy alone: `notification-desktop`'s `ActionNonce` is 256-bit
random, single-use (cleared from its store on first consumption), TTL 120 s,
`MAX_STORE_SIZE = 256`; `InvokeAction` is accepted only on a
`desktop-observer` session (`ADR-046-provider-notification-desktop.md`).

**FD-only delivery, never method arguments.** The universal pattern for
transferring sensitive bytes between a compositor/host process and a
consuming component is an `SCM_RIGHTS`-delivered FD (clipboard, D-Bus
session, CTAP relay) — never a serialized byte payload inside a ttrpc
method argument, resource spec, or stream frame. This both keeps the bytes
out of any place a redaction filter would need to catch them and keeps the
delivery bounded by the same attachment-credit/CLOEXEC machinery as every
other FD (see [LaunchTicket integrity](#launchticket-integrity)).

## Audit vs. OTEL: redaction, cardinality, durability

Telemetry and authoritative audit are two distinct subsystems with **no
shared writer path** (`ADR-046-telemetry-audit-and-support` lines 85-105).
OTEL is best-effort, buffered, lossy, and is never an authorization input;
audit must be committed **before** the operation it describes completes.

**Durability classes.**

| Class | Records | Durability | Rate limiting |
| --- | --- | --- | --- |
| Privileged | `ResourceMutation` (RBAC verbs), `RBACChange`, `SessionConnect` (auth failure), `StateReset` | Durable fsync before the operation completes | Never rate-limited under any load condition |
| Standard | Most resource lifecycle events | Durable within a bounded window | Rate-limited, default `DEFAULT_AUDIT_WRITES_PER_SECOND = 4096` |
| Best-effort | Informational | Async | Dropped under rate limit |

Audit unavailability for a privileged record **fails the operation closed**
(`audit-unavailable`) — this is the concrete recovery control for AC8 that
makes "attack under load, then corrupt state while audit is dropped"
impossible for privileged operations (lines 1141-1165).

**`subject_digest`.** SHA-256 of the normalized canonical subject string
(never the raw name) is what identifies the acting principal in audit
records (lines 1141-1165, 1861).

**Forbidden audit fields** (lines 914-1170): resource name, spec bytes,
status bytes, raw filesystem paths, device identifiers, broker operation
arguments, credential bytes/key material, PIDs/process identities, `argv`,
environment variables.

**`no_isolation` is audit-only** (line 1966; see
[User-domain Host: no isolation](#user-domain-host-no-isolation)): present
in `ProcessEffect` records, forbidden in any metric label, span attribute, or
log field.

**OTEL forbidden metric-label values** (lines 283-303): VM/Zone/Provider/
resource names; user or resource UIDs; Host/Guest/User/Volume/Network/
Device names; filesystem paths, `argv`, environment variables; status detail
messages; subject names; PIDs, pidfds, cgroup paths; operation IDs, endpoint
addresses. `d2b.zone` is allowed in resource attributes but never in a
metric label value. Cardinality is enforced by policy tests
(`policy_observability.rs`, and the new cross-spec
`policy_telemetry_redaction.rs` under `ADR046-telem-008`).

**Audit record types** (lines 914-1170): `ResourceMutation`, `RBACChange`,
`SessionConnect`, `RouteAdmission`, `BrokerEffect`, `ProcessEffect`,
`StateReset`. `ResourceMutation`/`RBACChange` are emitted by the store actor
inside the write transaction before `commit` returns; the sink must durably
fsync before the commit is reported successful (privileged durability class,
line 1861-1862). Zone tree paths in audit records are replaced by opaque
digests (ZR line 2098).

**Segment lifecycle.** Rotation at 64 MiB or UTC midnight, whichever comes
first; 30-day default retention; export requires the admin-only
`audit-export` verb; hash-chain breaks are reported inline in the export
stream, never silently skipped (line 1846).

**`observability-otel` is never a bootstrap dependency and never reads
audit.** Zone startup does not wait for `observability-otel`; unavailability
causes bounded frame drops (`d2b_telemetry_drop_total`), never blocks Zone
startup or a resource mutation (`ADR-046-provider-observability-otel.md`;
D065). The Provider never reads from the authoritative audit sink — strict
separation between OTEL telemetry and authoritative audit is preserved even
though both may describe the same underlying event.

## Lifecycle security: update, revocation, finalizer, restart, adoption, quarantine

**Finalizer/deletion ordering is fixed and never force-clears an undrained
finalizer** (D084). The owning Provider controller drains/deletes its
effects/children and clears its own finalizer; the core resource-store
transaction then writes an event-only `Deleted` revision and removes the
row/index atomically; the audit subsystem appends the deletion record
afterward using a dedup/exactly-once recovery key. Generation-retention
pruning removes only the historical bundle record, never a live resource's
finalizer state; a resource whose finalizers have not drained remains
`Degraded`/blocked until it finishes normally — it is never silently
force-deleted to unblock a generation sweep.

**Generation cleanup never touches controller/API-managed resources.**
Config-owned (`managedBy: configuration`) resources absent from a new bundle
generation receive an async `Delete`; `managedBy: controller` and
`managedBy: api` resources are **never** touched by generation cleanup
(D069). The Zone transitions to `Degraded` while cleanup is pending; if a
cleanup candidate exceeds `cleanupStuckThreshold` (default 5 minutes), a
`GenerationCleanupFailed` condition is set — the runtime never force-removes
finalizers to clear it (`ADR-046-resources-zone-control` lines 2808-2912).
The generation counter itself rejects replay/downgrade: a bundle
`generation` must be strictly greater than
`store_meta.active_configuration_revision` (lines 2753-2754).

**Provider upgrade policies.** `drain-then-replace` (drain the old component
before launching new), `rolling` (phased replacement), and `immediate` (stop
all and replace) are the closed set (RZC lines 645-683); test
`provider-upgrade-drain-then-replace` confirms old-component drain completes
before new-component launch is attempted.

**Trust/conformance failure quarantines, never deletes** (see
[Signed package/manifest/schema/config trust](#signed-packagemanifestschemaconfig-trust-and-publisher-roots)).
Provider deletion itself uses the `core.provider-api-binding` finalizer: all
exported ResourceTypes must be withdrawn, and the controller verifies no
resource of those types remains before the finalizer clears.

**Credential revocation precedes finalizer clearing.** When a Credential
resource has `deletionRequestedAt` set, the Zone runtime revokes the
resolved secret binding **before** clearing the `core.credential-revoke`
finalizer (RZC lines 2808-2821, 3108) — this closes the "delete the
Credential resource but leave the underlying secret-service/IMDS/Entra
binding live" gap.

**Quarantine-not-kill is the universal ambiguous-identity response.** On any
adoption ambiguity — a restarted controller finding a process/Volume/Guest it
cannot unambiguously re-identify — the response is quarantine
(`Degraded`/`Quarantined`, cgroup leaf isolated, no signal sent to the
candidate process), never a broad kill:

- `Provider/system-minijail`: ambiguous adoption → `AdoptionState::Quarantined`
  + `runtime-security-violation` audit; the cgroup leaf is blocked from
  reuse until externally established process-absence proof
  (`ADR-046-provider-system-minijail.md` §22 Invariant 4).
- `Provider/system-systemd`: multiple candidate units matching the adoption
  heuristic → `AdoptionState::Quarantined`; no signal sent; operator must
  confirm absence before reuse.
- `Provider/runtime-cloud-hypervisor`: ambiguous VMM candidate on controller
  restart → `Degraded`; operator must resolve, never an automatic kill.
- `Provider/runtime-azure-container-apps`: ambiguous ACA sandbox state on
  restart → `Degraded`, never re-provisioned without an explicit
  `RuntimeAdopt` confirmation.
- `Provider/volume-local`: a partially removed Volume with a valid marker is
  quarantined rather than silently re-provisioned.

This is the direct v3 continuation of ADR 0034's restart-adoption contract
("D2b re-adopts live runner processes when it can prove identity,
quarantines ... otherwise") and the pidfd non-exportability invariants in
[Process least privilege](#process-least-privilege-controllerserviceworker).

## Availability and DoS controls

Every shared control-plane resource has an explicit, tested ceiling; none is
"reasonably bounded by the client's good behavior."

**ComponentSession/d2b-bus ceilings** (adapted unchanged from historical main
ADR 0045, re-affirmed by `ADR-046-zone-routing`/`ADR-046-cli-and-operations`):

| Resource | Ceiling |
| --- | --- |
| Canonical handshake offer | 16 KiB |
| Logical ttrpc request/response or named-stream message | 1 MiB |
| Active named streams per session | 128 |
| Attachments in one packet / one request / one operation / one session | 32 / 64 / 128 / 256 |
| Process-global / Host-wide transferable attachment credits | 2,048 / 8,192 |
| Reserved non-attachment control FD headroom | 64 |
| Queued plaintext per named stream, each direction | 256 KiB |
| Aggregate queued named-stream plaintext, each direction | 4 MiB |
| Request lifetime (`MAX_REQUEST_LIFETIME_MS`) | 900,000 ms (15 min) |
| Max wall-clock skew accepted on a request | 30 s (never added to remaining duration) |
| Reconnect attempts / window | 10 / 300,000 ms |
| Provider agent in-flight dispatch | 64 (semaphore-guarded) |

**Zone-level ceilings** (`ADR-046-resources-zone-control`): `Quota`
(`enforcementPolicy: hard` rejects with `quota-exceeded`; `soft` sets
`overQuota: true` and warns; `quotaBytes: 0` is rejected outright);
`EmergencyPolicy` (union semantics — the most restrictive enabled flag wins
across `stopNewAdmissions`, `disconnectZoneLinks`, `stopProviderProcesses`,
`drainOngoingOperations`; `drainDeadlineSeconds` bounded 1-300 s;
`stopProviderProcesses` stops Processes **without** setting
`deletionRequestedAt`, so they resume automatically on deactivation rather
than requiring re-creation).

**ZoneLink-specific ceilings**: `maxPendingIntents` (max 1024, default 256),
`maxActiveStreams` (max 128), `localIntentPolicy` (`queue|drop|fail`), route
tree bounds (max 32 hops, max 4096 parent/route entries, max 16 labels per
path), named-stream credit backpressure (source blocks rather than buffers
unboundedly when credit is exhausted).

**Checked arithmetic, never a panic path.** Every admission, fragmentation,
queue-credit, and pre-allocation calculation uses checked arithmetic
including all wire overhead; underflow, overflow, or a value above the
selected profile limit is a typed pre-allocation rejection — it never
reaches indexing, allocation, or a panic (historical main ADR 0045 lines
1514-1523, carried forward unchanged as a `d2b-bus` invariant).

**Priority scheduling under load.** No lock is held across `await`; a
stalled data stream cannot consume reserved control credit. Priority order:
(1) fatal close/revocation/session control, (2) ttrpc control and
cancellation, (3) attachment acknowledgement, (4) named-stream data with
bounded round-robin fairness. This is the mechanism that keeps a
backpressured data path from starving the cancellation path an operator
needs to recover from it.

**Admission observability.** `d2b_api_admission_rejected_total` carries a
closed `reason` label (`auth|quota|conflict|invalid|schema`) so a DoS
attempt is visible in aggregate without leaking which specific resource was
targeted.

## Incident response and support bundle

**`d2b zone doctor`.** Resource status reads only; no resource names, paths,
`argv`, or PIDs in output; includes an audit hash-chain integrity check
(`ADR-046-telemetry-audit-and-support`, `ADR046-doctor-001`).

**`d2b zone support-bundle`.** No spec bytes and no `metadata.name` in the
bundle; metadata and status only. When a Provider is quarantined, the bundle
reports `bundle_completeness: "partial"` rather than silently omitting the
gap or blocking entirely (`ADR046-doctor-002`). This is the concrete tool an
operator or a coordinated-disclosure responder uses to gather evidence
without themselves becoming a redaction bypass — it is bound by exactly the
same audit/status redaction rules as every other read path in
[Audit vs. OTEL](#audit-vs-otel-redaction-cardinality-durability).

**`d2b zone audit export`.** Admin-only (`audit-export` verb); hash-chain
breaks are reported inline in the export stream rather than silently
truncating history; output carries no old field names (`realm`/`node`/
`workload_id`) and no path/`argv` content (`ADR046-audit-004`).

**Coordination with disclosure policy.** `SECURITY.md`'s GitHub Security
Advisory channel, response-time targets (7-day acknowledgment, 30-day
assessment), and scope table are unchanged by this spec and remain the
authoritative disclosure process for the v3 Zone control plane; this section
only adds the operator-facing evidence-gathering commands a responder or
reporter is expected to use while a v3 advisory is investigated.

## Reset boundary

Reset in v3 is Zone/Provider/Host/Guest-scoped, not the whole-host factory
reset the historical main ADR 0045 defined for the v1→v2 cutover. The
`StateReset` audit record is the normative v3 contract
(`ADR-046-telemetry-audit-and-support` §"StateReset", lines 1115-1128):

```json
{
  "record_class": "state-reset",
  "state_reset_fields": {
    "scope":        "zone|provider|host|guest",
    "trigger":      "operator|upgrade|corruption|emergency",
    "generation":   5,
    "prior_digest": "sha256:<hex>",
    "outcome":      "ok|error"
  }
}
```

`StateReset` is a `Privileged` durability-class record: it is durably
fsynced before the reset operation completes, and it is never
rate-limited (§21). A reset that cannot durably record its `StateReset`
event fails closed with `audit-unavailable`, exactly like any other
privileged mutation.

**Why v3's reset boundary is structurally simpler than v2's.** The
historical v2 factory-reset design (main ADR 0045) needed a mandatory
live-session ceremony to delete Secret Service items and revoke TPM exports
*because v2 allowed persisted, host-readable secret material to exist at
all*. v3's [zero-secret invariant](#credential-security-end-to-end-delivery-and-zeroization)
means Credential Providers never persist secret bytes in the resource
store, audit, or telemetry in the first place — the backing secret (OS
keyring entry, IMDS binding, Entra token cache) lives entirely inside the
owning OS/cloud provider, not inside d2b state. A v3 reset therefore does not
need a bespoke pre-reset "delete d2b-owned Secret Service items" ceremony;
it needs the two operations d2b's own resource model already performs on
every deletion:

1. **Credential lease revocation before removal**, exactly as on ordinary
   Credential deletion (see [Lifecycle security](#lifecycle-security-update-revocation-finalizer-restart-adoption-quarantine)):
   any open lease for a Credential in the reset scope is revoked before its
   finalizer clears, so no stale lease survives the reset.
2. **Volume destruction with key-shredding**, exactly as on ordinary Volume
   deletion (see [Volume security](#volume-security)): sealed Volumes in the
   reset scope shred their sealing key material as the first destruction
   step, so no orphaned ciphertext-with-key survives a scope-bounded reset.

**Quiesce before destructive reset.** A `scope: host` or `scope: guest`
reset first drives the affected scope through `EmergencyPolicy`
(`stopNewAdmissions` + `drainOngoingOperations`, §23) so in-flight mutations
finish or are cleanly cancelled rather than being torn out from under a
live write transaction; only after the scope reports no in-flight mutation
does destructive cleanup (finalizer-driven resource deletion, Volume
destruction) proceed.

**No partial reset.** Exactly as with bundle activation (AC5) and RoleBinding
deletion (§8), a reset is atomic at its declared scope: every resource in
scope is either fully torn down (finalizers drained, `Deleted` revision
committed, `StateReset` record fsynced) or the reset fails closed and the
scope remains in its pre-reset state — there is no state where a reset has
"partially happened" and is silently retried later. A resumed reset after a
crash resumes only from the durable `StateReset` record already committed
for that generation; it never treats an in-progress reset as complete
without that record. Volume identity-marker fail-closed semantics (§17) mean
an aborted mid-destruction Volume is quarantined, not silently
re-provisioned or silently treated as already gone, on the next reconcile
after the interrupted reset.

**No host-wide "factory reset boot generation" concept in v3.** Because
reset is Zone/Provider/Host/Guest-scoped rather than whole-host, v3 has no
equivalent of the v2 dedicated reset boot generation, no-startable-daemon
generation, or `d2b host reset --factory` command; the closest analogue for
a full Host wipe is a `scope: host` `StateReset` covering every Zone
resource that resolves to that Host, which is bound by the same finalizer/
quiesce/atomicity rules as any other scope.

## Per-ResourceType threat matrix

The seventeen standard ResourceTypes (D035) each face a distinct primary
threat and are covered by controls already defined above. This table is the
single index a reviewer uses to confirm every ResourceType has at least one
documented threat/control pair; the "Detail" column points to the exact
section with the full control description.

| ResourceType | Primary threat | Prevention/detection/recovery | Detail |
| --- | --- | --- | --- |
| `Zone` | A second `Zone/<name>` masquerading as the Zone's own self resource | Cardinality-1 admission (`resource-already-exists`); `Zone.spec` must be exactly `{}` (`zone-spec-invalid` otherwise) | §26/§8 |
| `ZoneLink` | Child static key substitution / stale child identity reuse | `childStaticKeyFingerprint` re-verified every reconnect; `childZoneUid` mismatch resets cursor to revision 0 | §7/§10 |
| `Provider` | Forged/downgraded/malicious package | Signed `PackageIdentity`, build+runtime signature/conformance verification, quarantine-not-delete | §5 |
| `Role` | Wildcard privilege escalation via operator/Provider-authored Role | Wildcard restricted to core-controller-generated Roles; `provider-wildcard-permission-restricted` | §8 |
| `RoleBinding` | TOCTOU during deletion/re-creation; scope widening via narrowing | Atomic one-transaction deletion; `scopeNarrowing` restriction-only; immutable `roleRef` | §8 |
| `Quota` | Zero-quota bypass; soft-quota silently treated as enforcement | `component-quota-zero` rejection; `enforcementPolicy: hard` rejects over-quota, `soft` only warns | §23 |
| `EmergencyPolicy` | Reason field leaking into telemetry; policy silently deleting Processes instead of pausing | `reason` confined to spec/audit body, never OTEL/status; `stopProviderProcesses` never sets `deletionRequestedAt` | §23 |
| `Host` | Silent isolation-guarantee downgrade for user-domain execution | Non-suppressible three-surface `isolationPosture: "none"` propagation | §16 |
| `Guest` | Compromised Guest escalating via network/device/credential ambient authority | Guest-agent capability confinement to guest netns; ComponentSession KK enrollment; per-attachment Volume/Network/Device scoping | §3 (AC2) |
| `Process` | pidfd/identity confusion enabling signal-based attack on the wrong process | Nine pidfd non-exportability invariants; quarantine-not-kill on ambiguous adoption | §15/§22 |
| `EphemeralProcess` | Terminal-result retention becoming an unbounded secret/output store | Bounded `successfulTtl`/`failedTtl`; forbidden-field list identical to Process | §15 |
| `Volume` | Identity-marker tamper / silent re-provisioning after tamper or deletion race | HMAC identity marker, fail-closed `missing`/`replaced` status, quarantine on partial destruction | §17 |
| `Network` | Firewall/IPv6/east-west drift silently reopening isolation | Dual-point IPv6 suppression enforcement; `hostBlocklist` additive-only; `firewallDigest` drift detection | Cross-cutting network invariants (`ADR-046-resources-network`) |
| `Device` | Blanket device-path grant / cross-consumer device sharing (e.g. security-key + USBIP on one physical device) | No blanket grant; broker-derived node only; explicit mutual-exclusion enforcement (eval-time + runtime) | §13 |
| `User` | Numeric UID/GID leaking into authorization decisions or public surface | `User/<name>` typed refs only; `mappingClass: process-principal-root` never exposes numeric UID/GID publicly | §15 |
| `Credential` | Secret bytes reaching resource store/audit/telemetry, or a non-enrolled consumer reading a token | Zero-secret invariant; Noise KK-only sensitive delivery; exact `consumerRef` match | §9/§19 |
| `Endpoint` | A stable endpoint leaking a raw locator (path/address/CID/port/fd/credential) into spec/status/CLI, or an unauthorized consumer resolving it to a live transport/FD | No raw locator in spec/status (closed transport/locality classes only); Core/ProviderSupervisor resolves via EffectPort/LaunchTicket under authorization; unauthorized resolve denied with a typed error (D092) | §D092 |

## Per-Provider-family threat matrix

The 27 frozen initial Providers (D043-D049) each have a full `Security`
section in their own dossier under `docs/specs/providers/`; this table is
the cross-Provider index, grouped by family, so a reviewer can confirm no
family is missing a documented primary threat/mitigation pair. `EffectPort`
names and forbidden-design citations are given in full in
[Injected EffectPort boundary](#injected-effectport-boundary-and-the-privileged-broker)
and [Forbidden designs](#forbidden-designs).

| Family | Providers | Primary threat class | Key mitigation | Dossier |
| --- | --- | --- | --- | --- |
| System/bootstrap | `system-core`, `system-systemd`, `system-minijail` | Bootstrap-authority widening; unit/PID identity confusion | Compiled non-extensible bootstrap policy (D052); pidfd/InvocationID identity verification; quarantine-not-kill | `provider-system-{core,systemd,minijail}.md` |
| Guest runtime | `runtime-cloud-hypervisor`, `runtime-qemu-media`, `runtime-azure-container-apps`, `runtime-azure-virtual-machine` | Host holding cloud/relay credentials; adoption ambiguity re-provisioning a live workload | Mandatory `executionRef: Guest/*` for cloud-facing components (§11); `Degraded`-not-recreate on ambiguity | `provider-runtime-*.md` |
| Activation | `activation-nixos` | Store-path leakage revealing exact system closure; privilege escalation via `startRoot: true` | Sealed store path never in any public surface; `startRoot: true` paired with `noNewPrivileges: true` + zero host capabilities | `provider-activation-nixos.md` |
| Volume | `volume-local`, `volume-virtiofs` | Host path leakage; cross-VM store leakage; ADR 0021 capability violation | Opaque `sourcePolicyId`; `store-view/live` never real `/nix/store`; zero host capabilities for virtiofsd (conformance-kit tested) | `provider-volume-{local,virtiofs}.md` |
| Network | `network-local` | Internal bridge/tap topology (IfName) leakage; firewall/isolation drift | FNV-1a-hashed IfNames never exposed; dual-point IPv6 suppression; `firewallDigest` drift detection | `provider-network-local.md` |
| Device | `device-tpm`, `device-usbip`, `device-security-key`, `device-gpu` | Device-path leakage; cross-consumer device conflict; TPM re-provisioning as clean device | Broker-derived node only; mutual exclusion (security-key vs. USBIP); TPM fail-closed marker | `provider-device-*.md` |
| Credential | `credential-secret-service`, `credential-entra`, `credential-managed-identity` | Secret bytes crossing a process/session/audit boundary | Zero-secret-bytes port boundary; domain-locked Credential type (user vs. system) | `provider-credential-*.md` |
| Interaction | `display-wayland`, `audio-pipewire`, `clipboard-wayland`, `shell-terminal`, `notification-desktop` | Interactive content (clipboard/terminal/notification) exfiltration via observability surfaces | FD-only content delivery (SCM_RIGHTS); closed content-secrecy table (§20); nonce-bound action replay protection | `provider-{display-wayland,audio-pipewire,clipboard-wayland,shell-terminal,notification-desktop}.md` |
| Transport | `transport-unix`, `transport-vsock`, `transport-azure-relay` | Raw CID/port/socket-path leakage; FD/credential smuggling over a remote transport | Opaque endpoint IDs with no public accessor; structural `attachment_support=false` over ZoneLink transports | `provider-transport-*.md` |
| Observability | `observability-otel` | Credential/isolation-posture leakage into the optional telemetry pipeline; telemetry outage blocking the Zone | Redaction filter drops forbidden attributes before OTLP batching; Provider is never a bootstrap dependency | `provider-observability-otel.md` |

## Forbidden designs

Consolidated from every owning spec/dossier; each is a structural/admission-
time rejection, not an operational recommendation.

**Trust and packaging.**

- Authoring `system-core`/`system-minijail` as an operator-declared Provider
  in Nix (`ADR-046-resources-zone-control` lines 754-777, 2350-2354).
- Any Provider claiming a wildcard permission
  (`resourceNames: ["*"]`/empty `executionRefs`) without being a
  core-controller-generated Role (RZC lines 868-997, line 2985).
- Manifest-derived Provider spec fields set by the operator —
  `spec.exports`, `spec.components`, `spec.dependencies`,
  `spec.permissionClaims`, `spec.upgradePolicy`, `spec.restartPolicy` are
  resolved only from the signed manifest (RZC lines 2722-2723).
- Any Provider process importing a broker service/client/DTO, or receiving a
  broker socket, host path, device node, systemd D-Bus connection, or
  compositor socket by ambient discovery (§13, D077).

**Identity and authorization.**

- A payload supplying or replacing `uid`, `gid`, `pid`, a realm/Zone role, or
  broker authority (§7-§8).
- An established ComponentSession endpoint transferred by `SCM_RIGHTS`,
  pidfd duplication, inheritance, or broker handoff (§7).
- A cross-Zone `ownerRef`, a multi-Zone `CommitBatch`, or a `ResolveRef`
  crossing a Zone boundary (§10).
- A relay/managed-identity/Entra credential mapped to a local Role/Admin
  (§11).
- Cross-Zone credential minting — no parent-minted token delivered to a
  child Zone (§11).

**Process and sandbox.**

- `Type=forking` for any `Provider/system-systemd`-managed Process
  (§15).
- Treating a systemd unit name alone as process identity (§15).
- A Process ResourceSpec containing raw UID/GID, a raw cgroup path, a raw
  socket address/host path, raw seccomp BPF, a raw capability bitmask, raw
  minijail arguments, raw systemd unit properties, a raw broker operation
  name/parameter, or a credential/secret/token in an environment variable
  (§15).
- Pidfd serialized, stored, sent over d2b-bus, exposed in status, or used to
  infer identity across a restart (§15).
- A `LaunchTicket` executed after its compiled sandbox plan digest has
  changed since issue (§15).
- virtiofsd spawned with any non-empty host capability set, `startRoot:
  true`, `--sandbox=namespace`, or an `extra_args` field (§17,
  `ADR-046-provider-volume-virtiofs.md`).
- A broad kill of an entire cgroup/process group on ambiguous adoption,
  instead of quarantine (§22).

**Volume and state.**

- A raw host path in any Volume spec field; only `sourcePolicyId` is
  permitted (§17).
- `ephemeral`, `cache`, or `config` `persistenceClass` for a component state
  Volume (§17, `component-persistence-class-forbidden`).
- `host-backed-guest` placement for a Credential/audit/remote-node/
  cloud-control schema Volume (§18, `guest-local-required`).
- `quotaBytes: 0` on any persistent Volume (§17,
  `component-quota-zero`).
- Silent re-provisioning of a Volume after an identity-marker
  `missing`/`replaced` failure, or of a partially destroyed Volume (§17).
- virtiofsd serving the host's actual `/nix/store` instead of
  `store-view/live` (§17).

**Credential.**

- `Noise_NN_*` for sensitive credential/`SignChallenge` delivery — `Kk` is
  required (§19, D056).
- `#[derive(Debug)]` on any type touching key material or token bytes (§19).
- Auto-retry on an ambiguous credential-delivery outcome (§19).
- An inline secret byte value anywhere a `credentialRef: true` field is
  declared (§5, `inline-secret-in-settings`/`credential-value-must-be-ref`).
- Any `$`-prefixed config key other than `$credentialRef` (§5).

**Networking.**

- Removing or conditioning `lib.mkForce` on the net-VM's `10-eth-dhcp`
  neutralizer (carried forward unchanged from the pre-v3 baseline,
  `AGENTS.md` "Don'ts (security-relevant)").
- Removing IPv6 suppression from either the bridge-creation path or the
  periodic reconcile path — both are required together
  (`ADR-046-resources-network` INV-NET-002).
- Reducing or emptying `hostBlocklist` — additive-only (INV-NET-004).
- A workload VM name, user identifier, DHCP hostname, or workload label in a
  host nftables rule (INV-NET-006).
- `CAP_NET_ADMIN`/`BIND_SERVICE`/`RAW` present in the host network namespace
  effective capability set for a guest-agent Process (INV-NET-009).

**ZoneLink and transport.**

- An FD, credential byte, or host/socket/device/store path in any
  ZoneLink-forwarded frame (§10).
- `attachmentsEnabled: true` for any ZoneLink transport, or
  `socketKind: stream` with attachments enabled (§10, transport-unix/vsock
  dossiers).
- A Transport Provider calling raw transport syscalls itself (`AF_VSOCK`,
  raw `socket(2)`, etc.) instead of going through the core effect adapter
  (§10).
- A Transport Provider owning ZoneLink state directly instead of returning
  opaque handles to the core ZoneLink handler (§10, D081).

**Telemetry and audit.**

- `no_isolation`, a raw path, `argv`, a PID/pidfd/cgroup path, a resource
  name, or a credential/token appearing as an OTEL metric label, span
  attribute, or structured log field (§21).
- `observability-otel` reading the authoritative audit sink, or blocking
  Zone startup (§21).
- Any interactive content class from the [content-secrecy table](#content-secrecy-clipboard-terminal-ctap-notification)
  appearing in audit, OTEL, Debug output, or an error message (§20).

**Lifecycle.**

- Force-removing an undrained finalizer to unblock a generation sweep or a
  reset (§22, §25).
- Generation cleanup deleting a `managedBy: controller` or `managedBy: api`
  resource merely because it is absent from a new Nix bundle (§22, D069).
- A reset proceeding without a durably committed `StateReset` audit record,
  or resuming an interrupted reset without re-checking that record (§25).

## Residual risks and explicit non-goals

These are documented, accepted limitations — not gaps this spec claims to
close. Each maps to the attacker class it is scoped against.

1. **Compromised Host root, or any process sharing a UID with a d2b
   component, is not contained (AC3).** This matches `SECURITY.md`'s
   existing "does NOT defend against ... multi-user trust on a single host."
   A user-domain `Host` with `isolationPosture: "none"` makes this residual
   risk explicit and non-suppressible (§16) rather than eliminating it —
   elimination would require a VM/sandbox boundary, which is precisely what
   `isolationPosture: "none"` declares is absent.
2. **A fully compromised gateway Guest can still exhaust or misuse its own
   Zone's relay/cloud reachability.** Gateway custody (§11) bounds the blast
   radius to that Guest and its Zone — it does not claim the Guest itself is
   unbreachable. A compromised gateway Guest cannot escalate to Host root,
   the privileged broker, sibling Zones, or unrelated Host-resident
   workloads, matching ADR 0032's "protection direction" framing.
3. **TPM sealing binds TPM, host, user, and credential name — it does not
   cryptographically bind a specific systemd unit or Process identity.**
   PID1/broker-assigned identity (DAC, mount namespace, sandbox
   configuration), not TPM cryptography, is what isolates the receiving
   unit. This is an accepted limitation of TPM2 sealed-credential semantics,
   not a d2b defect.
4. **Physical attacks, CPU side channels, and upstream supply-chain
   compromise remain out of scope**, unchanged from `SECURITY.md`: disk
   encryption + TPM-bound unlock is a Lanzaboote concern; SMT/cache
   side-channels are a hardware-level concern; `nixpkgs`/Nix-store
   supply-chain attacks upstream of the pinned artifact catalog are deferred
   to upstream Nix/nixpkgs.
5. **A relay provider (Azure Relay or equivalent) sees connection metadata
   and traffic shape even though it never sees plaintext operations.**
   ZoneLink/ComponentSession confidentiality (§7, §10) protects payload
   content and identity, not the existence/timing/volume of cross-Zone
   traffic. Traffic analysis by the relay operator is an accepted residual
   risk, matching the existing ADR 0032 relay-trust framing.
6. **`observability-otel` telemetry loss under exporter outage is accepted,
   not prevented.** The design explicitly chooses availability of the Zone
   over completeness of telemetry (§21); a determined attacker able to
   induce sustained exporter outage can degrade observability (never
   authorization or audit) during that window.
7. **Non-goal: this spec does not define a new cryptographic primitive,
   key-exchange protocol, or audit storage engine.** It normatively
   references the Noise profiles, redb transaction model, and JSONL
   hash-chain format already defined by the owning specs; any future change
   to those primitives is scoped to the owning spec, not this one.
8. **Non-goal: whole-host physical/hardware attestation.** `guest-controller-host`-class
   attestation for confidential computing or hardware-backed vTPM is
   explicitly future work requiring its own ADR/spec and conformance
   contract before any Provider may advertise it (historical main ADR 0045
   invariant, carried forward as a residual non-goal since no v3 Guest
   runtime Provider claims this capability at this baseline).

## Current-code fit

| Item | Treatment |
| --- | --- |
| v3 current anchor | `packages/d2b-priv-broker/src/{sys.rs,ops/*}` (broker sole-executor, cgroup/namespace pre-establishment); `packages/d2b-core/src/{storage,processes,privileges,minijail_profile}.rs` (typed process DAG/minijail profiles/broker effects); `packages/d2b-realm-core/src/workload.rs` (`IsolationPosture::UnsafeLocal`); `packages/d2bd/src/{exec_session.rs,realm_access_resolver.rs}`; `nixos-modules/{assertions.nix,net.nix,manifest.nix,bundle-artifacts.nix}`; `SECURITY.md` (disclosure policy, v1/v2 trust-boundary deltas) |
| v3 evidence class | Mixed. Broker sole-executor, cgroup/namespace pre-establishment, TPM/USBIP device hardening, and Nix eval-time assertions are `implemented-and-reachable`. ComponentSession, native RBAC, Zone resource store, every standard ResourceType, and every frozen Provider are `ADR-only` (no `Provider/*` crate exists in the protected v3 baseline; see `ADR-046-current-code-migration-map.md` §8.3 disposition table) |
| Main reuse source | main `a1cc0b2d`: `d2b-session`/`d2b-session-unix` (ComponentSession Noise/record/attachment machinery), `d2b-contracts/src/{public_wire.rs,provider_registry_v2.rs}` (typed RPC/registry shape), `d2b-priv-broker/src/ops/{swtpm_dir.rs,storage_contract.rs}` (fail-closed marker/quarantine pattern) |
| Behavior retained | Broker-as-sole-privileged-executor; fail-closed typed errors; pidfd/InvocationID adoption identity; positive-capability provider traits; argv/secret/path redaction discipline; OTEL/audit architectural separation; quarantine-not-kill on ambiguous adoption (ADR 0034 continuation) |
| Required delta | Native Zone resource plane/redb store; ComponentSession production wiring with native RBAC; Provider packaging/signing/trust/quarantine; primitive ResourceSpecs (Host/Guest/Process/Volume/Network/Device/User/Credential); ZoneLink/d2b-bus routing; per-Provider EffectPort boundary; StateReset audit contract |
| Excluded assumptions | Historical main ADR 0045's Realm/gateway-VM-as-ResourceKind model, per-realm PID1 broker sockets, long-lived guest-control HMAC token, and whole-host factory-reset generation are not v3 architecture — D050 (Guest replaces gateway-VM-as-special-realm), D016/D017 (Zone/ZoneLink replace Realm), and this spec's own [reset boundary](#reset-boundary) supersede them for v3 |
| Feasibility proof | Per-spec hermetic/Nix-eval/container/host-integration/fuzz suites named in [Implementation work items](#implementation-work-items) below and in every owning spec's own work-item validation column |
| Future owner | `ADR046-security-*` work items below own cross-cutting security validation; each ResourceType/Provider spec's own `ADR046-*` work items own the implementation itself |

## Implementation work items

### ADR046-security-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-001` |
| Dependency/owner | All ResourceType/Provider specs; owned by the security/telemetry integrator alongside `ADR046-telem-008` |
| Current source | `packages/d2b-contract-tests/tests/policy_observability.rs` (existing v3 cardinality/label policy gate) |
| Reuse source | None (new cross-cutting gate; no equivalent exists in main) |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-contract-tests/tests/policy_telemetry_redaction.rs` |
| Detailed design | One policy test enumerating every forbidden metric-label/audit-field value from §21 and the content-secrecy table in §20 (store paths, `no_isolation`, credential bytes, raw paths/argv/PID/cgroup, CTAP/clipboard/terminal/notification content) and asserting, by static scan of instrumentation call sites plus a redaction-guard runtime test, that no `ADR046-*` Provider crate emits any of them |
| Integration | Runs as part of `make test-lint`/`make test-rust`; every Provider crate's own redaction test (e.g. `tests/stream_redaction.rs`) is a per-Provider instance of the same closed list |
| Data migration | None |
| Validation | Hermetic (`cargo test -p d2b-contract-tests policy_telemetry_redaction`); fails the build if a new Provider crate is added without a corresponding redaction test file under its `tests/` |
| Removal proof | Not applicable — this is a permanent gate, not a migration |

### ADR046-security-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-002` |
| Dependency/owner | `ADR046-session-001`/`ADR046-bus-001` (ComponentSession/d2b-bus implementation) |
| Current source | main `a1cc0b2d`: `d2b-session/tests/noise_vectors.rs`, `d2b-session/tests/component_session.rs`, `d2b-session-unix/tests/unix_session.rs` |
| Reuse source | Same main commit/paths |
| Reuse action | copy and adapt |
| Destination | `packages/d2b-session/tests/noise_conformance.rs`, `packages/d2b-session/fuzz/fuzz_targets/{handshake_offer,record_frame}.rs` |
| Detailed design | Property/fuzz test suite over the three Noise profiles (§7): exact NN/KK/IKpsk2 vectors and rejection mutations (copied), plus new `cargo-fuzz` targets mutating the canonical handshake offer, preface, and encrypted record frame to assert no panic/UB and that every malformed input is a typed rejection (never a partial accept) |
| Integration | Wired into `make test-rust` (vectors) and a separate `make test-fuzz` target (new; time-boxed nightly run, not part of the PR-blocking gate) |
| Data migration | None |
| Validation | Hermetic vector tests plus fuzz corpus with a minimum 4-hour nightly run and zero crashes/hangs as acceptance |
| Removal proof | Not applicable |

### ADR046-security-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-003` |
| Dependency/owner | `ADR046-zone-control-004`/`ADR046-zone-control-005` (Role/RoleBinding implementation) |
| Current source | `packages/d2bd/src/admission.rs` (`verb_requires_admin()` baseline verb table) |
| Reuse source | None beyond the verb-table adaptation already tracked by `ADR046-zone-control-004` |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-resource-store/tests/rbac_property.rs` |
| Detailed design | Property test asserting, for a randomly generated Role/RoleBinding/request corpus: (1) no request whose payload sets a subject/role field ever changes the resolved `AuthenticatedSubjectContext.subjectRef`; (2) no non-core Role with a wildcard grant is ever admitted; (3) `scopeNarrowing` never widens beyond the referenced Role; (4) RoleBinding deletion never leaves an observable intermediate state under concurrent readers |
| Integration | Runs against the real redb-backed resource store test harness, not a mock |
| Data migration | None |
| Validation | Hermetic property test (`proptest`/`quickcheck`-style, minimum 10,000 cases per property) |
| Removal proof | Not applicable |

### ADR046-security-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-004` |
| Dependency/owner | `ADR046-routing-005`/`ADR046-bus-001` (ZoneLink/d2b-bus relay implementation) |
| Current source | None (v3 ZoneLink relay is `ADR-only`) |
| Reuse source | None |
| Reuse action | extract and adapt (design copied from `ADR-046-zone-routing.md` structural-rejection sections) |
| Destination | `packages/d2b-bus/fuzz/fuzz_targets/zonelink_frame.rs`, `packages/d2b-bus/tests/zonelink_structural_rejection.rs` |
| Detailed design | Fuzz + property suite asserting that no mutation of a ZoneLink-bound frame (attachment count, credential-shaped byte runs, path-shaped strings, PID-shaped integers) is ever forwarded — every such mutation is rejected at serialization with `attachment-not-permitted-over-zone-link` or the transport-specific equivalent, never silently dropped or partially forwarded |
| Integration | `make test-fuzz`; a companion container test (`tests/integration/containers/zonelink-cross-zone.rs`) runs two real Zone runtime containers connected by a real ZoneLink and asserts the same property end to end over the wire, not just in the frame-serialization unit |
| Data migration | None |
| Validation | Fuzz corpus (`cargo fuzz run zonelink_frame -- -runs=1000000`, zero crashes); container test passes in `make test-integration` |
| Removal proof | Not applicable |

### ADR046-security-005

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-005` |
| Dependency/owner | Every Provider dossier's own crate; owned by the Provider packaging integrator (`ADR-046-provider-model-and-packaging`) |
| Current source | None (compile-time dependency audit does not exist yet; per-Provider "imports no broker DTO" claims are currently prose-only in each dossier) |
| Reuse source | `cargo-deny`/`cargo tree`-style dependency graph tooling already used by `nix flake check`'s `rust-deny`/`rust-audit` derivations (`AGENTS.md` "Disk hygiene contract") |
| Reuse action | adapt |
| Destination | `packages/xtask/src/effectport_boundary_check.rs`, wired into `make test-policy` |
| Detailed design | For every crate under `packages/d2b-provider-*`, walk its `Cargo.toml` dependency graph and fail the build if it transitively depends on `d2b-priv-broker` or any crate exposing a raw broker client/DTO type; separately, grep-scan for direct syscalls forbidden per dossier (e.g. `socket(AF_VSOCK` in `transport-vsock`, `Command::new("systemctl"` in `system-systemd`) |
| Integration | `make test-policy`; blocks any PR adding a forbidden dependency edge or forbidden syscall string to a Provider crate |
| Data migration | None |
| Validation | Hermetic (`cargo xtask effectport-boundary-check`); a negative test intentionally adds a forbidden dependency to a scratch crate and asserts the check fails |
| Removal proof | Not applicable — permanent gate |

### ADR046-security-006

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-006` |
| Dependency/owner | `ADR046-minijail-002`/`ADR046-minijail-003` (LaunchTicket/EffectPort implementation) |
| Current source | `packages/d2b-priv-broker/src/sys.rs` (`clone3_spawn_runner` user-namespace path); `packages/d2b-host/src/virtiofsd_argv.rs` |
| Reuse source | Same v3 paths (already `implemented-and-reachable` per the migration map) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-system-minijail/tests/launchticket_toctou.rs` |
| Detailed design | Fault-injection test that issues a `LaunchTicket`, then mutates the referenced `CompiledSandboxPlan` digest (simulating a race between issue and exec) before the broker execs, and asserts the spawn fails closed rather than launching with the old plan; a companion test kills the broker mid-`clone3` and asserts no half-initialized process (missing cgroup placement, non-zero host capabilities) is ever observable by a concurrent reader |
| Integration | `make test-rust` (unit-level fault injection via a fake clock/fault-injecting `EffectPort` test double); a host/KVM integration test (`tests/host-integration/launchticket-toctou.nix`) repeats the same scenario against the real broker and real `clone3(2)` |
| Data migration | None |
| Validation | Hermetic fault-injection test plus `make test-host-integration` NixOS/KVM test; acceptance is zero observable non-zero-capability or missing-cgroup-placement windows across 10,000 injected-fault iterations |
| Removal proof | Not applicable |

### ADR046-security-007

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-007` |
| Dependency/owner | `ADR046-minijail-005`, `ADR046-systemd-002`, `ADR046-aca-001`, `ADR046-volume-001` (every quarantine-on-ambiguity implementation) |
| Current source | `packages/d2b-priv-broker/src/ops/swtpm_dir.rs` (existing fail-closed marker/quarantine pattern, `implemented-and-reachable`) |
| Reuse source | Same v3 path, generalized |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-contract-tests/tests/quarantine_not_kill_matrix.rs` |
| Detailed design | One parameterized fault-injection matrix test, run once per adoption-capable Provider (`system-minijail`, `system-systemd`, `runtime-cloud-hypervisor`, `runtime-azure-container-apps`, `volume-local`), that restarts the controller with a deliberately ambiguous adoption candidate (duplicate InvocationID, mismatched marker inode, stale ACA operation handle) and asserts: (a) the resource transitions to `Degraded`/`Quarantined`, never `Deleted` or silently re-adopted; (b) no signal is sent to the ambiguous candidate process; (c) a `runtime-security-violation`-class audit record is emitted |
| Integration | `make test-rust` for the in-process cases; `make test-host-integration` for the real-pidfd/real-cgroup cases (`tests/host-integration/quarantine-not-kill.nix`) |
| Data migration | None |
| Validation | Matrix covers all five Providers listed; acceptance is 100% pass across all five with no signal sent to the ambiguous candidate in any case |
| Removal proof | Not applicable |

### ADR046-security-008

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-008` |
| Dependency/owner | `ADR046-exec-002` (Host `isolationPosture` propagation) |
| Current source | `packages/d2b-contracts/src/public_wire.rs:267` (`WorkloadPublicSummary.execution_posture`, `implemented-and-reachable`); `packages/d2bd/src/unsafe_local_helper.rs` (`HelperRegistry::dispatch_launch`, current gap: does not emit a `ProcessEffect`-class event) |
| Reuse source | Same v3 paths |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-system-core/tests/no_isolation_propagation.rs` |
| Detailed design | Integration test that creates a user-only `Host`, launches a Process on it, and asserts all three non-suppressible surfaces simultaneously: `status.isolationPosture == "none"` in JSON output; the unconditional stderr warning string is present with `--json` and non-JSON CLI invocation and cannot be suppressed by any combination of flags; the corresponding `ProcessEffect` audit record carries `no_isolation: true`; and a companion negative assertion that `no_isolation` never appears in any OTEL metric/span emitted during the same test run |
| Integration | `make test-rust` (CLI/status/audit assertions) plus a Nix eval test (`tests/unit/nix/cases/no-isolation-null-posture-rejected.nix`) asserting the D042/D067 bidirectional-rejection eval assertions fire |
| Data migration | None (closes the current `HelperRegistry::dispatch_launch` audit gap noted in `ADR-046-telemetry-audit-and-support.md`) |
| Validation | Hermetic CLI/audit integration test; Nix eval test; acceptance is zero code paths reaching a live user-only-Host Process without all three surfaces firing |
| Removal proof | Legacy `d2b-unsafe-local-helper` v2-protocol warning path removed only after this test passes against the v3 replacement and the legacy crate has no remaining callers |

### ADR046-security-009

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-009` |
| Dependency/owner | `ADR046-pstate-003` (Volume identity marker) |
| Current source | `tests/unit/nix/cases/per-vm-state-ownership.nix` (existing v3 ownership test, adapted target) |
| Reuse source | None new |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-volume-local/tests/marker_tamper_fault_injection.rs` |
| Detailed design | Fault-injection test that provisions a Volume, then out-of-band (as a simulated attacker with filesystem access) replaces the marker file, swaps the backing directory for a different inode on the same `st_dev`, and deletes the marker entirely — three separate scenarios — and asserts each transitions the Volume to `Failed` with `markerStatus: missing`/`replaced` respectively, never a silent re-provision, and that operator-only remediation is the only recovery path exercised |
| Integration | `make test-rust`; a host-integration variant (`tests/host-integration/volume-marker-tamper.nix`) repeats the inode-swap scenario against the real broker-maintained marker root on a real filesystem |
| Data migration | None |
| Validation | Hermetic + host/KVM fault-injection test; acceptance is 100% fail-closed across all three tamper scenarios |
| Removal proof | Not applicable |

### ADR046-security-010

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-010` |
| Dependency/owner | `ADR046-credential-001` through `ADR046-credential-007` |
| Current source | None (zero-secret invariant has no existing automated gate) |
| Reuse source | None |
| Reuse action | extract and adapt (design from `ADR-046-resources-credential.md` §1.1) |
| Destination | `packages/d2b-contract-tests/tests/zero_secret_invariant.rs` |
| Detailed design | Static + dynamic gate: (1) static — every DTO type reachable from a `Credential`-adjacent module must implement a hand-written redacted `Debug` and must not derive `Debug`, enforced by a `#[forbid(clippy::derive_debug_ambient)]`-style custom lint or an `xtask` AST scan; (2) dynamic — a property test that generates random `Credential` delivery sessions and asserts the delivered token/`SignChallenge` byte sequence never appears, byte-for-byte, in any captured audit record, OTEL span, log line, or resource-store row taken during the same test run |
| Integration | `make test-lint` (static scan) and `make test-rust` (dynamic property test) |
| Data migration | None |
| Validation | Hermetic; the dynamic test additionally runs as a canary-byte test (a unique random marker is embedded in the token and searched for across every observability surface) |
| Removal proof | Not applicable |

### ADR046-security-011

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-011` |
| Dependency/owner | `ADR046-notify-004`, clipboard/terminal/security-key Provider work items |
| Current source | `packages/d2b-unsafe-local-helper/src/{shell_runtime,shell_supervisor}.rs` (existing terminal-byte redaction discipline, `implemented-and-reachable`, adapted target) |
| Reuse source | Same v3 paths |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-{clipboard-wayland,shell-terminal,device-security-key,notification-desktop}/tests/stream_redaction.rs` (one per Provider, same shared test helper crate) |
| Detailed design | Shared canary-byte test helper: each Provider's test injects a unique random marker into its sensitive content path (clipboard bytes, terminal output, CTAP payload, notification body) and asserts the marker never appears in audit, OTEL, Debug output, or CLI error text captured during the test |
| Integration | `make test-rust`; a container integration test (`tests/integration/containers/content-secrecy.rs`) runs a real Wayland/D-Bus mock session end to end for the clipboard/notification cases |
| Data migration | None |
| Validation | Hermetic canary-byte test per Provider (4 Providers, shared helper crate); container test for the two D-Bus/Wayland-mediated cases |
| Removal proof | Not applicable |

### ADR046-security-012

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-012` |
| Dependency/owner | `ADR046-audit-002` (privileged audit durability) |
| Current source | `packages/d2bd/src/daemon_audit.rs` (existing audit-write path, adapted target) |
| Reuse source | Same v3 path |
| Reuse action | adapt |
| Destination | `packages/d2b-audit/tests/privileged_fail_closed.rs` |
| Detailed design | Fault-injection test that makes the audit sink's fsync fail (simulated ENOSPC/EIO) during a privileged `ResourceMutation`/`RBACChange`/`StateReset` write, and asserts the originating operation itself fails with `audit-unavailable` rather than completing with a lost audit record; a companion test floods `Standard`/`Best-effort` records past `DEFAULT_AUDIT_WRITES_PER_SECOND` and asserts privileged records are never dropped or delayed by the resulting backpressure |
| Integration | `make test-rust` |
| Data migration | None |
| Validation | Hermetic fault-injection test; acceptance is zero privileged-class operations that complete despite a failed durable audit write |
| Removal proof | Not applicable |

### ADR046-security-013

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-013` |
| Dependency/owner | `ADR046-zone-control-009`/`ADR046-zone-control-010` (Quota/EmergencyPolicy), `ADR046-bus-003` (session limits) |
| Current source | None new; ceiling values are already enumerated in `ADR-046-zone-routing.md`/`ADR-046-cli-and-operations.md` |
| Reuse source | main `a1cc0b2d`: `d2b-session` credit-accounting/priority-scheduling tests, copied for the checked-arithmetic/priority-ordering assertions |
| Reuse action | copy and adapt |
| Destination | `packages/d2b-bus/tests/dos_ceiling_fault_injection.rs` |
| Detailed design | Fault-injection/load test suite: (1) attachment-credit exhaustion at each of the six scopes (Packet/Request/Operation/Session/Process/Host), asserting typed rejection never a panic; (2) reconnect-storm exceeding `MAX_RECONNECT_ATTEMPTS`/`MAX_RECONNECT_WINDOW_MS`, asserting the session fails closed rather than looping; (3) ZoneLink hop-count/route-advertisement replay flood, asserting `hop-limit-exceeded`/`zone-advertisement-replay` rather than unbounded forwarding; (4) a stalled data stream under load, asserting control/cancellation traffic is never starved (priority-scheduling property) |
| Integration | `make test-rust`; item (4) additionally runs as a container load test (`tests/integration/containers/backpressure-priority.rs`) with a real slow consumer |
| Data migration | None |
| Validation | Hermetic fault-injection suite; container load test; acceptance is zero panics/unbounded-growth across all four scenarios |
| Removal proof | Not applicable |

### ADR046-security-014

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-014` |
| Dependency/owner | `ADR046-doctor-001`/`ADR046-doctor-002` (doctor/support-bundle commands) |
| Current source | None (both commands are `ADR-only`) |
| Reuse source | None |
| Reuse action | extract and adapt (design from `ADR-046-telemetry-audit-and-support.md`) |
| Destination | `packages/d2b/src/commands/{doctor,support_bundle}.rs` |
| Detailed design | `d2b zone doctor` performs read-only status/audit-hash-chain checks with the redaction rules from §21 enforced on every field it prints; `d2b zone support-bundle` assembles a bounded archive of metadata+status (never spec bytes or `metadata.name`) and sets `bundle_completeness: "partial"` when any Provider in scope is quarantined, rather than omitting the gap silently |
| Integration | `make test-rust` (CLI integration tests); a container test (`tests/integration/containers/support-bundle-quarantined.rs`) runs a real Zone with one quarantined Provider and asserts the bundle correctly reports `partial` |
| Data migration | None |
| Validation | Hermetic CLI test asserting no spec byte or `metadata.name` appears in a generated bundle; container test for the quarantined-Provider case |
| Removal proof | Not applicable |

### ADR046-security-015

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-015` |
| Dependency/owner | `ADR046-audit-001` (`StateReset` record), Volume/Credential lifecycle work items |
| Current source | Historical main ADR 0045 factory-reset design (`a1cc0b2d^:docs/adr/0045-provider-and-transport-framework.md`, reset process overview and apply-command verification steps) — reused only as a design precedent for atomicity/fail-closed sequencing, not as v3 architecture (see [Reset boundary](#reset-boundary) for the explicitly excluded assumptions) |
| Reuse source | Same historical commit, sequencing pattern only (no code reuse; historical implementation was bash/systemd-generation-based and does not exist in any Rust crate) |
| Reuse action | adapt (pattern only) |
| Destination | `packages/d2b-core-controller/src/reset.rs`, `packages/d2b-core-controller/tests/reset_atomicity.rs` |
| Detailed design | Implements the `scope` (`zone`, `provider`, `host`, or `guest`) `StateReset` flow from §25: quiesce via `EmergencyPolicy`, revoke open Credential leases in scope, destroy Volumes in scope (key-shred first), commit the `StateReset` audit record durably, and only then report the reset complete. A crash-recovery path re-derives "was this reset already committed?" solely from the durable `StateReset` record, never from partial filesystem state |
| Integration | `make test-rust` (unit-level state machine); a host/KVM integration test (`tests/host-integration/reset-atomicity.nix`) kills the process mid-reset at each of the four phases (quiesce, credential revoke, Volume destroy, audit commit) and asserts recovery never double-destroys, never silently completes without the audit record, and never leaves an orphaned sealed-Volume-without-key state |
| Data migration | None (v3-native; no v1/v2 reset-generation state to migrate) |
| Validation | Hermetic state-machine test; host/KVM crash-injection test at all four phases; acceptance is zero non-atomic outcomes across all injected crash points |
| Removal proof | Not applicable |

### ADR046-security-016

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-016` |
| Dependency/owner | Documentation/CI integrator; depends on every Provider dossier's own `Security` section existing |
| Current source | None (cross-reference check does not exist) |
| Reuse source | None |
| Reuse action | adapt (pattern from `tests/unit/gates/drift-check.sh`) |
| Destination | `tests/unit/gates/security-matrix-coverage.sh` |
| Detailed design | A drift-style gate that parses [Per-ResourceType threat matrix](#per-resourcetype-threat-matrix) and [Per-Provider-family threat matrix](#per-provider-family-threat-matrix), confirms every one of the 17 standard ResourceTypes and all 27 Provider dossiers under `docs/specs/providers/` has a row, and confirms every referenced dossier file actually contains a `## Security`-class section (by heading grep) — failing the gate if a new ResourceType/Provider is added without a corresponding row and dossier section |
| Integration | `make test-drift` |
| Data migration | None |
| Validation | Hermetic shell-script gate; a negative test adds a scratch Provider dossier missing a Security section and asserts the gate fails |
| Removal proof | Not applicable — permanent gate |

### ADR046-security-017

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-017` |
| Dependency/owner | `ADR046-routing-004`, gateway-custody Provider work items (`ADR046-aca-*`, `ADR046-azure-vm-*`, `ADR046-transport-relay-*`) |
| Current source | None |
| Reuse source | None |
| Reuse action | extract and adapt |
| Destination | `tests/integration/containers/malicious-child-zone.rs` |
| Detailed design | Container-based penetration test running a real parent Zone and a deliberately malicious child Zone container that attempts, over a real ZoneLink: FD smuggling, credential-shaped byte injection, cross-Zone `ownerRef` forgery, capability-ceiling widening claims, and route-advertisement replay. Every attempt must be rejected by the parent with the specific typed error named in §10, and none may reach the parent's resource store, Credential state, or Host substrate |
| Integration | `make test-integration` (requires podman, per `AGENTS.md` "Local Layer 1 + container integration") |
| Data migration | None |
| Validation | Container integration test; acceptance is zero successful attacks across all five attempted vectors |
| Removal proof | Not applicable |

### ADR046-security-018

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-security-018` |
| Dependency/owner | Manual validation owner; depends on `ADR046-azure-vm-*`, `ADR046-transport-relay-*`, `ADR046-aca-*`, device Provider work items reaching a testable state |
| Current source | None (manual cloud/hardware validation has no automated equivalent by design) |
| Reuse source | None |
| Reuse action | adapt (checklist pattern from `SECURITY.md`'s existing portability-roadmap manual milestones and `tests/README.md`'s manual hardware tier) |
| Destination | `docs/reference/security-manual-validation-checklist.md` (new reference doc, out of scope for this spec's own file but named here as the required destination for the future implementation PR) |
| Detailed design | A checklist covering the scenarios that cannot be hermetically or even container-tested: (1) real Azure Container Apps/Azure VM credential rotation and revocation under `AzureEffectPort`, confirming zeroization on a real managed-identity/Entra token; (2) real TPM 2.0 hardware NVRAM persistence/tamper-marker behavior across a real host reboot; (3) real USBIP/security-key hardware mutual-exclusion enforcement with a physical FIDO2 device; (4) real Azure Relay listener/sender credential acquisition and relay-identity-not-local-auth verification against a live relay namespace |
| Integration | Run manually before each tagged release touching a cloud/hardware Provider, per the existing `tests/README.md` manual-tier convention |
| Data migration | None |
| Validation | Checklist sign-off recorded in the release's validation evidence, not a CI gate (matches `D2b_LIVE=1` manual-tier precedent in `AGENTS.md`) |
| Removal proof | Not applicable |
