//! SeccompProfile registration boundary: the driver declaration is what the
//! plane registers, and the registry serves this type's decoder and factory
//! from it.
//!
//! The contract half is proven here too, because a registration is only
//! meaningful if the type it names is the one the canonical contract
//! declares: a driver that served the pre-cutover posture row would admit a
//! `SeccompProfile` carrying namespace, cgroup, and device-node authority that
//! the contract no longer expresses.

use d2b_contracts_resource::v3::seccomp_profile::{
    MAX_SECCOMP_SYSCALLS, SeccompProfileSpec, SyscallDefaultAction, SyscallFilter,
};
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::canonical_json_bytes;
use d2b_resource_types::WellKnownType;
use d2b_resource_types::assert_metadata_registration;
use serde_json::json;

/// The declared `SeccompProfile` type satisfies the shared declaration-only
/// metadata driver contract: the one type its crate owns, registered through
/// its declared decoder and factory.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn the_declaration_registers_the_seccomp_profile_type() {
    assert_metadata_registration(
        &d2b_provider_seccomp_profile::seccomp_profile_descriptor(),
        WellKnownType::SECCOMP_PROFILE,
    )
    .await;
}

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("bounded token")
}

fn filter(syscalls: &[&str]) -> SyscallFilter {
    SyscallFilter::new(
        SyscallDefaultAction::Errno,
        syscalls.iter().copied().map(token).collect(),
    )
    .expect("syscall filter")
}

/// The committed bytes read back as the contract's own narrow spec, and a
/// profile that drops a syscall is a different profile rather than the same
/// one normalized.
#[test]
fn the_canonical_row_carries_the_syscall_filter_and_nothing_else() {
    let spec = SeccompProfileSpec::new(filter(&["read", "write"]));
    let bytes = canonical_json_bytes(&spec).expect("the row renders");
    let restored: SeccompProfileSpec =
        serde_json::from_slice(&bytes).expect("the committed row decodes");
    assert_eq!(restored, spec);
    assert!(restored.filter().allows(&token("read")));
    assert!(!restored.filter().allows(&token("execve")));

    let narrowed = SeccompProfileSpec::new(filter(&["read"]));
    assert_ne!(
        restored,
        serde_json::from_slice::<SeccompProfileSpec>(&canonical_json_bytes(&narrowed).expect("renders"))
            .expect("the narrowed row decodes")
    );
}

/// Every retired access-authority field is refused at decode, in the
/// direction the contract states: not ignored, not defaulted, refused.
#[test]
fn a_row_carrying_access_authority_does_not_decode() {
    for field in [
        "namespaces",
        "cgroups",
        "deviceBinds",
        "devices",
        "mounts",
        "hostPaths",
    ] {
        let mut row = json!({
            "filter": {
                "defaultAction": "errno",
                "allowed": ["read", "write"]
            }
        });
        row.as_object_mut()
            .expect("the row is an object")
            .insert(field.to_owned(), json!({}));
        serde_json::from_slice::<SeccompProfileSpec>(
            &serde_json::to_vec(&row).expect("the row serializes"),
        )
        .expect_err("a profile carrying retired access authority is not a profile");
    }
}

/// The filter's own bound is the contract's, not this crate's: it is enforced
/// by the contract's own constructor and the crate does not restate it.
#[test]
fn the_filter_bound_is_the_contract_own() {
    let syscalls: Vec<BoundedToken> = (0..=MAX_SECCOMP_SYSCALLS)
        .map(|index| token(&format!("syscall-{index}")))
        .collect();
    assert!(SyscallFilter::new(SyscallDefaultAction::Errno, syscalls).is_err());
    assert!(SyscallFilter::new(
        SyscallDefaultAction::Errno,
        vec![token("read"), token("read")]
    )
    .is_err());
}
