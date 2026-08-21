//! Host primitive ResourceType base spec.
//!
//! `Host` is the physical or local execution, policy, and budget parent.
//! Layer 2 is this base spec; `spec.providerRef`, `spec.updatePolicy`, and
//! the Layer 3 `spec.provider` extension envelope live on the universal
//! `ResourceSpec` and are never restated here.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    ResourceRef,
    execution_policy::{
        BudgetSpec, DeviceAttachment, ExecutionDomain, ExecutionPolicy, ExecutionPolicyWire,
        NetworkAttachment, PrimitiveSpecError, redacted_debug,
    },
    resource_schema::CanonicalJsonObject,
};

/// The canonical ResourceType name for this module.
pub const HOST_RESOURCE_TYPE: &str = "Host";
/// The only Provider admitted by `Host.spec.providerRef`.
pub const HOST_PROVIDER_REF: &str = "Provider/system-core";

/// The explicit no-isolation posture of the user-only Host.
///
/// The posture is a promoted Host base field; it is never a
/// `spec.provider.settings` field, and `null` used to evade the
/// no-isolation warning is rejected.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum IsolationPosture {
    /// The Host provides no isolation boundary.
    #[serde(rename = "none")]
    NoIsolation,
}

/// The Host ResourceType base spec.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostSpec {
    #[serde(flatten)]
    policy: ExecutionPolicy,
    isolation_posture: Option<IsolationPosture>,
}

impl HostSpec {
    /// Construct a Host base spec after checking the posture invariant.
    ///
    /// The user-only no-isolation Host is exactly the tuple
    /// `defaultDomain = user`, `allowedDomains = [user]`, and a present
    /// `defaultUserRef`. That tuple requires the explicit posture, and the
    /// posture requires that tuple; neither direction may be evaded.
    pub fn new(
        policy: ExecutionPolicy,
        isolation_posture: Option<IsolationPosture>,
    ) -> Result<Self, PrimitiveSpecError> {
        let user_only = policy.default_domain() == ExecutionDomain::User
            && policy.allowed_domains() == [ExecutionDomain::User]
            && policy.default_user_ref().is_some();
        if user_only != isolation_posture.is_some() {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        Ok(Self {
            policy,
            isolation_posture,
        })
    }

    /// Construct the canonical minimal system Host base spec.
    pub fn system_default() -> Self {
        Self::new(ExecutionPolicy::system_default(), None)
            .expect("the minimal system Host spec is always valid")
    }

    /// Construct the v3 successor to the unsafe-local user-only Host.
    pub fn user_only(default_user_ref: ResourceRef) -> Result<Self, PrimitiveSpecError> {
        let policy = ExecutionPolicy::new(
            ExecutionDomain::User,
            vec![ExecutionDomain::User],
            Some(default_user_ref),
            BudgetSpec::default(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )?;
        Self::new(policy, Some(IsolationPosture::NoIsolation))
    }

    /// Borrow the shared execution policy.
    pub const fn policy(&self) -> &ExecutionPolicy {
        &self.policy
    }

    /// Return the explicit isolation posture.
    pub const fn isolation_posture(&self) -> Option<IsolationPosture> {
        self.isolation_posture
    }

    /// Whether this Host declares the explicit no-isolation posture.
    pub const fn is_no_isolation(&self) -> bool {
        self.isolation_posture.is_some()
    }
}

redacted_debug!(HostSpec);

impl<'de> Deserialize<'de> for HostSpec {
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
            isolation_posture: Option<IsolationPosture>,
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
        Self::new(policy, wire.isolation_posture).map_err(serde::de::Error::custom)
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

    const MINIMAL_HOST_SPEC: &[u8] = br#"{"allowedDomains":["system"],"budget":{},"defaultDomain":"system","defaultUserRef":null,"deviceAttachments":[],"isolationPosture":null,"networkAttachments":[],"volumeAttachmentDefaults":[]}"#;

    const USER_ONLY_HOST_SPEC: &[u8] = br#"{"allowedDomains":["user"],"budget":{},"defaultDomain":"user","defaultUserRef":"User/alice","deviceAttachments":[],"isolationPosture":"none","networkAttachments":[],"volumeAttachmentDefaults":[]}"#;

    #[test]
    fn schema_vectors_pin_the_minimal_and_user_only_host_base_specs() {
        let minimal = HostSpec::system_default();
        assert_eq!(canonical_json_bytes(&minimal).unwrap(), MINIMAL_HOST_SPEC);
        let parsed: HostSpec = serde_json::from_slice(MINIMAL_HOST_SPEC).unwrap();
        assert_eq!(parsed, minimal);

        let user_only = HostSpec::user_only(ResourceRef::parse("User/alice").unwrap()).unwrap();
        assert_eq!(
            canonical_json_bytes(&user_only).unwrap(),
            USER_ONLY_HOST_SPEC
        );
        let parsed: HostSpec = serde_json::from_slice(USER_ONLY_HOST_SPEC).unwrap();
        assert_eq!(parsed, user_only);
        assert!(parsed.is_no_isolation());
    }

    #[test]
    fn base_object_never_carries_a_universal_or_layer_three_field() {
        let base = to_base_object(&HostSpec::system_default()).unwrap();
        for reserved in ["providerRef", "updatePolicy", "provider"] {
            assert!(base.get(reserved).is_none());
        }
    }

    #[test]
    fn the_no_isolation_posture_is_required_and_cannot_be_evaded() {
        let user_only_policy = ExecutionPolicy::new(
            ExecutionDomain::User,
            vec![ExecutionDomain::User],
            Some(ResourceRef::parse("User/alice").unwrap()),
            BudgetSpec::default(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(
            HostSpec::new(user_only_policy, None),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert_eq!(
            HostSpec::new(
                ExecutionPolicy::system_default(),
                Some(IsolationPosture::NoIsolation)
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
    }

    #[test]
    fn unknown_base_fields_are_rejected() {
        let with_unknown = br#"{"defaultDomain":"system","command":"/bin/sh"}"#;
        assert!(serde_json::from_slice::<HostSpec>(with_unknown).is_err());
        let restated_provider =
            br#"{"defaultDomain":"system","providerRef":"Provider/system-core"}"#;
        assert!(serde_json::from_slice::<HostSpec>(restated_provider).is_err());
    }

    #[test]
    fn diagnostics_stay_redacted() {
        let spec = HostSpec::user_only(ResourceRef::parse("User/alice").unwrap()).unwrap();
        assert_eq!(format!("{spec:?}"), "HostSpec(<redacted>)");
    }
}
