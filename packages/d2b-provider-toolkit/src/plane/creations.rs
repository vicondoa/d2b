//! The child-creation fence.
//!
//! A driver may create a child only when its own descriptor declares the
//! `(child type, provider)` pair. The declaration handle is the call
//! argument, so there is no second table to restate the permission in and
//! nothing can drift: an undeclared creation is refused terminally, naming
//! the declaring type and the child type.
//!
//! Custody is part of the same fence. A child declared
//! [`ChildCustody::ControllerOwned`] is created by the controller on the
//! declaring driver's behalf, so a driver that reaches for one is refused
//! too - the declaration says who creates, and the fence enforces it.

use std::fmt;

use d2b_resource_types::{ChildCreation, ChildCustody, DriverDescriptor, WellKnownType};

/// Every child creation the provider's drivers declared.
#[derive(Debug, Clone, Default)]
pub struct CreationTable {
    entries: Vec<DeclaredCreation>,
}

#[derive(Debug, Clone, Copy)]
struct DeclaredCreation {
    declaring: WellKnownType,
    declaration: &'static ChildCreation,
}

impl CreationTable {
    /// Collect the declared creations of every driver.
    pub fn over(drivers: &[DriverDescriptor]) -> Self {
        let mut entries = Vec::new();
        for driver in drivers {
            for declaration in driver.creations {
                entries.push(DeclaredCreation {
                    declaring: driver.resource_type,
                    declaration,
                });
            }
        }
        Self { entries }
    }

    /// Collect the declared creations of explicit declaration rows.
    ///
    /// A provider crate assembles its `DriverDescriptor`s once; a test that
    /// exercises the fence without a driver factory states the same rows
    /// directly. Both feed one table, so the fence cannot differ between
    /// them.
    pub fn declare(rows: &'static [(WellKnownType, &'static [ChildCreation])]) -> Self {
        let mut entries = Vec::new();
        for (declaring, declarations) in rows {
            for declaration in *declarations {
                entries.push(DeclaredCreation {
                    declaring: *declaring,
                    declaration,
                });
            }
        }
        Self { entries }
    }

    /// Every declared creation, paired with the type that declared it.
    pub fn declarations(
        &self,
    ) -> impl Iterator<Item = (WellKnownType, &'static ChildCreation)> + '_ {
        self.entries
            .iter()
            .map(|entry| (entry.declaring, entry.declaration))
    }

    /// Whether any driver declared this child type.
    pub fn declares_child(&self, child: WellKnownType) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.declaration.child == child)
    }

    /// Build the fence one declaring driver creates through.
    pub fn fence(&self, declaring: WellKnownType) -> ChildCreationFence<'_> {
        ChildCreationFence {
            table: self,
            declaring,
        }
    }
}

/// The fence one driver's `create_child` call runs through.
#[derive(Debug, Clone, Copy)]
pub struct ChildCreationFence<'a> {
    table: &'a CreationTable,
    declaring: WellKnownType,
}

impl<'a> ChildCreationFence<'a> {
    /// The type whose declarations this fence authorizes against.
    pub const fn declaring(&self) -> WellKnownType {
        self.declaring
    }

    /// Authorize one creation.
    ///
    /// The declaration handle itself is the license: it must be a row of this
    /// driver's own `creations` table and must be driver-owned.
    pub fn authorize(
        &self,
        declaration: &ChildCreation,
    ) -> Result<&'static ChildCreation, CreationRefusal> {
        let declared = self.table.entries.iter().find(|entry| {
            entry.declaring == self.declaring
                && entry.declaration.child == declaration.child
                && entry.declaration.provider_ref == declaration.provider_ref
                && entry.declaration.custody == declaration.custody
                && entry.declaration.order == declaration.order
        });
        let Some(entry) = declared else {
            let foreign = self.table.entries.iter().find(|entry| {
                entry.declaration.child == declaration.child
                    && entry.declaration.provider_ref == declaration.provider_ref
            });
            return Err(match foreign {
                Some(entry) => CreationRefusal::ForeignDeclaration {
                    declaring: self.declaring,
                    owner: entry.declaring,
                    child: declaration.child,
                    provider_ref: declaration.provider_ref,
                },
                None => CreationRefusal::Undeclared {
                    declaring: self.declaring,
                    child: declaration.child,
                    provider_ref: declaration.provider_ref,
                },
            });
        };
        if entry.declaration.custody == ChildCustody::ControllerOwned {
            return Err(CreationRefusal::ControllerOwned {
                declaring: self.declaring,
                child: declaration.child,
                provider_ref: declaration.provider_ref,
            });
        }
        Ok(entry.declaration)
    }
}

/// Why a creation was refused.
///
/// Every variant is terminal: a driver never retries a creation the
/// declaration fence refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CreationRefusal {
    /// No driver declared this `(child type, provider)` pair.
    Undeclared {
        /// The type that tried to create the child.
        declaring: WellKnownType,
        /// The child type it named.
        child: WellKnownType,
        /// The provider it named.
        provider_ref: &'static str,
    },
    /// The pair is declared, but by a different driver.
    ForeignDeclaration {
        /// The type that tried to create the child.
        declaring: WellKnownType,
        /// The type that declared the creation.
        owner: WellKnownType,
        /// The child type it named.
        child: WellKnownType,
        /// The provider it named.
        provider_ref: &'static str,
    },
    /// The creation is declared controller-owned, so the driver may not
    /// perform it.
    ControllerOwned {
        /// The type that tried to create the child.
        declaring: WellKnownType,
        /// The child type it named.
        child: WellKnownType,
        /// The provider it named.
        provider_ref: &'static str,
    },
}

impl CreationRefusal {
    /// The stable lower-kebab code for this refusal.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Undeclared { .. } => "undeclared-creation",
            Self::ForeignDeclaration { .. } => "foreign-creation",
            Self::ControllerOwned { .. } => "controller-owned-creation",
        }
    }

    /// The child type the refused creation named.
    pub const fn child(&self) -> WellKnownType {
        match self {
            Self::Undeclared { child, .. }
            | Self::ForeignDeclaration { child, .. }
            | Self::ControllerOwned { child, .. } => *child,
        }
    }
}

/// Why one declared child creation did not commit.
///
/// The two variants keep the two fences apart: a refused declaration is a
/// driver asking for something it never declared, while a refused spec is a
/// declared creation whose child's own spec did not admit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildCreationFailure {
    /// The declaration fence refused the creation.
    Declaration(CreationRefusal),
    /// The declaration was authorized and the child's spec was refused, with
    /// that refusal's closed code.
    Spec(&'static str),
}

impl ChildCreationFailure {
    /// The stable lower-kebab code for this failure.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Declaration(refusal) => refusal.code(),
            Self::Spec(code) => code,
        }
    }
}

impl fmt::Display for ChildCreationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Declaration(refusal) => refusal.fmt(formatter),
            Self::Spec(code) => formatter.write_str(code),
        }
    }
}

impl std::error::Error for ChildCreationFailure {}

impl fmt::Display for CreationRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Undeclared {
                declaring, child, ..
            } => write!(
                formatter,
                "{}: {} may not create {}",
                self.code(),
                declaring.to_resource_type_name().as_str(),
                child.to_resource_type_name().as_str()
            ),
            Self::ForeignDeclaration {
                declaring,
                owner,
                child,
                ..
            } => write!(
                formatter,
                "{}: {} is declared by {}, not by {}",
                self.code(),
                child.to_resource_type_name().as_str(),
                owner.to_resource_type_name().as_str(),
                declaring.to_resource_type_name().as_str()
            ),
            Self::ControllerOwned {
                declaring, child, ..
            } => write!(
                formatter,
                "{}: {} does not create controller-owned {}",
                self.code(),
                declaring.to_resource_type_name().as_str(),
                child.to_resource_type_name().as_str()
            ),
        }
    }
}

impl std::error::Error for CreationRefusal {}

#[cfg(test)]
mod tests {
    use super::{CreationRefusal, CreationTable};
    use d2b_resource_types::{ChildCreation, ChildCustody, WellKnownType};

    /// One pair two creators create, as a declaring driver carries it when
    /// one runtime kind's children are its own and another kind's are the
    /// controller's: the same child type and Provider, one row per creator.
    const DRIVER_OWNED: ChildCreation = ChildCreation {
        child: WellKnownType::VOLUME,
        provider_ref: "core.d2bus.org",
        custody: ChildCustody::DriverOwned,
        order: 1,
    };
    const CONTROLLER_OWNED: ChildCreation = ChildCreation {
        child: WellKnownType::VOLUME,
        provider_ref: "core.d2bus.org",
        custody: ChildCustody::ControllerOwned,
        order: 1,
    };
    static ROWS: &[(WellKnownType, &[ChildCreation])] =
        &[(WellKnownType::GUEST, &[DRIVER_OWNED, CONTROLLER_OWNED])];

    /// The declaration handle selects the creator: the driver-owned row of a
    /// pair authorizes, and the controller-owned claim on the same pair is
    /// refused so the same child cannot be created twice.
    #[test]
    fn a_pair_two_creators_declare_authorizes_only_the_driver_owned_row() {
        let table = CreationTable::declare(ROWS);
        let fence = table.fence(WellKnownType::GUEST);
        assert_eq!(fence.authorize(&DRIVER_OWNED), Ok(&DRIVER_OWNED));
        assert_eq!(
            fence.authorize(&CONTROLLER_OWNED),
            Err(CreationRefusal::ControllerOwned {
                declaring: WellKnownType::GUEST,
                child: WellKnownType::VOLUME,
                provider_ref: "core.d2bus.org",
            })
        );
    }
}
