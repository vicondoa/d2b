//! The Export-owned virtiofsd worker.
//!
//! The worker's sandbox posture is frozen by ADR 0021: zero host
//! capabilities, no start as root, a chroot sandbox whose privileges live
//! only inside a broker-pre-established user namespace, and no
//! `open_by_handle_at` support. This module owns the path-free worker
//! plan; the private argv renderer below is the only place a resolved
//! path is joined to it, and it is not part of the public surface.

use serde::Serialize;

use d2b_contracts::v3::execution_policy::BoundedToken;
use d2b_contracts::v3::volume::{
    AttachmentAccess, AttachmentCache, InodeFileHandles, ViewRight, ViewSpec,
};

use crate::error::VirtiofsExportError;
use crate::export::ExportSpec;

/// The frozen sandbox mode of every virtiofsd worker.
pub const SANDBOX_MODE: &str = "chroot";
/// The frozen inode file-handle mode of every virtiofsd worker.
pub const INODE_FILE_HANDLES: &str = "never";
/// The Process template every Export-owned worker uses.
pub const WORKER_TEMPLATE: &str = "virtiofsd-worker";
/// The user-namespace mapping class the worker resolves through its
/// launch port.
pub const USER_NAMESPACE_MAPPING_CLASS: &str = "process-principal-root";

/// The path-free launch plan of one Export's virtiofsd worker.
///
/// It names no socket path, no shared directory, no numeric group, and
/// no store path. The effect adapter joins it to the private root
/// descriptor and the private socket path it alone derives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtiofsdWorkerPlan {
    /// The Process template this worker instantiates.
    pub template: &'static str,
    /// The number of worker threads, resolved from the attachment
    /// settings or the target Guest's vcpu count.
    pub thread_pool_size: u32,
    /// Whether the share is served read-only.
    pub readonly: bool,
    /// Whether POSIX ACLs are served.
    pub posix_acl: bool,
    /// Whether extended attributes are served.
    pub xattr: bool,
    /// The page-cache mode.
    pub cache: AttachmentCache,
    /// The mapping class the launch port resolves for the worker's user
    /// namespace.
    pub user_namespace_mapping_class: &'static str,
    /// The dedicated per-Volume principal the in-namespace root maps to.
    pub principal: BoundedToken,
}

impl VirtiofsdWorkerPlan {
    /// Build the worker plan for one Export.
    ///
    /// `vcpu_count` supplies the thread-pool size when the attachment
    /// declares none. The plan is read-only whenever the Export declares
    /// read-only access or the selected view grants no write right, so a
    /// view that never granted write cannot be widened by an Export.
    pub fn for_export(
        export: &ExportSpec,
        view: &ViewSpec,
        vcpu_count: u32,
        principal: BoundedToken,
    ) -> Result<Self, VirtiofsExportError> {
        if vcpu_count == 0 {
            return Err(VirtiofsExportError::InvalidExport);
        }
        let writes = view.rights().contains(&ViewRight::Write);
        if export.access() != AttachmentAccess::ReadOnly && !writes {
            return Err(VirtiofsExportError::ViewRightsInsufficient);
        }
        let settings = export.settings();
        let rendered =
            serde_json::to_value(settings).map_err(|_| VirtiofsExportError::InvalidExport)?;
        let flag = |name: &str| rendered.get(name).and_then(serde_json::Value::as_bool);
        let thread_pool_size = rendered
            .get("threadPoolSize")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(vcpu_count);
        if thread_pool_size == 0 {
            return Err(VirtiofsExportError::InvalidExport);
        }
        if settings.inode_file_handles() != InodeFileHandles::Never {
            return Err(VirtiofsExportError::SandboxInvariantViolated);
        }
        Ok(Self {
            template: WORKER_TEMPLATE,
            thread_pool_size,
            readonly: export.access() == AttachmentAccess::ReadOnly || !writes,
            posix_acl: flag("posixAcl").unwrap_or(false),
            xattr: flag("xattr").unwrap_or(false),
            cache: settings.cache(),
            user_namespace_mapping_class: USER_NAMESPACE_MAPPING_CLASS,
            principal,
        })
    }
}

/// The frozen sandbox posture every virtiofsd worker declares.
///
/// A worker that declares a host capability, starts as root, or asks for
/// a namespace sandbox violates ADR 0021 and is rejected before any
/// launch is requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerSandbox {
    capability_classes: Vec<BoundedToken>,
    start_root: bool,
    sandbox_mode: String,
    read_only_root: bool,
}

impl WorkerSandbox {
    /// Declare a sandbox posture as an implementation reports it.
    pub fn declared(
        capability_classes: Vec<BoundedToken>,
        start_root: bool,
        sandbox_mode: impl Into<String>,
        read_only_root: bool,
    ) -> Self {
        Self {
            capability_classes,
            start_root,
            sandbox_mode: sandbox_mode.into(),
            read_only_root,
        }
    }

    /// The one conformant posture.
    pub fn conformant() -> Self {
        Self::declared(Vec::new(), false, SANDBOX_MODE, true)
    }

    /// Reject any posture that is not the frozen one.
    pub fn assert_conformant(&self) -> Result<(), VirtiofsExportError> {
        if !self.capability_classes.is_empty()
            || self.start_root
            || self.sandbox_mode != SANDBOX_MODE
            || !self.read_only_root
        {
            return Err(VirtiofsExportError::SandboxInvariantViolated);
        }
        Ok(())
    }
}

/// Everything the effect adapter resolves privately for one worker.
///
/// This type is crate-private on purpose: it is the only place a
/// resolved shared directory, an export socket path, and a socket group
/// meet, and none of them may reach the public surface. Its callers are
/// the in-crate renderer below and, once ProviderSupervisor hosts it,
/// the effect adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct ResolvedWorkerPaths {
    pub(crate) binary_path: String,
    pub(crate) socket_path: String,
    pub(crate) socket_group: String,
    pub(crate) shared_dir: String,
}

/// Render one virtiofsd argv.
///
/// Adapted from the shipped host-side generator. The flag envelope is
/// preserved, with three differences the Volume spec freezes: the
/// sandbox mode is always `chroot`, inode file handles are always
/// `never`, and there is no free-form extra-argument channel.
///
/// It stays crate-private because it is the one function that joins a
/// resolved path to the worker plan. Its callers are the pinned argv
/// cases and, once ProviderSupervisor hosts it, the effect adapter.
#[allow(dead_code)]
pub(crate) fn render_argv(
    plan: &VirtiofsdWorkerPlan,
    paths: &ResolvedWorkerPaths,
) -> Result<Vec<String>, VirtiofsExportError> {
    if paths.binary_path.is_empty() || !paths.binary_path.starts_with('/') {
        return Err(VirtiofsExportError::InvalidExport);
    }
    for value in [&paths.socket_path, &paths.socket_group, &paths.shared_dir] {
        if value.is_empty() {
            return Err(VirtiofsExportError::InvalidExport);
        }
    }
    if paths.shared_dir == "/nix/store" {
        return Err(VirtiofsExportError::InvalidExport);
    }
    if plan.thread_pool_size == 0 {
        return Err(VirtiofsExportError::InvalidExport);
    }

    let cache = match plan.cache {
        AttachmentCache::Auto => "auto",
        AttachmentCache::Always => "always",
        AttachmentCache::Never => "never",
    };

    let mut argv = Vec::with_capacity(12);
    argv.push(paths.binary_path.clone());
    argv.push(format!("--socket-path={}", paths.socket_path));
    argv.push(format!("--socket-group={}", paths.socket_group));
    argv.push(format!("--shared-dir={}", paths.shared_dir));
    argv.push(format!("--thread-pool-size={}", plan.thread_pool_size));
    if plan.posix_acl {
        argv.push("--posix-acl".to_owned());
    }
    if plan.xattr {
        argv.push("--xattr".to_owned());
    }
    argv.push(format!("--cache={cache}"));
    argv.push(format!("--sandbox={SANDBOX_MODE}"));
    argv.push(format!("--inode-file-handles={INODE_FILE_HANDLES}"));
    if plan.readonly {
        argv.push("--readonly".to_owned());
    }
    Ok(argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fixtures;

    fn paths() -> ResolvedWorkerPaths {
        ResolvedWorkerPaths {
            binary_path: "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-virtiofsd/bin/virtiofsd"
                .to_owned(),
            socket_path: "/run/d2b/zones/dev/exports/0.sock".to_owned(),
            socket_group: "d2b-virtiofs".to_owned(),
            shared_dir: "/var/lib/d2b/vms/work-vm/store-view/live".to_owned(),
        }
    }

    fn plan(readonly: bool) -> VirtiofsdWorkerPlan {
        VirtiofsdWorkerPlan {
            template: WORKER_TEMPLATE,
            thread_pool_size: 4,
            readonly,
            posix_acl: false,
            xattr: false,
            cache: AttachmentCache::Auto,
            user_namespace_mapping_class: USER_NAMESPACE_MAPPING_CLASS,
            principal: fixtures::principal(),
        }
    }

    #[test]
    fn the_argv_pins_the_frozen_flag_envelope() {
        let argv = render_argv(&plan(true), &paths()).expect("renders");
        assert!(argv[0].ends_with("/virtiofsd"));
        let joined = argv.join(" ");
        assert!(joined.contains("--socket-path=/run/d2b/zones/dev/exports/0.sock"));
        assert!(joined.contains("--socket-group=d2b-virtiofs"));
        assert!(joined.contains("--shared-dir=/var/lib/d2b/vms/work-vm/store-view/live"));
        assert!(joined.contains("--thread-pool-size=4"));
        assert!(joined.contains("--cache=auto"));
        assert!(joined.contains("--sandbox=chroot"));
        assert!(joined.contains("--inode-file-handles=never"));
        assert!(joined.contains("--readonly"));
    }

    #[test]
    fn the_sandbox_and_file_handle_modes_are_never_negotiable() {
        for readonly in [true, false] {
            let argv = render_argv(&plan(readonly), &paths()).expect("renders");
            assert_eq!(
                argv.iter()
                    .filter(|arg| arg.starts_with("--sandbox="))
                    .count(),
                1
            );
            assert!(!argv.iter().any(|arg| arg == "--sandbox=namespace"));
            assert!(argv.iter().any(|arg| arg == "--inode-file-handles=never"));
            assert!(!argv.iter().any(|arg| arg.contains("prefer")));
            assert!(!argv.iter().any(|arg| arg.contains("mandatory")));
        }
    }

    #[test]
    fn a_read_write_share_omits_the_read_only_flag() {
        let argv = render_argv(&plan(false), &paths()).expect("renders");
        assert!(!argv.iter().any(|arg| arg == "--readonly"));
    }

    #[test]
    fn optional_flags_are_emitted_only_when_the_attachment_asks_for_them() {
        let mut opted = plan(true);
        opted.posix_acl = true;
        opted.xattr = true;
        let argv = render_argv(&opted, &paths()).expect("renders");
        assert!(argv.iter().any(|arg| arg == "--posix-acl"));
        assert!(argv.iter().any(|arg| arg == "--xattr"));

        let argv = render_argv(&plan(true), &paths()).expect("renders");
        assert!(!argv.iter().any(|arg| arg == "--posix-acl"));
        assert!(!argv.iter().any(|arg| arg == "--xattr"));
    }

    #[test]
    fn every_cache_mode_renders_its_frozen_spelling() {
        for (mode, expected) in [
            (AttachmentCache::Auto, "--cache=auto"),
            (AttachmentCache::Always, "--cache=always"),
            (AttachmentCache::Never, "--cache=never"),
        ] {
            let mut cached = plan(true);
            cached.cache = mode;
            let argv = render_argv(&cached, &paths()).expect("renders");
            assert!(argv.iter().any(|arg| arg == expected));
        }
    }

    #[test]
    fn a_non_absolute_or_empty_binary_is_rejected() {
        for binary in ["", "virtiofsd", "./virtiofsd"] {
            let mut broken = paths();
            broken.binary_path = binary.to_owned();
            assert_eq!(
                render_argv(&plan(true), &broken).unwrap_err(),
                VirtiofsExportError::InvalidExport
            );
        }
    }

    #[test]
    fn an_empty_socket_path_group_or_shared_dir_is_rejected() {
        for mutate in [
            |paths: &mut ResolvedWorkerPaths| paths.socket_path.clear(),
            |paths: &mut ResolvedWorkerPaths| paths.socket_group.clear(),
            |paths: &mut ResolvedWorkerPaths| paths.shared_dir.clear(),
        ] {
            let mut broken = paths();
            mutate(&mut broken);
            assert_eq!(
                render_argv(&plan(true), &broken).unwrap_err(),
                VirtiofsExportError::InvalidExport
            );
        }
    }

    #[test]
    fn a_zero_thread_pool_is_rejected() {
        let mut broken = plan(true);
        broken.thread_pool_size = 0;
        assert_eq!(
            render_argv(&broken, &paths()).unwrap_err(),
            VirtiofsExportError::InvalidExport
        );
    }

    #[test]
    fn the_store_view_share_is_the_farm_and_never_the_host_store() {
        let mut store = paths();
        store.shared_dir = "/nix/store".to_owned();
        assert_eq!(
            render_argv(&plan(true), &store).unwrap_err(),
            VirtiofsExportError::InvalidExport
        );
    }

    #[test]
    fn the_argv_carries_no_free_form_argument_channel() {
        let argv = render_argv(&plan(true), &paths()).expect("renders");
        let known = [
            "--socket-path=",
            "--socket-group=",
            "--shared-dir=",
            "--thread-pool-size=",
            "--posix-acl",
            "--xattr",
            "--cache=",
            "--sandbox=",
            "--inode-file-handles=",
            "--readonly",
        ];
        for arg in argv.iter().skip(1) {
            assert!(
                known.iter().any(|prefix| arg.starts_with(prefix)),
                "unexpected virtiofsd argument"
            );
        }
    }

    #[test]
    fn a_declared_host_capability_or_root_start_is_rejected() {
        assert!(WorkerSandbox::conformant().assert_conformant().is_ok());
        let capability = BoundedToken::parse("cap-sys-admin").expect("valid token");
        for broken in [
            WorkerSandbox::declared(vec![capability], false, SANDBOX_MODE, true),
            WorkerSandbox::declared(Vec::new(), true, SANDBOX_MODE, true),
            WorkerSandbox::declared(Vec::new(), false, "namespace", true),
            WorkerSandbox::declared(Vec::new(), false, SANDBOX_MODE, false),
        ] {
            assert_eq!(
                broken.assert_conformant().unwrap_err(),
                VirtiofsExportError::SandboxInvariantViolated
            );
        }
    }

    #[test]
    fn the_thread_pool_falls_back_to_the_guest_vcpu_count() {
        let export = fixtures::export("read-only");
        let view = fixtures::read_only_view();
        let plan = VirtiofsdWorkerPlan::for_export(&export, &view, 8, fixtures::principal())
            .expect("conformant plan");
        assert_eq!(plan.thread_pool_size, 8);
        assert!(plan.readonly);
    }

    #[test]
    fn a_write_export_over_a_read_only_view_is_rejected() {
        let export = fixtures::export("read-write");
        let view = fixtures::read_only_view();
        assert_eq!(
            VirtiofsdWorkerPlan::for_export(&export, &view, 4, fixtures::principal()).unwrap_err(),
            VirtiofsExportError::ViewRightsInsufficient
        );
    }

    #[test]
    fn a_non_never_inode_file_handle_setting_is_rejected() {
        let export = fixtures::export_with_settings(
            "read-only",
            serde_json::json!({ "inodeFileHandles": "prefer" }),
        );
        let view = fixtures::read_only_view();
        assert_eq!(
            VirtiofsdWorkerPlan::for_export(&export, &view, 4, fixtures::principal()).unwrap_err(),
            VirtiofsExportError::SandboxInvariantViolated
        );
    }
}
