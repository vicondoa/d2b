//! The Guest family's execution-parent classification (AE31-AE33).
//!
//! The old flattened `Guest.spec` execution-parent fragment carried three
//! unrelated meanings in one attachment list. This module classifies them
//! instead of copying them, using the shared `ExecutionParentInput` rather
//! than a second vocabulary of its own:
//!
//! - `deviceAttachments` and `networkAttachments` are a CHILD TARGET-SUPPORT
//!   CEILING (AE31). They bound what a child of this Guest may request, so
//!   they become one `ChildSupportCeiling`. A ceiling creates no binding, no
//!   reservation, and no access: a Guest that declares a Device it cannot
//!   use is no better off than one that declared nothing, and a ceiling
//!   never becomes a boot dependency.
//! - `volumeAttachmentDefaults` are CHILD REQUEST DEFAULTS (AE33). Each entry
//!   becomes one `ChildRequestDefaults` naming the single child it shapes.
//!   Applying it fills an unset field of that child's request and refuses
//!   every other consumer, so the Guest gains no access of its own.
//! - `defaultDomain`, `allowedDomains`, `defaultUserRef`, and `budget` are
//!   EXECUTION-PARENT FACTS. They are preserved verbatim in
//!   [`GuestExecutionFacts`] and grant no resource access.
//!
//! A parent use (AE32) is NOT one of these fields. The Guest's own storage
//! consumption is composed by this provider from the children it creates, so
//! it is attached through [`GuestExecutionParent::with_parent_use`] rather
//! than being read back out of the flattened fragment. That is what keeps a
//! child default from widening into a Guest binding: the only path into
//! [`GuestExecutionParent::parent_use`] is an explicit request the provider
//! composed.

use core::fmt;

use d2b_contracts_resource::v3::{
    BindingContractError, BindingKind, BindingSupportEntry, BoundedToken, BudgetSpec,
    ChildBindingRequest, ChildRequestDefaults, ChildSupportCeiling, DefaultedSource,
    ExecutionDomain, ExecutionParentInput, ExecutionPolicy, RequestedRights, ResourceRef,
    VolumeBindingRequest,
    resource_schema::{CanonicalJsonObject, CanonicalJsonValue},
};

/// One classified Guest input.
///
/// The parent-use arm is typed against the Volume request shape, because a
/// Guest's own consumption in this family is storage: the system Volume it
/// boots from and any export it mounts.
pub type GuestExecutionParentInput = ExecutionParentInput<VolumeBindingRequest>;

/// The Guest object's own non-authority facts.
///
/// These are the values an execution parent carries about itself: which
/// domains a child may run in, which User a child without an explicit
/// identity resolves to, and the aggregate budget a child is evaluated
/// against. None of them names a source, a view, a path, or a grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestExecutionFacts {
    default_domain: ExecutionDomain,
    allowed_domains: Vec<ExecutionDomain>,
    default_user_ref: Option<ResourceRef>,
    budget: BudgetSpec,
}

impl GuestExecutionFacts {
    /// Construct the facts from one execution-parent policy.
    fn from_policy(policy: &ExecutionPolicy) -> Self {
        Self {
            default_domain: policy.default_domain(),
            allowed_domains: policy.allowed_domains().to_vec(),
            default_user_ref: policy.default_user_ref().cloned(),
            budget: policy.budget().clone(),
        }
    }

    /// Return the default child execution domain.
    pub const fn default_domain(&self) -> ExecutionDomain {
        self.default_domain
    }

    /// Borrow the admitted child execution domains.
    pub fn allowed_domains(&self) -> &[ExecutionDomain] {
        &self.allowed_domains
    }

    /// Borrow the fallback User identity.
    pub const fn default_user_ref(&self) -> Option<&ResourceRef> {
        self.default_user_ref.as_ref()
    }

    /// Borrow the aggregate child budget.
    pub const fn budget(&self) -> &BudgetSpec {
        &self.budget
    }
}

/// One classified Guest execution-parent fragment.
///
/// The classified inputs are kept in one ordered list rather than split into
/// three fields so that "what did this fragment mean" stays a single
/// question with a single answer, and so an arm that was never constructed
/// is visible as an empty iterator instead of a defaulted field.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestExecutionParent {
    facts: GuestExecutionFacts,
    inputs: Vec<GuestExecutionParentInput>,
}

impl GuestExecutionParent {
    /// Borrow the Guest's own execution-parent facts.
    pub const fn facts(&self) -> &GuestExecutionFacts {
        &self.facts
    }

    /// Borrow the classified inputs in stable order.
    pub fn inputs(&self) -> &[GuestExecutionParentInput] {
        &self.inputs
    }

    /// Borrow the child target-support ceiling.
    ///
    /// The ceiling is always present, even when it admits nothing: a missing
    /// attachment list is a ceiling that admits nothing, which is what makes
    /// an undeclared child request a refusal at admission rather than a
    /// default allowance.
    pub fn support_ceiling(&self) -> Option<&ChildSupportCeiling> {
        self.inputs
            .iter()
            .find_map(GuestExecutionParentInput::support_ceiling)
    }

    /// Borrow the Guest's own consumption requests (AE32).
    ///
    /// Each request's consumer is this Guest. Nothing else reaches this
    /// iterator: a ceiling and a child default are inputs to someone else's
    /// decision.
    pub fn parent_use(&self) -> impl Iterator<Item = &VolumeBindingRequest> {
        self.inputs
            .iter()
            .filter_map(GuestExecutionParentInput::parent_use)
    }

    /// Borrow the defaults supplied to named children (AE33).
    pub fn child_defaults(&self) -> impl Iterator<Item = &ChildRequestDefaults> {
        self.inputs
            .iter()
            .filter_map(GuestExecutionParentInput::child_defaults)
    }

    /// Whether this Guest declares any consumption of its own.
    ///
    /// A fragment with no parent use is the honest answer for a Guest that
    /// only declares ceilings and defaults, and it is checkable rather than
    /// implied by the absence of a field.
    pub fn claims_parent_use(&self) -> bool {
        self.parent_use().next().is_some()
    }

    /// Attach the Guest's own consumption to the classified fragment.
    ///
    /// # Errors
    ///
    /// Refuses a request whose consumer is not this Guest. The Guest may not
    /// request a relationship on a child's behalf: that is what a child
    /// default is for, and widening one here is exactly the AE33 bug.
    pub fn with_parent_use(
        mut self,
        consumer_ref: &ResourceRef,
        request: VolumeBindingRequest,
    ) -> Result<Self, GuestExecutionParentRefusal> {
        if request.consumer_ref() != consumer_ref {
            return Err(GuestExecutionParentRefusal::ParentConsumer {
                reason: "the request names a consumer other than the Guest",
            });
        }
        self.inputs
            .push(GuestExecutionParentInput::ParentUse(request));
        Ok(self)
    }

    /// Shape one child's request with this Guest's defaults (AE33).
    ///
    /// A default belongs to exactly one child, so a request from any other
    /// consumer is returned untouched rather than shaped: the Guest imposes
    /// nothing on a child it does not name.
    pub fn shape_child_request(
        &self,
        draft: &ChildBindingRequest,
    ) -> Result<ChildBindingRequest, GuestExecutionParentRefusal> {
        let Some(defaults) = self
            .child_defaults()
            .find(|defaults| defaults.child_ref() == draft.consumer_ref())
        else {
            return Ok(draft.clone());
        };
        draft
            .apply_defaults(defaults)
            .map_err(|_| GuestExecutionParentRefusal::ChildRequest)
    }
}

impl fmt::Debug for GuestExecutionParent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestExecutionParent")
            .field("facts", &self.facts)
            .field("input_count", &self.inputs.len())
            .field(
                "claims_parent_use",
                &self.inputs.iter().any(GuestExecutionParentInput::yields_binding),
            )
            .finish_non_exhaustive()
    }
}

/// One Guest fragment this classifier refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuestExecutionParentRefusal {
    /// A support entry named rights the binding kinds do not admit.
    SupportEntry,
    /// The ceiling carried more entries than the contract's bound.
    SupportCeiling,
    /// A Volume default did not name exactly one typed source.
    DefaultSource {
        /// A stable, caller-free explanation of which half was unusable.
        reason: &'static str,
    },
    /// A Volume default did not name exactly one child that requests bindings.
    DefaultChild {
        /// A stable, caller-free explanation of which half was unusable.
        reason: &'static str,
    },
    /// A parent-use request named a consumer other than the Guest.
    ParentConsumer {
        /// A stable, caller-free explanation of the mismatch.
        reason: &'static str,
    },
    /// A child request could not carry the Guest's defaults.
    ChildRequest,
}

impl fmt::Display for GuestExecutionParentRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SupportEntry => "guest-execution-parent-support-entry-invalid",
            Self::SupportCeiling => "guest-execution-parent-support-ceiling-invalid",
            Self::DefaultSource { .. } => "guest-execution-parent-default-source-invalid",
            Self::DefaultChild { .. } => "guest-execution-parent-default-child-invalid",
            Self::ParentConsumer { .. } => "guest-execution-parent-consumer-invalid",
            Self::ChildRequest => "guest-execution-parent-child-request-invalid",
        })
    }
}

impl std::error::Error for GuestExecutionParentRefusal {}

/// The canonical field naming the source one Volume default supplies.
const DEFAULT_SOURCE_FIELD: &str = "volumeRef";
/// The canonical field naming the child one Volume default shapes.
const DEFAULT_CHILD_FIELD: &str = "consumerRef";
/// The canonical field naming the view one Volume default supplies.
const DEFAULT_VIEW_FIELD: &str = "view";

/// Classify one Guest's flattened execution-parent fragment.
///
/// # Errors
///
/// Refuses a fragment the typed vocabulary cannot express rather than
/// widening it: a support entry the binding kinds do not admit, a ceiling
/// over the contract's bound, or a Volume default that does not name exactly
/// one child and one `Volume` source.
pub fn classify_guest_execution_parent(
    policy: &ExecutionPolicy,
) -> Result<GuestExecutionParent, GuestExecutionParentRefusal> {
    let support = guest_support_ceiling(policy)?;
    let mut inputs = Vec::with_capacity(1 + policy.volume_attachment_defaults().len());
    inputs.push(GuestExecutionParentInput::ChildSupportCeiling(support));
    for entry in policy.volume_attachment_defaults() {
        inputs.push(GuestExecutionParentInput::ChildRequestDefaults(
            classify_default(entry)?,
        ));
    }
    Ok(GuestExecutionParent {
        facts: GuestExecutionFacts::from_policy(policy),
        inputs,
    })
}

/// The one child target-support ceiling a Guest's attachment lists mean.
///
/// The ceiling carries at most one entry per binding kind, listing every
/// right the Guest declares for it, so two attached Devices of different
/// exclusivity are one capability a child may request rather than two
/// competing claims about it. The Device and Network references themselves
/// are deliberately not retained: a ceiling bounds admission, and the exact
/// source a child binds is named by that child's own request.
fn guest_support_ceiling(
    policy: &ExecutionPolicy,
) -> Result<ChildSupportCeiling, GuestExecutionParentRefusal> {
    let mut device_rights: Vec<RequestedRights> = Vec::new();
    for attachment in policy.device_attachments() {
        let right = if attachment.is_exclusive() {
            RequestedRights::Exclusive
        } else {
            RequestedRights::Share
        };
        if !device_rights.contains(&right) {
            device_rights.push(right);
        }
    }
    let mut support = Vec::new();
    if !device_rights.is_empty() {
        support.push(
            BindingSupportEntry::new(BindingKind::Device, device_rights)
                .map_err(|_| GuestExecutionParentRefusal::SupportEntry)?,
        );
    }
    if !policy.network_attachments().is_empty() {
        support.push(
            BindingSupportEntry::new(BindingKind::Network, vec![RequestedRights::Consume])
                .map_err(|_| GuestExecutionParentRefusal::SupportEntry)?,
        );
    }
    ChildSupportCeiling::new(support).map_err(|_| GuestExecutionParentRefusal::SupportCeiling)
}

/// One default entry, classified into the contract shape.
fn classify_default(
    entry: &CanonicalJsonObject,
) -> Result<ChildRequestDefaults, GuestExecutionParentRefusal> {
    let child = string_field(entry, DEFAULT_CHILD_FIELD).ok_or(
        GuestExecutionParentRefusal::DefaultChild {
            reason: "the entry names no child request",
        },
    )?;
    let child = ResourceRef::parse(child).map_err(|_| GuestExecutionParentRefusal::DefaultChild {
        reason: "the named child is not a resource reference",
    })?;
    let source = string_field(entry, DEFAULT_SOURCE_FIELD).ok_or(
        GuestExecutionParentRefusal::DefaultSource {
            reason: "the entry names no source",
        },
    )?;
    let source = ResourceRef::parse(source).map_err(|_| GuestExecutionParentRefusal::DefaultSource {
        reason: "the named source is not a resource reference",
    })?;
    let view = match string_field(entry, DEFAULT_VIEW_FIELD) {
        None => None,
        Some(view) => Some(BoundedToken::parse(view).map_err(|_| {
            GuestExecutionParentRefusal::DefaultSource {
                reason: "the named view is not a bounded token",
            }
        })?),
    };
    let source = DefaultedSource::new(BindingKind::Volume, source, view).map_err(|_| {
        GuestExecutionParentRefusal::DefaultSource {
            reason: "the named source is not a Volume",
        }
    })?;
    ChildRequestDefaults::new(child, source).map_err(|_| GuestExecutionParentRefusal::DefaultChild {
        reason: "the named child is not a consumer that requests bindings",
    })
}

/// Read one canonical string field, treating any other shape as absent.
fn string_field<'a>(entry: &'a CanonicalJsonObject, field: &str) -> Option<&'a str> {
    match entry.get(field) {
        Some(CanonicalJsonValue::String(value)) => Some(value.as_str()),
        _ => None,
    }
}

/// Narrow the classifier's refusals for callers that only need the boolean.
impl From<BindingContractError> for GuestExecutionParentRefusal {
    fn from(_: BindingContractError) -> Self {
        Self::ChildRequest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::{
        BindingSlot, BudgetSpec, DeviceAttachment, ExecutionDomain, NetworkAttachment,
        VolumePresentation, volume::AttachmentAccess,
    };
    use serde_json::json;

    fn guest_ref() -> ResourceRef {
        ResourceRef::parse("Guest/work-vm").expect("valid fixture ref")
    }

    fn policy_with(
        devices: Vec<DeviceAttachment>,
        networks: Vec<NetworkAttachment>,
        defaults: Vec<serde_json::Value>,
    ) -> ExecutionPolicy {
        ExecutionPolicy::new(
            ExecutionDomain::System,
            vec![ExecutionDomain::System],
            None,
            BudgetSpec::default(),
            networks,
            devices,
            defaults.into_iter().map(default_entry).collect(),
        )
        .expect("valid fixture policy")
    }

    fn default_entry(value: serde_json::Value) -> CanonicalJsonObject {
        let bytes = serde_json::to_vec(&value).expect("serializable default entry");
        CanonicalJsonObject::parse(&bytes).expect("canonical default entry")
    }

    fn parent_request(consumer: &str) -> VolumeBindingRequest {
        VolumeBindingRequest::new(
            ResourceRef::parse("Volume/system").expect("valid fixture ref"),
            ResourceRef::parse(consumer).expect("valid fixture ref"),
            BindingSlot::parse("system").expect("valid slot"),
            BoundedToken::parse("system").expect("valid view"),
            AttachmentAccess::ReadWrite,
            VolumePresentation::filesystem("/var/lib/d2b/system").expect("valid destination"),
        )
        .expect("valid fixture request")
    }

    #[test]
    fn support_attachments_classify_to_a_ceiling_that_creates_nothing() {
        let policy = policy_with(
            vec![
                DeviceAttachment::new(ResourceRef::parse("Device/kvm").expect("valid ref"), true)
                    .expect("valid attachment"),
                DeviceAttachment::new(ResourceRef::parse("Device/usb").expect("valid ref"), false)
                    .expect("valid attachment"),
            ],
            vec![NetworkAttachment::new(
                ResourceRef::parse("Network/work").expect("valid ref"),
                false,
            )
            .expect("valid attachment")],
            Vec::new(),
        );
        let parent = classify_guest_execution_parent(&policy).expect("the fragment classifies");
        let ceiling = parent
            .support_ceiling()
            .expect("the ceiling is always present");
        assert!(ceiling.admits(BindingKind::Device, RequestedRights::Exclusive));
        assert!(ceiling.admits(BindingKind::Device, RequestedRights::Share));
        assert!(ceiling.admits(BindingKind::Network, RequestedRights::Consume));
        assert!(
            !parent.claims_parent_use(),
            "a support ceiling is an admission constraint, not consumption"
        );
        assert_eq!(parent.parent_use().count(), 0);
        assert_eq!(parent.child_defaults().count(), 0);
    }

    #[test]
    fn an_empty_fragment_is_a_ceiling_that_admits_nothing() {
        let parent =
            classify_guest_execution_parent(&policy_with(Vec::new(), Vec::new(), Vec::new()))
                .expect("the fragment classifies");
        let ceiling = parent
            .support_ceiling()
            .expect("the ceiling is always present");
        assert!(ceiling.entries().is_empty());
        assert!(
            !ceiling.admits(BindingKind::Volume, RequestedRights::Observe),
            "an undeclared capability is refused rather than allowed"
        );
    }

    #[test]
    fn a_volume_default_shapes_its_child_and_grants_the_guest_nothing() {
        let policy = policy_with(
            Vec::new(),
            Vec::new(),
            vec![json!({
                "consumerRef": "Process/worker",
                "volumeRef": "Volume/data",
                "view": "controller",
            })],
        );
        let parent = classify_guest_execution_parent(&policy).expect("the fragment classifies");
        assert!(
            !parent.claims_parent_use(),
            "a child default is not the Guest's own consumption"
        );
        let draft = ChildBindingRequest::new(
            ResourceRef::parse("Process/worker").expect("valid fixture ref"),
            BindingKind::Volume,
        )
        .expect("valid fixture draft");
        let shaped = parent
            .shape_child_request(&draft)
            .expect("the named child's request is shaped");
        assert_eq!(
            shaped.source_ref().map(ResourceRef::to_canonical_string),
            Some("Volume/data".to_owned())
        );
        assert_eq!(shaped.rights(), Some(RequestedRights::Observe));
        assert_eq!(parent.parent_use().count(), 0);
    }

    #[test]
    fn a_default_refuses_to_shape_another_childs_request() {
        let policy = policy_with(
            Vec::new(),
            Vec::new(),
            vec![json!({
                "consumerRef": "Process/worker",
                "volumeRef": "Volume/data",
            })],
        );
        let parent = classify_guest_execution_parent(&policy).expect("the fragment classifies");
        let sibling = ChildBindingRequest::new(
            ResourceRef::parse("Process/sibling").expect("valid fixture ref"),
            BindingKind::Volume,
        )
        .expect("valid fixture draft");
        let untouched = parent
            .shape_child_request(&sibling)
            .expect("an unnamed child's request is left alone");
        assert_eq!(untouched.source_ref(), None);
        assert_eq!(untouched.rights(), None);
    }

    #[test]
    fn guest_consumption_is_a_parent_binding_whose_consumer_is_the_guest() {
        let parent = classify_guest_execution_parent(&policy_with(
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ))
        .expect("the fragment classifies")
        .with_parent_use(&guest_ref(), parent_request("Guest/work-vm"))
        .expect("the Guest consumes this request");
        assert!(parent.claims_parent_use());
        let request = parent
            .parent_use()
            .next()
            .expect("the request is the only parent use");
        assert_eq!(request.consumer_ref(), &guest_ref());
        assert_eq!(
            request.source_ref().to_canonical_string(),
            "Volume/system"
        );
    }

    #[test]
    fn a_guest_may_not_attach_a_childs_request_as_its_own() {
        let refused = classify_guest_execution_parent(&policy_with(
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ))
        .and_then(|parent| {
            parent.with_parent_use(&guest_ref(), parent_request("Process/worker"))
        });
        assert_eq!(
            refused,
            Err(GuestExecutionParentRefusal::ParentConsumer {
                reason: "the request names a consumer other than the Guest",
            })
        );
    }

    #[test]
    fn a_default_naming_a_guest_as_its_child_is_refused() {
        // An execution parent is never a child: a default addressed to one
        // would be a disguised parent use, which is the AE33 widening.
        let refused = classify_guest_execution_parent(&policy_with(
            Vec::new(),
            Vec::new(),
            vec![json!({
                "consumerRef": "Guest/work-vm",
                "volumeRef": "Volume/data",
            })],
        ));
        assert!(matches!(
            refused,
            Err(GuestExecutionParentRefusal::DefaultChild { .. })
        ));
    }

    #[test]
    fn a_default_naming_a_non_volume_source_is_refused() {
        let refused = classify_guest_execution_parent(&policy_with(
            Vec::new(),
            Vec::new(),
            vec![json!({
                "consumerRef": "Process/worker",
                "volumeRef": "Device/usb",
            })],
        ));
        assert!(matches!(
            refused,
            Err(GuestExecutionParentRefusal::DefaultSource { .. })
        ));
    }
}