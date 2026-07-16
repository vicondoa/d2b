use std::{error::Error, fmt};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::InfrastructureHandle;

const MAX_CANONICAL_ID_BYTES: usize = 128;
const FINGERPRINT_HEX_BYTES: usize = 64;
const BINDING_DOMAIN: &[u8] = b"d2b.azure-vm.fake-sdk.infrastructure-binding.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingMaterialError {
    InvalidCanonicalField,
}

impl fmt::Display for BindingMaterialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("canonical infrastructure binding material is invalid")
    }
}

impl Error for BindingMaterialError {}

#[derive(Clone, Copy)]
pub struct InfrastructureBindingMaterial<'a> {
    schema_version: u32,
    provider_id: &'a str,
    handle_id: &'a str,
    realm_id: &'a str,
    provider_generation: u64,
    resource_generation: u64,
    configuration_fingerprint: &'a str,
}

impl<'a> InfrastructureBindingMaterial<'a> {
    pub fn new(
        schema_version: u32,
        provider_id: &'a str,
        handle_id: &'a str,
        realm_id: &'a str,
        provider_generation: u64,
        resource_generation: u64,
        configuration_fingerprint: &'a str,
    ) -> Result<Self, BindingMaterialError> {
        let valid = schema_version != 0
            && valid_id(provider_id)
            && valid_id(handle_id)
            && valid_id(realm_id)
            && valid_generation(provider_generation)
            && valid_generation(resource_generation)
            && configuration_fingerprint.len() == FINGERPRINT_HEX_BYTES
            && configuration_fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !valid {
            return Err(BindingMaterialError::InvalidCanonicalField);
        }
        Ok(Self {
            schema_version,
            provider_id,
            handle_id,
            realm_id,
            provider_generation,
            resource_generation,
            configuration_fingerprint,
        })
    }
}

impl fmt::Debug for InfrastructureBindingMaterial<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InfrastructureBindingMaterial")
            .field("schema_version", &self.schema_version)
            .field("canonical_fields", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InfrastructureBindingFingerprint([u8; 32]);

impl InfrastructureBindingFingerprint {
    pub fn compute(
        material: &InfrastructureBindingMaterial<'_>,
        resource: InfrastructureHandle,
    ) -> Self {
        let mut encoded = Vec::with_capacity(512);
        push_bytes(&mut encoded, BINDING_DOMAIN);
        push_u32(&mut encoded, material.schema_version);
        push_bytes(&mut encoded, material.provider_id.as_bytes());
        push_bytes(&mut encoded, material.handle_id.as_bytes());
        push_bytes(&mut encoded, material.realm_id.as_bytes());
        push_u64(&mut encoded, material.provider_generation);
        push_u64(&mut encoded, material.resource_generation);
        push_bytes(&mut encoded, material.configuration_fingerprint.as_bytes());
        push_u64(&mut encoded, resource.identity().get());
        push_u64(&mut encoded, resource.generation().get());
        Self(sha256(&encoded))
    }

    pub fn verifies(
        self,
        material: &InfrastructureBindingMaterial<'_>,
        resource: InfrastructureHandle,
    ) -> bool {
        self == Self::compute(material, resource)
    }
}

impl fmt::Debug for InfrastructureBindingFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InfrastructureBindingFingerprint(<redacted>)")
    }
}

impl Serialize for InfrastructureBindingFingerprint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&HexFingerprint(&self.0))
    }
}

impl<'de> Deserialize<'de> for InfrastructureBindingFingerprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        decode_fingerprint(&value)
            .map(Self)
            .ok_or_else(|| serde::de::Error::custom("invalid infrastructure binding fingerprint"))
    }
}

struct HexFingerprint<'a>(&'a [u8; 32]);

impl fmt::Display for HexFingerprint<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CANONICAL_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_generation(value: u64) -> bool {
    (1..=crate::types::MAX_SAFE_INTEGER).contains(&value)
}

fn push_bytes(encoded: &mut Vec<u8>, value: &[u8]) {
    encoded.extend_from_slice(&(value.len() as u32).to_be_bytes());
    encoded.extend_from_slice(value);
}

fn push_u32(encoded: &mut Vec<u8>, value: u32) {
    encoded.extend_from_slice(&value.to_be_bytes());
}

fn push_u64(encoded: &mut Vec<u8>, value: u64) {
    encoded.extend_from_slice(&value.to_be_bytes());
}

fn decode_fingerprint(value: &str) -> Option<[u8; 32]> {
    if value.len() != FINGERPRINT_HEX_BYTES {
        return None;
    }
    let mut decoded = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = decode_nibble(pair[0])?
            .checked_mul(16)?
            .checked_add(decode_nibble(pair[1])?)?;
    }
    Some(decoded)
}

fn decode_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

fn sha256(input: &[u8]) -> [u8; 32] {
    const INITIAL: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    const ROUND: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];

    let bit_length = (input.len() as u64).saturating_mul(8);
    let mut message = Vec::with_capacity(input.len() + 72);
    message.extend_from_slice(input);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());

    let mut state = INITIAL;
    for chunk in message.chunks_exact(64) {
        let mut schedule = [0_u32; 64];
        for (index, word) in chunk.chunks_exact(4).enumerate() {
            schedule[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let sigma0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let sigma1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(sigma0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(sigma1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ (!e & g);
            let first = h
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(ROUND[index])
                .wrapping_add(schedule[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let second = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(first);
            d = c;
            c = b;
            b = a;
            a = first.wrapping_add(second);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }

    let mut output = [0_u8; 32];
    for (destination, word) in output.chunks_exact_mut(4).zip(state) {
        destination.copy_from_slice(&word.to_be_bytes());
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_published_vectors() {
        assert_eq!(
            HexFingerprint(&sha256(b"")).to_string(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            HexFingerprint(&sha256(b"abc")).to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            HexFingerprint(&sha256(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
            ))
            .to_string(),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }
}
