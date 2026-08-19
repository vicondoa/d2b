# Architecture decision records

Architecture decision records (ADRs) capture load-bearing design choices for
d2b. They complement the Diataxis documentation tree; for the broader design
narrative, see [docs/explanation/design.md](../explanation/design.md).

The records below are the surviving architecture history. Their filenames are
stable identifiers; records that were retired with repository-only contributor
workflow tooling are intentionally absent.

## Records

- [0000 - Repository layout and Rust bootstrap](0000-repository-layout-and-rust-bootstrap.md)
- [0001 - Systemd-free VM orchestration](0001-systemd-free-vm-orchestration.md)
- [0002 - Non-root daemon and privileged broker](0002-non-root-daemon-and-privileged-broker.md)
- [0003 - Minijail provisioning and sandbox interface](0003-minijail-provisioning-and-sandbox-interface.md)
- [0004 - Cloud Hypervisor runner shape](0004-cloud-hypervisor-runner-shape.md)
- [0005 - Network firewall and TAP model](0005-network-firewall-and-tap-model.md)
- [0006 - Manifest bundle versioning](0006-manifest-bundle-versioning.md)
- [0007 - Bash coexistence and migration](0007-bash-coexistence-and-migration.md)
- [0008 - Supported platforms and rejected targets](0008-supported-platforms-and-rejected-targets.md)
- [0009 - Rust toolchain, MSRV, and supply chain](0009-rust-toolchain-msrv-and-supply-chain.md)
- [0010 - Wire protocol and typed errors](0010-wire-protocol-and-typed-errors.md)
- [0011 - cgroup v2 delegation and pidfd handoff](0011-cgroup-v2-delegation-and-pidfd-handoff.md)
- [0012 - IPv6-off sysctl set and hash ifname](0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md)
- [0013 - Firewall coexistence policy](0013-w3-firewall-coexistence-policy.md)
- [0014 - Modules, devices, and runner shape](0014-w3-modules-devices-runner-shape.md)
- [0015 - Daemon-only clean break](0015-daemon-only-clean-break.md)
- [0017 - No bash fallbacks invariant](0017-no-bash-fallbacks-invariant.md)
- [0018 - MicroVM Nix removal](0018-microvm-nix-removal.md)
- [0021 - Broker user namespace for virtiofsd](0021-broker-user-namespace-for-virtiofsd.md)
- [0022 - Stabilization-mode releases](0022-stabilization-mode-releases.md)
- [0023 - Runner-role lifecycle matrix](0023-runner-role-lifecycle-matrix.md)
- [0024 - In-VM guest config sync](0024-in-vm-guest-config-sync.md)
- [0025 - Wayland proxy host-jailed role](0025-wayland-proxy-host-jailed-role.md)
- [0026 - Native SigNoz observability](0026-native-signoz-observability.md)
- [0027 - Store-view hardlink live pool](0027-store-view-hardlink-live-pool.md)
- [0028 - Guest control plane over vsock](0028-guest-control-plane-over-vsock.md)
- [0029 - Framework SSH to typed guest RPC](0029-framework-ssh-to-typed-guest-rpc.md)
- [0030 - Guest exec as workload user](0030-guest-exec-as-workload-user.md)
- [0031 - Bare command and detached exec](0031-bare-command-and-detached-exec.md)
- [0032 - d2b v2 constellation control plane](0032-d2b-v2-constellation-control-plane.md)
- [0033 - Host collector parity](0033-host-collector-parity.md)
- [0034 - Storage lifecycle, restart, and synchronization](0034-storage-lifecycle-restart-and-synchronization.md)
- [0035 - Efficiency and simplification roadmap](0035-efficiency-and-simplification-roadmap.md)
- [0036 - QEMU media runtime](0036-qemu-media-runtime.md)
- [0037 - Local hypervisor runtime seam](0037-local-hypervisor-runtime-seam.md)
- [0038 - Persistent guest shell sessions](0038-persistent-guest-shell-sessions.md)
- [0039 - Constellation persistent shell routing](0039-constellation-persistent-shell-routing.md)
- [0040 - Graceful VM shutdown](0040-graceful-vm-shutdown.md)
- [0041 - Console and audio controls](0041-console-and-audio-controls.md)
- [0042 - d2b clipboard authority and picker split](0042-d2b-clipboard-authority-and-picker-split.md)
- [0043 - Realm-native control plane](0043-realm-native-control-plane.md)
- [0044 - Unsafe local runtime provider](0044-unsafe-local-runtime-provider.md)
- [0045 - Provider and transport framework](0045-provider-and-transport-framework.md)
- [0046 - d2b 3 provider control plane](0046-d2b-3-provider-control-plane.md)
- [0047 - Window identity chrome](0047-window-identity-chrome.md)
- [0049 - Store-owned mutation seal](0049-store-owned-mutation-seal.md)
- [0050 - Provider derivation artifact layout](0050-provider-derivation-artifact-layout.md)
- [0051 - Security key semantic backing set](0051-security-key-semantic-backing-set.md)
- [0053 - Gas City contributor infrastructure](0053-gascity-contributor-infrastructure.md)
- [0054 - Single product Cargo workspace](0054-single-product-cargo-workspace.md)
- [0056 - Gas City contributor environment](0056-gas-city-contributor-environment.md)

## Supporting records

- [Guest control feasibility dossier](guest-control-feasibility-dossier.md)
