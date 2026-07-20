#![deny(unsafe_code)]
// v1.2: workspace clippy lints inherited; some structural patterns warrant
// crate-level allows here to avoid churning broker-internal code that is
// otherwise correct and well-tested:
//
// - `deprecated`: cgroup vm_leaf_path migration is tracked but the deprecated
//   path is still referenced in legacy code paths kept for v1.1.x compat.
// - `clippy::dead_code`: helper functions (e.g. apply_mount_actions, apply)
//   are public API of internal modules that downstream callers may use.
// - `clippy::large_enum_variant`, `clippy::result_large_err`: TypedError
//   variants intentionally carry rich context; boxing tracked separately.
// - `clippy::too_many_arguments`: broker spawn pipeline has wide signatures
//   for safety (forgetting an arg = sandbox bypass).
// - `clippy::needless_borrows_for_generic_args`, `clippy::cmp_owned`,
//   `clippy::io_other`, `clippy::needless_borrow`: stylistic.
#![allow(deprecated)]
#![allow(dead_code)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::result_large_err)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::needless_borrows_for_generic_args)]
#![allow(clippy::cmp_owned)]
#![allow(clippy::io_other_error)]
#![allow(clippy::needless_borrow)]

pub mod allocator_service;
pub mod audit;
mod child_realm_guest_material;
pub mod child_realm_runtime;
pub mod fd_passing;
pub mod guest_material_audit;
pub mod guest_material_authority;
pub mod guest_material_store;
pub mod guest_session_material;
// Live broker request handlers (pidfd_open + clone3-based spawn +
// reconcile-executor calls). Pure-shaped: take their inputs directly so
// the dispatch layer is the only mixer of wire decoding + bundle
// resolution + live execution.
pub mod live_handlers;
pub mod ops;
pub mod runtime;
pub mod service_v2;
pub mod sys;

#[cfg(test)]
pub(crate) fn test_tempdir(component: &str) -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;

    let root = std::env::var_os("D2B_VALIDATION_OUTPUT_DIR")
        .map(std::path::PathBuf::from)
        .map(|root| root.join("rust-test-scratch/d2b-priv-broker"))
        .or_else(|| {
            std::env::var_os("CARGO_TARGET_DIR")
                .map(std::path::PathBuf::from)
                .map(|root| root.join("test-scratch/d2b-priv-broker"))
        });
    if let Some(root) = root {
        let root = root.join(component);
        std::fs::create_dir_all(&root).expect("create broker test output root");
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .expect("harden broker test output root");
        tempfile::tempdir_in(root).expect("create broker test tempdir")
    } else {
        tempfile::Builder::new()
            .prefix(&format!("d2bbr-{component}-"))
            .tempdir()
            .expect("create broker test tempdir")
    }
}

#[cfg(test)]
pub(crate) fn test_socket_tempdir() -> tempfile::TempDir {
    let root = std::env::var_os("D2B_VALIDATION_SOCKET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&root).expect("create broker test socket root");
    tempfile::Builder::new()
        .prefix("d2bbr-")
        .tempdir_in(root)
        .expect("create private broker socket tempdir")
}

// Behavioral + regression seccomp BPF tests.
#[cfg(test)]
mod seccomp_compile_tests;
