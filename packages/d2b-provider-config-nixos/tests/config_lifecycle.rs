use d2b_contracts_resource::v3::{ResourceRef, ZoneId};
use d2b_provider_config_nixos::{
    ConfigApproveRequest, ConfigCaller, ConfigDiffRequest, ConfigRejectRequest, ConfigService,
    ConfigServiceBackend, ConfigStageRequest, ConfigStagingStore, ConfigStatusRequest,
    ConfigSyncRequest, GuestConfigDocument, GuestConfigReader, GuestSessionEvidence,
};

#[test]
fn guest_read_requires_current_matching_session() {
    let guest = ResourceRef::parse("Guest/work").expect("guest ref");
    let request = ConfigSyncRequest::new(guest.clone()).expect("request");
    let evidence = GuestSessionEvidence::new(guest, "boot-commitment", 1).expect("evidence");
    let result = ConfigService
        .read_guest_config(ConfigCaller::Guest, &request, &evidence, b"{}")
        .expect("read");
    assert_eq!(result.identifier, "guest-config");
    assert_eq!(result.bytes, 2);
    assert!(result.document().is_ok());
}

fn zone() -> ZoneId {
    ZoneId::parse("work").expect("zone")
}

#[test]
fn stale_session_and_non_guest_callers_fail_closed() {
    let guest = ResourceRef::parse("Guest/work").expect("guest ref");
    let request = ConfigSyncRequest::new(guest.clone()).expect("request");
    let evidence = GuestSessionEvidence::new(guest, "boot-commitment", 1)
        .expect("evidence")
        .stale();
    assert_eq!(
        ConfigService
            .read_guest_config(ConfigCaller::Guest, &request, &evidence, b"{}")
            .expect_err("stale read")
            .code(),
        "config-session-stale"
    );
    assert_eq!(
        ConfigService
            .read_guest_config(
                ConfigCaller::User,
                &request,
                &GuestSessionEvidence::new(
                    ResourceRef::parse("Guest/work").expect("guest ref"),
                    "boot-commitment",
                    1
                )
                .expect("evidence"),
                b"{}"
            )
            .expect_err("user read")
            .code(),
        "config-unauthorized"
    );
}

#[test]
fn host_staging_lifecycle_is_typed_and_consumes_approved_content() {
    let guest = ResourceRef::parse("Guest/work").expect("guest ref");
    let document =
        GuestConfigDocument::new(b"services.foo.enable = true;\n".to_vec()).expect("document");
    let stage = ConfigStageRequest::new(guest.clone(), &document).expect("stage request");
    let mut store = ConfigStagingStore::default();

    let staged = store
        .stage(ConfigCaller::Admin, &zone(), &stage)
        .expect("stage content");
    assert_eq!(staged.bytes, document.len());
    assert_eq!(staged.sha256, document.sha256());

    let status = store
        .status(
            ConfigCaller::Lifecycle,
            &zone(),
            &ConfigStatusRequest::new(guest.clone()).expect("status request"),
        )
        .expect("status");
    assert!(status.pending);
    assert_eq!(status.bytes, Some(document.len()));

    let diff = store
        .diff(
            ConfigCaller::Admin,
            &zone(),
            &ConfigDiffRequest::new(
                guest.clone(),
                "sha256:0000000000000000000000000000000000000000000000000000000000000000",
            )
            .expect("diff request"),
        )
        .expect("diff");
    assert!(diff.differs);

    let approved = store
        .approve(
            ConfigCaller::Admin,
            &zone(),
            &ConfigApproveRequest::new(guest.clone(), "host-config").expect("approve request"),
        )
        .expect("approve");
    assert_eq!(approved.bytes, document.len());
    assert_eq!(approved.sha256, document.sha256());
    assert!(
        !store
            .status(
                ConfigCaller::Admin,
                &zone(),
                &ConfigStatusRequest::new(guest.clone()).expect("status request"),
            )
            .expect("status after approve")
            .pending
    );
}

#[test]
fn approval_retry_is_idempotent_after_downstream_publish_failure() {
    let guest = ResourceRef::parse("Guest/work").expect("guest ref");
    let document = GuestConfigDocument::new(b"true\n".to_vec()).expect("document");
    let mut store = ConfigStagingStore::default();
    store
        .stage(
            ConfigCaller::Admin,
            &zone(),
            &ConfigStageRequest::new(guest.clone(), &document).expect("stage request"),
        )
        .expect("stage");

    let request = ConfigApproveRequest::new(guest.clone(), "host-config").expect("approve request");
    let first = store
        .approve(ConfigCaller::Admin, &zone(), &request)
        .expect("first approval");
    // The host publish may fail after this receipt is recorded. Retrying the
    // same approval must not require the consumed staging bytes again.
    let retry = store
        .approve(ConfigCaller::Admin, &zone(), &request)
        .expect("retry approval");
    assert_eq!(retry, first);
    assert_eq!(
        store
            .approve(
                ConfigCaller::Admin,
                &zone(),
                &ConfigApproveRequest::new(guest.clone(), "other-target")
                    .expect("second destination request"),
            )
            .expect_err("different destination must not reuse approval")
            .code(),
        "config-approval-conflict"
    );
    assert!(
        store
            .reject(
                ConfigCaller::Admin,
                &zone(),
                &ConfigRejectRequest::new(guest).expect("reject request"),
            )
            .expect("reject approval receipt")
            .removed
    );
}

#[test]
fn staging_isolated_by_zone_for_same_guest_name() {
    let guest = ResourceRef::parse("Guest/work").expect("guest ref");
    let work_zone = zone();
    let personal_zone = ZoneId::parse("personal").expect("zone");
    let work_document = GuestConfigDocument::new(b"work = true\n".to_vec()).expect("document");
    let personal_document =
        GuestConfigDocument::new(b"personal = true\n".to_vec()).expect("document");
    let mut store = ConfigStagingStore::default();

    store
        .stage(
            ConfigCaller::Admin,
            &work_zone,
            &ConfigStageRequest::new(guest.clone(), &work_document).expect("stage request"),
        )
        .expect("work stage");
    store
        .stage(
            ConfigCaller::Admin,
            &personal_zone,
            &ConfigStageRequest::new(guest.clone(), &personal_document).expect("stage request"),
        )
        .expect("personal stage");

    assert_eq!(
        store
            .status(
                ConfigCaller::Admin,
                &work_zone,
                &ConfigStatusRequest::new(guest.clone()).expect("status request"),
            )
            .expect("work status")
            .sha256,
        Some(work_document.sha256())
    );
    assert_eq!(
        store
            .status(
                ConfigCaller::Admin,
                &personal_zone,
                &ConfigStatusRequest::new(guest).expect("status request"),
            )
            .expect("personal status")
            .sha256,
        Some(personal_document.sha256())
    );
}

#[test]
fn staging_rejects_paths_invalid_views_and_unauthorized_callers() {
    let guest = ResourceRef::parse("Guest/work").expect("guest ref");
    let document = GuestConfigDocument::new(b"true\n".to_vec()).expect("document");
    let mut store = ConfigStagingStore::default();
    let stage = ConfigStageRequest::new(guest.clone(), &document).expect("stage request");
    assert_eq!(
        store
            .stage(ConfigCaller::User, &zone(), &stage)
            .expect_err("user must be denied")
            .code(),
        "config-unauthorized"
    );
    assert!(ConfigDiffRequest::new(guest.clone(), "/etc/host.nix").is_err());
    assert!(ConfigApproveRequest::new(guest.clone(), "/etc/host.nix").is_err());
    assert_eq!(
        store
            .reject(
                ConfigCaller::Admin,
                &zone(),
                &ConfigRejectRequest::new(guest).expect("reject request"),
            )
            .expect("reject empty store")
            .removed,
        false
    );
}

#[cfg(unix)]
#[test]
fn guest_reader_rejects_hardlinked_config_files() {
    let root = std::env::current_dir()
        .expect("test working directory")
        .join(".scratch")
        .join(format!("config-reader-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("scratch directory");
    let source = root.join("source.nix");
    let hardlink = root.join("guest-config.nix");
    std::fs::write(&source, b"true\n").expect("source");
    std::fs::hard_link(&source, &hardlink).expect("hardlink");

    let reader = GuestConfigReader::new(
        ResourceRef::parse("Guest/work").expect("guest ref"),
        "boot-commitment",
        1,
        &hardlink,
    )
    .expect("reader");
    let request = ConfigSyncRequest::new(ResourceRef::parse("Guest/work").expect("guest ref"))
        .expect("request");
    let error = reader
        .dispatch(
            d2b_provider_config_nixos::ConfigOperation::ReadGuestConfig,
            serde_json::to_value(request).expect("request JSON"),
        )
        .expect_err("hardlinked files must fail closed");
    assert_eq!(error.code(), "config-request-invalid");
    std::fs::remove_dir_all(root).expect("cleanup");
}
