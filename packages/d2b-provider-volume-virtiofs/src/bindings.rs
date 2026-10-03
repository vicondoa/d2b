//! The neutral `VolumeBinding` resource as `volume-virtiofs` serves it.
//!
//! The Volume side mints one binding per Volume / execution-target /
//! named-view relationship (KTD1). volume-virtiofs observes stored
//! binding envelopes, resolves the named view dependency-only, and never
//! writes a Volume row.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use d2b_contracts_resource::v3::{
    ResourceGeneration, ResourceRef, ResourceUid, ZoneRevision,
    execution_policy::BoundedToken,
    resource_status::StatusCode,
    volume::{SourceKind, ViewSpec, VolumeSpec},
    volume_binding::{
        VolumeBindingReadinessFence, VolumeBindingSpec, VolumeBindingStatusResource,
    },
};

use crate::error::VirtiofsBindingError;
use crate::socket_path::derive_serving_socket_path;

/// The standard ResourceType name this Provider serves (canonical contract).
pub use d2b_contracts_resource::v3::volume_binding::VOLUME_BINDING_RESOURCE_TYPE;

/// The finalizer volume-virtiofs adds to each VolumeBinding, and to
/// nothing else.
pub const VOLUME_BINDING_FINALIZER: &str = "volume-virtiofs.d2bus.org/volume-binding";

/// Admit a view's relative path, or refuse it before it reaches a
/// composition step.
///
/// A view path is joined onto a declared root, so an absolute path, a
/// `.`/`..` component, or an empty interior component is refused here
/// rather than being normalized into a sibling of the root it belongs
/// under. An empty path names the root itself, which is the one legal
/// empty spelling.
fn admitted_view_path(view: &ViewSpec) -> Result<String, VirtiofsBindingError> {
    let path = view.path();
    if path.is_empty() {
        return Ok(String::new());
    }
    if path.starts_with('/')
        || path.contains('\0')
        || path
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(VirtiofsBindingError::InvalidBinding);
    }
    Ok(path.to_owned())
}

/// The source a binding's serving worker realizes, derived from the
/// admitted binding and the Volume it names.
///
/// The value is a LOCATOR, not a host path: it names the declared
/// storage row or the broker-managed store-view generation the view lives
/// in, plus the view's own relative path, and the composing side resolves
/// it against its own verified bundle. Nothing here is read back out of a
/// launch argument, and nothing here names a Device, a Guest runtime
/// directory, or a socket (R17-R20).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServingSource {
    /// A Volume whose source kind names a declared storage root.
    ///
    /// The view root is `<declared row root>/<volume name>/<view path>`,
    /// composed by the side that holds the trusted declaration. A source
    /// that names no storage row admits no view at all: serving an
    /// unnamed subtree is exactly the case a refusal exists for.
    DeclaredStorageRoot {
        /// The storage path row the Volume's own source policy names.
        source_policy: BoundedToken,
        /// The volume name the row's root is composed with.
        volume_name: BoundedToken,
        /// The view's relative path under that root; empty names the root.
        view_path: String,
    },
    /// A closure-sourced Volume.
    ///
    /// The bytes are the broker-managed per-consumer store-view farm, never
    /// a path into the shared content store: the farm holds hardlinks, and
    /// the shared store's own inodes are never a mutation target.
    ///
    /// No generation is named here. The generation is the broker's to pin,
    /// and it pins it on the export the publication runs under; a Provider
    /// that carried its own copy would be carrying a number it cannot
    /// enforce and that nothing would compare it against.
    ClosureStoreView {
        /// The volume name the farm is keyed by.
        volume_name: BoundedToken,
        /// The view's relative path under the farm's live tree.
        view_path: String,
    },
}

impl ServingSource {
    /// The view's relative path inside whichever root this source names.
    pub fn view_path(&self) -> &str {
        match self {
            Self::DeclaredStorageRoot { view_path, .. }
            | Self::ClosureStoreView { view_path, .. } => view_path,
        }
    }

    /// The volume name this source was derived for.
    pub const fn volume_name(&self) -> &BoundedToken {
        match self {
            Self::DeclaredStorageRoot { volume_name, .. }
            | Self::ClosureStoreView { volume_name, .. } => volume_name,
        }
    }
}

impl fmt::Display for ServingSource {
    /// Renders the source CLASS, never its material: a log line, an audit
    /// record, or an error never carries a root, a store path, or a
    /// generation id.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::DeclaredStorageRoot { .. } => "declared-storage-root",
            Self::ClosureStoreView { .. } => "closure-store-view",
        })
    }
}

impl Serialize for ServingSource {
    /// Serializes the source CLASS, exactly as [`fmt::Display`] does.
    ///
    /// The worker plan is a public, serializable type that reaches logs
    /// and audit records, so a serialized source must not carry the
    /// storage row, the volume name, the view path, or the generation it
    /// resolves to. The composing side already holds the locator it was
    /// handed in the plan itself.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::DeclaredStorageRoot { .. } => "declared-storage-root",
            Self::ClosureStoreView { .. } => "closure-store-view",
        })
    }
}

/// The opaque identity of one binding's private listening socket.
///
/// The socket path is a generated implementation detail of this
/// Provider. It is never a spec field, a status field, an audit field,
/// or CLI output. Only this digest is public, and the effect adapter
/// alone derives the private path it stands for.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SocketIdentity([u8; 32]);

impl SocketIdentity {
    /// Derive the identity of one relationship's socket.
    ///
    /// The digest covers the whole relationship - the Zone, the source, the
    /// consumer, and the NAMED VIEW. Two relationships that share a source
    /// and a consumer but name different views are two exports with two
    /// private sockets, so a worker can never be handed another view's
    /// socket, and a helper that restarted re-derives byte-identical output
    /// for its own relationship.
    ///
    /// [`StoredBinding::socket_identity`] and
    /// [`SocketIdentity::derive`] are the same function, so the serving
    /// side and the side that composes the launch can never disagree about
    /// which socket a binding stands for.
    pub fn derive(
        zone: &BoundedToken,
        volume_ref: &ResourceRef,
        execution_ref: &ResourceRef,
        view: &BoundedToken,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"d2b/volume-virtiofs/binding-socket/v3");
        hasher.update(zone.as_str().as_bytes());
        hasher.update([0u8]);
        hasher.update(volume_ref.to_canonical_string().as_bytes());
        hasher.update([0u8]);
        hasher.update(execution_ref.to_canonical_string().as_bytes());
        hasher.update([0u8]);
        hasher.update(view.as_str().as_bytes());
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

    /// Bind the serving identity of one COMMITTED manager row (KTD3).
    ///
    /// A pass that just committed a canonical row holds the manager's own
    /// handle for it - the durable uid, the row generation, and the exact
    /// spec bytes it committed under - and this is how that handle becomes a
    /// serving identity. The manager plane carries no Zone revision of its
    /// own and maps the row generation onto the wire revision (KTD8), so the
    /// fence's observation revision is exactly the committed generation; a
    /// projection pinned to any other ordinal would either claim an
    /// observation the row never held or fall behind the row it describes.
    pub fn from_committed_row(
        binding: VolumeBindingSpec,
        uid: ResourceUid,
        generation: ResourceGeneration,
    ) -> Self {
        Self::new(binding, uid, generation, ZoneRevision::new(generation.get()))
    }

    /// Parse a stored VolumeBinding resource envelope.
    ///
    /// The envelope must be a `VolumeBinding` owned by an existing
    /// Volume and served by this Provider, and it must be strictly
    /// neutral: the standard catalog admits no provider extension path
    /// for the type, so a `spec.provider` block -- legacy schema id
    /// or otherwise -- is rejected, and the envelope never carries
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
            != Some(crate::PROVIDER_REF)
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

    /// The fenced public status projection for one observation of this
    /// binding (KTD3).
    ///
    /// The projection is the controller's only public output, and its fence
    /// is derived from this binding's own identity, never from a caller: the
    /// UID pins reassignment, the generation pins spec changes, and the
    /// revision names the row revision the evidence was observed at. The
    /// manager plane carries no Zone revision and maps the row generation
    /// onto the wire revision (KTD8), so the caller passes that same row
    /// revision in [`StoredBinding::new`] and the projection's revision is
    /// exactly the revision of the row it describes. Currency stays a
    /// read-side decision ([`VolumeBindingStatusResource::readiness_is_current`]):
    /// a projection authored under any other identity, or ahead of the
    /// stored revision, is never current.
    pub fn status_projection(
        &self,
        ready: bool,
        reason: Option<VirtiofsBindingError>,
    ) -> VolumeBindingStatusResource {
        VolumeBindingStatusResource {
            ready,
            fence: self.fence(),
            reason: reason.map(|reason| {
                StatusCode::parse(reason.code())
                    .expect("frozen binding error codes are valid status codes")
            }),
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
            self.binding.view(),
        )
    }

    /// Derive one relationship's private socket identity from its parts.
    ///
    /// The composing side holds the same four facts the binding does, and
    /// deriving through this entry point instead of a second copy of the
    /// digest is what keeps the two sides from disagreeing about which
    /// socket a binding stands for.
    pub fn socket_identity_for(
        zone: &BoundedToken,
        volume_ref: &ResourceRef,
        execution_ref: &ResourceRef,
        view: &BoundedToken,
    ) -> SocketIdentity {
        SocketIdentity::derive(zone, volume_ref, execution_ref, view)
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

    /// The source this binding's serving worker realizes.
    ///
    /// Derived from the admitted binding and the Volume it names, and
    /// from nothing else: a Guest with no Device children derives exactly
    /// the source a Guest with children derives (AE6). A source kind this
    /// Provider cannot realize - a block image, a tmpfs, or a local path
    /// naming no storage row - is refused instead of served from a
    /// subtree the graph never admitted.
    pub fn serving_source(
        &self,
        volume: &VolumeSpec,
        view: &ViewSpec,
    ) -> Result<ServingSource, VirtiofsBindingError> {
        let view_path = admitted_view_path(view)?;
        let volume_name = BoundedToken::parse(self.binding.volume_ref().name().as_str())
            .map_err(|_| VirtiofsBindingError::InvalidBinding)?;
        match volume.source().settings().kind() {
            SourceKind::LocalPath => {
                let policy = volume
                    .source()
                    .settings()
                    .source_policy_id()
                    .ok_or(VirtiofsBindingError::SourceKindUnsupported)?;
                Ok(ServingSource::DeclaredStorageRoot {
                    source_policy: policy.clone(),
                    volume_name,
                    view_path,
                })
            }
            SourceKind::NixClosure => Ok(ServingSource::ClosureStoreView {
                volume_name,
                view_path,
            }),
            SourceKind::BlockImage | SourceKind::Tmpfs => {
                Err(VirtiofsBindingError::SourceKindUnsupported)
            }
        }
    }

    /// The private socket this binding's worker binds, as an opaque
    /// identity.
    ///
    /// Only the Zone token and this binding's own relationship reach the
    /// digest, so it is stable across a helper restart and distinct
    /// between two bindings that share a volume or a consumer.
    pub fn serving_socket(&self, zone: &BoundedToken) -> SocketIdentity {
        self.socket_identity(zone)
    }

    /// The private socket path this binding's worker binds.
    ///
    /// The path is derived from the binding's own relationship under the
    /// broker-owned runtime root the launch composes. No Guest row, no
    /// Device row, and no parsed launch argument reaches it, and a root
    /// that cannot be fenced with a component comparison or cannot hold a
    /// socket address is refused rather than normalized.
    pub fn serving_socket_path(
        &self,
        zone: &BoundedToken,
        runtime_root: &Path,
    ) -> Result<PathBuf, VirtiofsBindingError> {
        let volume =
            BoundedToken::parse(self.binding.volume_ref().name().as_str().to_owned())
                .map_err(|_| VirtiofsBindingError::InvalidBinding)?;
        let guest =
            BoundedToken::parse(self.binding.execution_ref().name().as_str().to_owned())
                .map_err(|_| VirtiofsBindingError::InvalidBinding)?;
        derive_serving_socket_path(runtime_root, zone, &volume, &guest)
            .map_err(|_| VirtiofsBindingError::ServingSocketPathUnresolved)
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
            "presentation": {
                "presentation": "filesystem",
                "destination": "/nix/.ro-store",
            },
            "slot": "store",
            "source": {
                "admittedRights": ["consume"],
                "arbitration": "shared",
                "realizedFacets": ["filesystem-presentation"],
            },
        })
    }

    /// The same relationship presented as a block device: the committed row
    /// carries its device slot and no destination at all.
    fn block_binding_spec() -> serde_json::Value {
        serde_json::json!({
            "providerRef": "Provider/volume-virtiofs",
            "volumeRef": "Volume/work-state",
            "executionRef": "Guest/work-vm",
            "view": "raw-store",
            "access": "read-only",
            "presentation": {
                "presentation": "block-device",
                "deviceSlot": 1,
            },
            "slot": "store",
            "source": {
                "admittedRights": ["consume"],
                "arbitration": "shared",
                "realizedFacets": ["consumer-device-slot"],
            },
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
        assert_eq!(stored.spec().presentation().destination(), Some("/nix/.ro-store"));
        assert_eq!(stored.spec().slot().as_str(), "store");
        assert_eq!(stored.generation().get(), 1);
        assert_eq!(stored.revision().get(), 7);
        assert_eq!(
            stored.fence().uid.to_canonical_string(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000")
                .expect("uid")
                .to_canonical_string()
        );
    }

    /// A block-device attachment is a committed row like any other: the stored
    /// envelope parses it, and the parsed row reports its device slot with no
    /// destination to invent for it.
    #[test]
    fn a_block_device_attachment_parses_as_a_committed_row_with_its_device_slot() {
        let stored = StoredBinding::from_resource_spec(&envelope(
            VOLUME_BINDING_RESOURCE_TYPE,
            serde_json::json!("Volume/work-state"),
            block_binding_spec(),
        ))
        .expect("a block presentation is a representable committed row");
        assert_eq!(stored.spec().presentation().device_slot(), Some(1));
        assert_eq!(stored.spec().presentation().destination(), None);
        assert_eq!(stored.spec().view().as_str(), "raw-store");
        assert_eq!(stored.spec().slot().as_str(), "store");
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
