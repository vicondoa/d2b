//! The Process family through the plane's registry.
//!
//! Both member types register from the family's own declarations, the
//! descriptor carries the decoder the manager wires per type, and the factory
//! builds a driver for either type once the declaration path is used.

use std::sync::Arc;
use std::time::Duration;

use d2b_contracts_resource::v3::process::{EphemeralProcessSpec, ProcessSpec};
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, ZoneId};
use d2b_process_conformance::AdoptionCandidate;
use d2b_provider_process::{
    ExecutionMode, ProcessDriverArgs, ProcessEffectFacets, ProcessFamilySpec,
    ProcessProviderRuntime, ProcessResourceContext, ProcessResourceIdentity, ProviderAdoption,
    ProviderLaunch, ProviderLiveness, process_family_descriptors,
};
use d2b_resource_runtime::identity::ResourceKey;
use d2b_resource_runtime::provider::{DriverRegistration, ProviderDirectory};

/// A runtime facet that refuses every effect: this test proves the
/// declaration and registration path, never a launch. The reconciliation
/// surface is unreachable in these tests, so the bundle-facing reads
/// refuse loudly rather than inventing trusted data.
struct RefusingRuntime;

#[async_trait::async_trait]
impl ProcessProviderRuntime for RefusingRuntime {
    fn bundle(&self) -> &d2b_core::bundle_resolver::BundleResolver {
        unreachable!("the registration tests never reconcile a row")
    }

    fn socket_runtime_dir(&self) -> &std::path::Path {
        unreachable!("the registration tests never reconcile a row")
    }

    fn guest_setup_descriptor_digest(
        &self,
        _zone: &ZoneId,
        _guest_ref: &ResourceRef,
    ) -> Option<d2b_contracts_resource::v3::SchemaFingerprint> {
        None
    }

    async fn resolve_device_worker_launch(
        &self,
        _ctx: &mut d2b_resource_runtime::context::ResourceContext,
        _identity: &ProcessResourceIdentity,
        _spec: &ProcessFamilySpec,
    ) -> Result<Option<d2b_provider_process::DeviceWorkerLaunch>, &'static str> {
        Err("refused")
    }

    async fn launch_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &ProcessSpec,
        _timeout: Duration,
    ) -> Result<ProviderLaunch, String> {
        Err("refused".to_owned())
    }

    async fn launch_ephemeral_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &EphemeralProcessSpec,
        _timeout: Duration,
    ) -> Result<ProviderLaunch, String> {
        Err("refused".to_owned())
    }

    async fn adopt_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &ProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        Err("refused".to_owned())
    }

    async fn probe_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &ProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        Err("refused".to_owned())
    }

    async fn adopt_ephemeral_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &EphemeralProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        Err("refused".to_owned())
    }

    async fn probe_ephemeral_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &EphemeralProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        Err("refused".to_owned())
    }

    async fn stop_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &ProcessSpec,
        _term_timeout: Duration,
        _kill_timeout: Duration,
    ) -> Result<bool, String> {
        Err("refused".to_owned())
    }

    async fn stop_ephemeral_resource(
        &self,
        _context: ProcessResourceContext<'_>,
        _spec: &EphemeralProcessSpec,
        _term_timeout: Duration,
        _kill_timeout: Duration,
    ) -> Result<bool, String> {
        Err("refused".to_owned())
    }

    async fn stop_stale_resource(
        &self,
        _provider_ref: &ResourceRef,
        _candidate: &AdoptionCandidate,
    ) -> Result<(), String> {
        Err("refused".to_owned())
    }

    async fn finalize_resource(
        &self,
        _context: ProcessResourceContext<'_>,
    ) -> Result<(), String> {
        Err("refused".to_owned())
    }

    fn has_active_resource_in_zone(
        &self,
        _zone: &ZoneId,
        _zone_uid: Option<&ResourceUid>,
        _resource_ref: &ResourceRef,
    ) -> bool {
        false
    }
}

fn descriptors() -> [d2b_resource_types::DriverDescriptor; 2] {
    process_family_descriptors(ProcessDriverArgs {
        zone: ZoneId::parse("work").expect("zone"),
        facets: ProcessEffectFacets {
            runtime: Arc::new(RefusingRuntime),
            committed: None,
            guest_owners: None,
        },
        zone_uid: None,
        policy_revision: None,
        provider_assignment_generation: None,
        controller_generation: d2b_contracts_resource::v3::ControllerGeneration::new(1)
            .expect("controller generation"),
        guest_execution: None,
        mode: ExecutionMode::Host,
    })
}

/// One `Process` row's stored envelope, exactly as the spec store holds it.
fn process_spec_bytes() -> Vec<u8> {
    br#"{"providerRef":"Provider/system-minijail","executionRef":"Host/host-system","processClass":"worker","template":"reaction","drainTimeout":"250ms"}"#.to_vec()
}

/// Both member types register through the family's declarations: one
/// descriptor per type, both served by the family's decoder and factory.
#[test]
fn the_family_registers_both_member_types_from_its_declarations() {
    let mut providers = ProviderDirectory::new();
    for descriptor in descriptors() {
        providers
            .register_driver(&descriptor)
            .expect("the family registers");
    }

    let registered: Vec<String> = providers
        .registered_types()
        .into_iter()
        .map(|type_name| type_name.as_str().to_owned())
        .collect();
    assert_eq!(registered, vec!["EphemeralProcess", "Process"]);

    // The mask carries the presence obligation: both types are admitted by the
    // built-in and startup sources and cannot arrive late, so a plane that
    // opens without them fails closed by the registry's own contract.
    let descriptors = descriptors();
    for descriptor in &descriptors {
        assert!(
            descriptor.allowed_sources.requires_plane_registration(),
            "{} is required before the plane opens",
            descriptor.resource_type.to_resource_type_name().as_str()
        );
        assert!(
            !descriptor.exportable,
            "a process is never an export subject"
        );
    }

    providers.mark_plane_open();
}

/// The decoder a descriptor carries is the family's own: it decodes a stored
/// row envelope and refuses one that is not a resource spec.
#[test]
fn the_declared_decoder_decodes_the_family_rows() {
    for descriptor in descriptors() {
        let decoder = DriverRegistration::decoder(&descriptor);
        decoder
            .decode(&process_spec_bytes())
            .expect("a stored Process row envelope decodes");
        assert!(
            decoder.decode(b"{not-json").is_err(),
            "an unreadable envelope is refused"
        );
    }
}

/// The registry serves the family's factory per type, and a second
/// registration for the same type is refused rather than clobbering the first.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn the_registry_serves_one_factory_per_member_type() {
    let descriptors = descriptors();
    let mut providers = ProviderDirectory::new();
    providers
        .register_driver(&descriptors[0])
        .expect("the first registration succeeds");
    assert!(
        providers.register_driver(&descriptors[0]).is_err(),
        "one driver per resource type"
    );

    let key = ResourceKey::new("work", "Process", "worker");
    providers
        .create_driver(&key)
        .await
        .expect("the registered factory builds the driver");
    assert!(
        providers
            .create_driver(&ResourceKey::new("work", "Volume", "data"))
            .await
            .is_err(),
        "an unregistered type has no driver"
    );
}
