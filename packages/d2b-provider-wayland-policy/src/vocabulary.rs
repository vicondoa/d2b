//! The interaction family's shared spec vocabulary.
//!
//! The family engine is the shared home of the reference shapes every member
//! type reads: the audio type identities, and the shell pool/session spec
//! reference checks. Each per-type crate's driver and the family's production
//! effects read the same vocabulary from here, so a reference shape changes in
//! one place and every consumer follows.
//!
//! The provider-reference fences stay with the shapes: a pool or session spec
//! must select the shell Provider, exactly as the shell crates checked it
//! before the shapes moved here.

use d2b_contracts_resource::v3::ResourceRef;
use serde_json::Value;

use crate::InteractionEffectError;

/// The canonical ResourceType name of an AudioService row.
pub const AUDIO_SERVICE_TYPE: &str = "audio.d2bus.org.AudioService";

/// The canonical ResourceType name of an AudioBinding row.
pub const AUDIO_BINDING_TYPE: &str = "audio.d2bus.org.AudioBinding";

/// The shell Provider reference the pool and session specs must select.
const SHELL_PROVIDER_REF: &str = "Provider/shell-terminal";

/// The shell pool's own ResourceType name (the session's pool reference must
/// name it).
const SHELL_POOL_TYPE_NAME: &str = "shell-terminal.d2bus.org.ShellPool";

/// The shell pool's execution and user references.
///
/// The execution reference must be a Host or Guest, the user reference a User,
/// and the login shell a bounded `artifact://` reference.
pub fn shell_pool_spec(
    base: &Value,
    provider_ref: Option<&str>,
) -> Result<(ResourceRef, ResourceRef), InteractionEffectError> {
    if provider_ref != Some(SHELL_PROVIDER_REF) {
        return Err(InteractionEffectError::InvalidResource);
    }
    let execution_ref = spec_ref(base, "/executionRef")?;
    if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
        return Err(InteractionEffectError::InvalidResource);
    }
    let user_ref = spec_ref(base, "/userRef")?;
    let login_shell = base
        .pointer("/loginShellRef")
        .and_then(Value::as_str)
        .ok_or(InteractionEffectError::InvalidResource)?;
    if user_ref.resource_type().as_str() != "User"
        || !login_shell.starts_with("artifact://")
        || login_shell.len() > 255
    {
        return Err(InteractionEffectError::InvalidResource);
    }
    Ok((execution_ref, user_ref))
}

/// The shell session's execution and optional user references.
///
/// The execution reference must be a Host or Guest, an optional user reference
/// a User, and the login shell a bounded `artifact://` reference.
pub fn shell_session_execution(
    base: &Value,
    provider_ref: Option<&str>,
) -> Result<(ResourceRef, Option<ResourceRef>), InteractionEffectError> {
    if provider_ref != Some(SHELL_PROVIDER_REF) {
        return Err(InteractionEffectError::InvalidResource);
    }
    let execution_ref = spec_ref(base, "/executionRef")?;
    if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
        return Err(InteractionEffectError::InvalidResource);
    }
    let user_ref = base
        .pointer("/userRef")
        .and_then(Value::as_str)
        .map(ResourceRef::parse)
        .transpose()
        .map_err(|_| InteractionEffectError::InvalidResource)?;
    if user_ref
        .as_ref()
        .is_some_and(|reference| reference.resource_type().as_str() != "User")
        || base
            .pointer("/loginShellRef")
            .and_then(Value::as_str)
            .is_none_or(|shell| !shell.starts_with("artifact://") || shell.len() > 255)
    {
        return Err(InteractionEffectError::InvalidResource);
    }
    Ok((execution_ref, user_ref))
}

/// The session's pool reference: it must name a ShellPool.
pub fn shell_session_pool_ref(
    base: &Value,
    provider_ref: Option<&str>,
) -> Result<ResourceRef, InteractionEffectError> {
    if provider_ref != Some(SHELL_PROVIDER_REF) {
        return Err(InteractionEffectError::InvalidResource);
    }
    let pool_ref = spec_ref(base, "/poolRef")?;
    if pool_ref.resource_type().as_str() != SHELL_POOL_TYPE_NAME {
        return Err(InteractionEffectError::InvalidResource);
    }
    Ok(pool_ref)
}

/// One reference field of a shell spec.
fn spec_ref(value: &Value, path: &str) -> Result<ResourceRef, InteractionEffectError> {
    value
        .pointer(path)
        .and_then(Value::as_str)
        .and_then(|reference| ResourceRef::parse(reference).ok())
        .ok_or(InteractionEffectError::InvalidResource)
}
