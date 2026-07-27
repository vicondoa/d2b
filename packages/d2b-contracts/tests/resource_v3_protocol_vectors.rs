use d2b_contracts::resource_proto as wire;
use protobuf::{Enum, EnumOrUnknown, Message, MessageField};

fn assert_wire<M: Message>(message: &M, expected: &[u8]) {
    assert_eq!(message.write_to_bytes().unwrap(), expected);
}

fn identity() -> wire::ResourceIdentity {
    let mut value = wire::ResourceIdentity::new();
    value.zone = "x".to_owned();
    value
}

fn envelope() -> wire::ResourceEnvelopeBytes {
    let mut value = wire::ResourceEnvelopeBytes::new();
    value.canonical_json = vec![1];
    value
}

fn meta() -> wire::RequestMeta {
    let mut value = wire::RequestMeta::new();
    value.operation_id = "x".to_owned();
    value
}

fn precondition() -> wire::Precondition {
    let mut value = wire::Precondition::new();
    value.kind = EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_CREATE_ABSENT);
    value
}

fn projection() -> wire::Projection {
    let mut value = wire::Projection::new();
    value.kind = EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL);
    value
}

fn filter() -> wire::ListFilter {
    let mut value = wire::ListFilter::new();
    value.field = "x".to_owned();
    value
}

fn cursor() -> wire::PageCursor {
    let mut value = wire::PageCursor::new();
    value.value = "x".to_owned();
    value
}

fn credits() -> wire::WatchCredits {
    let mut value = wire::WatchCredits::new();
    value.initial = 1;
    value
}

fn resource_error() -> wire::ResourceError {
    let mut value = wire::ResourceError::new();
    value.kind =
        EnumOrUnknown::new(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_NOT_FOUND);
    value
}

fn mutation() -> wire::Mutation {
    let mut value = wire::Mutation::new();
    value.kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_CREATE);
    value
}

macro_rules! vector {
    ($ty:ident, $field:ident = $value:expr, $expected:expr) => {{
        let mut message = wire::$ty::new();
        message.$field = $value;
        assert_wire(&message, $expected);
    }};
}

#[test]
fn support_message_field_numbers_are_literal_wire_vectors() {
    vector!(RequestMeta, operation_id = "x".to_owned(), &[0x0a, 1, b'x']);
    vector!(
        RequestMeta,
        idempotency_key = "x".to_owned(),
        &[0x12, 1, b'x']
    );
    vector!(
        RequestMeta,
        correlation_id = "x".to_owned(),
        &[0x1a, 1, b'x']
    );
    vector!(RequestMeta, trace_id = "x".to_owned(), &[0x22, 1, b'x']);
    vector!(RequestMeta, deadline_ms = 1, &[0x28, 1]);

    vector!(ResourceIdentity, zone = "x".to_owned(), &[0x0a, 1, b'x']);
    vector!(
        ResourceIdentity,
        resource_type = "x".to_owned(),
        &[0x12, 1, b'x']
    );
    vector!(ResourceIdentity, name = "x".to_owned(), &[0x1a, 1, b'x']);
    vector!(
        ResourceIdentity,
        uid = Some("x".to_owned()),
        &[0x22, 1, b'x']
    );
    vector!(ResourceIdentity, generation = Some(1), &[0x28, 1]);
    vector!(ResourceIdentity, revision = Some(1), &[0x30, 1]);

    vector!(
        ResourceEnvelopeBytes,
        identity = MessageField::some(identity()),
        &[0x0a, 3, 0x0a, 1, b'x']
    );
    vector!(
        ResourceEnvelopeBytes,
        canonical_json = vec![1],
        &[0x12, 1, 1]
    );
    vector!(
        ResourceEnvelopeBytes,
        payload_digest = "x".to_owned(),
        &[0x1a, 1, b'x']
    );

    vector!(
        Precondition,
        kind = EnumOrUnknown::new(wire::PreconditionKind::PRECONDITION_KIND_CREATE_ABSENT),
        &[0x08, 1]
    );
    vector!(Precondition, expected_revision = Some(1), &[0x10, 1]);
    vector!(
        Precondition,
        expected_uid = Some("x".to_owned()),
        &[0x1a, 1, b'x']
    );
    vector!(
        Projection,
        kind = EnumOrUnknown::new(wire::ProjectionKind::PROJECTION_KIND_FULL),
        &[0x08, 1]
    );
    vector!(ListFilter, field = "x".to_owned(), &[0x0a, 1, b'x']);
    vector!(ListFilter, values = vec!["x".to_owned()], &[0x12, 1, b'x']);
    vector!(PageCursor, value = "x".to_owned(), &[0x0a, 1, b'x']);
    vector!(WatchCredits, initial = 1, &[0x08, 1]);

    vector!(
        ChangeEvent,
        kind = EnumOrUnknown::new(wire::ChangeKind::CHANGE_KIND_CREATED),
        &[0x08, 1]
    );
    vector!(ChangeEvent, revision = 1, &[0x10, 1]);
    vector!(ChangeEvent, ordinal = 1, &[0x18, 1]);
    vector!(
        ChangeEvent,
        identity = MessageField::some(identity()),
        &[0x22, 3, 0x0a, 1, b'x']
    );
    vector!(
        ChangeEvent,
        resource = MessageField::some(envelope()),
        &[0x2a, 3, 0x12, 1, 1]
    );

    vector!(
        ResourceError,
        kind = EnumOrUnknown::new(wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_NOT_FOUND),
        &[0x08, 1]
    );
    vector!(ResourceError, current_revision = Some(1), &[0x10, 1]);
    vector!(ResourceError, retry_after_ms = Some(1), &[0x18, 1]);
    vector!(
        ResourceError,
        retry_class = EnumOrUnknown::new(wire::RetryClass::RETRY_CLASS_NEVER),
        &[0x20, 1]
    );
    vector!(ResourceError, reason = "x".to_owned(), &[0x2a, 1, b'x']);

    vector!(
        Mutation,
        kind = EnumOrUnknown::new(wire::MutationKind::MUTATION_KIND_CREATE),
        &[0x08, 1]
    );
    vector!(
        Mutation,
        target = MessageField::some(identity()),
        &[0x12, 3, 0x0a, 1, b'x']
    );
    vector!(
        Mutation,
        precondition = MessageField::some(precondition()),
        &[0x1a, 2, 0x08, 1]
    );
    vector!(
        Mutation,
        resource = MessageField::some(envelope()),
        &[0x22, 3, 0x12, 1, 1]
    );
    vector!(
        Mutation,
        add_finalizers = vec!["x".to_owned()],
        &[0x2a, 1, b'x']
    );
    vector!(
        Mutation,
        remove_finalizers = vec!["x".to_owned()],
        &[0x32, 1, b'x']
    );
    vector!(Mutation, wait_for_reconcile = true, &[0x38, 1]);
    vector!(Mutation, reconcile_deadline_ms = 1, &[0x40, 1]);
    vector!(
        Mutation,
        owner = MessageField::some(identity()),
        &[0x4a, 3, 0x0a, 1, b'x']
    );
}

#[test]
fn method_message_field_numbers_are_literal_wire_vectors() {
    vector!(
        GetRequest,
        meta = MessageField::some(meta()),
        &[0x0a, 3, 0x0a, 1, b'x']
    );
    vector!(
        GetRequest,
        target = MessageField::some(identity()),
        &[0x12, 3, 0x0a, 1, b'x']
    );
    vector!(
        GetRequest,
        projection = MessageField::some(projection()),
        &[0x1a, 2, 0x08, 1]
    );
    vector!(
        GetResponse,
        resource = MessageField::some(envelope()),
        &[0x0a, 3, 0x12, 1, 1]
    );
    vector!(
        GetResponse,
        error = MessageField::some(resource_error()),
        &[0x12, 2, 0x08, 1]
    );

    vector!(
        ListRequest,
        meta = MessageField::some(meta()),
        &[0x0a, 3, 0x0a, 1, b'x']
    );
    vector!(
        ListRequest,
        resource_types = vec!["x".to_owned()],
        &[0x12, 1, b'x']
    );
    vector!(
        ListRequest,
        filters = vec![filter()],
        &[0x1a, 3, 0x0a, 1, b'x']
    );
    vector!(ListRequest, page_size = 1, &[0x20, 1]);
    vector!(
        ListRequest,
        cursor = MessageField::some(cursor()),
        &[0x2a, 3, 0x0a, 1, b'x']
    );
    vector!(
        ListRequest,
        projection = MessageField::some(projection()),
        &[0x32, 2, 0x08, 1]
    );
    vector!(
        ListResponse,
        resources = vec![envelope()],
        &[0x0a, 3, 0x12, 1, 1]
    );
    vector!(ListResponse, snapshot_revision = 1, &[0x10, 1]);
    vector!(
        ListResponse,
        next_cursor = MessageField::some(cursor()),
        &[0x1a, 3, 0x0a, 1, b'x']
    );
    vector!(ListResponse, truncated = true, &[0x20, 1]);
    vector!(
        ListResponse,
        error = MessageField::some(resource_error()),
        &[0x2a, 2, 0x08, 1]
    );

    vector!(
        WatchRequest,
        meta = MessageField::some(meta()),
        &[0x0a, 3, 0x0a, 1, b'x']
    );
    vector!(
        WatchRequest,
        resource_types = vec!["x".to_owned()],
        &[0x12, 1, b'x']
    );
    vector!(
        WatchRequest,
        filters = vec![filter()],
        &[0x1a, 3, 0x0a, 1, b'x']
    );
    vector!(WatchRequest, after_revision = 1, &[0x20, 1]);
    vector!(
        WatchRequest,
        credits = MessageField::some(credits()),
        &[0x2a, 2, 0x08, 1]
    );
    vector!(
        WatchRequest,
        projection = MessageField::some(projection()),
        &[0x32, 2, 0x08, 1]
    );
    vector!(
        WatchResponse,
        stream_name = "x".to_owned(),
        &[0x0a, 1, b'x']
    );
    vector!(WatchResponse, snapshot_revision = 1, &[0x10, 1]);
    vector!(
        WatchResponse,
        error = MessageField::some(resource_error()),
        &[0x1a, 2, 0x08, 1]
    );

    vector!(
        CreateRequest,
        meta = MessageField::some(meta()),
        &[0x0a, 3, 0x0a, 1, b'x']
    );
    vector!(
        CreateRequest,
        mutation = MessageField::some(mutation()),
        &[0x12, 2, 0x08, 1]
    );
    assert_expedited_response_vectors::<wire::CreateResponse>();
    vector!(
        UpdateSpecRequest,
        meta = MessageField::some(meta()),
        &[0x0a, 3, 0x0a, 1, b'x']
    );
    vector!(
        UpdateSpecRequest,
        mutation = MessageField::some(mutation()),
        &[0x12, 2, 0x08, 1]
    );
    assert_update_spec_response_vectors();
    assert_simple_mutation_vectors();
    assert_remaining_method_vectors();
}

trait ExpeditedResponse: Message + Default {
    fn set_resource(&mut self, value: MessageField<wire::ResourceEnvelopeBytes>);
    fn set_revision(&mut self, value: u64);
    fn set_error(&mut self, value: MessageField<wire::ResourceError>);
    fn set_disposition(&mut self, value: EnumOrUnknown<wire::ReconcileDisposition>);
    fn set_status_persistence(&mut self, value: EnumOrUnknown<wire::StatusPersistence>);
    fn set_last_persisted_status_revision(&mut self, value: Option<u64>);
    fn set_reconcile_projection(&mut self, value: MessageField<wire::ResourceEnvelopeBytes>);
}

impl ExpeditedResponse for wire::CreateResponse {
    fn set_resource(&mut self, value: MessageField<wire::ResourceEnvelopeBytes>) {
        self.resource = value;
    }
    fn set_revision(&mut self, value: u64) {
        self.revision = value;
    }
    fn set_error(&mut self, value: MessageField<wire::ResourceError>) {
        self.error = value;
    }
    fn set_disposition(&mut self, value: EnumOrUnknown<wire::ReconcileDisposition>) {
        self.disposition = value;
    }
    fn set_status_persistence(&mut self, value: EnumOrUnknown<wire::StatusPersistence>) {
        self.status_persistence = value;
    }
    fn set_last_persisted_status_revision(&mut self, value: Option<u64>) {
        self.last_persisted_status_revision = value;
    }
    fn set_reconcile_projection(&mut self, value: MessageField<wire::ResourceEnvelopeBytes>) {
        self.reconcile_projection = value;
    }
}

fn assert_expedited_response_vectors<T: ExpeditedResponse>() {
    let mut value = T::default();
    value.set_resource(MessageField::some(envelope()));
    assert_wire(&value, &[0x0a, 3, 0x12, 1, 1]);
    let mut value = T::default();
    value.set_revision(1);
    assert_wire(&value, &[0x10, 1]);
    let mut value = T::default();
    value.set_error(MessageField::some(resource_error()));
    assert_wire(&value, &[0x1a, 2, 0x08, 1]);
    let mut value = T::default();
    value.set_disposition(EnumOrUnknown::new(
        wire::ReconcileDisposition::RECONCILE_DISPOSITION_CONVERGED,
    ));
    assert_wire(&value, &[0x20, 1]);
    let mut value = T::default();
    value.set_status_persistence(EnumOrUnknown::new(
        wire::StatusPersistence::STATUS_PERSISTENCE_PENDING,
    ));
    assert_wire(&value, &[0x28, 1]);
    let mut value = T::default();
    value.set_last_persisted_status_revision(Some(1));
    assert_wire(&value, &[0x30, 1]);
    let mut value = T::default();
    value.set_reconcile_projection(MessageField::some(envelope()));
    assert_wire(&value, &[0x3a, 3, 0x12, 1, 1]);
}

fn assert_update_spec_response_vectors() {
    macro_rules! one {
        ($field:ident = $value:expr, $expected:expr) => {
            vector!(UpdateSpecResponse, $field = $value, $expected);
        };
    }
    one!(
        resource = MessageField::some(envelope()),
        &[0x0a, 3, 0x12, 1, 1]
    );
    one!(revision = 1, &[0x10, 1]);
    one!(
        error = MessageField::some(resource_error()),
        &[0x1a, 2, 0x08, 1]
    );
    one!(
        disposition =
            EnumOrUnknown::new(wire::ReconcileDisposition::RECONCILE_DISPOSITION_CONVERGED),
        &[0x20, 1]
    );
    one!(
        status_persistence =
            EnumOrUnknown::new(wire::StatusPersistence::STATUS_PERSISTENCE_PENDING),
        &[0x28, 1]
    );
    one!(last_persisted_status_revision = Some(1), &[0x30, 1]);
    one!(
        reconcile_projection = MessageField::some(envelope()),
        &[0x3a, 3, 0x12, 1, 1]
    );
}

macro_rules! simple_mutation_pair {
    ($request:ident, $response:ident) => {{
        vector!(
            $request,
            meta = MessageField::some(meta()),
            &[0x0a, 3, 0x0a, 1, b'x']
        );
        vector!(
            $request,
            mutation = MessageField::some(mutation()),
            &[0x12, 2, 0x08, 1]
        );
        vector!(
            $response,
            resource = MessageField::some(envelope()),
            &[0x0a, 3, 0x12, 1, 1]
        );
        vector!($response, revision = 1, &[0x10, 1]);
        vector!(
            $response,
            error = MessageField::some(resource_error()),
            &[0x1a, 2, 0x08, 1]
        );
    }};
}

fn assert_simple_mutation_vectors() {
    simple_mutation_pair!(UpdateStatusRequest, UpdateStatusResponse);
    simple_mutation_pair!(UpdateMetadataRequest, UpdateMetadataResponse);
    simple_mutation_pair!(UpdateFinalizersRequest, UpdateFinalizersResponse);
    vector!(
        DeleteRequest,
        meta = MessageField::some(meta()),
        &[0x0a, 3, 0x0a, 1, b'x']
    );
    vector!(
        DeleteRequest,
        mutation = MessageField::some(mutation()),
        &[0x12, 2, 0x08, 1]
    );
    vector!(
        DeleteResponse,
        resource = MessageField::some(identity()),
        &[0x0a, 3, 0x0a, 1, b'x']
    );
    vector!(DeleteResponse, revision = 1, &[0x10, 1]);
    vector!(
        DeleteResponse,
        error = MessageField::some(resource_error()),
        &[0x1a, 2, 0x08, 1]
    );
    vector!(
        DeleteResponse,
        disposition =
            EnumOrUnknown::new(wire::ReconcileDisposition::RECONCILE_DISPOSITION_CONVERGED),
        &[0x20, 1]
    );
}

fn assert_remaining_method_vectors() {
    vector!(
        CommitBatchRequest,
        meta = MessageField::some(meta()),
        &[0x0a, 3, 0x0a, 1, b'x']
    );
    vector!(
        CommitBatchRequest,
        mutations = vec![mutation()],
        &[0x12, 2, 0x08, 1]
    );
    vector!(
        CommitBatchResponse,
        resources = vec![envelope()],
        &[0x0a, 3, 0x12, 1, 1]
    );
    vector!(CommitBatchResponse, revision = 1, &[0x10, 1]);
    vector!(
        CommitBatchResponse,
        error = MessageField::some(resource_error()),
        &[0x1a, 2, 0x08, 1]
    );

    vector!(
        ResolveRefRequest,
        meta = MessageField::some(meta()),
        &[0x0a, 3, 0x0a, 1, b'x']
    );
    vector!(
        ResolveRefRequest,
        target = MessageField::some(identity()),
        &[0x12, 3, 0x0a, 1, b'x']
    );
    vector!(
        ResolveRefResponse,
        resource = MessageField::some(identity()),
        &[0x0a, 3, 0x0a, 1, b'x']
    );
    vector!(
        ResolveRefResponse,
        error = MessageField::some(resource_error()),
        &[0x12, 2, 0x08, 1]
    );

    vector!(
        InspectSchemaRequest,
        meta = MessageField::some(meta()),
        &[0x0a, 3, 0x0a, 1, b'x']
    );
    vector!(
        InspectSchemaRequest,
        resource_type = "x".to_owned(),
        &[0x12, 1, b'x']
    );
    vector!(
        InspectSchemaResponse,
        schema = MessageField::some(envelope()),
        &[0x0a, 3, 0x12, 1, 1]
    );
    vector!(
        InspectSchemaResponse,
        error = MessageField::some(resource_error()),
        &[0x12, 2, 0x08, 1]
    );

    vector!(
        UpgradeRequest,
        meta = MessageField::some(meta()),
        &[0x0a, 3, 0x0a, 1, b'x']
    );
    vector!(
        UpgradeRequest,
        target = MessageField::some(identity()),
        &[0x12, 3, 0x0a, 1, b'x']
    );
    vector!(
        UpgradeRequest,
        action = EnumOrUnknown::new(wire::UpgradeAction::UPGRADE_ACTION_ASSESS),
        &[0x18, 1]
    );
    vector!(UpgradeRequest, recursive = true, &[0x20, 1]);
    vector!(
        UpgradeRequest,
        precondition = MessageField::some(precondition()),
        &[0x2a, 2, 0x08, 1]
    );
    vector!(
        UpgradeResponse,
        resource = MessageField::some(envelope()),
        &[0x0a, 3, 0x12, 1, 1]
    );
    vector!(
        UpgradeResponse,
        plan = vec![identity()],
        &[0x12, 3, 0x0a, 1, b'x']
    );
    vector!(UpgradeResponse, revision = 1, &[0x18, 1]);
    vector!(
        UpgradeResponse,
        error = MessageField::some(resource_error()),
        &[0x22, 2, 0x08, 1]
    );
}

#[test]
fn enum_discriminants_are_frozen_protocol_values() {
    assert_eq!(
        [
            wire::PreconditionKind::PRECONDITION_KIND_UNSPECIFIED.value(),
            wire::PreconditionKind::PRECONDITION_KIND_CREATE_ABSENT.value(),
            wire::PreconditionKind::PRECONDITION_KIND_EXACT_REVISION.value(),
        ],
        [0, 1, 2]
    );
    assert_eq!(
        [
            wire::ProjectionKind::PROJECTION_KIND_UNSPECIFIED.value(),
            wire::ProjectionKind::PROJECTION_KIND_FULL.value(),
            wire::ProjectionKind::PROJECTION_KIND_BASE_ONLY.value(),
            wire::ProjectionKind::PROJECTION_KIND_METADATA_ONLY.value(),
        ],
        [0, 1, 2, 3]
    );
    assert_eq!(
        [
            wire::ChangeKind::CHANGE_KIND_UNSPECIFIED.value(),
            wire::ChangeKind::CHANGE_KIND_CREATED.value(),
            wire::ChangeKind::CHANGE_KIND_UPDATED.value(),
            wire::ChangeKind::CHANGE_KIND_DELETED.value(),
        ],
        [0, 1, 2, 3]
    );
    assert_eq!(
        [
            wire::MutationKind::MUTATION_KIND_UNSPECIFIED.value(),
            wire::MutationKind::MUTATION_KIND_CREATE.value(),
            wire::MutationKind::MUTATION_KIND_UPDATE_SPEC.value(),
            wire::MutationKind::MUTATION_KIND_UPDATE_STATUS.value(),
            wire::MutationKind::MUTATION_KIND_UPDATE_METADATA.value(),
            wire::MutationKind::MUTATION_KIND_UPDATE_FINALIZERS.value(),
            wire::MutationKind::MUTATION_KIND_DELETE.value(),
        ],
        [0, 1, 2, 3, 4, 5, 6]
    );
    assert_eq!(
        [
            wire::UpgradeAction::UPGRADE_ACTION_UNSPECIFIED.value(),
            wire::UpgradeAction::UPGRADE_ACTION_ASSESS.value(),
            wire::UpgradeAction::UPGRADE_ACTION_PLAN.value(),
            wire::UpgradeAction::UPGRADE_ACTION_EXECUTE.value(),
        ],
        [0, 1, 2, 3]
    );
    assert_eq!(
        [
            wire::ReconcileDisposition::RECONCILE_DISPOSITION_UNSPECIFIED.value(),
            wire::ReconcileDisposition::RECONCILE_DISPOSITION_CONVERGED.value(),
            wire::ReconcileDisposition::RECONCILE_DISPOSITION_PROGRESSING.value(),
            wire::ReconcileDisposition::RECONCILE_DISPOSITION_BLOCKED.value(),
            wire::ReconcileDisposition::RECONCILE_DISPOSITION_UPGRADE_REQUIRED.value(),
            wire::ReconcileDisposition::RECONCILE_DISPOSITION_FAILED.value(),
        ],
        [0, 1, 2, 3, 4, 5]
    );
    assert_eq!(
        [
            wire::StatusPersistence::STATUS_PERSISTENCE_UNSPECIFIED.value(),
            wire::StatusPersistence::STATUS_PERSISTENCE_PENDING.value(),
            wire::StatusPersistence::STATUS_PERSISTENCE_COMMITTED.value(),
        ],
        [0, 1, 2]
    );
    assert_eq!(
        [
            wire::RetryClass::RETRY_CLASS_UNSPECIFIED.value(),
            wire::RetryClass::RETRY_CLASS_NEVER.value(),
            wire::RetryClass::RETRY_CLASS_IMMEDIATE.value(),
            wire::RetryClass::RETRY_CLASS_AFTER_DELAY.value(),
            wire::RetryClass::RETRY_CLASS_REAUTHORIZE.value(),
        ],
        [0, 1, 2, 3, 4]
    );
    assert_eq!(
        [
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_UNSPECIFIED.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_NOT_FOUND.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_ALREADY_EXISTS.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_CONFLICT.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_SCHEMA_INVALID.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_REF_INVALID.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_OWNER_CYCLE.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_OWNER_DEPTH.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_FINALIZER_DENIED.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_PROVIDER_UNAVAILABLE.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_CONTROLLER_MISMATCH.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_STATUS_OWNER_MISMATCH.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_STATUS_OVERSIZE.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_STATUS_PROVIDER_SCHEMA_INVALID.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_STATUS_PROVIDER_OVERLAP.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_SPEC_PROVIDER_SCHEMA_INVALID.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_SPEC_PROVIDER_SHADOW.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_UNSUPPORTED_CAPABILITY.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_EXPEDITED_NOT_AUTHORIZED.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_EXPEDITED_QUOTA_EXCEEDED.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_EXPEDITED_RECONCILE_PENDING.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_UPGRADE_REQUIRED.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_ENDPOINT_RESOLVE_DENIED.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RELAY_DENIED.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_ROLE_RELAY_GRANT_RESTRICTED.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_AUTHORIZATION_DENIED.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_REVISION_EXPIRED.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_BACKPRESSURE.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_TIMEOUT.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_CANCELLED.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_RESOURCE_PLANE_UNAVAILABLE.value(),
            wire::ResourceErrorKind::RESOURCE_ERROR_KIND_INTERNAL_INTEGRITY_FAILURE.value(),
        ],
        [
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23,
            24, 25, 26, 27, 28, 29, 30, 31,
        ]
    );
}

#[test]
fn proto_has_one_dynamic_carrier_and_no_peer_supplied_authority() {
    let proto = include_str!("../proto/d2b-resource-v3.proto");
    assert_eq!(proto.matches("bytes canonical_json").count(), 1);
    for forbidden in [
        "google.protobuf.Any",
        "oneof ",
        "map<",
        "authenticated_subject",
        "subject_context",
        "role_binding",
        "authorization_outcome",
        "allow_decision",
    ] {
        assert!(
            !proto.contains(forbidden),
            "forbidden protobuf surface: {forbidden}"
        );
    }
}
