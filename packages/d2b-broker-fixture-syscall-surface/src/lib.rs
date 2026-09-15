//! Fixture crate with a deliberately hostile dependency surface.
//!
//! The broker's dependency-surface audit (U6 approach item 3,
//! `d2b_broker_composition::dependency_surface`) must reject this crate on
//! every axis it checks:
//!
//! - a syscall-surface dependency (`libc`),
//! - raw-syscall code (`asm!`),
//! - link-time entry machinery (the used static in a custom section),
//! - panic-hook registration.
//!
//! The crate is a dev-dependency of the composition root only, so the audit
//! has a real crate to scan while the production broker binary never links
//! it. None of the functions are ever called.

/// A raw-syscall surface: `close` spelled as inline assembly.
///
/// x86_64 keeps the inline form the audit probes; other targets fall back
/// to the libc call so the crate still compiles everywhere it is scanned.
#[cfg(target_arch = "x86_64")]
pub fn raw_syscall_close(fd: i32) -> i32 {
    let status: i64;
    // SAFETY: fixture-only surface, never called; the register use is
    // exactly the `close` syscall convention on x86_64.
    unsafe {
        std::arch::asm!(
            "mov rax, 3",
            "syscall",
            in("rdi") fd as i64,
            out("rax") status,
            options(nostack),
        );
    }
    status as i32
}

/// The libc-backed close for non-x86_64 scans.
#[cfg(not(target_arch = "x86_64"))]
pub fn raw_syscall_close(fd: i32) -> i32 {
    // SAFETY: fixture-only surface, never called.
    unsafe { libc::close(fd) }
}

/// A link-time entry marker: a used static in a custom output section.
///
/// A linker-placed, runtime-initialized section is the same entry shape the
/// `ctor` crates produce; the audit rejects the section attribute itself.
#[used]
#[unsafe(link_section = ".d2b_fixture_marker")]
static FIXTURE_MARKER: u8 = 0;

/// A panic-hook registration: crate-global runtime behavior installed on
/// first call, exactly the hook the audit rejects.
pub fn install_isolation_silencer() {
    std::panic::set_hook(Box::new(|_info| {}));
}