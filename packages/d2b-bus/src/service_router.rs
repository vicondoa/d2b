//! Service-package route gate.

use super::routing::{RouteDirection, RouteTable, RouteTableError};

/// A route request admitted by the closed service table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRoute {
    service: String,
    method: String,
    direction: RouteDirection,
}

impl ServiceRoute {
    /// Borrow the service package.
    pub fn service(&self) -> &str {
        &self.service
    }

    /// Borrow the method.
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Return route direction.
    pub const fn direction(&self) -> RouteDirection {
        self.direction
    }
}

/// Build a route only after closed-set service/method resolution.
pub fn admit_route(
    table: &RouteTable,
    service: impl Into<String>,
    method: impl Into<String>,
    direction: RouteDirection,
) -> Result<ServiceRoute, RouteTableError> {
    let service = service.into();
    let method = method.into();
    table.resolve(&service, &method)?;
    Ok(ServiceRoute {
        service,
        method,
        direction,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_gate_does_not_accept_free_form_services() {
        let mut table = RouteTable::default();
        table.register("d2b.audit.v3", ["Export"]).unwrap();
        assert!(admit_route(&table, "d2b.audit.v3", "Export", RouteDirection::Local).is_ok());
        assert!(admit_route(&table, "unknown", "Export", RouteDirection::Local).is_err());
    }
}
