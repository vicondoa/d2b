//! The observed lifecycle and realization vocabulary of a binding.
//!
//! These are the words a provider reports back about a relationship that
//! already exists, and the words an implementation declares it can actually
//! realize. They are kept apart from the desired request vocabulary on
//! purpose: a request cannot name its own observed state, and a support set is
//! an implementation's declaration rather than a consumer's ask.
//!
//! # An observed state is never a desired field
//!
//! [`BindingLifecycleState`], [`CompletionCondition`], and
//! [`ReleaseOutcome`] describe what is true now. Nothing a consumer writes into
//! a request can move one of them, and an uncertain observation stays
//! `Unknown` or `Degraded` rather than being reported as granted use.
//!
//! # A support declaration is checked, never assumed
//!
//! [`BindingRealizationSupport`] is what one implementation declares it can
//! realize. A facet outside that set is refused at admission rather than
//! skipped, so an unapplied mount policy, an unclaimed device, or an unproven
//! endpoint cannot pass as a satisfied presentation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{
    authority::RefusalReason, binding::BindingContractError, execution_policy::redacted_debug,
};

/// The observed lifecycle of one binding relationship.
///
/// This is a status vocabulary, never a desired field: a request cannot say it
/// is `Active`, and an uncertain observation stays `Unknown` or `Degraded`
/// rather than being reported as granted use.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum BindingLifecycleState {
    /// Identities are declared but access is not authorized yet.
    Requested,
    /// The exact requested relationship is authorized against current evidence.
    Admitted,
    /// Source-side access and delivery prerequisites exist.
    Prepared,
    /// The consumer is using the prepared relationship.
    Active,
    /// New use is blocked ahead of typed release.
    Revoking,
    /// Outstanding use is being driven to the kind's safe state.
    Draining,
    /// No outstanding use or lease remains for this relationship.
    Released,
    /// Admission failed for one typed reason.
    Refused,
    /// The relationship exists but an effect cannot be proven effective.
    Degraded,
    /// Completion could not be proven either way.
    Unknown,
}

impl BindingLifecycleState {
    /// Whether this observed state still admits new use.
    pub const fn admits_new_use(self) -> bool {
        matches!(
            self,
            Self::Requested | Self::Admitted | Self::Prepared | Self::Active
        )
    }

    /// Whether this observed state is finished for good.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Released | Self::Refused)
    }

    /// Whether this observed state proves an effective result.
    ///
    /// `Degraded` and `Unknown` are uncertainty, not success: recovery has to
    /// prove adoption or report the refusal rather than read either as
    /// granted access.
    pub const fn proves_effect(self) -> bool {
        matches!(self, Self::Prepared | Self::Active | Self::Released)
    }
}

/// One side of a binding relationship being ready.
///
/// Source preparation and consumer-side completion stay separate so a
/// relationship that must exist before its consumer starts never forms a
/// startup cycle with the observation that consumer can see it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum CompletionCondition {
    /// Not established yet.
    Pending,
    /// Established under this relationship's own identity.
    Complete,
    /// Not established, for one typed reason.
    Failed(RefusalReason),
}

impl CompletionCondition {
    /// Whether this condition is established.
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// What happened to the relationship's use.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ReleaseOutcome {
    /// Use is outstanding or has not started.
    Outstanding,
    /// New use is blocked and existing use is being driven closed.
    Draining,
    /// No outstanding use remains; the shared source itself is untouched.
    Released,
}

/// The realization facet a requested presentation depends on.
///
/// A presentation the selected backend cannot enforce is refused rather than
/// skipped: an unapplied mount policy, an unclaimed device, or an unproven
/// endpoint is not success.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum BindingRealizationFacet {
    /// A private mount tree carrying the exact named view at the destination.
    FilesystemPresentation,
    /// A block device in a consumer device slot.
    ConsumerDeviceSlot,
    /// A verified device descriptor or mediated attachment.
    DeviceAttachment,
    /// An inherited interface in the consumer's own network namespace.
    NamespaceInterface,
    /// Membership in the provider-owned shared fabric realization.
    SharedFabric,
    /// A verified connected or listening descriptor for the exact endpoint.
    EndpointDescriptor,
    /// A private binding of the exact socket where the backend needs a name.
    EndpointPathname,
    /// Credential material delivered inside an admitted delivery session.
    CredentialDelivery,
}

/// What one binding implementation declares it can realize.
#[derive(Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BindingRealizationSupport {
    facets: Vec<BindingRealizationFacet>,
}

impl BindingRealizationSupport {
    /// Construct a support set, requiring unique entries.
    pub fn new(facets: Vec<BindingRealizationFacet>) -> Result<Self, BindingContractError> {
        let mut sorted = facets.clone();
        sorted.sort_unstable();
        sorted.dedup();
        if sorted.len() != facets.len() {
            return Err(BindingContractError::InvalidCollection);
        }
        Ok(Self { facets })
    }

    /// Whether the implementation realizes `facet`.
    pub fn realizes(&self, facet: BindingRealizationFacet) -> bool {
        self.facets.contains(&facet)
    }

    /// Borrow the declared facets.
    pub fn facets(&self) -> &[BindingRealizationFacet] {
        &self.facets
    }
}

redacted_debug!(BindingRealizationSupport);
