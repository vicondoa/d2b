//! The driver descriptor: one resource type's complete declaration.

use std::sync::Arc;

use d2b_resource_runtime::context::SpecDecoder;
use d2b_resource_runtime::driver::ResourceDriverFactory;

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
