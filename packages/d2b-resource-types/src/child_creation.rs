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


