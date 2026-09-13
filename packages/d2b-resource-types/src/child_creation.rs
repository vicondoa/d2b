//! Declared child resources and who owns their creation.

use crate::WellKnownType;

/// Who creates and tears down a declared child resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChildCustody {
    /// The declaring driver creates the child and owns its teardown.
    DriverOwned,
    /// The controller creates the child on the declaring driver's behalf and
    /// owns its teardown.
    ControllerOwned,
}

/// One child creation a driver declares.
///
/// The declaration is the only license to create: a driver may create a child
/// only when its (child type, provider) pair appears here, and the runtime
/// terminally refuses an undeclared creation, naming the declaring family and
/// the child type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChildCreation {
    /// The child resource type.
    pub child: WellKnownType,
    /// The provider that serves the child resource.
    pub provider_ref: &'static str,
    /// Who creates and tears down the child.
    pub custody: ChildCustody,
    /// The creation and deletion ordering rank.
    ///
    /// Children are created in ascending rank order and deleted in descending
    /// rank order, so a child that depends on an earlier rank is realized
    /// before it is created and retires before its dependency.
    pub order: u16,
}

#[cfg(test)]
mod tests {
    use super::{ChildCreation, ChildCustody};
    use crate::WellKnownType;

    const CREATION: ChildCreation = ChildCreation {
        child: WellKnownType::PROCESS,
        provider_ref: "core.d2bus.org",
        custody: ChildCustody::DriverOwned,
        order: 3,
    };

    /// The declaration keeps every field exactly as declared.
    #[test]
    fn a_child_creation_keeps_every_field() {
        assert_eq!(CREATION.child, WellKnownType::PROCESS);
        assert_eq!(CREATION.child.to_resource_type_name().as_str(), "Process");
        assert_eq!(CREATION.provider_ref, "core.d2bus.org");
        assert_eq!(CREATION.custody, ChildCustody::DriverOwned);
        assert_eq!(CREATION.order, 3);
    }

    /// Custody is a closed two-value choice, not a boolean-shaped default.
    #[test]
    fn custody_distinguishes_its_two_values() {
        assert_ne!(ChildCustody::DriverOwned, ChildCustody::ControllerOwned);
        assert_eq!(
            ChildCreation {
                custody: ChildCustody::ControllerOwned,
                ..CREATION
            }
            .custody,
            ChildCustody::ControllerOwned
        );
    }
}
