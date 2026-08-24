//! Deterministic child-resource intents for explicit semantic Bindings.
//!
//! A Binding controller uses this module to describe the Process,
//! EphemeralProcess, and Endpoint resources it owns.  The returned intents
//! contain no store-generated UID and are safe to submit through Core's
//! resource admission path.  Construction requires an authored Binding,
//! Service, target, and Provider reference; there is deliberately no API that
//! derives a child from a Service alone.

use std::fmt::Write;

use sha2::{Digest, Sha256};

use super::{BindingTargetType, SemanticFamily};
use d2b_contracts_resource::v3::{ExecutionDomain, ResourceName, ResourceRef};

/// The resource kinds a semantic Binding may own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BindingChildKind {
    /// A long-lived provider component.
    Process,
    /// A one-shot provider worker.
    EphemeralProcess,
    /// A stable endpoint produced by a child process.
    Endpoint,
}

impl BindingChildKind {
    /// Return the canonical ResourceType name.
    pub const fn resource_type(self) -> &'static str {
        match self {
            Self::Process => "Process",
            Self::EphemeralProcess => "EphemeralProcess",
            Self::Endpoint => "Endpoint",
        }
    }

    const fn teardown_rank(self) -> u8 {
        match self {
            Self::Endpoint => 0,
            Self::EphemeralProcess => 1,
            Self::Process => 2,
        }
    }
}

/// The target on which a child resource is reconciled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BindingChildPlacement {
    /// The host target.
    Host,
    /// The consuming guest target.
    Guest,
}

/// One controller-declared child shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingChildRequest {
    kind: BindingChildKind,
    placement: BindingChildPlacement,
    role: &'static str,
    producer_role: Option<&'static str>,
    process_provider: Option<&'static str>,
    process_template: Option<&'static str>,
    process_domain: Option<ExecutionDomain>,
    process_class: Option<&'static str>,
    process_user: bool,
}

impl BindingChildRequest {
    /// Construct a child request with no Endpoint producer dependency.
    pub const fn new(
        kind: BindingChildKind,
        placement: BindingChildPlacement,
        role: &'static str,
    ) -> Self {
        Self {
            kind,
            placement,
            role,
            producer_role: None,
            process_provider: None,
            process_template: None,
            process_domain: None,
            process_class: None,
            process_user: false,
        }
    }

    /// Construct a Process or EphemeralProcess request with its signed
    /// execution contract.
    pub const fn process(
        kind: BindingChildKind,
        placement: BindingChildPlacement,
        role: &'static str,
        provider: &'static str,
        template: &'static str,
        domain: ExecutionDomain,
        class: &'static str,
    ) -> Self {
        Self {
            kind,
            placement,
            role,
            producer_role: None,
            process_provider: Some(provider),
            process_template: Some(template),
            process_domain: Some(domain),
            process_class: Some(class),
            process_user: false,
        }
    }

    /// Construct a user-domain Process request whose identity is supplied by
    /// the authored Binding target.
    pub const fn process_for_user(
        kind: BindingChildKind,
        placement: BindingChildPlacement,
        role: &'static str,
        provider: &'static str,
        template: &'static str,
        class: &'static str,
    ) -> Self {
        Self {
            kind,
            placement,
            role,
            producer_role: None,
            process_provider: Some(provider),
            process_template: Some(template),
            process_domain: Some(ExecutionDomain::User),
            process_class: Some(class),
            process_user: true,
        }
    }

    /// Construct an Endpoint request produced by another declared child.
    pub const fn endpoint(
        placement: BindingChildPlacement,
        role: &'static str,
        producer_role: &'static str,
    ) -> Self {
        Self {
            kind: BindingChildKind::Endpoint,
            placement,
            role,
            producer_role: Some(producer_role),
            process_provider: None,
            process_template: None,
            process_domain: None,
            process_class: None,
            process_user: false,
        }
    }

    /// Return the requested kind.
    pub const fn kind(self) -> BindingChildKind {
        self.kind
    }

    /// Return the requested placement.
    pub const fn placement(self) -> BindingChildPlacement {
        self.placement
    }

    /// Return the Provider-local semantic role.
    pub const fn role(self) -> &'static str {
        self.role
    }

    /// Return the Provider-local producer role, if this is an Endpoint.
    pub const fn producer_role(self) -> Option<&'static str> {
        self.producer_role
    }

    /// Return the Process Provider reference, if explicitly declared.
    pub const fn process_provider(self) -> Option<&'static str> {
        self.process_provider
    }

    /// Return the signed Process template, if explicitly declared.
    pub const fn process_template(self) -> Option<&'static str> {
        self.process_template
    }

    /// Return the signed Process execution domain, if explicitly declared.
    pub const fn process_domain(self) -> Option<ExecutionDomain> {
        self.process_domain
    }

    /// Return the signed Process class, if explicitly declared.
    pub const fn process_class(self) -> Option<&'static str> {
        self.process_class
    }

    /// Whether the Process requires the Binding's User identity.
    pub const fn process_user(self) -> bool {
        self.process_user
    }
}

/// One UID-free child-resource intent owned by a Binding.
#[derive(Clone, PartialEq, Eq)]
pub struct BindingChildIntent {
    owner_ref: ResourceRef,
    provider_ref: ResourceRef,
    resource_ref: ResourceRef,
    execution_ref: ResourceRef,
    kind: BindingChildKind,
    placement: BindingChildPlacement,
    role: &'static str,
    producer_ref: Option<ResourceRef>,
    process_provider: Option<&'static str>,
    process_template: Option<&'static str>,
    process_domain: Option<ExecutionDomain>,
    process_class: Option<&'static str>,
    process_user: Option<ResourceRef>,
}

impl core::fmt::Debug for BindingChildIntent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BindingChildIntent")
            .field("kind", &self.kind)
            .field("placement", &self.placement)
            .field("role", &self.role)
            .field("has_producer", &self.producer_ref.is_some())
            .finish_non_exhaustive()
    }
}

impl BindingChildIntent {
    /// Borrow the authored Binding owner reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the Provider that owns the child controller.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the UID-free child reference.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the Core-resolved execution target for this child.
    ///
    /// Host children always execute under the canonical Host resource. Guest
    /// children execute under the Binding's admitted Guest target.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Return the child kind.
    pub const fn kind(&self) -> BindingChildKind {
        self.kind
    }

    /// Return the resolved target placement.
    pub const fn placement(&self) -> BindingChildPlacement {
        self.placement
    }

    /// Return the Provider-local role.
    pub const fn role(&self) -> &'static str {
        self.role
    }

    /// Borrow the Endpoint producer reference, if this child is an Endpoint.
    pub const fn producer_ref(&self) -> Option<&ResourceRef> {
        self.producer_ref.as_ref()
    }

    /// Return the Process Provider reference, if explicitly declared.
    pub const fn process_provider(&self) -> Option<&'static str> {
        self.process_provider
    }

    /// Return the signed Process template, if explicitly declared.
    pub const fn process_template(&self) -> Option<&'static str> {
        self.process_template
    }

    /// Return the signed Process execution domain, if explicitly declared.
    pub const fn process_domain(&self) -> Option<ExecutionDomain> {
        self.process_domain
    }

    /// Return the signed Process class, if explicitly declared.
    pub const fn process_class(&self) -> Option<&'static str> {
        self.process_class
    }

    /// Borrow the Process User identity, if one was admitted.
    pub const fn process_user(&self) -> Option<&ResourceRef> {
        self.process_user.as_ref()
    }
}

/// An ordered set of children owned by one explicit Binding.
#[derive(Clone, PartialEq, Eq)]
pub struct BindingChildSet {
    family: SemanticFamily,
    owner_ref: ResourceRef,
    target_ref: ResourceRef,
    children: Vec<BindingChildIntent>,
}

impl core::fmt::Debug for BindingChildSet {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("BindingChildSet")
            .field("family", &self.family)
            .field("child_count", &self.children.len())
            .finish_non_exhaustive()
    }
}

impl BindingChildSet {
    /// Return the semantic family represented by this set.
    pub const fn family(&self) -> SemanticFamily {
        self.family
    }

    /// Borrow the explicit Binding owner.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the Binding's admitted consuming target.
    pub const fn target_ref(&self) -> &ResourceRef {
        &self.target_ref
    }

    /// Iterate over children in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = &BindingChildIntent> {
        self.children.iter()
    }

    /// Return a child by its Provider-local role.
    pub fn child(&self, role: &str) -> Option<&BindingChildIntent> {
        self.children.iter().find(|child| child.role == role)
    }

    /// Iterate over children placed on one target.
    pub fn at(
        &self,
        placement: BindingChildPlacement,
    ) -> impl Iterator<Item = &BindingChildIntent> {
        self.children
            .iter()
            .filter(move |child| child.placement == placement)
    }

    /// Return all child references without store-generated identities.
    pub fn resource_refs(&self) -> impl Iterator<Item = &ResourceRef> {
        self.children.iter().map(BindingChildIntent::resource_ref)
    }

    /// Return children in safe deletion order.
    ///
    /// Endpoints are deleted before their producing Process resources.  The
    /// original declaration order is retained for children with the same
    /// teardown rank.
    pub fn teardown_order(&self) -> Vec<&BindingChildIntent> {
        let mut children = self.children.iter().collect::<Vec<_>>();
        children.sort_by_key(|child| child.kind.teardown_rank());
        children
    }

    /// Whether the set contains a child of the supplied kind.
    pub fn contains_kind(&self, kind: BindingChildKind) -> bool {
        self.children.iter().any(|child| child.kind == kind)
    }
}

/// Closed errors while constructing child intents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingChildError {
    /// The Binding reference did not name this family's Binding ResourceType.
    InvalidBindingRef,
    /// The Service reference did not name this family's Service ResourceType.
    InvalidServiceRef,
    /// The target reference did not match the admitted target type.
    InvalidTargetRef,
    /// The Provider reference did not name a Provider.
    InvalidProviderRef,
    /// No child declarations were supplied.
    EmptyDeclaration,
    /// A role was malformed or repeated.
    InvalidRole,
    /// An Endpoint named a missing producer role.
    MissingProducer,
    /// A deterministic child reference could not be constructed.
    InvalidChildRef,
    /// A child placement did not match the Binding's admitted target.
    InvalidPlacement,
    /// An Endpoint producer role did not name a Process child.
    InvalidProducer,
    /// A user-domain Process requires an authored User reference.
    MissingUser,
    /// A supplied User reference did not name a User resource.
    InvalidUserRef,
}

impl core::fmt::Display for BindingChildError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidBindingRef => "binding-child-binding-ref-invalid",
            Self::InvalidServiceRef => "binding-child-service-ref-invalid",
            Self::InvalidTargetRef => "binding-child-target-ref-invalid",
            Self::InvalidProviderRef => "binding-child-provider-ref-invalid",
            Self::EmptyDeclaration => "binding-child-declaration-empty",
            Self::InvalidRole => "binding-child-role-invalid",
            Self::MissingProducer => "binding-child-producer-missing",
            Self::InvalidChildRef => "binding-child-ref-invalid",
            Self::InvalidPlacement => "binding-child-placement-invalid",
            Self::InvalidProducer => "binding-child-producer-invalid",
            Self::MissingUser => "binding-child-user-missing",
            Self::InvalidUserRef => "binding-child-user-ref-invalid",
        })
    }
}

impl std::error::Error for BindingChildError {}

/// Construct child intents for one explicitly authored semantic Binding.
///
/// This is the only constructor for a [`BindingChildSet`].  In particular,
/// callers must provide the Binding and Service references; a Ready Service
/// alone cannot create consumer children.
pub fn explicit_binding_children(
    family: SemanticFamily,
    binding_ref: ResourceRef,
    service_ref: ResourceRef,
    target_ref: ResourceRef,
    provider_ref: ResourceRef,
    declarations: &[BindingChildRequest],
) -> Result<BindingChildSet, BindingChildError> {
    explicit_binding_children_with_user(
        family,
        binding_ref,
        service_ref,
        target_ref,
        provider_ref,
        None,
        declarations,
    )
}

/// Construct child intents while supplying the Binding's admitted User
/// identity for user-domain Processes.
pub fn explicit_binding_children_with_user(
    family: SemanticFamily,
    binding_ref: ResourceRef,
    service_ref: ResourceRef,
    target_ref: ResourceRef,
    provider_ref: ResourceRef,
    user_ref: Option<ResourceRef>,
    declarations: &[BindingChildRequest],
) -> Result<BindingChildSet, BindingChildError> {
    let contract = family.contract();
    if binding_ref.resource_type() != contract.binding().resource_type() {
        return Err(BindingChildError::InvalidBindingRef);
    }
    if service_ref.resource_type() != contract.service().resource_type() {
        return Err(BindingChildError::InvalidServiceRef);
    }
    if provider_ref.resource_type().as_str() != "Provider" {
        return Err(BindingChildError::InvalidProviderRef);
    }
    if let Some(user_ref) = &user_ref
        && user_ref.resource_type().as_str() != "User"
    {
        return Err(BindingChildError::InvalidUserRef);
    }
    let target = binding_target_type(&target_ref)?;
    contract
        .admit_binding_refs(&service_ref, target)
        .map_err(|_| BindingChildError::InvalidTargetRef)?;
    if declarations.is_empty() {
        return Err(BindingChildError::EmptyDeclaration);
    }
    if declarations.iter().any(|declaration| {
        matches!(declaration.placement, BindingChildPlacement::Guest)
            && target != BindingTargetType::Guest
    }) {
        return Err(BindingChildError::InvalidPlacement);
    }

    let mut roles = Vec::with_capacity(declarations.len());
    let mut refs = Vec::with_capacity(declarations.len());
    for declaration in declarations {
        if !valid_role(declaration.role) || roles.contains(&declaration.role) {
            return Err(BindingChildError::InvalidRole);
        }
        if declaration.producer_role.is_some() && declaration.kind != BindingChildKind::Endpoint {
            return Err(BindingChildError::InvalidProducer);
        }
        if declaration.kind == BindingChildKind::Endpoint
            && (declaration.process_provider.is_some()
                || declaration.process_template.is_some()
                || declaration.process_domain.is_some()
                || declaration.process_class.is_some()
                || declaration.process_user)
        {
            return Err(BindingChildError::InvalidProducer);
        }
        if declaration.kind != BindingChildKind::Endpoint {
            if let Some(provider) = declaration.process_provider {
                let provider_ref = ResourceRef::parse(provider)
                    .map_err(|_| BindingChildError::InvalidProviderRef)?;
                if provider_ref.resource_type().as_str() != "Provider" {
                    return Err(BindingChildError::InvalidProviderRef);
                }
            }
            if let Some(template) = declaration.process_template {
                if !valid_role(template) {
                    return Err(BindingChildError::InvalidRole);
                }
            }
            if let Some(class) = declaration.process_class {
                if !matches!(class, "controller" | "service" | "worker") {
                    return Err(BindingChildError::InvalidRole);
                }
            }
            if declaration.process_user && user_ref.is_none() {
                return Err(BindingChildError::MissingUser);
            }
        }
        roles.push(declaration.role);
        let name = child_name(family, &binding_ref, declaration);
        let resource_ref = ResourceRef::parse(&format!(
            "{}/{}",
            declaration.kind.resource_type(),
            name.as_str()
        ))
        .map_err(|_| BindingChildError::InvalidChildRef)?;
        refs.push(resource_ref);
    }

    let mut children = Vec::with_capacity(declarations.len());
    for (declaration, resource_ref) in declarations.iter().zip(refs.iter().cloned()) {
        if declaration.kind == BindingChildKind::Endpoint && declaration.producer_role.is_none() {
            return Err(BindingChildError::MissingProducer);
        }
        let producer_ref = declaration
            .producer_role
            .map(|role| refs_for_role(declarations, &refs, role));
        if let Some(producer_role) = declaration.producer_role {
            let producer = declarations
                .iter()
                .find(|candidate| candidate.role == producer_role)
                .ok_or(BindingChildError::MissingProducer)?;
            if !matches!(
                producer.kind,
                BindingChildKind::Process | BindingChildKind::EphemeralProcess
            ) {
                return Err(BindingChildError::InvalidProducer);
            }
            if producer.placement != declaration.placement {
                return Err(BindingChildError::InvalidPlacement);
            }
        }
        let execution_ref = match declaration.placement {
            BindingChildPlacement::Host => {
                ResourceRef::parse("Host/host-system").expect("canonical Host reference")
            }
            BindingChildPlacement::Guest => target_ref.clone(),
        };
        let Some(producer_ref) = producer_ref.transpose()? else {
            children.push(BindingChildIntent {
                owner_ref: binding_ref.clone(),
                provider_ref: provider_ref.clone(),
                resource_ref,
                execution_ref,
                kind: declaration.kind,
                placement: declaration.placement,
                role: declaration.role,
                producer_ref: None,
                process_provider: declaration.process_provider,
                process_template: declaration.process_template,
                process_domain: declaration.process_domain,
                process_class: declaration.process_class,
                process_user: if declaration.process_user {
                    user_ref.clone()
                } else {
                    None
                },
            });
            continue;
        };
        children.push(BindingChildIntent {
            owner_ref: binding_ref.clone(),
            provider_ref: provider_ref.clone(),
            resource_ref,
            execution_ref,
            kind: declaration.kind,
            placement: declaration.placement,
            role: declaration.role,
            producer_ref: Some(producer_ref),
            process_provider: declaration.process_provider,
            process_template: declaration.process_template,
            process_domain: declaration.process_domain,
            process_class: declaration.process_class,
            process_user: if declaration.process_user {
                user_ref.clone()
            } else {
                None
            },
        });
    }
    Ok(BindingChildSet {
        family,
        owner_ref: binding_ref,
        target_ref,
        children,
    })
}

fn refs_for_role(
    declarations: &[BindingChildRequest],
    refs: &[ResourceRef],
    role: &'static str,
) -> Result<ResourceRef, BindingChildError> {
    let index = declarations
        .iter()
        .position(|declaration| declaration.role == role)
        .ok_or(BindingChildError::MissingProducer)?;
    refs.get(index)
        .cloned()
        .ok_or(BindingChildError::InvalidChildRef)
}

fn binding_target_type(target_ref: &ResourceRef) -> Result<BindingTargetType, BindingChildError> {
    match target_ref.resource_type().as_str() {
        "Guest" => Ok(BindingTargetType::Guest),
        "User" => Ok(BindingTargetType::User),
        "Zone" => Ok(BindingTargetType::Zone),
        _ => Err(BindingChildError::InvalidTargetRef),
    }
}

fn valid_role(role: &str) -> bool {
    let bytes = role.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 24
        && bytes.last() != Some(&b'-')
        && bytes.iter().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || (*byte == b'-' && index > 0 && bytes[index - 1] != b'-')
        })
}

fn child_name(
    family: SemanticFamily,
    binding_ref: &ResourceRef,
    declaration: &BindingChildRequest,
) -> ResourceName {
    let mut input = String::with_capacity(128);
    input.push_str(family.namespace());
    input.push('\0');
    input.push_str(&binding_ref.to_canonical_string());
    input.push('\0');
    input.push_str(declaration.kind.resource_type());
    input.push('\0');
    input.push_str(declaration.role);
    let digest = Sha256::digest(input.as_bytes());
    let mut short_digest = String::with_capacity(12);
    for byte in &digest[..6] {
        write!(&mut short_digest, "{byte:02x}").expect("writing to String cannot fail");
    }
    let candidate = format!(
        "{}-{short_digest}-{}",
        family_slug(family),
        declaration.role
    );
    ResourceName::parse(candidate).expect("child name is bounded by closed constants")
}

const fn family_slug(family: SemanticFamily) -> &'static str {
    match family {
        SemanticFamily::Audio => "audio",
        SemanticFamily::SecurityKey => "security-key",
        SemanticFamily::Telemetry => "telemetry",
        SemanticFamily::Usb => "usb",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs() -> (ResourceRef, ResourceRef, ResourceRef, ResourceRef) {
        (
            ResourceRef::parse("audio.d2bus.org.AudioBinding/mic").unwrap(),
            ResourceRef::parse("audio.d2bus.org.AudioService/host").unwrap(),
            ResourceRef::parse("Guest/dev-vm").unwrap(),
            ResourceRef::parse("Provider/audio-pipewire").unwrap(),
        )
    }

    #[test]
    fn explicit_binding_children_are_deterministic_and_uid_free() {
        let (binding, service, target, provider) = refs();
        let declarations = [
            BindingChildRequest::new(
                BindingChildKind::Process,
                BindingChildPlacement::Host,
                "host-effect",
            ),
            BindingChildRequest::endpoint(
                BindingChildPlacement::Host,
                "host-endpoint",
                "host-effect",
            ),
            BindingChildRequest::new(
                BindingChildKind::Process,
                BindingChildPlacement::Guest,
                "guest-agent",
            ),
            BindingChildRequest::endpoint(
                BindingChildPlacement::Guest,
                "guest-endpoint",
                "guest-agent",
            ),
        ];
        let first = explicit_binding_children(
            SemanticFamily::Audio,
            binding.clone(),
            service.clone(),
            target.clone(),
            provider.clone(),
            &declarations,
        )
        .unwrap();
        let second = explicit_binding_children(
            SemanticFamily::Audio,
            binding,
            service,
            target,
            provider,
            &declarations,
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.iter().count(), 4);
        assert_eq!(
            first.child("host-effect").unwrap().execution_ref(),
            &ResourceRef::parse("Host/host-system").unwrap()
        );
        assert_eq!(
            first.child("guest-agent").unwrap().execution_ref(),
            &ResourceRef::parse("Guest/dev-vm").unwrap()
        );
        assert_eq!(
            first.target_ref(),
            &ResourceRef::parse("Guest/dev-vm").unwrap()
        );
        assert_eq!(
            first
                .child("host-endpoint")
                .unwrap()
                .producer_ref()
                .unwrap()
                .resource_type()
                .as_str(),
            "Process"
        );
        assert_eq!(
            first
                .teardown_order()
                .iter()
                .take(2)
                .map(|child| child.kind())
                .collect::<Vec<_>>(),
            vec![BindingChildKind::Endpoint, BindingChildKind::Endpoint]
        );
    }

    #[test]
    fn service_alone_cannot_create_children() {
        let (_, service, target, provider) = refs();
        let result = explicit_binding_children(
            SemanticFamily::Audio,
            ResourceRef::parse("audio.d2bus.org.AudioService/host").unwrap(),
            service,
            target,
            provider,
            &[BindingChildRequest::new(
                BindingChildKind::Process,
                BindingChildPlacement::Host,
                "host-effect",
            )],
        );
        assert_eq!(result, Err(BindingChildError::InvalidBindingRef));
    }

    #[test]
    fn wrong_service_target_and_producer_are_rejected() {
        let (binding, service, _, provider) = refs();
        let declaration = [BindingChildRequest::new(
            BindingChildKind::Process,
            BindingChildPlacement::Guest,
            "guest-agent",
        )];
        assert_eq!(
            explicit_binding_children(
                SemanticFamily::Audio,
                binding.clone(),
                ResourceRef::parse("usb.d2bus.org.UsbService/usb").unwrap(),
                ResourceRef::parse("Guest/dev-vm").unwrap(),
                provider.clone(),
                &declaration,
            ),
            Err(BindingChildError::InvalidServiceRef)
        );
        assert_eq!(
            explicit_binding_children(
                SemanticFamily::Audio,
                binding.clone(),
                service.clone(),
                ResourceRef::parse("Zone/dev").unwrap(),
                provider.clone(),
                &declaration,
            ),
            Err(BindingChildError::InvalidTargetRef)
        );
        assert_eq!(
            explicit_binding_children(
                SemanticFamily::Audio,
                binding.clone(),
                service.clone(),
                ResourceRef::parse("Guest/dev-vm").unwrap(),
                provider.clone(),
                &[BindingChildRequest::endpoint(
                    BindingChildPlacement::Guest,
                    "guest-endpoint",
                    "missing",
                )],
            ),
            Err(BindingChildError::MissingProducer)
        );
        assert_eq!(
            explicit_binding_children(
                SemanticFamily::Telemetry,
                ResourceRef::parse("telemetry.d2bus.org.TelemetryBinding/metrics").unwrap(),
                ResourceRef::parse("telemetry.d2bus.org.TelemetryService/host").unwrap(),
                ResourceRef::parse("Zone/dev").unwrap(),
                ResourceRef::parse("Provider/observability-otel").unwrap(),
                &[BindingChildRequest::new(
                    BindingChildKind::Process,
                    BindingChildPlacement::Guest,
                    "guest-agent",
                )],
            ),
            Err(BindingChildError::InvalidPlacement)
        );
        assert_eq!(
            explicit_binding_children(
                SemanticFamily::Audio,
                ResourceRef::parse("audio.d2bus.org.AudioBinding/mic").unwrap(),
                ResourceRef::parse("audio.d2bus.org.AudioService/host").unwrap(),
                ResourceRef::parse("Guest/dev-vm").unwrap(),
                ResourceRef::parse("Provider/audio-pipewire").unwrap(),
                &[
                    BindingChildRequest::new(
                        BindingChildKind::Process,
                        BindingChildPlacement::Host,
                        "host-effect",
                    ),
                    BindingChildRequest::endpoint(
                        BindingChildPlacement::Host,
                        "host-endpoint",
                        "host-endpoint",
                    ),
                ],
            ),
            Err(BindingChildError::InvalidProducer)
        );
        assert_eq!(
            explicit_binding_children(
                SemanticFamily::Audio,
                ResourceRef::parse("audio.d2bus.org.AudioBinding/mic").unwrap(),
                ResourceRef::parse("audio.d2bus.org.AudioService/host").unwrap(),
                ResourceRef::parse("Guest/dev-vm").unwrap(),
                ResourceRef::parse("Provider/audio-pipewire").unwrap(),
                &[BindingChildRequest::new(
                    BindingChildKind::Endpoint,
                    BindingChildPlacement::Guest,
                    "guest-endpoint",
                )],
            ),
            Err(BindingChildError::MissingProducer)
        );
        assert_eq!(
            explicit_binding_children(
                SemanticFamily::Audio,
                binding,
                service,
                ResourceRef::parse("Guest/dev-vm").unwrap(),
                ResourceRef::parse("Provider/audio-pipewire").unwrap(),
                &[
                    BindingChildRequest::new(
                        BindingChildKind::Process,
                        BindingChildPlacement::Host,
                        "host-effect",
                    ),
                    BindingChildRequest::endpoint(
                        BindingChildPlacement::Guest,
                        "guest-endpoint",
                        "host-effect",
                    ),
                ],
            ),
            Err(BindingChildError::InvalidPlacement)
        );
    }

    #[test]
    fn malformed_roles_are_rejected_without_panicking() {
        let (binding, service, target, provider) = refs();
        assert_eq!(
            explicit_binding_children(
                SemanticFamily::Audio,
                binding,
                service,
                target,
                provider,
                &[BindingChildRequest::new(
                    BindingChildKind::Process,
                    BindingChildPlacement::Guest,
                    "guest-agent-",
                )],
            ),
            Err(BindingChildError::InvalidRole)
        );
    }
}
