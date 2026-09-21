//! The AudioService resource type's driver crate.
//!
//! The crate owns the `audio.d2bus.org.AudioService` type completely: the
//! driver, its spec decoder, and the driver declaration the resource plane
//! registers the type by. The driver verbs come from the interaction family's
//! shared engine (`d2b-provider-wayland-policy`), which also owns the
//! family's shared vocabulary (the `AUDIO_SERVICE_TYPE` identity lives
//! there); the family's production audio effects live in the engine crate
//! too.

#![deny(missing_docs)]

mod audio_service;

pub use audio_service::{
    AUDIO_SERVICE_PROVIDER_REF, AUDIO_SERVICE_RESYNC,
    AudioService, AudioServiceDriver, AudioServiceFactory,
    audio_service_descriptor, audio_service_spec_decoder,
};
