use std::{error::Error, fmt};

use d2b_contracts::v2_provider::{ProviderContractError, ProviderFailure};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactoryError {
    Rejected,
    Unavailable,
}

impl fmt::Display for FactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Rejected => "provider factory rejected configuration",
            Self::Unavailable => "provider factory unavailable",
        })
    }
}

impl Error for FactoryError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryBuildError {
    Contract(ProviderContractError),
    DuplicateFactory,
    DuplicateProvider,
    MissingFactory,
    FactoryFailed(FactoryError),
    DescriptorMismatch,
    CapabilityMismatch,
    GenerationMismatch,
    BoundExceeded,
    EmptyRegistry,
    TransactionAborted,
}

impl fmt::Display for RegistryBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Contract(error) => {
                return write!(formatter, "provider contract validation failed ({error})");
            }
            Self::DuplicateFactory => "duplicate provider factory",
            Self::DuplicateProvider => "duplicate provider instance",
            Self::MissingFactory => "provider factory is not registered",
            Self::FactoryFailed(error) => {
                return write!(formatter, "provider factory construction failed ({error})");
            }
            Self::DescriptorMismatch => "provider descriptor does not match registry axis",
            Self::CapabilityMismatch => "provider capability publication does not match descriptor",
            Self::GenerationMismatch => "provider generation does not match registry generation",
            Self::BoundExceeded => "provider registry bound exceeded",
            Self::EmptyRegistry => "provider registry has no configured instances",
            Self::TransactionAborted => "provider registry transaction was aborted",
        };
        formatter.write_str(message)
    }
}

impl Error for RegistryBuildError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Contract(error) => Some(error),
            Self::FactoryFailed(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ProviderContractError> for RegistryBuildError {
    fn from(value: ProviderContractError) -> Self {
        Self::Contract(value)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum ProviderRuntimeError {
    Contract(ProviderContractError),
    Provider(Box<ProviderFailure>),
    NotAccepting,
    UnknownProvider,
    CapabilityDenied,
    InFlightLimit,
    Cancelled,
    DeadlineExpired,
    SessionIdentityMismatch,
    ResponseMismatch,
    InvalidLifecycleTransition,
}

impl fmt::Debug for ProviderRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Contract(error) => formatter.debug_tuple("Contract").field(error).finish(),
            Self::Provider(error) => formatter.debug_tuple("Provider").field(error).finish(),
            Self::NotAccepting => formatter.write_str("NotAccepting"),
            Self::UnknownProvider => formatter.write_str("UnknownProvider"),
            Self::CapabilityDenied => formatter.write_str("CapabilityDenied"),
            Self::InFlightLimit => formatter.write_str("InFlightLimit"),
            Self::Cancelled => formatter.write_str("Cancelled"),
            Self::DeadlineExpired => formatter.write_str("DeadlineExpired"),
            Self::SessionIdentityMismatch => formatter.write_str("SessionIdentityMismatch"),
            Self::ResponseMismatch => formatter.write_str("ResponseMismatch"),
            Self::InvalidLifecycleTransition => formatter.write_str("InvalidLifecycleTransition"),
        }
    }
}

impl fmt::Display for ProviderRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Contract(error) => {
                return write!(formatter, "provider contract validation failed ({error})");
            }
            Self::Provider(error) => {
                return write!(
                    formatter,
                    "provider operation failed ({:?}, retry={:?}, type={:?}, reason={:?}, remediation={:?})",
                    error.kind, error.retry, error.provider_type, error.reason, error.remediation
                );
            }
            Self::NotAccepting => "provider registry is not accepting calls",
            Self::UnknownProvider => "provider is not registered",
            Self::CapabilityDenied => "provider capability is not registered",
            Self::InFlightLimit => "provider in-flight limit reached",
            Self::Cancelled => "provider operation cancelled",
            Self::DeadlineExpired => "provider operation deadline expired",
            Self::SessionIdentityMismatch => "authenticated provider session identity mismatch",
            Self::ResponseMismatch => "provider response binding mismatch",
            Self::InvalidLifecycleTransition => "invalid provider registry lifecycle transition",
        };
        formatter.write_str(message)
    }
}

impl Error for ProviderRuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Contract(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ProviderContractError> for ProviderRuntimeError {
    fn from(value: ProviderContractError) -> Self {
        Self::Contract(value)
    }
}

impl From<ProviderFailure> for ProviderRuntimeError {
    fn from(value: ProviderFailure) -> Self {
        Self::Provider(Box::new(value))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegistryShutdownReport {
    pub drained: bool,
    pub unresolved_in_flight: usize,
}
