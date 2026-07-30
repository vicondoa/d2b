//! Frozen Noise handshake vectors for the three v3 profiles.
//!
//! These are the `d2b-session` golden vectors, ported unchanged: the same
//! endpoint policies, the same fixed ephemeral and static keys, the same
//! bootstrap PSK, and the same frozen expected hex. Only the import path
//! changes - every item is reached through this crate's re-export surface
//! rather than the source crate directly.
//!
//! That is exactly what the port needs to prove. The work item requires the
//! Noise profiles to be carried over verbatim; running the original vectors
//! through this crate's surface and getting byte-identical first messages,
//! second messages, transcript hashes, and split keys is the evidence that
//! nothing was forked, re-derived, or subtly re-parameterised on the way in.
//! A vector regenerated for this crate would have proved only self-consistency.
//!
//! One test is added rather than ported: the Zone-typed ZoneLink and bootstrap
//! policies this crate introduces are driven through the same runtime, so a
//! policy that lowers must also handshake.

use d2b_contracts::v3::component_session::{
    AttachmentPolicy, AttachmentPolicyKind, EndpointPolicy, EndpointPurpose, EndpointRole,
    IdentityEvidenceRequirement, LimitProfile, Locality, NoiseProfile, PurposeClass,
    ServicePackage, TransportBinding, TransportClass,
};
use snow::{Builder, HandshakeState, params::NoiseParams};

use crate::session::contract::fixtures::{enrolled_zone_link, zone_link_bootstrap};
use crate::session::{
    HandshakeCredentials, HandshakeRole, NoiseHandshake, Secret32, encode_offer, negotiate_offer,
    x25519_public_key,
};

const INIT_PAYLOAD: &[u8] = b"d2b-component-session-v3-init";
const ACCEPT_PAYLOAD: &[u8] = b"d2b-component-session-v3-accept";
const INITIATOR_EPHEMERAL: [u8; 32] = [0x33; 32];
const RESPONDER_EPHEMERAL: [u8; 32] = [0x44; 32];
const INITIATOR_STATIC: [u8; 32] = [0x11; 32];
const RESPONDER_STATIC: [u8; 32] = [0x22; 32];
const BOOTSTRAP_PSK: [u8; 32] = [0x55; 32];

fn policy(profile: NoiseProfile) -> EndpointPolicy {
    let (
        purpose,
        purpose_class,
        initiator_role,
        responder_role,
        service,
        transport,
        locality,
        identity_evidence,
    ) = match profile {
        NoiseProfile::Nn25519ChaChaPolySha256 => (
            EndpointPurpose::LocalLifecycle,
            PurposeClass::Local,
            EndpointRole::ZoneController,
            EndpointRole::Component,
            ServicePackage::ResourceV3,
            TransportClass::UnixSeqpacket,
            Locality::HostLocal,
            IdentityEvidenceRequirement::DirectionalUnix,
        ),
        NoiseProfile::Kk25519ChaChaPolySha256 => (
            EndpointPurpose::ZoneLink,
            PurposeClass::Enrolled,
            EndpointRole::ZoneController,
            EndpointRole::Relay,
            ServicePackage::ControllerV3,
            TransportClass::ProviderStream,
            Locality::Remote,
            IdentityEvidenceRequirement::EnrolledStaticKeys,
        ),
        NoiseProfile::Ikpsk2_25519ChaChaPolySha256 => (
            EndpointPurpose::Bootstrap,
            PurposeClass::Bootstrap,
            EndpointRole::ZoneController,
            EndpointRole::GuestAgent,
            ServicePackage::ControllerV3,
            TransportClass::NativeVsock,
            Locality::GuestLocal,
            IdentityEvidenceRequirement::ParentStaticAndSingleUsePsk,
        ),
    };
    EndpointPolicy {
        purpose,
        purpose_class,
        initiator_role,
        responder_role,
        service,
        schema_fingerprint: [0x11; 32],
        noise_profile: profile,
        limits: LimitProfile::local_default(),
        transport_binding: TransportBinding {
            transport,
            locality,
            channel_binding: [0x22; 32],
            identity_evidence,
        },
        reconnect_generation: 7,
        attachment_policy: if transport == TransportClass::UnixSeqpacket {
            AttachmentPolicy {
                kind: AttachmentPolicyKind::PacketAtomic,
                max_per_packet: 1,
                max_per_request: 1,
                max_per_operation: 1,
                max_per_session: 1,
                credentials_allowed: true,
            }
        } else {
            AttachmentPolicy::disabled()
        },
    }
}

fn public(private: [u8; 32]) -> [u8; 32] {
    x25519_public_key(&private).expect("derive a public key")
}

fn prologue(policy: &EndpointPolicy) -> Vec<u8> {
    let (preface, offer) = encode_offer(policy).expect("encode the offer");
    [preface.as_slice(), offer.as_slice()].concat()
}

fn state(
    profile: NoiseProfile,
    initiator: bool,
    prologue: &[u8],
    responder_public: [u8; 32],
    initiator_public: [u8; 32],
    psk: [u8; 32],
) -> HandshakeState {
    let params: NoiseParams = profile.as_str().parse().expect("parse the Noise pattern");
    let mut builder = Builder::new(params)
        .prologue(prologue)
        .expect("set the prologue")
        .fixed_ephemeral_key_for_testing_only(if initiator {
            &INITIATOR_EPHEMERAL
        } else {
            &RESPONDER_EPHEMERAL
        });
    builder = match (profile, initiator) {
        (NoiseProfile::Nn25519ChaChaPolySha256, _) => builder,
        (NoiseProfile::Kk25519ChaChaPolySha256, true) => builder
            .local_private_key(&INITIATOR_STATIC)
            .expect("initiator static")
            .remote_public_key(&responder_public)
            .expect("responder public"),
        (NoiseProfile::Kk25519ChaChaPolySha256, false) => builder
            .local_private_key(&RESPONDER_STATIC)
            .expect("responder static")
            .remote_public_key(&initiator_public)
            .expect("initiator public"),
        (NoiseProfile::Ikpsk2_25519ChaChaPolySha256, true) => builder
            .local_private_key(&INITIATOR_STATIC)
            .expect("initiator static")
            .remote_public_key(&responder_public)
            .expect("responder public")
            .psk(2, &psk)
            .expect("bootstrap psk"),
        (NoiseProfile::Ikpsk2_25519ChaChaPolySha256, false) => builder
            .local_private_key(&RESPONDER_STATIC)
            .expect("responder static")
            .psk(2, &psk)
            .expect("bootstrap psk"),
    };
    if initiator {
        builder.build_initiator().expect("build the initiator")
    } else {
        builder.build_responder().expect("build the responder")
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn exact_vector(profile: NoiseProfile) -> String {
    let endpoint = policy(profile);
    let prologue = prologue(&endpoint);
    let responder_public = public(RESPONDER_STATIC);
    let initiator_public = public(INITIATOR_STATIC);
    let mut initiator = state(
        profile,
        true,
        &prologue,
        responder_public,
        initiator_public,
        BOOTSTRAP_PSK,
    );
    let mut responder = state(
        profile,
        false,
        &prologue,
        responder_public,
        initiator_public,
        BOOTSTRAP_PSK,
    );
    let mut wire = vec![0; 65_535];
    let mut plaintext = vec![0; 65_535];
    let first_len = initiator
        .write_message(INIT_PAYLOAD, &mut wire)
        .expect("write the first message");
    let first = hex(&wire[..first_len]);
    let read = responder
        .read_message(&wire[..first_len], &mut plaintext)
        .expect("read the first message");
    assert_eq!(&plaintext[..read], INIT_PAYLOAD);
    let second_len = responder
        .write_message(ACCEPT_PAYLOAD, &mut wire)
        .expect("write the second message");
    let second = hex(&wire[..second_len]);
    let read = initiator
        .read_message(&wire[..second_len], &mut plaintext)
        .expect("read the second message");
    assert_eq!(&plaintext[..read], ACCEPT_PAYLOAD);
    assert_eq!(
        initiator.get_handshake_hash(),
        responder.get_handshake_hash()
    );
    let transcript = hex(initiator.get_handshake_hash());
    let split = initiator.dangerously_get_raw_split();
    let send_key = hex(split.0.as_slice());
    let receive_key = hex(split.1.as_slice());
    format!("{first}|{second}|{transcript}|{send_key}|{receive_key}")
}

fn expected_vector(profile: NoiseProfile) -> &'static str {
    match profile {
        NoiseProfile::Nn25519ChaChaPolySha256 => {
            "7b0d47d93427f8311160781c7c733fd89f88970aef490d8aa0ee19a4cb8a1b146432622d636f6d706f6e656e742d73657373696f6e2d76332d696e6974|ff2ee45601ec1b67310c7790404585ae697331eee1c1f8cf2419731c1fff3e6b7788b91129aac188f259fba3a4fb1bb8c23a8a4005f470096d6b233730527e02fec12926ecabf255ed431c364bed3c|22ff512deea5281615593e33ce31a423239a344e8f092f709a3dda450101d913|080e8d3a3cdf86d0f43256d8eeeabbf74d5c4564c90f3f13e3f8dc6d317d45c7|05cff8dd234aaeb61f5a9e6f85e9715dd7094d8debce99a4700e2ea30110f51c"
        }
        NoiseProfile::Kk25519ChaChaPolySha256 => {
            "7b0d47d93427f8311160781c7c733fd89f88970aef490d8aa0ee19a4cb8a1b14e19c02a1301d883bb126028394b2fe29b636a1f1b816eecc3253f30764b69eb11d899775746e4aa894dee3bb24|ff2ee45601ec1b67310c7790404585ae697331eee1c1f8cf2419731c1fff3e6b7254a9c09f590b7660a9da8045ec0049759cd6d33c294d823cb70a2946e250789130224c8a37c8cc1d71657147cb64|52987dc015d6df3cd4f7f8b82bb88a03b49e0985eab28880b1c159aa82890c41|8eb71debf1365e827d0def975d75e9e74e82b770bf2836c0a2010f4f4ef656dd|0c355661dc34e39cfb061ee75e7ae12fb498317a534d66a01cbc7606ee34fb28"
        }
        NoiseProfile::Ikpsk2_25519ChaChaPolySha256 => {
            "7b0d47d93427f8311160781c7c733fd89f88970aef490d8aa0ee19a4cb8a1b144b3944efabe6dbf0bbf813c9a6b1fd373da6585a9d41e0f4d219fb69bd0be9352ebedb6c5bf244b4124a28e7956eb955e7c90c1512bb224be505d8ae3f56f6353d1c9eae944df1d52656ef74699d71bb56d073a32002be47ff30ed6062|ff2ee45601ec1b67310c7790404585ae697331eee1c1f8cf2419731c1fff3e6b37beb3215105a766c7934667000495a225e22461286fcc138ac684d32fcf926d7c9ebc5f4130a9a41f18fb61b42b7f|1c796b60c0ac1ce29438720f8617e16f54d04b290b111ed00ff97ce2d6a9af9f|a418003c613eaa5b5eb8cea932fd59f0bcd1e122ecc7fafe3fae8c6795464d4b|810fc2a6c723e788a0724b9265aabf6d282ca818cd385bd0a3accabf07e8c4df"
        }
    }
}

const ALL_PROFILES: [NoiseProfile; 3] = [
    NoiseProfile::Nn25519ChaChaPolySha256,
    NoiseProfile::Kk25519ChaChaPolySha256,
    NoiseProfile::Ikpsk2_25519ChaChaPolySha256,
];

#[test]
fn exact_nn_kk_and_ikpsk2_vectors_are_frozen() {
    for profile in ALL_PROFILES {
        assert_eq!(
            exact_vector(profile),
            expected_vector(profile),
            "{profile:?}"
        );
    }
}

#[test]
fn transcript_and_credentials_reject_mutations() {
    for profile in ALL_PROFILES {
        let endpoint = policy(profile);
        let prologue = prologue(&endpoint);
        let responder_public = public(RESPONDER_STATIC);
        let initiator_public = public(INITIATOR_STATIC);
        let mut mutated = prologue.clone();
        mutated[0] ^= 1;
        let mut initiator = state(
            profile,
            true,
            &prologue,
            responder_public,
            initiator_public,
            BOOTSTRAP_PSK,
        );
        let mut responder = state(
            profile,
            false,
            &mutated,
            responder_public,
            initiator_public,
            BOOTSTRAP_PSK,
        );
        let mut wire = vec![0; 65_535];
        let mut plaintext = vec![0; 65_535];
        let written = initiator
            .write_message(INIT_PAYLOAD, &mut wire)
            .expect("write the first message");
        if responder
            .read_message(&wire[..written], &mut plaintext)
            .is_ok()
        {
            let response = responder
                .write_message(ACCEPT_PAYLOAD, &mut wire)
                .expect("write the second message");
            assert!(
                initiator
                    .read_message(&wire[..response], &mut plaintext)
                    .is_err()
            );
        }
    }
}

#[test]
fn public_runtime_handshake_matches_exact_profiles() {
    for profile in ALL_PROFILES {
        let endpoint = policy(profile);
        let (preface, offer) = encode_offer(&endpoint).expect("encode");
        let negotiated = negotiate_offer(&preface, &offer, &endpoint).expect("negotiate");
        let responder_public = public(RESPONDER_STATIC);
        let initiator_credentials = match profile {
            NoiseProfile::Nn25519ChaChaPolySha256 => HandshakeCredentials::Nn,
            NoiseProfile::Kk25519ChaChaPolySha256 => HandshakeCredentials::Kk {
                local_private: Secret32::new(INITIATOR_STATIC).expect("a nonzero private key"),
                remote_public: responder_public,
            },
            // IKpsk2 needs an admitted single-use PSK, which only a bootstrap
            // admission can produce; the exact-vector test above covers the
            // profile's bytes.
            NoiseProfile::Ikpsk2_25519ChaChaPolySha256 => continue,
        };
        NoiseHandshake::new(HandshakeRole::Initiator, &negotiated, initiator_credentials)
            .expect("build the handshake");
    }
}

/// The Zone-typed policies this crate introduces drive the same runtime.
///
/// A ZoneLink policy that lowers must also negotiate and build a handshake;
/// otherwise "lowers for the wire" would be a weaker claim than it reads.
#[test]
fn the_zone_typed_zone_link_policies_negotiate_through_the_same_runtime() {
    let enrolled = enrolled_zone_link(7)
        .lower()
        .expect("lower the enrolled policy");
    let (preface, offer) = encode_offer(&enrolled).expect("encode");
    let negotiated = negotiate_offer(&preface, &offer, &enrolled).expect("negotiate");
    NoiseHandshake::new(
        HandshakeRole::Initiator,
        &negotiated,
        HandshakeCredentials::Kk {
            local_private: Secret32::new(INITIATOR_STATIC).expect("a nonzero private key"),
            remote_public: public(RESPONDER_STATIC),
        },
    )
    .expect("an enrolled ZoneLink handshake builds");

    // The bootstrap policy negotiates too. Building its handshake requires an
    // admitted PSK, which is the bootstrap admission's to issue, so the
    // negotiation is the boundary this test asserts.
    let bootstrap = zone_link_bootstrap(1)
        .lower()
        .expect("lower the bootstrap policy");
    let (preface, offer) = encode_offer(&bootstrap).expect("encode");
    negotiate_offer(&preface, &offer, &bootstrap).expect("negotiate the bootstrap offer");
}

/// A ZoneLink offer never carries the un-extended taxonomy by accident.
///
/// The enrolled ZoneLink policy must lower to the `zone-link` purpose and
/// nothing else; if a future edit widened the fixture, the frozen prologue
/// bytes above would change silently but this assertion would not.
#[test]
fn the_enrolled_zone_link_offer_names_the_zone_link_purpose() {
    let enrolled = enrolled_zone_link(7).lower().expect("lower");
    assert_eq!(enrolled.purpose, EndpointPurpose::ZoneLink);
    assert_eq!(enrolled.purpose_class, PurposeClass::Enrolled);
    assert_eq!(
        enrolled.noise_profile,
        NoiseProfile::Kk25519ChaChaPolySha256
    );
    assert_eq!(
        enrolled.attachment_policy.kind,
        AttachmentPolicyKind::Disabled
    );
}
