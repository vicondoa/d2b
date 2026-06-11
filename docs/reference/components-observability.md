# `nixling.observability.*` / `nixling.vms.<vm>.observability.*`

> Reference for the `observability` component surface.
> Option source: [`nixos-modules/options-observability.nix`](../../nixos-modules/options-observability.nix), [`nixos-modules/options-vms.nix`](../../nixos-modules/options-vms.nix)
> Manifest contract: [manifest schema](./manifest-schema.md)
> Implementation modules: [`nixos-modules/observability-vm.nix`](../../nixos-modules/observability-vm.nix), [`nixos-modules/components/observability/{host,guest,stack}.nix`](../../nixos-modules/components/observability/)

## Overview

nixling observability is an opt-in telemetry subsystem: set `nixling.observability.enable = true` to turn on the host-side forwarding/exporter layer and reserve the auto-declared `sys-obs-stack` VM, then set `nixling.vms.<vm>.observability.enable = true` on each monitored workload VM to attach that VM's guest Alloy agent and OTLP relay path into the stack. The default-off invariant is part of the public surface: when the host flag is left at `false`, the observability env, stack VM, and per-VM telemetry sidecars are out of scope; when the host flag is `true`, the subsystem spans the host, the auto-declared `sys-obs-stack` VM, and one per-monitored-VM guest Alloy agent.

## Architecture diagram

```text
workload VM <vm>
┌──────────────────────────────────────────────────────────────────────┐
│ guest Alloy                                                         │
│   │                                                                  │
│   ▼                                                                  │
│ /run/nixling/otlp.sock            guest-local OTLP receiver          │
│   │                                                                  │
│   ▼                                                                  │
│ /run/nixling/otlp-egress.sock     guest relay handoff               │
│   │                                                                  │
│   ▼                                                                  │
│ nixling-otel-vsock-out.service    socat                             │
└───┬───────────────────────────────────────────────────────────────────┘
    │ AF_VSOCK: CID 2, port 14317
    ▼
host
┌──────────────────────────────────────────────────────────────────────┐
│ nixling-otel-relay@<vm>.service   socat                             │
│   │                                                                  │
│   ├─ workload backend: /var/lib/nixling/vms/<vm>/vsock.sock         │
│   └─ obs backend:      /var/lib/nixling/vms/sys-obs-stack/vsock.sock│
└───┬───────────────────────────────────────────────────────────────────┘
    │
    ▼
obs VM: sys-obs-stack
┌──────────────────────────────────────────────────────────────────────┐
│ nixling-otel-vsock-in.service     socat                             │
│   │                                                                  │
│   ▼                                                                  │
│ /run/nixling/obs-ingress.sock                                        │
│   │                                                                  │
│   ▼                                                                  │
│ obs Alloy                                                            │
│   ├─▶ Prometheus                                                     │
│   ├─▶ Loki                                                           │
│   └─▶ Tempo                                                          │
└──────────────────────────────────────────────────────────────────────┘

parallel host path
┌──────────────────────────────────────────────────────────────────────┐
│ host Alloy                                                           │
│   │                                                                  │
│   ▼                                                                  │
│ /run/nixling/host-otlp.sock                                          │
│   │                                                                  │
│   ▼                                                                  │
│ /run/nixling/host-egress.sock                                        │
│   │                                                                  │
│   ▼                                                                  │
│ nixling-otel-host-bridge.service                                     │
└───┬───────────────────────────────────────────────────────────────────┘
    │ same obs-VM backend socket on the host
    ▼
/var/lib/nixling/vms/sys-obs-stack/vsock.sock
    │
    ▼
nixling-otel-vsock-in.service → /run/nixling/obs-ingress.sock → obs Alloy
```

The observability data path is vsock and Unix sockets end-to-end. It does not traverse `network.nix`'s IP routing path.

## Option reference

The tables below copy the live committed schema. Types, defaults, and descriptions follow the current option declarations.

### Host-level options under `nixling.observability.*`

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.observability.enable` | bool | `false` | Enable the auto-declared observability VM, host forwarders/exporters, and per-VM guest telemetry sidecars. |
| `nixling.observability.env` | str | `"obs"` | Name of the auto-declared observability env. When observability is enabled, the framework materialises `nixling.envs.<env>` from this value. |
| `nixling.observability.vmName` | str | `"sys-obs-stack"` | VM name reserved for the auto-declared observability stack VM. |
| `nixling.observability.index` | int | `10` | Workload-style LAN index reserved for the observability stack VM inside `lanSubnet`. |
| `nixling.observability.lanSubnet` | str | `"10.40.0.0/24"` | LAN CIDR for the auto-declared observability env. |
| `nixling.observability.uplinkSubnet` | str | `"203.0.113.0/30"` | Host↔observability-stack point-to-point CIDR for the auto-declared observability env. |
| `nixling.observability.retention.metrics` | str | `"30d"` | Retention window for metrics in the observability stack. |
| `nixling.observability.retention.logs` | str | `"14d"` | Retention window for logs in the observability stack. |
| `nixling.observability.retention.traces` | str | `"7d"` | Retention window for traces in the observability stack. |
| `nixling.observability.grafana.listenAddress` | str | `"10.40.0.10"` | Address Grafana binds inside the observability env. The default tracks the observability VM's derived IP (`lanSubnet` + `index`). |
| `nixling.observability.grafana.listenPort` | port | `3000` | TCP port Grafana listens on inside the observability env. |
| `nixling.observability.grafana.secretKeyFile` | path or null | `null` | Optional file containing Grafana's session signing secret. When null, framework generates a per-install secret on the **host** at `${nixling.site.stateDir}/observability/grafana-secret-key` (mode 0400 root:root) and shares it read-only into `sys-obs-stack` at `/run/nixling-obs-secrets/grafana-secret-key`. When set, the path is loaded via systemd LoadCredential. Use this to source the secret from sops-nix, agenix, or another declarative secrets framework. |
| `nixling.observability.grafana.adminPasswordFile` | path or null | `null` | Optional file containing Grafana's `nixling-admin` user password. When null, framework generates a per-install password on the **host** at `${nixling.site.stateDir}/observability/grafana-admin-password` (mode 0400 root:root) and shares it read-only into `sys-obs-stack` at `/run/nixling-obs-secrets/grafana-admin-password`. Host operators can read it directly via `sudo cat <path>` — no cross-VM SSH required. When set, the path is loaded via systemd LoadCredential. Use this to source the password from sops-nix, agenix, or another declarative secrets framework. |
| `nixling.observability.grafana.anonymousViewer.enable` | bool | `false` | Opt-in: allow unauthenticated Viewer access to Grafana. **Only use on trusted single-host LAN deployments** — anyone reachable to the obs VM's Grafana port can read all telemetry without authentication. Default is `false` (auth required as `nixling-admin`). |
| `nixling.observability.ch.exporter.enable` | bool | `true` | Enable the host-side Cloud Hypervisor metrics exporter. |
| `nixling.observability.ch.exporter.listenPort` | port | `9101` | Loopback port the host-side Cloud Hypervisor metrics exporter listens on. |
| `nixling.observability.ch.exporter.includeTopologyLabels` | bool | `false` | Opt-in: include `bridge`, `tap`, `tpm`, `graphics`, `audio`, `usbip_yubikey` labels on emitted CH metrics. Default `false` to keep the security-posture surface narrow; enable for debug. |
| `nixling.observability.alerts.<name>.enable` | bool | `true` | Per-alert toggle. The 8 default alerts (NixlingVMDown, NixlingNetVMDownWithRunningWorkloads, NixlingObsVMUnreachableFromHost, NixlingVsockRelayDown, NixlingCHAPISocketMissing, NixlingStoreSyncFailure, NixlingGuestTelemetryMissing, NixlingObsVMStackUnhealthy) can be individually disabled by setting `<name>.enable = false`. Disabled alerts are omitted from the generated rule file entirely. |
| `nixling.observability.cli.traces.enable` | bool | `true` | Include OpenTelemetry trace helpers in the `nixling` CLI. |
| `nixling.observability.transport.relayPackage` | package | `pkgs.socat` | Package providing the observability byte-relay binary. Must expose a `bin/socat`-compatible CLI because nixling passes socat-specific arguments today; defaults to `pkgs.socat`. A future stable relay interface may replace this contract, but the socat-compatible path will stay supported for at least one minor release after that lands. |
| `nixling.observability.transport.relayPackage` | package | `pkgs.socat` | Package providing the observability byte-relay binary. Must expose a `bin/socat`-compatible CLI because nixling passes socat-specific arguments today; defaults to `pkgs.socat`. v0.3.0 will define a stable relay-binary interface. |
| `nixling.observability.transport.relayPackage` | package | `pkgs.socat` | Package providing the observability byte-relay binary. Must expose a `bin/socat`-compatible CLI because nixling passes socat-specific arguments today; defaults to `pkgs.socat`. That compatibility requirement remains in force for the current transport. When `nixling-otel-relay` lands, nixling will add a dedicated relay interface first and keep `bin/socat` compatibility for at least one minor release with CHANGELOG-guided migration notes before removal. |

### Per-VM options under `nixling.vms.<vm>.observability.*`

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.vms.<vm>.observability.enable` | bool | `false` | Enable the guest Alloy agent and reverse OTLP tunnel for this workload VM. |
| `nixling.vms.<vm>.observability.scrapeJournal` | bool | `true` | Whether the observability guest component should scrape this VM's journald stream. |
| `nixling.vms.<vm>.observability.scrapeNodeMetrics` | bool | `true` | Whether the observability guest component should scrape this VM's node/system metrics. |

### Per-VM audit forwarding under `nixling.vms.<vm>.audit.*`

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.vms.<vm>.audit.enable` | bool | `false` | Enable guest-side `auditd` forwarding for this VM. Requires `nixling.vms.<vm>.observability.enable = true` on the same VM. |
| `nixling.vms.<vm>.audit.rules` | list of strings | curated ruleset | Guest audit rules passed to `security.audit.rules`. The default watches `/etc/passwd`, `/etc/shadow`, and `/etc/sudoers`; it intentionally omits syscall-heavy rules such as `execve`/`connect` because those records frequently carry command-line secrets and executable paths. |

When audit forwarding is enabled, the guest path is:

1. `auditd` runs in the workload VM with `audisp-syslog` active.
2. `audisp-syslog` forwards audit events into journald.
3. The guest Alloy config tails journald with
   `_TRANSPORT=syslog SYSLOG_IDENTIFIER=audisp-syslog` and labels the
   stream `source="audit"`, `unit="audisp-syslog"`, `vm`, and `env`.
4. The existing reverse OTLP/vsock transport carries that stream into
   the observability stack VM and on to Loki.

## Port and CID allocation

The observability transport has its own CID/port/path contract. Host-side vsock backend sockets follow the live manifest layout under `/var/lib/nixling/vms/<vm>/...`.

| Resource | Value | Owner |
|---|---|---|
| Host vsock CID | `2` (kernel-fixed) | kernel |
| Obs VM vsock CID | `1000` | framework |
| Env VM vsock CID | `100 + envIndex * 1000 + slot` (`envIndex` = 0-based lexicographic position in `lib.attrNames config.nixling.envs`; `slot = 1` for the env net VM and `slot = nixling.vms.<vm>.index` for workload VMs) | framework |
| Vsock service port | `14317` | host relay listener / obs receiver |
| Grafana TCP | `cfg.grafana.listenPort` (default `3000`), bound to `cfg.grafana.listenAddress` | obs VM |
| CH exporter TCP | `cfg.ch.exporter.listenPort` (default `9101`), bound to `127.0.0.1` | host |
| Guest OTLP UDS | `/run/nixling/otlp.sock` | guest |
| Guest OTLP egress UDS | `/run/nixling/otlp-egress.sock` | guest |
| Host OTLP UDS | `/run/nixling/host-otlp.sock` | host |
| Host egress UDS | `/run/nixling/host-egress.sock` | host |
| Obs VM ingress UDS | `/run/nixling/obs-ingress.sock` | obs VM |
| Workload VM vsock backend socket | `/var/lib/nixling/vms/<vm>/vsock.sock` | host (Cloud Hypervisor creates it) |
| Obs VM vsock backend socket | `/var/lib/nixling/vms/<cfg.vmName>/vsock.sock` | host (Cloud Hypervisor creates it) |

Manifest v3 keeps a deterministic md5-based fallback CID for env-less legacy VMs so the always-emitted `observability` block stays populated, but env-backed VMs use the formula above. The per-VM `observability.vsockHostSocket` field is the base Cloud Hypervisor vsock socket; guest-to-host OTLP traffic uses the suffixed `<base>_14317` listener.

## Naming conventions

| Kind | Pattern | Example |
|---|---|---|
| System-declared VM | `sys-<env>-<role>` | `sys-obs-stack` |
| Per-VM templated host unit | `nixling-<role>@<vm>.service` | `nixling-otel-relay@work-aad.service` |
| Global host singleton unit | `nixling-<role>.service` | `nixling-ch-exporter.service`, `nixling-otel-host-bridge.service` |
| In-VM unit (guest or obs) | plain `nixling-<role>.service` | `nixling-otel-vsock-out.service`, `nixling-otel-vsock-in.service` |
| Per-VM state | `/var/lib/nixling/vms/<vm>/...` | `/var/lib/nixling/vms/work-aad/vsock.sock` |
| Per-VM runtime (in-VM) | `/run/nixling/...` | `/run/nixling/otlp.sock` |

**Service names**: nixling-defined services use `nixling-<role>.service`
(templated: `nixling-<role>@<vm>.service`). Services declared via
upstream NixOS modules (`services.alloy`, `services.grafana`, etc.)
keep their upstream names. This matches the existing precedent for
`pipewire.service` (audio), `swtpm@<vm>.service`, and
`microvm@<vm>.service`.

## Systemd units

nixling-defined observability sidecars follow the v0.1.7 H7 lifecycle
rule: `restartIfChanged = false` is set at the top level of the service
definition, not as `unitConfig.X-RestartIfChanged`. That invariant does
not extend to upstream NixOS services. `alloy.service`,
`grafana.service`, `prometheus.service`, `loki.service`, and
`tempo.service` keep their upstream nixpkgs defaults (currently
`restartIfChanged = true`).

| Unit | Scope | Restart | `restartIfChanged` | What it does |
|---|---|---|---|---|
| `nixling-otel-relay@<vm>.service` | host | `on-failure` | `false` (top-level) | Bridges one monitored workload VM's host-side vsock backend socket to the obs VM's host-side vsock backend socket. |
| `nixling-otel-host-bridge.service` | host | `on-failure` | `false` (top-level) | Bridges the host Alloy egress path into the obs VM without using IP transport. |
| `nixling-ch-exporter.service` | host | `on-failure` | `false` (top-level) | Polls each VM's Cloud Hypervisor API socket and exposes Prometheus text on loopback for host Alloy to scrape. |
| `alloy.service` | host | `always` | `true` (upstream default) | Host forwarder that scrapes host journald, node metrics, and selected systemd-unit metrics, then forwards upstream to the obs stack. |
| `nixling-otel-vsock-out.service` | workload guest | `on-failure` | `false` (top-level) | Bridges the guest-side OTLP Unix socket into AF_VSOCK toward host CID `2`, port `14317`. |
| `alloy.service` | workload guest | `always` | `true` (upstream default) | Guest-local Alloy agent that scrapes journald and node metrics and receives in-guest OTLP traffic. |
| `nixling-otel-vsock-in.service` | obs VM | `on-failure` | `false` (top-level) | Receives AF_VSOCK traffic on port `14317` and forwards it to the obs VM's ingress UDS. |
| `alloy.service` | obs VM | `always` | `true` (upstream default) | Central Alloy receiver for workload-VM and host telemetry. |
| `grafana.service` | obs VM | `on-failure` | `true` (upstream default) | Dashboard and query UI backed by provisioned Prometheus, Loki, and Tempo datasources. |
| `prometheus.service` | obs VM | `always` | `true` (upstream default) | Metrics storage and query engine. |
| `loki.service` | obs VM | `always` | `true` (upstream default) | Log storage and query engine. |
| `tempo.service` | obs VM | `always` | `true` (upstream default) | Trace storage and query backend. |

> Note: The H7 invariant applies to nixling-defined sidecars only;
> upstream services follow nixpkgs conventions.

## Security boundaries

### Vsock attack surface

The additional device surface is the kernel virtio-vsock path. That is not a new trust class for nixling: the framework already relies on other virtio devices (`net`, `blk`, `fs`, `snd`, `gpu`). The observability vsock device is treated as the same kind of host-mediated paravirtual device, not a qualitatively different exposure.

### Host as trust broker

The host is already the trust broker for VM-adjacent sidecars such as `virtiofsd`, `swtpm`, and audio mediation. The observability relays keep that posture: they add a byte relay on the host, but they do not create a new trust tier. Compromise of the host already implies compromise of every VM; observability does not change that baseline.

### No cross-VM credentials

The observability stack VM holds no credentials that grant access to monitored workload VMs. There is no reverse SSH tunnel, no per-VM `authorized_keys` material for telemetry transport, and no workload-VM credential copied into the stack. Compromise of the obs VM therefore does not itself grant access to workload VMs.

### Network policy unchanged

`network.nix` stays untouched. Existing `hostBlocklist` handling and deny-by-default outbound policy continue to apply exactly as before. The observability transport bypasses IP entirely by using vsock plus Unix sockets, so no new LAN/uplink firewall exception is part of the design.

The auto-declared `obs` env (lanSubnet `10.40.0.0/24`, uplinkSubnet `203.0.113.0/30`) and the framework-owned `sys-obs-net` VM nevertheless go through the same per-env net-VM contract as user-declared envs. `tests/net-vm-network-eval.sh` pins that contract end to end (see the case-10 block in its header doc): `sys-obs-net` must derive its `10-uplink`/`10-lan` static addresses from the env CIDRs, must drop every peer env LAN/uplink CIDR before the broad LAN -> uplink accept (and reciprocally, peer envs must drop the obs CIDRs), must keep `30-lan-obs` bridge `Isolated = true`, and must NOT acquire the MSS-clamp or LAN-to-LAN forward rules — enabling observability cannot become a hidden east-west tunnel between previously-isolated envs.

### Attribute hygiene

Telemetry labels and attributes are an explicit allowlist. Permitted examples are `vm.name`, `vm.env`, `vm.role`, `nixling.subcommand`, `systemd.unit`, `tap`, `bridge`, `static_ip`, and `generation`. Forbidden payload includes SSH key paths, command output, Nix derivation paths, and any Entra-, TPM-, or audio-user-specific data.

## Label conventions

### Prometheus

Prometheus labels use `snake_case`: for example `vm`, `env`, `role`, `usbip_yubikey`, `bridge`, and `tap`.

### Journal / Loki disambiguation

Host Alloy, each workload-VM guest Alloy, and the obs-VM Alloy all log
as `unit=alloy.service` because nixling preserves the upstream unit
name. When querying logs, filter on both `host` and `vm` labels to
separate the host forwarder, workload-VM guest agents, and the
`sys-obs-stack` receiver; the `loki.source.journal` pipelines attach
those labels alongside `unit`.

### OTel span/trace attributes

OpenTelemetry span and trace attributes use dot-notation aligned with OTel semantic-convention style: for example `vm.name`, `vm.env`, `vm.role`, `nixling.subcommand`, and `systemd.unit`.

## Retention defaults

| Signal | Default | Notes |
|---|---|---|
| Metrics | `30d` | Mirrors `nixling.observability.retention.metrics`. |
| Logs | `14d` | Mirrors `nixling.observability.retention.logs`. |
| Traces | `7d` | Mirrors `nixling.observability.retention.traces`. |
| Profiles | not enabled | Profiles are not part of v0.2.0. |

## Dashboard inventory

`sys-obs-stack` provisions 6 dashboards in Grafana's **Nixling** folder:

| Title | UID | Folder | Refresh | Purpose |
|---|---|---|---|---|
| Nixling Overview | `nixling-overview` | Nixling | `30s` | VM state, CH API up, vsock relay health |
| VM Resources | `nixling-vm-resources` | Nixling | `30s` | Per-VM CPU/mem/FS/net + CH counters |
| Lifecycle Traces | `nixling-lifecycle-traces` | Nixling | `30s` | Reserved Tempo dashboard for `nixling vm start/down/switch/...`; populated only when `otel-cli` is pointed at a reachable OTLP endpoint. |
| Logs | `nixling-logs` | Nixling | `30s` | Loki filtered by vm/env/unit/severity |
| Per-VM Store | `nixling-per-vm-store` | Nixling | `30s` | Generation, sync result, path count |
| Obs VM Health | `nixling-obs-vm-health` | Nixling | `30s` | Stack self-health, disk, ingestion rates |

## Alerting

`sys-obs-stack` provisions 8 default Prometheus alert rules via `services.prometheus.ruleFiles`:

| Name | Severity | Expr summary | Threshold |
|---|---|---|---|
| `NixlingVMDown` | warning | `up == 0` | `for 5m` |
| `NixlingNetVMDownWithRunningWorkloads` | critical | net VM down + workload up | composite |
| `NixlingObsVMUnreachableFromHost` | warning | obs VM CH API unreachable | `for 10m` |
| `NixlingVsockRelayDown` | warning | `nixling-otel-relay@` failing | `for 3m` |
| `NixlingCHAPISocketMissing` | warning | VM running but CH API down | `for 2m` |
| `NixlingStoreSyncFailure` | warning | Loki: store-sync FAIL in 10m | rate-based |
| `NixlingGuestTelemetryMissing` | info | absent scrape timestamp | `for 10m` |
| `NixlingObsVMStackUnhealthy` | critical | `up{job=...} == 0` for any stack service | `for 5m` |

The shipped rules combine host-side CH-exporter gauges, host Alloy's `systemd-units` collector, guest telemetry heartbeat metrics, and local self-scrapes of Grafana/Prometheus/Loki/Tempo/Alloy inside the obs VM.

Notification channels are deliberately left unconfigured. Operators decide whether to route alerts through Alertmanager, Grafana contact points, or another downstream system.

Disable individual default alerts by setting
`nixling.observability.alerts.<AlertName>.enable = false`. The
toggle is honored at rule-file generation time, so disabled
alerts are absent from the rendered Prometheus rule file
entirely. The 8 default alerts are listed in the Alerts table
above.

### Deferred

`NixlingVMStuckWithoutSSH` is deferred to v0.3.0; the host exporter does not yet expose the proposed `nixling_vm_ssh_ready` metric.

## Disabling individual signals

- Disable guest journald scraping per VM with `nixling.vms.<vm>.observability.scrapeJournal = false`.
- Disable guest node/system metrics per VM with `nixling.vms.<vm>.observability.scrapeNodeMetrics = false`.
- Disable CLI lifecycle traces host-wide with `nixling.observability.cli.traces.enable = false`.
- Disable the host Cloud Hypervisor exporter with `nixling.observability.ch.exporter.enable = false`.

## See also

- `docs/how-to/enable-observability.md` — step-by-step enablement,
  verification, and troubleshooting for the shipped v0.2.0 stack.
- [design.md](../explanation/design.md) — design rationale for the
  shipped single-host observability path, including vsock vs
  reverse-SSH.
- [manifest schema](./manifest-schema.md)
