//! Service-package route gate.

use super::audit::{BusAuditWriter, RouteAuditContext};
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

/// Resolve a route and append its bounded admission decision.
pub fn admit_route_with_audit(
    table: &RouteTable,
    service: impl Into<String>,
    method: impl Into<String>,
    direction: RouteDirection,
    context: &RouteAuditContext,
    audit: &BusAuditWriter<'_>,
) -> Result<ServiceRoute, RouteTableError> {
    let service = service.into();
    let method = method.into();
    let resolution = table.resolve(&service, &method);
    audit
        .record_resolution(context, &service, &method, direction, resolution)
        .map_err(|_| RouteTableError::AuditUnavailable)?;
    resolution?;
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

    #[test]
    fn audited_route_gate_records_only_the_closed_route_shape() {
        let mut table = RouteTable::default();
        table.register("d2b.audit.v3", ["Export"]).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let sink = d2b_audit::AuditSink::open(directory.path()).unwrap();
        let context = RouteAuditContext::new(
            1,
            "work",
            "operation",
            "correlation",
            "bus",
            d2b_audit::genesis_hash(),
            "sha256:subject",
            "allowed",
            1,
            None,
        );
        let writer = BusAuditWriter::new(&sink);
        let route = admit_route_with_audit(
            &table,
            "d2b.audit.v3",
            "Export",
            RouteDirection::Local,
            &context,
            &writer,
        )
        .unwrap();
        assert_eq!(route.direction(), RouteDirection::Local);
        assert!(
            std::fs::read_dir(directory.path())
                .unwrap()
                .next()
                .is_some()
        );
    }
}
