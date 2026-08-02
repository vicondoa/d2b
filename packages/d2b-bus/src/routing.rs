//! Adapted Zone service routing table.

use std::collections::BTreeMap;

/// Closed route direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteDirection {
    /// Component-local route.
    Local,
    /// Host-side route.
    Host,
    /// Guest-side route.
    Guest,
    /// ZoneLink route.
    ZoneLink,
}

impl RouteDirection {
    /// Stable wire label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Host => "host",
            Self::Guest => "guest",
            Self::ZoneLink => "zone_link",
        }
    }
}

/// Stable routing error class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusErrorKind {
    /// The route succeeded.
    Ok,
    /// Authorization denied it.
    Denied,
    /// The service or method was absent.
    NotFound,
    /// The route failed internally.
    Error,
}

impl BusErrorKind {
    /// Stable code used in audit and metrics.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Denied => "denied",
            Self::NotFound => "not_found",
            Self::Error => "error",
        }
    }
}

/// Closed service route table.
#[derive(Debug, Default)]
pub struct RouteTable {
    services: BTreeMap<String, Vec<String>>,
}

impl RouteTable {
    /// Register a service package and its methods.
    pub fn register(
        &mut self,
        service: impl Into<String>,
        methods: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<(), RouteTableError> {
        let service = service.into();
        if service.is_empty() || self.services.contains_key(&service) {
            return Err(RouteTableError::DuplicateService);
        }
        let mut methods = methods.into_iter().map(Into::into).collect::<Vec<_>>();
        methods.sort();
        methods.dedup();
        if methods.is_empty() {
            return Err(RouteTableError::EmptyService);
        }
        self.services.insert(service, methods);
        Ok(())
    }

    /// Resolve a service/method pair without accepting caller-supplied Zone
    /// or subject identity.
    pub fn resolve(&self, service: &str, method: &str) -> Result<(), RouteTableError> {
        let methods = self
            .services
            .get(service)
            .ok_or(RouteTableError::ServiceNotFound)?;
        if methods.iter().any(|candidate| candidate == method) {
            Ok(())
        } else {
            Err(RouteTableError::MethodNotFound)
        }
    }
}

/// Closed route-table failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteTableError {
    /// Duplicate service package.
    DuplicateService,
    /// A service had no methods.
    EmptyService,
    /// Service absent.
    ServiceNotFound,
    /// Method absent.
    MethodNotFound,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_table_is_closed_and_identity_free() {
        let mut table = RouteTable::default();
        table.register("d2b.resource.v3", ["Get", "List"]).unwrap();
        assert!(table.resolve("d2b.resource.v3", "Get").is_ok());
        assert_eq!(
            table.resolve("d2b.resource.v3", "Delete"),
            Err(RouteTableError::MethodNotFound)
        );
    }
}
