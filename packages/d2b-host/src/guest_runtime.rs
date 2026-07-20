//! Shared host-side bindings for direct GuestV2 runtime sessions.

use sha2::{Digest, Sha256};

pub const CONTROLLER_STATIC_IDENTITY_CREDENTIAL: &str = "d2b-controller-static-v2";
pub const CONTROLLER_STATIC_IDENTITY_RESOURCE_ID: &str = "controller-static-identity-v2";
pub const CONTROLLER_STATIC_IDENTITY_FD_ENV: &str = "D2B_CONTROLLER_STATIC_IDENTITY_FD";
pub const CONTROLLER_SESSION_GENERATION_ENV: &str = "D2B_CONTROLLER_SESSION_GENERATION";
pub const GUEST_V2_VSOCK_PORT: u32 = 14_318;
pub const GUEST_MATERIAL_WIRE_PREFIX: &str = "guest-session-";
pub const GUEST_ENROLLMENT_WIRE_PREFIX: &str = "guest-enrollment-";

pub fn guest_material_resource_id(workload_id: &str) -> String {
    compact_resource_id(GUEST_MATERIAL_WIRE_PREFIX, workload_id)
}

pub fn guest_enrollment_resource_id(workload_id: &str) -> String {
    compact_resource_id(GUEST_ENROLLMENT_WIRE_PREFIX, workload_id)
}

fn compact_resource_id(prefix: &str, workload_id: &str) -> String {
    let digest = Sha256::digest(workload_id.as_bytes());
    let suffix = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{prefix}{suffix}")
}

pub fn controller_session_generation(realm_id: &str, controller_generation: &str) -> u64 {
    let mut digest = Sha256::new();
    digest.update(b"d2b-controller-session-generation-v1\0");
    digest_field(&mut digest, realm_id.as_bytes());
    digest_field(&mut digest, controller_generation.as_bytes());
    let bytes = digest.finalize();
    let mut generation = u64::from_be_bytes(bytes[..8].try_into().expect("fixed digest"));
    if generation == 0 {
        generation = 1;
    }
    generation
}

pub struct GuestRuntimeChannelBindingInput<'a> {
    pub realm_id: &'a str,
    pub workload_id: &'a str,
    pub controller_generation: u64,
    pub runtime_instance_digest: &'a [u8; 32],
    pub vsock_cid: u32,
    pub vsock_port: u32,
    pub boot_nonce: &'a [u8; 32],
}

pub fn guest_runtime_channel_binding(input: GuestRuntimeChannelBindingInput<'_>) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b-guest-runtime-channel-v1\0");
    digest_field(&mut digest, input.realm_id.as_bytes());
    digest_field(&mut digest, input.workload_id.as_bytes());
    digest.update(input.controller_generation.to_be_bytes());
    digest.update(input.runtime_instance_digest);
    digest.update(input.vsock_cid.to_be_bytes());
    digest.update(input.vsock_port.to_be_bytes());
    digest.update(input.boot_nonce);
    digest.finalize().into()
}

pub struct GuestMaterialApplyDigestInput<'a> {
    pub realm_id: &'a str,
    pub workload_id: &'a str,
    pub operation_id: &'a str,
    pub session_storage_ref: &'a str,
    pub session_generation: u64,
}

pub fn guest_material_apply_digest(input: GuestMaterialApplyDigestInput<'_>) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b.broker.v2\0BrokerService\0Apply\0guest-session-material-v2\0");
    digest_field(&mut digest, input.realm_id.as_bytes());
    digest_field(&mut digest, input.workload_id.as_bytes());
    digest_field(&mut digest, input.operation_id.as_bytes());
    digest_field(&mut digest, input.session_storage_ref.as_bytes());
    digest.update(input.session_generation.to_be_bytes());
    digest.finalize().into()
}

pub struct GuestEnrollmentApplyDigestInput<'a> {
    pub realm_id: &'a str,
    pub workload_id: &'a str,
    pub operation_id: &'a str,
    pub enrollment_ref: &'a str,
    pub session_generation: u64,
    pub credential_digest: &'a [u8; 32],
}

pub fn guest_enrollment_apply_digest(input: GuestEnrollmentApplyDigestInput<'_>) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(b"d2b.broker.v2\0BrokerService\0Apply\0guest-enrollment-v1\0");
    digest_field(&mut digest, input.realm_id.as_bytes());
    digest_field(&mut digest, input.workload_id.as_bytes());
    digest_field(&mut digest, input.operation_id.as_bytes());
    digest_field(&mut digest, input.enrollment_ref.as_bytes());
    digest.update(input.session_generation.to_be_bytes());
    digest.update(input.credential_digest);
    digest.finalize().into()
}

fn digest_field(digest: &mut Sha256, field: &[u8]) {
    digest.update(u32::try_from(field.len()).unwrap_or(u32::MAX).to_be_bytes());
    digest.update(field);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_change_for_every_runtime_authority_axis() {
        let binding = |realm: &str,
                       workload: &str,
                       generation: u64,
                       runtime: [u8; 32],
                       cid: u32,
                       port: u32,
                       nonce: [u8; 32]| {
            guest_runtime_channel_binding(GuestRuntimeChannelBindingInput {
                realm_id: realm,
                workload_id: workload,
                controller_generation: generation,
                runtime_instance_digest: &runtime,
                vsock_cid: cid,
                vsock_port: port,
                boot_nonce: &nonce,
            })
        };
        let baseline = binding("work", "editor", 7, [1; 32], 42, 14_318, [2; 32]);
        for changed in [
            binding("other", "editor", 7, [1; 32], 42, 14_318, [2; 32]),
            binding("work", "browser", 7, [1; 32], 42, 14_318, [2; 32]),
            binding("work", "editor", 8, [1; 32], 42, 14_318, [2; 32]),
            binding("work", "editor", 7, [3; 32], 42, 14_318, [2; 32]),
            binding("work", "editor", 7, [1; 32], 43, 14_318, [2; 32]),
            binding("work", "editor", 7, [1; 32], 42, 14_319, [2; 32]),
            binding("work", "editor", 7, [1; 32], 42, 14_318, [4; 32]),
        ] {
            assert_ne!(baseline, changed);
        }
    }

    #[test]
    fn controller_generation_is_stable_and_nonzero() {
        let first = controller_session_generation("work", "generation-1");
        assert_ne!(first, 0);
        assert_eq!(first, controller_session_generation("work", "generation-1"));
        assert_ne!(first, controller_session_generation("work", "generation-2"));
    }
}
