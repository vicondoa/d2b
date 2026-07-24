# ADR 0046 Provider dossier: transport-azure-relay

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-transport-azure-relay` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `packages/d2b-provider-transport-azure-relay/` |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-componentsession-and-bus`, `ADR-046-zone-routing`, `ADR-046-resources-credential`, `ADR-046-provider-model-and-packaging`, `ADR-046-telemetry-audit-and-support`, `ADR-046-nix-configuration` |
| Supersedes | `d2b-provider-relay` gateway-display relay path (`AcaWorkloadProvider` + `RelayProvider` traits in `d2b-realm-provider`); `d2b-gateway-runtime/src/bin/d2b-gateway-relay.rs`; `packages/d2b-provider-relay/src/lib.rs` as a first-party transport surface |

## Purpose

This dossier specifies `Provider/transport-azure-relay`, the transport
Provider that carries ZoneLink ComponentSession traffic over Azure Relay
Hybrid Connections. It is one of three initial ZoneLink transport Providers
alongside `Provider/transport-unix` (local allocator-issued FD) and
`Provider/transport-vsock` (local host/guest vsock).

The Provider's responsibilities are:

- Accept typed `spec.provider.settings` from a `ZoneLink` spec and
  establish a reliable bidirectional byte-stream channel to the remote Zone
  over an Azure Relay Hybrid Connection.
- Present the established channel as a named opaque byte stream to
  ComponentSession layer; the Provider has no visibility into what
  ComponentSession carries.
- Acquire relay credentials exclusively through the Credential system (Credential
  ref → Credential Provider → end-to-end KK session); no inline secret bytes
  ever enter resource spec, status, process environment, audit, OTEL, or logs.
- Operate as an **opaque intermediary carrier only**; the enrolled Noise KK
  session between adjacent Zone controllers provides end-to-end
  authentication and confidentiality independent of the relay service.
- Never map a relay-authenticated identity to a d2b local authorization role.
  Relay auth is carriage evidence; it neither is nor implies a d2b Zone subject.
- The Provider's service components run exclusively inside the gateway Guest VM
  identified by `config.executionRef` (per ADR 0032). No host process acquires
  or presents raw Azure relay credential bytes; only the gateway Guest's service
  component receives them over KK and zeroizes them after Azure authentication.

---

## Source and reuse policy

The pre-ADR-0045 v3 baseline contains a working Azure Relay WebSocket connect/
accept implementation in `packages/d2b-provider-relay/src/lib.rs`. That code:

- implements `RelayEndpoint` (`namespace` + `entity`), `RelayCredential`
  (SAS-key, SAS-token, Entra bearer), `RelayRole` (Listener/Sender), and
  `RelayStream` (bidirectional async byte stream);
- provides `connect()` / `listen()` async entry points and a `mint_sas()`
  SAS-token generator;
- has redacted `Debug` on all credential types;
- is tested for credential redaction and auth-failure error mapping.

This low-level relay plumbing is the primary reuse source for the new Provider
crate. The gateway-display logic (`d2b-gateway-runtime`, `AcaWorkloadProvider`
composition, `d2b-realm-provider` trait objects) is **not** reused; only the
relay WebSocket connect/accept mechanics are extracted.

Main commit `a1cc0b2da4a08ca3240a770a972fe4da6f912bef` provides the
ComponentSession/transport/named-stream abstractions that the relay
Provider adapter must implement. See
[ADR-046-componentsession-and-bus](../ADR-046-componentsession-and-bus.md)
for the exact symbol inventory.

---

## Provider identity

```text
Provider/transport-azure-relay
```

| Field | Value |
| --- | --- |
| Crate | `packages/d2b-provider-transport-azure-relay/` |
| Binary: listener service | `d2b-transport-azure-relay-listener` |
| Binary: sender service | `d2b-transport-azure-relay-sender` |
| Provider API major | 1 |
| ResourceTypes exported | none (transport-only; no Zone ResourceType ownership) |
| ResourceTypes consumed | `Credential`, `Network`, `Provider` (ZoneLink is read and reconciled only by core) |
| Placement | Gateway Guest only (per ADR 0032; see §Provider config schema) |

**D089 desired-spec shape.** This transport Provider owns no ResourceType; core
reconciles the `ZoneLink` base `spec.*` fields, including `spec.providerRef`.
Azure-Relay desired transport input is carried only by the canonical
`ZoneLink.spec.provider = { schemaId, schemaVersion, settings }` envelope, whose
`settings` object mirrors `status.provider.details`, is registered/signed in the
Provider manifest, deny-unknown, bounded, versioned/digested, validated against
`spec.providerRef` at Nix build and API admission, and cannot shadow base
fields. Shared fields are promoted to the ResourceType base. The Provider
implements the exact base spec/status schema version/fingerprint, accepts the
canonical minimal base Spec, passes base conformance, and rejects an
unsupported optional base capability only through its signed capability matrix plus
provider-neutral `unsupported-capability`.
`spec.provider` aligns with `status.provider`. The `Provider` resource itself
keeps the D075 `spec.{artifactId, config}` exception.

Crate layout (enforced by workspace policy):

```
packages/d2b-provider-transport-azure-relay/
  src/
    bin/
      d2b-transport-azure-relay-listener.rs
      d2b-transport-azure-relay-sender.rs
    lib.rs
    relay_transport.rs
    credential_client.rs
    transport_settings.rs
    reconnect.rs
    backpressure.rs
    metrics.rs
    audit.rs
  tests/
    transport_settings_schema.rs
    credential_redaction.rs
    fake_relay_transport.rs
    reconnect_open_transport.rs
    backpressure_credit.rs
    idempotency_key.rs
    listener_sender_conformance.rs
    integration/
      README
      fake_relay_server.rs
      zone_link_connect.rs
      credential_delivery.rs
      reconnect_scenario.rs
  README.md
```

`src/`, `src/tests/integration/`, and `README.md` must all be present.
The workspace policy gate rejects a Provider crate missing any of these.

---

## Provider config schema

`Provider.spec.config` is the root configuration object, validated against the
signed JSON Schema identified by the artifact catalog's `configSchemaDigest`
before the Provider's components launch. No field accepts secret bytes.

```yaml
spec:
  artifactId: provider-transport-azure-relay
  config:
    # Required: execution context for this Provider's service components.
    # Must be a Guest/<name> ref identifying the gateway Guest VM.
    # ADR 0032: no host process may hold relay credential bytes.
    # Service components run in this Guest; Credential scopes must match.
    executionRef: Guest/work-gateway           # required; must be Guest/<name>

    # Required: Network resource governing egress to Azure Relay.
    # Service Process network uses this ref with allowEgress=true.
    # TLS trust is governed by Network policy; no Credential ref is used for TLS.
    networkRef: Network/relay-egress           # required; must be Network/<name>

    # Credential alias bindings: maps bounded alias IDs (used in
    # ZoneLink.spec.provider.settings) to same-Zone Credential refs.
    # Alias IDs match ^[a-z][a-z0-9-]*$; max 16 entries.
    # Each referenced Credential must have scope.executionRef matching
    # config.executionRef.  No credential byte, SAS key, or token appears here.
    credentialBindings:
      relay-listen: Credential/relay-listen-k2   # example; operator-chosen
      relay-send:   Credential/relay-send-k2     # example

    # Maximum number of concurrent relay sessions this Provider instance
    # may multiplex across all ZoneLinks it serves.
    maxConcurrentSessions: 32           # 1–256; default 32

    # WebSocket connection timeout before failing.
    connectTimeoutSeconds: 30           # 5–300; default 30
```

Config field rules:

| Field | Type | Required | Rules |
| --- | --- | --- | --- |
| `executionRef` | ResourceRef | Yes | Must be `Guest/<name>`; no host value accepted; all service components execute in this Guest |
| `networkRef` | ResourceRef | Yes | Must be `Network/<name>`; used for service Process egress routing; TLS trust governed by Network policy |
| `credentialBindings` | map[alias→ResourceRef] | Yes | Alias keys match `^[a-z][a-z0-9-]*$`; values must be `Credential/<name>` in same Zone with `scope.executionRef = config.executionRef`; max 16 entries; no secret bytes |
| `maxConcurrentSessions` | u32 | No | 1–256; default 32 |
| `connectTimeoutSeconds` | u32 | No | 5–300; default 30 |

No SAS key, SAS token, bearer token, private key, connection string, or TLS
certificate byte may appear in `config` at any path. All credential material
arrives via `Credential/<name>` refs and is acquired at runtime through the
end-to-end KK Credential session inside the gateway Guest.

---

## `spec.provider.settings` schema

The transport Provider publishes a signed settings schema at:

```
docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json
```

This schema is committed alongside the crate and kept in sync by
`make test-drift` (via `xtask gen-provider-transport-schemas && git diff --exit-code`).
The Nix build phase validates every `ZoneLink.spec.provider.settings` object
against it before emitting the resource bundle.

### Canonical `spec.provider.settings` object

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://d2bus.org/schemas/v3/providers/transport-azure-relay.transport-settings.json",
  "title": "AzureRelayTransportSettings",
  "type": "object",
  "additionalProperties": false,
  "required": ["relayNamespaceId", "relayEntityId", "listenerCredentialAlias", "senderCredentialAlias"],
  "properties": {
    "relayNamespaceId": {
      "type": "string",
      "description": "Plain Azure Relay namespace identifier (not the FQDN; no scheme or host suffix).  Example: 'relns-d2b-prod'.  Non-secret; validated against ^[a-zA-Z0-9][a-zA-Z0-9-]{2,48}[a-zA-Z0-9]$.",
      "pattern": "^[a-zA-Z0-9][a-zA-Z0-9-]{2,48}[a-zA-Z0-9]$",
      "maxLength": 50
    },
    "relayEntityId": {
      "type": "string",
      "description": "Hybrid Connection entity name within the namespace.  Example: 'hc-d2b-k2'.  Non-secret.  Validated against ^[a-z][a-z0-9-]{1,49}$.",
      "pattern": "^[a-z][a-z0-9-]{1,49}$",
      "maxLength": 50
    },
    "listenerCredentialAlias": {
      "type": "string",
      "description": "Alias ID (plain string) resolved against the parent Zone's Provider.spec.config.credentialBindings map to obtain the Listen-role Credential ref.  No Credential ref, SAS key, SAS token, or bearer token byte may appear here.",
      "pattern": "^[a-z][a-z0-9-]*$",
      "maxLength": 64
    },
    "senderCredentialAlias": {
      "type": "string",
      "description": "Alias ID (plain string) resolved against the child Zone's Provider.spec.config.credentialBindings map to obtain the Send-role Credential ref.  Each Zone instance acquires its own credential locally inside its gateway Guest.  No Credential ref, SAS key, or token byte may appear here.",
      "pattern": "^[a-z][a-z0-9-]*$",
      "maxLength": 64
    }
  }
}
```

### Field rules

| Field | Required | Secret | Rules |
| --- | --- | --- | --- |
| `relayNamespaceId` | Yes | No | Plain Azure Relay namespace label only; no `.servicebus.windows.net` suffix, no scheme; validated by regex; max 50 chars |
| `relayEntityId` | Yes | No | Hybrid Connection entity name; lowercase kebab; max 50 chars |
| `listenerCredentialAlias` | Yes | No | Plain alias ID; resolved by parent Zone's Provider instance against its own `config.credentialBindings` map; never a Credential ref or token |
| `senderCredentialAlias` | Yes | No | Plain alias ID; resolved by child Zone's Provider instance against its own `config.credentialBindings` map; never a Credential ref or token |

The build emitter **rejects** any `spec.provider.settings` field:

- annotated `"secret": true`;
- containing a top-level key named `socketPath`, `hostPath`, `password`,
  `token`, `key`, or any key ending in `CredRef` or `Credential` (Credential
  refs must not appear in `spec.provider.settings`; only alias IDs are permitted);
- containing a value matching a SAS token shape (`SharedAccessSignature sr=…`
  or `?sv=…&sig=…`);
- containing a value that is a private-key PEM block.

Violations of these constraints are caught at **both** Nix build time (before
the resource bundle is emitted) and at runtime admission (before any Provider
component is started with the settings). Both enforcement points apply; neither
is exclusive.

### Nix authoring example

```nix
# Network resource that governs egress routing to Azure Relay.
# TLS trust is governed by Network policy; no Credential ref needed.
d2b.zones.k1.resources.relay-egress = {
  type = "Network";
  spec = {
    allowEgress = true;
    egressPolicy.destinations = [ "*.servicebus.windows.net:443" ];
  };
};

# Credential for the Listener-role SAS key.
# scope.executionRef must match Provider config.executionRef.
d2b.zones.k1.resources.relay-listen-k2 = {
  type = "Credential";
  spec = {
    providerRef       = "Provider/credential-secret-service";
    audience          = "azure-relay-listen";
    allowedOperations = [ "acquire-token" ];
    consumerRef       = "Provider/transport-azure-relay";
    scope.executionRef = "Guest/work-gateway";   # must match executionRef below
    rotation.policy   = "proactive";
    rotation.proactiveWindowMs = 300000;
  };
};

# Provider resource.  config.executionRef gates all component placement;
# config.networkRef governs egress routing.
d2b.zones.k1.resources.transport-azure-relay = {
  type = "Provider";
  spec = {
    artifactId = "provider-transport-azure-relay";
    config = {
      executionRef = "Guest/work-gateway";       # required; service components run here
      networkRef   = "Network/relay-egress";     # required; Network governs TLS trust
      credentialBindings = {
        relay-listen = "Credential/relay-listen-k2";
      };
      maxConcurrentSessions = 32;
      connectTimeoutSeconds = 30;
    };
  };
};

# ZoneLink K1→K2 using transport-azure-relay.
# spec.provider.settings carries plain alias IDs; parent and child each
# resolve aliases locally inside their respective gateway Guests.
d2b.zones.k1.resources.k2-guest = {
  type = "ZoneLink";
  spec = {
    childZoneName        = "k2";
    providerRef = "Provider/transport-azure-relay";
    provider = {
      schemaId      = "transport-azure-relay.d2bus.org/ZoneLink/spec";
      schemaVersion = "1.0";
      settings = {
        relayNamespaceId        = "relns-d2b-prod";  # non-secret namespace label
        relayEntityId           = "hc-d2b-k2";       # non-secret entity name
        listenerCredentialAlias = "relay-listen";    # alias ID; parent resolves locally
        senderCredentialAlias   = "relay-send";      # alias ID; child resolves locally
      };
    };
    childStaticKeyFingerprint = "9d2e1f...7f01";   # 64 lower-hex chars
    capabilityCeiling = {
      resourceTypes = [ "Process" ];
      verbs         = [ "create" "get" "list" "watch" ];
      zones         = [];
      executionRefs = [];
    };
    localIntentPolicy   = "queue";
    maxQueuedIntents    = 128;
    routeRenewalSeconds = 120;
    maxHops             = 4;
  };
};
```

---

## Enrolled Noise KK end-to-end authentication

### Role of the relay

Azure Relay is an **opaque byte-stream intermediary**. It:

- carries WebSocket frames between the listener and sender endpoints;
- authenticates the relay WebSocket connection using Azure SAS or Entra tokens
  (carriage auth only; see §Credential model);
- has no visibility into the bytes it forwards;
- cannot terminate, decrypt, or modify the Noise KK session records.

The Noise `KK` pattern (`Noise_KK_25519_ChaChaPoly_SHA256`) provides
end-to-end mutual authentication and record confidentiality between the two
Zone controllers. Azure Relay auth proves the right to open a channel; it
never authenticates a d2b Zone subject.

### KK enrollment contract

Before a ZoneLink becomes Ready, both Zone controllers must complete an
out-of-band enrollment exchange that establishes:

- **Parent** (listener side): the parent's controller enrolls its own static
  25519 key pair. The public key fingerprint is committed in the parent's
  Zone config and pinned in `ZoneLink.spec.childStaticKeyFingerprint` for the
  reverse direction.
- **Child** (sender side): the child's controller enrolls its own static
  25519 key pair. Its public key fingerprint is committed in
  `ZoneLink.spec.childStaticKeyFingerprint` on the parent's ZoneLink spec.

The enrollment record carries the static public keys in an opaque, bounded
format. No private key material enters any resource spec, status, bundle
artifact, OTEL span, or audit record.

On every session establishment:

1. The sender initiates a Noise KK handshake (`-> e, es, ss` / `<- e, ee, se`).
2. The Noise prologue binds the canonical ZoneLink `spec` object (preface ‖
   canonical offer), including `childStaticKeyFingerprint`, `childZoneName`,
   `spec.provider.settings`, `capabilityCeiling`, and `reconnectGeneration`.
3. The listener verifies that the initiator's static public key matches the
   enrolled fingerprint. Any mismatch fails closed.
4. On success, both sides derive directional Noise transport keys used for
   all subsequent `ComponentSession` records.
5. The established `ComponentSession` carries the shared
   `AuthenticatedSubjectContext` (see
   [ADR-046-componentsession-and-bus](../ADR-046-componentsession-and-bus.md)
   §Authenticated subject). The subject is `Zone/<childZoneName>` for the
   child-to-parent direction. The relay endpoint identity does **not** appear
   in this context.

A relay-issued AMQP claim, SAS authorization, or Entra bearer is carriage
evidence only. It never maps to an `AuthenticatedSubjectContext.subjectRef` and
never grants a d2b Zone RBAC role.

### No relay identity → local Admin mapping

The transport Provider **must not** derive a d2b subject, Role, or RoleBinding
from Azure Relay auth material:

- A relay token proving the `Listen` SAS claim does not grant `Admin` or any
  local authorization role.
- A managed identity token proving the `Send` Entra claim does not grant access
  to the Zone's resource API beyond what the KK-enrolled child key authorizes.
- The Provider reports relay auth success/failure only as transport connection
  status; the ZoneLink controller maps only the KK-enrolled static key to the
  child Zone subject.

This is a load-bearing invariant from ADR 0032 (§Load-bearing invariant:
relay identity is not local auth) carried forward to d2b 3.0.

---

## Credential model

### Azure auth is carriage only; Noise KK is end-to-end Zone identity

Azure Relay authentication (SAS token or Entra bearer) proves the right to
open a Hybrid Connection channel. It is **carriage evidence only** and never
authenticates a d2b Zone subject. The enrolled Noise `KK` session between the
two Zone service components provides end-to-end mutual authentication and
record confidentiality, entirely independent of the relay service.

**Raw Azure credential bytes enter only the exact gateway service component
that performs the relay authentication.** Those bytes are acquired inside the
gateway Guest over an end-to-end KK session with the Credential Provider
running in the same Guest, presented to Azure Relay to open the channel, and
immediately **zeroized** from the service component's memory after Azure
authentication succeeds. They never travel to the host Zone controller, never
appear in any resource spec, status, audit record, OTEL span, log line, or
environment variable.

### Mandatory gateway Guest placement

`Provider.spec.config.executionRef` is **required** and must be a
`Guest/<name>` ref. The admission controller rejects any Provider resource with
`executionRef` absent or referencing the host. This enforces ADR 0032: no host
process may hold relay credential bytes or present them to Azure.

The Credential resources bound by `credentialBindings` must have
`spec.scope.executionRef` equal to `config.executionRef`. A Credential whose
scope does not match is refused by the Credential controller with
`authorization-denied` when the gateway service component attempts
`acquire-token`.

### Alias-based credential binding

`Provider.spec.config.credentialBindings` maps bounded alias IDs (e.g.
`"relay-listen"`, `"relay-send"`) to same-Zone `Credential/<name>` refs.
Alias IDs are plain strings carrying no credential bytes.

`ZoneLink.spec.provider.settings` carries `listenerCredentialAlias` and
`senderCredentialAlias` — plain alias ID strings, not Credential refs:

- **Parent Zone** (`listenerCredentialAlias`): the parent gateway Guest's
  listener service resolves the alias against its own `config.credentialBindings`
  to find the Credential ref, then acquires that credential inside the gateway
  Guest via the Credential KK session.
- **Child bootstrap**: the child receives `spec.provider.settings` as part of the
  ZoneLink bootstrap. The child gateway Guest's sender service resolves
  `senderCredentialAlias` against **its own** `config.credentialBindings` and
  acquires its credential independently, inside its own gateway Guest.

No cross-Zone credential bytes exist. No parent-minted token is delivered to the
child. Each Zone's gateway service component is independently responsible for
credential acquisition and relay authentication.

### Credential acquisition over Noise KK

Both listener and sender credentials are acquired by the same mechanism, inside
the gateway Guest:

1. The gateway service component (fully enrolled Provider component identity)
   initiates a Noise KK session with the Credential Provider running in the
   same gateway Guest.
2. The session prologue binds the Credential ref UID, generation, audience,
   operation class (`acquire-token`), and `consumerRef` digest.
3. The Credential Provider delivers the raw token bytes through the protected
   KK session record. The KK session is end-to-end between the service component
   and the Credential Provider; d2b-bus forwards opaque protected records and
   cannot terminate or decrypt them.
4. The token bytes are held in zeroizing in-process memory inside the gateway
   Guest. They are presented to Azure Relay to authenticate the WebSocket
   connection and then zeroized. They never enter a log, OTEL span, audit
   record, resource spec, status field, or any cross-process communication.

---

## Components and processes

Both components are long-lived **service** processes. A `service` component
has d2b-bus access and can therefore initiate Credential KK sessions. Worker
processes have no bus and cannot acquire Credential bytes; using a worker here
would make credential acquisition impossible.

### Service: listener

| Field | Value |
| --- | --- |
| Component ID | `listener` |
| Type | service |
| Binary | `d2b-transport-azure-relay-listener` |
| Execution domain | system |
| Placement | Gateway Guest identified by `config.executionRef` |
| Cardinality | 0/1 per Provider instance; internally multiplexes up to `maxConcurrentSessions` relay sessions across all active ZoneLinks |
| ResourceTypes owned | none (returns `RelayTransportObservation` to core-controller ZoneLink handler; does not write ZoneLink.status) |

Responsibilities:

- Acquires the listener Azure credential inside the gateway Guest via the
  Credential KK session (resolving `listenerCredentialAlias` from
  `config.credentialBindings`); zeroizes raw bytes after relay authentication.
- Opens and maintains the Azure Relay Hybrid Connection control channel.
- Accepts incoming WebSocket connections from sender services; multiplexes
  sessions up to `maxConcurrentSessions`.
- Wraps each accepted connection as a named opaque byte stream and registers
  it with d2b-bus for the requesting ZoneLink session.
- TLS and WebSocket state remain in this process; only end-to-end Noise record
  bytes traverse the named byte stream. The listener cannot decrypt them.
- Returns typed `RelayTransportObservation` values to the core-controller
  ZoneLink handler; the ZoneLink handler is the sole `update-status` writer on
  ZoneLink resources.
- Emits OTEL metrics and spans (see §OTEL).
- Emits audit records for authentication events (see §Audit).

The listener service does **not**:

- run on the host or outside the gateway Guest;
- persist raw Azure credential bytes beyond the relay authentication call;
- derive d2b subjects from Azure auth;
- write ZoneLink `spec` or `status` fields directly.

Canonical Process resource (listener):

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: transport-azure-relay-listener
  ownerRef: Provider/transport-azure-relay
spec:
  template: listener
  providerRef: Provider/system-minijail
  executionRef: <config.executionRef>       # gateway Guest ref resolved from Provider config
  domain: system
  processClass: service
  sandbox:
    namespaceClasses: [mount, pid, ipc]
    capabilityClasses: []
    seccompClass: transport-azure-relay-egress
    startRoot: false
    noNewPrivileges: true
    environmentClass: minimal
    readOnlyRoot: true
  budget:
    cpu:
      request: "200m"
      limit: "500m"
    memory:
      request: "32Mi"
      limit: "64Mi"
    pids:
      limit: 64
    fds:
      limit: 256
  mounts:
    - volumeRef: Volume/transport-azure-relay--listener--state--work-gateway
      view: main
      mountPath: /state
      access: read-only
      required: true
  networkUsage:
    networkRef: <config.networkRef>
    ports: []
    allowEgress: true
  readiness:
    initialDelay: "0s"
    timeout: "60s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
  restartPolicy:
    class: on-failure
    backoffBase: "2s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
```

### Service: sender

| Field | Value |
| --- | --- |
| Component ID | `sender` |
| Type | service |
| Binary | `d2b-transport-azure-relay-sender` |
| Execution domain | system |
| Placement | Gateway Guest identified by `config.executionRef` of the child Zone's Provider instance |
| Cardinality | 0/1 per Provider instance; internally multiplexes bounded send sessions |

Responsibilities:

- Acquires the sender Azure credential inside the child's gateway Guest via the
  Credential KK session (resolving `senderCredentialAlias` from the child's own
  `config.credentialBindings`); zeroizes raw bytes after relay authentication.
- Dials the Azure Relay Hybrid Connection as the Send role.
- Pumps TLS/WebSocket bytes between the relay connection and the local
  ComponentSession transport; remains alive for the duration of each session.
- Internally multiplexes active sender sessions up to `maxConcurrentSessions`.
- Returns typed observations to the core-controller ZoneLink handler.

The sender service does **not**:

- run on the host or outside the child's gateway Guest;
- persist raw Azure credential bytes beyond the relay authentication call.

Canonical Process resource (sender):

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: transport-azure-relay-sender
  ownerRef: Provider/transport-azure-relay
spec:
  template: sender
  providerRef: Provider/system-minijail
  executionRef: <config.executionRef>       # child's gateway Guest ref resolved from its Provider config
  domain: system
  processClass: service
  sandbox:
    namespaceClasses: [mount, pid, ipc]
    capabilityClasses: []
    seccompClass: transport-azure-relay-egress
    startRoot: false
    noNewPrivileges: true
    environmentClass: minimal
    readOnlyRoot: true
  budget:
    cpu:
      request: "100m"
      limit: "300m"
    memory:
      request: "16Mi"
      limit: "32Mi"
    pids:
      limit: 32
    fds:
      limit: 128
  mounts:
    - volumeRef: Volume/transport-azure-relay--sender--state--work-gateway
      view: main
      mountPath: /state
      access: read-only
      required: true
  networkUsage:
    networkRef: <config.networkRef>
    ports: []
    allowEgress: true
  readiness:
    initialDelay: "0s"
    timeout: "60s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
  restartPolicy:
    class: on-failure
    backoffBase: "2s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
```

### Endpoint resources (D092)

The relay services' stable transport portals are owned `Endpoint` resources,
not inline `ProcessSpec` fields. They describe the visible stable binding while
keeping Azure namespace/entity names, addresses, paths, ports, FDs, and
credentials out of `Endpoint.spec` and `Endpoint.status`:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: transport-azure-relay-listener-service
  ownerRef: Process/transport-azure-relay-listener
spec:
  providerRef: Provider/transport-azure-relay
  producerRef: Process/transport-azure-relay-listener
  endpointClass: transport
  transport: opaque-carriage
  purpose: transport-azure-relay.d2bus.org/listener
  serviceFingerprint: transport-azure-relay.d2bus.org/listener.v1
  locality: cross-domain
  visibility: zone-private
  attachmentPolicy: none
  consumerPolicy: [core-controller.d2bus.org/zonelink]
  lifecyclePolicy: producer-owned
status:
  phase: Ready
  readiness: Ready
  observedProducerGeneration: 1
  observedResourceGeneration: 1
  endpointGeneration: 1
  connectionAvailability: Available
  leaseAvailability: Available
  conditions: []
---
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: transport-azure-relay-sender-service
  ownerRef: Process/transport-azure-relay-sender
spec:
  providerRef: Provider/transport-azure-relay
  producerRef: Process/transport-azure-relay-sender
  endpointClass: transport
  transport: opaque-carriage
  purpose: transport-azure-relay.d2bus.org/sender
  serviceFingerprint: transport-azure-relay.d2bus.org/sender.v1
  locality: cross-domain
  visibility: zone-private
  attachmentPolicy: none
  consumerPolicy: [core-controller.d2bus.org/zonelink]
  lifecyclePolicy: producer-owned
status:
  phase: Ready
  readiness: Ready
  observedProducerGeneration: 1
  observedResourceGeneration: 1
  endpointGeneration: 1
  connectionAvailability: Available
  leaseAvailability: Available
  conditions: []
```

Consumers refer to `Endpoint/<name>` and resolve it only through the authorized
EffectPort/LaunchTicket path; unauthorized callers receive
`endpoint-resolve-denied`. A listener or sender Process restart bumps the child
Endpoint `endpointGeneration`, which dependents observe as `dependency-changed`.
ZoneLink session state remains owned by the core ZoneLink controller.

### Retained opaque handles (D092)

Per-session `OpenTransport` named streams, `OwnedTransport` byte-stream handles,
relay transport connection handles, WebSocket/TLS carriage handles, pidfds, FD
indexes, and `operationId` values are controller-internal or high-churn opaque
handles. They are not `Endpoint` resources and are never exposed as stable
resource identities.

### No static PID1 units

Neither the listener service nor the sender service is a systemd static unit.
Both are `Process` resources supervised by the Zone's process Provider inside
the gateway Guest. There are no per-Zone or per-ZoneLink static `.socket` or
`.service` units.

### Provider state volumes

A **ProviderStateSet** is the optional, query-time set of the *declared*
`Volume` resources in a Zone whose `metadata.ownerRef` resolves to
`Provider/transport-azure-relay`. It is not a ResourceType or stored artifact
and is empty for a Provider that declares no state Volume:

```text
ProviderStateSet(zone, "transport-azure-relay") =
  { v : Volume | v.metadata.ownerRef == "Provider/transport-azure-relay" }
```

`Provider/transport-azure-relay` declares **no** Provider state Volume; its
`ProviderStateSet` is empty. All relay session state is transient in-process
memory (`OwnedTransport`/WebSocket handles, per D081) and is never persisted.
Its bounded non-secret operational state — listener/sender readiness,
transport-open/close reconcile stage, bounded reconnect/connection counters,
and closed-enum error detail — lives in the owning resource's `status`
subresource and the core Operation ledger (D087). All ZoneLink session state is
owned by the core ZoneLink controller under the `ZoneLink` resource.

No relay auth token, WebSocket handle, session key, or credential byte is ever
persisted; those bytes cross the sensitive KK delivery path in process memory
only. Because this Provider's operational state is fully derivable from spec,
`status`, the core Operation ledger, and its live process memory, it fails the
storage-need test and declares no state namespace, no state Volume (neither
Host-backed nor guest-local), no state-view mount, and no dedicated
state-layout `User/<name>` principal. There is no empty identity-only Volume.

---

## ZoneLink/ComponentSession integration

### Transport service interface

The core ZoneLink controller drives the relay transport lifecycle. The relay
Provider is a **typed transport service**; it does not read ZoneLink resources,
own Zone session state, or initiate Zone-level operations. Core calls the
Provider with already-validated `spec.provider.settings` and receives an opaque
byte-stream handle in return.

#### `OpenTransport(spec.provider.settings) → TransportHandle`

Called by the core ZoneLink controller when it needs a new relay channel. The
listener service:

1. Resolves `listenerCredentialAlias` from `config.credentialBindings`, acquires
   the Azure credential via KK inside the gateway Guest, and authenticates to
   Azure Relay (carriage auth only; raw bytes zeroized after auth).
2. Waits for a sender service connection on the Hybrid Connection control channel.
3. Returns a `TransportHandle` representing a **named opaque byte stream**
   exposed by the listener service process on its `transport-service` Unix
   endpoint. The relay service process retains the TLS and WebSocket state
   internally; only end-to-end Noise record bytes produced by core's KK
   machinery traverse the named stream. The relay service cannot interpret,
   inspect, or modify those bytes.

One `OpenTransport` call creates one carriage. When the relay WebSocket closes
for any reason, the named byte stream is closed by the Provider process and
core receives the close as a transport loss event. Core then applies its own
reconnect policy and issues a new `OpenTransport` call.

#### `CloseTransport(handle)`

Called by the core ZoneLink controller to tear down the relay channel. The
relay service closes the WebSocket and releases the carriage session.

#### `ObserveTransport(handle) → Stream<TransportObservation>`

Returns a stream of bounded carriage health observations: Azure auth events,
WebSocket open/close events, and stable error codes. **Core aggregates these
into the ZoneLink transport status sub-object.** The relay service does not
write ZoneLink status directly.

### Core ownership

The following are owned exclusively by the core ZoneLink controller. The relay
transport service has no authority or visibility over them:

| Concern | Core responsibility |
| --- | --- |
| Noise KK handshake and key derivation | Core initiates and verifies; relay sees opaque bytes only |
| Session generation counter | Core increments on each reconnect |
| Reconnect policy and backoff | Core's reconnect policy drives reconnect; core calls `CloseTransport` then `OpenTransport` after applying its own backoff |
| Idempotency key tracking | Core assigns and deduplicates `ZoneLinkIdempotencyKey` |
| Route state and Watch cursors | Core manages; relay has no view |
| ZoneLink resource status and finalizer | Core writes; relay returns observations only |

### Currency and upgrade (D091)

The core ZoneLink controller, not the relay transport service, implements
`assess_update`, `plan_upgrade`, and `execute_upgrade`. A Provider generation or
signed artifact generation/digest change updates universal `status.update` with
`state: UpdateAvailable` or `state: UpgradeRequired`, `reasons` including
`ProviderGenerationChanged` or `ArtifactChanged`, observed/target generation or
digest IDs, `disruption: Reload` or `disruption: Restart`, `preserveState:
true`, bounded `owned`/`dependencies`, and `lastAssessedAt`. Disruptive changes
MUST return `UpgradeRequired` rather than applying in place; non-disruptive
changes reconcile normally. Upgrade recycles the relay service realization;
open byte-stream handles are re-established by core reconnect. ZoneLink session
state remains owned by the core ZoneLink controller. `status.update` MUST NOT
contain secrets.

### Expedited reconcile on mutation (D090)

For `Create`, `UpdateSpec`, and `Delete` with `waitForReconcile`, core MUST
perform no `OpenTransport`/`CloseTransport`, finalizer change, or status
mutation until it supplies
`CommittedRevisionProof {resourceUid,generation,revision,operationId}`. Abort
before that proof has no effect. After durable commit, the commit is never
rolled back if the reconcile pass times out. The response returns the committed
object, post-pass projected layered status, disposition
(`Converged|Progressing|Blocked|UpgradeRequired|Failed`), and
`statusPersistence: pending|committed`. Effect idempotency keys derive from
`(UID,generation,revision,operationId)` and use the same per-resource
single-flight priority lane.

### Named opaque byte-stream properties

The `TransportHandle` refers to a per-session named byte stream exposed by the
relay service process after core resolves the D092 `Endpoint/<name>` under
authorization. The named stream itself remains an internal high-churn handle;
it is not an `Endpoint` and carries no raw locator. Core and d2b-bus exchange
Noise record bytes over that stream. Key properties:

- **TLS/WebSocket state stays in the Provider process.** The relay service
  owns the TLS handshake and WebSocket framing internally. It pumps bytes
  between the WebSocket and the named stream. No raw TLS or WebSocket FD
  is transferred to d2b-bus or core.
- **Opaque Noise records only.** The named stream carries only the
  2-byte length-prefixed Noise record bytes produced and consumed by core's
  KK machinery. The relay service cannot decrypt or interpret them.
- **Attachment support: false.** FD transfer via `SCM_RIGHTS` is a local-Unix
  operation; it is rejected at the source d2b-bus before any relay frame is
  sent (`attachment-not-permitted-over-zone-link`).
- **Locality: Remote.** Local-only operations (`SO_PEERCRED`, pidfd) are
  unavailable over the ZoneLink.
- **Atomic: false.** Framing uses 2-byte length-prefixed records (consistent
  with `UnixStreamTransport`). The relay service treats every byte as opaque.
- **One carriage per `OpenTransport`.** When the WebSocket closes, the Provider
  process closes its end of the named stream, signalling transport loss to core.
  Core must call `OpenTransport` again to establish new carriage.

### No credential or path forwarding

The relay transport passes opaque encrypted records. It explicitly enforces
the zone routing constraint (see [ADR-046-zone-routing](../ADR-046-zone-routing.md)
§No FD, credential, or host path forwarding):

- No token, session PSK, private key, bearer token, enrollment secret, or
  credential lease byte may appear in a relay frame payload.
- No host path, socket path, device path, or store path may appear in relay
  routing metadata.
- PIDs, pidfds, and broker ops are not propagated.

### Backpressure and credit

The relay transport implements the same bounded-credit backpressure model
used for all ZoneLink named streams:

- Outbound frames are gated by ComponentSession named-stream credit from the
  remote peer.
- The relay service's WebSocket send buffer is bounded; when Azure Relay is slow,
  WebSocket write backpressure propagates inward through the named stream to the
  ComponentSession `FairScheduler`, which withholds named-stream send-credit
  from the caller.
- Named stream credit is independently managed at each hop; the relay hop
  cannot cause unbounded memory growth at the source Zone.
- `MAX_NAMED_STREAM_QUEUE_BYTES` and `MAX_AGGREGATE_NAMED_STREAM_QUEUE_BYTES`
  from the `v3_component_session` contract constants apply; the relay transport
  does not relax them.

---

## RBAC and security

### Transport Provider authorization

The transport-azure-relay service component processes are authorized by native
RBAC only for what a typed transport service requires. They do **not** receive
ZoneLink or ResourceAPI access; core calls them with already-validated
`spec.provider.settings`.

Granted:

- `acquire-token` on the `Credential` resources bound by
  `config.credentialBindings` (subject to `Credential.spec.consumerRef =
  "Provider/transport-azure-relay"`, `allowedOperations` includes `acquire-token`,
  and `scope.executionRef` matches the service component's gateway Guest);
- `get` on the `Network` resource named by `config.networkRef`.

Not held:

- `get`, `watch`, `update-status`, `create`, `update-spec`, or `delete` on any
  ZoneLink (the core ZoneLink controller owns those; the relay service receives
  `spec.provider.settings` as a parameter from core, not by reading the resource);
- any ResourceAPI verb on Zone resources other than the above;
- any relay-forwarding verb for routing calls to child Zones.

### No relay → admin escalation

The Azure Relay service authentication (SAS token or Entra bearer) is not
accepted as a d2b authorization credential at any layer:

- A process that has authenticated to the Azure Relay as a Sender cannot use
  that auth to invoke d2b resource API methods.
- A relay-listening process that has acquired the listener SAS is not
  implicitly `Admin`.
- A managed identity that holds the Azure Relay `Listen` RBAC role does not
  gain any d2b Zone RBAC role by virtue of that Azure assignment.
- The relay WebSocket connection uses TLS with Azure's server certificate.
  The relay server's certificate is not used as a d2b trust anchor; it is
  carriage TLS only.

Zone subjects are established exclusively by the Noise KK enrolled static
key registered in `ZoneLink.spec.childStaticKeyFingerprint`. All other auth
material is scoped to the relay transport layer.

### Sandboxing

Both the listener and sender service processes run under the sandbox declared
in their canonical Process resources above. Sandbox fields are compiled by
`Provider/system-minijail`:

- `namespaceClasses: [mount, pid, ipc]` — mount, PID, and IPC namespaces
  are unshared; no network namespace (egress is handled via `networkUsage`);
- `capabilityClasses: []` — no capability grants beyond the process class base;
- `seccompClass: transport-azure-relay-egress` — named Provider seccomp profile
  from the transport Provider's compiled catalog; permits only the syscalls
  required for TLS/WebSocket egress and IPC;
- `startRoot: false` — process never starts as in-namespace root;
- `noNewPrivileges: true` — `PR_SET_NO_NEW_PRIVS` set before exec;
- `environmentClass: minimal` — only the fixed approved environment set;
  no inherited host variables;
- `readOnlyRoot: true` — rootfs mounted read-only.

Network egress is governed by the `Network` resource referenced in
`config.networkRef` via `networkUsage.networkRef`. TLS trust for the Azure
Relay endpoint comes from the Network policy configuration, not from a
Credential resource.

---

## Lifecycle and status

### Core-aggregated transport status

The relay transport service does **not** write ZoneLink status. Core receives
a `Stream<TransportObservation>` from `ObserveTransport` and aggregates those
observations into the ZoneLink transport status sub-object.

Per D088, core writes the ZoneLink universal `ResourceStatus` base at top-level
`status.*` and cross-provider ZoneLink/transport observation under
`status.resource`. Azure-specific bounded, non-secret observation belongs in
`status.provider` with `providerRef: Provider/transport-azure-relay`, qualified
`schemaId: transport-azure-relay.d2bus.org/ZoneLink/status`, `schemaVersion` (semver MAJOR.MINOR),
`observedProviderGeneration`, and a strict unknown-field-denied, ≤32 KiB,
redacted `details` object registered and signed in the Provider manifest. Core
writes all present layers atomically in one status mutation; shared fields are
promoted to `status.resource` and never duplicated into `status.provider`.

The ZoneLink handler writes the following D088 shape based on observations
received:

```yaml
status:
  # ZoneLink core status (managed by core ZoneLink controller):
  phase: Ready
  conditions: [...]
  resource:
    zoneLink:
      sessionGeneration: 3
      reconnectCount: 1
      lastConnectedAt: 2026-07-22T00:00:00Z
      ...
    transport:
      providerRef: Provider/transport-azure-relay
      phase: Connected           # Pending | Connected | Reconnecting | Failed | Unknown
      lastConnectedAt: 2026-07-22T00:00:00Z
      lastDisconnectedAt: null
      lastDisconnectReason: null       # bounded redacted string; no secret bytes
      reconnectAttempt: 0
  provider:
    providerRef: Provider/transport-azure-relay
    schemaId: transport-azure-relay.d2bus.org/ZoneLink/status
    schemaVersion: "1.0"
    observedProviderGeneration: 3
    details:
      relayEndpoint:
        namespaceId: relns-d2b-prod   # non-secret; echoed from spec.provider.settings
        entityId: hc-d2b-k2           # non-secret; echoed from spec.provider.settings
      credentialExpiresAtUnixMs: 1753232401000   # listener credential expiry
      conditions:
        - type: RelayConnected
          status: "True"
          reason: websocket-open
          lastTransitionAt: 2026-07-22T00:00:00Z
        - type: CredentialValid
          status: "True"
          reason: lease-active
          lastTransitionAt: 2026-07-22T00:00:01Z
```

Rules:

- `status.provider.details.relayEndpoint.namespaceId` and `.entityId` are
  non-secret Azure identifiers echoed from `spec.provider.settings` for operator
  diagnostics.
- `lastDisconnectReason` is a bounded (max 256 chars), redacted string.
  It never contains token bytes, SAS values, stack traces, or internal paths.
- `status.provider.details.credentialExpiresAtUnixMs` is the listener credential
  lease expiry from `Credential.status.credential.expiresAtUnixMs`. Never a
  token or key.
- The provider-extension `RelayConnected` and `CredentialValid` conditions
  reflect carriage health, not Zone routing health. Core derives
  `ZoneLink.status.conditions.SessionEstablished` from the Noise KK handshake
  outcome, which succeeds only after relay transport is `Connected`.

### Status phases

| Phase | Meaning |
| --- | --- |
| `Pending` | Listener service process started inside gateway Guest; relay control channel not yet open |
| `Connected` | Relay channel open; `OpenTransport` returned a handle; awaiting core Noise KK |
| `Reconnecting` | Core called `CloseTransport` after disconnect; re-issuing `OpenTransport` |
| `Failed` | Core reconnect policy exhausted or credential unrecoverable |
| `Unknown` | Core cannot determine carriage state from `ObserveTransport` stream |

Core transitions the ZoneLink resource to phase `Failed` after the reconnect
policy is exhausted or when the listener Credential transitions to `LeaseRevoked`
with no replacement; the relay service reports the carriage health observation
and core owns the final status write.

---

## Errors

Stable error codes returned by the relay transport service via the
`ObserveTransport` stream. Core maps these to ZoneLink status
`lastDisconnectReason` values and `SessionErrorCode` entries in
`v3_component_session`.

| Code | Meaning |
| --- | --- |
| `relay-connect-timeout` | WebSocket connect did not complete within `connectTimeoutSeconds` |
| `relay-auth-failed` | Azure Relay returned HTTP 401/403; credential may be expired or revoked |
| `relay-endpoint-not-found` | Azure Relay returned HTTP 404 for the namespace/entity combination |
| `relay-tls-failed` | TLS handshake failure to Azure Relay service |
| `relay-websocket-closed` | Relay closed the WebSocket gracefully |
| `relay-websocket-error` | Relay closed the WebSocket with an error frame |
| `relay-credential-unavailable` | Credential Provider could not supply a credential within the deadline |
| `relay-max-reconnect-exhausted` | Reconnect attempts exceeded `maxAttempts` |
| `relay-invalid-transport-settings` | `spec.provider.settings` failed runtime schema validation |

Error observation fields:

- are bounded (max 256 chars);
- never contain token bytes, SAS values, connection strings, or internal paths;
- never echo credential material, even partially.

---

## Audit

Provider audit covers **carriage authentication and health observations only**.
It is **separate from resource audit** (which is owned by core). Resource
lifecycle events (ZoneLink state transitions, session generation, reconnect
decisions, idempotency outcomes) appear in the core resource audit trail, not
here. Provider audit records are appended through the Zone runtime's audit log
interface; appends are not atomic with Zone resource state in redb.

| Audit kind | Fields | Trigger |
| --- | --- | --- |
| `relay-carriage-auth-success` | `zone`, `zoneLinkName`, `relayNamespaceId`, `relayEntityId`, `correlationId` | Azure Relay WebSocket authenticated successfully (carriage only; no Noise KK outcome here) |
| `relay-carriage-auth-failed` | `zone`, `zoneLinkName`, `relayNamespaceId`, `relayEntityId`, `reason`, `correlationId` | Azure Relay returned auth failure; bounded `reason` code |
| `relay-carriage-closed` | `zone`, `zoneLinkName`, `relayNamespaceId`, `relayEntityId`, `reason`, `correlationId` | WebSocket closed; bounded `reason` code |
| `relay-credential-acquired` | `zone`, `zoneLinkName`, `credentialAliasId`, `leaseHandle`, `operationClass`, `correlationId` | Credential successfully acquired; opaque `leaseHandle` digest only, never token bytes |
| `relay-credential-failed` | `zone`, `zoneLinkName`, `credentialAliasId`, `reason`, `correlationId` | Credential acquisition failed; bounded reason code; no token material |

Rules:

- Audit records **never** contain token bytes, SAS values, connection strings,
  bearer tokens, private keys, or any credential material.
- `leaseHandle` is the opaque bounded handle from
  `Credential.status.credential.leaseHandle`, not a token.
- `reason` fields use stable bounded codes, not provider-internal diagnostics.
- `relayNamespaceId` and `relayEntityId` are non-secret identifiers.
- `correlationId` links audit records to OTEL spans without carrying span payload.
- Noise KK outcomes, session generation, and resource state transitions are
  recorded in the core resource audit trail, not here.

---

## OTEL telemetry

### Metrics

All metric labels are closed sets. No label carries credential bytes, relay
token shapes, private key fragments, connection-string substrings, store paths,
or internal provider diagnostics.

| Metric name | Type | Labels | Description |
| --- | --- | --- | --- |
| `d2b_relay_transport_connect_total` | Counter | `zone`, `outcome` (`success`/`failed`), `error_code` | Relay WebSocket connect attempts |
| `d2b_relay_transport_disconnect_total` | Counter | `zone`, `reason` (stable bounded code) | Relay WebSocket disconnects |
| `d2b_relay_transport_reconnect_total` | Counter | `zone`, `outcome` | Reconnect attempts |
| `d2b_relay_transport_session_seconds` | Histogram | `zone` | Duration of relay WebSocket sessions |
| `d2b_relay_transport_bytes_sent_total` | Counter | `zone` | Bytes sent over relay (post-encryption; opaque payload size) |
| `d2b_relay_transport_bytes_received_total` | Counter | `zone` | Bytes received over relay |
| `d2b_relay_transport_frames_sent_total` | Counter | `zone` | Frames sent |
| `d2b_relay_transport_frames_received_total` | Counter | `zone` | Frames received |
| `d2b_relay_transport_send_queue_bytes` | Gauge | `zone` | Current outbound frame queue depth (bytes) |
| `d2b_relay_transport_credential_expiry_seconds` | Gauge | `zone` | Seconds until listener credential expiry; 0 when no active lease |
| `d2b_relay_transport_backpressure_events_total` | Counter | `zone` | Times outbound send blocked on WebSocket write backpressure |

Permitted label keys: `zone` (Zone name; max 63 chars, same regex as Zone name),
`outcome`, `reason`, `error_code`. The `zone` label carries the Zone name, not a
workload name, IP, or store path.

Forbidden label keys: namespace FQDN, entity name, relay region, credential ref,
token shape, connection string, or any credential material.

### Traces

OTEL spans are emitted for:

- Relay WebSocket connect and accept operations (span: `relay.connect`,
  `relay.accept`).
- Noise KK handshake initiation and completion (span: `kk.handshake`; no
  key material in attributes).
- Credential acquisition requests (span: `credential.acquire`; carries only
  `credentialRef` as an opaque ResourceRef string, never token bytes).
- Reconnect cycles (span: `relay.reconnect`).

Span attributes never include:
- token bytes, SAS values, connection strings, bearer tokens;
- relay namespace FQDN (only the non-secret `namespaceId` label);
- relay region (operator-supplied; not emitted as OTEL attribute);
- host paths, socket paths, or store paths;
- private key fragments or Noise key material.

---

## Async performance

All relay transport operations are fully async with no inline synchronous
I/O:

- Named byte stream send and receive on the `transport-service` Unix endpoint
  are async; they do not `block_on` internally.
- Blocking TLS and WebSocket handshake operations use `tokio::task::spawn_blocking`
  adapters with bounded task quota.
- Credential acquisition is async via the Credential KK ComponentSession inside
  the gateway Guest.
- The `FairScheduler` credit-gating mechanism is the sole backpressure
  mechanism; no thread-level blocking occurs when the relay network is slow.
- Both listener and sender are long-lived service processes that internally
  multiplex relay sessions; no per-session process spawn is required.

Performance targets:

| Metric | Target |
| --- | --- |
| Relay WebSocket connect latency (P99, LAN) | < 300 ms |
| Relay WebSocket connect latency (P99, cross-region) | < 2 s |
| End-to-end Noise KK handshake latency (P99, relay) | < 500 ms |
| Memory per active relay transport session | < 256 KiB |
| Reconnect time from disconnect to Noise KK established (P99) | < 5 s |

These targets inform the `connectTimeoutSeconds` default (30 s). They are
not contractually enforced at the API level; they guide conformance test tuning.

---

## Nix artifact

The Provider is declared in the artifact catalog:

```nix
d2b.artifacts.provider-transport-azure-relay = {
  package = inputs.d2b-providers.packages.${system}.transport-azure-relay;
  type    = "provider";
};
```

The artifact entry resolves to:

```
packages/d2b-provider-transport-azure-relay/
  -> d2b-transport-azure-relay-listener   (system binary)
  -> d2b-transport-azure-relay-sender     (system binary)
  -> provider-manifest.json               (signed)
  -> transport-settings.schema.json       (settings schema; committed separately
                                           under docs/reference/schemas/v3/providers/)
```

The settings schema file at:

```
docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json
```

is committed, version-controlled, and kept in sync with the Rust
`AzureRelayTransportSettings` type by `make test-drift` (via
`xtask gen-provider-transport-schemas && git diff --exit-code`).

The artifact **never** includes:

- a SAS key, SAS token, or bearer token;
- a relay namespace FQDN (it is a runtime `spec.provider.settings` field, not an
  artifact constant);
- a TLS private key.

Provider resource `spec.artifactId = "provider-transport-azure-relay"` selects
this entry. Selection is exact digest; no version-range solving, runtime
download, or PATH scan.

---

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `packages/d2b-provider-relay/src/lib.rs` (relay WebSocket connect/accept, `RelayEndpoint`, `RelayCredential`, `RelayStream`); `packages/d2b-provider-relay/src/bin/d2b-relay.rs` (relay CLI) |
| Evidence class | WebSocket connect/accept implemented-and-tested; credential redaction implemented; Provider resource/process/ComponentSession integration absent |
| Reuse source | `d2b-provider-relay/src/lib.rs`: extract `RelayEndpoint`, `RelayCredential`, `RelayRole`, `RelayStream`, `connect()`, `listen()`, `mint_sas()`, credential redaction; adapt for v3 named byte-stream transport inside long-lived service process |
| Excluded current behavior | Gateway-display relay path (`d2b-gateway-runtime/src/bin/d2b-gateway-relay.rs`); ACA provider composition (`d2b-provider-aca`, `AcaWorkloadProvider`); `RelayProvider` trait objects from `d2b-realm-provider` |
| Excluded ADR45 assumptions | ADR45 fixed 4-unit PID1 endpoint set; `SD_LISTEN_FDS` relay bootstrap; `d2b-realm-router` ProviderInstance relay composition; ADR45 bundle version constants |
| Required delta | Provider resource/catalog; one crate with mandatory layout; named byte-stream transport via `transport-service` Unix endpoint; Credential KK session acquisition inside gateway Guest; `config.executionRef`/`networkRef` placement gates; typed `spec.provider.settings` schema; long-lived service process multiplexing; reconnect response to `CloseTransport`/`OpenTransport`; backpressure/credit integration; audit/OTEL; Nix artifact |
| Behavior retained | Relay WebSocket connect/accept mechanics; credential redaction invariant; SAS mint logic; auth-error mapping |
| Removal proof | `d2b-provider-relay` retired only after gateway-display path migrates to Provider resource model; `d2b-gateway-relay.rs` binary retired only after ACA Provider dossier integrates |
| Feasibility proof | Fake relay server in `src/tests/integration/` enabling hermetic reconnect and credential scenarios without live Azure service |

---

## Implementation work items

### ADR046-transport-relay-001

| Field | Value |
| --- | --- |
| Dependency/owner | W0 shared contract root; ComponentSession transport adapter owner |
| Current source | `packages/d2b-provider-relay/src/lib.rs` (`RelayEndpoint`, `RelayCredential`, `RelayRole`, `RelayStream`, `connect()`, `listen()`, `mint_sas()`) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-transport-azure-relay/src/relay_transport.rs` |
| Detailed design | Adapt `RelayStream` as relay transport service process; expose named opaque byte stream on the `transport-service` Unix endpoint; add 2-byte length-prefixed framing; preserve credential redaction; TLS/WebSocket state stays in-process — only Noise record bytes traverse the named stream; register named stream with d2b-bus as `TransportHandle`; transport descriptor: `attachment_support: false`, `locality: Remote`, `atomic: false`; expose `OpenTransport`/`CloseTransport`/`ObserveTransport` interface to core; long-lived service process multiplexes sessions internally Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Core ZoneLink controller calls `OpenTransport(spec.provider.settings)` → receives named byte stream handle; relay service cannot interpret plaintext bytes; one carriage per call; WebSocket loss closes the named stream |
| Data migration | No compatibility with current relay sessions; v3 sessions are independent |
| Validation | `tests/fake_relay_transport.rs`: connect/accept, framing, credential redaction, named stream roundtrip; `tests/listener_sender_conformance.rs`: named stream contract; Noise KK binding; relay identity exclusion |
| Removal proof | `d2b-provider-relay/src/lib.rs` relay plumbing retained until ACA display migration completes |

### ADR046-transport-relay-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-transport-relay-001; Credential KK session; ComponentSession/d2b-bus owner |
| Current source | None (new) |
| Reuse action | create |
| Destination | `packages/d2b-provider-transport-azure-relay/src/credential_client.rs` |
| Detailed design | Async Credential KK session client for service components (service processes have d2b-bus access, enabling Credential KK; workers do not); acquire listener credential via KK inside the gateway Guest using `config.credentialBindings[listenerCredentialAlias]`; acquire sender credential independently inside the child's gateway Guest via the same KK model using `config.credentialBindings[senderCredentialAlias]`; raw credential bytes held in zeroizing memory inside the gateway Guest, presented to Azure Relay, then immediately zeroized; no credential bytes cross process, network, or Guest boundary; redacted Debug; no credential bytes in logs/audit/OTEL; core ProviderDeployment creates a private persistent Volume (per ADR-046-provider-state) for each component before its Process starts — the transport Provider does not own or create these Volumes; `Provider/volume-local` reconciles them; `migrationPolicy: none` means no migration worker is ever spawned; no relay auth token, WebSocket handle, session key, or credential byte is written to that Volume; all relay session state remains transient in-process memory |
| Integration | Listener service invokes before each relay connect attempt inside gateway Guest; child Zone's sender service acquires its own credential independently inside its own gateway Guest |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `tests/credential_redaction.rs`: credential bytes never reach any Debug/log/audit/OTEL path; `src/tests/integration/credential_delivery.rs`: end-to-end credential delivery using injected fake Credential effect port |
| Removal proof | N/A; new module |

### ADR046-transport-relay-003

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-transport-relay-001; reconnect contract; ZoneLink handler |
| Current source | None (new; core drives reconnect, not the transport Provider) |
| Reuse action | create |
| Destination | `packages/d2b-provider-transport-azure-relay/src/reconnect.rs` |
| Detailed design | Relay service responds to `CloseTransport`+`OpenTransport` cycle from core; core owns reconnect policy and backoff scheduling; relay service tears down the current WebSocket when core calls `CloseTransport` and establishes a new WebSocket connection when core calls `OpenTransport`; relay service does not maintain a backoff state machine or independently retry — it starts a new WebSocket on demand and emits the connect result via `ObserveTransport`; listener and sender are long-lived service processes that do not re-spawn on reconnect |
| Integration | `ObserveTransport` delivers `TransportObservation::Disconnected` to core; core drives reconnect via `CloseTransport` then `OpenTransport` after applying its own backoff |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `tests/reconnect_backoff.rs`: relay responds to CloseTransport/OpenTransport cycle; WebSocket starts on demand; ObserveTransport reports connect result; `src/tests/integration/reconnect_scenario.rs`: full reconnect cycle including Credential re-acquisition |
| Removal proof | N/A; new module |

### ADR046-transport-relay-004

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-transport-relay-001; transport settings schema; Nix configuration owner |
| Current source | `docs/specs/ADR-046-zone-routing.md` transport settings Nix example |
| Reuse action | create |
| Destination | `packages/d2b-provider-transport-azure-relay/src/transport_settings.rs`; `docs/reference/schemas/v3/providers/transport-azure-relay.transport-settings.json` |
| Detailed design | `AzureRelayTransportSettings` Rust struct with serde; validation against committed JSON Schema; reject `secret`-annotated fields; enforce `^[a-z][a-z0-9-]*$` pattern for `listenerCredentialAlias`/`senderCredentialAlias` alias ID fields (never `Credential/<name>` refs); xtask `gen-provider-transport-schemas` integration |
| Integration | `make test-drift` gate: `xtask gen-provider-transport-schemas && git diff --exit-code` |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `tests/transport_settings_schema.rs`: valid/invalid schema vectors; eval-time Nix assertion coverage from `nix-unit: transport-settings-secret-key` test (see zone-routing spec) |
| Removal proof | N/A; new contract |

### ADR046-transport-relay-005

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-transport-relay-001; backpressure/credit contract |
| Current source | `packages/d2b-session/src/scheduler.rs`, `streams.rs` (main commit `a1cc0b2d`) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-transport-azure-relay/src/backpressure.rs` |
| Detailed design | Outbound WebSocket send buffer bounded at `MAX_AGGREGATE_NAMED_STREAM_QUEUE_BYTES`; relay WebSocket write backpressure propagates to `FairScheduler` credit; `d2b_relay_transport_backpressure_events_total` counter emitted; no unbounded memory growth under slow relay |
| Integration | Named stream send on `transport-service` Unix endpoint blocks on relay WebSocket write; d2b-bus `FairScheduler` observes backpressure via credit stall |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `tests/backpressure_credit.rs`: slow relay writer saturates outbound queue; named-stream credit stalls before unbounded growth; source Zone never buffers beyond aggregate limit |
| Removal proof | N/A; new module |

### ADR046-transport-relay-006

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-transport-relay-001 through ADR046-transport-relay-005; telemetry/audit owner |
| Current source | `packages/d2bd/src/metrics.rs` (hand-rolled Prometheus; baseline) |
| Reuse action | create |
| Destination | `packages/d2b-provider-transport-azure-relay/src/{metrics.rs, audit.rs}` |
| Detailed design | Emit all OTEL metrics and audit records listed in §OTEL and §Audit; closed label sets; never label secret bytes; provider audit covers **carriage authentication and health observations only** — Azure auth events, WebSocket lifecycle, credential acquisition outcomes — and is **separate from resource audit** (resource lifecycle events are owned by core); audit records appended through the Zone runtime audit log interface (no atomicity guarantee with Zone resource state in redb; best-effort delivery per the Zone's audit provider configuration); OTEL via lightweight emitter ring (no direct OTEL SDK dependency in Provider) |
| Integration | `Provider/observability-otel` receives emitter ring frames; audit log via Zone runtime `d2b.audit.transport` category |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | `tests/credential_redaction.rs` extended to cover audit/OTEL paths; `tests/fake_relay_transport.rs` asserts audit record fields against schema |
| Removal proof | N/A; new module |

### ADR046-transport-relay-007 (integration README)

| Field | Value |
| --- | --- |
| Dependency/owner | Provider crate owner; integration test owner |
| Current source | None — net-new v3 work; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | `packages/d2b-provider-transport-azure-relay/src/tests/integration/README` |
| Detailed design | Required content: fake relay server setup and teardown using the injected fake Relay effect port; how to run hermetic integration tests without a live Azure service; how to configure the injected fake Credential effect port for credential delivery tests; how to run with a real Azure namespace (requires a `Credential` resource declared in `spec.config.credentialBindings`, not environment-variable credential paths); integration test scenarios and expected outcomes; CI/local execution instructions |
| Integration | `make test-integration` invokes `tests/integration/containers/` scenarios which inject the fake relay and credential port implementations from `src/tests/integration/fake_relay_server.rs` |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | File must be present; workspace policy gate enforces `src/tests/integration/README` |
| Removal proof | N/A; mandatory layout |

---

## Required tests

### Unit tests (in `tests/`)

The following test modules must be present and pass before the first integration
wave:

| Test module | Required test functions | What they prove |
| --- | --- | --- |
| `transport_settings_schema.rs` | `valid_settings_roundtrip`, `missing_required_field_rejected`, `secret_key_field_rejected`, `sas_token_value_rejected`, `pem_key_value_rejected`, `unknown_field_rejected`, `credential_alias_pattern_enforced`, `credential_ref_in_settings_rejected` | Schema validation vectors; no secret-shaped value admitted; alias IDs accepted, Credential refs rejected |
| `credential_redaction.rs` | `sas_key_debug_is_redacted`, `entra_bearer_debug_is_redacted`, `sas_token_debug_is_redacted`, `credential_not_in_audit_record`, `credential_not_in_otel_span`, `credential_not_in_status`, `credential_not_in_log` | Credential bytes never reach any observable surface |
| `fake_relay_transport.rs` | `listener_accepts_sender_connection`, `framing_is_length_prefixed`, `send_receive_roundtrip_over_fake_relay`, `attachment_support_is_false`, `locality_is_remote`, `transport_descriptor_contract` | Named byte-stream contract; framing; transport descriptor |
| `reconnect_open_transport.rs` | `websocket_starts_on_open_transport`, `websocket_closes_on_close_transport`, `observe_transport_reports_connect_result`, `reconnect_clears_generation`, `reconnect_triggers_new_kk` | Relay responds to CloseTransport/OpenTransport cycle; named stream closes on WebSocket loss |
| `backpressure_credit.rs` | `slow_relay_stalls_credit`, `aggregate_queue_bounded`, `source_never_buffers_beyond_limit`, `backpressure_event_counter_increments` | Backpressure and credit invariants |
| `idempotency_key.rs` | `idempotency_key_carried_in_noise_record`, `idempotency_key_not_in_relay_frame_metadata`, `replay_at_relay_level_does_not_deduplicate`, `dedup_at_child_zone_resource_api` | Idempotency is child-Zone-owned; relay carries opaquely |
| `listener_sender_conformance.rs` | `conformance_vectors_listener`, `conformance_vectors_sender`, `noise_kk_prologue_binds_transport_settings`, `mismatched_fingerprint_fails_closed`, `relay_identity_not_in_subject_context` | Named stream contract; Noise KK binding; relay identity exclusion |

### Integration tests (in `src/tests/integration/`)

| Test | Fixture | What it proves |
| --- | --- | --- |
| `zone_link_connect.rs` | `fake_relay_server.rs` (in-process async fake relay via injected effect port) | Full ZoneLink bootstrap over fake relay: connect, Noise KK, resource API ping, Watch |
| `credential_delivery.rs` | Injected fake Credential effect port | Listener credential acquired via KK session; token bytes zeroized; sender credential acquired independently by child Zone instance; no cross-Zone token minting |
| `reconnect_scenario.rs` | Fake relay server with injected disconnect | Disconnect triggers new relay connect; new Noise KK; Watch resumes from last revision; queued intents replayed |
| `fake_relay_server.rs` | — | Fake Azure Relay server that accepts listener and sender WebSocket roles; no real Azure service required; controllable inject-disconnect API; passed as constructor argument or injected effect port to test subjects |

The `src/tests/integration/fake_relay_server.rs` must implement the Azure Relay Hybrid
Connection WebSocket protocol sufficiently to:

- accept a Listener role connection with a SAS or Entra token;
- accept a Sender role connection;
- forward frames between them bidirectionally;
- expose a test-only `inject_disconnect(role: RelayRole)` method for
  reconnect testing.

It must NOT require network access to `*.servicebus.windows.net` in CI. The
fake relay server and fake Credential port are injected as constructor
arguments or via the toolkit's fake-port infrastructure; no environment
variable activates them. Live/manual integration tests that target a real
Azure namespace must declare Credential resources in `spec.config.credentialBindings`
and supply those Credential resources in the test configuration — never via
`D2B_RELAY_NAMESPACE`, `D2B_RELAY_ENTITY`, `D2B_RELAY_SAS_ENV`, or any
environment-variable credential path. Tests that require a live Azure endpoint
are gated with `#[ignore]` and documented in `src/tests/integration/README`.

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has p95 ≤50 ms with no wall-clock
sleep, and `cargo test -p d2b-provider-transport-azure-relay --lib --tests` completes in ≤2 s warm-cache
execution time (compilation excluded). They use a deterministic fake clock/RNG
and the toolkit fakes/FakeEffectPort only — no process spawn, container,
network, DBus, systemd, broker daemon, Nix eval/build, KVM, USB/GPU/TPM
hardware, or live cloud, and no filesystem tree beyond tiny temp fixtures. Any
scenario needing those lives only in `integration/`, which keeps a lane
timeout/budget, parallel isolation, and fake external services by default; such
a need is re-placed into `integration/`, never given a sleep, larger timeout,
or `#[ignore]`. Bounded crypto/property tests are the only classified
exception, each named with a capped case count and a declared higher per-test
budget.

---

## Removal conditions

The following existing code may be removed only after the conditions below
are met:

| Existing symbol/crate | Removal condition |
| --- | --- |
| `packages/d2b-provider-relay/src/lib.rs` relay WebSocket plumbing | All callers replaced by `d2b-provider-transport-azure-relay`; ACA display path migrated to v3 Provider model (`ADR046-transport-relay-001` complete and ACA Provider dossier integrates relay) |
| `packages/d2b-gateway-runtime/src/bin/d2b-gateway-relay.rs` | ACA gateway display path migrated to `Provider/runtime-azure-container-apps` + `Provider/transport-azure-relay`; gateway guest ZoneLink bootstrap tested |
| `d2b-realm-provider` `RelayProvider` / `TransportProvider` traits | All implementations migrated to Provider ResourceType model; `d2b-provider-relay` usage in `d2bd` removed |
| `D2B_RELAY_NAMESPACE` / `D2B_RELAY_ENTITY` environment variables in `d2b-relay.rs` CLI | Relay CLI replaced by `d2b transport status` and Provider-managed lifecycle; live integration tests updated to use Credential resources in config |

Removal proof is not self-issued; it requires the matching Provider dossier's
integration wave to record the removal in its panel and changelog.

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass —
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.
