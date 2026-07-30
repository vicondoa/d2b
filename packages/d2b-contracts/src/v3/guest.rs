//! Guest primitive ResourceType base spec.
//!
//! `Guest` is the VM, sandbox, cloud, or remote execution, policy, and budget
//! parent. It uses the same `ExecutionPolicy` fields as `Host` and adds the
//! optional NixOS system artifact ID that local VM Providers boot.
//! Provider-specific boot, identity, and runtime settings belong to the Layer
//! 3 `spec.provider` envelope on the universal `ResourceSpec`, never here.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    ResourceRef,
    execution_policy::{
        BoundedToken, BudgetSpec, DeviceAttachment, ExecutionDomain, ExecutionPolicy,
        ExecutionPolicyWire, NetworkAttachment, redacted_debug,
    },
    resource_schema::CanonicalJsonObject,
};

/// The canonical ResourceType name for this module.
pub const GUEST_RESOURCE_TYPE: &str = "Guest";

/// The Guest ResourceType base spec.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GuestSpec {
    #[serde(flatten)]
    policy: ExecutionPolicy,
    system_artifact_id: Option<BoundedToken>,
}

impl GuestSpec {
    /// Construct a Guest base spec.
    ///
    /// `system_artifact_id` is `None` for cloud and remote Providers that do
    /// not boot a Nix-built system.
    pub const fn new(policy: ExecutionPolicy, system_artifact_id: Option<BoundedToken>) -> Self {
        Self {
            policy,
            system_artifact_id,
        }
    }

    /// Construct the canonical minimal system Guest base spec.
    pub fn system_default() -> Self {
        Self::new(ExecutionPolicy::system_default(), None)
    }

    /// Borrow the shared execution policy.
    pub const fn policy(&self) -> &ExecutionPolicy {
        &self.policy
    }

    /// Borrow the NixOS system closure artifact ID.
    pub const fn system_artifact_id(&self) -> Option<&BoundedToken> {
        self.system_artifact_id.as_ref()
    }
}

redacted_debug!(GuestSpec);

impl<'de> Deserialize<'de> for GuestSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default = "system_domain")]
            default_domain: ExecutionDomain,
            #[serde(default = "system_domains")]
            allowed_domains: Vec<ExecutionDomain>,
            #[serde(default)]
            default_user_ref: Option<ResourceRef>,
            #[serde(default)]
            budget: BudgetSpec,
            #[serde(default)]
            network_attachments: Vec<NetworkAttachment>,
            #[serde(default)]
            device_attachments: Vec<DeviceAttachment>,
            #[serde(default)]
            volume_attachment_defaults: Vec<CanonicalJsonObject>,
            #[serde(default)]
            system_artifact_id: Option<BoundedToken>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let policy = ExecutionPolicyWire {
            default_domain: wire.default_domain,
            allowed_domains: wire.allowed_domains,
            default_user_ref: wire.default_user_ref,
            budget: wire.budget,
            network_attachments: wire.network_attachments,
            device_attachments: wire.device_attachments,
            volume_attachment_defaults: wire.volume_attachment_defaults,
        }
        .into_policy()
        .map_err(serde::de::Error::custom)?;
        Ok(Self::new(policy, wire.system_artifact_id))
    }
}

const fn system_domain() -> ExecutionDomain {
    ExecutionDomain::System
}

fn system_domains() -> Vec<ExecutionDomain> {
    vec![ExecutionDomain::System]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::{execution_policy::to_base_object, resource_schema::canonical_json_bytes};

    const MINIMAL_GUEST_SPEC: &[u8] = br#"{"allowedDomains":["system"],"budget":{},"defaultDomain":"system","defaultUserRef":null,"deviceAttachments":[],"networkAttachments":[],"systemArtifactId":null,"volumeAttachmentDefaults":[]}"#;

    #[test]
    fn schema_vector_pins_the_minimal_guest_base_spec() {
        let minimal = GuestSpec::system_default();
        assert_eq!(canonical_json_bytes(&minimal).unwrap(), MINIMAL_GUEST_SPEC);
        let parsed: GuestSpec = serde_json::from_slice(MINIMAL_GUEST_SPEC).unwrap();
        assert_eq!(parsed, minimal);
    }

    #[test]
    fn guest_and_host_share_one_execution_policy_definition() {
        let guest: GuestSpec = serde_json::from_slice(
            br#"{"defaultDomain":"user","allowedDomains":["system","user"],"defaultUserRef":"User/alice"}"#,
        )
        .unwrap();
        assert!(guest.policy().admits_user_domain());
        assert!(
            serde_json::from_slice::<GuestSpec>(
                br#"{"defaultDomain":"user","allowedDomains":["system","user"]}"#
            )
            .is_err()
        );
    }

    #[test]
    fn system_artifact_id_is_a_bounded_token() {
        let guest: GuestSpec =
            serde_json::from_slice(br#"{"systemArtifactId":"guest-system"}"#).unwrap();
        assert_eq!(guest.system_artifact_id().unwrap().as_str(), "guest-system");
        assert!(
            serde_json::from_slice::<GuestSpec>(br#"{"systemArtifactId":"/nix/store/x"}"#).is_err()
        );
    }

    #[test]
    fn base_object_never_carries_a_universal_or_layer_three_field() {
        let base = to_base_object(&GuestSpec::system_default()).unwrap();
        for reserved in ["providerRef", "updatePolicy", "provider"] {
            assert!(base.get(reserved).is_none());
        }
        assert!(serde_json::from_slice::<GuestSpec>(br#"{"vcpus":2}"#).is_err());
    }

    #[test]
    fn diagnostics_stay_redacted() {
        assert_eq!(
            format!("{:?}", GuestSpec::system_default()),
            "GuestSpec(<redacted>)"
        );
    }
}
