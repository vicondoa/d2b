use d2b_provider_clipboard_wayland::{
    ClipboardAuditEvent, ClipboardAuditQueue, ClipboardConfig, ClipboardController, ClipboardEntry,
    ClipboardHistory, ClipboardProviderDescriptor, ClipboardReason, ClipdHost, DependencyStatus,
    FdCapModel, FdObjectKind, FdStatModel, FileSystemKind, PickerRequest, PickerResult, Policy,
    SizeBucket, classify_fd_model, validate_fd_cap, validate_recvmsg_control,
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
    let mut history = ClipboardHistory::new(config.clone()).unwrap();
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
fn audit_queue_fails_closed_and_never_renders_clipboard_bytes() {
    let mut queue = ClipboardAuditQueue::new(1);
    let event = ClipboardAuditEvent::new(
        "zone-a",
        "zone-b",
        ClipboardReason::Allowed,
        SizeBucket::Lt1K,
    );
    queue.push(event.clone()).unwrap();
    assert!(queue.push(event).is_err());
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
fn cross_zone_paste_is_denied_and_guest_lock_blocks_paste() {
    let mut host = ClipdHost::new(Policy::default(), 4, Some(true)).unwrap();
    assert!(
        host.authorize_paste("zone-a", "zone-b", "Guest/work")
            .is_err()
    );
    host.suspend_guest("Guest/work");
    assert!(
        host.authorize_paste("zone-a", "zone-a", "Guest/work")
            .is_err()
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
    assert_eq!(PickerResult::Cancelled, PickerResult::Cancelled);
}

#[test]
fn clipboard_descriptor_publishes_only_typed_attachment_classes() {
    let descriptor = ClipboardProviderDescriptor::default();
    assert!(descriptor.validate().is_ok());
    assert_eq!(descriptor.attachment_classes().len(), 3);
    assert!(!descriptor.provider_state_volume);
}
