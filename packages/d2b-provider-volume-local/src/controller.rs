//! The volume-local controller.
//!
//! It is the sole Volume writer. It validates the Volume base spec,
//! resolves the source through the injected source port, reconciles every
//! declared layout entry in parent-before-child order through the
//! injected layout port, admits attachments, and writes the aggregated
//! status. It performs no privileged mutation itself.

use std::collections::BTreeSet;

use d2b_contracts::v3::ResourceUid;
use d2b_contracts::v3::execution_policy::BoundedToken;
use d2b_contracts::v3::volume::{SourceKind, VolumeSpec};

use crate::error::VolumeLocalError;
use crate::identity::VolumeRootHandle;
use crate::layout::{ConditionSeverity, EntryCondition, EntryRequest, plan_cleanup, plan_entry};
use crate::port::{QuotaCapability, VolumeLayoutEffectPort, VolumeSourceEffectPort};
use crate::source::{SourcePolicyCatalog, validate_source_spec};
use crate::status::{AttachmentState, AttachmentStatus, LayoutPhase, VolumeStatusReport};
use crate::views::admit_attachments;

/// The declared conformance profile of one volume-local instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeLocalProfile {
    provider: BoundedToken,
    supported_source_kinds: BTreeSet<SourceKind>,
    supports_shared_write: bool,
    source_policies: Option<SourcePolicyCatalog>,
}

impl VolumeLocalProfile {
    /// Declare a profile. A Provider that supports no source kind is
    /// rejected.
    pub fn new(
        provider: BoundedToken,
        supported_source_kinds: BTreeSet<SourceKind>,
        supports_shared_write: bool,
    ) -> Result<Self, VolumeLocalError> {
        if supported_source_kinds.is_empty() {
            return Err(VolumeLocalError::InvalidSpec);
        }
        Ok(Self {
            provider,
            supported_source_kinds,
            supports_shared_write,
            source_policies: None,
        })
    }

    /// The default shipped profile: every source kind, no shared write.
    pub fn shipped() -> Self {
        Self {
            provider: BoundedToken::parse("volume-local").expect("frozen provider name"),
            supported_source_kinds: [
                SourceKind::LocalPath,
                SourceKind::BlockImage,
                SourceKind::Tmpfs,
            ]
            .into_iter()
            .collect(),
            supports_shared_write: false,
            source_policies: None,
        }
    }

    /// Borrow the Provider name.
    pub const fn provider(&self) -> &BoundedToken {
        &self.provider
    }

    /// Borrow the supported source kinds.
    pub const fn supported_source_kinds(&self) -> &BTreeSet<SourceKind> {
        &self.supported_source_kinds
    }

    /// Whether this Provider admits `shared-write` attachments.
    pub const fn supports_shared_write(&self) -> bool {
        self.supports_shared_write
    }

    /// Attach the private source-policy catalog used for strict admission.
    pub fn with_source_policy_catalog(mut self, catalog: SourcePolicyCatalog) -> Self {
        self.source_policies = Some(catalog);
        self
    }
}

/// The volume-local controller over its two injected effect ports.
#[derive(Debug)]
pub struct VolumeLocalController<S, L> {
    profile: VolumeLocalProfile,
    source: S,
    layout: L,
}

impl<S: VolumeSourceEffectPort, L: VolumeLayoutEffectPort> VolumeLocalController<S, L> {
    /// Build a controller over the injected ports.
    pub const fn new(profile: VolumeLocalProfile, source: S, layout: L) -> Self {
        Self {
            profile,
            source,
            layout,
        }
    }

    /// Borrow the declared profile.
    pub const fn profile(&self) -> &VolumeLocalProfile {
        &self.profile
    }

    /// Reconcile one Volume and return its public status projection.
    pub async fn reconcile(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
    ) -> Result<VolumeStatusReport, VolumeLocalError> {
        validate_source_spec(spec)?;
        if let Some(catalog) = &self.profile.source_policies {
            catalog.validate(spec)?;
        }
        let kind = spec.source().settings().kind();
        if !self.profile.supported_source_kinds().contains(&kind) {
            return Err(VolumeLocalError::SourceKindUnsupported);
        }
        let attachments = admit_attachments(spec, self.profile.supports_shared_write())?;

        let root = self
            .source
            .resolve_root(spec.source().settings().source_policy_id(), kind)
            .await?;
        self.assert_quota(spec, &root).await?;

        let marker = self.layout.marker_state(&root).await?;
        let mut phase = LayoutPhase::Pending;
        let mut conditions = Vec::new();

        let mut ordered_entries: Vec<_> = spec.layout().iter().collect();
        ordered_entries.sort_by_key(|entry| {
            (
                !entry.path().is_empty(),
                entry.path().split('/').count(),
                entry.path(),
            )
        });

        for declared in ordered_entries {
            let entry = EntryRequest::resolve(volume_uid, declared)?;
            let observed = self.layout.observe(&root, &entry).await?;
            let plan = plan_entry(&entry, &observed, marker);
            if let Some(condition) = plan.condition {
                conditions.push(condition);
                phase = phase.worse(severity_phase(condition));
            }
            if plan.recreate {
                self.layout.cleanup(&root, &entry).await?;
            }
            if plan.provision {
                self.layout.provision(&root, &entry).await?;
            }
            if !plan.repair.is_empty() {
                self.layout.repair(&root, &entry, &plan.repair).await?;
            }
            if plan.apply_acl {
                self.layout.apply_acl(&root, &entry).await?;
            }
            if plan.condition.is_none() {
                phase = phase.worse(LayoutPhase::Ready);
            }
        }

        Ok(VolumeStatusReport {
            provider: self.profile.provider().clone(),
            kind: spec.kind(),
            layout_phase: if spec.layout().is_empty() {
                LayoutPhase::Ready
            } else {
                phase
            },
            layout_conditions: conditions,
            attachment_statuses: attachments
                .into_iter()
                .map(|plan| AttachmentStatus {
                    execution_ref: plan.execution_ref,
                    view: plan.view,
                    access: plan.access,
                    state: AttachmentState::Pending,
                    export_ready: false,
                    guest_mount_ready: false,
                })
                .collect(),
        })
    }

    /// Remove every declared entry whose cleanup policy admits removal.
    ///
    /// Returns the digests of the entries that were removed. An entry
    /// with `cleanup-policy: never` is always preserved, and a
    /// process-scoped entry is removed only with proof its owner is gone.
    pub async fn cleanup(
        &self,
        volume_uid: &ResourceUid,
        spec: &VolumeSpec,
    ) -> Result<Vec<crate::identity::EntryDigest>, VolumeLocalError> {
        let root = self
            .source
            .resolve_root(
                spec.source().settings().source_policy_id(),
                spec.source().settings().kind(),
            )
            .await?;
        let mut removed = Vec::new();
        let mut ordered_entries: Vec<_> = spec.layout().iter().collect();
        ordered_entries.sort_by_key(|entry| {
            (
                !entry.path().is_empty(),
                core::cmp::Reverse(entry.path().split('/').count()),
                core::cmp::Reverse(entry.path()),
            )
        });
        for declared in ordered_entries {
            let entry = EntryRequest::resolve(volume_uid, declared)?;
            let observed = self.layout.observe(&root, &entry).await?;
            if plan_cleanup(&entry, &observed) {
                self.layout.cleanup(&root, &entry).await?;
                removed.push(entry.digest());
            }
        }
        Ok(removed)
    }

    async fn assert_quota(
        &self,
        spec: &VolumeSpec,
        root: &VolumeRootHandle,
    ) -> Result<(), VolumeLocalError> {
        use d2b_contracts::v3::volume::QuotaEnforcement;
        let Some(quota) = spec.quota() else {
            return Ok(());
        };
        if quota.enforcement() != QuotaEnforcement::Hard {
            return Ok(());
        }
        match self.source.quota_capability(root).await? {
            QuotaCapability::Enforceable => Ok(()),
            QuotaCapability::Unenforceable => Err(VolumeLocalError::QuotaUnenforceable),
        }
    }
}

const fn severity_phase(condition: EntryCondition) -> LayoutPhase {
    match condition.severity {
        ConditionSeverity::Degraded => LayoutPhase::Degraded,
        ConditionSeverity::Failed => LayoutPhase::Failed,
    }
}
