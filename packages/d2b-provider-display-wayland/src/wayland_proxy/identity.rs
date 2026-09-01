use d2b_contracts::{workload::WorkloadProviderKind, workload_identity::WorkloadTarget};
use sha2::{Digest, Sha256};

/// Authenticated provider-neutral identity for one proxy instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyIdentity {
    target: WorkloadTarget,
    provider_kind: WorkloadProviderKind,
}

impl ProxyIdentity {
    pub fn canonical(target: WorkloadTarget, provider_kind: WorkloadProviderKind) -> Self {
        Self {
            target,
            provider_kind,
        }
    }

    pub fn target(&self) -> &WorkloadTarget {
        &self.target
    }

    pub fn provider_kind(&self) -> WorkloadProviderKind {
        self.provider_kind
    }

    pub fn provider_kind_label(&self) -> &'static str {
        match self.provider_kind {
            WorkloadProviderKind::LocalVm => "local-vm",
            WorkloadProviderKind::QemuMedia => "qemu-media",
            WorkloadProviderKind::ProviderManaged => "provider-managed",
            WorkloadProviderKind::UnsafeLocal => "unsafe-local",
        }
    }

    pub fn canonical_target(&self) -> String {
        self.target.to_canonical()
    }

    pub fn log_label(&self) -> String {
        match self.provider_kind {
            WorkloadProviderKind::UnsafeLocal => self.canonical_target(),
            _ => self.target.workload.as_str().to_owned(),
        }
    }

    pub fn bridge_component(&self) -> String {
        let digest = Sha256::digest(self.canonical_target().as_bytes());
        let mut encoded = String::with_capacity(24);
        for byte in &digest[..12] {
            use std::fmt::Write as _;
            let _ = write!(encoded, "{byte:02x}");
        }
        format!("endpoint-{encoded}")
    }

    pub fn default_app_id_prefix(&self) -> String {
        match self.provider_kind {
            WorkloadProviderKind::UnsafeLocal => format!("d2b.{}.", self.canonical_target()),
            _ => format!("d2b.{}.", self.target.workload.as_str()),
        }
    }

    pub fn default_title_prefix(&self) -> String {
        match self.provider_kind {
            WorkloadProviderKind::UnsafeLocal => {
                format!("[unsafe-local {}] ", self.canonical_target())
            }
            _ => format!("[{}] ", self.target.workload.as_str()),
        }
    }

    pub fn default_warning_label(&self) -> String {
        match self.provider_kind {
            WorkloadProviderKind::UnsafeLocal => {
                format!("{} · unsafe-local", self.canonical_target())
            }
            _ => self.canonical_target(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_unsafe_local_identity_uses_target_not_workload_aliases() {
        let identity = ProxyIdentity::canonical(
            WorkloadTarget::parse("tools.host.d2b").unwrap(),
            WorkloadProviderKind::UnsafeLocal,
        );

        assert_eq!(identity.canonical_target(), "tools.host.d2b");
        assert_eq!(
            identity.bridge_component(),
            "endpoint-fc002cd9909aab17c2232e85"
        );
        assert_eq!(identity.default_app_id_prefix(), "d2b.tools.host.d2b.");
        assert_eq!(
            identity.default_title_prefix(),
            "[unsafe-local tools.host.d2b] "
        );
        assert_eq!(
            identity.default_warning_label(),
            "tools.host.d2b · unsafe-local"
        );
    }

    #[test]
    fn canonical_local_identity_uses_its_workload_label_for_presentation() {
        let identity = ProxyIdentity::canonical(
            WorkloadTarget::parse("work.local.d2b").unwrap(),
            WorkloadProviderKind::LocalVm,
        );

        assert_eq!(identity.default_app_id_prefix(), "d2b.work.");
        assert_eq!(identity.default_title_prefix(), "[work] ");
    }

    #[test]
    fn canonical_identity_bridge_component_is_zone_collision_safe() {
        let first = ProxyIdentity::canonical(
            WorkloadTarget::parse("work.local.d2b").unwrap(),
            WorkloadProviderKind::LocalVm,
        );
        let second = ProxyIdentity::canonical(
            WorkloadTarget::parse("work.personal.d2b").unwrap(),
            WorkloadProviderKind::LocalVm,
        );

        assert_ne!(first.bridge_component(), second.bridge_component());
    }
}
