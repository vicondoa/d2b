//! Recording test doubles shared with downstream crates' unit tests.
//!
//! Gated behind the `test-support` Cargo feature so production consumers
//! never pull this in.
//!
//! The double's ordered log is the toolkit's `SharedLog`
//! (`d2b_provider_toolkit::testing`), the canonical recorder shape every
//! family crate's test-support module shares. It scripts the whole device
//! mediation seam - the claim, the two observations, and the two releases -
//! and counts what each pass actually changed on the host, so a test can pin
//! that a restart re-adopts a realized attachment instead of attaching the
//! consumer a second time. Every script knob is an atomic, so the double
//! holds no lock across a suspension point.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use d2b_provider_toolkit::testing::SharedLog;

use crate::facets::{
    AttachmentMediation, AttachmentObservation, DeviceAttachment, DeviceBindingEffectFacets,
    DeviceEstablishOutcome, DeviceRefusal,
};

/// The refusal a scripted establish answers with.
///
/// The refusal's own text is the trusted adapter's, not this double's, so
/// the double scripts the refusal class - which is what the driver's
/// classification and the stable provider reason are derived from - and
/// supplies its own bounded detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptedRefusal {
    /// The trusted inventory does not admit the named function.
    Unauthorized,
    /// The capability is no longer backed: the claim is stale.
    Stale,
    /// Another live relationship holds the capability exclusively.
    Conflicted,
    /// The mediation adapter could not complete the drive.
    MediationFailed,
}

impl ScriptedRefusal {
    const fn code(self) -> u8 {
        match self {
            Self::Unauthorized => 1,
            Self::Stale => 2,
            Self::Conflicted => 3,
            Self::MediationFailed => 4,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Unauthorized),
            2 => Some(Self::Stale),
            3 => Some(Self::Conflicted),
            4 => Some(Self::MediationFailed),
            _ => None,
        }
    }

    fn refusal(self) -> DeviceRefusal {
        let detail = match self {
            Self::Unauthorized => "scripted: the function is not in the trusted inventory",
            Self::Stale => "scripted: the capability is no longer backed",
            Self::Conflicted => "scripted: another live claim holds the capability",
            Self::MediationFailed => "scripted: the adapter could not complete the drive",
        };
        match self {
            Self::Unauthorized => DeviceRefusal::Unauthorized {
                detail: detail.to_owned(),
            },
            Self::Stale => DeviceRefusal::Stale {
                detail: detail.to_owned(),
            },
            Self::Conflicted => DeviceRefusal::Conflicted {
                detail: detail.to_owned(),
            },
            Self::MediationFailed => DeviceRefusal::MediationFailed {
                detail: detail.to_owned(),
            },
        }
    }
}

/// Scripted device mediation over the caller's ordered log, so the tests
/// assert one sequence across the driver's passes and the port.
pub struct FakeAttachmentEffects {
    log: SharedLog,
    /// Whether this double has already realized the attachment it was asked
    /// for: the idempotence the port contract promises, so a second drive of
    /// the same attachment reports it in place instead of attaching twice.
    realized: AtomicBool,
    ready: AtomicBool,
    held: AtomicBool,
    refused: AtomicU8,
    fail_release_attachment: AtomicBool,
    fail_release_slot: AtomicBool,
    realized_count: AtomicUsize,
    released_slot_count: AtomicUsize,
}

impl FakeAttachmentEffects {
    /// A fresh double with its own ordered log.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            log: SharedLog::new(),
            realized: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            held: AtomicBool::new(false),
            refused: AtomicU8::new(0),
            fail_release_attachment: AtomicBool::new(false),
            fail_release_slot: AtomicBool::new(false),
            realized_count: AtomicUsize::new(0),
            released_slot_count: AtomicUsize::new(0),
        })
    }

    /// A double whose ordered log is shared with the caller's manager logger,
    /// so manager calls and mediation effects read as one sequence.
    pub fn shared(log: SharedLog) -> Arc<Self> {
        Arc::new(Self {
            log,
            realized: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            held: AtomicBool::new(false),
            refused: AtomicU8::new(0),
            fail_release_attachment: AtomicBool::new(false),
            fail_release_slot: AtomicBool::new(false),
            realized_count: AtomicUsize::new(0),
            released_slot_count: AtomicUsize::new(0),
        })
    }

    /// Script every drive to be refused with this exact refusal class.
    pub fn refuse_establish(&self, refusal: ScriptedRefusal) {
        self.refused.store(refusal.code(), Ordering::SeqCst);
    }

    /// Script the attachment to be observable now.
    pub fn make_ready(&self) {
        self.ready.store(true, Ordering::SeqCst);
    }

    /// Script the consumer to still hold the attachment: the drain gate must
    /// block.
    pub fn make_held(&self) {
        self.held.store(true, Ordering::SeqCst);
    }

    /// Script `release_attachment` to fail with an operational error.
    pub fn set_fail_release_attachment(&self, fail: bool) {
        self.fail_release_attachment.store(fail, Ordering::SeqCst);
    }

    /// Script `release_slot` to fail with an operational error.
    pub fn set_fail_release_slot(&self, fail: bool) {
        self.fail_release_slot.store(fail, Ordering::SeqCst);
    }

    /// How many times this double actually realized an attachment, which is
    /// the count a restart must not increase.
    pub fn realized_count(&self) -> usize {
        self.realized_count.load(Ordering::SeqCst)
    }

    /// How many times the consumer's device slot was released.
    pub fn released_slot_count(&self) -> usize {
        self.released_slot_count.load(Ordering::SeqCst)
    }

    /// The ordered mediation log.
    pub fn call_order(&self) -> Vec<String> {
        self.log.entries()
    }

    /// The facet set the plane and this crate's tests build the driver and
    /// the effects service from: the scripted mediation behind the two
    /// facets.
    pub fn facet_set(self: &Arc<Self>) -> DeviceBindingEffectFacets {
        DeviceBindingEffectFacets {
            mediation: Arc::new(ScriptedMediation(Arc::clone(self))),
            observation: Arc::new(ScriptedObservation(Arc::clone(self))),
        }
    }

    /// The scripted establish: refused when a refusal class is armed,
    /// otherwise the idempotent answer - realized once, in place afterwards.
    fn establish(&self) -> Result<DeviceEstablishOutcome, DeviceRefusal> {
        self.log.record("establish".to_owned());
        if let Some(refusal) = ScriptedRefusal::from_code(self.refused.load(Ordering::SeqCst)) {
            return Err(refusal.refusal());
        }
        if self.realized.swap(true, Ordering::SeqCst) {
            return Ok(DeviceEstablishOutcome::AlreadyRealized);
        }
        self.realized_count.fetch_add(1, Ordering::SeqCst);
        Ok(DeviceEstablishOutcome::Realized)
    }
}

/// The scripted mediation facet: the claim-and-attach drive and the two
/// release steps, recorded on the shared double.
struct ScriptedMediation(Arc<FakeAttachmentEffects>);

#[async_trait::async_trait]
impl AttachmentMediation for ScriptedMediation {
    async fn establish(
        &self,
        _attachment: &DeviceAttachment,
    ) -> Result<DeviceEstablishOutcome, DeviceRefusal> {
        self.0.establish()
    }

    async fn release_attachment(&self, _attachment: &DeviceAttachment) -> Result<(), String> {
        self.0.log.record("release-attachment".to_owned());
        if self.0.fail_release_attachment.load(Ordering::SeqCst) {
            return Err("scripted attachment release failure".to_owned());
        }
        // Releasing the attachment leaves the claim free: a later drive of
        // the same row realizes it again rather than reporting a stale one.
        self.0.realized.store(false, Ordering::SeqCst);
        self.0.held.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn release_slot(&self, _attachment: &DeviceAttachment) -> Result<(), String> {
        self.0.log.record("release-slot".to_owned());
        if self.0.fail_release_slot.load(Ordering::SeqCst) {
            return Err("scripted slot release failure".to_owned());
        }
        self.0.released_slot_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// The scripted observation facet: the readiness and drain-gate evidence,
/// recorded on the shared double.
struct ScriptedObservation(Arc<FakeAttachmentEffects>);

#[async_trait::async_trait]
impl AttachmentObservation for ScriptedObservation {
    async fn attachment_ready(&self, _attachment: &DeviceAttachment) -> bool {
        self.0.log.record("ready".to_owned());
        self.0.ready.load(Ordering::SeqCst)
    }

    async fn attachment_held(&self, _attachment: &DeviceAttachment) -> Result<bool, String> {
        self.0.log.record("held".to_owned());
        Ok(self.0.held.load(Ordering::SeqCst))
    }
}