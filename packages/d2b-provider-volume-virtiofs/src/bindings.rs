//! The neutral `VolumeBinding` resource as `volume-virtiofs` serves it.
//!
//! The Volume side mints one binding per Volume / execution-target /
//! named-view relationship (KTD1). volume-virtiofs observes stored
//! binding envelopes, resolves the named view dependency-only, and never
//! writes a Volume row.

use std::fmt;

use serde::Serialize;
use sha2::{Digest, Sha256};

use d2b_contracts_resource::v3::{
    ResourceGeneration, ResourceRef, ResourceUid, ZoneRevision,
    execution_policy::BoundedToken,
    volume_binding::{VolumeBindingReadinessFence, VolumeBindingSpec},
};

use crate::error::VirtiofsBindingError;

/// The standard ResourceType name this Provider serves (canonical contract).
pub use d2b_contracts_resource::v3::volume_binding::VOLUME_BINDING_RESOURCE_TYPE;

/// The finalizer volume-virtiofs adds to each VolumeBinding, and to
/// nothing else.
pub const VOLUME_BINDING_FINALIZER: &str = "volume-virtiofs.d2bus.org/volume-binding";

/// The opaque identity of one binding's private listening socket.
///
/// The socket path is a generated implementation detail of this
/// Provider. It is never a spec field, a status field, an audit field,
/// or CLI output. Only this digest is public, and the effect adapter
/// alone derives the private path it stands for.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SocketIdentity([u8; 32]);

impl SocketIdentity {
    /// Derive the identity of one binding's socket.
    pub fn derive(
        zone: &BoundedToken,
        volume_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"d2b/volume-virtiofs/binding-socket/v1");
        hasher.update(zone.as_str().as_bytes());
        hasher.update([0u8]);
        hasher.update(volume_ref.to_canonical_string().as_bytes());
        hasher.update([0u8]);
        hasher.update(execution_ref.to_canonical_string().as_bytes());
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&hasher.finalize());
        Self(bytes)
    }

    /// Render the identity as lowercase hex.
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
            out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
        }
        out
    }

    /// Return the eight-character tag used by the private socket filename.
    pub fn short_tag(self) -> String {
        let mut out = String::with_capacity(8);
        for byte in self.0.into_iter().take(4) {
            out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
            out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
        }
        out
    }
}

impl fmt::Debug for SocketIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SocketIdentity(<redacted>)")
    }
}

impl Serialize for SocketIdentity {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

/// One stored VolumeBinding as the serving reconciler sees it: the strict
/// neutral spec plus the identity the fenced status projection is
/// validated against (KTD3).
///
/// The neutral envelope carries no attachment settings (KTD1); the
/// serving posture is the frozen default declared by the worker plan.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredBinding {
    binding: VolumeBindingSpec,
    uid: ResourceUid,
    generation: ResourceGeneration,
    revision: ZoneRevision,
}

impl StoredBinding {
    /// Construct a stored binding from its typed spec and identity.
    pub fn new(
        binding: VolumeBindingSpec,
        uid: ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
    ) -> Self {
        Self {
            binding,
            uid,
            generation,
            revision,
        }
    }

    /// Parse a stored VolumeBinding resource envelope.
    ///
    /// The envelope must be a `VolumeBinding` owned by an existing
    /// Volume and served by this Provider, and it must be strictly
    /// neutral: the standard catalog admits no provider extension path
    /// for the type, so a `spec.provider` block — legacy schema id
    /// or otherwise — is rejected, and the envelope never carries
    /// attachment settings (KTD1). The serving posture is the frozen
    /// default declared by the worker plan.
    pub fn from_resource_spec(value: &serde_json::Value) -> Result<Self, VirtiofsBindingError> {
        let invalid = VirtiofsBindingError::InvalidBinding;
        if value.get("type").and_then(serde_json::Value::as_str)
            != Some(VOLUME_BINDING_RESOURCE_TYPE)
        {
            return Err(invalid);
        }
        let metadata = value
            .get("metadata")
            .and_then(serde_json::Value::as_object)
            .ok_or(invalid)?;
        let uid = metadata
            .get("uid")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| ResourceUid::parse(value).ok())
            .ok_or(invalid)?;
        let generation = metadata
            .get("generation")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| ResourceGeneration::new(value).ok())
            .ok_or(invalid)?;
        let revision = metadata
            .get("revision")
            .and_then(serde_json::Value::as_u64)
            .map(ZoneRevision::new)
            .ok_or(invalid)?;
        VolumeBindingSpec::admit_owner_ref(
            metadata
                .get("ownerRef")
                .and_then(serde_json::Value::as_str)
                .and_then(|owner| ResourceRef::parse(owner).ok())
                .as_ref(),
        )
        .map_err(|_| invalid)?;
        let spec = value
            .get("spec")
            .and_then(serde_json::Value::as_object)
            .ok_or(invalid)?;
        if spec.get("providerRef").and_then(serde_json::Value::as_str)
            != Some("Provider/volume-virtiofs")
        {
            return Err(invalid);
        }
        if spec.contains_key("provider") {
            // No provider extension path exists for the standard type
            // (U1 admission); an envelope carrying one is never served.
            return Err(invalid);
        }
        let mut base = spec.clone();
        base.remove("providerRef");
        let binding: VolumeBindingSpec = serde_json::from_value(serde_json::Value::Object(base))
            .map_err(|_| invalid)?;
        if metadata
            .get("ownerRef")
            .and_then(serde_json::Value::as_str)
            != Some(binding.volume_ref().to_canonical_string().as_str())
        {
            return Err(invalid);
        }
        Ok(Self::new(binding, uid, generation, revision))
    }

    /// Borrow the strict neutral binding specification.
    pub const fn spec(&self) -> &VolumeBindingSpec {
        &self.binding
    }

    /// Borrow the binding UID the fence is pinned to.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// Return the binding spec generation the fence is pinned to.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    /// Return the Zone-store revision the fence is pinned to.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// The readiness fence this binding reports under (KTD3).
    pub fn fence(&self) -> VolumeBindingReadinessFence {
        VolumeBindingReadinessFence {
            uid: self.uid.clone(),
            generation: self.generation,
            revision: self.revision,
        }
    }

    /// Derive the stable per-Volume worker principal name.
    pub fn worker_principal(&self) -> Result<BoundedToken, VirtiofsBindingError> {
        BoundedToken::parse(format!("vol-{}-vfd", self.binding.volume_ref().name().as_str()))
            .map_err(|_| VirtiofsBindingError::InvalidBinding)
    }

    /// Derive this binding's private socket identity within a Zone.
    pub fn socket_identity(&self, zone: &BoundedToken) -> SocketIdentity {
        SocketIdentity::derive(
            zone,
            self.binding.volume_ref(),
            self.binding.execution_ref(),
        )
    }

    /// Derive the binding-owned virtiofsd Process reference.
    pub fn worker_process_ref(&self) -> Result<ResourceRef, VirtiofsBindingError> {
        derive_child_ref(
            "Process",
            self.binding.volume_ref(),
            self.binding.execution_ref(),
            "worker",
        )
    }

    /// Derive the binding-owned Endpoint reference.
    pub fn endpoint_ref(&self) -> Result<ResourceRef, VirtiofsBindingError> {
        derive_child_ref(
            "Endpoint",
            self.binding.volume_ref(),
            self.binding.execution_ref(),
            "endpoint",
        )
    }

}

fn derive_child_ref(
    resource_type: &str,
    volume_ref: &ResourceRef,
    execution_ref: &ResourceRef,
    role: &str,
) -> Result<ResourceRef, VirtiofsBindingError> {
    let mut hasher = Sha256::new();
    hasher.update(b"d2b/volume-virtiofs/child/v1");
    hasher.update(volume_ref.to_canonical_string().as_bytes());
    hasher.update([0]);
    hasher.update(execution_ref.to_canonical_string().as_bytes());
    hasher.update([0]);
    hasher.update(role.as_bytes());
    let digest = hasher.finalize();
    let suffix = digest[..10]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    ResourceRef::parse(&format!("{resource_type}/vol-vfd-{suffix}"))
        .map_err(|_| VirtiofsBindingError::InvalidBinding)
}

impl fmt::Debug for StoredBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StoredBinding(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(type_name: &str, owner: serde_json::Value, spec: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "type": type_name,
            "metadata": {
                "uid": "123e4567-e89b-42d3-a456-426614174000",
                "generation": 1,
                "revision": 7,
                "ownerRef": owner,
            },
            "spec": spec,
        })
    }

    fn binding_spec() -> serde_json::Value {
        serde_json::json!({
            "providerRef": "Provider/volume-virtiofs",
            "volumeRef": "Volume/work-state",
            "executionRef": "Guest/work-vm",
            "view": "ro-store",
            "access": "read-only",
            "mountPath": "/nix/.ro-store",
        })
    }

    #[test]
    fn stored_binding_parses_a_strictly_neutral_envelope() {
        let stored = StoredBinding::from_resource_spec(&envelope(
            VOLUME_BINDING_RESOURCE_TYPE,
            serde_json::json!("Volume/work-state"),
            binding_spec(),
        ))
        .expect("conformant stored binding");
        assert_eq!(
            stored.spec().volume_ref().to_canonical_string(),
            "Volume/work-state"
        );
        assert_eq!(stored.spec().mount_path(), "/nix/.ro-store");
        assert_eq!(stored.generation().get(), 1);
        assert_eq!(stored.revision().get(), 7);
        assert_eq!(
            stored.fence().uid.to_canonical_string(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000")
                .expect("uid")
                .to_canonical_string()
        );
    }

    #[test]
    fn any_provider_extension_rejects_the_envelope() {
        // The standard catalog admits no provider extension path for the
        // neutral type (U1); the old Export schema identity never
        // re-enters through the binding envelope.
        for schema_id in [
            "volume-virtiofs.d2bus.org/virtiofs.d2bus.org.Export/spec",
            "volume-virtiofs.d2bus.org/VolumeBinding/spec",
        ] {
            let mut extended = envelope(
                VOLUME_BINDING_RESOURCE_TYPE,
                serde_json::json!("Volume/work-state"),
                binding_spec(),
            );
            extended["spec"]["provider"] = serde_json::json!({
                "schemaId": schema_id,
                "schemaVersion": "1.0",
                "settings": {},
            });
            assert!(StoredBinding::from_resource_spec(&extended).is_err());
        }

        let mut foreign_owner = envelope(
            VOLUME_BINDING_RESOURCE_TYPE,
            serde_json::json!("Guest/work-vm"),
            binding_spec(),
        );
        foreign_owner["metadata"]["ownerRef"] = serde_json::json!("Guest/work-vm");
        assert!(StoredBinding::from_resource_spec(&foreign_owner).is_err());
    }

    #[test]
    fn the_envelope_never_carries_attachment_settings() {
        let mut tuned = envelope(
            VOLUME_BINDING_RESOURCE_TYPE,
            serde_json::json!("Volume/work-state"),
            binding_spec(),
        );
        tuned["spec"]["threadPoolSize"] = serde_json::json!(2);
        assert!(StoredBinding::from_resource_spec(&tuned).is_err());
    }

    #[test]
    fn worker_and_endpoint_children_keep_distinct_stable_refs() {
        let stored = StoredBinding::from_resource_spec(&envelope(
            VOLUME_BINDING_RESOURCE_TYPE,
            serde_json::json!("Volume/work-state"),
            binding_spec(),
        ))
        .expect("conformant stored binding");
        assert_ne!(
            stored.worker_process_ref().unwrap(),
            stored.endpoint_ref().unwrap()
        );
        assert_eq!(
            stored.worker_process_ref().unwrap(),
            stored.worker_process_ref().unwrap()
        );
    }
}
