//! Decode contract for the sealed Azure VM recovery record.

use d2b_provider_guest_azure_virtual_machine::{AzureOperationHandle, AzureVmRecoveryState};
use serde_json::Value;

/// A record written before the in-flight operation was grouped: the
/// `operation` and `operationStartedAtUnixMs` pair, with the caller
/// choosing which side of the pair carries a value.
fn legacy_record(operation: Option<Value>, started_at: Option<u64>) -> String {
    serde_json::json!({
        "phase": "deleting",
        "finalizerInstalled": true,
        "operation": operation.unwrap_or(Value::Null),
        "operationStartedAtUnixMs": started_at.map_or(Value::Null, Value::from),
        "pendingDeleteOperationId": null,
        "bootstrapStartedAtUnixMs": null,
        "pskDeliveryAttempts": 1,
        "bootstrapServiceState": "Waiting",
    })
    .to_string()
}

#[test]
fn a_legacy_record_with_a_half_paired_operation_refuses_the_decode() {
    let handle = serde_json::to_value(
        AzureOperationHandle::from_core(b"opaque-operation").expect("a bounded operation handle"),
    )
    .expect("an operation handle serializes");

    // The write side emits both members or neither, and both load: the
    // pair folds into the grouped in-flight operation.
    let paired: AzureVmRecoveryState =
        serde_json::from_str(&legacy_record(Some(handle.clone()), Some(1_700_000_000_000)))
            .expect("a legacy record with both members present folds into the grouped shape");
    assert!(paired.in_flight_operation.is_some());

    let empty: AzureVmRecoveryState = serde_json::from_str(&legacy_record(None, None))
        .expect("a legacy record with neither member present carries no in-flight operation");
    assert!(empty.in_flight_operation.is_none());

    // A half-paired record is malformed and the decode refuses it rather
    // than folding one member away. The enclosing `Repr` is untagged, so
    // the refusal is observed on the error itself and not through the
    // inner message; the two controls above are what tie it to the half
    // pair instead of to some other property of the record.
    let operation_only = legacy_record(Some(handle), None);
    assert!(
        serde_json::from_str::<AzureVmRecoveryState>(&operation_only).is_err(),
        "an operation without its start stamp is refused, not folded"
    );
    let started_only = legacy_record(None, Some(1_700_000_000_000));
    assert!(
        serde_json::from_str::<AzureVmRecoveryState>(&started_only).is_err(),
        "a start stamp without its operation is refused, not folded"
    );
}
