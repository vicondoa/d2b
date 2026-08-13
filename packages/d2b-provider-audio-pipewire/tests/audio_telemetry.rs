use d2b_provider_audio_pipewire::telemetry::{
    AudioTelemetryError, AudioTelemetryOperation, record,
};

#[test]
fn telemetry_uses_closed_labels_and_rejects_identity_or_path_labels() {
    let observed = record(
        AudioTelemetryOperation::BindingReconcile,
        "ready",
        [("role".to_owned(), "owner".to_owned())],
    )
    .unwrap();
    assert_eq!(
        observed.labels.get("role").map(String::as_str),
        Some("owner")
    );
    assert!(matches!(
        record(
            AudioTelemetryOperation::BindingReconcile,
            "ready",
            [("zone".to_owned(), "zone-a".to_owned())],
        ),
        Err(AudioTelemetryError::InvalidLabel)
    ));
    assert!(matches!(
        record(
            AudioTelemetryOperation::BindingReconcile,
            "ready",
            [("role".to_owned(), "/run/private".to_owned())],
        ),
        Err(AudioTelemetryError::InvalidLabel)
    ));
}
