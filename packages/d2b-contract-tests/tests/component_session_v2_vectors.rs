use std::collections::BTreeSet;

use d2b_contract_tests::read_repo_file;
use d2b_contracts::v2_component_session::{
    BootstrapPskBinding, BootstrapPskState, EndpointPurpose, HandshakeRejectReason, NoiseProfile,
    OperationId,
};
use serde_json::Value;
use snow::{
    Builder, HandshakeState,
    params::{DHChoice, NoiseParams},
    resolvers::{CryptoResolver, DefaultResolver},
};

fn hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0, "hex must have complete bytes");
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).unwrap();
            u8::from_str_radix(text, 16).unwrap()
        })
        .collect()
}

fn field<'a>(value: &'a Value, name: &str) -> &'a str {
    value[name]
        .as_str()
        .unwrap_or_else(|| panic!("fixture field {name} must be a string"))
}

fn optional_hex(value: &Value, name: &str) -> Option<Vec<u8>> {
    value[name].as_str().map(hex)
}

fn derive_public(private: &[u8]) -> Vec<u8> {
    let mut dh = DefaultResolver
        .resolve_dh(&DHChoice::Curve25519)
        .expect("snow default resolver must provide 25519");
    dh.set(private);
    dh.pubkey().to_vec()
}

fn declared_public_matches(private: &[u8], declared_public: &[u8]) -> bool {
    derive_public(private) == declared_public
}

#[derive(Clone)]
struct VectorMaterial {
    prologue: Vec<u8>,
    initiator_ephemeral: Vec<u8>,
    initiator_ephemeral_public: Vec<u8>,
    responder_ephemeral: Vec<u8>,
    responder_ephemeral_public: Vec<u8>,
    initiator_static: Option<Vec<u8>>,
    initiator_public: Option<Vec<u8>>,
    responder_static: Option<Vec<u8>>,
    responder_public: Option<Vec<u8>>,
    psk: Option<[u8; 32]>,
}

impl VectorMaterial {
    fn from_fixture(vector: &Value) -> Self {
        Self {
            prologue: hex(field(vector, "prologueHex")),
            initiator_ephemeral: hex(field(vector, "initiatorEphemeralPrivateHex")),
            initiator_ephemeral_public: hex(field(vector, "initiatorEphemeralPublicHex")),
            responder_ephemeral: hex(field(vector, "responderEphemeralPrivateHex")),
            responder_ephemeral_public: hex(field(vector, "responderEphemeralPublicHex")),
            initiator_static: optional_hex(vector, "initiatorStaticPrivateHex"),
            initiator_public: optional_hex(vector, "initiatorStaticPublicHex"),
            responder_static: optional_hex(vector, "responderStaticPrivateHex"),
            responder_public: optional_hex(vector, "responderStaticPublicHex"),
            psk: optional_hex(vector, "pskHex").map(|value| <[u8; 32]>::try_from(value).unwrap()),
        }
    }
}

fn builder<'a>(vector: &'a Value, initiator: bool, material: &'a VectorMaterial) -> HandshakeState {
    let params: NoiseParams = field(vector, "protocolName").parse().unwrap();
    let mut builder = Builder::new(params)
        .prologue(&material.prologue)
        .unwrap()
        .fixed_ephemeral_key_for_testing_only(if initiator {
            &material.initiator_ephemeral
        } else {
            &material.responder_ephemeral
        });
    match field(vector, "protocolName") {
        "Noise_NN_25519_ChaChaPoly_SHA256" => {}
        "Noise_KK_25519_ChaChaPoly_SHA256" => {
            builder = if initiator {
                builder
                    .local_private_key(material.initiator_static.as_deref().unwrap())
                    .unwrap()
                    .remote_public_key(material.responder_public.as_deref().unwrap())
                    .unwrap()
            } else {
                builder
                    .local_private_key(material.responder_static.as_deref().unwrap())
                    .unwrap()
                    .remote_public_key(material.initiator_public.as_deref().unwrap())
                    .unwrap()
            };
        }
        "Noise_IKpsk2_25519_ChaChaPoly_SHA256" => {
            builder = if initiator {
                builder
                    .local_private_key(material.initiator_static.as_deref().unwrap())
                    .unwrap()
                    .remote_public_key(material.responder_public.as_deref().unwrap())
                    .unwrap()
                    .psk(2, material.psk.as_ref().unwrap())
                    .unwrap()
            } else {
                builder
                    .local_private_key(material.responder_static.as_deref().unwrap())
                    .unwrap()
                    .psk(2, material.psk.as_ref().unwrap())
                    .unwrap()
            };
        }
        profile => panic!("unapproved Noise profile {profile}"),
    }
    if initiator {
        builder.build_initiator().unwrap()
    } else {
        builder.build_responder().unwrap()
    }
}

#[test]
fn committed_noise_vectors_verify_with_pinned_snow() {
    let fixture: Value = serde_json::from_str(&read_repo_file(
        "docs/reference/component-session-v2-vectors.json",
    ))
    .unwrap();
    assert_eq!(
        fixture["contract"],
        "d2b-component-session-v2-noise-vectors"
    );
    assert_eq!(fixture["formatVersion"], 1);
    assert_eq!(fixture["snowVersion"], "0.10.0");
    let vectors = fixture["vectors"].as_array().unwrap();
    assert_eq!(vectors.len(), EndpointPurpose::ALL.len() + 1);

    let mut purposes = BTreeSet::new();
    let mut fixture_ids = BTreeSet::new();
    let mut profiles = BTreeSet::new();
    let mut purpose_classes = BTreeSet::new();
    for vector in vectors {
        purposes.insert(field(vector, "purpose"));
        fixture_ids.insert(field(vector, "fixtureId"));
        profiles.insert(field(vector, "protocolName"));
        purpose_classes.insert(field(vector, "purposeClass"));

        let material = VectorMaterial::from_fixture(vector);
        assert!(
            declared_public_matches(
                &material.initiator_ephemeral,
                &material.initiator_ephemeral_public
            ),
            "{} initiator ephemeral public key",
            field(vector, "purpose")
        );
        assert!(
            declared_public_matches(
                &material.responder_ephemeral,
                &material.responder_ephemeral_public
            ),
            "{} responder ephemeral public key",
            field(vector, "purpose")
        );
        match (&material.initiator_static, &material.initiator_public) {
            (Some(private), Some(public)) => assert!(
                declared_public_matches(private, public),
                "{} initiator static public key",
                field(vector, "purpose")
            ),
            (None, None) => {}
            _ => panic!(
                "{} has incomplete initiator static key material",
                field(vector, "purpose")
            ),
        }
        match (&material.responder_static, &material.responder_public) {
            (Some(private), Some(public)) => assert!(
                declared_public_matches(private, public),
                "{} responder static public key",
                field(vector, "purpose")
            ),
            (None, None) => {}
            _ => panic!(
                "{} has incomplete responder static key material",
                field(vector, "purpose")
            ),
        }
        let mut initiator = builder(vector, true, &material);
        let mut responder = builder(vector, false, &material);
        let payloads = vector["handshakePayloadsHex"].as_array().unwrap();
        let expected_messages = vector["handshakeMessagesHex"].as_array().unwrap();
        let mut message = vec![0; 65_535];
        let mut plaintext = vec![0; 65_535];

        let payload_1 = hex(payloads[0].as_str().unwrap());
        let written = initiator.write_message(&payload_1, &mut message).unwrap();
        assert_eq!(
            &message[..written],
            hex(expected_messages[0].as_str().unwrap()),
            "{} initiator handshake",
            field(vector, "purpose")
        );
        let read = responder
            .read_message(&message[..written], &mut plaintext)
            .unwrap();
        assert_eq!(&plaintext[..read], payload_1);

        let payload_2 = hex(payloads[1].as_str().unwrap());
        let written = responder.write_message(&payload_2, &mut message).unwrap();
        assert_eq!(
            &message[..written],
            hex(expected_messages[1].as_str().unwrap()),
            "{} responder handshake",
            field(vector, "purpose")
        );
        let read = initiator
            .read_message(&message[..written], &mut plaintext)
            .unwrap();
        assert_eq!(&plaintext[..read], payload_2);
        assert_eq!(
            initiator.get_handshake_hash(),
            hex(field(vector, "transcriptHashHex"))
        );
        assert_eq!(
            initiator.get_handshake_hash(),
            responder.get_handshake_hash()
        );

        let expected_i_key = hex(field(&vector["transportKeysHex"], "initiatorToResponder"));
        let expected_r_key = hex(field(&vector["transportKeysHex"], "responderToInitiator"));
        let split = initiator.dangerously_get_raw_split();
        assert_eq!(split.0.as_slice(), expected_i_key);
        assert_eq!(split.1.as_slice(), expected_r_key);
        assert_eq!(responder.dangerously_get_raw_split(), split);

        let mut initiator = initiator.into_transport_mode().unwrap();
        let mut responder = responder.into_transport_mode().unwrap();
        let records = &vector["firstRecords"];
        let initiator_plaintext = hex(field(records, "initiatorPlaintextHex"));
        let written = initiator
            .write_message(&initiator_plaintext, &mut message)
            .unwrap();
        assert_eq!(
            &message[..written],
            hex(field(records, "initiatorCiphertextHex"))
        );
        let read = responder
            .read_message(&message[..written], &mut plaintext)
            .unwrap();
        assert_eq!(&plaintext[..read], initiator_plaintext);
        let responder_plaintext = hex(field(records, "responderPlaintextHex"));
        let written = responder
            .write_message(&responder_plaintext, &mut message)
            .unwrap();
        assert_eq!(
            &message[..written],
            hex(field(records, "responderCiphertextHex"))
        );
        let read = initiator
            .read_message(&message[..written], &mut plaintext)
            .unwrap();
        assert_eq!(&plaintext[..read], responder_plaintext);

        let mutations: BTreeSet<_> = vector["rejectionMutations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect();
        for required in [
            "transcript-downgrade",
            "cross-purpose",
            "purpose-class-mismatch",
            "role-mismatch",
            "schema-mismatch",
            "limit-mismatch",
            "channel-binding-mismatch",
        ] {
            assert!(
                mutations.contains(required),
                "{} {required}",
                field(vector, "purpose")
            );
        }
        if field(vector, "protocolName") == NoiseProfile::Ikpsk2_25519ChaChaPolySha256.as_str() {
            for required in ["wrong-operation", "expired-psk", "psk-replay"] {
                assert!(
                    mutations.contains(required),
                    "{} {required}",
                    field(vector, "purpose")
                );
            }
        }
    }

    let expected_purposes: BTreeSet<_> = EndpointPurpose::ALL
        .iter()
        .map(|purpose| purpose.as_str())
        .collect();
    assert_eq!(purposes, expected_purposes);
    assert_eq!(
        fixture_ids,
        BTreeSet::from([
            "component-session-v2-bootstrap-ikpsk2",
            "component-session-v2-enrolled-kk",
            "component-session-v2-local-nn",
        ])
    );
    assert_eq!(
        profiles,
        BTreeSet::from([
            "Noise_IKpsk2_25519_ChaChaPoly_SHA256",
            "Noise_KK_25519_ChaChaPoly_SHA256",
            "Noise_NN_25519_ChaChaPoly_SHA256",
        ])
    );
    assert_eq!(
        purpose_classes,
        BTreeSet::from(["bootstrap", "enrolled", "local"])
    );
}

#[test]
fn declared_noise_public_key_corruption_is_rejected() {
    let fixture: Value = serde_json::from_str(&read_repo_file(
        "docs/reference/component-session-v2-vectors.json",
    ))
    .unwrap();
    for vector in fixture["vectors"].as_array().unwrap() {
        for (private_field, public_field) in [
            (
                "initiatorEphemeralPrivateHex",
                "initiatorEphemeralPublicHex",
            ),
            (
                "responderEphemeralPrivateHex",
                "responderEphemeralPublicHex",
            ),
            ("initiatorStaticPrivateHex", "initiatorStaticPublicHex"),
            ("responderStaticPrivateHex", "responderStaticPublicHex"),
        ] {
            let Some(private) = optional_hex(vector, private_field) else {
                assert!(vector[public_field].is_null());
                continue;
            };
            let mut corrupted = hex(field(vector, public_field));
            corrupted[0] ^= 1;
            assert!(
                !declared_public_matches(&private, &corrupted),
                "{} accepted corrupted {public_field}",
                field(vector, "purpose")
            );
        }
    }
}

#[test]
fn bootstrap_fixture_mutations_execute_typed_admission_state() {
    let fixture: Value = serde_json::from_str(&read_repo_file(
        "docs/reference/component-session-v2-vectors.json",
    ))
    .unwrap();
    for vector in fixture["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|vector| {
            field(vector, "protocolName") == NoiseProfile::Ikpsk2_25519ChaChaPolySha256.as_str()
        })
    {
        let fixture_binding = &vector["bootstrapBinding"];
        let binding = BootstrapPskBinding {
            operation_id: OperationId::new(hex(field(fixture_binding, "operationIdHex"))).unwrap(),
            replay_nonce: hex(field(fixture_binding, "replayNonceHex"))
                .try_into()
                .unwrap(),
            expires_at_unix_ms: fixture_binding["expiresAtUnixMs"].as_u64().unwrap(),
        };
        let wrong_operation =
            OperationId::new(hex(field(fixture_binding, "wrongOperationIdHex"))).unwrap();
        let valid_at = fixture_binding["validAtUnixMs"].as_u64().unwrap();
        let expired_at = fixture_binding["expiredAtUnixMs"].as_u64().unwrap();

        let mut wrong = BootstrapPskState::new(binding.clone()).unwrap();
        assert_eq!(
            wrong.admit(&wrong_operation, &binding.replay_nonce, valid_at),
            Err(HandshakeRejectReason::BootstrapOperationMismatch)
        );
        assert!(!wrong.is_consumed());

        let mut expired = BootstrapPskState::new(binding.clone()).unwrap();
        assert_eq!(
            expired.admit(&binding.operation_id, &binding.replay_nonce, expired_at),
            Err(HandshakeRejectReason::BootstrapExpired)
        );
        assert!(!expired.is_consumed());

        let mut replay = BootstrapPskState::new(binding.clone()).unwrap();
        assert_eq!(
            replay.admit(&binding.operation_id, &binding.replay_nonce, valid_at),
            Ok(())
        );
        assert_eq!(
            replay.admit(&binding.operation_id, &binding.replay_nonce, valid_at),
            Err(HandshakeRejectReason::BootstrapReplayed)
        );
    }
}

#[test]
fn transcript_and_psk_mutations_are_rejected() {
    let fixture: Value = serde_json::from_str(&read_repo_file(
        "docs/reference/component-session-v2-vectors.json",
    ))
    .unwrap();
    for vector in fixture["vectors"].as_array().unwrap() {
        let material = VectorMaterial::from_fixture(vector);
        let mut wrong_material = material.clone();
        *wrong_material.prologue.last_mut().unwrap() ^= 1;
        let mut initiator = builder(vector, true, &material);
        let mut responder = builder(vector, false, &wrong_material);
        let payloads = vector["handshakePayloadsHex"].as_array().unwrap();
        let mut message = vec![0; 65_535];
        let mut plaintext = vec![0; 65_535];
        let payload_1 = hex(payloads[0].as_str().unwrap());
        let written = initiator.write_message(&payload_1, &mut message).unwrap();
        match responder.read_message(&message[..written], &mut plaintext) {
            Err(_) => continue,
            Ok(_) => {
                let payload_2 = hex(payloads[1].as_str().unwrap());
                let written = responder.write_message(&payload_2, &mut message).unwrap();
                assert!(
                    initiator
                        .read_message(&message[..written], &mut plaintext)
                        .is_err(),
                    "{} accepted a transcript mutation",
                    field(vector, "purpose")
                );
            }
        }
    }

    let bootstrap = fixture["vectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|vector| field(vector, "protocolName") == "Noise_IKpsk2_25519_ChaChaPoly_SHA256")
        .unwrap();
    let material = VectorMaterial::from_fixture(bootstrap);
    let mut wrong_material = material.clone();
    wrong_material.psk = Some([0x56; 32]);
    let mut initiator = builder(bootstrap, true, &material);
    let mut responder = builder(bootstrap, false, &wrong_material);
    let payload = hex(bootstrap["handshakePayloadsHex"][0].as_str().unwrap());
    let mut message = vec![0; 65_535];
    let mut plaintext = vec![0; 65_535];
    let written = initiator.write_message(&payload, &mut message).unwrap();
    responder
        .read_message(&message[..written], &mut plaintext)
        .unwrap();
    let response = hex(bootstrap["handshakePayloadsHex"][1].as_str().unwrap());
    let written = responder.write_message(&response, &mut message).unwrap();
    assert!(
        initiator
            .read_message(&message[..written], &mut plaintext)
            .is_err()
    );
}
