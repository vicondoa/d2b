use d2b_provider_shell_terminal::{
    ExecutionTarget, GuestPlacement, HostPlacement, IsolationPosture, ShellTerminalError,
    validate_guest_placement, validate_host_placement,
};

#[test]
fn host_and_guest_placement_refuse_identity_or_domain_mismatch() {
    assert_eq!(
        validate_host_placement(&HostPlacement {
            isolation_posture: IsolationPosture::None,
            workload_uid_verified: false,
        }),
        Err(ShellTerminalError::WorkloadIdentityMismatch)
    );
    assert_eq!(
        validate_guest_placement(
            &ExecutionTarget::guest("work"),
            &GuestPlacement {
                user_domain_allowed: false,
                default_user_matches: true,
            },
        ),
        Err(ShellTerminalError::GuestUserDomainUnsupported)
    );
    assert!(
        validate_guest_placement(
            &ExecutionTarget::guest("work"),
            &GuestPlacement {
                user_domain_allowed: true,
                default_user_matches: true,
            },
        )
        .is_ok()
    );
}
