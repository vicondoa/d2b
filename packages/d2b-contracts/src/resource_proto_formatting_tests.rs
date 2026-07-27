use std::fmt::{Debug, Display};

use protobuf::MessageField;

use crate::resource_proto::*;

fn assert_no_markers(rendered: &str, markers: &[&str]) {
    for marker in markers {
        assert!(
            !rendered.contains(marker),
            "protected marker appeared in diagnostic formatting"
        );
    }
}

fn assert_redacted<T: Debug + Display>(value: &T, type_name: &str, markers: &[&str]) {
    let expected = format!("{type_name}(<redacted>)");
    let debug = format!("{value:?}");
    let display = format!("{value}");
    assert_eq!(debug, expected);
    assert_eq!(display, expected);
    assert_no_markers(&debug, markers);
    assert_no_markers(&display, markers);
}

#[test]
fn every_resource_protobuf_message_has_closed_diagnostic_formatting() {
    let nonce = format!("{:x}", std::process::id());
    let zone = format!("zone-{nonce}");
    let name = format!("name-{nonce}");
    let uid = format!("123e4567-e89b-4{:0>3}-a456-{:0>12}", &nonce[..nonce.len().min(3)], nonce);
    let payload = format!("payload-marker-{nonce}");
    let markers = [zone.as_str(), name.as_str(), uid.as_str(), payload.as_str()];

    let meta = RequestMeta {
        operation_id: payload.clone(),
        idempotency_key: payload.clone(),
        correlation_id: payload.clone(),
        trace_id: payload.clone(),
        ..Default::default()
    };
    let identity = ResourceIdentity {
        zone: zone.clone(),
        resource_type: payload.clone(),
        name: name.clone(),
        uid: Some(uid.clone()),
        ..Default::default()
    };
    let envelope = ResourceEnvelopeBytes {
        identity: MessageField::some(identity.clone()),
        canonical_json: payload.as_bytes().to_vec(),
        payload_digest: payload.clone(),
        ..Default::default()
    };
    let precondition = Precondition {
        expected_uid: Some(uid.clone()),
        ..Default::default()
    };
    let projection = Projection::default();
    let filter = ListFilter {
        field: payload.clone(),
        values: vec![payload.clone()],
        ..Default::default()
    };
    let cursor = PageCursor {
        value: payload.clone(),
        ..Default::default()
    };
    let credits = WatchCredits::default();
    let event = ChangeEvent {
        identity: MessageField::some(identity.clone()),
        resource: MessageField::some(envelope.clone()),
        ..Default::default()
    };
    let resource_error = ResourceError {
        reason: payload.clone(),
        ..Default::default()
    };
    let mutation = Mutation {
        target: MessageField::some(identity.clone()),
        precondition: MessageField::some(precondition.clone()),
        resource: MessageField::some(envelope.clone()),
        add_finalizers: vec![payload.clone()],
        remove_finalizers: vec![payload.clone()],
        owner: MessageField::some(identity.clone()),
        ..Default::default()
    };

    assert_redacted(&meta, "RequestMeta", &markers);
    assert_redacted(&identity, "ResourceIdentity", &markers);
    assert_redacted(&envelope, "ResourceEnvelopeBytes", &markers);
    assert_redacted(&precondition, "Precondition", &markers);
    assert_redacted(&projection, "Projection", &markers);
    assert_redacted(&filter, "ListFilter", &markers);
    assert_redacted(&cursor, "PageCursor", &markers);
    assert_redacted(&credits, "WatchCredits", &markers);
    assert_redacted(&event, "ChangeEvent", &markers);
    assert_redacted(&resource_error, "ResourceError", &markers);
    assert_redacted(&mutation, "Mutation", &markers);

    let get_request = GetRequest {
        meta: MessageField::some(meta.clone()),
        target: MessageField::some(identity.clone()),
        projection: MessageField::some(projection.clone()),
        ..Default::default()
    };
    let get_response = GetResponse {
        resource: MessageField::some(envelope.clone()),
        error: MessageField::some(resource_error.clone()),
        ..Default::default()
    };
    let list_request = ListRequest {
        meta: MessageField::some(meta.clone()),
        resource_types: vec![payload.clone()],
        filters: vec![filter.clone()],
        cursor: MessageField::some(cursor.clone()),
        projection: MessageField::some(projection.clone()),
        ..Default::default()
    };
    let list_response = ListResponse {
        resources: vec![envelope.clone()],
        next_cursor: MessageField::some(cursor.clone()),
        error: MessageField::some(resource_error.clone()),
        ..Default::default()
    };
    let watch_request = WatchRequest {
        meta: MessageField::some(meta.clone()),
        resource_types: vec![payload.clone()],
        filters: vec![filter],
        credits: MessageField::some(credits.clone()),
        projection: MessageField::some(projection),
        ..Default::default()
    };
    let watch_response = WatchResponse {
        stream_name: payload.clone(),
        error: MessageField::some(resource_error.clone()),
        ..Default::default()
    };
    assert_redacted(&get_request, "GetRequest", &markers);
    assert_redacted(&get_response, "GetResponse", &markers);
    assert_redacted(&list_request, "ListRequest", &markers);
    assert_redacted(&list_response, "ListResponse", &markers);
    assert_redacted(&watch_request, "WatchRequest", &markers);
    assert_redacted(&watch_response, "WatchResponse", &markers);

    macro_rules! request_with_mutation {
        ($type:ident) => {
            $type {
                meta: MessageField::some(meta.clone()),
                mutation: MessageField::some(mutation.clone()),
                ..Default::default()
            }
        };
    }
    macro_rules! response_with_resource {
        ($type:ident) => {
            $type {
                resource: MessageField::some(envelope.clone()),
                error: MessageField::some(resource_error.clone()),
                ..Default::default()
            }
        };
    }

    assert_redacted(
        &request_with_mutation!(CreateRequest),
        "CreateRequest",
        &markers,
    );
    let create_response = CreateResponse {
        resource: MessageField::some(envelope.clone()),
        error: MessageField::some(resource_error.clone()),
        reconcile_projection: MessageField::some(envelope.clone()),
        ..Default::default()
    };
    assert_redacted(&create_response, "CreateResponse", &markers);
    assert_redacted(
        &request_with_mutation!(UpdateSpecRequest),
        "UpdateSpecRequest",
        &markers,
    );
    let update_spec_response = UpdateSpecResponse {
        resource: MessageField::some(envelope.clone()),
        error: MessageField::some(resource_error.clone()),
        reconcile_projection: MessageField::some(envelope.clone()),
        ..Default::default()
    };
    assert_redacted(&update_spec_response, "UpdateSpecResponse", &markers);
    assert_redacted(
        &request_with_mutation!(UpdateStatusRequest),
        "UpdateStatusRequest",
        &markers,
    );
    assert_redacted(
        &response_with_resource!(UpdateStatusResponse),
        "UpdateStatusResponse",
        &markers,
    );
    assert_redacted(
        &request_with_mutation!(UpdateMetadataRequest),
        "UpdateMetadataRequest",
        &markers,
    );
    assert_redacted(
        &response_with_resource!(UpdateMetadataResponse),
        "UpdateMetadataResponse",
        &markers,
    );
    assert_redacted(
        &request_with_mutation!(UpdateFinalizersRequest),
        "UpdateFinalizersRequest",
        &markers,
    );
    assert_redacted(
        &response_with_resource!(UpdateFinalizersResponse),
        "UpdateFinalizersResponse",
        &markers,
    );
    assert_redacted(
        &request_with_mutation!(DeleteRequest),
        "DeleteRequest",
        &markers,
    );
    let delete_response = DeleteResponse {
        resource: MessageField::some(identity.clone()),
        error: MessageField::some(resource_error.clone()),
        ..Default::default()
    };
    assert_redacted(&delete_response, "DeleteResponse", &markers);

    let batch_request = CommitBatchRequest {
        meta: MessageField::some(meta.clone()),
        mutations: vec![mutation],
        ..Default::default()
    };
    let batch_response = CommitBatchResponse {
        resources: vec![envelope.clone()],
        error: MessageField::some(resource_error.clone()),
        ..Default::default()
    };
    let resolve_request = ResolveRefRequest {
        meta: MessageField::some(meta.clone()),
        target: MessageField::some(identity.clone()),
        ..Default::default()
    };
    let resolve_response = ResolveRefResponse {
        resource: MessageField::some(identity.clone()),
        error: MessageField::some(resource_error.clone()),
        ..Default::default()
    };
    let inspect_request = InspectSchemaRequest {
        meta: MessageField::some(meta.clone()),
        resource_type: payload.clone(),
        ..Default::default()
    };
    let inspect_response = InspectSchemaResponse {
        schema: MessageField::some(envelope.clone()),
        error: MessageField::some(resource_error.clone()),
        ..Default::default()
    };
    let upgrade_request = UpgradeRequest {
        meta: MessageField::some(meta),
        target: MessageField::some(identity.clone()),
        precondition: MessageField::some(precondition),
        ..Default::default()
    };
    let upgrade_response = UpgradeResponse {
        resource: MessageField::some(envelope),
        plan: vec![identity],
        error: MessageField::some(resource_error),
        ..Default::default()
    };
    assert_redacted(&batch_request, "CommitBatchRequest", &markers);
    assert_redacted(&batch_response, "CommitBatchResponse", &markers);
    assert_redacted(&resolve_request, "ResolveRefRequest", &markers);
    assert_redacted(&resolve_response, "ResolveRefResponse", &markers);
    assert_redacted(&inspect_request, "InspectSchemaRequest", &markers);
    assert_redacted(&inspect_response, "InspectSchemaResponse", &markers);
    assert_redacted(&upgrade_request, "UpgradeRequest", &markers);
    assert_redacted(&upgrade_response, "UpgradeResponse", &markers);

    let deliberately_exposed = format!("{zone}/{name}/{uid}/{payload}");
    assert!(
        std::panic::catch_unwind(|| assert_no_markers(&deliberately_exposed, &markers)).is_err(),
        "sentinel assertion must fail when a protected field is exposed"
    );
}
