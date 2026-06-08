# Changelog

All notable changes to nixling are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Pre-1.0 minor releases may break public APIs. When practical,
deprecations ship one minor release before removal.

## [0.2.0] - 2026-05-20

Minor release introducing the **observability subsystem**: a new
opt-in component category that provisions a single-host telemetry
sink VM (`sys-obs-stack`) wired over virtio-vsock — no IP between
the observer and the observed VMs, no shared SSH credentials. The
release ships per-VM Alloy agents, a Cloud Hypervisor metrics
exporter, host-side journald forwarding, 6 provisioned Grafana
dashboards, 8 Prometheus alert rules, and `otel-cli`-based CLI
lifecycle traces with local trace-id generation for Loki↔Tempo
correlation. Manifest schema bumped from version 1 to 2 to add the
`_observability` reserved sentinel and per-VM `observability`
block. A new `AGENTS.md` policy makes the panel-review process a
**hard gate** per phase for multi-phase plans.

### Added

- **Observability subsystem** (`nixling.observability.enable`,
  default `false`). When enabled, the framework auto-declares the
  `obs` env (default `lanSubnet = 10.40.0.0/24`,
  `uplinkSubnet = 203.0.113.0/30`) and the `sys-obs-stack` VM that
  runs Grafana + Prometheus + Loki + Tempo + a central Alloy OTLP
  receiver. Retention defaults: metrics 30d, logs 14d, traces 7d
  (all per-knob configurable via
  `nixling.observability.retention.{metrics,logs,traces}`).
- **Per-VM guest agent** (opt-in via
  `nixling.vms.<vm>.observability.enable`). Each monitored guest
  runs Alloy scraping node metrics + journald (each
  individually toggleable via
  `vm.observability.{scrapeJournal,scrapeNodeMetrics}`), receives
  in-VM OTLP on a UDS, and exports over virtio-vsock through the
  hardened `nixling-otel-vsock-out.service` (socat sidecar:
  `RestrictAddressFamilies=[AF_UNIX AF_VSOCK]`,
  `DeviceAllow=/dev/vsock`, `restartIfChanged=false` per v0.1.7 H7).
- **Host-side forwarder** (`services.alloy` on the host, forwarder
  mode, no storage). Scrapes nixling sidecar units' journald + node
  metrics + the loopback CH-exporter `/metrics`. Pushes all signals
  through `nixling-otel-host-bridge.service` to the obs VM.
- **Cloud Hypervisor metrics exporter**
  (`nixling-ch-exporter.service`, pure-Bash + jq + curl + socat —
  no new language runtime in the host closure). Polls each VM's CH
  REST socket (`/vmm.ping`, `/vm.info`, `/vm.counters`), exposes
  Prometheus text on `127.0.0.1:9101/metrics`. Counter allowlist
  pinned to Cloud Hypervisor v50 device IDs (`_net*`, `_disk*`,
  `_fs*`, `_pmem*`, `__rng`, `__balloon`, `__console`); unknown
  schema rolls into `nixling_vm_unknown_counters_total`. Topology
  labels (`bridge`, `tap`, `tpm`, `graphics`, `audio`,
  `usbip_yubikey`) are off by default to keep the security-posture
  surface narrow — flip
  `nixling.observability.ch.exporter.includeTopologyLabels` on for
  debug. Detects both `microvm@<vm>.service` and
  `nixling-<vm>-gpu.service` so graphics VMs are reported running.
- **Vsock transport** — no IP between VMs, no SSH credentials
  between observer and observed. Cloud Hypervisor `--vsock cid=N,...`
  is appended to every observability-enabled VM and to
  `sys-obs-stack`; a per-VM `nixling-otel-relay@<vm>.service` (socat
  host relay, `RestrictAddressFamilies=[AF_UNIX]`) stitches
  workload-VM vsock to obs-VM vsock at the host. Relay is wired
  via `microvm@%i.service.wants` for headless VMs and via
  per-VM `wants` on `nixling-<vm>-gpu.service` for graphics VMs
  (graphics VMs do not use `microvm@`).
- **CLI lifecycle telemetry** — `nixling up/down/switch/boot/test/
  rollback/gc/usb/audio` emit OTel spans via `otel-cli` and
  structured JSON journald events for every high-value lifecycle
  step. Spans are populated with allowed labels only (`vm.name`,
  `vm.env`, `vm.role`, `nixling.subcommand`, `systemd.unit`, `tap`,
  `bridge`, `static_ip`, `generation`) — never command output, key
  paths, or Nix store paths. `nl_span_start` generates `trace_id` +
  `span_id` locally via `/dev/urandom` so Loki↔Tempo correlation
  works even when no upstream OTLP collector endpoint is configured;
  honors otel-cli's traceparent when one is. `otel-cli` is
  module-time-gated into `runtimeInputs` via
  `nixling.observability.cli.traces.enable` (default `true`); hosts
  with observability disabled pay zero closure cost.
- **6 provisioned Grafana dashboards** under the "Nixling" folder:
  Nixling Overview, VM Resources, Lifecycle Traces, Logs, Per-VM
  Store, Obs VM Health. Default refresh 30s. Tempo→Loki
  trace-to-logs correlation via `derivedFields`.
- **8 Prometheus alert rules**: `NixlingVMDown`,
  `NixlingNetVMDownWithRunningWorkloads`,
  `NixlingObsVMUnreachableFromHost`, `NixlingVsockRelayDown`,
  `NixlingCHAPISocketMissing`, `NixlingStoreSyncFailure`,
  `NixlingGuestTelemetryMissing`, `NixlingObsVMStackUnhealthy`.
  Each rule individually toggleable via
  `nixling.observability.alerts.<name>.enable`. Notification
  channels are intentionally unconfigured — operators choose
  Alertmanager / Grafana contact-points.
- **Grafana auth**: defaults to authenticated access as
  `nixling-admin`. Password is generated at activation and stored
  at `/var/lib/nixling-observability/grafana-admin-password` inside
  `sys-obs-stack`, or sourced from sops/agenix via
  `nixling.observability.grafana.adminPasswordFile`. Session signing
  key follows the same pattern via
  `nixling.observability.grafana.secretKeyFile`. Anonymous Viewer
  is opt-in only for trusted single-host LANs via
  `nixling.observability.grafana.anonymousViewer.enable`; the login
  form remains available even in that mode.
- **Eval assertions**: vsock CID uniqueness across enabled VMs
  (reserved CID 1000 for `nixling.observability.vmName`),
  per-VM-without-framework rejection, reserved-prefix exemption for
  `cfg.vmName`, env uplink CIDR materialization check.
- **Tests**: `tests/observability-eval.sh` (20 cases, 1 promtool
  skip when absent — covers option schema, auto-declaration,
  CID allocation, per-VM toggle defaults, name/prefix collisions,
  CLI-traces closure gating, relay ACL wiring, stack VM guest
  surface, dashboard schema validation, rule-file `promtool`
  validation, metric-reference coverage, scrape-job exact-set,
  and the graphics-VM runner wiring path).
- **Examples**: `examples/with-observability/` minimal consumer
  flake validated by the per-example flake-check loop.
- **Docs**:
  - `docs/reference/components-observability.md` — option schema,
    port/CID/UDS table, naming conventions, systemd unit
    inventory, dashboard inventory, alert severity table,
    security boundaries, label conventions, retention defaults,
    opt-out paths.
  - `docs/how-to/enable-observability.md` — step-by-step recipe
    including sops/agenix examples for both the Grafana
    secret-key and admin-password.
  - `docs/explanation/design.md` — appended Observability section
    explaining the vsock-vs-reverse-SSH-vs-guest-init trade-off,
    the two-bridge necessity, the alternatives-considered list,
    CLI attribute hygiene, and the trust-concentration risk on
    the obs VM.
  - `docs/reference/manifest-schema.md` — `manifestVersion = 2`
    rationale.

### Changed

- **`manifestVersion` 1 → 2** (breaking under pre-1.0 minor-bump
  policy). The manifest now ships a top-level `_observability`
  reserved sentinel and a per-VM `observability` block
  (`enabled`, `vsockCid`, `vsockHostSocket`). Existing consumers
  who do not enable `nixling.observability.enable` see the new
  fields populated with `enabled = false` defaults — the
  manifest still describes their VMs deterministically.
- **`docs/reference/manifest-schema.{md,json}`** updated to
  describe the v2 schema.
- **AGENTS.md** adds a "Panel review" hard-gate policy: multi-phase
  plans must pass plan-review BEFORE implementation and work-review
  BEFORE phase advancement, with documented escape hatches for
  trivial, hotfix, and docs-only changes.

### Security

- Telemetry sidecar trust posture: dedicated locked system users
  (`nixling-otel-relay`, `nixling-otel-bridge`,
  `nixling-ch-exporter`) with execute-only ACLs on per-VM state
  directories and `rw` ACLs only on the per-port vsock sockets
  they need (`vsock.sock_14317`, not the base `vsock.sock`).
  Activation-time ACL refresh is idempotent and revokes stale
  grants when an observed VM is later disabled.
- `nixling-otel-acl-refresh` rejects symlinked state paths,
  validates resolved paths stay under the state root, and uses
  `setfacl --physical` when available — closes the TOCTOU
  window on a group-writable state tree.
- Grafana `secret_key` and admin password are never written to
  the world-readable Nix store. Both are generated atomically at
  activation (write-to-tmp + `mv -f`) and loaded via systemd
  `LoadCredential` into `/run/credentials/grafana.service/`, or
  sourced from operator-supplied files via
  `nixling.observability.grafana.{secretKeyFile,adminPasswordFile}`.
- Loki query selectors in shipped dashboards never default to a
  whole-namespace scan: every variable-driven selector requires
  a non-empty match (`.+`, not `.*`), and the trace-to-logs
  derivedField is scoped by trace-derived `vm`/`env` labels.
- Alert annotation templates carry `vm` and `env` only; full
  unit/job names stay inside dashboards (not exported to
  whichever notification backend an operator wires up).
- CLI span attribute extras are filtered through an allowlist
  in `nl_filter_attrs`: caller-supplied keys outside
  `{step, result, systemd_unit, tap, bridge, static_ip, generation,
  vm_role}` are dropped with a journald warning, as are values
  matching common secret/store-path patterns.
- The guest UDS→vsock relay is fork-bounded
  (`max-children=16`, `TasksMax=32`, `MemoryMax=64M`,
  `LimitNOFILE=1024`) to bound in-guest DoS surface.
- The host telemetry bridge runs as `alloy` with
  `SupplementaryGroups=[kvm]` (no over-broad `nixling-otel-host-bridge`
  user) and connects to a narrowed
  `/run/nixling/alloy/` subdirectory rather than the shared
  `/run/nixling/` root.
- Documented trust-concentration risk: `sys-obs-stack` has read
  access to every monitored VM's telemetry; treat as privileged
  infrastructure. Single-host single-VM by design (multi-host
  is explicitly out of scope for v0.2.0).

### Deferred to v0.3.0

- **`NixlingVMStuckWithoutSSH` alert** — needs a new
  CH-exporter metric (`nixling_vm_ssh_ready`) before the rule
  can be defined non-trivially.
- **`nixling_vm_store_path_count`** — the Per-VM Store
  dashboard references this metric today but it is currently
  **future-work absent**: no exporter emits it yet. The dashboard
  panel renders empty until a future store-path-count exporter
  lands (planned for v0.3.0). The `obs-metric-references`
  test gate treats it as a documented future-work exception
  rather than an unknown metric.
- **`nixling_vm_counter_net_tx_bytes` and
  `nixling_vm_counter_net_rx_bytes`** — referenced by the VM
  Resources network panel for legacy compatibility; the actual
  emitted metric names are `nixling_vm_counter_virtio_net_*`
  (CH v50 device naming). Documented as **future-work absent**
  pending dashboard query simplification — both legacy and
  modern names will resolve via Prometheus `or` until the legacy
  names are removed.
- **Stable relay-binary interface.**
  `nixling.observability.transport.relayPackage` still
  requires a `bin/socat`-compatible CLI today. v0.3.0 will
  define a stable interface so non-socat relays (e.g. a
  purpose-built Rust binary) can be swapped in without
  socat-compat shims.
- **VM-runner abstraction.** Today the framework leaks the
  runner-unit name (`microvm@<vm>` for headless,
  `nixling-<vm>-gpu` for graphics) into the relay wiring, and
  the observability code has to wire to both. v0.3.0 will
  introduce a runner-agnostic abstraction (e.g.
  `nixling-vm-runner@<vm>.service` aliased by whichever
  concrete runner is used) so per-VM sidecar wiring stays
  on a single name.

--- *Wave 6 maturity additions (folded in pre-tag - v0.2.0 was deferred until the consumer-integration shakedown landed):* ---

### Changed

- **sshd host keys are now generated on the HOST and shared into
  every guest read-only via virtiofs.** A new module
  `nixos-modules/host-ssh-host-keys.nix` provisions per-VM ed25519
  host keys at host activation under
  `${nixling.site.stateDir}/vms/<name>/sshd-host-keys/` (mode 0400
  root:root). `nixos-modules/store.nix` shares the directory into
  the guest at `/run/nixling-sshd-host-keys/` (virtiofs tag
  `nl-ssh-host`). A new `nixos-modules/guest-sshd-host-keys.nix`,
  imported into every enabled VM by `host.nix`, points
  `services.openssh.hostKeys` at the shared path and disables the
  NixOS `ssh-keygen -A` activation hook. **Why**: pre-v0.2.0 each
  guest regenerated its sshd host keys on first boot and stored
  them on the tmpfs overlay over the read-only nix store, so they
  were ephemeral. Every VM restart regenerated them, the host's
  `known_hosts.nixling` pinned the first observed set and refused
  to overwrite subsequent ones (correctly: from the host's point
  of view, a host-key change IS a possible MITM/swap), and
  operator SSH from the host would soft-brick until manual
  `ssh-keygen -R` + a refresh-service kick. Host-managed keys
  eliminate the drift class entirely.
- **`nixos-modules/host-known-hosts.nix`**: the refresh script
  now reads the host-side `.pub` file directly instead of probing
  the live VM with `ssh-keyscan`. Faster (no boot wait), immune
  to the live-vs-pinned drift the old logic had to handle (a VM
  restart used to regenerate the in-VM key every time).
- **Observability admin password + secret key are now generated
  on the HOST, not inside `sys-obs-stack`.** A new module
  `nixos-modules/observability-host-secrets.nix` provisions both
  files at host activation under
  `${nixling.site.stateDir}/observability/` (default
  `/var/lib/nixling/observability/`, mode 0400 root:root) and
  shares them read-only into the stack VM via virtiofs at
  `/run/nixling-obs-secrets/`. The in-VM activation scripts that
  used to generate these secrets in
  `/var/lib/nixling-observability/` (inside `sys-obs-stack`) have
  been removed. **Why**: putting both secrets inside the VM
  pointed the trust flow the wrong way — anything on the host
  that needed the Grafana admin password (a launcher, a health
  probe, a backup) had to cross the VM boundary to read it, which
  in practice forced consumers to add an SSH-able operator
  account + sudoers rule inside `sys-obs-stack` just to claw the
  password back out. With this change, host-side
  `sudo cat ${nixling.site.stateDir}/observability/grafana-admin-password`
  is the supported path; no operator account inside the stack VM
  is required. The `nixling.observability.grafana.{secretKeyFile,
  adminPasswordFile}` overrides still work for sops-nix / agenix
  users.
- **Consumer extensions of the auto-declared observability VM are
  now allowed.** The pre-v0.2.0 assertion that rejected any
  user-side definition under `nixling.vms.<obsCfg.vmName>` was
  removed. The framework's auto-declaration block uses
  `lib.mkDefault` for every value, so a consumer override
  (e.g. `nixling.vms.sys-obs-stack.ssh.user = "root"`) merges
  cleanly. The matching `assertions-eval.sh` test was renamed to
  `observability-vmname-extension-allowed` and asserts the new
  behaviour.
- **Default obs-VM memory bumped 512 M → 2048 M.** Grafana
  alone wants ~200 M RSS on idle; the full
  Grafana+Prom+Loki+Tempo+Alloy stack in a single VM tripped the
  in-VM OOM killer within seconds of boot at the previous 512 M
  default. 2 GiB is the minimum that lets the whole stack come
  up with default retention windows on a single-host install
  monitoring ~tens of VMs. `lib.mkDefault` so operators can
  override either way.
- **`services.alloy` /run/nixling/alloy via `RuntimeDirectory`,
  not tmpfiles**, on host + every guest + stack VM. The previous
  tmpfiles rule could not chown to the DynamicUser-allocated
  `alloy` UID at activation time; the directory either never
  appeared or was owned by `nobody:nogroup`, breaking
  `nixling-otel-host-bridge` setfacl + alloy's writability
  expectations.
- **Alloy `labels = { ... }` map literals updated with trailing
  commas** in `components/observability/{host,guest}.nix`. Alloy
  DSL distinguishes between newline-separated *blocks* (no `=`)
  and comma-separated *map literals* (with `=`); the latter were
  emitted without commas and rejected by Alloy's parser at boot.
- **`host-otel-relay-acl` + `host-ch-exporter`**: added
  `excludeShellChecks = [ "SC2034" ]` for bash namerefs and
  positional placeholders in `read`. Both scripts use shell
  patterns shellcheck cannot follow; the warnings became fatal
  the moment `writeShellApplication` actually built them in a
  consumer rebuild.
- Eval test `obs-stack-vm-guest-surface: grafana LoadCredential
  wires secret_key credential file` updated to assert the new
  in-VM source path
  `/run/nixling-obs-secrets/grafana-secret-key` (was the in-VM
  `/var/lib/nixling-observability/grafana-secret-key`).

### Migration

- Fresh installs land on the new layout with no operator action.
- Pre-existing installs that booted v0.2.0 with the in-VM
  observability secret generator will see a **password rotation**
  at the next `nixos-rebuild switch`: the new host-generated
  secret displaces the old in-VM one. Operators should fetch the
  new password via
  `sudo cat /var/lib/nixling/observability/grafana-admin-password`
  on the host.
- Pre-existing installs that had ephemeral in-VM sshd host keys
  pinned in `/var/lib/nixling/known_hosts.nixling` will see a
  **one-time host-key change** for every VM at the next
  activation+restart: the host now generates a stable ed25519
  host key per VM and the refresh service swaps the pinned entry
  on the next `microvm@<vm>` start. The framework handles this
  automatically; operator SSH clients (outside the framework)
  may need a one-time `ssh-keygen -R <ip>` against their personal
  `~/.ssh/known_hosts` if they manually trusted the old key.


## [0.1.7] - 2026-05-19

Patch release. v0.1.6 panel review caught a silent bug in the
v0.1.5 lifecycle policy: three of the six per-VM sidecars used
`unitConfig.X-RestartIfChanged = false` instead of the top-level
NixOS option `restartIfChanged = false`. The two forms LOOK
equivalent and both compile to a setting on the unit file —
but NixOS's `switch-to-configuration` logic only reads
`X-RestartIfChanged=` from the `[Service]` section. The
`unitConfig.X-RestartIfChanged` form emits under `[Unit]`,
where it is silently ignored. Result: pre-v0.1.7, every
`nixos-rebuild switch` that touched the GPU, swtpm, or snd
sidecar config STILL cycled those sidecars under the running
VM, defeating the v0.1.5 policy on the exact services whose
restart causes the most damage (CH termination, TPM socket
loss, audio sidecar disconnect).

### Fixed

- **`nixos-modules/host-sidecars.nix`** (swtpm + GPU sidecars):
  replaced `unitConfig.X-RestartIfChanged = false` with
  top-level `restartIfChanged = false`.
- **`nixos-modules/components/audio/host.nix`** (snd sidecar):
  same fix.
- **`tests/restart-policy-eval.sh`** (Test-H7 regression added
  in v0.1.6): tightened the predicate to REJECT
  `unitConfig.X-RestartIfChanged`. The previous version
  accepted either form, so it would have passed against the
  v0.1.5/v0.1.6 broken setup. Now any service using the broken
  form fails the test with an explicit message pointing at this
  CHANGELOG entry.
- **AGENTS.md** "Adding new per-VM units" guidance: explicitly
  forbids `unitConfig.X-RestartIfChanged`; mandates the
  top-level `restartIfChanged = false` form.
- **`docs/reference/components-{graphics,tpm,audio}.md`**:
  updated lifecycle subsections to reference the corrected
  form. The Lifecycle section subheaders still call this
  v0.1.5+ behaviour because the policy was always v0.1.5;
  v0.1.7 is just making the v0.1.5 intent actually work.

### Verification

The three sidecar files now match the pattern already used in
`host-wrapper.nix`, `host-known-hosts.nix`, and `store.nix`
(`restartIfChanged = false` at the top level). The
`tests/restart-policy-eval.sh` gate now asserts the correct
form on all 6 services and would have caught the v0.1.5 bug
at landing time. All other v0.1.6 gates remain green.

Spec correction #39 added.

## [0.1.6] - 2026-05-19

Docs catch-up release. The v0.1.1–v0.1.5 patches shipped fixes for
five framework bugs surfaced during the first real consumer
migration, but the public docs hadn't been updated to describe the
resulting behavior changes. This release brings the docs in sync
with the code, plus a small audit-strict fix that completes
`v0.1.4`'s skip-stopped-VMs work, and (in the v0.1.6 follow-up
panel sweep) tightens the autostart wiring + adds regression tests
for every v0.1.x patch.

### Changed

- **`nixling list` status label**: `[pending switch]` →
  `[pending restart]`. The label tracks the *recommended action*,
  and the recommended action for unit-file drift after a host
  `nixos-rebuild switch` is `nixling restart <vm>` (clean down+up
  cycles the running closure over the staged unit files); `nixling
  switch <vm>` is the heavier per-VM-closure-rebuild path for
  VM-NixOS-module edits. CLI messages in `nixling status` and the
  `nixling list` trailer updated to match.

- **`systemd.targets.microvms.wants` is now `lib.mkForce []`** on
  every consumer. Previously v0.1.3 narrowed the list to
  autostart=true VMs; v0.1.6 narrows further to `[]` so all
  autostart wiring goes through `systemd.targets.multi-user.wants
  -> nixling@<vm>.service` exclusively. Removes the duplicate
  boot path (target.wants pulling `microvm@<vm>` directly,
  bypassing the framework wrapper).

### Added (assertions)

- **`graphics.enable + autostart` is now an eval-time error.** A
  graphics VM with `autostart = true` would boot through the
  upstream microvm@<vm> runner without the GPU sidecar's
  Wayland-socket bind, leaving the VM with no display. The
  assertion's remediation message points at `nixling up <vm>`
  from a Plasma terminal.

### Added (tests)

- `tests/smoke-eval-extraspecialargs.nix` — regression for Spec
  correction #30 (v0.1.1 extraSpecialArgs propagation through
  `nixos-modules/host.nix:165`).
- `tests/net-vm-network-eval.sh` extended — Spec correction #31
  (v0.1.2 ConfigureWithoutCarrier + route entry on the host's
  uplink bridge).
- `tests/autostart-wiring-eval.sh` — Spec corrections #32 + #33
  + v0.1.6 SWArch-M10 (`nixling@<vm>` is template-only;
  multi-user.target.wants wiring; `microvms.target.wants == []`).
- `tests/smoke-eval-graphics.nix` extended — Spec correction #34
  (v0.1.4 `/dev/net/tun rw` in the GPU sidecar's DeviceAllow).
- `tests/smoke-eval-tpm.nix` — Spec correction #35 (v0.1.4
  swtpm parent-dir ACL traversal grant).
- `tests/restart-policy-eval.sh` — Spec correction #37 (v0.1.5
  `restartIfChanged = false` across all six services).
- Negative-assertion regression for v0.1.6 SWArch-M9 in
  `tests/assertions-eval.sh` (`test_graphics_with_autostart`).

### Added (docs)

- **`docs/reference/cli-contract.md`** documents:
  - `nixling restart <vm> [--force]` (v0.1.5)
  - `pending-restart` indicator semantics in `nixling list` /
    `nixling status` (v0.1.5)
  - `nixling.site.extraSpecialArgs` consumer-side escape hatch
    (v0.1.1)

- **`docs/explanation/design.md`**:
  - New "VM lifecycle policy" section explaining
    `restartIfChanged = false` on all per-VM units, the
    `booted`/`current` symlink contract, and how
    `pending-restart` is computed (v0.1.5).
  - New "Per-env bridge bootstrap" subsection covering the
    `ConfigureWithoutCarrier = true` requirement on the uplink
    bridge and how it breaks the route-preflight deadlock at
    boot (v0.1.2).
  - New "GPU sidecar substitutes microvm-run" subsection
    explaining why the GPU sidecar carries `DeviceAllow=/dev/net/tun`
    (v0.1.4), the `microvm-set-booted`-equivalent ExecStartPre
    (v0.1.5), and the swtpm-user ACL grant (v0.1.4).
  - "Why not X" — new FAQ entry: "Why doesn't `nixos-rebuild
    switch` restart VMs?", cross-linking to the cli-contract's
    pending-restart predicate.
  - Removed `tests/static.sh doesn't iterate examples` and
    `ROOT defaults to /etc/nixos` from "Limitations / known
    gaps" (resolved in W6).

- **`docs/how-to/migrating-from-microvm.md`**:
  - Required minimum `nixling = github:vicondoa/nixling/v0.1.6`
    (or later) — earlier versions exposed framework bugs that
    blocked real-world graphics + TPM bring-up. (Aligned with
    the CHANGELOG; v0.1.6 is the first release where the docs
    match the shipping code.)
  - New "After every rebuild" step in the procedure: check
    `nixling list` for `[pending restart]`, apply with
    `nixling restart <vm>`. Cross-links to the cli-contract's
    pending-restart section.
  - New troubleshooting note: `nixling status <vm>` shows
    `booted` vs `current` mismatch and the exact remediation
    command.

- **`docs/reference/components-graphics.md`**:
  - Added `/dev/net/tun rw` to the documented DeviceAllow list,
    with the rationale (cloud-hypervisor attaches to the tap
    upstream microvm.nix's `microvm-tap-interfaces@<vm>.service`
    helper created).
  - New "Lifecycle" subsection: GPU sidecar IS the
    cloud-hypervisor process; `restartIfChanged = false` keeps
    rebuilds from killing the VM.

- **`docs/reference/components-tpm.md`**:
  - Added the ACL traversal grant on the parent state dir to
    the documented host-side resources. No manual `chown`
    required for v0.1.4+ consumers — the framework's
    `nixlingVmStatePerms` activation script handles it.
  - Updated the "DO NOT WIPE" warning to also point at the
    `pending-restart` indicator as the right signal for
    "TPM-bound creds may be re-read after restart".
  - New "Lifecycle (v0.1.5+)" subsection documenting
    `nixling-<vm>-swtpm.service`'s `unitConfig.X-RestartIfChanged
    = false`.

- **`docs/reference/components-audio.md`**:
  - New "Lifecycle (v0.1.5+)" subsection documenting
    `nixling-<vm>-snd.service`'s `unitConfig.X-RestartIfChanged
    = false`.

- **`AGENTS.md`**:
  - New "VM lifecycle policy" subsection documenting
    `restartIfChanged = false` as a framework invariant for
    contributors.
  - New convention: per-VM `wantedBy` ALWAYS via
    `systemd.targets.multi-user.wants` symlinks, never via
    per-instance `systemd.services."nixling@${name}"`
    declarations (which NixOS materializes as separate unit
    files lacking the template's lifecycle hooks).

- Example READMEs (`minimal`, `graphics-workstation`, `multi-env`,
  `with-entra-id`) gain a short "After subsequent rebuilds"
  cross-link block pointing at the template README's post-rebuild
  section.

- Plan/spec corrections (#30-#38) tracking the v0.1.x patches
  plus the v0.1.6 follow-up sweep.

### Fixed

- **`nixos-modules/cli.nix`** (`audit --strict`): the
  `bridge_isolated_workload.<vm>` skip-when-down predicate (added
  in v0.1.4) only checked `microvm@<vm>.service`. Graphics VMs
  run cloud-hypervisor via the `nixling-<vm>-gpu.service` sidecar
  (the GPU sidecar replaces the upstream runner), so the audit
  was blanket-skipping all graphics VMs even when they were
  running. Now: a VM is "running" if any of `nixling@<vm>`,
  `microvm@<vm>`, or `nixling-<vm>-gpu` is active.

- **`nixos-modules/cli.nix`** (`nixling list` / `nixling status`):
  pending-drift messages used to recommend `nixling switch <vm>`,
  which is the heavier per-VM-closure-rebuild path. The correct
  remediation for unit-file drift after a host `nixos-rebuild
  switch` is `nixling restart <vm>` (clean down+up cycles the
  running closure over the staged unit files). Messages updated;
  status label `[pending switch]` renamed to `[pending restart]`
  to match.

## [0.1.5] - 2026-05-19

Patch release. Three consumer-impacting items from the first
`/etc/nixos`-side migration: the framework's nixos-rebuild
hot-restart of per-VM sidecars was killing running VMs; the
load-host-keys group assumption broke for the standard NixOS user
shape; and once we stopped restarting, consumers had no signal that
config drift had built up.

### Added

- **`nixling restart <vm> [--force]`** — convenience wrapper around
  `down <vm>` + `up <vm>`. Idempotent (a stopped VM is just brought
  up). Graphics VMs still require a Wayland session for the up
  step. The `--force` flag is forwarded to the down step (lets you
  cycle a net VM without first stopping the env's workloads). Used
  in tandem with the new `pending-restart` indicator below: when
  `nixling list` flags a VM, `nixling restart <vm>` applies the
  pending config.

- **`pending-restart` signal in `nixling list` / `nixling status`.**
  Compares each VM's `current` symlink (latest declared closure)
  against `booted` (the closure the running VM actually exec'd).
  If they differ AND the VM is up, both UIs flag the VM:

  ```
  NAME             ENV    GRAPHICS TPM   USBIP   STATIC_IP       STATUS
  work-aad         work   true     true  true    10.20.0.10      systemd [pending restart]
  ```

  And `nixling status work-aad` adds:

  ```
  pending-restart: YES — unit files changed; run `nixling restart work-aad` to apply
    booted : /nix/store/...-microvm-cloud-hypervisor-work-aad
    current: /nix/store/...-microvm-cloud-hypervisor-work-aad
  ```

  Note: v0.1.5 originally shipped the label as `[pending switch]`
  with a `run nixling switch <vm>` recommendation; v0.1.6 renamed
  the label to `[pending restart]` and the message to recommend
  `nixling restart <vm>` (the correct action for unit-file drift
  is the lighter `restart`, not the per-VM-closure-rebuild
  `switch`). Pre-v0.1.6 docs may show the legacy strings.

  Required because of the `restartIfChanged = false` changes below
  — without that signal, consumers had no way to know their
  `nixos-rebuild switch` only landed unit-file changes and not VM
  behaviour.

### Fixed

- **`restartIfChanged = false` on every per-VM lifecycle service.**
  Pre-v0.1.5, every `nixos-rebuild switch` that touched any of the
  per-VM units killed the running VM mid-flight — for graphics
  VMs the GPU sidecar IS the cloud-hypervisor process, so its
  restart terminated CH, the guest's in-RAM Entra device-bound
  tokens evaporated, and the user lost their login session. Even
  for headless VMs, every framework-touched config (host-keys
  refresh wiring, virtiofsd hardening stanza) caused NixOS to
  override upstream microvm.nix's `X-RestartIfChanged=false` back
  to `true`. The new flag updates the unit files at rebuild time
  but does NOT cycle the running VM; consumers apply per-VM
  changes via `nixling restart <vm>` (or `nixling switch <vm>`
  for a per-VM closure rebuild + live activation).

  Services covered:
  - `nixling@<vm>.service` (user-facing wrapper)
  - `microvm@<vm>.service` (upstream runner; framework was
    overriding upstream's existing flag back to true via the
    host-known-hosts.nix drop-in)
  - `microvm-virtiofsd@<vm>.service` (per-VM virtiofs daemon;
    framework adds hardening stanza)
  - `nixling-<vm>-swtpm.service`
  - `nixling-<vm>-snd.service`
  - `nixling-<vm>-gpu.service`

- **`nixling-<vm>-gpu.service` updates the per-VM `booted`
  symlink.** Upstream microvm.nix's
  `microvm-set-booted@<vm>.service` only runs as part of
  `microvm@<vm>.service`'s lifecycle — but graphics VMs bypass
  that template (the GPU sidecar runs microvm-run directly).
  Pre-v0.1.5, `/var/lib/nixling/vms/<vm>/booted` simply didn't
  exist for graphics VMs, so the new pending-restart check
  couldn't compute anything. Added `ExecStartPre`
  (`+`-prefixed → root) that mirrors
  `microvm-set-booted_-start`:
  `rm -f booted && ln -s $(readlink current) booted`. Cleared
  by `ExecStopPost`.

- **`nixling-load-host-keys.service` primary-group resolution.**
  Pre-v0.1.5 the script assumed the guest user's primary group
  matched the username (`install -d ... -g "$SSH_USER"`). This
  only holds when the consumer's VM config sets
  `users.users.<u>.group = "<u>"` or uses DynamicUser. NixOS's
  `isNormalUser = true` default puts the user in the `users`
  group, breaking the install with
  `install: invalid group '<u>'`. Result: no nixling-managed
  pubkey ever reached the guest's `authorized_keys`, and SSH
  only worked for keys baked statically into
  `users.users.<u>.openssh.authorizedKeys.keys`.

  Now: resolve GID via `getent passwd | cut -d: -f4`, then GID →
  name via `getent group`. Works for both
  `users.users.<u>.group = "<u>"` and the NixOS default.

## [0.1.4] - 2026-05-19

Patch release. Four framework bugs surfaced during the first real
consumer migration's VM bring-up (paydro's /etc/nixos, after v0.1.3
got `nixling@<vm>` units working but the actual graphics+TPM VM
refused to boot).

### Fixed

- **`nixos-modules/host-sidecars.nix`**: per-VM GPU sidecar
  (`nixling-<vm>-gpu.service`) had `DevicePolicy = "closed"` without
  `/dev/net/tun` in `DeviceAllow`. Cloud-hypervisor needs to
  `open("/dev/net/tun")` + `ioctl(TUNSETIFF, …)` to attach to the
  VM's tap (created earlier by upstream microvm.nix's
  `microvm-tap-interfaces@<vm>.service` helper); without it
  graphics VMs crash in early boot with "Cannot create virtio-net
  device / Couldn't open /dev/net/tun / Operation not permitted".
  Added `/dev/net/tun rw` to DeviceAllow.

- **`nixos-modules/host-activation.nix`**: `nixlingVmStatePerms`
  granted ACL rwx on `/var/lib/nixling/vms/<vm>/` to
  `nixling-<vm>-gpu` but not to `nixling-<vm>-swtpm`. The swtpm
  service starts as the swtpm user, opens its `StateDirectory=`
  (which systemd creates at the correct path), then tries to read
  `tpm2-00.permall` — and EACCESes because traversing the parent
  dir requires +x for the swtpm user. libtpms enters failure mode
  and the VM boots with a freshly-initialised TPM, triggering
  Entra/Intune device-tampering alerts for tenant-enrolled VMs.
  Added `setfacl -m "u:nixling-<vm>-swtpm:--x" <stateDir>` (gated
  on `vm.tpm.enable`).

- **`nixos-modules/base.nix`**: `nixling-load-host-keys.service`
  inside the guest referenced `${"$"}{pkgs.coreutils}/bin/getent` —
  but `getent` is in glibc, not coreutils. The lookup silently
  failed with "No such file or directory" and the script printed
  `user '<u>' not found in /etc/passwd — skipping` even though the
  user existed. Result: nixling-managed pubkeys + the consumer's
  `userAuthorizedKeys` never reached the guest's
  `authorized_keys` — SSH worked only via any pubkey statically
  baked into the VM's `users.users.<u>.openssh.authorizedKeys.keys`.
  Fixed path to `${"$"}{pkgs.glibc.getent}/bin/getent`.

- **`nixos-modules/cli.nix`** (audit `--strict`): the
  `bridge_isolated_workload.<vm>` check ran unconditionally and
  STRICT-FAILed when the VM wasn't running (the workload tap
  doesn't exist on the bridge, so jq returned null). With the
  framework's default `nixling.vms.<vm>.autostart = false`, this
  blocked every post-activation `nixling-audit-check.service`
  hook → `nixos-rebuild switch` returned non-zero exit code 4.
  Added a `systemctl is-active microvm@<vm>` precondition that
  emits `AUDIT SKIP [bridge_isolated_workload.<vm>]: VM not
  running` (mirrors the existing virtiofsd skip-when-down
  semantic).

## [0.1.3] - 2026-05-19

Patch release. Two more framework bugs surfaced during the first
real consumer migration, both around the nixling@<vm> wrapper +
microvm.nix interaction.

### Fixed

- **`nixos-modules/host-wrapper.nix`**: per-VM `nixling@<vm>.service`
  units for `autostart=true` VMs were emitted as separate unit files
  (via `systemd.services."nixling@${name}"`) that NixOS materialised
  WITHOUT the template's `ExecStart`/`ExecStop`/`PropagatesStopTo`/
  `Type=oneshot` settings — so systemd refused them at boot with
  "Service has no ExecStart=, ExecStop=, or SuccessAction=. Refusing."

  Fix: drop the per-instance `systemd.services` declarations and
  use `systemd.targets.multi-user.wants` symlinks instead. systemd
  then resolves each `nixling@<vm>.service` against the template
  with all its lifecycle wiring intact.

- **`nixos-modules/host-wrapper.nix`**: upstream microvm.nix emits
  `systemd.targets.microvms.wants = ["microvm@<vm>.service" …]`
  for every `microvm.vms.<vm>` declaration. `microvms.target` is
  itself `wantedBy = ["multi-user.target"]`, so workload VMs got
  pulled into boot regardless of `microvm.autostart = []`. Setting
  `microvm.autostart` only controls upstream's `multi-user.target.wants`
  on the microvm@ unit, not the `microvms.target` Wants= relation.

  Fix: `lib.mkForce` `systemd.targets.microvms.wants` to enumerate
  ONLY `autostart=true` VMs. Workload VMs are now exclusively
  on-demand via `nixling up <vm>`.

## [0.1.2] - 2026-05-19

Patch release. Surfaced during the first real consumer migration to
v0.1.x — a runtime bootstrap deadlock between
`nixling-net-route-preflight.service` and the per-env uplink bridge.

### Fixed

- **`nixos-modules/network.nix`**: per-env uplink bridge
  (`br-<env>-up`) now has `networkConfig.ConfigureWithoutCarrier =
  true`. Without it, networkd refuses to apply the Address + static
  Route to the env's LAN subnet until the bridge has carrier. But
  carrier only appears when the per-env net VM attaches its uplink
  tap to the bridge, and the net VM start is gated on
  `nixling-net-route-preflight.service`, which checks the static
  route exists. Deadlock.

  The LAN bridge already had `ConfigureWithoutCarrier = true`; the
  uplink-bridge case was missing. The fix is one option per env;
  no consumer config changes required.

  Existing v0.1.0 / v0.1.1 consumers can work around by running
  `sudo ip route add <env-lan>/<mask> via <env-uplink-gw> dev
  br-<env>-up` once per env before any
  `nixos-rebuild switch` — but the proper fix is to upgrade to
  v0.1.2 and re-rebuild.

## [0.1.1] - 2026-05-19

Patch release. Two consumer-impacting items surfaced during the
first real `/etc/nixos`-side migration to v0.1.0.

### Added

- **`nixling.site.extraSpecialArgs`** (`attrsOf unspecified`,
  default `{}`). Merged into every per-VM
  `microvm.vms.<vm>.specialArgs` after the framework's own
  baseline. Consumer keys take precedence on collision, so a
  consumer that wants its full flake `inputs` (rather than just
  nixling's narrower input set) visible inside per-VM modules
  can set:
  ```nix
  nixling.site.extraSpecialArgs = { inherit inputs; };
  ```
  Mirrors `home-manager.extraSpecialArgs` from the Home-Manager
  NixOS module — same semantics, same intent.

### Fixed

- **`scripts/migrate-nixling-v0.1.0.sh`**: `[[ -d "$dir" ]] && info ...`
  under `set -euo pipefail` aborted the script silently when the
  optional private-TPM-state directory didn't exist (return-value
  of the compound `&&` chain propagated up as the function's exit
  status). Replaced with explicit `if [[ -d ]]; then info; fi` for
  set-e safety. The bug aborted the snapshot phase before the
  `tpm2_getcap` step could run, leaving the migration in an
  in-progress state that required a manual cleanup.

## [0.1.0] - 2026-05-19

First public alpha release.

**Audience:** single-user NixOS desktop wanting isolated workspaces
(work / personal / risky-dev) on one host. Wayland-native.

**Stable in v0.1.0:**

- `nixosModules.default` (host integration)
- `templates.default` (`nix flake init -t github:vicondoa/nixling`)
- `flake.checks.<sys>.eval-{minimal,multi-env,template,graphics}`
- `nixling@<vm>.service` lifecycle wrapper + the eight `nixling` CLI
  verbs (`up`, `down`, `status`, `list`, `switch`, `build`, `boot`,
  `test`, `rollback`, `generations`, `gc`, `audio`, `usb`, `console`,
  `keys`)
- `manifestVersion = 1` JSON contract (`/run/current-system/sw/share/nixling/vms.json`)
- VM-name regex `^[a-z][a-z0-9-]*$`, reserved prefixes `sys-` and
  exact name `launcher`
- Per-env isolated network (auto-declared `sys-<env>-net` net VM,
  point-to-point uplink, isolated LAN bridge, dnsmasq, nftables NAT)
- Per-VM `/nix/store` hardlink farm
- Nixling-managed SSH keys
- Components: `graphics`, `tpm`, `usbip`, `audio`, `home-manager`

**Composition:** Sibling flake [`vicondoa/nixos-entra-id`][nei] (also
v0.1.0) provides Entra ID device-join via the per-VM
`nixling.vms.<vm>.config.imports = [ inputs.nixos-entra-id.nixosModules.default ]`
seam.

[nei]: https://github.com/vicondoa/nixos-entra-id

> Maintainer GitHub metadata reminder (apply on the GitHub UI, not in git):
>
> - **Description:** "NixOS microVM framework with isolated per-env
>   networking, Wayland/audio/USBIP/TPM components, and a
>   `nix flake init` template scaffold."
> - **Topics:** `nixos`, `nix-flake`, `microvm`, `wayland`,
>   `microvm-nix`, `nixos-template`, `entra-id`.



### Added (W6)

- `flake.checks.<system>.eval-{minimal,multi-env,template,graphics}` —
  the root flake now gates the example flakes + the template
  scaffold. The `graphics` check is x86_64-only.
- `tests/static.sh` now iterates `examples/*/flake.nix` running
  `nix flake check --no-build --all-systems` on each.
- `SECURITY.md` — disclosure path (GitHub Security Advisory primary;
  email fallback) plus the v0.1.0 alpha support matrix.
- `docs/explanation/design.md` — full threat model + defenses-in-depth
  list + a *Why not X* rationale FAQ (~823 LOC).
- `docs/how-to/migrating-from-microvm.md` — option mapping +
  step-by-step migration procedure + troubleshooting. Ordering is
  now build-before-state-move per W6 followup H1.
- Five per-component reference docs under
  `docs/reference/components-*.md` (graphics, tpm, usbip, audio,
  home-manager).
- `docs/reference/manifest-schema.{md,json}` polished with a rendered
  example payload generated from `tests/smoke-eval.nix`.

### Fixed (W6)

- `tests/{static,nixling-store,audio,lib}.sh` no longer assume
  `ROOT=/etc/nixos`; the value is derived from the script's own path
  so the suite runs from any clone. Spec correction #28 closed.
- `tests/nixling-store.sh:33` SC2157 (preexisting).
- Host-specific `NL_FILES` entries (`vms/personal-dev.nix`,
  `vms/work-aad.nix`) dropped or guarded so the static gate stays
  useful for the public flake.
- `tests/audio.sh` `NL_WAYLAND_USER` resolution chain genericized
  (no longer hardcoded to the maintainer's host user).
- README polish: `microVM` is defined inline on first use; a
  maintainer-anecdote phrasing was replaced with neutral wording;
  an encrypted-backup callout was added for `/var/lib/nixling/`.
- Manifest schema `manifestVersion` tightened from `minimum: 1` to
  `const: 1` so the JSON Schema matches the documented prose.

### Changed (W6)

- `docs/README.md` IA now reflects the shipping how-to and
  explanation docs (was previously reference-only).

### Added (W5)

- **`examples/minimal/`** — headless starter example: one env, one
  workload VM, ~25-line flake. The "is nixling for me?" sanity
  test.
- **`examples/graphics-workstation/`** — desktop VM with
  `graphics.enable`, `audio.enable`, and `usbip.yubikey` all on.
  Exercises every host-side sidecar component.
- **`examples/multi-env/`** — two parallel `nixling.envs.<env>`
  instances (work + personal) demonstrating per-env LAN
  isolation, per-env net VMs, per-env USBIP backends, and the
  route-preflight fail-closed gate.
- **`examples/with-entra-id/`** — composition with the sibling
  [`vicondoa/nixos-entra-id`][nixos-entra-id] flake; shows how
  the two trees meet at `nixling.vms.<vm>.config.imports`
  without either flake depending on the other.
- **`templates/default/`** — `nix flake init` scaffold with
  seven numbered `TODO:` markers and a matching
  `assertions = [ … ]` block. `nix flake check` on an un-edited
  scaffold fails with actionable messages until each sentinel is
  replaced.
- **`flake.templates.default`** — wires the template above so
  consumers can `nix flake init -t github:vicondoa/nixling`.

[nixos-entra-id]: https://github.com/vicondoa/nixos-entra-id

### Fixed (W5)

- **`nixos-modules/net.nix`:** neutralize base.nix's catch-all
  `10-eth-dhcp` systemd-networkd network on per-env net VMs. The
  catch-all (`matchConfig.Type = "ether"`) sorted lex-first
  against the per-MAC `10-uplink`/`10-lan` definitions and
  DHCP'd both NICs, preempting the static config. Now overridden
  via `lib.mkForce` with a sentinel MAC that never matches.
  Workload VMs are unaffected — they still inherit the base.nix
  DHCP fallback.
- **`nixos-modules/manifest.nix`:** dropped the redundant
  `default = { }` on the readOnly `nixling.manifest` option.
  The nixpkgs module system treats `default` as an extra
  definition; combined with `readOnly = true` and the matching
  `config.nixling.manifest = …` assignment, it produced
  "set multiple times" only when a graphics VM was synthesized.
  See `tests/smoke-eval-graphics.nix` for the regression test.

### Changed (W5)

- **README:** restructured to lead with a "Where to start" table
  pointing at the four examples and the template, and rewrote
  the Quick start to walk through the template path; the prior
  manual paste-in walkthrough is preserved under
  "Manual integration (without the template)".
- **`docs/README.md`:** added a Tutorials/Examples section
  linking the examples and the template; previously the docs
  index only mentioned the reference quadrant.

### Known gaps (deferred to v0.2.0 or Phase 7/8)

- **USBIP per-env units materialise even when no VM opts in.** Each
  `nixling.envs.<env>` declares `nixling-sys-<env>-usbipd-backend.service`
  and the corresponding proxy socket regardless of whether any
  workload VM in the env has `usbip.yubikey = true`. The units are
  idle when nothing opts in, but they are still installed (per-env
  baseline plumbing was intentional in W2; the unconditional
  materialisation is the gap). Tracked for v0.2.0; the relevant
  conditional belongs around `nixos-modules/network.nix:484-650`.
- **No static lint for `mkOption { default = …; readOnly = true; }`
  + matching `config.<…>` assignment.** Spec correction #29 was
  caught by the W5 reviewer panel, not by tooling. A Phase 7a
  follow-up will add a grep-level lint to prevent the
  `default + readOnly + config-assignment` trio from re-appearing.
  Trio detection is necessary because `store.nix` legitimately
  carries `readOnly + default` on options that have NO matching
  `config.<…>` assignment, so a two-of-three match is fine; only
  the full three is a bug.
- **Per-example flake-check loop is not fully hermetic for
  `examples/with-entra-id`.** `tests/static.sh` iterates
  `examples/*/flake.nix` and runs `nix flake check --no-build
  --all-systems` per example, but `with-entra-id` depends on the
  sibling `vicondoa/nixos-entra-id` flake which the core flake
  cannot pull in as an input. The example's own flake.lock pins
  the sibling and the iteration step exercises eval through it,
  but a clean-tree CI run cannot fully isolate the eval graph
  from the sibling. Tracked for v0.2.0.
- **VM-to-VM east-west traffic within the same env is not
  supported.** Workload taps on the per-env LAN bridge are
  configured with `Isolated = true`, so two workload VMs sharing
  `nixling.envs.<env>` can each reach the net VM (and via NAT,
  the upstream LAN) but cannot directly reach each other.
  Documented in `docs/explanation/design.md` and the
  `nixling.hostLanCidrs` option text. A future opt-in
  (e.g. `nixling.envs.<env>.intraLanIsolation = false`) is on the
  v0.2.0 wishlist.

### Added (W4)

- **Manifest contract is now a documented, versioned interface.**
  - `nixos-modules/manifest.nix` — typed `config.nixling.manifest`
    `attrsOf submodule` option. Replaces the inline manifest
    construction previously folded into `cli.nix`. The Nix module
    system catches schema regressions at eval time.
  - `docs/reference/manifest-schema.md` + `docs/reference/manifest-schema.json`
    (JSON Schema Draft 2020-12) — the v1 public manifest contract
    for downstream consumers (e.g. the future Rust CLI port). The
    JSON Schema is the canonical type spec; the prose doc is a
    field-by-field walkthrough + compatibility policy.
  - `docs/reference/cli-contract.md` — behavioural contract for any
    `nixling` CLI implementation (lifecycle FSM, signal semantics,
    exit codes, JSON vs human output, what is/is-not in scope).
  - `nixling.site.flakePath` is now derived as the CLI's default
    flake reference when unset (cli.nix lifecycle subcommands).
- **`docs/README.md`** — Diataxis IA index (tutorials, how-to,
  reference, explanation). Only the reference quadrant has content
  in W4; the others land on the path to v0.1.0.
- **Multi-arch eval coverage.** `tests/smoke-eval-aarch64.nix` —
  cross-evaluates a headless workload VM on `aarch64-linux`,
  verifying the eval graph stays multi-arch clean. Runtime is still
  `x86_64-linux`-only (cloud-hypervisor + crosvm); aarch64 is
  eval-coverage only.
- **Manifest validation gate.** `tests/static.sh` now renders the
  smoke manifest and runs a 5-check sequence against
  `docs/reference/manifest-schema.json`: render → parse schema →
  JSON-Schema validate → schema-side field cross-check →
  `manifestVersion >= 1`. Plus (W4-followup) a 6th check that diffs
  the prose schema's Per-VM-entry table against the JSON Schema's
  `properties` keys to catch md ↔ json drift.

### Changed (W4)

- **BREAKING for manifest consumers (pre-v0.1.0):** `manifestVersion`
  bumped `0 → 1`. The schema is now the documented contract. Future
  schema changes follow SemVer: minor field additions are
  backward-compatible; breaking changes bump the major (`2`, `3`,
  …). Consumers MUST refuse manifests with a newer major version
  than they were built against.
- **`nixling.vms.<vm>.graphics.enable` and
  `nixling.vms.<vm>.audio.enable` now refuse to evaluate on
  `aarch64-linux`** at the `microvm.vms` translation point. The
  eval-time error explains the constraint. Headless workload VMs
  (`graphics.enable = false; audio.enable = false;`) DO evaluate on
  aarch64-linux for cross-eval testing. Actual runtime is still
  x86_64-linux-only — the aarch64 path is eval-coverage only.
- `pkgs/{crosvm-patched,crosvm-seccomp,vhost-device-sound}/default.nix`
  now carry `meta.platforms = [ "x86_64-linux" ]`.
  `pkgs/spectrum-ch/default.nix` deliberately omits this (see
  in-file comment).
- `nixos-modules/options.nix` (internal refactor, no consumer-
  visible change): split into `options.nix` (aggregator) +
  `options-site.nix` + `options-envs.nix` + `options-vms.nix` for
  reviewability. The smoke-eval drvPath is bit-identical pre/post
  the split.

### Changed (W4-followup)

- **BREAKING for manifest consumers, security fix:** `sshKeyPath`
  removed from the per-VM JSON manifest. The W4 security reviewer
  flagged the field as a private-key path leak — the manifest at
  `/run/current-system/sw/share/nixling/vms.json` is world-readable,
  so exposing a per-VM private-key path leaks the location of
  secret material to every local user. The CLI now resolves the
  private-key path locally at Nix-eval time from
  `nixling.site.keysDir` (or per-VM `ssh.keyPath` override) and
  bakes a static per-VM mapping into the shell wrapper. Consumers
  reimplementing the CLI should mirror that: read
  `nixling.site.keysDir` from their own privileged config access,
  not from this world-readable file. The PUBLIC key path is not
  currently exposed; if a use case warrants it, a future
  `sshPubKeyPath` field is the recommended addition. `manifestVersion`
  stays at `1` — the schema was published moments ago in W4 and no
  external consumers exist yet, so this is a free pre-v0.1.0 break.
- `docs/reference/manifest-schema.json`: `manifestVersion.minimum`
  raised from `0` to `1`. The schema is the contract for v1+;
  pre-v1 manifests (the W2-followup stub) are no longer valid under
  this schema. (test-Med)
- `docs/reference/cli-contract.md`: subcommand inventory reconciled
  with `nixling --help`. `audit` now correctly documents the
  `--strict` + `--human` flags (`--human` auto-enables on TTY);
  `rotate-known-host <vm>` (the companion to `trust`) added to the
  subcommand table and to the human/JSON output section.
- `docs/reference/cli-contract.md`: "What is NOT in this contract"
  section expanded. Spells out that microvm.nix internal lifecycle,
  swtpm internals, virtiofsd implementation, and polkit grant
  specifics are framework-internal; and draws the line between
  contract-bound unit names (`nixling@<vm>.service`,
  `microvm@<vm>.service`) and framework-internal unit names
  (sidecars, USBIP proxies — these MUST be read from the manifest's
  `audioService` etc. fields, not hardcoded).
- `tests/static.sh`: `nix flake check` now uses `--all-systems` so
  Layer-1 exercises both x86_64-linux and aarch64-linux flake
  outputs, not just the builder's system. (sw-arch-Med)
- `tests/static.sh`: 6th manifest-contract check added — diffs the
  field-name column of the prose Per-VM-entry table in
  `docs/reference/manifest-schema.md` against the JSON Schema's
  `$defs.vmEntry.properties` keys, failing the gate if either side
  has a field the other doesn't. (sw-arch-Med)
- README: project status now states runtime is tested on
  `x86_64-linux` desktop and eval-tested for headless
  `aarch64-linux` (reflects the W4 cross-eval coverage).
  (product+docs-Med)
- README: documentation section replaces "docs will live under
  docs/" with direct bullets pointing at the manifest schema and
  CLI contract under `docs/reference/`. (product+docs-Med)
- `tests/README.md`: refreshed for the W4 additions —
  `manifestVersion = 1`, 10/10 assertions-eval cases, the 6-step
  manifest-contract gate (including the new md/json drift detection),
  and the multi-arch eval coverage. (docs-Med)

### Reorganised (W4-followup)

- Diataxis reorg. `docs/manifest-schema.{md,json}` →
  `docs/reference/manifest-schema.{md,json}`; `docs/cli-contract.md`
  → `docs/reference/cli-contract.md`. Added `docs/README.md` as the
  IA index. (docs-Med). All path references in
  `nixos-modules/manifest.nix`, `tests/static.sh`, and the moved
  docs' cross-links updated.

### Removed

- (W3b/Phase-2b) **`nixling.vms.<vm>.entra-id.*` option removed.**
  Himmelblau / Microsoft Entra ID support has moved out of the
  nixling framework and into the sibling `vicondoa/nixos-entra-id`
  flake. To migrate, add the flake as an input and import its
  module into the VM's guest config:

  ```nix
  inputs.nixos-entra-id.url = "github:vicondoa/nixos-entra-id";

  nixling.vms.<vm>.config.imports = [
    inputs.nixos-entra-id.nixosModules.default
  ];

  # Move each `nixling.vms.<vm>.entra-id.<key>` into the guest
  # config; see the nixos-entra-id README for the new schema.
  ```

  The `nixling.vms.<vm>.entra-id` attribute is kept as a hidden
  stub option so leftover assignments produce a readable
  assertion error (with migration instructions) instead of a
  cryptic "option does not exist" message from the module
  system. Final removal of the stub is tracked for v0.2.0.

- (W3b/Phase-2b) Three host-side activation scripts removed from
  `nixos-modules/host-activation.nix`:
  - **`nixlingSbctlBackup`** — moved maintainer-specific
    `*-backup.tar.gz` files from `$HOME` into `/var/lib/sbctl/backup/`.
    Not a framework concern. Consumers who relied on this should
    handle their own backup-file relocation outside nixling.
  - **`nixlingStoreChownRepair`** — one-shot repair for a past chown
    bug (an earlier `modules/nixling/store.nix` revision leaked
    `group=kvm` into `/nix/store` inodes via the per-VM hardlink
    farm). New installs are unaffected. Consumers upgrading from a
    pre-public nixling that ran with the buggy revision should run
    the historical repair script from `/etc/nixos` once and then
    drop the activation script there; the bug cannot recur on
    Phase-2b and later code.
  - **`nixlingMigrateState`** — one-shot renamer
    (`/var/lib/microvms/<vm>` → `/var/lib/nixling/vms/<vm>`, plus
    `/var/lib/swtpm/<vm>` → `vms/<vm>/swtpm/`). New installs land
    directly on the Phase-2a layout. Pre-public consumers should
    use the Phase 9 migration script (or perform the moves manually
    following the same logic) before switching to the public flake.

  These deletions remove all host-specific bias from the public
  framework's activation phase. The remaining two activation
  scripts (`nixlingVmStatePerms`, `nixlingNetVmVarImgPerms`,
  formerly `nixlingRouterVarImgPerms`) only adjust file ownership
  on per-VM disk images and contain no host-specific assumptions.

### Changed (W3b)

- **`nixling.vms.<vm>.ssh.keyPath` is NOT removed.** Earlier W3b
  phase-2b commit messages claimed otherwise; that was a mis-
  description of the change. The option still exists. What changed
  is its effective default: when left unset (`null`), the CLI now
  derives the SSH-key path from `nixling.site.keysDir` as
  `<keysDir>/<vm>_ed25519`, matching the framework-managed Ed25519
  key generated by `host-keys.nix` on every activation. Consumers
  who explicitly set a path still win; the option's `null` default
  just means "let the framework pick". This makes the
  framework-managed key the zero-config happy path while keeping
  the option-shape stable for consumers supplying their own keys
  (e.g. a hardware-backed Yubikey-resident key).

- (W3b/2026-05-19) Net VM `users.allowNoPasswordLogin` is set to
  `lib.mkDefault true`. Net VMs receive SSH keys via runtime
  injection (`nixling-load-host-keys.service` reads
  `<stateDir>/vms/<vm>/host-keys/` over virtiofs); they have no
  eval-time authorized_keys. Without the flag, NixOS module-eval
  fires the `users.allowNoPasswordLogin` assertion before runtime
  injection runs. Sealed-appliance consumers can override with
  `mkForce`.
- (W3b/2026-05-19) GPU sidecar (`nixling-<vm>-gpu.service`)
  hardening tightened: `NoNewPrivileges`, `ProtectSystem=strict`,
  `PrivateTmp`, `ProtectHome`, `DevicePolicy=closed` with a
  `/dev/kvm` + render-node allowlist, `RestrictAddressFamilies =
  [ AF_UNIX AF_NETLINK AF_VSOCK ]`,
  `SystemCallArchitectures=native`, narrow `ReadWritePaths`.
  Two omissions documented in source comments:
  `MemoryDenyWriteExecute` (crosvm GPU JIT triggers SIGSYS) and
  `AF_VSOCK` retained (cloud-hypervisor sd_notify path).
- (W3b/2026-05-19) IPv6 disabled on workload + net VM guest
  networkd (`LinkLocalAddressing=no`, `IPv6AcceptRA=false`); net
  VM nft rules DROP `ip6` forward. Net stack is IPv4-only by
  construction.
- (W3b/2026-05-19) Route preflight oneshot
  (`nixling-net-route-preflight.service`) now FAILS CLOSED on
  conflict — exit 1 on any env-vs-route mismatch instead of
  WARN+exit 0. `RemainAfterExit=true`, `Before=` each enabled
  nixling-managed VM unit, `RequiredBy=` each wrapper, so a stale
  host route blocks VM start until the operator clears it. (W3b
  H1 followup.)
- (W3b/2026-05-19) Inter-env CIDR overlap check now performs real
  IPv4 prefix arithmetic (`lib.cidrOverlaps` in
  `nixos-modules/lib.nix`) instead of exact-string equality.
  Containment (e.g. `10.0.0.0/16` ⊃ `10.0.1.0/24`) is rejected.
  Env-vs-`hostLanCidrs` is checked under the same helper. (W3b H3
  followup.)
- (W3b/2026-05-19) `nixling.site.yubikey.enable = false` actually
  gates the host-side udev rules + `usbip-host` kernel module.
  Previous phase-2b commit declared the option but never read it.
  (W3b H4 followup.)
- (W3b/2026-05-19) `nixling keys rotate <vm>` now scrubs the OLD
  pubkey from the guest's `~/.ssh/authorized_keys` (matched by
  SHA256 fingerprint) AFTER the new key is verified — rotation
  used to leave the old key authorized forever. Retention
  bounded: 3 most recent generations under
  `<keysDir>/old/<ts>/`; older are pruned post-rotation. Help
  text updated. (W3b H5 followup.)

### Added (W3b)

- **`nixling.site.*` public option surface.** Site-specific knobs
  extracted from previously-hardcoded references to the
  maintainer's host setup. Every option is opt-in; defaults give a
  fully headless framework with no Wayland integration. Public
  options:
  - `nixling.site.stateDir` — root of every nixling-managed state
    file (default `/var/lib/nixling`). **Advisory only in v0.1.0**
    (see option description); full threading lands in v0.2.0.
  - `nixling.site.keysDir` — directory for framework-managed
    per-VM SSH keys (default `${stateDir}/keys`). Same advisory
    caveat for v0.1.0.
  - `nixling.site.waylandUser` — primary Wayland user; required
    for any VM with `graphics.enable = true` or `audio.enable =
    true`.
  - `nixling.site.launcherUsers` — users added to the
    `nixling-launcher` group (polkit grant for VM start/stop).
  - `nixling.site.userAuthorizedKeys` — global authorized SSH
    keys merged into every VM at boot. Validated at eval time
    against an allowlist of supported key types; private-key
    markers rejected.
  - `nixling.site.yubikey.enable` — host-side Yubico udev rules +
    `usbip-host` kernel module. Default true.
  - `nixling.site.flakePath` — default flake reference for the
    `nixling` CLI's lifecycle subcommands (`build`, `switch`,
    `boot`, `test`). Nullable.
- **`nixling.vms.<vm>.userAuthorizedKeys`** — per-VM
  authorized SSH keys, merged with `site.userAuthorizedKeys`.
- **`nixling.audio.users`** — host-side option propagating an
  audio-group membership list into the guest. Default falls back
  to `[ vm.ssh.user ]` when unset.
- **Framework-managed per-VM SSH keys.** Activation
  (`nixos-modules/host-keys.nix`) generates an Ed25519 keypair
  per enabled VM under `<keysDir>/<vm>_ed25519`. Atomic via
  staging + `mv -T`; protected by `flock` on `<keysDir>/.lock`.
  The pubkey is staged under
  `<stateDir>/vms/<vm>/host-keys/host.pub` and injected into the
  guest at boot via virtiofs.
- **`nixling keys` CLI subcommands.**
  - `nixling keys list [--json]` — fingerprint + path + mtime
    per VM.
  - `nixling keys show <vm>` — print the pubkey.
  - `nixling keys rotate <vm>` — atomic rotate-and-verify with
    SHA256-fingerprint-based old-key scrub + 3-generation
    retention (see Changed entry above).
- **`nixling-load-host-keys.service`** (guest-side) — fail-closed
  service that reads `/run/nixling-host-keys/` and writes the
  union of `host.pub` + user-authorized-keys into the SSH user's
  `~/.ssh/authorized_keys`.
- **`scripts/migrate-nixling-v0.1.0.sh`** (W3b H6) — one-shot host
  migration script for consumers upgrading from a pre-public
  in-tree nixling layout. Preserves TPM state byte-for-byte.
  Has `--dry-run` and `--rollback`. Committed under
  `scripts/` so CI can shellcheck it.
- **`tests/smoke-eval.nix`** (W3b H9) — minimal consumer-style
  nixosSystem that imports `nixling.nixosModules.default` and
  exercises the eval graph end-to-end. Wired into
  `tests/static.sh` Layer-1.
- **`tests/assertions-eval.sh`** (W3b H10) — 8 regression tests
  exercising every eval-time invariant in the schema (CIDR shape,
  CIDR overlap, key validation, `waylandUser` presence, …).
- **`nixos-modules/lib.nix#cidrOverlaps`** — pure-Nix IPv4 prefix
  overlap helper used by network.nix assertions. Same file gains
  `parseCidr` as a public helper.

### Added

- (W0/2026-05-18) Initial flake skeleton with Apache-2.0 license,
  `x86_64-linux` + `aarch64-linux` eval, `microvm.nix` input, and
  reserved-but-inert `nixosModules.default`.
- (W1/2026-05-18) Mechanical lift of nixling modules from
  `/etc/nixos/modules/nixling/` into the public flake:
  - 9 core modules under `nixos-modules/` (`default`, `options`,
    `lib`, `host`, `network`, `base`, `store`, `cli`;
    `router.nix` renamed to `net.nix`);
  - 6 component modules under `nixos-modules/components/`
    (`graphics`, `tpm`, `usbip`, `home-manager`; `audio` split into
    `audio/{guest,host}.nix`);
  - Extracted pkgs: `spectrum-ch`, `vhost-device-sound`,
    `crosvm-patched`, `crosvm-seccomp`, `patches`;
  - 6 generic test scripts under `tests/`.
- (W2/2026-05-18) `systemd.services."nixling@"` wrapper template with
  explicit `ExecStart` / `ExecStop` / `PropagatesStopTo` (planning-round
  Critical #1 — `BindsTo` alone does not propagate stops).
- (W2/2026-05-18) Eval-time assertions for VM names
  (`^[a-z0-9][a-z0-9-]*$`, no `sys-` prefix, not `launcher`) and env
  names (≤ 8 chars).
- (W2/2026-05-18) `nixos-modules/assertions.nix` as a dedicated
  assertions module.
- (W2-followup/2026-05-18) Top-level `manifestVersion = 0` stub field
  in the per-VM JSON manifest (Phase 5 bumps to 1). Stashed under
  the reserved `_manifest` sentinel key; user-declared VM names
  cannot start with `_` per the W2-followup H1 stricter regex.

### Changed

- (W2/2026-05-18) **BREAKING.** Option namespace renamed:
  - `nixling.networks.<env>` → `nixling.envs.<env>`;
  - `nixling.networks.<env>.routerName` →
    `nixling.envs.<env>.netName`;
  - `nixling.networks.<env>.extraRouterConfig` →
    `nixling.envs.<env>.extraNetConfig`.
- (W2/2026-05-18) **BREAKING.** Per-env auto-declared VM renamed:
  `<env>-router` → `sys-<env>-net`.
- (W2/2026-05-18) **BREAKING.** Systemd unit naming convention:
  - `swtpm@<vm>` → `nixling-<vm>-swtpm`;
  - `nixling-snd@<vm>` → `nixling-<vm>-snd`;
  - `nixling-gpu-<vm>` → `nixling-<vm>-gpu`;
  - `nixling-store-sync@<vm>` → `nixling-<vm>-store-sync`;
  - `usbipd-nixling` → `nixling-sys-usbipd`;
  - `usbipd-nixling-<env>` → `nixling-sys-<env>-usbipd-proxy`.
- (W2/2026-05-18) **BREAKING.** System users/groups renamed:
  `nixling-gpu-<vm>` → `nixling-<vm>-gpu`, `nixling-snd-<vm>` →
  `nixling-<vm>-snd`, `swtpm-<vm>` → `nixling-<vm>-swtpm`.
- (W2/2026-05-18) **BREAKING.** State-dir layout:
  - `<stateDir>/<vm>/` → `<stateDir>/vms/<vm>/`;
  - `<stateDir>/<env>-router/` → `<stateDir>/vms/sys-<env>-net/`;
  - `<stateDir>/swtpm/<vm>/` → `<stateDir>/vms/<vm>/swtpm/`;
  - `/run/nixling-snd/<vm>/snd.sock` →
    `/run/nixling/vms/<vm>/snd.sock`.
- (W2/2026-05-18) **BREAKING.** Manifest JSON contract: `isRouter` →
  `isNetVm`, `routerVm` → `netVm`. Top-level `manifestVersion = 0`
  added in W2-followup; Phase 5 will bump.
- (W2-followup/2026-05-18) **BREAKING.** VM/env name regex tightened
  from `^[a-z0-9][a-z0-9-]*$` to `^[a-z][a-z0-9-]*$` (require leading
  letter). Matches systemd-escape best practices; avoids ambiguity
  with tooling that treats a leading digit as a numeric index
  (`ip link show 42web-l10`). No existing in-tree names trip the
  stricter rule; consumers with numeric-prefixed VM/env names must
  rename.
- (W2-followup/2026-05-18) CLI: `nixling up/down/status` now target
  `nixling@<vm>.service` (the user-facing wrapper) instead of
  `microvm@<vm>.service` directly. Lifecycle propagates via the
  wrapper's BindsTo / ExecStop. Diagnostic flows
  (`status --verbose`, `journalctl` examples) keep their
  `microvm@<vm>` references but label them "backend" /
  "implementation detail".
- (W2-followup/2026-05-18) CLI: `nixling list` / `nixling status`
  output tag for system VMs changed from `(router)` to `(net-vm)`.
  Helper renames: `ensure_router_up` → `ensure_net_vm_up`,
  `router_active` → `net_vm_active`, `IS_ROUTER` → `IS_NET_VM`.
  User-facing prose `router` / `router VM` → `net` / `net VM` (kept
  `routing/routes` only where describing the network function).
- (W2-followup/2026-05-18) `nixling-launcher` polkit grant tightened
  to an exact-unit allowlist generated at NixOS eval time from
  `cfg.vms` + `cfg.envs`, restricted to `start` / `stop` / `restart`
  verbs only. Drops the bare `microvm@*` prefix wildcard; default-
  deny invariant restored. Recovery / debugging paths can still
  authenticate via sudo or polkit-prompt.

### Notes

- Pre-v0.1.0 — breaking changes do not get a deprecation period.
  There is no compat shim for the old `nixling.networks` namespace
  or for any of the renamed unit / user / state-dir identifiers.
- The first tagged release will be `v0.1.0`. Until then, treat
  `main` as unstable.
- v0.1.0 will ship in lockstep with
  [`vicondoa/nixos-entra-id`][nixos-entra-id] v0.1.0; consumers
  using both should pin matching tags.

[nixos-entra-id]: https://github.com/vicondoa/nixos-entra-id
