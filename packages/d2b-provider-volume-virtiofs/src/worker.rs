//! The binding-owned virtiofsd worker.
//!
//! The worker's sandbox posture is frozen by ADR 0021: zero host
//! capabilities, no start as root, a chroot sandbox whose privileges live
//! only inside a broker-pre-established user namespace, and no
//! `open_by_handle_at` support. This module owns the path-free worker
//! plan and the sandbox posture it must declare.

use serde::Serialize;

use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::volume::{AttachmentAccess, AttachmentCache, ViewRight, ViewSpec};

use crate::error::VirtiofsBindingError;
use crate::bindings::StoredBinding;

/// The frozen sandbox mode of every virtiofsd worker.
pub const SANDBOX_MODE: &str = "chroot";
/// The frozen inode file-handle mode of every virtiofsd worker.
pub const INODE_FILE_HANDLES: &str = "never";
/// The Process template every binding-owned worker uses.
pub const WORKER_TEMPLATE: &str = "virtiofsd-worker";
/// The user-namespace mapping class the worker resolves through its
/// launch port.
pub const USER_NAMESPACE_MAPPING_CLASS: &str = "process-principal-root";

/// The path-free launch plan of one binding's virtiofsd worker.
///
/// It names no socket path, no shared directory, no numeric group, and
/// no store path. The effect adapter joins it to the private root
/// descriptor and the private socket path it alone derives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtiofsdWorkerPlan {
    /// The Process template this worker instantiates.
    pub template: &'static str,
    /// The number of worker threads, resolved from the target Guest's
    /// vcpu count.
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
    /// Build the worker plan for one binding.
    ///
    /// `vcpu_count` supplies the thread-pool size. The plan is read-only
    /// whenever the binding declares read-only access or the selected
    /// view grants no write right, so a view that never granted write
    /// cannot be widened by a binding. The neutral binding envelope
    /// carries no attachment tuning (KTD1): the serving posture is the
    /// frozen default profile -- no POSIX ACLs, no xattrs, `auto` page
    /// cache, and `never` inode file handles -- so the ADR 0021 sandbox
    /// invariant holds by construction.
    pub fn for_binding(
        binding: &StoredBinding,
        view: &ViewSpec,
        vcpu_count: u32,
        principal: BoundedToken,
    ) -> Result<Self, VirtiofsBindingError> {
        if vcpu_count == 0 {
            return Err(VirtiofsBindingError::InvalidBinding);
        }
        let writes = view.rights().contains(&ViewRight::Write);
        let access = binding.spec().access();
        if access != AttachmentAccess::ReadOnly && !writes {
            return Err(VirtiofsBindingError::ViewRightsInsufficient);
        }
        Ok(Self {
            template: WORKER_TEMPLATE,
            thread_pool_size: vcpu_count,
            readonly: access == AttachmentAccess::ReadOnly || !writes,
            posix_acl: false,
            xattr: false,
            cache: AttachmentCache::Auto,
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
    pub fn assert_conformant(&self) -> Result<(), VirtiofsBindingError> {
        if !self.capability_classes.is_empty()
            || self.start_root
            || self.sandbox_mode != SANDBOX_MODE
            || !self.read_only_root
        {
            return Err(VirtiofsBindingError::SandboxInvariantViolated);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::fixtures;

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
                VirtiofsBindingError::SandboxInvariantViolated
            );
        }
    }

    #[test]
    fn the_thread_pool_falls_back_to_the_guest_vcpu_count() {
        let binding = fixtures::binding("read-only");
        let view = fixtures::read_only_view();
        let plan = VirtiofsdWorkerPlan::for_binding(&binding, &view, 8, fixtures::principal())
            .expect("conformant plan");
        assert_eq!(plan.thread_pool_size, 8);
        assert!(plan.readonly);
    }

    #[test]
    fn a_write_binding_over_a_read_only_view_is_rejected() {
        let binding = fixtures::binding("read-write");
        let view = fixtures::read_only_view();
        assert_eq!(
            VirtiofsdWorkerPlan::for_binding(&binding, &view, 4, fixtures::principal()).unwrap_err(),
            VirtiofsBindingError::ViewRightsInsufficient
        );
    }

    #[test]
    fn the_frozen_default_posture_survives_the_neutral_envelope() {
        // The neutral envelope carries no attachment tuning (KTD1); the
        // plan keeps the frozen default profile the pre-cutover default
        // settings produced (KTD9).
        let binding = fixtures::binding("read-only");
        let view = fixtures::read_only_view();
        let plan = VirtiofsdWorkerPlan::for_binding(&binding, &view, 4, fixtures::principal())
            .expect("conformant plan");
        assert_eq!(plan.template, WORKER_TEMPLATE);
        assert!(!plan.posix_acl);
        assert!(!plan.xattr);
        assert_eq!(plan.cache, AttachmentCache::Auto);
        assert_eq!(plan.user_namespace_mapping_class, USER_NAMESPACE_MAPPING_CLASS);
        assert!(WorkerSandbox::conformant().assert_conformant().is_ok());
    }
}
