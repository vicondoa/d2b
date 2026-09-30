//! The unsafe-local helper's graph authority (U13, R10, R25-R27, R37).
//!
//! The helper is an unprivileged, user-scoped process. It runs a workload
//! because the graph admitted that launch, for one committed row, for one
//! authenticated subject, under the family's explicit no-isolation posture -
//! and for no other reason. These tests pin that boundary from the helper's
//! side, which is the only side that can be wrong about it: the daemon that
//! composes the frame is privileged, and a privileged process that simply
//! obeys its transport is exactly the failure this conversion removes.
//!
//! Every case is hermetic. The admission decision is a pure function of the
//! frame and the identity the helper proved for itself, so nothing here
//! starts a supervisor, opens a D-Bus connection, or spawns a process.

use d2b_contracts::configured_argv::ConfiguredArgv;
use d2b_contracts::ids::OperationId;
use d2b_contracts::token::ProtocolToken;
use d2b_contracts::workload_identity::WorkloadTarget;
use d2b_contracts_control::unsafe_local_wire::{
    BindingRealizationFacet, DaemonToUnsafeLocalHelper, HelperFailureCode, HelperGraphAdmission,
    HelperLaunchRequest, MAX_HELPER_FRAME_SIZE, RealmAccentColor,
    UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION, UnsafeLocalHelperToDaemon, UnsafeLocalPosture,
    ZoneResourceIdentity, unsafe_local_helper_protocol_supported,
};
use d2b_unsafe_local_helper::runtime::{RuntimeError, admit_helper_launch};
use nix::unistd::Uid;

/// The identity this test process can prove for itself, which is the only
/// identity a helper may ever launch under.
fn helper_uid() -> u32 {
    Uid::current().as_raw()
}

/// The same row at a later generation, which is a different committed
/// identity and therefore not the row an older admission fences.
fn committed_row(zone: &str, zone_uid: &str, reference: &str, resource_uid: &str, generation: u64) -> ZoneResourceIdentity {
    serde_json::from_value(serde_json::json!({
        "zone": zone,
        "zoneUid": zone_uid,
        "resourceRef": reference,
        "resourceUid": resource_uid,
        "generation": generation,
        "revision": 1
    }))
    .expect("a canonical committed identity")
}

fn admitted_for(workload: &ZoneResourceIdentity) -> HelperGraphAdmission {
    HelperGraphAdmission::new(
        workload.clone(),
        helper_uid(),
        UnsafeLocalPosture::ExplicitNoIsolation,
        Vec::new(),
    )
    .expect("the fixture admission is constructible")
}

fn launch(workload: &ZoneResourceIdentity, admission: HelperGraphAdmission) -> HelperLaunchRequest {
    HelperLaunchRequest {
        request_id: 1,
        operation_id: OperationId::parse("op-graph-authority").expect("canonical operation"),
        admission,
        workload: workload.clone(),
        target: WorkloadTarget::parse("tools.host.d2b").expect("canonical target"),
        item_id: ProtocolToken::parse("browser").expect("canonical item"),
        argv: ConfiguredArgv::new(vec!["browser".to_owned()]).expect("bounded argv"),
        graphical: false,
        realm_accent_color: RealmAccentColor::new("#336699").expect("canonical color"),
    }
}

fn admitted_launch() -> HelperLaunchRequest {
    let workload = committed_row(
        "host",
        "123e4567-e89b-42d3-a456-426614174000",
        "Process/tools",
        "323e4567-e89b-42d3-a456-426614174002",
        1,
    );
    launch(&workload, admitted_for(&workload))
}

/// Scenario 1 / AE29: the launch runs only under the admission its own
/// subject carries, and a transport's identity is only ever a check on that
/// admission, never a supply of one.
///
/// An admission issued for another subject is refused, and so is an
/// admission for another committed row: the fence is the exact row, and the
/// requester is whatever the graph admitted rather than whoever is on the
/// other end of the socket.
#[test]
fn a_launch_runs_only_under_the_admission_its_own_subject_carries() {
    let committed = committed_row(
        "host",
        "123e4567-e89b-42d3-a456-426614174000",
        "Process/tools",
        "323e4567-e89b-42d3-a456-426614174002",
        1,
    );

    // The admitted requester's own launch is admitted, so the refusals below
    // are a decision rather than a blanket denial.
    assert_eq!(
        admit_helper_launch(&admitted_launch(), helper_uid()),
        Ok(())
    );

    // An admission issued for a different subject is refused even though its
    // digest verifies and the row matches exactly.
    let foreign = HelperGraphAdmission::new(
        committed.clone(),
        helper_uid().wrapping_add(1).max(1),
        UnsafeLocalPosture::ExplicitNoIsolation,
        Vec::new(),
    )
    .expect("a foreign admission is constructible");
    assert_eq!(
        foreign.admit_launch(&committed, helper_uid()),
        Err(HelperFailureCode::RequesterMismatch),
        "a launch admitted for another subject never runs under this helper"
    );
    assert_eq!(
        admit_helper_launch(&launch(&committed, foreign), helper_uid()),
        Err(RuntimeError::RequesterMismatch)
    );

    // An admission for a different committed row is refused for the same
    // reason the daemon side refuses it: the row is the fence.
    let rebound = admitted_for(&committed_row(
        "personal",
        "223e4567-e89b-42d3-a456-426614174001",
        "Process/editor",
        "423e4567-e89b-42d3-a456-426614174003",
        1,
    ));
    assert_eq!(
        rebound.admit_launch(&committed, helper_uid()),
        Err(HelperFailureCode::GraphAdmissionRequired)
    );
    assert_eq!(
        admit_helper_launch(&launch(&committed, rebound), helper_uid()),
        Err(RuntimeError::GraphAdmissionRequired)
    );

    // A later generation of the same row is a different committed identity,
    // so an admission issued for generation 1 does not launch generation 2:
    // the fence moves with the row rather than with a payload.
    let replaced = committed_row(
        "host",
        "123e4567-e89b-42d3-a456-426614174000",
        "Process/tools",
        "323e4567-e89b-42d3-a456-426614174002",
        2,
    );
    assert_eq!(
        admitted_for(&committed).admit_launch(&replaced, helper_uid()),
        Err(HelperFailureCode::GraphAdmissionRequired),
        "a replaced row is not the row the admission fenced"
    );
}

/// Scenario 5 / AE19: this family realizes no filesystem presentation, so an
/// admission that depends on a destination or a named view refuses before
/// anything is spawned rather than launching without the confinement.
#[test]
fn a_presentation_this_family_cannot_realize_refuses_before_launch() {
    let committed = committed_row(
        "host",
        "123e4567-e89b-42d3-a456-426614174000",
        "Process/tools",
        "323e4567-e89b-42d3-a456-426614174002",
        1,
    );

    for facet in [
        BindingRealizationFacet::FilesystemPresentation,
        BindingRealizationFacet::NamespaceInterface,
        BindingRealizationFacet::ConsumerDeviceSlot,
        BindingRealizationFacet::EndpointDescriptor,
        BindingRealizationFacet::CredentialDelivery,
    ] {
        let admission = HelperGraphAdmission::new(
            committed.clone(),
            helper_uid(),
            UnsafeLocalPosture::ExplicitNoIsolation,
            vec![facet],
        )
        .expect("a presentation-bearing admission is constructible");
        assert_eq!(
            admission.admit_launch(&committed, helper_uid()),
            Err(HelperFailureCode::GraphAdmissionRequired),
            "{facet:?} is not something this family can confine a workload to"
        );
        assert_eq!(
            admit_helper_launch(&launch(&committed, admission), helper_uid()),
            Err(RuntimeError::GraphAdmissionRequired),
            "{facet:?} must refuse before anything is started"
        );
    }
}

/// Scenario 4: an old helper or workload frame cannot bypass graph
/// admission. The protocol version moved when the admission was added, and
/// the launch frame's admission field is required, so a frame written before
/// it existed decodes to nothing at all.
#[test]
fn an_old_helper_or_workload_frame_cannot_reach_a_launch() {
    // A version 3 peer is refused at the greeting, so the two never
    // negotiate a session.
    assert!(
        !unsafe_local_helper_protocol_supported(3),
        "a version 3 peer predates the admitted launch frame"
    );
    assert!(unsafe_local_helper_protocol_supported(
        UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION
    ));
    let old_hello: UnsafeLocalHelperToDaemon = serde_json::from_value(serde_json::json!({
        "type": "hello",
        "payload": { "protocolVersion": 3, "generation": 1, "features": [] }
    }))
    .expect("the fixture frame itself is well formed");

    // A version 3 launch frame - the shape the old helper protocol carried -
    // is not a launch any more: the admission field is required.
    let old_frame = serde_json::json!({
        "type": "launch",
        "payload": {
            "requestId": 1,
            "operationId": "op-old-frame",
            "workload": serde_json::to_value(committed_row(
        "host",
        "123e4567-e89b-42d3-a456-426614174000",
        "Process/tools",
        "323e4567-e89b-42d3-a456-426614174002",
        1,
    )).expect("canonical identity"),
            "target": "tools.host.d2b",
            "itemId": "browser",
            "argv": ["browser"],
            "graphical": false,
            "realmAccentColor": "#336699"
        }
    });
    assert!(
        serde_json::from_value::<DaemonToUnsafeLocalHelper>(old_frame).is_err(),
        "a frame with no admission is not a launch"
    );
    assert!(matches!(
        old_hello,
        UnsafeLocalHelperToDaemon::Hello(_)
    ));

    // The same frame with an admission does decode, so the refusal above is
    // the missing admission and not the frame's shape.
    assert!(
        serde_json::from_value::<DaemonToUnsafeLocalHelper>(serde_json::json!({
            "type": "launch",
            "payload": serde_json::to_value(admitted_launch()).expect("canonical frame")
        }))
        .is_ok()
    );

    // A frame that claims an admission whose digest no longer describes it is
    // refused on decode, so a tampered decision never reaches the service
    // loop either.
    let mut encoded = serde_json::to_value(admitted_launch()).expect("canonical frame");
    encoded["admission"]["requesterUid"] =
        serde_json::json!(helper_uid().wrapping_add(7).max(1));
    assert!(
        serde_json::from_value::<DaemonToUnsafeLocalHelper>(serde_json::json!({
            "type": "launch",
            "payload": encoded
        }))
        .is_err(),
        "an admission whose digest no longer describes it is not decodable"
    );
}

/// The explicit no-isolation posture survives the round trip as a declared
/// value, and the requester is redacted everywhere a launch is rendered, so
/// an admission cannot leak the identity it was issued for into a log line.
#[test]
fn the_posture_is_declared_and_the_requester_stays_redacted() {
    let request = admitted_launch();
    assert_eq!(request.admission.posture(), UnsafeLocalPosture::ExplicitNoIsolation);
    assert!(request.admission.presentation().is_empty());
    assert!(!format!("{:?}", request.admission).contains(&helper_uid().to_string()));
    assert!(!format!("{request:?}").contains("tools.host.d2b"));
}

/// The protocol stays bounded: the admission cannot be used to smuggle an
/// unbounded payload past the queue.
#[test]
fn an_admitted_launch_stays_inside_the_frame_ceiling() {
    let encoded = serde_json::to_vec(&DaemonToUnsafeLocalHelper::Launch(Box::new(
        admitted_launch(),
    )))
    .expect("canonical frame");
    assert!(
        encoded.len() <= MAX_HELPER_FRAME_SIZE,
        "an admitted launch frame fits the bounded control frame"
    );
}
