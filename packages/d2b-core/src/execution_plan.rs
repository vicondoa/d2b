//! The resolved private execution plan (U10, KTD8).
//!
//! KTD8 moves the privileged boundary. An invocation names what it wants; the
//! broker answers with the exact private values that invocation runs against.
//! This module is the broker's answer: the sources, destinations, views,
//! identities, and dependency versions one admitted effect is resolved to,
//! derived from the U2 binding contracts and the U6 accepted graph and fenced
//! by U1's [`FreshnessTuple`].
//!
//! # What makes these values private
//!
//! Not a visibility modifier. Three facts make them private:
//!
//! 1. **No serialized type has a field that can carry one.** The invocation
//!    carrier - `AdmittedEffectInvocation` in the broker wire crate - names
//!    an `Operation`, a subject, typed relationship selections, typed
//!    non-authority parameters, expected dependency versions, and an
//!    idempotency key. It has no field for a host path, a numerical
//!    credential, a mount policy, or a launch command line, so no such value
//!    can be authored by a caller: not ignored, not sanitized, absent.
//! 2. **Admission derives every one of them.** Nothing below reads a
//!    caller-supplied value. Each is looked up in the broker's own
//!    [`PrivateExecutionTable`], which the broker builds from its accepted
//!    graph and its trusted implementation contract, and a dependency the
//!    broker holds no value for is refused rather than approximated.
//! 3. **Nothing here serializes.** This module derives no `Serialize` and no
//!    `Deserialize`: a plan is constructed inside the privileged process,
//!    handed to one declared implementation, and dropped. It is derived
//!    execution data, never an authored resource, and it is not a second
//!    policy hierarchy - it decides nothing, it records what
//!    [`GraphAuthority`] and [`admit_binding_request`] already decided.
//!
//! # The authority-bearing parameter screen
//!
//! A closed payload contract alone is not enough. A provider that declared a
//! field named `hostPath`, `argv`, `env`, or `uid` in its own schema would
//! reopen exactly the boundary this unit closes, so [`admit_parameters`]
//! screens the declared schema and the supplied object against
//! [`AUTHORITY_PARAMETER_NAMES`]: an operation whose contract declares such a
//! field is refused by name, and a supplied object carrying one is refused
//! even when a schema declared it. The screen is data, not a naming
//! convention, and it is closed - there is no way to spell a new
//! authority-bearing field past it (R50, R52, AE7).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use d2b_contracts_resource::v3::{
    AdmissionDecision, AdmissionStage, AuthoritySubject, BindingAdmission, BindingAuthorization,
    BindingKey, BindingKind, BindingRealizationFacet, BindingSlotAddress, CallableOperation,
    CanonicalJsonObject, FdContract, FreshnessTuple, OPERATION_RESOURCE_TYPE, RefusalReason,
    RequestedRights, ResourceRef, ResourceUid, StoreIncarnation, ZoneId, canonical_digest,
    execution_policy::BoundedToken, operation::OperationImplementation,
};

use crate::resource_authority::{
    AcceptedGraph, BindingAdmissionRequest, GraphAuthority, GraphMutation, MutationKind,
    MutationSubjectEvidence, TransportIdentity,
};

/// The parameter names no admitted effect may carry, at any depth.
///
/// The set is closed and it names the four things KTD8 removes from the
/// privileged boundary: an authority-bearing host path, a numerical host
/// credential, an arbitrary mount policy, and a free-form launch command
/// line. The screen applies to the *declared* schema as well as to the
/// supplied object, so a provider cannot reintroduce the boundary by
/// declaring one of these names - which is what makes it a bound rather than
/// a convention (R50, R52).
pub const AUTHORITY_PARAMETER_NAMES: [&str; 35] = [
    "argv",
    "args",
    "binary",
    "binaryPath",
    "capabilities",
    "capabilityClasses",
    "command",
    "commandLine",
    "device",
    "devicePath",
    "environment",
    "environmentClass",
    "env",
    "gid",
    "groups",
    "hostPath",
    "launchArgs",
    "mount",
    "mountPolicy",
    "mounts",
    "namespace",
    "namespaceClasses",
    "namespaces",
    "noNewPrivileges",
    "oomScoreAdj",
    "path",
    "readOnlyRoot",
    "sandboxPlan",
    "seccomp",
    "seccompClass",
    "seccompPolicy",
    "startRoot",
    "uid",
    "umask",
    "userNamespace",
];

/// The domain tag framing the digest of one admitted parameter object.
pub const PARAMETER_DIGEST_DOMAIN_TAG: &str = "d2b:v3:effect-parameters";

/// Maximum top-level fields one admitted effect's parameters may carry.
pub const MAX_ADMITTED_PARAMETERS: usize = 64;
/// Maximum relationship legs one resolved plan may run on.
pub const MAX_PLAN_LEGS: usize = 8;
/// Maximum named views one resolved plan may address.
pub const MAX_PLAN_VIEWS: usize = 16;
/// Maximum presentation destinations one resolved plan may apply.
pub const MAX_PLAN_DESTINATIONS: usize = 8;
/// Maximum arguments one trusted executable template contributes.
pub const MAX_PLAN_ARGV: usize = 64;
/// Maximum environment entries one trusted executable template contributes.
pub const MAX_PLAN_ENVIRONMENT: usize = 64;
/// Maximum supplementary groups one resolved identity carries.
pub const MAX_PLAN_SUPPLEMENTARY_GROUPS: usize = 32;
/// Maximum private-path bytes one resolved value may carry.
pub const MAX_PLAN_PATH_BYTES: usize = 4096;

// ---------------------------------------------------------------------------
// Typed non-authority parameters
// ---------------------------------------------------------------------------

/// The parameters one invocation supplied, after they were validated against
/// the operation's declared contract and screened for authority.
///
/// Construction is closed: the only way to hold one is [`admit_parameters`],
/// so a handler that receives this type knows the object it reads was refused
/// if it carried an authority-bearing field.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmittedParameters {
    values: CanonicalJsonObject,
}

impl core::fmt::Debug for AdmittedParameters {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "AdmittedParameters(<{} fields>)",
            self.values.len()
        )
    }
}

impl AdmittedParameters {
    /// Borrow the admitted object.
    pub const fn values(&self) -> &CanonicalJsonObject {
        &self.values
    }

    /// The canonical digest of the admitted object.
    ///
    /// It is the parameter half of an idempotency key's identity: one key
    /// presented twice with two different parameter objects is a conflict,
    /// not a replay.
    pub fn digest(&self) -> String {
        parameters_digest(&self.values)
    }
}

/// Why one parameter object was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterRefusal {
    /// The object carried more top-level fields than the boundary admits.
    TooManyFields {
        /// The observed field count.
        count: usize,
    },
    /// The object carried a field the operation's contract does not declare.
    UndeclaredField {
        /// The undeclared field name.
        name: String,
    },
    /// The object omitted a field the operation's contract declares required.
    MissingRequiredField {
        /// The missing field name.
        name: String,
    },
    /// A field whose name is authority-bearing reached the boundary, from the
    /// declared schema or from the supplied object (AE7, R50).
    AuthorityBearingField {
        /// The offending field name.
        name: String,
    },
    /// A write-only (secret) field was supplied a value.
    WriteOnlyField {
        /// The offending field name.
        name: String,
    },
    /// The object's canonical bytes exceeded the operation's payload ceiling.
    OverPayloadBound {
        /// The observed byte length.
        bytes: usize,
    },
}

impl ParameterRefusal {
    /// The closed code one refusal is reported under.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::TooManyFields { .. } => "effect-parameters-too-many",
            Self::UndeclaredField { .. } => "effect-parameters-undeclared",
            Self::MissingRequiredField { .. } => "effect-parameters-missing",
            Self::AuthorityBearingField { .. } => "effect-parameters-authority-bearing",
            Self::WriteOnlyField { .. } => "effect-parameters-write-only",
            Self::OverPayloadBound { .. } => "effect-parameters-over-bound",
        }
    }

    /// The typed reason one refusal is reported under.
    pub const fn reason(&self) -> RefusalReason {
        match self {
            // A payload the operation's own contract does not admit is a
            // malformed invocation, not an authority failure.
            Self::TooManyFields { .. }
            | Self::UndeclaredField { .. }
            | Self::MissingRequiredField { .. }
            | Self::OverPayloadBound { .. } => RefusalReason::ConflictingDeclaration,
            // A write-only field carrying a value is a secret the caller had
            // no business supplying.
            Self::WriteOnlyField { .. } => RefusalReason::SourcePolicyRefused,
            // The screen's own refusal: the caller reached for a value the
            // boundary no longer accepts from anyone.
            Self::AuthorityBearingField { .. } => RefusalReason::UntrustedImplementation,
        }
    }
}

impl core::fmt::Display for ParameterRefusal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooManyFields { count } => {
                write!(formatter, "{count} parameters exceed the admitted ceiling")
            }
            Self::UndeclaredField { name } => {
                write!(
                    formatter,
                    "parameter `{name}` is not declared by the operation"
                )
            }
            Self::MissingRequiredField { name } => {
                write!(formatter, "required parameter `{name}` was not supplied")
            }
            Self::AuthorityBearingField { name } => write!(
                formatter,
                "parameter `{name}` is an authority-bearing value no invocation may carry"
            ),
            Self::WriteOnlyField { name } => {
                write!(formatter, "parameter `{name}` is write-only and takes no value")
            }
            Self::OverPayloadBound { bytes } => {
                write!(formatter, "{bytes} parameter bytes exceed the declared ceiling")
            }
        }
    }
}

impl std::error::Error for ParameterRefusal {}

/// The canonical digest of one supplied parameter object.
///
/// The broker computes it before admission so a retry is classified against
/// the recorded outcome without re-deriving an admitted value it may never
/// be granted.
pub fn parameters_digest(supplied: &CanonicalJsonObject) -> String {
    canonical_digest(PARAMETER_DIGEST_DOMAIN_TAG, &supplied.to_canonical_bytes())
}

/// Whether one declared or supplied parameter name is authority-bearing.
///
/// A name is authority-bearing when its first dotted or colon-separated
/// segment is one of [`AUTHORITY_PARAMETER_NAMES`]. Matching the first
/// segment is what makes the screen robust against a field renamed to dodge
/// it: `hostPath` is refused, and so is `hostPathOverride`.
pub fn is_authority_parameter(name: &str) -> bool {
    let head = name.split(['.', ':']).next().unwrap_or(name);
    AUTHORITY_PARAMETER_NAMES
        .iter()
        .any(|blocked| head.eq_ignore_ascii_case(blocked))
}

/// Validate one supplied parameter object against the operation's declared
/// contract, after screening both for authority-bearing names.
///
/// Validation is closed in both directions: a field the contract does not
/// declare is refused rather than ignored, and a declared field that is
/// write-only takes no value. The authority screen runs over the declared
/// schema as well as over the supplied object, so an operation whose own
/// contract declares a host path, a command line, a numerical credential, or
/// a mount policy is refused by name at admission rather than becoming a
/// second route to the host.
///
/// # Errors
///
/// Refuses with [`ParameterRefusal`] naming the first field that failed: the
/// declared schema is screened first, then the supplied object in canonical
/// field order, then the contract's required fields, then the payload bound.
pub fn admit_parameters(
    callable: &CallableOperation,
    supplied: &CanonicalJsonObject,
) -> Result<AdmittedParameters, ParameterRefusal> {
    let schema = callable.payload_schema();
    for name in schema.property_names() {
        if is_authority_parameter(name) {
            return Err(ParameterRefusal::AuthorityBearingField {
                name: name.to_owned(),
            });
        }
    }
    if supplied.len() > MAX_ADMITTED_PARAMETERS {
        return Err(ParameterRefusal::TooManyFields {
            count: supplied.len(),
        });
    }
    for name in supplied.keys() {
        if is_authority_parameter(name) {
            return Err(ParameterRefusal::AuthorityBearingField {
                name: name.to_owned(),
            });
        }
        if !schema.declares(name) {
            return Err(ParameterRefusal::UndeclaredField {
                name: name.to_owned(),
            });
        }
        if schema.is_write_only(name) {
            return Err(ParameterRefusal::WriteOnlyField {
                name: name.to_owned(),
            });
        }
    }
    for name in schema.required_names() {
        if !supplied.keys().any(|supplied| supplied == name) {
            return Err(ParameterRefusal::MissingRequiredField {
                name: name.to_owned(),
            });
        }
    }
    let bytes = supplied.to_canonical_bytes();
    if bytes.len() > callable.bounds().max_payload_bytes() as usize {
        return Err(ParameterRefusal::OverPayloadBound { bytes: bytes.len() });
    }
    Ok(AdmittedParameters {
        values: supplied.clone(),
    })
}

// ---------------------------------------------------------------------------
// The broker's private execution values
// ---------------------------------------------------------------------------

/// One private host path the broker resolved for an admitted effect.
///
/// The value is deliberately opaque in `Debug` and `Display` and has no
/// `Serialize`: a log line, an audit record, or a serialized frame cannot
/// carry a host path out of this module. The only way to read it is
/// [`PrivatePath::as_path`], which the broker's own handlers call with the
/// plan they were handed.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PrivatePath(String);

impl PrivatePath {
    /// Construct one private path after checking its bounds.
    ///
    /// # Errors
    ///
    /// Returns [`PlanValueError::PathTooLong`] past
    /// [`MAX_PLAN_PATH_BYTES`], [`PlanValueError::PathNotCanonical`] for a
    /// path carrying a NUL byte, and [`PlanValueError::RelativePath`] for a
    /// path that is not absolute: every private execution path the broker
    /// resolves is a host path, never a consumer-supplied relative one.
    pub fn parse(value: impl Into<String>) -> Result<Self, PlanValueError> {
        let value = value.into();
        if value.len() > MAX_PLAN_PATH_BYTES {
            return Err(PlanValueError::PathTooLong);
        }
        if value.contains('\0') {
            return Err(PlanValueError::PathNotCanonical);
        }
        if !value.starts_with('/') {
            return Err(PlanValueError::RelativePath);
        }
        Ok(Self(value))
    }

    /// Borrow the resolved path.
    pub fn as_path(&self) -> &Path {
        Path::new(&self.0)
    }

    /// Consume the resolved path into the broker's own owned form.
    pub fn into_path_buf(self) -> PathBuf {
        PathBuf::from(self.0)
    }
}

impl core::fmt::Debug for PrivatePath {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PrivatePath(<redacted>)")
    }
}

impl core::fmt::Display for PrivatePath {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("PrivatePath(<redacted>)")
    }
}

/// Why one private execution value was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanValueError {
    /// A private path exceeded its bound.
    PathTooLong,
    /// A private path carried a NUL byte.
    PathNotCanonical,
    /// A private path was not absolute.
    RelativePath,
    /// A value list was empty where the contract requires at least one entry.
    Empty,
    /// A value list exceeded its bound.
    OverBound,
    /// An argument or environment entry carried a NUL byte the exec path
    /// cannot round-trip.
    EntryWithNul,
}

impl core::fmt::Display for PlanValueError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::PathTooLong => "private path is over its bound",
            Self::PathNotCanonical => "private path carries a NUL byte",
            Self::RelativePath => "private execution path must be absolute",
            Self::Empty => "private execution value list is empty",
            Self::OverBound => "private execution value list is over its bound",
            Self::EntryWithNul => "private execution entry carries a NUL byte",
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for PlanValueError {}

/// The class of host object one admitted source's backing resolves to.
///
/// The class is closed so a consumer cannot select a presentation the source
/// never declared: a `Device` source resolves to a device node or a mediated
/// attachment, never to a pathname it chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateBacking {
    /// A filesystem tree, file, or named view.
    Filesystem,
    /// A character device node.
    CharacterDevice,
    /// A block device node.
    BlockDevice,
    /// A socket the broker opened and verified for the exact endpoint.
    Socket,
    /// The provider-owned shared fabric realization for a network source.
    SharedFabric,
    /// Material delivered inside an admitted delivery session.
    CredentialDelivery,
}

impl PrivateBacking {
    /// The presentation facet a backing of this class realizes.
    pub const fn facet(self) -> BindingRealizationFacet {
        match self {
            Self::Filesystem => BindingRealizationFacet::FilesystemPresentation,
            Self::CharacterDevice | Self::BlockDevice => BindingRealizationFacet::ConsumerDeviceSlot,
            Self::Socket => BindingRealizationFacet::EndpointDescriptor,
            Self::SharedFabric => BindingRealizationFacet::SharedFabric,
            Self::CredentialDelivery => BindingRealizationFacet::CredentialDelivery,
        }
    }
}

/// One named view of a source, with the right its own row admits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedView {
    name: BoundedToken,
    rights: RequestedRights,
    path: PrivatePath,
}

impl PlannedView {
    /// Construct one named view.
    pub const fn new(name: BoundedToken, rights: RequestedRights, path: PrivatePath) -> Self {
        Self { name, rights, path }
    }

    /// The view's declared name.
    pub const fn name(&self) -> &BoundedToken {
        &self.name
    }

    /// The right this view admits.
    pub const fn rights(&self) -> RequestedRights {
        self.rights
    }

    /// The private path the broker resolved for this view.
    pub const fn path(&self) -> &PrivatePath {
        &self.path
    }
}

/// One exact source the broker resolved for an admitted effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSource {
    reference: ResourceRef,
    uid: ResourceUid,
    kind: BindingKind,
    freshness: FreshnessTuple,
    backing: PrivateBacking,
    backing_path: PrivatePath,
    views: Vec<PlannedView>,
}

impl PlannedSource {
    /// Construct one resolved source.
    ///
    /// # Errors
    ///
    /// Returns [`PlanValueError::OverBound`] when the named-view list exceeds
    /// [`MAX_PLAN_VIEWS`].
    pub fn new(
        reference: ResourceRef,
        uid: ResourceUid,
        kind: BindingKind,
        freshness: FreshnessTuple,
        backing: PrivateBacking,
        backing_path: PrivatePath,
        views: Vec<PlannedView>,
    ) -> Result<Self, PlanValueError> {
        if views.len() > MAX_PLAN_VIEWS {
            return Err(PlanValueError::OverBound);
        }
        Ok(Self {
            reference,
            uid,
            kind,
            freshness,
            backing,
            backing_path,
            views,
        })
    }

    /// The exact source reference.
    pub const fn reference(&self) -> &ResourceRef {
        &self.reference
    }

    /// The source's store-assigned identity.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// The relationship family this source backs.
    pub const fn kind(&self) -> BindingKind {
        self.kind
    }

    /// The committed row state the broker observed for this source.
    pub const fn freshness(&self) -> &FreshnessTuple {
        &self.freshness
    }

    /// The host object class the broker resolved.
    pub const fn backing(&self) -> PrivateBacking {
        self.backing
    }

    /// The private path the broker resolved for this source.
    pub const fn backing_path(&self) -> &PrivatePath {
        &self.backing_path
    }

    /// The named views this source publishes.
    pub fn views(&self) -> &[PlannedView] {
        &self.views
    }
}

/// One private presentation destination inside the consumer's own tree.
///
/// The destination is resolved here, not supplied: a caller names the
/// relationship and the presentation facet it depends on, and the broker
/// decides where the exact source lands. That is what removes arbitrary mount
/// policy from the boundary (R37, AE7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedDestination {
    leg: BindingSlotAddress,
    presentation: BindingRealizationFacet,
    path: PrivatePath,
    read_only: bool,
}

impl PlannedDestination {
    /// Construct one resolved destination.
    pub const fn new(
        leg: BindingSlotAddress,
        presentation: BindingRealizationFacet,
        path: PrivatePath,
        read_only: bool,
    ) -> Self {
        Self {
            leg,
            presentation,
            path,
            read_only,
        }
    }

    /// The consumer slot this destination realizes.
    pub const fn leg(&self) -> &BindingSlotAddress {
        &self.leg
    }

    /// The presentation facet this destination applies.
    pub const fn presentation(&self) -> BindingRealizationFacet {
        self.presentation
    }

    /// The private mount point the broker resolved.
    pub const fn path(&self) -> &PrivatePath {
        &self.path
    }

    /// Whether the presentation is read-only.
    pub const fn read_only(&self) -> bool {
        self.read_only
    }
}

/// The private user namespace one admitted identity runs inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannedUserNamespace {
    /// The in-namespace uid the effect presents.
    pub inner_uid: u32,
    /// The in-namespace gid the effect presents.
    pub inner_gid: u32,
    /// Whether the effect presents in-namespace uid 0.
    pub fake_root: bool,
}

/// One identity an admitted effect may run as.
///
/// The identity is the graph's, never the caller's: the subject's own
/// admitted `User` row resolves to these numbers inside the broker, and no
/// invocation names a uid or gid anywhere (R26, R37).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedIdentity {
    subject: ResourceRef,
    uid: u32,
    gid: u32,
    supplementary_groups: Vec<u32>,
    user_namespace: Option<PlannedUserNamespace>,
}

impl PlannedIdentity {
    /// Construct one resolved identity.
    ///
    /// # Errors
    ///
    /// Returns [`PlanValueError::OverBound`] when the supplementary-group
    /// list exceeds [`MAX_PLAN_SUPPLEMENTARY_GROUPS`]. Whether a
    /// supplementary group may equal the primary gid is the broker's own
    /// launch preflight's question, not this type's: it records what the
    /// graph admitted and does not second-guess it.
    pub fn new(
        subject: ResourceRef,
        uid: u32,
        gid: u32,
        supplementary_groups: Vec<u32>,
        user_namespace: Option<PlannedUserNamespace>,
    ) -> Result<Self, PlanValueError> {
        if supplementary_groups.len() > MAX_PLAN_SUPPLEMENTARY_GROUPS {
            return Err(PlanValueError::OverBound);
        }
        Ok(Self {
            subject,
            uid,
            gid,
            supplementary_groups,
            user_namespace,
        })
    }

    /// The subject this identity belongs to.
    pub const fn subject(&self) -> &ResourceRef {
        &self.subject
    }

    /// The resolved host uid.
    pub const fn uid(&self) -> u32 {
        self.uid
    }

    /// The resolved host gid.
    pub const fn gid(&self) -> u32 {
        self.gid
    }

    /// The resolved supplementary groups.
    pub fn supplementary_groups(&self) -> &[u32] {
        &self.supplementary_groups
    }

    /// The user namespace the effect runs in, when it declared one.
    pub const fn user_namespace(&self) -> Option<&PlannedUserNamespace> {
        self.user_namespace.as_ref()
    }
}

/// The trusted executable template one admitted effect runs.
///
/// Every field here comes from the verified implementation contract. The
/// caller supplies none of them: there is no field on the invocation carrier
/// for an argument, an environment entry, or a program, and the plan carries
/// what the declaration resolved to. This is the structural answer to
/// "authorization inferred from argv, environment, or naming conventions"
/// (R50): the posture an effect runs under is not on the wire at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedExecutable {
    implementation: OperationImplementation,
    template: BoundedToken,
    program: PrivatePath,
    argv: Vec<String>,
    environment: Vec<String>,
}

impl PlannedExecutable {
    /// Construct one trusted executable resolution.
    ///
    /// # Errors
    ///
    /// Returns [`PlanValueError::Empty`] when the template contributes no
    /// arguments or no environment entries, [`PlanValueError::OverBound`]
    /// when either list is past its bound, and
    /// [`PlanValueError::EntryWithNul`] when an entry carries a NUL byte the
    /// exec path cannot round-trip.
    pub fn new(
        implementation: OperationImplementation,
        template: BoundedToken,
        program: PrivatePath,
        argv: Vec<String>,
        environment: Vec<String>,
    ) -> Result<Self, PlanValueError> {
        if argv.is_empty() || environment.is_empty() {
            return Err(PlanValueError::Empty);
        }
        if argv.len() > MAX_PLAN_ARGV || environment.len() > MAX_PLAN_ENVIRONMENT {
            return Err(PlanValueError::OverBound);
        }
        if argv.iter().chain(&environment).any(|entry| entry.contains('\0')) {
            return Err(PlanValueError::EntryWithNul);
        }
        Ok(Self {
            implementation,
            template,
            program,
            argv,
            environment,
        })
    }

    /// The declared implementation this executable realizes.
    pub const fn implementation(&self) -> &OperationImplementation {
        &self.implementation
    }

    /// The declared template identity.
    pub const fn template(&self) -> &BoundedToken {
        &self.template
    }

    /// The private program path the deployment resolved.
    pub const fn program(&self) -> &PrivatePath {
        &self.program
    }

    /// The declared argument vector, template-resolved.
    pub fn argv(&self) -> &[String] {
        &self.argv
    }

    /// The declared environment, template-resolved.
    pub fn environment(&self) -> &[String] {
        &self.environment
    }
}

/// The broker's private execution values, keyed by exact identity.
///
/// This is what "the broker resolves the private execution values from its
/// accepted graph and trusted implementation contract" means concretely. A
/// dependency with no entry is refused, never approximated: absence of a
/// private value is not a default path, a default uid, or a default program.
#[derive(Debug, Clone, Default)]
pub struct PrivateExecutionTable {
    sources: BTreeMap<ResourceUid, PlannedSource>,
    observed: BTreeMap<ResourceUid, FreshnessTuple>,
    identities: BTreeMap<ResourceRef, PlannedIdentity>,
    destinations: BTreeMap<BindingSlotAddress, Vec<PlannedDestination>>,
    executables: BTreeMap<ResourceRef, PlannedExecutable>,
}

impl PrivateExecutionTable {
    /// An empty table: every lookup refuses.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Record one resolved source and the committed state it was resolved at.
    #[must_use]
    pub fn with_source(mut self, source: PlannedSource) -> Self {
        self.observed
            .insert(source.uid().clone(), source.freshness().clone());
        self.sources.insert(source.uid().clone(), source);
        self
    }

    /// Record one committed row's observed state, for a row an admitted
    /// effect depends on without being its source: the consumer, a provider
    /// assignment, or an execution policy.
    #[must_use]
    pub fn with_observed(mut self, freshness: FreshnessTuple) -> Self {
        self.observed
            .insert(freshness.resource_uid().clone(), freshness);
        self
    }

    /// Record one resolved identity.
    #[must_use]
    pub fn with_identity(mut self, identity: PlannedIdentity) -> Self {
        self.identities
            .insert(identity.subject().clone(), identity);
        self
    }

    /// Record one resolved destination for a consumer slot.
    #[must_use]
    pub fn with_destination(mut self, destination: PlannedDestination) -> Self {
        self.destinations
            .entry(destination.leg().clone())
            .or_default()
            .push(destination);
        self
    }

    /// Record one trusted executable resolution, keyed by the `Operation` that
    /// declares it.
    #[must_use]
    pub fn with_executable(mut self, operation: ResourceRef, executable: PlannedExecutable) -> Self {
        self.executables.insert(operation, executable);
        self
    }

    /// The source resolved for one store-assigned identity.
    pub fn source(&self, uid: &ResourceUid) -> Option<&PlannedSource> {
        self.sources.get(uid)
    }

    /// The committed state the broker observed for one row.
    pub fn observed(&self, uid: &ResourceUid) -> Option<&FreshnessTuple> {
        self.observed.get(uid)
    }

    /// The identity resolved for one subject.
    pub fn identity(&self, subject: &ResourceRef) -> Option<&PlannedIdentity> {
        self.identities.get(subject)
    }

    /// The destinations resolved for one consumer slot.
    pub fn destinations(&self, leg: &BindingSlotAddress) -> &[PlannedDestination] {
        self.destinations
            .get(leg)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// The trusted executable one `Operation` declares.
    pub fn executable(&self, operation: &ResourceRef) -> Option<&PlannedExecutable> {
        self.executables.get(operation)
    }
}

// ---------------------------------------------------------------------------
// The resolved plan
// ---------------------------------------------------------------------------

/// One relationship leg an admitted effect runs on, after admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLeg {
    key: BindingKey,
    admission: BindingAdmission,
    presentation: Vec<BindingRealizationFacet>,
    helper: Option<ResourceRef>,
}

impl ResolvedLeg {
    /// The exact admitted relationship.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// The admission this leg is fenced by.
    pub const fn admission(&self) -> &BindingAdmission {
        &self.admission
    }

    /// The presentation facets the effect depends on.
    pub fn presentation(&self) -> &[BindingRealizationFacet] {
        &self.presentation
    }

    /// The helper this leg is realized for, when it is an attenuated
    /// realization leg rather than the consumer's own use (KTD9, AE27).
    pub const fn helper(&self) -> Option<&ResourceRef> {
        self.helper.as_ref()
    }
}

/// The exact desired state one resolved plan is fenced against.
///
/// This is the union of every committed row the effect depends on, as the
/// broker currently observes it. Comparing it is what decides whether earlier
/// authority still holds: an ownership, view, consumer, provider-assignment,
/// or execution-policy change advances a row's desired revision even when the
/// rendered spec bytes do not change, so the fence moves without a spec
/// generation moving (R35, AE16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectFreshness {
    zone: ZoneId,
    store: StoreIncarnation,
    observed: Vec<FreshnessTuple>,
}

impl EffectFreshness {
    /// The Zone the plan runs in.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// The store generation the plan's rows were observed in.
    pub const fn store(&self) -> &StoreIncarnation {
        &self.store
    }

    /// Every observed dependency, in canonical order.
    pub fn observed(&self) -> &[FreshnessTuple] {
        &self.observed
    }

    /// Whether every observed dependency still matches `current`.
    ///
    /// Re-checked immediately before dispatch, so an acceptance that lands
    /// between admission and execution fences the effect rather than letting
    /// a superseded plan run.
    pub fn is_current(&self, current: &[FreshnessTuple]) -> bool {
        self.observed
            .iter()
            .all(|admitted| rows_match(admitted, current))
    }
}

/// Whether one admitted row is still present in `current` at the same store
/// generation, identity, revision, and digest.
fn rows_match(admitted: &FreshnessTuple, current: &[FreshnessTuple]) -> bool {
    current.iter().any(|row| {
        row.store_incarnation() == admitted.store_incarnation()
            && row.resource_uid() == admitted.resource_uid()
            && row.desired_revision() == admitted.desired_revision()
            && row.desired_digest() == admitted.desired_digest()
    })
}

/// One nested leg's already-validated correlation identity.
///
/// The broker admits the correlation before the plan is resolved, so the
/// plan records a fact rather than deciding it: a nested leg's initiating
/// subject is the root leg's recorded subject, never a substitution the
/// transport offers (AE15, R8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrelationRecord {
    root_invocation_id: String,
    initiating_subject: AuthoritySubject,
    invoking_subject: AuthoritySubject,
    depth: u8,
}

impl CorrelationRecord {
    /// Construct one validated correlation record.
    pub const fn new(
        root_invocation_id: String,
        initiating_subject: AuthoritySubject,
        invoking_subject: AuthoritySubject,
        depth: u8,
    ) -> Self {
        Self {
            root_invocation_id,
            initiating_subject,
            invoking_subject,
            depth,
        }
    }

    /// The invocation identifier every leg of this chain shares.
    pub fn root_invocation_id(&self) -> &str {
        &self.root_invocation_id
    }

    /// The subject the whole chain was initiated under.
    pub const fn initiating_subject(&self) -> &AuthoritySubject {
        &self.initiating_subject
    }

    /// The subject whose handler this leg is.
    pub const fn invoking_subject(&self) -> &AuthoritySubject {
        &self.invoking_subject
    }

    /// This leg's depth, zero for the root.
    pub const fn depth(&self) -> u8 {
        self.depth
    }
}

/// One relationship leg an invocation names.
///
/// A leg is a *claim* about which admitted relationship the effect runs on and
/// which presentation it depends on. It carries no path, no right the source
/// has not admitted, and no policy: the source's own decision, the
/// realization's declared support, and the dependency fence are all evaluated
/// against the accepted graph when the plan is resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingPlanRequest {
    key: BindingKey,
    rights: RequestedRights,
    presentation: Vec<BindingRealizationFacet>,
    helper: Option<ResourceRef>,
}

impl BindingPlanRequest {
    /// Construct one leg claim.
    pub const fn new(
        key: BindingKey,
        rights: RequestedRights,
        presentation: Vec<BindingRealizationFacet>,
        helper: Option<ResourceRef>,
    ) -> Self {
        Self {
            key,
            rights,
            presentation,
            helper,
        }
    }

    /// The exact relationship this leg names.
    pub const fn key(&self) -> &BindingKey {
        &self.key
    }

    /// The right the effect claims.
    pub const fn rights(&self) -> RequestedRights {
        self.rights
    }

    /// The presentation facets the effect depends on.
    pub fn presentation(&self) -> &[BindingRealizationFacet] {
        &self.presentation
    }

    /// The helper this leg is realized for, when it is an attenuated leg.
    pub const fn helper(&self) -> Option<&ResourceRef> {
        self.helper.as_ref()
    }
}

/// Everything one admitted effect's plan is resolved from.
///
/// The request is assembled by the broker from the wire carrier plus the
/// broker's own accepted graph. It carries no authority value: an
/// `Operation`, a subject, relationship claims, typed parameters, the
/// dependency versions the caller expects, where the call arrived, and the
/// already-validated correlation of a nested leg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectPlanRequest {
    operation: ResourceRef,
    callable: CallableOperation,
    subject: AuthoritySubject,
    legs: Vec<BindingPlanRequest>,
    parameters: AdmittedParameters,
    expected: Vec<FreshnessTuple>,
    transport: TransportIdentity,
    correlation: Option<CorrelationRecord>,
}

impl EffectPlanRequest {
    /// Assemble one plan request.
    ///
    /// # Errors
    ///
    /// Returns [`PlanRequestError`] when the reference does not name an
    /// `Operation`, when no leg is named, when more legs than
    /// [`MAX_PLAN_LEGS`] are named, when the expected dependency set is
    /// empty, or when that set spans two store generations or two Zones.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operation: ResourceRef,
        callable: CallableOperation,
        subject: AuthoritySubject,
        legs: Vec<BindingPlanRequest>,
        parameters: AdmittedParameters,
        expected: Vec<FreshnessTuple>,
        transport: TransportIdentity,
        correlation: Option<CorrelationRecord>,
    ) -> Result<Self, PlanRequestError> {
        if operation.resource_type().as_str() != OPERATION_RESOURCE_TYPE {
            return Err(PlanRequestError::NotAnOperation);
        }
        if legs.is_empty() {
            return Err(PlanRequestError::NoLeg);
        }
        if legs.len() > MAX_PLAN_LEGS {
            return Err(PlanRequestError::TooManyLegs);
        }
        if expected.is_empty() {
            return Err(PlanRequestError::NoExpectedDependency);
        }
        let anchor = &expected[0];
        if expected
            .iter()
            .any(|tuple| !tuple.same_store(anchor) || tuple.zone() != anchor.zone())
        {
            return Err(PlanRequestError::MixedStoreGeneration);
        }
        Ok(Self {
            operation,
            callable,
            subject,
            legs,
            parameters,
            expected,
            transport,
            correlation,
        })
    }

    /// The exact `Operation` the invocation names.
    pub const fn operation(&self) -> &ResourceRef {
        &self.operation
    }

    /// The committed `Operation`'s declared contract.
    pub const fn callable(&self) -> &CallableOperation {
        &self.callable
    }

    /// The subject the decision is made for.
    pub const fn subject(&self) -> &AuthoritySubject {
        &self.subject
    }

    /// The relationship legs the invocation names.
    pub fn legs(&self) -> &[BindingPlanRequest] {
        &self.legs
    }

    /// The admitted typed parameters.
    pub const fn parameters(&self) -> &AdmittedParameters {
        &self.parameters
    }

    /// The dependency versions the caller expects.
    pub fn expected(&self) -> &[FreshnessTuple] {
        &self.expected
    }

    /// Where the request arrived. Recorded, never authority.
    pub const fn transport(&self) -> TransportIdentity {
        self.transport
    }

    /// The validated correlation of a nested leg, when this is one.
    pub const fn correlation(&self) -> Option<&CorrelationRecord> {
        self.correlation.as_ref()
    }
}

/// Why one plan request was refused before it was resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanRequestError {
    /// The reference did not name an `Operation` row.
    NotAnOperation,
    /// No relationship leg was named.
    NoLeg,
    /// More legs than the plan admits.
    TooManyLegs,
    /// No expected dependency version was named.
    NoExpectedDependency,
    /// The expected dependencies span two store generations or two Zones.
    MixedStoreGeneration,
}

impl core::fmt::Display for PlanRequestError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::NotAnOperation => "an admitted effect must name an Operation row",
            Self::NoLeg => "an admitted effect must name the relationship leg it runs on",
            Self::TooManyLegs => "the invocation names more legs than the plan admits",
            Self::NoExpectedDependency => {
                "an admitted effect must name its expected dependency versions"
            }
            Self::MixedStoreGeneration => {
                "the expected dependencies span more than one store generation or Zone"
            }
        };
        formatter.write_str(text)
    }
}

impl std::error::Error for PlanRequestError {}

/// What one plan resolution refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanRefusalKind {
    /// The expected dependency set is not in the accepted graph's store
    /// generation or Zone.
    StoreIncarnationMismatch,
    /// The subject is not authorized for one leg's relationship row.
    LegNotAuthorized,
    /// The source's own decision or the realization's declared support refused
    /// the leg.
    LegRefused,
    /// The broker holds no private value for a row the effect depends on.
    SourceUnproven,
    /// The broker holds no trusted executable for this `Operation`.
    ImplementationUntrusted,
    /// The declared implementation of the `Operation` is not the one the
    /// broker resolved.
    ImplementationMismatch,
    /// A caller-expected dependency version no longer matches what the broker
    /// observes: authority moved without a spec generation moving (AE16).
    StaleAuthority,
    /// The `Operation`'s declared contract is not one this plan realizes.
    ContractMismatch,
}

impl PlanRefusalKind {
    /// The closed code one refusal is reported under.
    pub const fn code(self) -> &'static str {
        match self {
            Self::StoreIncarnationMismatch => "effect-store-incarnation-mismatch",
            Self::LegNotAuthorized => "effect-leg-not-authorized",
            Self::LegRefused => "effect-leg-refused",
            Self::SourceUnproven => "effect-source-unproven",
            Self::ImplementationUntrusted => "effect-implementation-untrusted",
            Self::ImplementationMismatch => "effect-implementation-mismatch",
            Self::StaleAuthority => "effect-stale-authority",
            Self::ContractMismatch => "effect-contract-mismatch",
        }
    }

    /// The stage the refusal was made at.
    pub const fn stage(self) -> AdmissionStage {
        match self {
            Self::SourceUnproven | Self::ContractMismatch => AdmissionStage::Prepare,
            Self::LegRefused => AdmissionStage::Admit,
            Self::StoreIncarnationMismatch
            | Self::LegNotAuthorized
            | Self::StaleAuthority
            | Self::ImplementationUntrusted
            | Self::ImplementationMismatch => AdmissionStage::Authorize,
        }
    }

    /// The typed reason the refusal is reported under.
    pub const fn reason(self) -> RefusalReason {
        match self {
            Self::StoreIncarnationMismatch => RefusalReason::StoreIncarnationMismatch,
            Self::LegNotAuthorized => RefusalReason::IdentityNotAuthorized,
            Self::LegRefused => RefusalReason::SourcePolicyRefused,
            Self::SourceUnproven => RefusalReason::UnprovenEffect,
            Self::ImplementationUntrusted | Self::ImplementationMismatch => {
                RefusalReason::UntrustedImplementation
            }
            Self::StaleAuthority => RefusalReason::StaleAuthority,
            Self::ContractMismatch => RefusalReason::MandatoryFacetUnsupported,
        }
    }
}

impl core::fmt::Display for PlanRefusalKind {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let text = match self {
            Self::StoreIncarnationMismatch => {
                "the expected dependencies are not in the accepted store generation"
            }
            Self::LegNotAuthorized => "the subject is not authorized for this relationship",
            Self::LegRefused => "the source refused this relationship",
            Self::SourceUnproven => "the broker holds no private value for a dependency",
            Self::ImplementationUntrusted => "this Operation declares no trusted implementation here",
            Self::ImplementationMismatch => {
                "the declared implementation is not the one the broker resolved"
            }
            Self::StaleAuthority => "an expected dependency version is behind the accepted state",
            Self::ContractMismatch => "the Operation's declared contract is not realized here",
        };
        formatter.write_str(text)
    }
}

/// One refused plan resolution, with its enforcing stage and typed reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanRefusal {
    kind: PlanRefusalKind,
}

impl PlanRefusal {
    /// Construct one refusal.
    pub const fn new(kind: PlanRefusalKind) -> Self {
        Self { kind }
    }

    /// The refusal's class.
    pub const fn kind(self) -> PlanRefusalKind {
        self.kind
    }

    /// The stage the refusal was made at.
    pub const fn stage(self) -> AdmissionStage {
        self.kind.stage()
    }

    /// The typed reason the refusal is reported under.
    pub const fn reason(self) -> RefusalReason {
        self.kind.reason()
    }
}

impl core::fmt::Display for PlanRefusal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(formatter, "{}", self.kind)
    }
}

impl std::error::Error for PlanRefusal {}

/// The exact private values one admitted effect runs against.
///
/// The plan is what the broker hands one declared implementation. It is
/// derived execution data: it decides nothing, and it is not an authored
/// resource. It does not serialize, so it cannot be persisted, replayed, or
/// returned to a caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPlan {
    operation: ResourceRef,
    executable: PlannedExecutable,
    subject: AuthoritySubject,
    legs: Vec<ResolvedLeg>,
    sources: Vec<PlannedSource>,
    views: Vec<PlannedView>,
    destinations: Vec<PlannedDestination>,
    identity: Option<PlannedIdentity>,
    parameters: AdmittedParameters,
    freshness: EffectFreshness,
    transport: TransportIdentity,
    correlation: Option<CorrelationRecord>,
}

impl ExecutionPlan {
    /// The exact `Operation` the effect runs.
    pub const fn operation(&self) -> &ResourceRef {
        &self.operation
    }

    /// The trusted executable the effect runs.
    pub const fn executable(&self) -> &PlannedExecutable {
        &self.executable
    }

    /// The subject the effect is admitted for.
    pub const fn subject(&self) -> &AuthoritySubject {
        &self.subject
    }

    /// The admitted relationship legs.
    pub fn legs(&self) -> &[ResolvedLeg] {
        &self.legs
    }

    /// The exact sources this effect addresses.
    pub fn sources(&self) -> &[PlannedSource] {
        &self.sources
    }

    /// The exact named views this effect addresses.
    pub fn views(&self) -> &[PlannedView] {
        &self.views
    }

    /// The exact presentation destinations this effect applies.
    pub fn destinations(&self) -> &[PlannedDestination] {
        &self.destinations
    }

    /// The identity this effect runs as, when the graph admitted one.
    pub const fn identity(&self) -> Option<&PlannedIdentity> {
        self.identity.as_ref()
    }

    /// The admitted typed parameters.
    pub const fn parameters(&self) -> &AdmittedParameters {
        &self.parameters
    }

    /// The fence this plan is admitted under.
    pub const fn freshness(&self) -> &EffectFreshness {
        &self.freshness
    }

    /// Where the request arrived. Recorded, never authority.
    pub const fn transport(&self) -> TransportIdentity {
        self.transport
    }

    /// The validated correlation of a nested leg, when this is one.
    pub const fn correlation(&self) -> Option<&CorrelationRecord> {
        self.correlation.as_ref()
    }
}

/// Resolve one admitted effect's private plan.
///
/// The function is a pure composition of the two admission boundaries that
/// already exist - [`GraphAuthority::admit_mutation`] for the subject's grant
/// on the relationship's own row, and [`GraphAuthority::admit_binding`] for
/// the source's decision, the realization's declared support, and the
/// dependency fence - followed by a lookup of every private value in the
/// broker's own table. It refuses at the first failure, so a caller always
/// sees one enforcing stage.
///
/// Two properties are load-bearing:
///
/// - **The caller's expectation is checked against the broker's
///   observation.** Every expected dependency version must be a row the
///   broker currently observes at the same store generation, revision, and
///   digest, and every leg's own dependencies must appear in the caller's
///   expected set at that exact state. A caller that formed its expectation
///   before an ownership, view, consumer, provider-assignment, or
///   execution-policy change is refused, even though the rendered spec bytes
///   - and therefore any spec generation - did not move (R35, AE16).
/// - **Absence is a refusal.** A leg whose source, the consumer, the
///   subject's identity, or the `Operation`'s executable has no entry in the
///   broker's table is refused; nothing defaults to a path, a uid, or a
///   program.
///
/// # Errors
///
/// Refuses with [`PlanRefusal`] when the request names a store generation the
/// accepted graph does not describe, when the subject is unauthorized for a
/// leg, when the source or realization refuses a leg, when a dependency's
/// private value is absent, when a caller-expected version is behind the
/// accepted state, or when the `Operation`'s declared implementation is not
/// the one the broker resolved.
pub fn resolve_execution_plan(
    request: &EffectPlanRequest,
    accepted: &AcceptedGraph,
    table: &PrivateExecutionTable,
) -> Result<ExecutionPlan, PlanRefusal> {
    let zone = accepted.zone().clone();
    let store = accepted.store().clone();
    let refuse = PlanRefusal::new;

    // The expected versions and the accepted graph must describe one store
    // generation: a different generation is a different store, not a newer
    // one.
    for expected in request.expected() {
        if expected.store_incarnation() != &store || expected.zone() != &zone {
            return Err(refuse(PlanRefusalKind::StoreIncarnationMismatch));
        }
        if table.observed(expected.resource_uid()).is_none() {
            return Err(refuse(PlanRefusalKind::SourceUnproven));
        }
    }

    // The Operation's declared payload provenance is evidence about where the
    // payload comes from, not a second authority source: a `Derived`
    // operation derives its values from the plan below, a `Request` or
    // `Bundle` one from the admitted parameters or the bundle, and none of
    // the three can widen what the plan resolved.

    // The trusted implementation: the broker resolves it by the exact
    // `Operation`, and it must be the implementation that row declares.
    let executable = table
        .executable(request.operation())
        .ok_or(PlanRefusal::new(PlanRefusalKind::ImplementationUntrusted))?;
    if executable.implementation() != request.callable().implementation() {
        return Err(refuse(PlanRefusalKind::ImplementationMismatch));
    }

    let evidence = MutationSubjectEvidence::new(request.subject().clone(), request.transport());
    let mut legs = Vec::with_capacity(request.legs().len());
    let mut sources: Vec<PlannedSource> = Vec::new();
    let mut views: Vec<PlannedView> = Vec::new();
    let mut destinations: Vec<PlannedDestination> = Vec::new();
    let mut fence: Vec<FreshnessTuple> = Vec::new();
    let mut seen_legs: BTreeSet<BindingSlotAddress> = BTreeSet::new();

    for claim in request.legs() {
        let key = claim.key();
        if !seen_legs.insert(key.address()) {
            return Err(refuse(PlanRefusalKind::ContractMismatch));
        }

        // The subject's grant is read from the prior accepted graph for the
        // relationship's own row: a well-formed claim is not authorization,
        // and a grant that is not in the prior state authorizes nothing.
        let row = binding_row_ref(key).ok_or_else(|| refuse(PlanRefusalKind::LegRefused))?;
        let mutation =
            GraphMutation::new(zone.clone(), evidence.clone(), MutationKind::Create, row);
        let authorization = match GraphAuthority::admit_mutation(&mutation, accepted) {
            AdmissionDecision::Admitted => BindingAuthorization::granted(),
            AdmissionDecision::Refused { .. } => BindingAuthorization::absent(),
        };

        // The private values for this leg: the source's committed state, the
        // consumer's, and the source itself. Absence refuses.
        let source_state = table
            .observed(key.source_uid())
            .ok_or_else(|| refuse(PlanRefusalKind::SourceUnproven))?;
        let consumer_state = table
            .observed(key.consumer_uid())
            .ok_or_else(|| refuse(PlanRefusalKind::SourceUnproven))?;
        let source = table
            .source(key.source_uid())
            .ok_or_else(|| refuse(PlanRefusalKind::SourceUnproven))?;
        if source.kind() != key.kind() || source.reference() != key.source_ref() {
            return Err(refuse(PlanRefusalKind::SourceUnproven));
        }

        let dependencies = vec![source_state.clone(), consumer_state.clone()];
        let admission = GraphAuthority::admit_binding(
            BindingAdmissionRequest::new(
                key.clone(),
                claim.rights(),
                claim.presentation().to_vec(),
                authorization,
                dependencies,
            ),
            accepted,
        )
        .map_err(|refusal| {
            refuse(if refusal.reason() == RefusalReason::IdentityNotAuthorized {
                PlanRefusalKind::LegNotAuthorized
            } else {
                PlanRefusalKind::LegRefused
            })
        })?;

        // The AE16 fence: the caller must have expected the state the broker
        // observes. A dependency that moved without advancing a spec
        // generation still moved its desired revision and digest, so the
        // caller's older expectation is refused rather than accepted.
        if !admission.is_current(request.expected()) {
            return Err(refuse(PlanRefusalKind::StaleAuthority));
        }

        for tuple in [source_state, consumer_state] {
            if !fence
                .iter()
                .any(|admitted| admitted.resource_uid() == tuple.resource_uid())
            {
                fence.push(tuple.clone());
            }
        }
        for view in source.views() {
            if !views
                .iter()
                .any(|existing| existing.name() == view.name())
            {
                views.push(view.clone());
            }
        }
        destinations.extend_from_slice(table.destinations(&key.address()));
        if !sources
            .iter()
            .any(|existing| existing.uid() == source.uid())
        {
            sources.push(source.clone());
        }
        legs.push(ResolvedLeg {
            key: key.clone(),
            admission,
            presentation: claim.presentation().to_vec(),
            helper: claim.helper().cloned(),
        });
    }

    if views.len() > MAX_PLAN_VIEWS {
        return Err(refuse(PlanRefusalKind::ContractMismatch));
    }
    if destinations.len() > MAX_PLAN_DESTINATIONS {
        return Err(refuse(PlanRefusalKind::ContractMismatch));
    }

    // The subject's own identity, when the graph admits one. A bootstrap or
    // operator subject names no resource, so it runs as whatever the trusted
    // implementation's own contract fixes and carries no graph identity.
    let identity = match request.subject().resource_ref() {
        None => None,
        Some(subject) => {
            let identity = table
                .identity(subject)
                .ok_or_else(|| refuse(PlanRefusalKind::SourceUnproven))?;
            if identity.subject() != subject {
                return Err(refuse(PlanRefusalKind::SourceUnproven));
            }
            Some(identity.clone())
        }
    };

    Ok(ExecutionPlan {
        operation: request.operation().clone(),
        executable: executable.clone(),
        subject: request.subject().clone(),
        legs,
        sources,
        views,
        destinations,
        identity,
        parameters: request.parameters().clone(),
        freshness: EffectFreshness {
            zone,
            store,
            observed: fence,
        },
        transport: request.transport(),
        correlation: request.correlation().cloned(),
    })
}

/// The canonical `ResourceRef` one relationship's own row carries.
///
/// A relationship is a row like any other, and its authorization is read
/// against that row. The reference is derived from the KTD3 key - the binding
/// row's own `ResourceType` plus the stable consumer slot, which is unique per
/// Zone, consumer, family, and slot - so it is a function of the relationship,
/// never a second authored identity for it.
pub fn binding_row_ref(key: &BindingKey) -> Option<ResourceRef> {
    ResourceRef::parse(&format!(
        "{}/{}",
        key.kind().resource_type(),
        key.slot().as_str()
    ))
    .ok()
}

/// The declared response descriptors one `Operation` must return.
///
/// The count and the kinds are read from the `Operation`'s own fd contract, so
/// a result is checked against what the declaration promised rather than
/// against what happened to arrive (R30, R38).
pub fn declared_response_descriptors(callable: &CallableOperation) -> &[FdContract] {
    callable.fds().response()
}
