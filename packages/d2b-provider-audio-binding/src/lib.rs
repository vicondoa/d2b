//! The AudioBinding resource type's driver crate.
//!
//! The crate owns the `audio.d2bus.org.AudioBinding` type completely: the
//! driver, its spec decoder, its child-intent source, and the driver
//! declaration the resource plane registers the type by. The driver verbs
//! come from the interaction family's shared engine
//! (`d2b-provider-wayland-policy`), which also owns the family's shared
//! vocabulary (the `AUDIO_BINDING_TYPE` identity lives there); the family's
//! production audio effects live in the engine crate too.

#![deny(missing_docs)]

mod audio_binding;

pub use audio_binding::{
    AUDIO_BINDING_PROVIDER_REF, AUDIO_BINDING_RESYNC,
    AudioBinding, AudioBindingChildRequest, AudioBindingChildSource, AudioBindingDriver,
    AudioBindingFactory, BindingChildSource, audio_binding_descriptor,
    audio_binding_spec_decoder,
};
