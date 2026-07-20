use d2b_contracts::v2_services::{MethodSpec, ServiceSpec};

pub(super) fn owns(service: &ServiceSpec, method: &MethodSpec) -> bool {
    service.package == "d2b.broker.v2" && !matches!(method.name, "Allocate" | "Spawn")
}
