//! The `AudioBinding` resource type: one audio backing attached to a target.
//!
//! A binding reads the AudioService it realizes from and the Guest it attaches
//! to, owns the audio worker and endpoint children the Provider derives from
//! the binding, and drives the Provider's lease lifecycle through the family
//! effect port. The children are the audio Provider's realization
//! (`AudioBindingController::child_resources`), so this crate authors them
//! through its own [`AudioBindingChildSource`] implementation and owns the
//! manager child shape they materialize into.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_provider::v3::semantic_services::child_resources::BindingChildIntent;
use d2b_contracts_resource::v3::{ResourceRef, ZoneId};
use d2b_provider_audio_pipewire::{
    AUDIO_REPAIR_INTERVAL_SECS, AudioBindingController, AudioBindingSpec, FakeAudioMediator,
};
use d2b_resource_runtime::context::{ChildEnsure, SpecDecoder};
use d2b_resource_types::{AllowedSources, DriverDescriptor, WellKnownType};

use d2b_provider_wayland_policy::{
    AUDIO_BINDING_TYPE,
    interaction::{
        InteractionChildContext, InteractionDriver, InteractionDriverArgs,
        InteractionDriverFactory, InteractionEffectError, InteractionKind,
        InteractionSpecEnvelope, InteractionType, binding_child_ensure, spec_decoder,
    },
};

/// The Provider reference the type's rows select.
pub const AUDIO_BINDING_PROVIDER_REF: &str = d2b_provider_audio_pipewire::PROVIDER_REF;

/// The preserved reconcile resync cadence of the type.
pub const AUDIO_BINDING_RESYNC: Duration = Duration::from_secs(AUDIO_REPAIR_INTERVAL_SECS);

/// Everything the audio Provider's controller needs to declare one binding's
/// children.
pub struct AudioBindingChildRequest<'a> {
    /// The Zone the binding belongs to (the child bodies bind to it).
    pub zone: &'a ZoneId,
    /// The binding's resource reference.
    pub binding_ref: &'a ResourceRef,
    /// The binding's decoded spec.
    pub spec: &'a AudioBindingSpec,
}

/// The source of one binding's child intents.
///
/// The audio Provider's controller holds the realization, so the intents are
/// authored by this crate's own implementation over the controller's child
/// synthesis; a caller that needs a different source may supply one through
/// [`AudioBinding::new`].
pub trait AudioBindingChildSource: Send + Sync + 'static {
    /// The Process and Endpoint intents one binding owns.
    fn binding_children(
        &self,
        request: &AudioBindingChildRequest<'_>,
    ) -> Result<Vec<BindingChildIntent>, InteractionEffectError>;
}

/// The crate's own child-intent source: the audio Provider controller's
/// child synthesis.
///
/// Child synthesis is mediator-independent, so the controller's associated
/// function is invoked over the crate's public fake mediator stand-in; the
/// stand-in is never constructed on this path.
#[derive(Clone, Copy, Debug, Default)]
pub struct BindingChildSource;

impl AudioBindingChildSource for BindingChildSource {
    fn binding_children(
        &self,
        request: &AudioBindingChildRequest<'_>,
    ) -> Result<Vec<BindingChildIntent>, InteractionEffectError> {
        AudioBindingController::<FakeAudioMediator>::child_resources(
            request.binding_ref,
            request.spec,
        )
        .map(|set| set.iter().cloned().collect())
        .map_err(|_| InteractionEffectError::InvalidResource)
    }
}

/// The `AudioBinding` driver behavior and declaration.
#[derive(Clone)]
pub struct AudioBinding {
    children: Arc<dyn AudioBindingChildSource>,
}

impl AudioBinding {
    /// Build the behavior over a child-intent source.
    pub fn new(children: Arc<dyn AudioBindingChildSource>) -> Self {
        Self { children }
    }
}

impl Default for AudioBinding {
    fn default() -> Self {
        Self::new(Arc::new(BindingChildSource))
    }
}

impl InteractionType for AudioBinding {
    const KIND: InteractionKind = InteractionKind::AudioBinding;
    const RESOURCE_TYPE: &'static str = AUDIO_BINDING_TYPE;
    const PROVIDER_REF: &'static str = AUDIO_BINDING_PROVIDER_REF;
    /// The audio Provider's typed specs carry the universal `providerRef`
    /// themselves, so the row must select this Provider.
    const SPEC_PROVIDER_SELECTOR: bool = true;

    fn resync(&self) -> Duration {
        AUDIO_BINDING_RESYNC
    }

    /// The binding spec decodes with its typed `providerRef` re-inserted.
    fn validate(
        &self,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<(), InteractionEffectError> {
        envelope.spec_with_provider_ref::<AudioBindingSpec>().map(|_| ())
    }

    /// The service the binding realizes from and the target it attaches to.
    fn dependencies(
        &self,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ResourceRef>, InteractionEffectError> {
        let spec = envelope.spec_with_provider_ref::<AudioBindingSpec>()?;
        Ok(vec![spec.service_ref.clone(), spec.target_ref.clone()])
    }

    /// The binding's worker and endpoint children, as manager child rows.
    fn desired_children(
        &self,
        children: &InteractionChildContext<'_>,
        envelope: &InteractionSpecEnvelope,
    ) -> Result<Vec<ChildEnsure>, InteractionEffectError> {
        let spec = envelope.spec_with_provider_ref::<AudioBindingSpec>()?;
        let binding_ref = d2b_provider_wayland_policy::interaction::key_ref(children.key);
        self.children
            .binding_children(&AudioBindingChildRequest {
                zone: children.zone,
                binding_ref: &binding_ref,
                spec: &spec,
            })?
            .iter()
            .map(|intent| binding_child_ensure(intent, children.zone))
            .collect()
    }
}

/// The driver for one `AudioBinding` row.
pub type AudioBindingDriver = InteractionDriver<AudioBinding>;

/// The factory the registry serves for `AudioBinding`.
pub type AudioBindingFactory = InteractionDriverFactory<AudioBinding>;

/// The manager-wired decode hook for `AudioBinding` rows.
pub fn audio_binding_spec_decoder() -> Arc<dyn SpecDecoder> {
    spec_decoder()
}

/// The `AudioBinding` driver declaration.
///
/// The registry keys the type by this declaration, so the type reaches the
/// plane only through it.
pub fn audio_binding_descriptor(args: InteractionDriverArgs<AudioBinding>) -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::AUDIO_BINDING,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP | AllowedSources::RUNTIME,
        verbs: d2b_provider_wayland_policy::INTERACTION_VERBS,
        execution: d2b_provider_wayland_policy::INTERACTION_EXECUTION_DOMAINS,
        exportable: false,
        reads: &[WellKnownType::AUDIO_SERVICE, WellKnownType::GUEST],
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: audio_binding_spec_decoder(),
        factory: Arc::new(AudioBindingFactory::new(args)),
    }
}
