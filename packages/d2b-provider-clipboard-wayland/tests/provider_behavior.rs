use d2b_provider_clipboard_wayland::{
    ClipboardAuditEvent, ClipboardAuditQueue, ClipboardConfig, ClipboardController, ClipboardEntry,
    ClipboardHistory, ClipboardReason, DependencyStatus, FdCapModel, FdObjectKind, FdStatModel,
    FileSystemKind, PickerRequest, Policy, SizeBucket, classify_fd_model, validate_fd_cap,
    validate_recvmsg_control,
};

#[test]
fn mime_and_secret_hint_policy_is_closed() {
    assert!(Policy::default().allows_mime("text/plain"));
    assert!(Policy::default().allows_mime("image/png"));
    assert!(!Policy::default().allows_mime("application/octet-stream"));
    assert!(Policy::is_secret_hint("x-kde-passwordManagerHint"));
    assert!(!Policy::is_secret_hint("text/plain"));
}

#[test]
fn fd_validation_rejects_unsafe_files_and_truncated_control_messages() {
    assert!(
        classify_fd_model(FdStatModel {
            object_kind: FdObjectKind::Pipe,
            filesystem_kind: FileSystemKind::Unknown,
        })
        .is_ok()
    );
    assert!(
        classify_fd_model(FdStatModel {
            object_kind: FdObjectKind::Regular,
            filesystem_kind: FileSystemKind::DiskBacked,
        })
        .is_err()
    );
    assert!(
        validate_fd_cap(FdCapModel {
            requested_cap: 64,
            rlimit_nofile: 256,
            base_reserved: 64,
            max_fds_per_recvmsg: 16,
        })
        .is_ok()
    );
    assert!(validate_recvmsg_control(true, 2).is_err());
}

#[test]
fn history_is_bounded_ttl_aware_and_purges_guest_state() {
    let config = ClipboardConfig::default();
    let mut history = ClipboardHistory::new(config.clone());
    let entry = ClipboardEntry::new("Guest/work", "text/plain", b"hello", 100).unwrap();
    history.insert(entry).unwrap();
    assert_eq!(history.len(), 1);
    history.suspend_guest("Guest/work");
    assert!(history.authorize_guest("Guest/work").is_err());
    history.resume_guest("Guest/work");
    history.purge_guest("Guest/work");
    assert!(history.is_empty());
    let expired = ClipboardEntry::new("Guest/work", "text/plain", b"expired", 1).unwrap();
    history.insert(expired).unwrap();
    history.gc(1 + config.guest_entry_ttl_secs());
    assert!(history.is_empty());
}

#[test]
fn duplicate_history_tokens_do_not_double_count_quota() {
    let policy = Policy::new(true, true, true, true, false, 3, 4096, 4096, 32, 60).unwrap();
    let config = ClipboardConfig::from_policy(policy);
    let mut history = ClipboardHistory::new(config);
    let first = ClipboardEntry::new("Guest/work", "text/plain", &[1; 2000], 100).unwrap();
    let duplicate = ClipboardEntry::new("Guest/work", "text/plain", &[1; 2000], 100).unwrap();
    let second = ClipboardEntry::new("Guest/work", "text/plain", &[2; 2000], 101).unwrap();
    history.insert(first).unwrap();
    history.insert(duplicate).unwrap();
    history.insert(second).unwrap();
    assert_eq!(history.len(), 2);
}

#[test]
fn audit_queue_fails_closed_and_never_renders_clipboard_bytes() {
    let mut queue = ClipboardAuditQueue::new(1);
    let event = ClipboardAuditEvent::new(
        "zone-a",
        "zone-b",
        ClipboardReason::Allowed,
        SizeBucket::Lt1K,
    );
    queue.push(event.clone()).unwrap();
    assert_eq!(queue.push(event), Err(ClipboardReason::AuditQueueFull));
    assert!(!queue.to_wire().contains("hello"));
}

#[test]
fn controller_owns_no_state_volume_and_display_dependency_is_optional() {
    let controller = ClipboardController::new("Host/host-system", "User/alice").unwrap();
    assert!(controller.provider_state_set_empty());
    assert_eq!(controller.dependency_status(None), DependencyStatus::Absent);
    assert!(
        controller
            .plan_processes()
            .iter()
            .all(|process| !process.mounts_state_volume)
    );
}

#[test]
fn picker_protocol_carries_metadata_only() {
    let request = PickerRequest::new(
        "operation-1",
        "zone-a",
        "Guest/work",
        vec!["text/plain".to_owned()],
    )
    .unwrap();
    assert!(!format!("{request:?}").contains("Guest/work"));
}

#[test]
fn clipboard_component_contract_disables_legacy_scheduling_without_resource_authority() {
    let contract = d2b_provider_clipboard_wayland::clipboard_runner_contract();
    assert_eq!(contract.service_package(), "d2b.clipboard.v3");
    assert_eq!(contract.repair_interval_secs(), 300);
    assert!(contract.component_session_only());
    assert!(contract.watched_configuration_is_dependency());
}

// --- U28: declared service methods and declared endpoint grants ---

use d2b_contracts_resource::v3::{
    DesiredRevision, EndpointAttachmentKind, ResourceGeneration, ResourceRef, ResourceUid,
    StoreIncarnation, ZoneDesiredSequence, ZoneId, identity::ReconnectGeneration,
};
use d2b_provider_clipboard_wayland::{
    BRIDGE_SERVICE, CLIPBOARD_SERVICES, ClipboardEndpointBinding, ClipboardEndpointEvidence,
    ClipboardEndpointFence, ClipboardEndpointGrant, ClipboardEndpointRole, ClipboardHostEndpoints,
    ClipboardServiceRole, MANAGEMENT_SERVICE, PICKER_SERVICE, admit_clipboard_endpoint,
    clipboard_endpoint_bindings, clipboard_service_declaration, clipboard_service_declares,
    clipboard_service_role,
};

const GUEST_ENDPOINT: &str = "Endpoint/clipboard-guest-transfer";
const HOST_READ_ENDPOINT: &str = "Endpoint/clipboard-host-selection-read";
const HOST_SUPPLY_ENDPOINT: &str = "Endpoint/clipboard-host-selection-supply";
const BRIDGE_CONSUMER: &str = "Process/clipboard-bridge";

fn endpoints() -> ClipboardHostEndpoints {
    ClipboardHostEndpoints::new(
        ResourceRef::parse(GUEST_ENDPOINT).expect("endpoint"),
        ResourceRef::parse(HOST_READ_ENDPOINT).expect("endpoint"),
        ResourceRef::parse(HOST_SUPPLY_ENDPOINT).expect("endpoint"),
    )
    .expect("declared endpoints")
}

fn consumer() -> ResourceRef {
    ResourceRef::parse(BRIDGE_CONSUMER).expect("consumer")
}

fn binding(role: ClipboardEndpointRole) -> ClipboardEndpointBinding {
    clipboard_endpoint_bindings(&endpoints(), &consumer())
        .expect("bindings")
        .into_iter()
        .find(|declared| declared.role() == role)
        .expect("declared binding")
}

fn fence() -> ClipboardEndpointFence {
    ClipboardEndpointFence::new(
        ZoneId::parse("work").expect("zone"),
        StoreIncarnation::parse("store-one").expect("store"),
        DesiredRevision::INITIAL.try_next().expect("revision"),
        ZoneDesiredSequence::INITIAL.try_next().expect("sequence"),
        ResourceGeneration::new(3).expect("source generation"),
        ResourceGeneration::new(5).expect("consumer generation"),
        ReconnectGeneration::new(2).expect("reconnect"),
    )
}

fn evidence(fence: &ClipboardEndpointFence) -> ClipboardEndpointEvidence {
    ClipboardEndpointEvidence {
        zone: fence.zone().clone(),
        store: fence.store().clone(),
        source_generation: fence.source_generation(),
        consumer_generation: fence.consumer_generation(),
        desired_revision: fence.desired_revision(),
        sequence: fence.sequence(),
        reconnect: ReconnectGeneration::new(7).expect("reconnect"),
    }
}

fn source_uid() -> ResourceUid {
    ResourceUid::parse("cccccccc-0000-4000-8000-000000000001").expect("uid")
}

fn consumer_uid() -> ResourceUid {
    ResourceUid::parse("cccccccc-0000-4000-8000-000000000002").expect("uid")
}

#[test]
fn service_roles_and_methods_resolve_through_one_declared_source() {
    assert_eq!(CLIPBOARD_SERVICES.len(), 3);
    assert_eq!(clipboard_service_role(MANAGEMENT_SERVICE), Some(ClipboardServiceRole::Management));
    assert_eq!(clipboard_service_role(BRIDGE_SERVICE), Some(ClipboardServiceRole::Bridge));
    assert_eq!(clipboard_service_role(PICKER_SERVICE), Some(ClipboardServiceRole::Picker));
    assert_eq!(clipboard_service_role("d2b.display.v3"), None);
    assert_eq!(clipboard_service_role("d2b.other.v3"), None);

    for declared in CLIPBOARD_SERVICES {
        assert_eq!(
            clipboard_service_declaration(declared.role).id,
            declared.declaration.id
        );
        assert_eq!(
            clipboard_service_role(declared.declaration.id),
            Some(declared.role)
        );
    }

    // Declared method names are unique across the Provider, so a method name
    // can never be served by two services.
    let mut names: Vec<&str> = CLIPBOARD_SERVICES
        .iter()
        .flat_map(|declared| declared.declaration.methods.iter())
        .map(|method| method.name)
        .collect();
    let total = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), total);

    assert!(clipboard_service_declares(
        ClipboardServiceRole::Bridge,
        "capture-host-selection"
    ));
    assert!(clipboard_service_declares(
        ClipboardServiceRole::Bridge,
        "capture-guest-selection"
    ));
    assert!(clipboard_service_declares(
        ClipboardServiceRole::Management,
        "flush-audit"
    ));
    assert!(clipboard_service_declares(
        ClipboardServiceRole::Picker,
        "complete-picker"
    ));
    // The management and picker services carry no selection carriage.
    assert!(!clipboard_service_declares(
        ClipboardServiceRole::Management,
        "capture-host-selection"
    ));
    assert!(!clipboard_service_declares(
        ClipboardServiceRole::Picker,
        "capture-guest-selection"
    ));
    assert!(!clipboard_service_declares(
        ClipboardServiceRole::Bridge,
        "deliver-source"
    ));
}

#[test]
fn every_delivery_channel_declares_its_own_slot_and_purpose() {
    let bindings = clipboard_endpoint_bindings(&endpoints(), &consumer()).expect("bindings");
    assert_eq!(bindings.len(), ClipboardEndpointRole::ALL.len());
    let mut slots: Vec<&str> = Vec::new();
    let mut purposes: Vec<&str> = Vec::new();
    for declared in &bindings {
        let role = declared.role();
        assert_eq!(declared.request().slot().as_str(), role.slot());
        assert_eq!(declared.request().purpose().as_str(), role.purpose());
        assert_eq!(declared.request().attachment(), EndpointAttachmentKind::Connect);
        assert_eq!(
            declared.request().attachment(),
            role.attachment(),
            "the attachment form is a declared channel facet"
        );
        assert_eq!(declared.consumer_ref(), &consumer());
        assert_eq!(declared.source_ref(), endpoints().source_ref(role));
        slots.push(declared.request().slot().as_str());
        purposes.push(declared.request().purpose().as_str());
    }
    slots.sort_unstable();
    slots.dedup();
    assert_eq!(slots.len(), ClipboardEndpointRole::ALL.len());
    purposes.sort_unstable();
    purposes.dedup();
    assert_eq!(purposes.len(), ClipboardEndpointRole::ALL.len());
}

#[test]
fn the_endpoint_gate_refuses_stale_draining_and_forged_relationships() {
    let host_read = binding(ClipboardEndpointRole::HostSelectionRead);
    let committed = fence();
    let observed = evidence(&committed);

    let admitted = admit_clipboard_endpoint(
        &endpoints(),
        &consumer(),
        &host_read,
        &committed,
        &observed,
        &source_uid(),
        &consumer_uid(),
    )
    .expect("admitted");
    assert_eq!(admitted.role(), ClipboardEndpointRole::HostSelectionRead);
    assert!(admitted.admits_delivery());

    // A revoked or draining fence is refused before anything else is read.
    let revoked = committed.clone().revoke();
    let revoked_observed = evidence(&revoked);
    assert_eq!(
        admit_clipboard_endpoint(
            &endpoints(),
            &consumer(),
            &host_read,
            &revoked,
            &revoked_observed,
            &source_uid(),
            &consumer_uid(),
        )
        .map_err(|refusal| refusal.code()),
        Err("clipboard-endpoint-relationship-revoked")
    );
    let draining = committed.clone().drain();
    let draining_observed = evidence(&draining);
    assert_eq!(
        admit_clipboard_endpoint(
            &endpoints(),
            &consumer(),
            &host_read,
            &draining,
            &draining_observed,
            &source_uid(),
            &consumer_uid(),
        )
        .map_err(|refusal| refusal.code()),
        Err("clipboard-endpoint-relationship-draining")
    );

    // A foreign Zone and a stale reconnect generation are refused too.
    let foreign = ClipboardEndpointEvidence {
        zone: ZoneId::parse("other").expect("zone"),
        ..observed.clone()
    };
    assert_eq!(
        admit_clipboard_endpoint(
            &endpoints(),
            &consumer(),
            &host_read,
            &committed,
            &foreign,
            &source_uid(),
            &consumer_uid(),
        )
        .map_err(|refusal| refusal.code()),
        Err("clipboard-endpoint-foreign-zone")
    );
    let stale = ClipboardEndpointEvidence {
        reconnect: ReconnectGeneration::new(1).expect("reconnect"),
        ..observed.clone()
    };
    assert_eq!(
        admit_clipboard_endpoint(
            &endpoints(),
            &consumer(),
            &host_read,
            &committed,
            &stale,
            &source_uid(),
            &consumer_uid(),
        )
        .map_err(|refusal| refusal.code()),
        Err("clipboard-endpoint-stale-reconnect-generation")
    );

    // A relationship derived over a different declared endpoint set is a
    // conflicting declaration, not a relationship the gate repairs.
    let swapped = ClipboardHostEndpoints::new(
        ResourceRef::parse(HOST_READ_ENDPOINT).expect("endpoint"),
        ResourceRef::parse(GUEST_ENDPOINT).expect("endpoint"),
        ResourceRef::parse(HOST_SUPPLY_ENDPOINT).expect("endpoint"),
    )
    .expect("declared endpoints");
    let forged = clipboard_endpoint_bindings(&swapped, &consumer())
        .expect("bindings")
        .into_iter()
        .find(|declared| declared.role() == ClipboardEndpointRole::HostSelectionRead)
        .expect("declared binding");
    assert_ne!(forged.request(), host_read.request());
    assert_eq!(
        admit_clipboard_endpoint(
            &endpoints(),
            &consumer(),
            &forged,
            &committed,
            &observed,
            &source_uid(),
            &consumer_uid(),
        )
        .map_err(|refusal| refusal.code()),
        Err("clipboard-endpoint-request-mismatch")
    );
}

#[test]
fn a_withdrawn_relationship_stops_delivery_instead_of_another_channel() {
    let host_read = binding(ClipboardEndpointRole::HostSelectionRead);
    let committed = fence();
    let observed = evidence(&committed);
    let admitted = admit_clipboard_endpoint(
        &endpoints(),
        &consumer(),
        &host_read,
        &committed,
        &observed,
        &source_uid(),
        &consumer_uid(),
    )
    .expect("admitted");

    let grant = ClipboardEndpointGrant::new(admitted);
    assert_eq!(grant.delivery_refusal(), None);
    assert_eq!(grant.role(), ClipboardEndpointRole::HostSelectionRead);

    // Revocation.
    assert_eq!(
        grant
            .clone()
            .refenced(committed.clone().revoke())
            .delivery_refusal()
            .map(|refusal| refusal.code()),
        Some("clipboard-endpoint-relationship-revoked")
    );
    // Draining.
    assert_eq!(
        grant
            .clone()
            .refenced(committed.clone().drain())
            .delivery_refusal()
            .map(|refusal| refusal.code()),
        Some("clipboard-endpoint-relationship-draining")
    );
    // A fence that moved on without a phase change is also refused: the
    // relationship was admitted against evidence the graph has replaced.
    let next_revision = committed
        .desired_revision()
        .try_next()
        .expect("revision");
    assert_eq!(
        grant
            .refenced(committed.advance_desired_revision(next_revision))
            .delivery_refusal()
            .map(|refusal| refusal.code()),
        Some("clipboard-endpoint-relationship-superseded")
    );
    // A raised minimum reconnect generation is the same answer.
    let host_read = binding(ClipboardEndpointRole::HostSelectionRead);
    let committed = fence();
    let observed = evidence(&committed);
    let admitted = admit_clipboard_endpoint(
        &endpoints(),
        &consumer(),
        &host_read,
        &committed,
        &observed,
        &source_uid(),
        &consumer_uid(),
    )
    .expect("admitted");
    let raised = committed.raise_minimum_reconnect(ReconnectGeneration::new(9).expect("reconnect"));
    assert_eq!(
        ClipboardEndpointGrant::new(admitted)
            .refenced(raised)
            .delivery_refusal()
            .map(|refusal| refusal.code()),
        Some("clipboard-endpoint-relationship-superseded")
    );
}

#[test]
fn a_relationship_for_one_direction_does_not_authorize_the_other() {
    let committed = fence();
    let observed = evidence(&committed);
    for role in ClipboardEndpointRole::ALL {
        let declared = binding(role);
        for other in ClipboardEndpointRole::ALL {
            if other == role {
                continue;
            }
            // The two directions name different endpoints, so neither
            // relationship can stand in for the other.
            assert_ne!(
                declared.request().purpose(),
                binding(other).request().purpose()
            );
            assert_ne!(declared.source_ref(), binding(other).source_ref());
        }
        assert!(admit_clipboard_endpoint(
            &endpoints(),
            &consumer(),
            &declared,
            &committed,
            &observed,
            &source_uid(),
            &consumer_uid(),
        )
        .is_ok());
    }
}
