//! Tree-only route advertisements for ADR 0043 realm routing.

use crate::capability::CapabilitySet;
use crate::enrollment::{KeyFingerprint, RealmKeyRole};
use crate::ids::{ControllerGenerationId, CorrelationId, RealmId, RouteId};
use crate::realm::RealmPath;
use crate::token::ProtocolToken;
use schemars::{
    JsonSchema,
    r#gen::SchemaGenerator,
    schema::{InstanceType, Schema, SchemaObject, SingleOrVec, StringValidation},
};
use serde::{Deserialize, Deserializer, Serialize};

/// Maximum routes in one advertisement.
pub const MAX_ADVERTISED_ROUTES: usize = 64;
/// Maximum bytes in a detached signature reference/fingerprint.
pub const MAX_SIGNATURE_REF_LEN: usize = 128;

/// Detached signature reference or fingerprint. This is not private key
/// material and is redacted from `Debug`.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SignatureRef(String);

impl SignatureRef {
    /// Validate a bounded printable signature reference.
    pub fn parse(raw: impl Into<String>) -> Option<Self> {
        let raw = raw.into();
        if raw.is_empty()
            || raw.len() > MAX_SIGNATURE_REF_LEN
            || !raw.as_bytes()[0].is_ascii_alphanumeric()
            || !raw
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '-' | '_' | '.'))
        {
            return None;
        }
        Some(Self(raw))
    }

    /// Borrow the signature reference.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for SignatureRef {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "SignatureRef(<{} bytes>)", self.0.len())
    }
}

impl<'de> Deserialize<'de> for SignatureRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?)
            .ok_or_else(|| serde::de::Error::custom("invalid signature reference"))
    }
}

impl JsonSchema for SignatureRef {
    fn schema_name() -> String {
        "SignatureRef".to_owned()
    }

    fn json_schema(_gen: &mut SchemaGenerator) -> Schema {
        Schema::Object(SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
            string: Some(Box::new(StringValidation {
                max_length: Some(MAX_SIGNATURE_REF_LEN as u32),
                min_length: Some(1),
                pattern: Some("^[A-Za-z0-9][A-Za-z0-9:._-]*$".to_owned()),
            })),
            ..Default::default()
        })
    }
}

/// Parent/child tree edge metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealmTreeEdge {
    /// Parent realm.
    pub parent: RealmPath,
    /// Direct child realm.
    pub child: RealmPath,
}

impl RealmTreeEdge {
    /// Construct only when `child` is exactly one label below `parent`.
    pub fn new(parent: RealmPath, child: RealmPath) -> Option<Self> {
        if child.is_direct_child_of(&parent) {
            Some(Self { parent, child })
        } else {
            None
        }
    }
}

impl<'de> Deserialize<'de> for RealmTreeEdge {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            parent: RealmPath,
            child: RealmPath,
        }
        let raw = Raw::deserialize(deserializer)?;
        Self::new(raw.parent, raw.child)
            .ok_or_else(|| serde::de::Error::custom("child realm is not a direct child of parent"))
    }
}

/// One descendant route advertised by a realm controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DescendantRoute {
    /// Route id for this descendant path.
    pub route_id: RouteId,
    /// Descendant reachable below the advertising realm.
    pub descendant: RealmPath,
    /// Next child label below the advertiser.
    pub next_hop_child: RealmId,
    /// Positive capabilities reachable on this route.
    pub capabilities: CapabilitySet,
}

/// Signature metadata for a route advertisement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteSignature {
    /// Signature algorithm token.
    pub algorithm: ProtocolToken,
    /// Controller key role used for signing.
    pub key_role: RealmKeyRole,
    /// Fingerprint of the signing key; no key bytes.
    pub signing_key_fingerprint: KeyFingerprint,
    /// Detached signature reference or bounded signature fingerprint.
    pub signature_ref: SignatureRef,
}

/// Signed, expiring descendant-only route advertisement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouteAdvertisement {
    /// Advertising realm.
    pub advertising_realm: RealmPath,
    /// Parent/child edge that authorizes this advertisement.
    pub tree_edge: RealmTreeEdge,
    /// Controller generation that signed the advertisement.
    pub controller_generation: ControllerGenerationId,
    /// Bounded correlation id shared across route-decision audit records.
    pub correlation_id: CorrelationId,
    /// Routes below `advertising_realm`.
    pub routes: Vec<DescendantRoute>,
    /// Issue time as Unix seconds.
    pub issued_at_unix_seconds: u64,
    /// Expiry time as Unix seconds. Must be greater than issue time.
    pub expires_at_unix_seconds: u64,
    /// Signature metadata.
    pub signature: RouteSignature,
}

impl RouteAdvertisement {
    /// Validate descendant-only and bounded route invariants.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        advertising_realm: RealmPath,
        tree_edge: RealmTreeEdge,
        controller_generation: ControllerGenerationId,
        correlation_id: CorrelationId,
        routes: Vec<DescendantRoute>,
        issued_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
        signature: RouteSignature,
    ) -> Option<Self> {
        if tree_edge.child != advertising_realm
            || routes.is_empty()
            || routes.len() > MAX_ADVERTISED_ROUTES
            || expires_at_unix_seconds <= issued_at_unix_seconds
            || routes.iter().any(|route| {
                !route.descendant.is_descendant_of(&advertising_realm)
                    || next_hop_child(&route.descendant, &advertising_realm)
                        != Some(&route.next_hop_child)
            })
        {
            return None;
        }
        Some(Self {
            advertising_realm,
            tree_edge,
            controller_generation,
            correlation_id,
            routes,
            issued_at_unix_seconds,
            expires_at_unix_seconds,
            signature,
        })
    }
}

impl<'de> Deserialize<'de> for RouteAdvertisement {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Raw {
            advertising_realm: RealmPath,
            tree_edge: RealmTreeEdge,
            controller_generation: ControllerGenerationId,
            correlation_id: CorrelationId,
            routes: Vec<DescendantRoute>,
            issued_at_unix_seconds: u64,
            expires_at_unix_seconds: u64,
            signature: RouteSignature,
        }

        let raw = Raw::deserialize(deserializer)?;
        Self::new(
            raw.advertising_realm,
            raw.tree_edge,
            raw.controller_generation,
            raw.correlation_id,
            raw.routes,
            raw.issued_at_unix_seconds,
            raw.expires_at_unix_seconds,
            raw.signature,
        )
        .ok_or_else(|| serde::de::Error::custom("invalid route advertisement shape"))
    }
}

fn next_hop_child<'a>(descendant: &'a RealmPath, advertiser: &RealmPath) -> Option<&'a RealmId> {
    let descendant_index = descendant
        .labels()
        .len()
        .checked_sub(advertiser.labels().len() + 1)?;
    descendant.labels().get(descendant_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::Capability;
    use crate::ids::RealmId;
    use schemars::schema_for;

    fn fp() -> KeyFingerprint {
        KeyFingerprint::parse(format!("sha256:{}", "b".repeat(64))).unwrap()
    }

    fn realm(labels: &[&str]) -> RealmPath {
        RealmPath::new(
            labels
                .iter()
                .map(|label| RealmId::parse(*label).unwrap())
                .collect(),
        )
        .unwrap()
    }

    fn signature() -> RouteSignature {
        RouteSignature {
            algorithm: ProtocolToken::parse("ed25519-v1").unwrap(),
            key_role: RealmKeyRole::ControllerGeneration,
            signing_key_fingerprint: fp(),
            signature_ref: SignatureRef::parse("sig-1").unwrap(),
        }
    }

    #[test]
    fn signature_ref_rejects_leading_punctuation() {
        assert!(SignatureRef::parse("sig-1").is_some());
        assert!(SignatureRef::parse("-sig-1").is_none());
        assert!(SignatureRef::parse(".sig-1").is_none());
        assert!(SignatureRef::parse("_sig-1").is_none());
        assert!(SignatureRef::parse(":sig-1").is_none());
    }

    #[test]
    fn route_advertisement_accepts_descendants_only() {
        let parent = realm(&["work"]);
        let child = realm(&["payments", "work"]);
        let edge = RealmTreeEdge::new(parent, child.clone()).unwrap();
        let route = DescendantRoute {
            route_id: RouteId::parse("route-1").unwrap(),
            descendant: realm(&["api", "payments", "work"]),
            next_hop_child: RealmId::parse("api").unwrap(),
            capabilities: CapabilitySet::empty().with(Capability::Exec),
        };
        assert!(
            RouteAdvertisement::new(
                child,
                edge,
                ControllerGenerationId::parse("gen-1").unwrap(),
                CorrelationId::parse("corr-1").unwrap(),
                vec![route],
                10,
                20,
                signature(),
            )
            .is_some()
        );
    }

    #[test]
    fn route_advertisement_rejects_sibling_parent_and_unbounded_routes() {
        let work = realm(&["work"]);
        let payments = realm(&["payments", "work"]);
        let edge = RealmTreeEdge::new(work, payments.clone()).unwrap();
        let sibling = DescendantRoute {
            route_id: RouteId::parse("route-1").unwrap(),
            descendant: realm(&["dev"]),
            next_hop_child: RealmId::parse("dev").unwrap(),
            capabilities: CapabilitySet::empty(),
        };
        assert!(
            RouteAdvertisement::new(
                payments.clone(),
                edge.clone(),
                ControllerGenerationId::parse("gen-1").unwrap(),
                CorrelationId::parse("corr-1").unwrap(),
                vec![sibling],
                10,
                20,
                signature(),
            )
            .is_none()
        );

        let bad_next_hop = DescendantRoute {
            route_id: RouteId::parse("route-2").unwrap(),
            descendant: realm(&["api", "payments", "work"]),
            next_hop_child: RealmId::parse("payments").unwrap(),
            capabilities: CapabilitySet::empty(),
        };
        assert!(
            RouteAdvertisement::new(
                payments.clone(),
                edge.clone(),
                ControllerGenerationId::parse("gen-1").unwrap(),
                CorrelationId::parse("corr-1").unwrap(),
                vec![bad_next_hop],
                10,
                20,
                signature(),
            )
            .is_none()
        );

        let too_many = (0..=MAX_ADVERTISED_ROUTES)
            .map(|i| DescendantRoute {
                route_id: RouteId::parse(format!("route-{i}")).unwrap(),
                descendant: realm(&[&format!("api{i}"), "payments", "work"]),
                next_hop_child: RealmId::parse(format!("api{i}")).unwrap(),
                capabilities: CapabilitySet::empty(),
            })
            .collect::<Vec<_>>();
        assert!(
            RouteAdvertisement::new(
                payments,
                edge,
                ControllerGenerationId::parse("gen-1").unwrap(),
                CorrelationId::parse("corr-1").unwrap(),
                too_many,
                10,
                20,
                signature(),
            )
            .is_none()
        );
    }

    #[test]
    fn route_advertisement_decode_rejects_unknown_fields_and_bad_expiry() {
        let json = "{\"advertisingRealm\":[\"payments\",\"work\"],\
            \"treeEdge\":{\"parent\":[\"work\"],\"child\":[\"payments\",\"work\"]},\
            \"controllerGeneration\":\"gen-1\",\"correlationId\":\"corr-1\",\
            \"routes\":[{\"routeId\":\"route-1\",\"descendant\":[\"api\",\"payments\",\"work\"],\
            \"nextHopChild\":\"api\",\"capabilities\":[\"exec\"]}],\
            \"issuedAtUnixSeconds\":20,\"expiresAtUnixSeconds\":10,\
            \"signature\":{\"algorithm\":\"ed25519-v1\",\"keyRole\":\"controller-generation\",\
            \"signingKeyFingerprint\":\"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\",\
            \"signatureRef\":\"sig-1\"}}";
        assert!(serde_json::from_str::<RouteAdvertisement>(json).is_err());

        let edge = "{\"parent\":[\"work\"],\"child\":[\"payments\",\"work\"],\"extra\":1}";
        assert!(serde_json::from_str::<RealmTreeEdge>(edge).is_err());
    }

    #[test]
    fn route_schema_is_generated() {
        assert!(schema_for!(RouteAdvertisement).schema.metadata.is_some());
    }
}
