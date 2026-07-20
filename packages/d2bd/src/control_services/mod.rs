use d2b_contracts::v2_services::{MethodSpec, ServiceSpec};

mod allocator;
mod broker;
mod daemon;
mod guest;
mod provider;
mod realm;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Component {
    Allocator,
    Broker,
    Daemon,
    Guest,
    Provider,
    Realm,
}

const COMPONENTS: [Component; 6] = [
    Component::Allocator,
    Component::Broker,
    Component::Daemon,
    Component::Guest,
    Component::Provider,
    Component::Realm,
];

impl Component {
    fn owns(self, service: &ServiceSpec, method: &MethodSpec) -> bool {
        match self {
            Self::Allocator => allocator::owns(service, method),
            Self::Broker => broker::owns(service, method),
            Self::Daemon => daemon::owns(service, method),
            Self::Guest => guest::owns(service, method),
            Self::Provider => provider::owns(service, method),
            Self::Realm => realm::owns(service, method),
        }
    }
}

pub(crate) fn owner(service: &ServiceSpec, method: &MethodSpec) -> Option<Component> {
    let mut owners = COMPONENTS
        .into_iter()
        .filter(|component| component.owns(service, method));
    let owner = owners.next();
    assert!(
        owners.next().is_none(),
        "control-service method has multiple composition owners"
    );
    owner
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v2_services::SERVICE_INVENTORY;

    const OWNED_PACKAGES: [&str; 5] = [
        "d2b.broker.v2",
        "d2b.daemon.v2",
        "d2b.guest.v2",
        "d2b.provider.v2",
        "d2b.realm.v2",
    ];

    #[test]
    fn service_methods_have_one_composition_owner() {
        for service in SERVICE_INVENTORY {
            for method in service.methods {
                let owner = owner(service, method);
                assert_eq!(
                    owner.is_some(),
                    OWNED_PACKAGES.contains(&service.package),
                    "{}.{}.{}",
                    service.package,
                    service.service,
                    method.name
                );
            }
        }
    }

    #[test]
    fn allocator_owns_only_allocate_and_spawn() {
        let broker = SERVICE_INVENTORY
            .iter()
            .find(|service| service.package == "d2b.broker.v2")
            .expect("broker service");
        let allocator_methods = broker
            .methods
            .iter()
            .filter(|method| owner(broker, method) == Some(Component::Allocator))
            .map(|method| method.name)
            .collect::<Vec<_>>();
        assert_eq!(allocator_methods, ["Allocate", "Spawn"]);
    }
}
