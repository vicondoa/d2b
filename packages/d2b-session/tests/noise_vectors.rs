use std::collections::BTreeSet;

use d2b_contracts::v2_component_session::{EndpointPurpose, NoiseProfile};
use serde_json::Value;
use snow::{
    Builder, HandshakeState,
    params::{DHChoice, NoiseParams},
    resolvers::{CryptoResolver, DefaultResolver},
};

const VECTORS: &str = include_str!("../../../docs/reference/component-session-v2-vectors.json");

fn hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn field<'a>(value: &'a Value, name: &str) -> &'a str {
    value[name].as_str().unwrap()
}

fn optional_hex(value: &Value, name: &str) -> Option<Vec<u8>> {
    value[name].as_str().map(hex)
}

fn public(private: &[u8]) -> Vec<u8> {
    let mut dh = DefaultResolver.resolve_dh(&DHChoice::Curve25519).unwrap();
    dh.set(private);
    dh.pubkey().to_vec()
}

struct Material {
    prologue: Vec<u8>,
    initiator_ephemeral: Vec<u8>,
    responder_ephemeral: Vec<u8>,
    initiator_static: Option<Vec<u8>>,
    initiator_public: Option<Vec<u8>>,
    responder_static: Option<Vec<u8>>,
    responder_public: Option<Vec<u8>>,
    psk: Option<[u8; 32]>,
}

impl Material {
    fn from(vector: &Value) -> Self {
        let material = Self {
            prologue: hex(field(vector, "prologueHex")),
            initiator_ephemeral: hex(field(vector, "initiatorEphemeralPrivateHex")),
            responder_ephemeral: hex(field(vector, "responderEphemeralPrivateHex")),
            initiator_static: optional_hex(vector, "initiatorStaticPrivateHex"),
            initiator_public: optional_hex(vector, "initiatorStaticPublicHex"),
            responder_static: optional_hex(vector, "responderStaticPrivateHex"),
            responder_public: optional_hex(vector, "responderStaticPublicHex"),
            psk: optional_hex(vector, "pskHex").map(|value| value.try_into().unwrap()),
        };
        assert_eq!(
            public(&material.initiator_ephemeral),
            hex(field(vector, "initiatorEphemeralPublicHex"))
        );
        assert_eq!(
            public(&material.responder_ephemeral),
            hex(field(vector, "responderEphemeralPublicHex"))
        );
        if let (Some(private), Some(declared)) =
            (&material.initiator_static, &material.initiator_public)
        {
            assert_eq!(public(private), *declared);
        }
        if let (Some(private), Some(declared)) =
            (&material.responder_static, &material.responder_public)
        {
            assert_eq!(public(private), *declared);
        }
        material
    }
}

fn state(vector: &Value, initiator: bool, material: &Material) -> HandshakeState {
    let params: NoiseParams = field(vector, "protocolName").parse().unwrap();
    let mut builder = Builder::new(params)
        .prologue(&material.prologue)
        .unwrap()
        .fixed_ephemeral_key_for_testing_only(if initiator {
            &material.initiator_ephemeral
        } else {
            &material.responder_ephemeral
        });
    builder = match field(vector, "protocolName") {
        "Noise_NN_25519_ChaChaPoly_SHA256" => builder,
        "Noise_KK_25519_ChaChaPoly_SHA256" if initiator => builder
            .local_private_key(material.initiator_static.as_deref().unwrap())
            .unwrap()
            .remote_public_key(material.responder_public.as_deref().unwrap())
            .unwrap(),
        "Noise_KK_25519_ChaChaPoly_SHA256" => builder
            .local_private_key(material.responder_static.as_deref().unwrap())
            .unwrap()
            .remote_public_key(material.initiator_public.as_deref().unwrap())
            .unwrap(),
        "Noise_IKpsk2_25519_ChaChaPoly_SHA256" if initiator => builder
            .local_private_key(material.initiator_static.as_deref().unwrap())
            .unwrap()
            .remote_public_key(material.responder_public.as_deref().unwrap())
            .unwrap()
            .psk(2, material.psk.as_ref().unwrap())
            .unwrap(),
        "Noise_IKpsk2_25519_ChaChaPoly_SHA256" => builder
            .local_private_key(material.responder_static.as_deref().unwrap())
            .unwrap()
            .psk(2, material.psk.as_ref().unwrap())
            .unwrap(),
        profile => panic!("unapproved Noise profile {profile}"),
    };
    if initiator {
        builder.build_initiator().unwrap()
    } else {
        builder.build_responder().unwrap()
    }
}

#[test]
fn every_canonical_w2_vector_verifies_exactly_with_snow_0_10() {
    let fixture: Value = serde_json::from_str(VECTORS).unwrap();
    assert_eq!(fixture["snowVersion"], "0.10.0");
    let vectors = fixture["vectors"].as_array().unwrap();
    assert_eq!(vectors.len(), EndpointPurpose::ALL.len() + 1);
    let mut fixture_ids = BTreeSet::new();
    let mut purposes = BTreeSet::new();
    for vector in vectors {
        fixture_ids.insert(field(vector, "fixtureId"));
        purposes.insert(field(vector, "purpose"));
        let material = Material::from(vector);
        let mut initiator = state(vector, true, &material);
        let mut responder = state(vector, false, &material);
        let payloads = vector["handshakePayloadsHex"].as_array().unwrap();
        let messages = vector["handshakeMessagesHex"].as_array().unwrap();
        let mut wire = vec![0; 65_535];
        let mut plaintext = vec![0; 65_535];

        let first_payload = hex(payloads[0].as_str().unwrap());
        let written = initiator.write_message(&first_payload, &mut wire).unwrap();
        assert_eq!(&wire[..written], hex(messages[0].as_str().unwrap()));
        let read = responder
            .read_message(&wire[..written], &mut plaintext)
            .unwrap();
        assert_eq!(&plaintext[..read], first_payload);

        let second_payload = hex(payloads[1].as_str().unwrap());
        let written = responder.write_message(&second_payload, &mut wire).unwrap();
        assert_eq!(&wire[..written], hex(messages[1].as_str().unwrap()));
        let read = initiator
            .read_message(&wire[..written], &mut plaintext)
            .unwrap();
        assert_eq!(&plaintext[..read], second_payload);
        assert_eq!(
            initiator.get_handshake_hash(),
            hex(field(vector, "transcriptHashHex"))
        );
        assert_eq!(
            initiator.get_handshake_hash(),
            responder.get_handshake_hash()
        );
        let split = initiator.dangerously_get_raw_split();
        assert_eq!(
            split.0.as_slice(),
            hex(field(&vector["transportKeysHex"], "initiatorToResponder"))
        );
        assert_eq!(
            split.1.as_slice(),
            hex(field(&vector["transportKeysHex"], "responderToInitiator"))
        );

        let mut initiator = initiator.into_transport_mode().unwrap();
        let mut responder = responder.into_transport_mode().unwrap();
        let records = &vector["firstRecords"];
        let initiator_plaintext = hex(field(records, "initiatorPlaintextHex"));
        let written = initiator
            .write_message(&initiator_plaintext, &mut wire)
            .unwrap();
        assert_eq!(
            &wire[..written],
            hex(field(records, "initiatorCiphertextHex"))
        );
        assert_eq!(
            responder
                .read_message(&wire[..written], &mut plaintext)
                .unwrap(),
            initiator_plaintext.len()
        );
        let responder_plaintext = hex(field(records, "responderPlaintextHex"));
        let written = responder
            .write_message(&responder_plaintext, &mut wire)
            .unwrap();
        assert_eq!(
            &wire[..written],
            hex(field(records, "responderCiphertextHex"))
        );
        assert_eq!(
            initiator
                .read_message(&wire[..written], &mut plaintext)
                .unwrap(),
            responder_plaintext.len()
        );

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
            assert!(mutations.contains(required));
        }
        if field(vector, "protocolName") == NoiseProfile::Ikpsk2_25519ChaChaPolySha256.as_str() {
            for required in ["wrong-operation", "expired-psk", "psk-replay"] {
                assert!(mutations.contains(required));
            }
        }
    }
    assert_eq!(
        purposes,
        EndpointPurpose::ALL
            .iter()
            .map(|purpose| purpose.as_str())
            .collect()
    );
    assert_eq!(
        fixture_ids,
        BTreeSet::from([
            "component-session-v2-bootstrap-ikpsk2",
            "component-session-v2-enrolled-kk",
            "component-session-v2-local-nn",
        ])
    );
}
