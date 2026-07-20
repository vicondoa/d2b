use d2b_contracts::v2_services::{MethodSpec, ServiceSpec};

pub(super) fn owns(service: &ServiceSpec, _: &MethodSpec) -> bool {
    service.package == "d2b.guest.v2"
}
