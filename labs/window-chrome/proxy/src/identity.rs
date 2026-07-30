use d2b_core::workload_identity::WorkloadTarget;
use d2b_realm_core::WorkloadProviderKind;
use sha2::{Digest, Sha256};

/// Authenticated provider-neutral identity for one proxy instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyIdentity {
    target: WorkloadTarget,
    provider_kind: WorkloadProviderKind,
    legacy_vm_name: Option<String>,
}

impl ProxyIdentity {
    pub fn canonical(target: WorkloadTarget, provider_kind: WorkloadProviderKind) -> Self {
        Self {
            target,
            provider_kind,
            legacy_vm_name: None,
        }
    }

    pub fn legacy_vm(
        vm_name: impl Into<String>,
        target: WorkloadTarget,
        provider_kind: WorkloadProviderKind,
    ) -> Result<Self, ProxyIdentityError> {
        let vm_name = vm_name.into();
        validate_legacy_vm_name(&vm_name)?;
        if provider_kind == WorkloadProviderKind::UnsafeLocal {
            return Err(ProxyIdentityError::UnsafeLocalLegacyVm);
        }
        Ok(Self {
            target,
            provider_kind,
            legacy_vm_name: Some(vm_name),
        })
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

    pub fn legacy_vm_name(&self) -> Option<&str> {
        self.legacy_vm_name.as_deref()
    }

    pub fn canonical_target(&self) -> String {
        self.target.to_canonical()
    }

    pub fn log_label(&self) -> String {
        self.legacy_vm_name
            .clone()
            .unwrap_or_else(|| self.canonical_target())
    }

    pub fn bridge_component(&self) -> String {
        self.legacy_vm_name.clone().unwrap_or_else(|| {
            let digest = Sha256::digest(self.canonical_target().as_bytes());
            let mut encoded = String::with_capacity(24);
            for byte in &digest[..12] {
                use std::fmt::Write as _;
                let _ = write!(encoded, "{byte:02x}");
            }
            format!("endpoint-{encoded}")
        })
    }

    pub fn default_app_id_prefix(&self) -> String {
        match &self.legacy_vm_name {
            Some(vm_name) => format!("d2b.{vm_name}."),
            None => format!("d2b.{}.", self.canonical_target()),
        }
    }

    pub fn default_title_prefix(&self) -> String {
        match self.provider_kind {
            WorkloadProviderKind::UnsafeLocal => {
                format!("[unsafe-local {}] ", self.canonical_target())
            }
            _ => match &self.legacy_vm_name {
                Some(vm_name) => format!("[{vm_name}] "),
                None => format!("[{}] ", self.canonical_target()),
            },
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

impl From<String> for ProxyIdentity {
    fn from(vm_name: String) -> Self {
        let target = WorkloadTarget::parse(&format!("{vm_name}.local.d2b"))
            .expect("legacy VM label must form a canonical target");
        Self::legacy_vm(vm_name, target, WorkloadProviderKind::LocalVm)
            .expect("legacy VM label must be valid")
    }
}

impl From<&str> for ProxyIdentity {
    fn from(vm_name: &str) -> Self {
        vm_name.to_owned().into()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProxyIdentityError {
    #[error("invalid legacy VM name")]
    InvalidLegacyVmName,
    #[error("unsafe-local identity cannot carry a legacy VM name")]
    UnsafeLocalLegacyVm,
}

fn validate_legacy_vm_name(value: &str) -> Result<(), ProxyIdentityError> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.contains('/')
        || value.contains('\0')
    {
        return Err(ProxyIdentityError::InvalidLegacyVmName);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_unsafe_local_identity_uses_target_not_vm_assumptions() {
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
        assert!(identity.legacy_vm_name().is_none());
    }

    #[test]
    fn legacy_vm_identity_preserves_compatibility_prefixes() {
        let identity = ProxyIdentity::legacy_vm(
            "work",
            WorkloadTarget::parse("work.local.d2b").unwrap(),
            WorkloadProviderKind::LocalVm,
        )
        .unwrap();

        assert_eq!(identity.bridge_component(), "work");
        assert_eq!(identity.default_app_id_prefix(), "d2b.work.");
        assert_eq!(identity.default_title_prefix(), "[work] ");
    }

    #[test]
    fn unsafe_local_cannot_be_coerced_to_a_vm_identity() {
        assert_eq!(
            ProxyIdentity::legacy_vm(
                "tools",
                WorkloadTarget::parse("tools.host.d2b").unwrap(),
                WorkloadProviderKind::UnsafeLocal,
            ),
            Err(ProxyIdentityError::UnsafeLocalLegacyVm)
        );
    }
}
