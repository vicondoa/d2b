//! Semantic user-namespace mapping for minijail-class processes.

use d2b_contracts::v3::process::MappingClass;

use d2b_process_conformance::ProcessConformanceError;

/// A user-namespace mapping plan with no numeric host identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserNamespacePlan {
    mapping_class: MappingClass,
}

impl UserNamespacePlan {
    /// Build the only currently admitted mapping.
    pub fn new(mapping_class: MappingClass) -> Result<Self, ProcessConformanceError> {
        if mapping_class != MappingClass::ProcessPrincipalRoot {
            return Err(ProcessConformanceError::SandboxRejected);
        }
        Ok(Self { mapping_class })
    }

    /// Return the semantic mapping class.
    pub const fn mapping_class(self) -> MappingClass {
        self.mapping_class
    }
}
