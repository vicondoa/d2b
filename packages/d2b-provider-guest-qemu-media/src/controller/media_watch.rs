//! Volume readiness and virtio-blk attachment validation.

use d2b_contracts_resource::v3::ResourceRef;

use crate::types::RemovableVolumeRef;

/// Volume phase observed by the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumePhase {
    /// Volume is not ready.
    Pending,
    /// Volume is ready for attachment.
    Ready,
    /// Volume failed closed.
    Failed,
    /// Volume was deleted.
    Deleted,
}

/// One Volume attachment observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeAttachment {
    /// Guest execution reference.
    pub execution_ref: ResourceRef,
    /// Attachment transport.
    pub transport: String,
    /// Named view.
    pub view: String,
    /// Access mode.
    pub access: String,
}

/// One watched Volume observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeObservation {
    /// Volume reference.
    pub volume_ref: ResourceRef,
    /// Observed Zone.
    pub zone: String,
    /// Volume phase.
    pub phase: VolumePhase,
    /// Current attachments.
    pub attachments: Vec<VolumeAttachment>,
}

impl VolumeObservation {
    /// Build a pending observation.
    pub fn pending(volume_ref: ResourceRef) -> Self {
        Self {
            volume_ref,
            zone: "default".to_owned(),
            phase: VolumePhase::Pending,
            attachments: Vec::new(),
        }
    }

    /// Build a ready virtio-blk observation for hermetic tests.
    pub fn ready_virtio_blk(volume_ref: ResourceRef) -> Self {
        let guest_ref = ResourceRef::parse("Guest/media-vm").expect("fixture ref");
        Self {
            volume_ref,
            zone: "default".to_owned(),
            phase: VolumePhase::Ready,
            attachments: vec![VolumeAttachment {
                execution_ref: guest_ref,
                transport: "virtio-blk".to_owned(),
                view: "guest-attach".to_owned(),
                access: "read-only".to_owned(),
            }],
        }
    }

    /// Return whether this Volume has a matching virtio-blk attachment.
    pub fn has_virtio_blk_for(&self, guest_ref: &ResourceRef, view: &str) -> bool {
        self.attachments.iter().any(|attachment| {
            attachment.execution_ref == *guest_ref
                && attachment.transport == "virtio-blk"
                && attachment.view == view
        })
    }
}

/// Result of media dependency observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaReadiness {
    /// Boot media is ready.
    pub boot_ready: bool,
    /// All removable media are ready.
    pub removable_ready: bool,
}

/// Media dependency watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaWatch {
    guest_ref: ResourceRef,
    boot_media_ref: Option<ResourceRef>,
    boot_media_view: String,
    removable_refs: Vec<RemovableVolumeRef>,
}

impl MediaWatch {
    /// Construct a media watch.
    pub fn new(
        guest_ref: ResourceRef,
        boot_media_ref: Option<ResourceRef>,
        removable_refs: Vec<RemovableVolumeRef>,
    ) -> Self {
        Self {
            guest_ref,
            boot_media_ref,
            boot_media_view: "guest-attach".to_owned(),
            removable_refs,
        }
    }

    /// Set the configured boot Volume view.
    pub fn with_boot_media_view(mut self, view: impl Into<String>) -> Self {
        self.boot_media_view = view.into();
        self
    }

    /// Validate all required Volume observations.
    pub fn observe(
        &self,
        observations: impl IntoIterator<Item = VolumeObservation>,
    ) -> Result<MediaReadiness, MediaObservationError> {
        let observations: Vec<_> = observations.into_iter().collect();
        let find = |reference: &ResourceRef| {
            observations
                .iter()
                .find(|observation| observation.volume_ref == *reference)
        };
        let boot_ready = match &self.boot_media_ref {
            None => true,
            Some(reference) => {
                let observation = find(reference).ok_or(MediaObservationError::Missing)?;
                if observation.phase == VolumePhase::Failed {
                    return Err(MediaObservationError::Failed);
                }
                if observation.phase != VolumePhase::Ready {
                    return Err(MediaObservationError::NotReady);
                }
                observation.has_virtio_blk_for(&self.guest_ref, &self.boot_media_view)
            }
        };
        if !boot_ready {
            return Err(MediaObservationError::MissingVirtioBlk);
        }
        for removable in &self.removable_refs {
            let observation = find(&removable.volume_ref).ok_or(MediaObservationError::Missing)?;
            if observation.phase == VolumePhase::Failed {
                return Err(MediaObservationError::Failed);
            }
            if observation.phase != VolumePhase::Ready {
                return Err(MediaObservationError::NotReady);
            }
            if !observation.has_virtio_blk_for(&self.guest_ref, &removable.view) {
                return Err(MediaObservationError::MissingVirtioBlk);
            }
        }
        Ok(MediaReadiness {
            boot_ready,
            removable_ready: true,
        })
    }
}

/// Media dependency observation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaObservationError {
    /// A required Volume was absent.
    Missing,
    /// A Volume was not ready.
    NotReady,
    /// A Volume failed.
    Failed,
    /// A Volume has no matching virtio-blk attachment.
    MissingVirtioBlk,
}

impl MediaObservationError {
    /// Return the stable condition code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Missing | Self::NotReady => "media-volume-not-ready",
            Self::Failed => "media-volume-failed",
            Self::MissingVirtioBlk => "media-volume-access-denied",
        }
    }
}
