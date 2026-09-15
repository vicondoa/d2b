use d2b_contracts_broker::broker_wire::BrokerProfile;

#[test]
fn host_profile_keeps_the_complete_closed_operation_catalog() {
    let operations = BrokerProfile::Host.operations();

    // U10 retired the process-family wire variants (SpawnRunner among
    // them) at wire v6: the variants are gone from the enum, so the host
    // catalog no longer carries them; their privileged cores are the
    // broker-generic kernels served through EnvelopeInvoke. U11 retired
    // ConsumeLifecycleLease the same way — the lease rides the generic
    // consume-cell/complete-cell kernels. U12 retired the thirteen
    // network-fds family variants the same way — their cores are the
    // broker-generic network kernels served through EnvelopeInvoke.
    for operation in [
        "EnvelopeInvoke",
        "ApplyHostGenerationHandoff",
        "ExportBrokerAudit",
    ] {
        assert!(
            operations.contains(&operation),
            "host profile lost the existing operation {operation}"
        );
        assert!(
            BrokerProfile::Host.allows_operation(operation),
            "host profile must admit {operation}"
        );
    }
    for operation in [
        "SpawnRunner",
        "OpenPidfd",
        "PollChildReaped",
        "ConsumeLifecycleLease",
        "ApplyNftables",
        "ApplyNftablesProjection",
        "ApplyNmUnmanaged",
        "ApplyRoute",
        "ApplySysctl",
        "CreateBridge",
        "DeleteBridge",
        "CreatePersistentTap",
        "DeletePersistentTap",
        "CreateTapFd",
        "SetBridgePortFlags",
        "UpdateHostsFile",
        "SeedDnsmasqLease",
    ] {
        assert!(
            !operations.contains(&operation),
            "host profile must not re-admit the retired operation {operation}"
        );
    }
}

#[test]
fn host_profile_is_not_an_open_ended_default() {
    assert!(!BrokerProfile::Host.allows_operation("SelectProfile"));
    assert!(!BrokerProfile::Host.allows_operation("UnknownFutureOperation"));
}
