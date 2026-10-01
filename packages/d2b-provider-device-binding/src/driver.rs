//! The `DeviceBinding` resource type's driver surface.

#![deny(missing_docs)]

use std::sync::Arc;

use d2b_resource_types::{AllowedSources, DriverDescriptor, WellKnownType};

/// The `DeviceBinding` ResourceType name.
pub const BINDING_TYPE_NAME: &str = "DeviceBinding";

/// The verbs the `DeviceBinding` driver serves.
const BINDING_VERBS: &[&str] = d2b_resource_types::CONVERTED_TYPE_VERBS;

/// The spec decoder for a `DeviceBinding` row.
pub fn binding_spec_decoder() -> Arc<dyn d2b_resource_runtime::context::SpecDecoder> {
    d2b_resource_runtime::metadata::metadata_spec_decoder()
}

/// The driver descriptor the plane registers `DeviceBinding` by.
pub fn binding_descriptor() -> DriverDescriptor {
    DriverDescriptor {
        resource_type: WellKnownType::DEVICE_BINDING,
        allowed_sources: AllowedSources::BUILTIN | AllowedSources::STARTUP,
        verbs: BINDING_VERBS,
        execution: &[],
        exportable: false,
        reads: &[],
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: binding_spec_decoder(),
        factory: Arc::new(
            d2b_resource_runtime::metadata::MetadataDriverFactory::new(
                d2b_resource_runtime::identity::ResourceTypeName::new(BINDING_TYPE_NAME),
            ),
        ),
    }
}
