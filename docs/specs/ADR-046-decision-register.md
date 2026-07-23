# ADR 0046 decision register

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-decision-register` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | ADR 0046 integrator |
| Depends on | None |
| Supersedes | None |

This register records resolved architecture choices for the ADR 0046 normative
set. It contains design decisions and rationale, not conversation transcripts,
review metadata, or implementation status.

## Resolved decisions

| ID | Decision | Rationale and consequences | Affected specs |
| --- | --- | --- | --- |
| D001 | ADR 0046 is authored and implemented from protected `v3` at `b5ddbed6`, the final pre-ADR-0045 commit. | Main is not ancestry or current-state evidence, but any useful main-branch implementation may be copied/adapted through explicit reuse work items and independently integrated/validated against v3 contracts. | All |
| D002 | Keep number 0046 for cross-branch historical clarity. | The v3 branch has no ADR 0045, but the number distinguishes this replacement design from main-branch ADR 0045. | Parent, all specs |
| D003 | Reject kcp as the runtime substrate. | The spike proved desired object semantics but measured about 490 MiB RSS and a 176 MiB executable. v3 retains semantics without Kubernetes wire compatibility. | Resource store/API/reconciliation |
| D004 | Use one redb database per Zone. | redb supplies embedded ACID transactions, one writer with concurrent MVCC readers, crash safety, and a pure-Rust deployment shape. d2b owns schemas, indexes, watches, RBAC, operations, and routing. | Resource store |
| D005 | Use a single writer with optimistic conflicts. | Concurrent clients are accepted, but commits serialize. Every mutation has an expected revision; a stale client receives conflict and may re-read/retry. | Resource object/store/API/reconciliation |
| D006 | Embed the redb store and resource service in the Zone runtime. | This minimizes memory/process overhead and gives one clear owner. Provider/controller processes never access redb directly. | Resource store, process model |
| D007 | One fixed core-controller process per Zone hosts all trusted core controller handlers. | Separate handlers keep contracts explicit while one process avoids per-controller memory/process overhead. It uses d2b-bus like every other controller. | Core controllers, process model |
| D008 | Resource-plane acceptance is aggregate idle RSS <=64 MiB, readiness <=500 ms, p95 local reads <=2 ms, and p95 crash-safe writes <=10 ms on the pinned fixture. | The aggregate includes Zone resource service/store plus mandatory system-core and system-minijail controller processes. Optional Provider processes have separate budgets. | Resource store, validation |
| D009 | Controllers own their watch/coalescing queue and retry decisions; core minimizes unnecessary runs. | Core validates watch plans, maintains reverse indexes, suppresses irrelevant/converged/self-status events, coalesces revisions, and emits bounded hints. Provider logic remains independent. | Reconciliation |
| D010 | Every primitive is a complete low-level ResourceSpec. | Helpers, broker ops, syscalls, and sandbox fragments are implementation mechanisms, not primitives. Provider controllers compose behavior by creating primitive resources. | Resource object, primitive composition |
| D011 | d2b-bus over ComponentSession and a d2b transport is the only controller control-plane channel. | All get/list/watch/mutate/status/hint/checkpoint traffic uses the same authenticated route. There is no direct store, HTTP, or ambient IPC shortcut. | Bus/session/transport, resource API |
| D012 | Every Provider is one independently buildable crate/package; a Provider crate may build multiple process binaries. | The boundary supports separate controller/service/worker processes now and later extraction to one GitHub repository per Provider. | Provider model, every Provider dossier |
| D013 | ADR 0046 is a concise parent plus a manifest-bound `docs/specs/ADR-046-*` set. | The design is too large for one reviewable ADR. All specs are normative and accepted together. | Parent, all specs |
| D014 | Use one ADR/spec PR with user review before the final panel and again after it. | The panel runs only after the first approval. Any content change invalidates validation/panel evidence and repeats the final review. | Validation/delivery |
| D015 | Every field ending in `Ref` names another same-Zone resource using `<ResourceType>/<resource_name>`. | Plain enums use plain names. Standard ResourceTypes are Zone-unique; vendor types are qualified; API binding collisions fail. | Resource object/API, all resource specs |
| D016 | Every resource belongs to exactly one Zone. | Refs never resolve across Zones. A Provider must be installed as a Zone-local `Provider/<name>` resource before use. Cross-Zone resource relationships require separate review. | Zone, Provider, routing, resource API |
| D017 | Every Zone store has exactly one authoritative `Zone/<zone-name>` self resource; parents use local `ZoneLink/<name>` resources. | Parent access uses the child Zone API; a parent does not mount or mirror the child store. | Zone resources/routing |
| D018 | Physical/local host execution contexts are `Host/<name>` resources; VM, sandbox, cloud, and remote execution contexts are `Guest/<name>` resources. | A Zone may have multiple Hosts reconciled by Provider/system-core and multiple Guests reconciled by runtime Providers. Process/EphemeralProcess uses one executionRef to either type. | Host/Guest, Provider model, process placement |
| D019 | Host and Guest share one ExecutionPolicy with plain `system|user` domains. | `defaultDomain`, `allowedDomains`, `defaultUserRef`, budgets, and attachment defaults are common. Process uses executionRef, optional domain, and conditional userRef; user-only Hosts/Guests reject system processes. | Host/Guest/Process specs |
| D020 | Resource ownership is singular. | Every resource has zero or one ownerRef. Any child mutation triggers owner reconciliation; owner cycles fail; deletion is child-first; UID binding prevents name-reuse confusion. | Resource object/store/reconciliation |
| D021 | `Provider/system-core`, `Provider/system-systemd`, and `Provider/system-minijail` are distinct. | `system-core` implements Host plus local User discovery/status only. systemd and minijail are interchangeable Process/EphemeralProcess implementations for Host or Guest execution. Volume, Network, Device, and Credential use separate Providers. | Resource catalog, Provider dossiers |
| D022 | Pidfd is mandatory local Process Provider behavior, not a ResourceSpec. | Minijail receives pidfd from clone3 and owns wait/reap. systemd binds InvocationID/cgroup/MainPID/start-time, opens pidfd, while systemd owns wait/reap. Pidfds never persist or cross the bus. | Process spec, Process Providers |
| D023 | Provider/controller descriptors declare supported Host/Guest Provider capabilities and Process domains. | ResourceTypes are declared once. Controller instances may run under several executionRef targets without host/guest-specific Process schemas. | Provider model, reconciliation, primitive composition |
| D024 | The initial task output is documentation only. | ADR/specs, manifests, indexes, changelog/instructions, PR, validation, and panel evidence are delivered. Future W0-W10 implementation requires a separate request. | Validation/delivery |
| D025 | Spec writing is parallel only after the shared foundation is stable. | File-disjoint Resource, core-controller, cross-cutting, and Provider dossier agents launch concurrently. Agents return decision-required rather than inventing shared contracts. | Validation/delivery |
| D026 | Standardize on ResourceType/ResourceSpec terminology and top-level `type`; do not use Kubernetes-style ResourceKind/kind terminology. | The vocabulary matches the native d2b model and canonical `<ResourceType>/<resource_name>` refs. | All resource/controller specs |
| D027 | Every resource has `metadata`, `spec`, and `status`; common identity/Zone/revision/timestamps live in metadata. | Even resources with no desired fields carry `spec: {}`. Status is a separately authorized controller-owned subresource. | Resource object/store/API, every ResourceType |
| D028 | Status stores the latest bounded observation, not an embedded history. | Common status has numeric observedGeneration, phase, conditions, RFC 3339 timestamps, stable outcome code, optional exitCode, detailed bounded/redacted message, retryability, and ResourceType-specific typed fields. Earlier values remain in revision_log until compaction. | Resource object/store/API/reconciliation |
| D029 | The parent title is `ADR 0046: d2b 3.0 Provider control plane`. | It names the release and central architecture without exposing implementation-specific storage terminology. | Parent ADR/index |
| D030 | Reconciliation APIs and controller loops are asynchronous with hard local reaction targets. | Durable commit-to-controller-handler p95 is <=5 ms and ready Process commit-to-launch-attempt p95 is <=20 ms. Watch reception never waits for a running effect; independent resources reconcile/start concurrently within budgets; no polling/debounce delay. | Resource store/API/reconciliation, Process Providers, validation |
| D031 | Use `Process` for long-lived processes and `EphemeralProcess` for one-shot asynchronous processes. | They share a common execution spec and pidfd contract. EphemeralProcess reports terminal status directly and never references/creates a Process child merely to run. | Process resources/Providers, reconciliation |
| D032 | Replace File/Directory/ACL/FilesystemView ResourceTypes with one Volume ResourceType. | Volume has anchored relative layout entries, fine-grained owner/mode/access/default ACLs and lifecycle rules, named views, and same-Zone Host/Guest attachments. Process mounts select a volumeRef/view. | Volume, Process, storage migration |
| D033 | Volume supports Host-to-Guest transports such as virtiofs. | The Volume source may use an explicitly authorized Host path under its Provider policy; attachments name target executionRef, transport, mount path, view/access, and settings. The controller may own a Host virtiofsd Process and reports per-attachment export/mount status. | Volume/Host/Guest/Process specs and Providers |
| D034 | EphemeralProcess has outcome-specific terminal retention. | `successfulTtl` defaults to `1h`; `failedTtl` defaults to `24h`. TTL begins at status.completedAt and cleanup remains revision/finalizer/incident-hold safe. | EphemeralProcess, core cleanup controller, validation |
| D035 | Freeze the minimal standard ResourceType catalog. | Standard types are Zone, ZoneLink, Provider, Role, RoleBinding, Quota, EmergencyPolicy, Host, Guest, Process, EphemeralProcess, Volume, Network, Device, User, and Credential. Other behavior is inline or Provider-specific. | All resource/provider specs |
| D036 | The fixed core-controller process is also the `Provider/system-core` controller. | It creates/reconciles the first Host and Users and is one of two fixed Provider bootstrap exceptions. | Core controllers, system-core Provider, process/bootstrap |
| D037 | The universal status phase enum is `Pending`, `Ready`, `Succeeded`, `Degraded`, `Failed`, `Deleted`, or `Unknown`. | Conditions carry starting/deleting/retrying and other detail without multiplying common phases. | Resource object/API/store, all controllers |
| D038 | Final deletion has no retained resource tombstone. | After finalizers complete, core emits a Deleted status revision and removes the resource immediately. revision_log is the only deletion history; GET returns not found. | Resource store/API/reconciliation |
| D039 | ComponentSession uses Noise-based authenticated/record-protected profiles. | Copy/adapt the proven `d2b-session` and `d2b-session-unix` implementation from main, then reversion, integrate, and validate it against v3 Zone/resource/RBAC contracts. | ComponentSession/bus/security |
| D041 | Main is an unrestricted implementation reuse source, not the v3 baseline. | Work items may copy/adapt any useful main code. Each names the exact main commit/file/symbol/tests, proves the selected behavior, maps it to exact v3 destinations, and excludes unrelated ADR 0045 architecture. | Every implementation work item |
| D042 | Unsafe-local is not a separate v3 Provider. | It becomes a user-only Host using Provider/system-core, defaultDomain=user, allowedDomains=[user], and defaultUserRef. Its explicit no-isolation posture/warnings remain in Host status/UI; Processes use normal Process Providers. | Host/system-core/Nix/UI/reset |
| D043 | Freeze four Guest Provider families. | runtime-cloud-hypervisor, runtime-qemu-media, runtime-azure-container-apps, and runtime-azure-virtual-machine implement Guest. | Guest/Provider dossiers |
| D044 | Freeze two Volume Provider families. | volume-local owns anchored local storage/layout/ACL/views; volume-virtiofs owns same-Zone host-source→Guest virtiofs attachments and virtiofsd Processes. | Volume/Provider dossiers |
| D045 | Freeze one initial Network Provider. | network-local owns shared local fabrics; Azure/ACA networking remains in their Guest Providers until independently shared networking is required. | Network/Provider dossier |
| D046 | Freeze four Device Provider families. | device-tpm, device-usbip, device-security-key, and combined device-gpu (GPU/video) implement Device. Security-key owns unprivileged relay/frontend Processes; fixed broker only opens/passes hidraw. | Device/Provider dossiers |
| D047 | Freeze five interaction Provider families. | display-wayland, audio-pipewire, clipboard-wayland, notification-desktop, and shell-terminal own semantic resources/processes. Exec uses EphemeralProcess directly. | Interaction/Provider dossiers |
| D048 | Freeze three Credential Provider families. | credential-secret-service, credential-entra, and credential-managed-identity acquire/retain credentials and may deliver authorized token/signature bytes only through D055/D056 sensitive sessions. | Credential/Provider dossiers |
| D049 | Freeze transport/observability/activation Providers and omit an orchestrator. | transport-unix, transport-vsock, transport-azure-relay, observability-otel, and activation-nixos are initial Providers. Ordinary controllers perform composition; there is no orchestrator-standard Provider. | Provider dossiers, routing/telemetry/activation |
| D050 | Rename the non-host execution parent from Workload to Guest and add Host as a separate ResourceType. | Host excludes guests and is reconciled by system-core. Guest covers VM/sandbox/cloud/remote execution. Process/EphemeralProcess uses canonical executionRef to either Host or Guest; both share ExecutionPolicy. | All resource/provider/process/routing/Nix specs |
| D051 | Provider/system-minijail is the second fixed non-resource Provider/controller bootstrap process. | The first Process controller cannot itself depend on a Process controller. The fixed minijail controller launches/reconciles later Process resources, including the system-systemd controller. | Process/bootstrap/system Providers |
| D052 | Use compiled narrow bootstrap authorization before stored RBAC exists. | Only exact Provider/system-core and Provider/system-minijail subjects receive the closed initial recovery/config publication verbs. It is non-configurable and cannot be widened; normal stored Role/RoleBinding governs later work. | Resource API/authz/core startup/session |
| D053 | Use bounded group commit in the single writer. | Independently validated non-conflicting queued mutations may share one redb transaction/fsync and Zone revision with ordered ordinals; each caller receives its own result. Conflicting/dependent writes remain ordered/separate. | Resource store/watches/performance |
| D054 | Terminology/identity owns one shared AuthenticatedSubjectContext. | ComponentSession maps trusted transport/Noise/bootstrap evidence into it; d2b-bus/resource API consume it for Role/RoleBinding evaluation. No spec defines a parallel identity context. | Identity/session/bus/resource API |
| D055 | Credential Providers may deliver raw tokens or SignChallenge signatures to an authorized consumer Provider only over a dedicated end-to-end ComponentSession. | Sensitive bytes remain absent from resources/store/status/audit/telemetry. d2b-bus/Zone/relay intermediaries authorize then forward opaque protected records without decryption. Provider-level consumerRef is sufficient; its signed descriptor/RBAC selects the receiving component. | Credential/session/bus/security |
| D056 | Raw token delivery requires fully enrolled Noise_KK peers. | NN local and IKpsk2 bootstrap sessions cannot carry raw tokens. The session binds Credential/consumer Provider/component generations, audience/operation, deadline, route, schema, limits, and transcript. | Credential/session/enrollment |
| D057 | Every ResourceType/Provider spec includes Nix authoring, canonical rendering, and NixOS eval/build schema/reference validation. | Nix emits an integrity-pinned Zone generation. Removing a configuration-managed resource activates the new generation and deletes it asynchronously through owner/finalizers with visible Degraded cleanup status; controller-managed resources are not broadly swept. | All resource/provider/Nix/configuration specs |
| D058 | Nix authoring mirrors ResourceSpec directly. | Users set type, optional metadata.ownerRef/presentation metadata, and exact canonical spec. metadata.name/zone derive from the Nix attr path, apiVersion defaults, and status/UID/generation/revision/timestamps/finalizers/managedBy/configurationGeneration are omitted/read-only. | Every ResourceType/Provider/Nix spec |
| D059 | Every Provider crate has `src/`, `tests/`, `integration/`, and `README.md`. | src owns implementation/unit tests; tests owns hermetic Cargo/conformance tests; integration owns heavier Host/Guest/container scenarios; README documents complete Provider usage/contracts. Missing paths fail policy. | Provider model, every Provider dossier, validation |
| D060 | User resource name and OS username are separate. | metadata.name is the canonical Zone-local key; spec.osUsername is the actual bounded OS username resolved by system-core. | User/Process/Volume ACL/Nix specs |
| D061 | network-local reconciles runtime networks dynamically. | Closed broker operations create/delete bridges and apply state; mDNS is an owned Process; USBIP Processes remain device-usbip-owned. | Network/broker/device/Nix specs |
| D062 | Freeze Volume source/sharing/security defaults. | volume-local supports block-image and bounded tmpfs; requested quotas are hard/fail-if-unsupported; bounds are 1024 layout/64 views/64 attachments; virtiofs is single-writer by default with explicit supported shared-write; symlinks are relative/in-Volume only; typed User ACL refs and continuous repair apply. | Volume/Guest/Process specs |
| D063 | Freeze Device arbitration/status defaults. | First probe failure => Unknown, absent after 3; render-node may share, full/VFIO exclusive; Device attachment is desired state; security-key/frontend and other workers are Processes; conservative broker/FD limits apply. | Device/Process/broker specs |
| D064 | Freeze CLI context/loading/output/cutover. | Global --zone with host-command exemptions; local standard ResourceType validation and live vendor validation; lazy per-invocation Provider CLI; remove v2 aliases at cutover; non-TTY defaults JSON. | CLI/docs/tests |
| D065 | Full OTEL SDK/exporter runs only in optional Provider/observability-otel Process. | Mandatory Zone/core processes use lightweight bounded emitters; telemetry outage degrades/drops telemetry without blocking startup; audit remains authoritative. | Telemetry/Provider/process/performance |
| D066 | Freeze Zone/Nix trust and generation defaults. | ZoneLink transport providerRef is explicit/no default; built-in official signing root plus additional per-Zone roots; retain 3 prior generations by default, configurable 1–16. | ZoneLink/Provider trust/Nix/configuration |
| D067 | Unsafe-local successor isolation posture is not an OTEL dimension. | It remains explicit in Host status, CLI/UI, and authoritative audit only; no metric label/span attribute/log field. | Host/CLI/audit/telemetry |
| D068 | SignChallenge signatures use the same sensitive KK delivery channel as tokens. | Outer response carries only outcome metadata; signature bytes are end-to-end protected and opaque to intermediaries. | Credential/session/bus/security |
| D069 | Core-managed metadata.managedBy is `configuration`, `controller`, or `api`. | Configuration cleanup deletes only omitted configuration-managed resources; controller manages owned children; API-managed resources persist until explicit API deletion. | Resource object/API/Nix/cleanup |
| D070 | Nix derivations live in a separate named `d2b.artifacts` catalog. | Provider/Guest ResourceSpecs use plain artifactId/systemArtifactId fields. Nix builds/hashes closures and emits a private ID→type/digest/closure catalog; store paths never enter public resources. | Nix/Provider/Guest/package/trust specs |
| D071 | Zone.spec is empty; Zone-wide quota and emergency control are separate ResourceTypes. | Quota owns aggregate ceilings/usage and may be referenced by Host/Guest/Process; EmergencyPolicy owns disable scopes/actions/status. | Zone/Quota/EmergencyPolicy/core/Nix specs |
| D072 | ZoneLink uses explicit transportProviderRef plus signed settings/Credential refs. | It has no default/inferred transport. | ZoneLink/routing/Nix/transport Providers |
| D073 | Freeze Provider/RBAC bounds. | Provider: 8 controllers, 8 services, 32 worker templates, 16 ResourceTypes/controller. Role: 32 rules; rule max 16 types, 16 verbs, 64 names, 32 executionRefs. RoleBinding: 128 subjects. | Zone-control/Provider/RBAC schemas/tests |
| D074 | RoleBinding has no expiry field. | Revocation/lifecycle uses normal spec update or deletion, avoiding time-derived authorization state. | RoleBinding/authz/Nix/API |
| D040 | ComponentSession and resource control use one authorization system. | Session authentication maps to a canonical resource subject; native Role/RoleBinding authorizes connect, invoke, stream, and resource verbs. Handshakes cannot self-assert roles; revision-bound leases revoke on policy change. | ComponentSession/bus/resource API/RBAC |

## Open decisions

No unresolved foundation decision is currently recorded. New ambiguity
must be added here before an author selects a normative answer.

## Implementation work items

This decision register has no future production implementation owner. Its
authoring work item is:

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-decisions-001` |
| Dependency/owner | ADR 0046 integrator |
| Current source | v3 evidence baseline plus parent/spec contradictions |
| Reuse action | adapt |
| Destination | `docs/specs/ADR-046-decision-register.md` |
| Detailed design | Record every evidence-underdetermined choice before dependent spec work proceeds |
| Integration | Parent decision summary and all affected specs link the decision ID |
| Data migration | None |
| Validation | Zero unresolved entries before pre-panel review; manifest/link consistency |
| Removal proof | Not applicable |
