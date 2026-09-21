// @generated
// Provenance: emitted from the per-crate `resource-types.json` declarations
// by `cargo xtask check-provider-crate-layout --fix`; the layout check's
// authority drift gate regenerates this file byte-for-byte, and refuses a
// hand edit.

/// Known role types in the ADR 0004 process graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessRole {
    /// Host reconciliation before VM-specific startup.
    HostReconcile,
    /// A static Provider controller launched from a signed Process template.
    ProviderController,
    /// Store and virtiofs preflight validation.
    StoreVirtiofsPreflight,
    /// swtpm pre-start flush step.
    SwtpmPreStartFlush,
    /// swtpm sidecar.
    Swtpm,
    /// virtiofsd sidecar.
    Virtiofsd,
    /// Optional video sidecar.
    Video,
    /// Optional GPU/graphics sidecar.
    Gpu,
    /// Optional GPU/graphics sidecar - render-node-only mode (fd-passing, no device bind-mounts).
    /// Selected when `graphics.renderNodeOnly = true`; uses broker-pre-NS pattern (ADR 0021).
    GpuRenderNode,
    /// Optional audio sidecar.
    Audio,
    /// Cloud Hypervisor runner.
    CloudHypervisorRunner,
    /// QEMU media runner.
    QemuMediaRunner,
    /// Target-local one-shot NixOS activation runner.
    /// 
    /// This role is emitted only for the Guest execution target. The
    /// activation Provider creates an EphemeralProcess resource and the
    /// target-local process Provider resolves this role from the trusted
    /// bundle.
    ActivationNixosRunner,
    /// vsock relay sidecar.
    VsockRelay,
    /// Host-to-observability-VM OTLP bridge.
    OtelHostBridge,
    /// Authenticated ComponentSession Health readiness probe.
    /// 
    /// Readiness is a full authenticated ComponentSession identity exchange
    /// and Health check over the enrolled vsock, not a raw TCP-22 probe.
    /// It fails closed.
    ComponentSessionHealth,
    /// USBIP proxy or attach helper.
    Usbip,
    /// Guest-side CTAPHID relay frontend via UHID virtual HID device and
    /// AF_VSOCK transport. Runs inside the guest VM and visible to the host
    /// DAG as a component-specific role node that gates the broker vsock
    /// endpoint readiness.
    SecurityKeyFrontend,
    /// Host-jailed Wayland proxy. Runs as `d2b-<vm>-wlproxy`
    /// with the real host compositor socket bound read/write at a fixed
    /// in-jail upstream path and the per-VM proxy socket at
    /// `/run/d2b-wlproxy/<vm>`. Empty host capabilities; mandatory
    /// `seccompPolicyRef`; no PipeWire/Pulse socket access.
    WaylandProxy,
}
