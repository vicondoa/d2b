//! The AudioBinding resource type's driver crate.
//!
//! The crate owns the `audio.d2bus.org.AudioBinding` type completely: the
//! driver, its spec decoder, its child-intent port, and the driver declaration
//! the resource plane registers the type by. The driver verbs come from the
//! interaction family's shared engine (`d2b-provider-wayland-policy`); the
//! daemon keeps the production audio effects and the Provider's child-intent
//! source behind the ports this crate declares.

#![deny(missing_docs)]

mod audio_binding;

pub use audio_binding::{
    AUDIO_BINDING_CONTROLLER_REF, AUDIO_BINDING_PROVIDER_REF, AUDIO_BINDING_RESYNC,
    AUDIO_BINDING_TYPE, AudioBinding, AudioBindingChildRequest, AudioBindingChildSource,
    AudioBindingDriver, AudioBindingFactory, audio_binding_descriptor,
    audio_binding_spec_decoder,
};
