//! W3 per-role ioctl allowlist derivation.
//!
//! Derives the per-role ioctl allowlist from the typed
//! [`crate::devices::DeviceClass`] matrix (`RoleResources`) so role
//! handlers never carry the catch-all `ioctl: 1` deny per ADR 0003.
//! Constants for KVM/TAP/vhost-net/FUSE/DRM/USBIP/TPM live in
//! [`constants`] so the L1c negative-allowlist test surface
//! (`tests/ioctl-negative.sh`) can drive every fake backend without
//! linking to libc headers.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::devices::DeviceClass;

pub mod constants {
    //! Versioned ioctl request numbers consumed by W3.
    //!
    //! The values match the kernel UAPI on Linux x86_64. They are
    //! redeclared here (rather than `include!`ed via bindgen) so the
    //! `#![forbid(unsafe_code)]` invariant on `nixling-host` holds and
    //! so the L1c canary tests can reference them in fake backends
    //! without a libc dep. Keep in sync with `<linux/kvm.h>`,
    //! `<linux/if_tun.h>`, `<linux/vhost.h>`, `<linux/fuse.h>`,
    //! `<linux/usbip.h>`, `<linux/tpm.h>`, `<drm/drm.h>`.
    pub type Number = u64;

    /// `<linux/if_tun.h>` — set up the TAP/TUN interface.
    pub const TUNSETIFF: Number = 0x400454ca;
    /// `<linux/if_tun.h>` — set persistent flag.
    pub const TUNSETPERSIST: Number = 0x400454cb;
    /// `<linux/if_tun.h>` — set TAP owner uid.
    pub const TUNSETOWNER: Number = 0x400454cc;
    /// `<linux/if_tun.h>` — set TAP owner gid.
    pub const TUNSETGROUP: Number = 0x400454ce;
    /// `<linux/if_tun.h>` — attach a BPF filter (denied by W3 — leads
    /// to undeclared packet inspection paths).
    pub const TUNATTACHFILTER: Number = 0x401054d5;

    /// `<linux/kvm.h>` — create a VM.
    pub const KVM_CREATE_VM: Number = 0xae01;
    /// `<linux/kvm.h>` — get the KVM API version.
    pub const KVM_GET_API_VERSION: Number = 0xae00;
    /// `<linux/kvm.h>` — create a VCPU.
    pub const KVM_CREATE_VCPU: Number = 0xae41;
    /// `<linux/kvm.h>` — run a VCPU.
    pub const KVM_RUN: Number = 0xae80;

    /// `<linux/vhost.h>` — vhost owner registration.
    pub const VHOST_SET_OWNER: Number = 0xaf01;
    /// `<linux/vhost.h>` — vhost get features.
    pub const VHOST_GET_FEATURES: Number = 0x8008af00;
    /// `<linux/vhost.h>` — vhost-net set backend.
    pub const VHOST_NET_SET_BACKEND: Number = 0x4008af30;

    /// `<linux/fuse.h>` — current FUSE versions don't define request
    /// numbers; the device handle is read/written. The constant here
    /// is the sentinel "no ioctl ops" so the role table can still
    /// enumerate FUSE.
    pub const FUSE_NO_IOCTL: Number = 0;

    /// `<drm/drm.h>` — DRM_IOCTL_VERSION.
    pub const DRM_IOCTL_VERSION: Number = 0xc0406400;
    /// `<drm/drm.h>` — DRM_IOCTL_GET_UNIQUE.
    pub const DRM_IOCTL_GET_UNIQUE: Number = 0xc0106401;

    // ---------------------------------------------------------------
    // P1 kernel-2 + gpu-seccomp closure: DRM_IOCTL_VIRTGPU_* family.
    //
    // virtgpu (drm/virtgpu_drm.h, base 0x40 + DRM_COMMAND_BASE 0x40)
    // — required for virgl/venus/cross-domain Wayland on the Gpu role.
    // The ioctl numbers below are the kernel UAPI for the request
    // codes we currently exercise on this host's NVIDIA Quadro T1000
    // via crosvm-gpu cross-domain (verified on personal-dev/work-aad
    // 2026-05-30).
    //
    // Values are computed directly from
    // /nix/store/.../linux-headers-6.18.7/include/drm/virtgpu_drm.h
    // via a small C oracle so they stay accurate against the kernel
    // UAPI struct sizes (P1 kernel-r1-1 closure: the previous
    // hand-derived constants had nrs shifted by one and SUBMIT_CMD
    // collided with WAIT). When the upstream UAPI bumps a struct
    // size, regenerate via the oracle in
    // tests/golden/runner-shape/virtgpu-ioctl-values.txt.
    // ---------------------------------------------------------------
    /// virtgpu — map resource into guest address space (nr=0x01).
    pub const DRM_IOCTL_VIRTGPU_MAP: Number = 0xc0106441;
    /// virtgpu — execbuffer (nr=0x02; the only submit-path ioctl in
    /// the upstream UAPI; older Mesa branches called this SUBMIT_CMD
    /// but that token is not a kernel UAPI symbol).
    pub const DRM_IOCTL_VIRTGPU_EXECBUFFER: Number = 0xc0406442;
    /// virtgpu — get capability params (nr=0x03).
    pub const DRM_IOCTL_VIRTGPU_GETPARAM: Number = 0xc0106443;
    /// virtgpu — create resource (texture/buffer) (nr=0x04).
    pub const DRM_IOCTL_VIRTGPU_RESOURCE_CREATE: Number = 0xc0386444;
    /// virtgpu — wait for fence (nr=0x08).
    pub const DRM_IOCTL_VIRTGPU_WAIT: Number = 0xc0086448;
    /// virtgpu — get host capability set version + descriptor (nr=0x09).
    pub const DRM_IOCTL_VIRTGPU_GET_CAPS: Number = 0xc0186449;
    /// virtgpu — create resource via blob (zero-copy import; nr=0x0a).
    pub const DRM_IOCTL_VIRTGPU_RESOURCE_CREATE_BLOB: Number = 0xc030644a;
    /// virtgpu — create per-process 3D context (nr=0x0b).
    pub const DRM_IOCTL_VIRTGPU_CONTEXT_INIT: Number = 0xc010644b;

    /// `<linux/udmabuf.h>` — UDMABUF_CREATE: wrap a memfd as a dma-buf.
    /// Used by the Gpu role's cross-domain Wayland surface so the
    /// host compositor can import the guest's framebuffers without
    /// a copy. The newer LIST variant is the path crosvm uses.
    pub const UDMABUF_CREATE: Number = 0x40187542;
    /// `<linux/udmabuf.h>` — UDMABUF_CREATE_LIST: batched variant.
    pub const UDMABUF_CREATE_LIST: Number = 0x40087543;

    /// `<linux/usbip.h>` — USBIP_VHCI_IMPORT_DEV.
    pub const USBIP_VHCI_IMPORT_DEV: Number = 0x40087500;

    /// `<linux/tpm.h>` — TPM_TRANSMIT_CMD passthrough.
    pub const TPM_TRANSMIT_CMD: Number = 0x40187401;
}

/// Typed role resource bundle: what device classes a role legitimately
/// touches. The matrix is supplied by the trusted bundle; the broker
/// dispatcher refuses any request that references a class outside the
/// role's bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleResources {
    pub role: String,
    pub device_classes: Vec<DeviceClass>,
}

/// Returns the W3 per-role ioctl allowlist derived from the device
/// classes the role declares. The result is sorted + deduplicated so
/// fixture comparisons stay deterministic.
pub fn ioctl_allowlist(resources: &RoleResources) -> Vec<constants::Number> {
    let mut set: BTreeSet<constants::Number> = BTreeSet::new();
    for class in &resources.device_classes {
        for num in class_ioctls(*class) {
            set.insert(*num);
        }
    }
    set.into_iter().collect()
}

fn class_ioctls(class: DeviceClass) -> &'static [constants::Number] {
    use constants::*;
    match class {
        DeviceClass::Kvm => &[KVM_GET_API_VERSION, KVM_CREATE_VM, KVM_CREATE_VCPU, KVM_RUN],
        DeviceClass::NetTun => &[TUNSETIFF, TUNSETPERSIST, TUNSETOWNER, TUNSETGROUP],
        DeviceClass::VhostNet => &[VHOST_SET_OWNER, VHOST_GET_FEATURES, VHOST_NET_SET_BACKEND],
        DeviceClass::Fuse => &[FUSE_NO_IOCTL],
        // P1 kernel-2 + gpu-seccomp: virtgpu needs the full
        // virtgpu DRM family for cross-domain Wayland. Dri retains
        // its narrow set (used by non-virtgpu role consumers like
        // direct-render passthrough scenarios).
        DeviceClass::Dri => &[
            DRM_IOCTL_VERSION,
            DRM_IOCTL_GET_UNIQUE,
            DRM_IOCTL_VIRTGPU_GET_CAPS,
            DRM_IOCTL_VIRTGPU_CONTEXT_INIT,
            DRM_IOCTL_VIRTGPU_RESOURCE_CREATE,
            DRM_IOCTL_VIRTGPU_RESOURCE_CREATE_BLOB,
            DRM_IOCTL_VIRTGPU_EXECBUFFER,
            DRM_IOCTL_VIRTGPU_WAIT,
            DRM_IOCTL_VIRTGPU_MAP,
            DRM_IOCTL_VIRTGPU_GETPARAM,
        ],
        DeviceClass::NvidiaRender => &[DRM_IOCTL_VERSION, DRM_IOCTL_GET_UNIQUE],
        DeviceClass::NvidiaCtl | DeviceClass::NvidiaUvm => &[DRM_IOCTL_VERSION],
        DeviceClass::PipewireSocket => &[],
        DeviceClass::UsbipHost => &[USBIP_VHCI_IMPORT_DEV],
        DeviceClass::Tpm => &[TPM_TRANSMIT_CMD],
        DeviceClass::Vfio => &[],
        DeviceClass::Udmabuf => &[UDMABUF_CREATE, UDMABUF_CREATE_LIST],
    }
}

/// True if `ioctl` is on the allowlist derived from `resources`.
pub fn is_allowed(resources: &RoleResources, ioctl: constants::Number) -> bool {
    resources
        .device_classes
        .iter()
        .any(|class| class_ioctls(*class).contains(&ioctl))
}

/// Surface for the negative-allowlist matrix (plan.md §"W3 seccomp/
/// ioctl negative-allowlist matrix").
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NegativeMatrixClass {
    TapTun,
    CgroupChown,
    SysctlWrite,
    NftBatchApply,
    DeviceOpen,
}

/// Single fake-backend negative case. The shell test compares the
/// decision boolean against the expected outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NegativeMatrixCase {
    pub class: NegativeMatrixClass,
    pub name: String,
    pub expected_allowed: bool,
    pub rationale: String,
}

/// Returns the canonical 5-class negative-allowlist surface used by
/// `tests/ioctl-negative.sh`. Each case is paired with the test's
/// fake-backend stub via [`NegativeMatrixClass`].
pub fn negative_matrix() -> Vec<NegativeMatrixCase> {
    vec![
        NegativeMatrixCase {
            class: NegativeMatrixClass::TapTun,
            name: "TUNSETIFF allowed for net-runner role".to_owned(),
            expected_allowed: true,
            rationale: "Declared TAP setup ioctl for the runner.".to_owned(),
        },
        NegativeMatrixCase {
            class: NegativeMatrixClass::TapTun,
            name: "TUNATTACHFILTER refused".to_owned(),
            expected_allowed: false,
            rationale: "Out-of-band BPF filter installation is not declared by any W3 role."
                .to_owned(),
        },
        NegativeMatrixCase {
            class: NegativeMatrixClass::CgroupChown,
            name: "fchown on broker-owned leaf allowed".to_owned(),
            expected_allowed: true,
            rationale: "Broker chowns delegated nixling.slice subtree only.".to_owned(),
        },
        NegativeMatrixCase {
            class: NegativeMatrixClass::CgroupChown,
            name: "fchownat on /sys/fs/cgroup refused".to_owned(),
            expected_allowed: false,
            rationale: "Ancestor chown would escape the W3 delegation contract.".to_owned(),
        },
        NegativeMatrixCase {
            class: NegativeMatrixClass::SysctlWrite,
            name: "declared per-link disable_ipv6 write allowed".to_owned(),
            expected_allowed: true,
            rationale: "IPv6-off ordering writes to the link the role declared.".to_owned(),
        },
        NegativeMatrixCase {
            class: NegativeMatrixClass::SysctlWrite,
            name: "foreign-link sysctl refused".to_owned(),
            expected_allowed: false,
            rationale: "Sysctl write to a link outside host.json is not authorized.".to_owned(),
        },
        NegativeMatrixCase {
            class: NegativeMatrixClass::NftBatchApply,
            name: "declared inet nixling apply allowed".to_owned(),
            expected_allowed: true,
            rationale: "Batch confined to nixling-owned table.".to_owned(),
        },
        NegativeMatrixCase {
            class: NegativeMatrixClass::NftBatchApply,
            name: "ruleset replace on foreign table refused".to_owned(),
            expected_allowed: false,
            rationale: "ApplyNftables refuses to touch foreign tables.".to_owned(),
        },
        NegativeMatrixCase {
            class: NegativeMatrixClass::DeviceOpen,
            name: "/dev/kvm open allowed when role declares Kvm".to_owned(),
            expected_allowed: true,
            rationale: "Declared OpenKvm with matrix entry present.".to_owned(),
        },
        NegativeMatrixCase {
            class: NegativeMatrixClass::DeviceOpen,
            name: "/dev/sg0 open refused".to_owned(),
            expected_allowed: false,
            rationale: "Generic SCSI device is not in the W3 device-node matrix.".to_owned(),
        },
        NegativeMatrixCase {
            class: NegativeMatrixClass::DeviceOpen,
            name: "/dev/mem open refused".to_owned(),
            expected_allowed: false,
            rationale: "Raw physical memory access is never declared by any W3 role.".to_owned(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kvm_role_includes_kvm_run_but_not_tun_setiff() {
        let r = RoleResources {
            role: "kvm-runner".to_owned(),
            device_classes: vec![DeviceClass::Kvm],
        };
        assert!(is_allowed(&r, constants::KVM_RUN));
        assert!(!is_allowed(&r, constants::TUNSETIFF));
    }

    #[test]
    fn net_role_allows_tunsetiff_refuses_tunattachfilter() {
        let r = RoleResources {
            role: "net-runner".to_owned(),
            device_classes: vec![DeviceClass::NetTun, DeviceClass::VhostNet],
        };
        assert!(is_allowed(&r, constants::TUNSETIFF));
        assert!(!is_allowed(&r, constants::TUNATTACHFILTER));
        assert!(is_allowed(&r, constants::VHOST_SET_OWNER));
    }

    #[test]
    fn allowlist_is_sorted_and_deduplicated() {
        let r = RoleResources {
            role: "kvm-net".to_owned(),
            device_classes: vec![DeviceClass::Kvm, DeviceClass::NetTun, DeviceClass::Kvm],
        };
        let list = ioctl_allowlist(&r);
        let mut sorted = list.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(list, sorted);
        assert!(list.contains(&constants::KVM_RUN));
        assert!(list.contains(&constants::TUNSETIFF));
    }

    #[test]
    fn negative_matrix_covers_every_class() {
        let cases = negative_matrix();
        let classes: BTreeSet<_> = cases.iter().map(|c| &c.class).collect();
        assert_eq!(classes.len(), 5, "expected one row per W3 negative class");
        assert!(cases.iter().any(|c| !c.expected_allowed));
        assert!(cases.iter().any(|c| c.expected_allowed));
    }

    #[test]
    fn empty_role_resources_produces_empty_allowlist() {
        let r = RoleResources {
            role: "audit-only".to_owned(),
            device_classes: vec![],
        };
        assert!(ioctl_allowlist(&r).is_empty());
    }
}
