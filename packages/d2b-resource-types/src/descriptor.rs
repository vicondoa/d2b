//! The driver descriptor: one resource type's complete declaration.

use std::sync::Arc;

use d2b_resource_runtime::context::SpecDecoder;
use d2b_resource_runtime::driver::ResourceDriverFactory;
use d2b_resource_runtime::identity::ResourceTypeName;
use d2b_resource_runtime::provider::DriverRegistration;

use crate::{
    AllowedSources, ChildCreation, OperationDef, ServiceDecl, StartupStep, WellKnownType,
};

/// The declaration of one resource type's driver.
///
/// A descriptor covers exactly one resource type. The registry keys the
/// driver by [`DriverDescriptor::resource_type`], so a driver that serves a
/// family declares one descriptor per member type, and no descriptor can
/// claim a second type.
///
/// The descriptor carries declarations only: the driver implementation, the
/// spec decoder, and the operation handlers stay in the declaring per-type
/// crate.
pub struct DriverDescriptor {
    /// The one resource type this descriptor declares.
    pub resource_type: WellKnownType,
    /// The registration sources that admit the driver.
    pub allowed_sources: AllowedSources,
    /// The resource verbs the type supports.
    ///
    /// The labels stay plain strings because the typed verb vocabulary
    /// (`RoleResourceVerb`) is owned by `d2b-contracts-zone-session`, which
    /// this declaration crate deliberately does not depend on. A consumer
    /// that needs the typed vocabulary validates the labels against it.
    pub verbs: &'static [&'static str],
    /// The execution domains the type can be reconciled in.
    pub execution: &'static [&'static str],
    /// Whether resources of the type can be exported to another zone.
    pub exportable: bool,
    /// The resource types the driver reads while reconciling.
    pub reads: &'static [WellKnownType],
    /// The broker operations the driver serves, with their handlers.
    pub operations: &'static [OperationDef],
    /// The children the driver may create.
    pub creations: &'static [ChildCreation],
    /// The startup steps the driver contributes.
    pub startup: &'static [StartupStep],
    /// The services the provider serves.
    pub services: &'static [ServiceDecl],
    /// The decoder for the type's stored desired-spec envelopes.
    ///
    /// The decoder contract lives in `d2b_resource_runtime::context`, next to
    /// the resource context the manager wires it into.
    pub decoder: Arc<dyn SpecDecoder>,
    /// The factory that builds this type's driver.
    pub factory: Arc<dyn ResourceDriverFactory>,
}

/// The registry's view of one descriptor, beside the type it serves.
///
/// The runtime registry cannot name this crate (the dependency runs the other
/// way: this crate consumes the runtime's decoder and factory contracts), so
/// the bridge between the declaration and the registry's view lives here. The
/// view carries what registration enforces: one resource type per driver, the
/// allowed-source mask predicate that gates when the driver may arrive, the
/// canonical operation references the handler table indexes, and the decoder
/// and factory the registry serves.
impl DriverRegistration for DriverDescriptor {
    fn resource_type(&self) -> ResourceTypeName {
        self.resource_type.to_resource_type_name()
    }

    fn requires_plane_registration(&self) -> bool {
        self.allowed_sources.requires_plane_registration()
    }

    fn operation_refs(&self) -> Vec<String> {
        self.operations
            .iter()
            .map(|operation| operation.operation_ref.to_canonical_string())
            .collect()
    }

    fn declared_verbs(&self) -> Vec<String> {
        self.verbs.iter().map(|verb| (*verb).to_owned()).collect()
    }

    fn decoder(&self) -> Arc<dyn SpecDecoder> {
        Arc::clone(&self.decoder)
    }

    fn factory(&self) -> Arc<dyn ResourceDriverFactory> {
        Arc::clone(&self.factory)
    }
}
