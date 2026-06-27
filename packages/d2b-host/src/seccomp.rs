//! BPF seccomp compilation from the ioctl_policy matrix.
//!
//! Provides [`compile_ioctl_policy_to_bpf`] which turns the declarative
//! per-role ioctl allowlist (`ioctl_policy::ioctl_allowlist`) into a
//! compact BPF seccomp program ready for installation by the broker via
//! `SECCOMP_SET_MODE_FILTER`. This module lives in `d2b-host` (not
//! `d2b-core`) so that [`DeviceClass`] is available without creating
//! a dep-graph cycle; `d2b-priv-broker` depends on `d2b-host` and
//! converts the [`CompiledSeccompProgram`] to `libc::sock_filter` slices
//! in its quarantined `sys.rs`.
//!
//! ## BPF program layout (for N allowed ioctls)
//!
//! ```text
//! 0:       LD [0]                        // load syscall nr from seccomp_data
//! 1:       JEQ #16, 0, N+2              // if nr != SYS_ioctl → jump to ALLOW
//! 2:       LD [24]                       // load args[1] lo32 (ioctl request)
//! 3..3+N-1 JEQ #allowed[i], N-i, 0     // if match → jump to ALLOW
//! 3+N:     RET SECCOMP_RET_KILL_PROCESS // deny: unrecognised ioctl
//! 3+N+1:   RET SECCOMP_RET_ALLOW        // allow: all other syscalls + allowed ioctls
//! ```
//!
//! All syscalls other than `ioctl` (nr 16 on x86_64) pass through
//! unconditionally.  `ioctl` requests absent from the per-role matrix
//! kill the process (`SECCOMP_RET_KILL_PROCESS`).

use crate::devices::DeviceClass;
use crate::ioctl_policy::{RoleResources, ioctl_allowlist};

/// A single BPF instruction.
///
/// Matches the memory layout of `struct sock_filter` in the Linux kernel
/// UAPI (`linux/filter.h`).  Defined as a plain safe Rust struct in
/// `d2b-host` (which forbids unsafe code); the broker's quarantined
/// `sys.rs` converts it to `libc::sock_filter` when installing the filter.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BpfInstruction {
    pub code: u16,
    pub jt: u8,
    pub jf: u8,
    pub k: u32,
}

/// A compiled seccomp BPF program.
///
/// Safe to construct and inspect in `d2b-host`.  Unsafe syscall
/// installation (`seccomp(SECCOMP_SET_MODE_FILTER, ...)`) lives entirely
/// in the broker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledSeccompProgram {
    pub instructions: Vec<BpfInstruction>,
}

// ── BPF class encodings (linux/filter.h) ─────────────────────────────
const BPF_LD: u16 = 0x00;
const BPF_JMP: u16 = 0x05;
const BPF_RET: u16 = 0x06;
const BPF_W: u16 = 0x00; // 32-bit word operand size
const BPF_ABS: u16 = 0x20; // absolute addressing mode
const BPF_JEQ: u16 = 0x10; // jump-if-equal opcode modifier
const BPF_K: u16 = 0x00; // use the K (immediate) field

// ── `struct seccomp_data` field offsets (linux/seccomp.h) ─────────────
/// Offset of `int nr` inside `struct seccomp_data`.
const SECCOMP_DATA_NR_OFFSET: u32 = 0;
/// Offset of the low 32 bits of `__u64 args[1]` (the ioctl request
/// number), which sits at byte offset 24 in `struct seccomp_data` on
/// all architectures (`args[0]` at 16, `args[1]` at 24).
const SECCOMP_DATA_ARGS1_LO_OFFSET: u32 = 24;

// ── x86_64 Linux syscall number for ioctl ─────────────────────────────
/// `SYS_ioctl` on x86_64 Linux. The BPF program targets x86_64 only;
/// the broker already enforces `AUDIT_ARCH_X86_64` when installing the
/// filter via `seccomp(2)`.
const SYS_IOCTL: u32 = 16;

// ── seccomp return values ─────────────────────────────────────────────
/// Kills the entire process (all threads) immediately; not catchable.
const SECCOMP_RET_KILL_PROCESS: u32 = 0x8000_0000;
/// Allows the syscall to proceed normally.
const SECCOMP_RET_ALLOW: u32 = 0x7fff_0000;

// ── BPF instruction constructors ──────────────────────────────────────

fn stmt(code: u16, k: u32) -> BpfInstruction {
    BpfInstruction {
        code,
        jt: 0,
        jf: 0,
        k,
    }
}

fn jmp(code: u16, k: u32, jt: u8, jf: u8) -> BpfInstruction {
    BpfInstruction { code, jt, jf, k }
}

/// Compiles a BPF seccomp program from the declarative ioctl allowlist
/// for the supplied `device_classes`.
///
/// The resulting program:
/// - Allows **all** syscalls except `ioctl` (nr 16 on x86_64).
/// - For `ioctl`: allows request codes present in the union of
///   `class_ioctls` entries for every class in `device_classes`
///   (via [`ioctl_allowlist`]); kills the process for any other request.
///
/// The `FUSE_NO_IOCTL` sentinel (value 0) is excluded from the BPF
/// allowlist because it represents "no ioctl operations" rather than a
/// real request code.
///
/// # Panics
///
/// Panics if the deduplicated allowlist length exceeds 251 — the BPF
/// jump offsets would overflow `u8`.  In practice the v1.2 per-role
/// maximum is < 30.
pub fn compile_ioctl_policy_to_bpf(device_classes: &[DeviceClass]) -> CompiledSeccompProgram {
    let resources = RoleResources {
        role: "bpf-compile".to_owned(),
        device_classes: device_classes.to_vec(),
    };
    // Exclude the sentinel FUSE_NO_IOCTL (0); it carries no kernel opcode.
    let allowed: Vec<u32> = ioctl_allowlist(&resources)
        .into_iter()
        .filter(|&n| n != 0)
        .map(|n| n as u32)
        .collect();

    let n = allowed.len();
    // Jump offsets are u8; the farthest jump (from instruction 1,
    // jf = N+2) must fit.  N ≤ 251 keeps N+2 ≤ 253 ≤ u8::MAX.
    assert!(
        n <= 251,
        "ioctl allowlist has {n} entries — exceeds u8 BPF jump range"
    );

    // Edge case: when the allowlist is empty after sentinel filtering
    // (e.g. DeviceClass::Fuse with only FUSE_NO_IOCTL), the role has no
    // declarative ioctl constraint. Building a filter that kills on any
    // ioctl would be a regression vs the pre-D4 Ok(None) behavior. Emit
    // a permissive program that allows all syscalls including ioctl.
    //
    // This preserves the post-D4 invariant that the filter is INSTALLED
    // (Seccomp: 2 visible in /proc/<pid>/status — needed by D18 doctor
    // probe), without imposing a deny-all-ioctl policy that the
    // declarative matrix never authorized.
    if n == 0 {
        let instrs = vec![stmt(BPF_RET | BPF_K, SECCOMP_RET_ALLOW)];
        return CompiledSeccompProgram {
            instructions: instrs,
        };
    }

    let mut instrs = Vec::with_capacity(n + 5);

    // 0: load syscall nr from seccomp_data.
    instrs.push(stmt(BPF_LD | BPF_W | BPF_ABS, SECCOMP_DATA_NR_OFFSET));

    // 1: if nr != SYS_ioctl → jump to ALLOW.
    //    ALLOW is at position 3+n+1; from instruction 1, jf offset =
    //    (3+n+1) - (1+1) = n+2.
    instrs.push(jmp(BPF_JMP | BPF_JEQ | BPF_K, SYS_IOCTL, 0, (n as u8) + 2));

    // 2: load args[1] low 32 bits (the ioctl request code).
    instrs.push(stmt(BPF_LD | BPF_W | BPF_ABS, SECCOMP_DATA_ARGS1_LO_OFFSET));

    // 3..3+N-1: per-ioctl equality checks.
    for (i, &req) in allowed.iter().enumerate() {
        // jt = distance from this instruction's successor to ALLOW.
        //     = (3+n+1) - (3+i+1) = n-i
        let jt = (n - i) as u8;
        instrs.push(jmp(BPF_JMP | BPF_JEQ | BPF_K, req, jt, 0));
    }

    // 3+N: deny — unrecognised ioctl kills the process.
    instrs.push(stmt(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS));

    // 3+N+1: allow — all other syscalls (and matched ioctls).
    instrs.push(stmt(BPF_RET | BPF_K, SECCOMP_RET_ALLOW));

    CompiledSeccompProgram {
        instructions: instrs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::devices::DeviceClass;
    use crate::ioctl_policy::constants;

    // ── helper: run compile and verify the ALLOW / DENY instructions ──

    fn allow_instr() -> BpfInstruction {
        stmt(BPF_RET | BPF_K, SECCOMP_RET_ALLOW)
    }

    fn kill_instr() -> BpfInstruction {
        stmt(BPF_RET | BPF_K, SECCOMP_RET_KILL_PROCESS)
    }

    /// Extracts the set of u32 ioctl request codes that the program
    /// will ALLOW (all JEQ targets in the ioctl-check range).
    fn allowed_codes(prog: &CompiledSeccompProgram) -> Vec<u32> {
        // Instructions 3..len-2 are the ioctl-check JEQ entries.
        // For empty-allowlist programs (single RET_ALLOW), return empty.
        let instrs = &prog.instructions;
        if instrs.len() < 5 {
            return Vec::new();
        }
        let n = instrs.len() - 5;
        instrs[3..3 + n].iter().map(|i| i.k).collect()
    }

    #[test]
    fn empty_device_classes_produces_allow_all_when_no_constraints() {
        // v1.2 D4 regression fix: when the declarative allowlist is empty
        // (e.g. Fuse with only the FUSE_NO_IOCTL sentinel), the program
        // must NOT deny all ioctls — virtiofsd needs ioctls (FUSE mount
        // handshake, file flags) that the matrix never authorized as
        // restrictions. Emit a single RET_ALLOW so the filter is
        // installed (Seccomp:2 in /proc/<pid>/status, D18 doctor probe
        // passes) without imposing a deny-all-ioctl policy.
        let prog = compile_ioctl_policy_to_bpf(&[]);
        assert_eq!(prog.instructions.len(), 1);
        assert_eq!(prog.instructions[0], allow_instr());
        assert!(allowed_codes(&prog).is_empty());
    }

    #[test]
    fn kvm_class_contains_exactly_matrix_ioctls() {
        let prog = compile_ioctl_policy_to_bpf(&[DeviceClass::Kvm]);
        let codes = allowed_codes(&prog);
        let expected = [
            constants::KVM_GET_API_VERSION as u32,
            constants::KVM_CREATE_VM as u32,
            constants::KVM_CREATE_VCPU as u32,
            constants::KVM_RUN as u32,
        ];
        for &e in &expected {
            assert!(codes.contains(&e), "KVM ioctl 0x{e:08x} missing from BPF");
        }
        assert_eq!(codes.len(), expected.len(), "unexpected extra KVM entries");
    }

    #[test]
    fn net_tun_class_contains_tunsetiff_tunsetgroup_only() {
        let prog = compile_ioctl_policy_to_bpf(&[DeviceClass::NetTun]);
        let codes = allowed_codes(&prog);
        assert!(codes.contains(&(constants::TUNSETIFF as u32)));
        assert!(codes.contains(&(constants::TUNSETGROUP as u32)));
        assert!(
            !codes.contains(&(constants::TUNSETPERSIST as u32)),
            "TUNSETPERSIST must NOT be in NetTun BPF (broker-only)"
        );
        assert!(
            !codes.contains(&(constants::TUNSETOWNER as u32)),
            "TUNSETOWNER must NOT be in NetTun BPF (broker-only)"
        );
        assert!(
            !codes.contains(&(constants::TUNATTACHFILTER as u32)),
            "TUNATTACHFILTER must never appear in any per-role BPF"
        );
        assert_eq!(codes.len(), 2);
    }

    #[test]
    fn vhost_net_class_contains_exactly_matrix_ioctls() {
        let prog = compile_ioctl_policy_to_bpf(&[DeviceClass::VhostNet]);
        let codes = allowed_codes(&prog);
        let expected = [
            constants::VHOST_SET_OWNER as u32,
            constants::VHOST_GET_FEATURES as u32,
            constants::VHOST_NET_SET_BACKEND as u32,
        ];
        for &e in &expected {
            assert!(codes.contains(&e), "VhostNet ioctl 0x{e:08x} missing");
        }
        assert_eq!(codes.len(), expected.len());
    }

    #[test]
    fn fuse_class_produces_allow_all_program() {
        // v1.2 D4 regression fix: FUSE_NO_IOCTL = 0 is a sentinel for
        // "no ioctl constraint"; the resulting BPF must be allow-all
        // (not deny-all) so virtiofsd's required ioctls (FUSE mount
        // handshake etc.) survive.
        let prog = compile_ioctl_policy_to_bpf(&[DeviceClass::Fuse]);
        assert_eq!(prog.instructions.len(), 1);
        assert_eq!(prog.instructions[0], allow_instr());
        assert!(allowed_codes(&prog).is_empty());
    }

    #[test]
    fn tpm_class_contains_tpm_transmit_cmd() {
        let prog = compile_ioctl_policy_to_bpf(&[DeviceClass::Tpm]);
        let codes = allowed_codes(&prog);
        assert!(codes.contains(&(constants::TPM_TRANSMIT_CMD as u32)));
        assert_eq!(codes.len(), 1);
    }

    #[test]
    fn usbip_host_class_contains_usbip_vhci_import_dev() {
        let prog = compile_ioctl_policy_to_bpf(&[DeviceClass::UsbipHost]);
        let codes = allowed_codes(&prog);
        assert!(codes.contains(&(constants::USBIP_VHCI_IMPORT_DEV as u32)));
        assert_eq!(codes.len(), 1);
    }

    #[test]
    fn dri_class_contains_virtgpu_family() {
        let prog = compile_ioctl_policy_to_bpf(&[DeviceClass::Dri]);
        let codes = allowed_codes(&prog);
        for &c in &[
            constants::DRM_IOCTL_VERSION as u32,
            constants::DRM_IOCTL_GET_UNIQUE as u32,
            constants::DRM_IOCTL_VIRTGPU_MAP as u32,
            constants::DRM_IOCTL_VIRTGPU_EXECBUFFER as u32,
            constants::DRM_IOCTL_VIRTGPU_GETPARAM as u32,
            constants::DRM_IOCTL_VIRTGPU_RESOURCE_CREATE as u32,
            constants::DRM_IOCTL_VIRTGPU_WAIT as u32,
            constants::DRM_IOCTL_VIRTGPU_GET_CAPS as u32,
            constants::DRM_IOCTL_VIRTGPU_RESOURCE_CREATE_BLOB as u32,
            constants::DRM_IOCTL_VIRTGPU_CONTEXT_INIT as u32,
        ] {
            assert!(codes.contains(&c), "Dri missing 0x{c:08x}");
        }
    }

    #[test]
    fn udmabuf_class_contains_create_and_create_list() {
        let prog = compile_ioctl_policy_to_bpf(&[DeviceClass::Udmabuf]);
        let codes = allowed_codes(&prog);
        assert!(codes.contains(&(constants::UDMABUF_CREATE as u32)));
        assert!(codes.contains(&(constants::UDMABUF_CREATE_LIST as u32)));
        assert_eq!(codes.len(), 2);
    }

    #[test]
    fn pipewire_socket_class_produces_allow_all_program() {
        // v1.2 D4 regression fix: PipewireSocket has no ioctl
        // constraints (it operates via AF_UNIX, not ioctl), so the
        // class yields an empty allowlist. Same allow-all treatment
        // as Fuse.
        let prog = compile_ioctl_policy_to_bpf(&[DeviceClass::PipewireSocket]);
        assert_eq!(prog.instructions.len(), 1);
        assert_eq!(prog.instructions[0], allow_instr());
        assert!(allowed_codes(&prog).is_empty());
    }

    #[test]
    fn multi_class_ch_runner_union() {
        // cloud-hypervisor-runner = Kvm + VhostNet + NetTun
        let prog = compile_ioctl_policy_to_bpf(&[
            DeviceClass::Kvm,
            DeviceClass::VhostNet,
            DeviceClass::NetTun,
        ]);
        let codes = allowed_codes(&prog);
        // All three sets must appear; no duplicates (ioctl_allowlist dedupes).
        for &c in &[
            constants::KVM_RUN as u32,
            constants::VHOST_SET_OWNER as u32,
            constants::TUNSETIFF as u32,
            constants::TUNSETGROUP as u32,
        ] {
            assert!(codes.contains(&c), "CH runner missing 0x{c:08x}");
        }
        assert!(!codes.contains(&(constants::TUNATTACHFILTER as u32)));
        // Verify deduplication: no element appears twice.
        let mut sorted = codes.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(codes.len(), sorted.len(), "BPF allowlist has duplicates");
    }

    #[test]
    fn program_ends_with_kill_then_allow_when_constraints_present() {
        // v1.2 D4 regression fix: the kill-then-allow tail only applies
        // when the allowlist is non-empty. Empty-allowlist classes
        // (Fuse, PipewireSocket, []) produce a single allow-all RET.
        for cls in &[vec![DeviceClass::Kvm], vec![DeviceClass::NetTun]] {
            let prog = compile_ioctl_policy_to_bpf(cls);
            let n = prog.instructions.len();
            assert!(n >= 5, "program too short ({n} instrs) for {cls:?}");
            assert_eq!(
                prog.instructions[n - 2],
                kill_instr(),
                "second-to-last must be RET KILL for {cls:?}"
            );
            assert_eq!(
                prog.instructions[n - 1],
                allow_instr(),
                "last must be RET ALLOW for {cls:?}"
            );
        }
    }
}
