//! The Host family's execution-parent classification (AE31-AE33).
//!
//! The old flattened `Host.spec` execution-parent fragment carried three
//! unrelated meanings in one attachment list. This module classifies them
//! instead of copying them, using U2's `ExecutionParentInput` rather than a
//! second vocabulary of its own:
//!
//! - `deviceAttachments` and `networkAttachments` are a CHILD TARGET-SUPPORT
//!   CEILING (AE31). They bound what a child of this Host may request, so they
//!   become one `ChildSupportCeiling`. A ceiling creates no binding, no
//!   reservation, and no access for anyone - the Host itself holds no
//!   relationship here.
//! - `volumeAttachmentDefaults` are CHILD REQUEST DEFAULTS (AE33). Each entry
//!   becomes one `ChildRequestDefaults` naming the single child it shapes.
//!   Applying it fills an unset field of that child's request and refuses
//!   every other consumer, so the Host gains no access of its own.
//! - `defaultDomain`, `allowedDomains`, `defaultUserRef`, and `budget` are
//!   EXECUTION-PARENT FACTS. They are preserved verbatim in
//!   [`HostExecutionFacts`] and grant no resource access.
//!
//! Nothing here produces a parent-use relationship (AE32): the Host family
//! declares no attachment as its own consumption, so an entry that would have
//! to mean that is refused rather than quietly widened into a Host binding.

use core::fmt;

use d2b_contracts_resource::v3::{
    BindingContractError, BindingKind, BindingSupportEntry, BudgetSpec, CanonicalJsonObject,
    CanonicalJsonValue, ChildBindingRequest, ChildRequestDefaults, ChildSupportCeiling,
    DefaultedSource, ExecutionDomain, ExecutionParentInput, ExecutionPolicy, RequestedRights,
    ResourceRef, VolumeBindingRequest,
    execution_policy::BoundedToken,
};

/// One classified Host input.
///
/// The parent-use arm is typed against the Volume request shape and is never
/// constructed by this family: a Host that declared its own consumption would
/// be a different contract, and [`HostExecutionParent::claims_parent_use`]
/// makes that absence checkable rather than accidental.
pub type HostExecutionParentInput = ExecutionParentInput<VolumeBindingRequest>;

/// The Host object's own non-authority facts.
///
/// These are the values an execution parent carries about itself: which
/// domains a child may run in, which User a child without an explicit identity
/// resolves to, and the aggregate budget a child is evaluated against. None of
/// them names a source, a view, a path, or a grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostExecutionFacts {
    default_domain: ExecutionDomain,
    allowed_domains: Vec<ExecutionDomain>,
    default_user_ref: Option<ResourceRef>,
    budget: BudgetSpec,
}

impl HostExecutionFacts {
    /// The domain a child Process runs in when it names none.
    pub const fn default_domain(&self) -> ExecutionDomain {
        self.default_domain
    }

    /// The domains a child of this Host may run in.
    pub fn allowed_domains(&self) -> &[ExecutionDomain] {
        &self.allowed_domains
    }

    /// The User a child without an explicit identity resolves to.
    pub const fn default_user_ref(&self) -> Option<&ResourceRef> {
        self.default_user_ref.as_ref()
    }

    /// The aggregate budget a child is evaluated against.
    pub const fn budget(&self) -> &BudgetSpec {
        &self.budget
    }
}

/// One classified Host execution-parent fragment.
#[derive(Clone, PartialEq, Eq)]
pub struct HostExecutionParent {
    facts: HostExecutionFacts,
    inputs: Vec<HostExecutionParentInput>,
}

impl fmt::Debug for HostExecutionParent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostExecutionParent")
            .field("facts", &self.facts)
            .field(
                "inputs",
                &self
                    .inputs
                    .iter()
                    .map(ExecutionParentInput::class)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl HostExecutionParent {
    /// The Host's own preserved facts.
    pub const fn facts(&self) -> &HostExecutionFacts {
        &self.facts
    }

    /// Every classified input, in a canonical order.
    ///
    /// The ceiling comes first and the child defaults follow in declaration
    /// order, so two runs over one Host spec classify identically.
    pub fn inputs(&self) -> &[HostExecutionParentInput] {
        &self.inputs
    }

    /// The child target-support ceiling this Host carries.
    ///
    /// A ceiling bounds child admission and grants nothing: there is no API
    /// here that turns it into a binding, which is exactly what AE31 requires.
    pub fn support_ceiling(&self) -> Option<&ChildSupportCeiling> {
        self.inputs
            .iter()
            .find_map(ExecutionParentInput::support_ceiling)
    }

    /// Every child's request defaults this Host supplies.
    pub fn child_defaults(&self) -> Vec<&ChildRequestDefaults> {
        self.inputs
            .iter()
            .filter_map(ExecutionParentInput::child_defaults)
            .collect()
    }

    /// Whether any classified input claims the Host's own consumption.
    ///
    /// The Host family never produces one, and this accessor makes that a
    /// property a caller can assert rather than an accident to discover.
    pub fn claims_parent_use(&self) -> bool {
        self.inputs.iter().any(|input| input.parent_use().is_some())
    }

    /// Apply this Host's defaults to one child's draft request.
    ///
    /// The one legal direction: a default belongs to exactly the child it
    /// names, so a draft for any other consumer is refused rather than
    /// widened.
    ///
    /// # Errors
    ///
    /// Returns [`BindingContractError::WrongConsumer`] when no default names
    /// this draft's consumer, and propagates
    /// [`ChildBindingRequest::apply_defaults`] for a kind disagreement.
    pub fn apply_defaults(
        &self,
        draft: &ChildBindingRequest,
    ) -> Result<ChildBindingRequest, BindingContractError> {
        let Some(defaults) = self
            .child_defaults()
            .into_iter()
            .find(|defaults| defaults.child_ref() == draft.consumer_ref())
        else {
            return Err(BindingContractError::WrongConsumer);
        };
        draft.apply_defaults(defaults)
    }
}

/// One Host fragment this classifier refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionParentRefusal {
    /// A support-ceiling entry the typed binding vocabulary does not admit.
    SupportEntry(BindingContractError),
    /// A ceiling this Host declares over the contract's bound.
    SupportCeiling(BindingContractError),
    /// A Volume default whose source is not a named `Volume` reference.
    DefaultSource {
        /// Why the entry is not a usable default.
        reason: &'static str,
    },
    /// A Volume default that names no child, or names something that is not a
    /// child's own resource type.
    DefaultChild {
        /// Why the entry is not a usable default.
        reason: &'static str,
    },
}

impl fmt::Display for ExecutionParentRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SupportEntry(reason) => write!(
                formatter,
                "the Host declares a support entry the binding vocabulary refuses: {reason}"
            ),
            Self::SupportCeiling(reason) => {
                write!(formatter, "the Host child support ceiling is refused: {reason}")
            }
            Self::DefaultSource { reason } => {
                write!(formatter, "a Host Volume default names no usable source: {reason}")
            }
            Self::DefaultChild { reason } => {
                write!(formatter, "a Host Volume default names no usable child: {reason}")
            }
        }
    }
}

impl std::error::Error for ExecutionParentRefusal {}

/// The canonical field naming the source one Volume default supplies.
const DEFAULT_SOURCE_FIELD: &str = "volumeRef";
/// The canonical field naming the child one Volume default shapes.
const DEFAULT_CHILD_FIELD: &str = "consumerRef";
/// The canonical field naming the view one Volume default supplies.
const DEFAULT_VIEW_FIELD: &str = "view";

/// Classify one Host's flattened execution-parent fragment.
///
/// # Errors
///
/// Refuses a fragment the typed vocabulary cannot express rather than widening
/// it: a support entry the binding kinds do not admit, a ceiling over the
/// contract's bound, or a Volume default that does not name exactly one child
/// and one `Volume` source.
pub fn classify_host_execution_parent(
    policy: &ExecutionPolicy,
) -> Result<HostExecutionParent, ExecutionParentRefusal> {
    // The ceiling carries at most one entry per binding kind, listing every
    // right the Host declares for it, so two attached Devices of different
    // exclusivity are one capability the child may request rather than two
    // competing claims about it.
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
                .map_err(ExecutionParentRefusal::SupportEntry)?,
        );
    }
    if !policy.network_attachments().is_empty() {
        support.push(
            BindingSupportEntry::new(BindingKind::Network, vec![RequestedRights::Consume])
                .map_err(ExecutionParentRefusal::SupportEntry)?,
        );
    }
    // An empty ceiling still travels: a missing attachment list is a ceiling
    // that admits nothing, which is what makes an undeclared child request a
    // refusal at admission rather than a default allowance.
    let ceiling = ChildSupportCeiling::new(support)
        .map_err(ExecutionParentRefusal::SupportCeiling)?;

    let mut inputs = Vec::with_capacity(1 + policy.volume_attachment_defaults().len());
    inputs.push(HostExecutionParentInput::ChildSupportCeiling(ceiling));
    for entry in policy.volume_attachment_defaults() {
        inputs.push(HostExecutionParentInput::ChildRequestDefaults(
            classify_default(entry)?,
        ));
    }
    Ok(HostExecutionParent {
        facts: HostExecutionFacts {
            default_domain: policy.default_domain(),
            allowed_domains: policy.allowed_domains().to_vec(),
            default_user_ref: policy.default_user_ref().cloned(),
            budget: policy.budget().clone(),
        },
        inputs,
    })
}

/// One default entry, classified into the contract shape.
fn classify_default(
    entry: &CanonicalJsonObject,
) -> Result<ChildRequestDefaults, ExecutionParentRefusal> {
    let child = string_field(entry, DEFAULT_CHILD_FIELD).ok_or(
        ExecutionParentRefusal::DefaultChild {
            reason: "the entry names no child request",
        },
    )?;
    let child = ResourceRef::parse(child).map_err(|_| ExecutionParentRefusal::DefaultChild {
        reason: "the named child is not a resource reference",
    })?;
    let source = string_field(entry, DEFAULT_SOURCE_FIELD).ok_or(
        ExecutionParentRefusal::DefaultSource {
            reason: "the entry names no source",
        },
    )?;
    let source = ResourceRef::parse(source).map_err(|_| ExecutionParentRefusal::DefaultSource {
        reason: "the named source is not a resource reference",
    })?;
    let view = match string_field(entry, DEFAULT_VIEW_FIELD) {
        None => None,
        Some(view) => Some(BoundedToken::parse(view).map_err(|_| {
            ExecutionParentRefusal::DefaultSource {
                reason: "the named view is not a bounded token",
            }
        })?),
    };
    let source = DefaultedSource::new(BindingKind::Volume, source, view).map_err(|_| {
        ExecutionParentRefusal::DefaultSource {
            reason: "the named source is not a Volume",
        }
    })?;
    ChildRequestDefaults::new(child, source).map_err(|_| ExecutionParentRefusal::DefaultChild {
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

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::{
        DeviceAttachment, ExecutionParentInputClass, NetworkAttachment,
    };
    use serde_json::json;

    fn object(value: serde_json::Value) -> CanonicalJsonObject {
        let bytes = serde_json::to_vec(&value).expect("serializable object");
        CanonicalJsonObject::parse(&bytes).expect("canonical object")
    }

    fn host_policy() -> ExecutionPolicy {
        ExecutionPolicy::new(
            ExecutionDomain::System,
            vec![ExecutionDomain::System],
            None,
            BudgetSpec::default(),
            vec![
                NetworkAttachment::new(ResourceRef::parse("Network/work").expect("network"), true)
                    .expect("network attachment"),
            ],
            vec![
                DeviceAttachment::new(ResourceRef::parse("Device/gpu").expect("device"), true)
                    .expect("exclusive device attachment"),
                DeviceAttachment::new(ResourceRef::parse("Device/tpm").expect("device"), false)
                    .expect("shared device attachment"),
            ],
            Vec::new(),
        )
        .expect("host policy")
    }

    /// The same Host plus one well-formed Volume default and one entry that
    /// names no child.
    fn host_policy_with_defaults(entries: Vec<serde_json::Value>) -> ExecutionPolicy {
        ExecutionPolicy::new(
            ExecutionDomain::System,
            vec![ExecutionDomain::System],
            None,
            BudgetSpec::default(),
            Vec::new(),
            Vec::new(),
            entries.into_iter().map(object).collect(),
        )
        .expect("host policy with defaults")
    }

    fn consumer(consumer_ref: &str) -> ChildBindingRequest {
        ChildBindingRequest::new(
            ResourceRef::parse(consumer_ref).expect("consumer"),
            BindingKind::Volume,
        )
        .expect("child draft")
    }

    #[test]
    fn device_and_network_attachments_become_one_support_ceiling() {
        let parent = classify_host_execution_parent(&host_policy()).expect("classified");
        let ceiling = parent.support_ceiling().expect("child support ceiling");
        // The ceiling bounds what a child may request and grants nothing: the
        // Host itself claims no consumption, and no binding is produced.
        assert!(!parent.claims_parent_use());
        assert!(ceiling.admits(BindingKind::Device, RequestedRights::Exclusive));
        assert!(ceiling.admits(BindingKind::Device, RequestedRights::Share));
        assert!(ceiling.admits(BindingKind::Network, RequestedRights::Consume));
        // A kind or a right the Host never declared is outside the ceiling.
        assert!(!ceiling.admits(BindingKind::Credential, RequestedRights::Consume));
        assert!(!ceiling.admits(BindingKind::Device, RequestedRights::Observe));
        let classes = parent
            .inputs()
            .iter()
            .map(ExecutionParentInput::class)
            .collect::<Vec<_>>();
        assert_eq!(classes, vec![ExecutionParentInputClass::ChildSupportCeiling]);
    }

    #[test]
    fn volume_defaults_shape_only_the_child_they_name() {
        let policy = host_policy_with_defaults(vec![json!({
            "consumerRef": "Process/virtiofsd",
            "volumeRef": "Volume/media-library",
            "view": "ro"
        })]);
        let parent = classify_host_execution_parent(&policy).expect("classified");
        let defaults = parent.child_defaults();
        assert_eq!(defaults.len(), 1);
        assert_eq!(
            defaults[0].child_ref().to_canonical_string(),
            "Process/virtiofsd"
        );
        assert_eq!(
            defaults[0]
                .source()
                .source_ref()
                .to_canonical_string(),
            "Volume/media-library"
        );
        assert_eq!(
            defaults[0].source().view().map(BoundedToken::as_str),
            Some("ro")
        );

        // The named child's own unset fields are filled.
        let filled = parent
            .apply_defaults(&consumer("Process/virtiofsd"))
            .expect("defaults apply");
        assert_eq!(
            filled
                .source_ref()
                .map(ResourceRef::to_canonical_string),
            Some("Volume/media-library".to_owned())
        );
        // Another child is refused rather than widened onto this default.
        assert_eq!(
            parent
                .apply_defaults(&consumer("Process/other"))
                .err()
                .expect("another child is refused"),
            BindingContractError::WrongConsumer
        );
    }

    #[test]
    fn a_default_naming_no_child_is_refused_rather_than_expanded() {
        let error = classify_host_execution_parent(&host_policy_with_defaults(vec![
            json!({
                "consumerRef": "Process/virtiofsd",
                "volumeRef": "Volume/media-library"
            }),
            json!({ "volumeRef": "Volume/media-library" }),
        ]))
            .err()
            .expect("the nameless default is refused");
        assert_eq!(
            error,
            ExecutionParentRefusal::DefaultChild {
                reason: "the entry names no child request",
            }
        );
    }

    #[test]
    fn domains_user_and_budget_stay_facts_and_grant_nothing() {
        let policy = ExecutionPolicy::new(
            ExecutionDomain::User,
            vec![ExecutionDomain::User],
            Some(ResourceRef::parse("User/alice").expect("user")),
            BudgetSpec::default(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("user-only policy");
        let parent = classify_host_execution_parent(&policy).expect("classified");
        let facts = parent.facts();
        assert_eq!(facts.default_domain(), ExecutionDomain::User);
        assert_eq!(facts.allowed_domains(), [ExecutionDomain::User]);
        assert_eq!(
            facts.default_user_ref().map(ResourceRef::to_canonical_string),
            Some("User/alice".to_owned())
        );
        // Facts alone produce no classified input beyond the empty ceiling:
        // no binding, no child default, and no parent use.
        assert!(parent.child_defaults().is_empty());
        assert!(!parent.claims_parent_use());
        assert!(
            parent
                .support_ceiling()
                .expect("ceiling")
                .entries()
                .is_empty()
        );
    }
}
