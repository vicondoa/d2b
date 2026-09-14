//! The production Process effects: the daemon's side of the family's effect
//! port.
//!
//! The Process family crate declares the port; this module implements it over
//! the composed fixed Process Providers and the daemon-owned state they read -
//! the committed Provider and Guest-owner identities the plane publishes, the
//! trusted bundle's intents and projected site artifacts, and the daemon's own
//! runtime roots. Nothing here is visible to the family crate.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::process::{EphemeralProcessSpec, ProcessClass, ProcessSpec};
use d2b_contracts_resource::v3::{
    ResourceRef, ResourceSpec, ResourceUid, SchemaFingerprint, ZoneId, ZoneRevision,
};
use d2b_process_conformance::{AdoptionCandidate, ProcessIdentityDigest};
use d2b_provider_process::{
    CommittedProviderIdentitySource, DeviceWorkerFamily, DeviceWorkerLaunch, GuestOwnerIdentitySource,
    ProcessDriverEffects, ProcessFamilySpec, ProcessResourceIdentity, ProviderAdoption,
    ProviderLiveness, device_worker_family, device_worker_vm, resolve_guest_owner_uid,
    resource_uid_from_bytes,
};
use d2b_resource_runtime::context::ResourceContext;
use d2b_resource_runtime::identity::ResourceKey;

use crate::process_provider_runtime::{ProcessResourceContext, ProductionProcessProviders};

/// The video sidecar posture fence: the declared row's template and the
/// owning Device's `videoNvidiaDecode` setting are one decision, so they must
/// agree. The NVIDIA template with the setting off (a stale or hand-authored
/// row) and the plain template with the setting on (the setting silently
/// dropped - the regression this fence exists for) both refuse by name. The
/// posture itself - the bound device nodes - comes from the row's template
/// through the broker's posture table, never from the setting.
fn video_nvidia_posture(
    template: &str,
    settings: &d2b_provider_device_gpu::GpuSettings,
) -> Result<(), &'static str> {
    if settings.video_nvidia_decode != (template == "video-worker-nvidia") {
        return Err("device-worker-nvidia-posture-mismatch");
    }
    Ok(())
}

/// The host Wayland socket the GPU sidecar renders into.
///
/// The trusted bundle projects it (`site.json`, emitted from the site's own
/// `d2b.site.waylandUser` / `waylandDisplay`), so the daemon never derives
/// `/run/user/<uid>/...` itself: the daemon's own `/run/user` is its runtime
/// directory, not the session user's. `None` - a bundle that predates the
/// artifact, or a site without a Wayland session - keeps the slot unbound so
/// the GPU launch refuses with its own closed code instead of running
/// against a path no trusted artifact names.
fn device_worker_wayland_sock(
    site: Option<&d2b_core::site::SiteJson>,
) -> Option<std::path::PathBuf> {
    site.and_then(|site| site.wayland_socket())
        .map(std::path::PathBuf::from)
}

/// The GPU worker's Wayland input, refused by name when the bundle does not
/// project one.
fn gpu_worker_wayland_sock(
    site: Option<&d2b_core::site::SiteJson>,
) -> Result<std::path::PathBuf, &'static str> {
    device_worker_wayland_sock(site).ok_or("device-worker-wayland-sock-unbound")
}

/// One per-VM device socket under the daemon runtime root
/// (`/run/d2b/vms/<vm>/<name>`), the convention the guest VMM's
/// `--tpm socket=` / `--gpu socket=` / `--vhost-user-media socket=`
/// arguments name (see `nixos-modules/vm-options.nix` and
/// `packages/d2b-provider-device-tpm/nix/guest.nix`).
fn device_runtime_socket(
    socket_runtime_dir: &std::path::Path,
    vm_name: &str,
    file_name: &str,
) -> std::path::PathBuf {
    socket_runtime_dir.join("vms").join(vm_name).join(file_name)
}

/// The per-VM video-decoder socket (`/run/d2b-video/<vm>/video.sock`): the
/// video module's own `RuntimeDirectory` and the guest's
/// `--vhost-user-media socket=` argument name it, so the video runtime root is
/// a sibling of the daemon's runtime root.
fn video_runtime_socket(
    socket_runtime_dir: &std::path::Path,
    vm_name: &str,
) -> Option<std::path::PathBuf> {
    let root = socket_runtime_dir.parent()?.join("d2b-video");
    Some(root.join(vm_name).join("video.sock"))
}

/// Production effects over the composed fixed Providers.
pub(crate) struct ProductionProcessDriverEffects {
    providers: Arc<ProductionProcessProviders>,
    /// Committed Provider identities (KTD7), wired by the plane's
    /// construction path from the composition-resolved snapshot.
    committed_provider_identities: Option<Arc<dyn CommittedProviderIdentitySource>>,
    /// Guest-owner durable identities (KTD7), wired by the plane's
    /// construction path from the pre-v3 plane that owns `Guest` rows.
    guest_owner_identities: Option<Arc<dyn GuestOwnerIdentitySource>>,
}

impl ProductionProcessDriverEffects {
    pub(crate) fn new(providers: Arc<ProductionProcessProviders>) -> Self {
        Self {
            providers,
            committed_provider_identities: None,
            guest_owner_identities: None,
        }
    }

    /// Attach the committed Provider identity source (KTD7). Unwired effects
    /// keep the driver-derived (unbound) identity, which the provider ticket
    /// path refuses closed.
    pub(crate) fn with_committed_provider_identities(
        mut self,
        source: Arc<dyn CommittedProviderIdentitySource>,
    ) -> Self {
        self.committed_provider_identities = Some(source);
        self
    }

    /// Attach the Guest-owner identity source (KTD7). Unwired effects leave a
    /// Guest-owned row's owner uid unbound, so the Cloud Hypervisor launch
    /// refuses closed instead of inventing an identity.
    pub(crate) fn with_guest_owner_identities(
        mut self,
        source: Arc<dyn GuestOwnerIdentitySource>,
    ) -> Self {
        self.guest_owner_identities = Some(source);
        self
    }

    /// The provider-layer context for one row: the committed
    /// controller-provider identity (KTD7), the owning Guest's durable uid
    /// for a guest-owned row, and the catalog-bound Guest setup descriptor
    /// digest.
    async fn resource_context<'a>(
        &self,
        identity: &'a ProcessResourceIdentity,
    ) -> ProcessResourceContext<'a> {
        let guest_owner_uid =
            resolve_guest_owner_uid(self.guest_owner_identities.as_deref(), identity).await;
        process_resource_context(
            identity,
            self.committed_provider_identities.as_deref(),
            guest_owner_uid.as_ref(),
            |zone, guest| self.providers.guest_setup_descriptor_digest(zone, guest),
        )
    }

    /// The state directory backing the Device's controller-created TPM state
    /// Volume: the controller-created Volume's name under the trusted per-VM
    /// `path:swtpm-state:<vm>` storage row
    /// (`packages/d2b-provider-volume-local/nix/storage-json.nix`). Both the
    /// name and the root come from trusted artifacts - the Volume body is the
    /// TPM Provider's own builder, and the root is the bundle's storage row -
    /// so a worker can never be pointed at a path no trusted artifact names.
    fn device_state_dir(
        &self,
        zone: &ZoneId,
        device_uid: &ResourceUid,
        device_ref: &ResourceRef,
        execution_ref: &str,
        vm_name: &str,
    ) -> Result<std::path::PathBuf, &'static str> {
        let execution_ref =
            ResourceRef::parse(execution_ref).map_err(|_| "device-worker-execution-ref-invalid")?;
        let document = d2b_provider_device_tpm::build_tpm_state_volume_resource(
            device_uid,
            device_ref,
            zone.as_str(),
            &execution_ref,
        )
        .map_err(|_| "device-worker-state-volume-unresolved")?;
        let volume_name = document
            .pointer("/metadata/name")
            .and_then(serde_json::Value::as_str)
            .ok_or("device-worker-state-volume-unresolved")?;
        let storage_path_id = format!("path:swtpm-state:{vm_name}");
        self.providers
            .bundle()
            .resolve_volume_view_root(&storage_path_id, volume_name, "")
            .ok_or("device-worker-state-dir-unresolved")
    }

    /// The owning Device's declared GPU settings (the closed
    /// `device-gpu.d2bus.org` Device extension); a Device that declares none
    /// keeps the Provider's own bounded default.
    async fn device_gpu_settings(
        &self,
        ctx: &mut ResourceContext,
        owner_key: &ResourceKey,
    ) -> Result<d2b_provider_device_gpu::GpuSettings, &'static str> {
        let Some(row) = ctx
            .get(owner_key)
            .await
            .map_err(|_| "device-worker-device-row-unreadable")?
        else {
            return Err("device-worker-device-row-missing");
        };
        decode_device_gpu_settings(&row.spec)
    }
}

/// Decode the GPU settings declared by one Device row's stored spec.
///
/// Only an absent Provider extension decodes to the Provider's bounded
/// default. A present settings payload that does not decode as the closed
/// `device-gpu.d2bus.org` extension refuses with its own code instead: folding
/// it into the default made an undecodable declaration indistinguishable from
/// a Device that declares nothing, and the default's context classes
/// (including `CrossDomain`) are wider than anything the Device declared.
fn decode_device_gpu_settings(
    stored_spec: &[u8],
) -> Result<d2b_provider_device_gpu::GpuSettings, &'static str> {
    let envelope = serde_json::from_slice::<ResourceSpec>(stored_spec)
        .map_err(|_| "device-worker-device-row-unreadable")?;
    let Some(provider) = envelope.provider() else {
        return Ok(d2b_provider_device_gpu::GpuSettings::default());
    };
    let settings = provider.settings().to_canonical_bytes();
    serde_json::from_slice::<d2b_provider_device_gpu::GpuSettings>(&settings)
        .map_err(|_| "device-worker-gpu-settings-invalid")
}

/// Build the provider-layer context for one row: the committed
/// controller-provider identity (KTD7) first, then the owning Guest's
/// durable uid and the catalog-bound `Guest` setup descriptor digest for a
/// guest-owned row.
///
/// The digest is the old runner's `set_guest_descriptor_digests` input and the
/// private guest VMM intent lookup (`find_guest_vmm_intent`) refuses a ticket
/// without it (`provider-ticket:guest-descriptor-unbound`), so a
/// controller-minted `Process/<guest>-vmm` row cannot launch end to end until
/// the bundle's descriptor digest is bound. The owner uid is the linkage the
/// old composer read from the durable row (the broker refuses a Cloud
/// Hypervisor launch without it). Rows without a Guest owner bind nothing, and
/// a Guest the plane or bundle does not retain stays unbound - the ticket
/// path still refuses closed.
pub(crate) fn process_resource_context<'a>(
    identity: &'a ProcessResourceIdentity,
    committed_provider_identities: Option<&dyn CommittedProviderIdentitySource>,
    guest_owner_uid: Option<&ResourceUid>,
    guest_descriptor_digest: impl Fn(&ZoneId, &ResourceRef) -> Option<SchemaFingerprint>,
) -> ProcessResourceContext<'a> {
    let context =
        bind_committed_controller_provider_identity(identity, committed_provider_identities);
    let Some(guest) = identity
        .launch
        .owner_ref()
        .filter(|owner| owner.resource_type().as_str() == "Guest")
    else {
        return context;
    };
    let context = match guest_owner_uid {
        Some(guest_owner_uid) => context.with_owner_uid(Some(guest_owner_uid.clone())),
        None => context,
    };
    match guest_descriptor_digest(&identity.zone, guest) {
        Some(digest) => context.with_guest_descriptor_digest(Some(&digest)),
        None => context,
    }
}

/// Build the borrowed provider-layer context of one row from its identity
/// alone. The ticket machinery is entirely inside the provider layer; the
/// driver never assembles a ticket.
///
/// A free function rather than an inherent method because
/// [`ProcessResourceIdentity`] lives in `d2b-process` and no inherent impl can
/// be written for a foreign type.
pub(crate) fn identity_resource_context(
    identity: &ProcessResourceIdentity,
) -> ProcessResourceContext<'_> {
    ProcessResourceContext::new(
        identity.zone.clone(),
        &identity.resource_ref,
        &identity.resource_uid,
        identity.resource_generation,
        // The new store has no zone-wide commit revision: the durable
        // revision of a row is its generation. The launch ticket requires
        // a non-zero resource revision, and the provider identity fence
        // compares generations, not revisions, so the row generation is
        // the honest binding here.
        ZoneRevision::new(identity.resource_generation.get()),
        &identity.provider_ref,
        identity.controller_generation,
        identity.launch.target_ref().cloned(),
    )
    .with_guest_execution(identity.guest_execution.as_ref())
    .with_lifecycle_identity(
        identity.zone_uid.clone(),
        identity.policy_revision,
        identity.provider_assignment_generation,
    )
    .with_owner_ref(identity.launch.owner_ref().cloned())
    .with_owner_uid(identity.launch.owner_uid().cloned())
    .with_provider_identity(
        identity.controller_provider_uid.as_ref(),
        identity.controller_provider_generation,
    )
    .with_worker_launch(identity.worker_launch.clone())
    .with_device_worker_launch(identity.device_worker_launch.clone())
    .with_launch_identity(identity.launch.clone())
}

/// Bind the committed Provider row's identity (KTD7) onto one controller
/// row's provider context: a controller Process owned by a `Provider` takes
/// that Provider's committed uid/generation when the driver left the identity
/// unbound. Every other row - another process class, another owner type, an
/// already-bound identity, or a Provider with no committed row - keeps the
/// driver-derived context, so a genuinely missing row still refuses closed.
fn bind_committed_controller_provider_identity<'a>(
    identity: &'a ProcessResourceIdentity,
    source: Option<&dyn CommittedProviderIdentitySource>,
) -> ProcessResourceContext<'a> {
    let context = identity_resource_context(identity);
    if identity.process_class != ProcessClass::Controller
        || identity.controller_provider_uid.is_some()
        || identity.controller_provider_generation.is_some()
    {
        return context;
    }
    let Some(provider_owner) = identity
        .launch
        .owner_ref()
        .filter(|owner| owner.resource_type().as_str() == "Provider")
    else {
        return context;
    };
    match source.and_then(|source| source.committed_provider_identity(provider_owner)) {
        Some((uid, generation)) => context.with_provider_identity(Some(&uid), Some(generation)),
        None => context,
    }
}

#[async_trait::async_trait]
impl ProcessDriverEffects for ProductionProcessDriverEffects {
    async fn launch(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        timeout: Duration,
    ) -> Result<ProcessIdentityDigest, String> {
        let context = self.resource_context(identity).await;
        self.providers
            .launch_resource(context, spec, timeout)
            .await
            .map(|launch| launch.identity)
    }

    async fn launch_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
        timeout: Duration,
    ) -> Result<ProcessIdentityDigest, String> {
        let context = self.resource_context(identity).await;
        self.providers
            .launch_ephemeral_resource(context, spec, timeout)
            .await
            .map(|launch| launch.identity)
    }

    async fn adopt(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.providers
            .adopt_resource(self.resource_context(identity).await, spec)
            .await
    }

    async fn probe(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        self.providers
            .probe_resource(self.resource_context(identity).await, spec)
            .await
    }

    async fn adopt_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.providers
            .adopt_ephemeral_resource(self.resource_context(identity).await, spec)
            .await
    }

    async fn probe_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        self.providers
            .probe_ephemeral_resource(self.resource_context(identity).await, spec)
            .await
    }

    async fn stop(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.providers
            .stop_resource(
                self.resource_context(identity).await,
                spec,
                term_timeout,
                kill_timeout,
            )
            .await
    }

    async fn stop_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.providers
            .stop_ephemeral_resource(
                self.resource_context(identity).await,
                spec,
                term_timeout,
                kill_timeout,
            )
            .await
    }

    async fn stop_stale(
        &self,
        provider_ref: &ResourceRef,
        candidate: &AdoptionCandidate,
    ) -> Result<(), String> {
        self.providers
            .stop_stale_resource(provider_ref, candidate)
            .await
    }

    async fn finalize(&self, identity: &ProcessResourceIdentity) -> Result<(), String> {
        self.providers
            .finalize_resource(self.resource_context(identity).await)
            .await
    }

    fn has_active(
        &self,
        zone: &ZoneId,
        zone_uid: Option<&ResourceUid>,
        resource_ref: &ResourceRef,
    ) -> bool {
        self.providers
            .has_active_resource_in_zone(zone, zone_uid, resource_ref)
    }

    /// Derive the typed launch parameters of one declared Device-owned worker
    /// row (`U17` gap closure).
    ///
    /// The Device Providers declare their worker rows path-free and the
    /// Process spec is argv-free by contract, so the inputs the device argv
    /// generators need come from the three sources the Process controller
    /// owns:
    ///
    /// - the owned `Device` row: its uid keys the controller-created state
    ///   Volume, and its declared Provider settings carry the GPU context
    ///   classes, displays, and EGL/Vulkan flags;
    /// - the trusted declared template: the bundle's Device-worker intent for
    ///   this exact declared row pins the worker binary and the principal the
    ///   sockets belong to, and `device_worker_posture` pins the template's
    ///   closed role posture;
    /// - the daemon's runtime paths: the swtpm state directory backing the
    ///   controller-created state Volume, and the per-VM socket roots under
    ///   the daemon runtime root (the same conventions the guest VMM's
    ///   `--tpm socket=` / `--gpu socket=` / `--vhost-user-media socket=`
    ///   arguments name) - plus the bundle's `site.json`, which projects the
    ///   host Wayland socket the GPU worker renders into.
    ///
    /// Returns `None` for every row that is not one of the declared Device
    /// worker templates. A declared template whose trusted inputs cannot be
    /// resolved refuses the launch (the named code is the diagnosis) instead
    /// of launching bare.
    async fn device_worker_launch(
        &self,
        ctx: &mut ResourceContext,
        identity: &ProcessResourceIdentity,
        spec: &ProcessFamilySpec,
    ) -> Result<Option<DeviceWorkerLaunch>, &'static str> {
        use d2b_provider_process::{GpuWorkerParams, SwtpmFlushParams, SwtpmWorkerParams, VideoWorkerParams};
        let execution = spec.execution();
        let template = execution.template().as_str();
        let family = device_worker_family(template);
        let Some(family) = family else {
            return Ok(None);
        };
        let launch = &identity.launch;
        // Owner fence: these rows are Device-declared children of the Device
        // that owns the physical function, and every path below is derived
        // from that Device.
        let owner_key = ctx
            .owner_key()
            .cloned()
            .ok_or("device-worker-owner-unresolved")?;
        if owner_key.type_name != "Device" {
            return Err("device-worker-owner-not-device");
        }
        let device_ref = launch
            .owner_ref()
            .filter(|owner| owner.resource_type().as_str() == "Device")
            .cloned()
            .ok_or("device-worker-owner-not-device")?;
        let device_uid = ctx
            .owner()
            .and_then(|bytes| resource_uid_from_bytes(bytes).ok())
            .ok_or("device-worker-owner-uid-unresolved")?;
        // A Device-owned worker row declares `executionRef Host/host-system`
        // and no Guest target, so the row's own launch identity names no VM
        // by construction. The coherent VM scope is the owning Device's
        // Guest owner - the same derivation `tpm_device_targets_vm` requires
        // (`Device.metadata.ownerRef == Guest/<vm>`) and the TPM
        // shared-provider effects mint their `VmId` from. A Device with no
        // Guest owner is the genuinely unresolvable case.
        let vm_name = match launch.vm() {
            Some(vm) => vm.to_owned(),
            None => device_worker_vm(ctx, &owner_key).await?,
        };
        // Template fence: the trusted intent must exist for this exact
        // declared row name + template, and the template must belong to a
        // Device Provider's closed posture table.
        let execution_ref = execution.execution_ref().to_canonical_string();
        let user_ref = execution.user_ref().map(ResourceRef::to_canonical_string);
        let domain = match execution
            .domain()
            .unwrap_or(d2b_contracts_resource::v3::execution_policy::ExecutionDomain::System)
        {
            d2b_contracts_resource::v3::execution_policy::ExecutionDomain::System => {
                d2b_core::processes::ProcessExecutionDomain::System
            }
            d2b_contracts_resource::v3::execution_policy::ExecutionDomain::User => {
                d2b_core::processes::ProcessExecutionDomain::User
            }
        };
        let intent = self
            .providers
            .bundle()
            .find_device_worker_intent(
                &identity.resource_ref,
                &execution_ref,
                domain,
                user_ref.as_deref(),
                template,
            )
            .ok_or("device-worker-intent-unresolved")?;
        let Some(posture) = d2b_core::bundle_resolver::device_worker_posture(
            intent.owner_ref.as_deref().unwrap_or_default(),
            template,
        ) else {
            return Err("device-worker-template-refused");
        };
        if !intent.accepts_launch_args {
            return Err("device-worker-template-refused");
        }
        // The socket owner ids the worker asks swtpm for, in the namespace
        // the launch actually runs in: a posture with the ADR 0021
        // single-entry user namespace names the in-namespace identity (`0`,
        // the only id the mapping declares), a posture without one keeps the
        // host principal. Naming the host principal inside its own namespace
        // made swtpm's socket chown fail with EINVAL and the worker exit 1
        // before it bound anything.
        let (socket_uid, socket_gid) = posture.launch_ids(intent.uid, intent.gid);
        let socket_runtime_dir = self.providers.socket_runtime_dir().to_path_buf();
        let params = match family {
            DeviceWorkerFamily::Swtpm => {
                let state_dir = self.device_state_dir(
                    &identity.zone,
                    &device_uid,
                    &device_ref,
                    &execution_ref,
                    &vm_name,
                )?;
                DeviceWorkerLaunch::Swtpm(Box::new(SwtpmWorkerParams {
                    binary_path: intent.binary_path.clone(),
                    vm_name: vm_name.clone(),
                    ctrl_socket_path: state_dir.join("ctrl.sock"),
                    server_socket_path: device_runtime_socket(
                        &socket_runtime_dir,
                        &vm_name,
                        "tpm.sock",
                    ),
                    state_dir,
                    uid: socket_uid,
                    gid: socket_gid,
                    log_level: d2b_provider_device_tpm::SwtpmSettings::default().log_level,
                }))
            }
            DeviceWorkerFamily::SwtpmFlush => {
                let state_dir = self.device_state_dir(
                    &identity.zone,
                    &device_uid,
                    &device_ref,
                    &execution_ref,
                    &vm_name,
                )?;
                DeviceWorkerLaunch::SwtpmFlush(Box::new(SwtpmFlushParams {
                    ioctl_binary_path: intent.binary_path.clone(),
                    vm_name: vm_name.clone(),
                    ctrl_socket_path: state_dir.join("ctrl.sock"),
                }))
            }
            DeviceWorkerFamily::Gpu => {
                let settings = self.device_gpu_settings(ctx, &owner_key).await?;
                // The Wayland socket the sidecar renders into is projected by
                // the trusted bundle from the site's own Wayland session
                // (`d2b.site.waylandUser` / `waylandDisplay`, see
                // `nixos-modules/site-json.nix`). A bundle without the
                // artifact, or a headless site, leaves the slot unbound and
                // the launch refuses with its own code instead of naming a
                // path no trusted artifact names.
                let wayland_sock =
                    gpu_worker_wayland_sock(self.providers.bundle().site.as_ref())?;
                // The family crate cannot depend on the realizer provider
                // crate, so the Device's declared settings travel as the
                // canonical JSON of the provider's own `GpuParams` and the
                // argv seat decodes them back.
                let params = serde_json::to_value(d2b_provider_device_gpu::GpuParams {
                    context_types: settings
                        .context_types
                        .iter()
                        .map(|context| match context {
                            d2b_provider_device_gpu::ContextType::Virgl => {
                                d2b_provider_device_gpu::GpuContextType::Virgl
                            }
                            d2b_provider_device_gpu::ContextType::Virgl2 => {
                                d2b_provider_device_gpu::GpuContextType::Virgl2
                            }
                            d2b_provider_device_gpu::ContextType::CrossDomain => {
                                d2b_provider_device_gpu::GpuContextType::CrossDomain
                            }
                        })
                        .collect(),
                    displays: settings
                        .displays
                        .iter()
                        .map(|display| d2b_provider_device_gpu::GpuDisplayConfig {
                            hidden: display.hidden,
                        })
                        .collect(),
                    egl: settings.egl,
                    vulkan: settings.vulkan,
                })
                .map_err(|_| "device-worker-gpu-settings-invalid")?;
                DeviceWorkerLaunch::Gpu(Box::new(GpuWorkerParams {
                    binary_path: intent.binary_path.clone(),
                    vm_name: vm_name.clone(),
                    socket_path: device_runtime_socket(&socket_runtime_dir, &vm_name, "gpu.sock"),
                    wayland_sock,
                    params,
                }))
            }
            DeviceWorkerFamily::Video => {
                // The declared row's template and the owning Device's
                // `videoNvidiaDecode` setting are one decision (the posture
                // binds the NVIDIA nodes only through the
                // `video-worker-nvidia` template), so a disagreement is a
                // refusal rather than a launch where the setting is silently
                // ignored.
                let settings = self.device_gpu_settings(ctx, &owner_key).await?;
                video_nvidia_posture(template, &settings)?;
                DeviceWorkerLaunch::Video(Box::new(VideoWorkerParams {
                    binary_path: intent.binary_path.clone(),
                    vm_name: vm_name.clone(),
                    socket_path: video_runtime_socket(&socket_runtime_dir, &vm_name)
                        .ok_or("device-worker-video-socket-unresolved")?,
                }))
            }
        };
        Ok(Some(params))
    }


}

#[cfg(test)]
mod tests {
    use super::*;

    use d2b_contracts_resource::v3::process::ProcessClass;
    use d2b_contracts_resource::v3::{
        ControllerGeneration, ResourceGeneration, ResourceUid, SchemaFingerprint,
    };
    use d2b_process_conformance::LaunchIdentity;
    use d2b_provider_process::{LaunchRow, ProcessResourceIdentity, resolve_launch_identity};

    // -- fixtures ------------------------------------------------------------

    /// The committed Provider uid the test source publishes.
    const COMMITTED_PROVIDER_UID: &str = "123e4567-e89b-42d3-a456-426614174010";
    /// The committed Provider generation the test source publishes.
    const COMMITTED_PROVIDER_GENERATION: u64 = 4;
    /// The durable uid the pre-v3 plane publishes for the owning Guest.
    const GUEST_UID: &str = "323e4567-e89b-42d3-a456-426614174001";
    /// The canonical provider reference every test row selects.
    const PROVIDER_REF: &str = "Provider/system-minijail";

    fn zone() -> ZoneId {
        ZoneId::parse("work").expect("zone")
    }

    /// The one canonical launch identity of a row, resolved by the family's
    /// own resolver: the daemon's context builders read only what this
    /// resolves, so every field below is the field the ticket path would see.
    fn launch_identity(
        owner: &str,
        process_name: &str,
        template: &str,
    ) -> LaunchIdentity {
        let owner = ResourceRef::parse(owner).expect("owner ref");
        let execution_ref = ResourceRef::parse("Host/host-system").expect("execution ref");
        resolve_launch_identity(&LaunchRow {
            owner_ref: Some(&owner),
            owner_uid: None,
            execution_ref: &execution_ref,
            process_name,
            template,
            declared_target: None,
        })
        .expect("complete launch identity")
    }

    fn identity(
        resource: &str,
        process_name: &str,
        template: &str,
        owner: &str,
        process_class: ProcessClass,
    ) -> ProcessResourceIdentity {
        ProcessResourceIdentity {
            zone: zone(),
            resource_ref: ResourceRef::parse(resource).expect("resource ref"),
            resource_uid: ResourceUid::parse("423e4567-e89b-42d3-a456-426614174002")
                .expect("resource uid"),
            resource_generation: ResourceGeneration::new(7).expect("generation"),
            process_class,
            provider_ref: ResourceRef::parse(PROVIDER_REF).expect("provider ref"),
            launch: launch_identity(owner, process_name, template),
            zone_uid: None,
            policy_revision: None,
            provider_assignment_generation: None,
            controller_generation: ControllerGeneration::new(1).expect("controller generation"),
            controller_provider_uid: None,
            controller_provider_generation: None,
            guest_execution: None,
            worker_launch: None,
            device_worker_launch: None,
        }
    }

    /// A controller-class row owned by a Provider: the row shape the
    /// controller ticket needs its owner's committed identity for.
    fn controller_identity() -> ProcessResourceIdentity {
        identity(
            "Process/controller",
            "controller",
            "reaction",
            "Provider/network-local",
            ProcessClass::Controller,
        )
    }

    /// A Guest-owned guest-runtime row: its launch targets its owning Guest.
    fn guest_vmm_identity() -> ProcessResourceIdentity {
        identity(
            "Process/acceptance-guest-vmm",
            "acceptance-guest-vmm",
            "cloud-hypervisor-runner",
            "Guest/acceptance-guest",
            ProcessClass::Worker,
        )
    }

    /// Fixed committed Provider rows: the view the plane registry publishes
    /// after bundle ingestion.
    #[derive(Default)]
    struct FixedProviderIdentities(
        std::collections::BTreeMap<String, (ResourceUid, ResourceGeneration)>,
    );

    impl FixedProviderIdentities {
        fn with(mut self, provider_ref: &str, uid: &str, generation: u64) -> Self {
            self.0.insert(
                provider_ref.to_owned(),
                (
                    ResourceUid::parse(uid).expect("provider uid"),
                    ResourceGeneration::new(generation).expect("provider generation"),
                ),
            );
            self
        }
    }

    impl CommittedProviderIdentitySource for FixedProviderIdentities {
        fn committed_provider_identity(
            &self,
            provider_ref: &ResourceRef,
        ) -> Option<(ResourceUid, ResourceGeneration)> {
            self.0.get(&provider_ref.to_canonical_string()).cloned()
        }
    }

    // -- committed controller-provider identity -------------------------------

    /// A controller row owned by a Provider takes the owner's committed
    /// uid/generation into the provider context, so the controller bootstrap
    /// ticket forms instead of refusing with
    /// `provider-controller-provider-identity-missing`.
    #[tokio::test]
    async fn controller_provider_identity_binds_the_committed_provider_row() {
        let identity = controller_identity();
        let source = FixedProviderIdentities::default().with(
            "Provider/network-local",
            COMMITTED_PROVIDER_UID,
            COMMITTED_PROVIDER_GENERATION,
        );
        let context = super::bind_committed_controller_provider_identity(
            &identity,
            Some(&source as &dyn CommittedProviderIdentitySource),
        );
        assert_eq!(
            context.provider_uid.as_ref().map(ResourceUid::as_str),
            Some(COMMITTED_PROVIDER_UID)
        );
        assert_eq!(
            context.provider_generation,
            Some(ResourceGeneration::new(COMMITTED_PROVIDER_GENERATION).expect("generation"))
        );
    }

    /// A Provider the daemon retains no committed row for - and an unwired
    /// production effects value - leaves the identity unbound, so the ticket
    /// path still refuses closed instead of inventing an identity.
    #[tokio::test]
    async fn controller_provider_identity_stays_unbound_without_a_committed_row() {
        let identity = controller_identity();
        let empty = FixedProviderIdentities::default();
        let unretained = super::bind_committed_controller_provider_identity(
            &identity,
            Some(&empty as &dyn CommittedProviderIdentitySource),
        );
        assert_eq!(unretained.provider_uid, None);
        assert_eq!(unretained.provider_generation, None);

        let unwired = super::bind_committed_controller_provider_identity(&identity, None);
        assert_eq!(unwired.provider_uid, None);
        assert_eq!(unwired.provider_generation, None);
    }

    // -- catalog-bound Guest setup descriptor digest -------------------------

    /// The catalog digest the bundle resolves for one guest.
    fn guest_descriptor_digest() -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}", "a".repeat(64))).expect("guest digest")
    }

    /// The launch ticket for a Guest-owned guest-runtime Process must carry
    /// the owner identity the old descriptor composer produced for the same
    /// row: the authored `owner_ref`, the durable owner uid the pre-v3 plane
    /// resolved from it, and the owning Guest as the cross-target selector.
    ///
    /// The manager row cannot carry the linkage for an unconverted owner
    /// (`Guest` stays on the pre-v3 plane), so the process effects resolve the
    /// same durable uid from that plane.
    #[tokio::test]
    async fn guest_vmm_ticket_carries_the_old_descriptor_owner_identity() {
        let guest_ref = ResourceRef::parse("Guest/acceptance-guest").expect("guest ref");
        let guest_uid = ResourceUid::parse(GUEST_UID).expect("guest uid");
        let identity = guest_vmm_identity();
        assert_eq!(
            identity.launch.owner_uid(),
            None,
            "a manager row cannot link an unconverted Guest owner"
        );

        let context =
            super::process_resource_context(&identity, None, Some(&guest_uid), |_, _| None);

        assert_eq!(context.owner_ref.as_ref(), Some(&guest_ref));
        assert_eq!(context.owner_uid.as_ref(), Some(&guest_uid));
        assert_eq!(context.target_ref.as_ref(), Some(&guest_ref));
    }

    /// The old runner bound the bundle's Guest setup descriptor digest for
    /// guest-owned rows (`set_guest_descriptor_digests`); the private guest VMM
    /// intent lookup refuses a ticket without it
    /// (`provider-ticket:guest-descriptor-unbound`), so the descriptor must
    /// reach the provider context.
    #[tokio::test]
    async fn guest_owned_row_binds_the_catalog_guest_descriptor_digest() {
        let identity = guest_vmm_identity();
        let digest = guest_descriptor_digest();
        let consulted = std::cell::Cell::new(false);
        let context = super::process_resource_context(&identity, None, None, |zone, guest| {
            consulted.set(true);
            assert_eq!(zone.as_str(), "work");
            assert_eq!(guest.name().as_str(), "acceptance-guest");
            Some(digest.clone())
        });
        assert!(
            consulted.get(),
            "a guest-owned row must resolve its descriptor from the bundle"
        );
        assert_eq!(context.guest_descriptor_digest.as_ref(), Some(&digest));
    }

    /// Non-guest rows never consult the bundle descriptor source, so the
    /// context keeps the descriptor slot unbound.
    #[tokio::test]
    async fn non_guest_rows_keep_the_guest_descriptor_digest_unbound() {
        let identity = controller_identity();
        let context = super::process_resource_context(&identity, None, None, |_, _| {
            panic!("a Provider-owned row must not consult a Guest descriptor")
        });
        assert_eq!(context.guest_descriptor_digest, None);
    }

    /// A Guest the bundle retains no descriptor for stays unbound - the
    /// ticket path still refuses closed instead of inventing a digest.
    #[tokio::test]
    async fn missing_catalog_descriptor_keeps_the_guest_digest_unbound() {
        let identity = guest_vmm_identity();
        let context = super::process_resource_context(&identity, None, None, |_, _| None);
        assert_eq!(context.guest_descriptor_digest, None);
    }

    // -- Device-worker derivation --------------------------------------------

    /// A Device that declares GPU settings keeps them, a Device that declares
    /// none keeps the Provider's bounded default, and a present payload that
    /// does not decode refuses with its own code instead of silently becoming
    /// the default - whose `CrossDomain` context class the Device never
    /// declared.
    #[test]
    fn device_gpu_settings_refuse_an_undecodable_declaration() {
        let declared = br#"{"providerRef":"Provider/device-gpu","provider":{"schemaId":"device-gpu.d2bus.org/Device/spec","schemaVersion":"1.0","settings":{"contextTypes":["virgl"],"displays":[{"hidden":false}],"egl":false,"vulkan":false}}}"#;
        let settings =
            super::decode_device_gpu_settings(declared).expect("declared settings decode");
        assert_eq!(
            settings.context_types,
            vec![d2b_provider_device_gpu::ContextType::Virgl]
        );
        assert!(!settings.egl, "the declared setting wins over the default");
        assert!(
            super::decode_device_gpu_settings(br#"{"providerRef":"Provider/device-gpu"}"#)
                .expect("absent settings keep the default")
                == d2b_provider_device_gpu::GpuSettings::default(),
            "a Device that declares nothing keeps the Provider default"
        );
        let undecodable = br#"{"providerRef":"Provider/device-gpu","provider":{"schemaId":"device-gpu.d2bus.org/Device/spec","schemaVersion":"1.0","settings":{"contextTypes":["bogus"]}}}"#;
        assert_eq!(
            super::decode_device_gpu_settings(undecodable),
            Err("device-worker-gpu-settings-invalid"),
            "an undecodable declaration is never read as absent"
        );
        assert_eq!(
            super::decode_device_gpu_settings(b"{not-json"),
            Err("device-worker-device-row-unreadable")
        );
    }

    /// The owning Device's `videoNvidiaDecode` setting and the declared
    /// video row's template are one decision: the NVIDIA posture binds its
    /// device nodes only through the `video-worker-nvidia` template, so each
    /// disagreement refuses by name instead of launching a sidecar where the
    /// setting (on the plain template) or the template (on a Device that
    /// turned the setting off) is silently ignored.
    #[test]
    fn video_nvidia_posture_refuses_a_setting_template_mismatch() {
        let mut settings = d2b_provider_device_gpu::GpuSettings::default();
        assert!(!settings.video_nvidia_decode, "the default posture is plain");
        assert_eq!(super::video_nvidia_posture("video-worker", &settings), Ok(()));
        assert_eq!(
            super::video_nvidia_posture("video-worker-nvidia", &settings),
            Err("device-worker-nvidia-posture-mismatch"),
            "the NVIDIA template without its setting is a refusal"
        );

        settings.video_nvidia_decode = true;
        assert_eq!(
            super::video_nvidia_posture("video-worker-nvidia", &settings),
            Ok(())
        );
        assert_eq!(
            super::video_nvidia_posture("video-worker", &settings),
            Err("device-worker-nvidia-posture-mismatch"),
            "the setting on the plain template is never a silent no-op"
        );
    }

    // -- Device-worker Wayland projection ------------------------------------
    //
    // The host Wayland socket is trusted bundle data (`site.json`), never a
    // daemon-derived path: the reader resolves the projected value and refuses
    // by name when the bundle carries none, so a bundle that predates the
    // artifact - or a site without a Wayland session - cannot launch the GPU
    // worker against an invented socket.

    #[test]
    fn gpu_worker_wayland_sock_reads_the_projected_site_and_refuses_without_it() {
        let site = d2b_core::site::SiteJson {
            schema_version: "v1".to_owned(),
            wayland_socket: Some("/run/user/1001/wayland-7".to_owned()),
        };
        assert_eq!(
            super::gpu_worker_wayland_sock(Some(&site)),
            Ok(std::path::PathBuf::from("/run/user/1001/wayland-7")),
            "the socket is exactly the bundle-projected value"
        );

        let headless = d2b_core::site::SiteJson {
            schema_version: "v1".to_owned(),
            wayland_socket: None,
        };
        assert_eq!(
            super::gpu_worker_wayland_sock(Some(&headless)),
            Err("device-worker-wayland-sock-unbound")
        );
        assert_eq!(
            super::gpu_worker_wayland_sock(None),
            Err("device-worker-wayland-sock-unbound"),
            "a bundle that predates site.json keeps the GPU launch refused by name"
        );
    }
}
