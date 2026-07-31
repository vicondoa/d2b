use d2b_contracts::v3::execution_policy::BoundedToken;
use d2b_provider_volume_local::atomic::{AtomicWriteError, check_soft_quota};
use d2b_provider_volume_local::effect_port::{ExecutionDomain, VolumeEffectError, validate_domain};

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("valid bounded token")
}

#[test]
fn cross_domain_volume_access_is_rejected() {
    let guest = ExecutionDomain::Guest(token("work-vm"));
    assert_eq!(
        validate_domain(&guest, &ExecutionDomain::Host(token("host-system"))),
        Err(VolumeEffectError::DomainMismatch)
    );
    assert_eq!(
        validate_domain(&guest, &ExecutionDomain::Guest(token("personal-vm"))),
        Err(VolumeEffectError::DomainMismatch)
    );
}

#[test]
fn quota_soft_check_accounts_for_replaced_bytes_and_rejects_overage() {
    assert!(check_soft_quota(8192, 4096, 4096, 8192).is_ok());
    assert_eq!(
        check_soft_quota(8192, 4096, 4097, 8192),
        Err(AtomicWriteError::QuotaExceeded)
    );
    assert_eq!(
        check_soft_quota(0, 0, 1, 0),
        Err(AtomicWriteError::QuotaExceeded)
    );
}
