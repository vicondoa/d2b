#![cfg(feature = "v2-identity")]

use d2b_contracts::v2_identity::{
    CanonicalEncoding, ConfiguredProviderId, IdentityDomain, IdentityError,
    LINUX_UNIX_PATH_MAX_BYTES, ProviderId, ProviderType, RealmId, RealmLabel, RealmPath, RoleId,
    RoleKind, SHORT_ID_LEN, ShortId, WorkloadId, WorkloadName, bytes_to_hex,
    recompute_canonical_identity, unix_path_headroom, validate_global_identities,
    verify_canonical_identity,
};
use serde::Deserialize;
use std::str::FromStr;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Vectors {
    schema_version: u32,
    short_id_proof: ShortIdProof,
    valid: Vec<Vector>,
    partition_boundary: Vec<Vector>,
    malformed: Vec<Malformed>,
    malformed_short_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ShortIdProof {
    alphabet: String,
    length_bytes: usize,
    contains_nul: bool,
    linux_pathname_max_bytes: usize,
    remaining_bytes_after_single_id: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Vector {
    case: String,
    domain: String,
    parts: Vec<String>,
    encoded: String,
    encoded_hex: String,
    sha256: String,
    short_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Malformed {
    case: String,
    encoded: Option<String>,
    #[serde(rename = "encodedHex")]
    encoded_hex: Option<String>,
}

fn vectors() -> Vectors {
    serde_json::from_str(include_str!(
        "../../../docs/reference/v2-identity-vectors.json"
    ))
    .expect("identity vectors must be valid JSON")
}

fn malformed_input(row: &Malformed) -> String {
    match (&row.encoded, &row.encoded_hex) {
        (Some(encoded), None) => encoded.clone(),
        (None, Some(encoded_hex)) => {
            assert_eq!(encoded_hex.len() % 2, 0, "{} hex length", row.case);
            let bytes = encoded_hex
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| {
                    let pair = std::str::from_utf8(pair).expect("ASCII hex pair");
                    u8::from_str_radix(pair, 16).expect("valid hex pair")
                })
                .collect::<Vec<_>>();
            String::from_utf8(bytes).expect("malformed vector remains UTF-8")
        }
        _ => panic!(
            "{} must contain exactly one of encoded or encodedHex",
            row.case
        ),
    }
}

#[test]
fn canonical_vectors_match_encoding_digest_and_short_id() {
    let vectors = vectors();
    assert_eq!(vectors.schema_version, 1);
    for vector in &vectors.valid {
        let domain = IdentityDomain::from_str(&vector.domain)
            .unwrap_or_else(|error| panic!("{}: {error}", vector.case));
        let encoding = CanonicalEncoding::new(domain, vector.parts.clone())
            .unwrap_or_else(|error| panic!("{}: {error}", vector.case));
        assert_eq!(encoding.encode(), vector.encoded, "{} encoded", vector.case);
        assert_eq!(
            bytes_to_hex(encoding.encode().as_bytes()),
            vector.encoded_hex,
            "{} bytes",
            vector.case
        );
        assert_eq!(
            bytes_to_hex(&encoding.digest()),
            vector.sha256,
            "{} digest",
            vector.case
        );
        assert_eq!(
            encoding.short_id().as_str(),
            vector.short_id,
            "{} short ID",
            vector.case
        );
        assert_eq!(
            CanonicalEncoding::parse(&vector.encoded).unwrap(),
            encoding,
            "{} parse",
            vector.case
        );
        assert_eq!(
            recompute_canonical_identity(&vector.encoded)
                .unwrap_or_else(|error| panic!("{}: {error}", vector.case))
                .short_id()
                .as_str(),
            vector.short_id,
            "{} recomputation",
            vector.case
        );
    }
}

#[test]
fn closed_provider_and_role_sets_are_completely_vectored() {
    let vectors = vectors();
    let provider_values: Vec<_> = ProviderType::ALL
        .into_iter()
        .map(|value| value.as_str())
        .collect();
    let role_values: Vec<_> = RoleKind::ALL
        .into_iter()
        .map(|value| value.as_str())
        .collect();
    let provider_vectors: Vec<_> = vectors
        .valid
        .iter()
        .filter(|row| row.case.starts_with("provider-") && row.case != "provider-instance-rename")
        .map(|row| row.parts[1].as_str())
        .collect();
    let role_vectors: Vec<_> = vectors
        .valid
        .iter()
        .filter(|row| row.case.starts_with("role-"))
        .map(|row| row.parts[2].as_str())
        .collect();
    assert_eq!(provider_values, provider_vectors);
    assert_eq!(role_values, role_vectors);
    assert!(serde_json::from_str::<ProviderType>("\"unknown\"").is_err());
    assert!(serde_json::from_str::<RoleKind>("\"unknown\"").is_err());
    assert_eq!(
        serde_json::to_string(&IdentityDomain::Realm).unwrap(),
        "\"d2b-v2:realm\""
    );
    assert_eq!(
        serde_json::from_str::<IdentityDomain>("\"d2b-v2:role\"").unwrap(),
        IdentityDomain::Role
    );
    assert!(serde_json::from_str::<IdentityDomain>("\"role\"").is_err());
}

#[test]
fn partition_boundaries_are_unambiguous() {
    let vectors = vectors();
    assert_eq!(vectors.partition_boundary.len(), 2);
    for vector in &vectors.partition_boundary {
        let domain = IdentityDomain::from_str(&vector.domain).unwrap();
        let encoding = CanonicalEncoding::new(domain, vector.parts.clone()).unwrap();
        assert_eq!(encoding.encode(), vector.encoded, "{}", vector.case);
        assert_eq!(
            bytes_to_hex(&encoding.digest()),
            vector.sha256,
            "{}",
            vector.case
        );
        assert_eq!(
            encoding.short_id().as_str(),
            vector.short_id,
            "{}",
            vector.case
        );
        assert!(encoding.recompute().is_err(), "{}", vector.case);
    }
    assert_ne!(
        vectors.partition_boundary[0].encoded,
        vectors.partition_boundary[1].encoded
    );
    assert_ne!(
        vectors.partition_boundary[0].short_id,
        vectors.partition_boundary[1].short_id
    );
}

#[test]
fn malformed_and_noncanonical_inputs_fail_closed() {
    let vectors = vectors();
    for row in &vectors.malformed {
        let encoded = malformed_input(row);
        assert!(
            recompute_canonical_identity(&encoded).is_err(),
            "{} unexpectedly recomputed",
            row.case
        );
    }
    for value in &vectors.malformed_short_ids {
        assert!(
            ShortId::parse(value).is_err(),
            "malformed short ID unexpectedly parsed"
        );
    }
}

#[test]
fn human_names_paths_and_deserialization_are_validated() {
    assert!(RealmLabel::parse("personal-dev").is_ok());
    assert!(WorkloadName::parse("workload-1").is_ok());
    assert!(ConfiguredProviderId::parse("primary-runtime").is_ok());
    for invalid in [
        "",
        "Upper",
        "with.dot",
        "with_underscore",
        "é",
        &"a".repeat(64),
    ] {
        assert!(RealmLabel::parse(invalid).is_err());
        assert!(WorkloadName::parse(invalid).is_err());
        assert!(ConfiguredProviderId::parse(invalid).is_err());
    }

    let root = RealmPath::root();
    let dev = RealmPath::child(&RealmLabel::parse("dev").unwrap(), &root);
    let leaf = RealmPath::child(&RealmLabel::parse("personal-dev").unwrap(), &dev);
    assert_eq!(leaf.as_str(), "personal-dev.dev.local-root");
    assert!(RealmPath::parse(leaf.as_str()).is_ok());
    for invalid in [
        "",
        "local-root.dev",
        "dev..local-root",
        "dev.local-root.",
        "dev.local-root.d2b",
        "Dév.local-root",
    ] {
        assert!(RealmPath::parse(invalid).is_err(), "{invalid}");
    }

    assert!(serde_json::from_str::<RealmPath>("\"dev.local-root\"").is_ok());
    assert!(serde_json::from_str::<RealmPath>("\"local-root.dev\"").is_err());
    assert!(serde_json::from_str::<WorkloadName>("\"Upper\"").is_err());
}

#[test]
fn renames_create_new_runtime_ids() {
    let realm_a = RealmId::derive(&RealmPath::parse("dev.local-root").unwrap());
    let realm_b = RealmId::derive(&RealmPath::parse("engineering.local-root").unwrap());
    assert_ne!(realm_a, realm_b);

    let workload_a = WorkloadId::derive(&realm_a, &WorkloadName::parse("personal-dev").unwrap());
    let workload_b =
        WorkloadId::derive(&realm_a, &WorkloadName::parse("personal-dev-next").unwrap());
    assert_ne!(workload_a, workload_b);

    let provider_a = ProviderId::derive(
        &realm_a,
        ProviderType::Runtime,
        &ConfiguredProviderId::parse("primary").unwrap(),
    );
    let provider_b = ProviderId::derive(
        &realm_a,
        ProviderType::Runtime,
        &ConfiguredProviderId::parse("secondary").unwrap(),
    );
    assert_ne!(provider_a, provider_b);

    let role_a = RoleId::derive(&realm_a, &workload_a, RoleKind::CloudHypervisor);
    let role_b = RoleId::derive(&realm_a, &workload_a, RoleKind::QemuMedia);
    assert_ne!(role_a, role_b);
}

#[test]
fn recomputation_verifies_claimed_id_without_echoing_input() {
    let vector = vectors()
        .valid
        .into_iter()
        .find(|row| row.case == "realm-dev")
        .unwrap();
    let expected = ShortId::parse(vector.short_id).unwrap();
    verify_canonical_identity(&vector.encoded, &expected).unwrap();
    let wrong = ShortId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap();
    let error = verify_canonical_identity(&vector.encoded, &wrong).unwrap_err();
    assert_eq!(error, IdentityError::RecomputedIdMismatch);
    assert_eq!(
        error.to_string(),
        "canonical identity recomputation mismatch"
    );
    assert!(!error.to_string().contains("dev.local-root"));
}

#[test]
fn duplicate_provider_and_global_short_id_collisions_are_rejected() {
    let realm = RealmId::parse("aaaaaaaaaaaaaaaaaaaa").unwrap();
    let workload = WorkloadId::parse("baaaaaaaaaaaaaaaaaaq").unwrap();
    let provider = ProviderId::parse("caaaaaaaaaaaaaaaaaaq").unwrap();
    let role = RoleId::parse("daaaaaaaaaaaaaaaaaaq").unwrap();
    assert!(
        validate_global_identities(
            std::slice::from_ref(&realm),
            std::slice::from_ref(&workload),
            std::slice::from_ref(&provider),
            std::slice::from_ref(&role),
        )
        .is_ok()
    );
    assert_eq!(
        validate_global_identities(&[], &[], &[provider.clone(), provider], &[]),
        Err(IdentityError::DuplicateProviderId)
    );

    let colliding_workload = WorkloadId::parse(realm.as_str()).unwrap();
    assert_eq!(
        validate_global_identities(&[realm], &[colliding_workload], &[], &[]),
        Err(IdentityError::ShortIdCollision)
    );
}

#[test]
fn short_ids_and_generic_path_proof_preserve_linux_headroom() {
    let vectors = vectors();
    let proof = vectors.short_id_proof;
    assert_eq!(proof.alphabet, "abcdefghijklmnopqrstuvwxyz234567");
    assert_eq!(proof.length_bytes, SHORT_ID_LEN);
    assert!(!proof.contains_nul);
    assert_eq!(proof.linux_pathname_max_bytes, LINUX_UNIX_PATH_MAX_BYTES);
    assert_eq!(
        proof.remaining_bytes_after_single_id,
        LINUX_UNIX_PATH_MAX_BYTES - SHORT_ID_LEN
    );
    for vector in vectors.valid {
        let id = ShortId::parse(vector.short_id).unwrap();
        assert!(id.has_path_safe_shape());
        assert!(!id.as_str().as_bytes().contains(&0));
    }
    assert_eq!(unix_path_headroom(&"x".repeat(107)), Ok(0));
    assert_eq!(
        unix_path_headroom(&"x".repeat(108)),
        Err(IdentityError::UnixPathTooLong)
    );
    assert_eq!(
        unix_path_headroom("prefix\0suffix"),
        Err(IdentityError::UnixPathContainsNul)
    );
}
