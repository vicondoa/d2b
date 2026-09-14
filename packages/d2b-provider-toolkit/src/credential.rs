//! Credential Provider realization shared by the Credential realizer crates.
//!
//! `Provider/credential-secret-service`, `Provider/credential-entra`, and
//! `Provider/credential-managed-identity` are three separate Provider
//! binaries with three separate Provider references, but they emit one audit
//! record shape, one telemetry frame shape, and one dispatch shape. The
//! closed vocabulary for all three lives in the shared v3 contract catalog;
//! what the crates were repeating is how a Provider *uses* it. That use is
//! here, keyed on [`CredentialProviderKind`], so a change to the record, the
//! frame, or the dispatch seam cannot reach two of the three and miss the
//! third.

use d2b_contracts_provider::v3::credential::{
    CredentialAuthorization, CredentialMethod, CredentialProvider, CredentialRequest,
    CredentialResponse, CredentialServiceError, PlacementBinding,
    dispatch_authorized_provider_async,
};
use d2b_contracts_provider::v3::credential_controller::{
    CredentialAuditDigest, CredentialAuditOutcome, CredentialAuditRecord, CredentialObservabilityError,
    CredentialProviderKind, CredentialTelemetryFrame, CredentialTelemetryOperation,
    CredentialTelemetryOutcome,
};

/// Build the one caller-initiated audit record for a Credential service call.
///
/// A denied call returns no identity-bearing record and never inspects the
/// presented identity, so the digests are derived only after authorization.
#[allow(clippy::too_many_arguments)]
pub fn authorized_service_record(
    provider: CredentialProviderKind,
    authorized: bool,
    zone: &str,
    subject_identity: &[u8],
    credential_name: &[u8],
    method: CredentialMethod,
    outcome: CredentialAuditOutcome,
    rotation_generation: u64,
    idempotency_key: Option<&[u8]>,
) -> Result<Option<CredentialAuditRecord>, CredentialObservabilityError> {
    if !authorized {
        return CredentialAuditRecord::authorized_service(
            false,
            provider,
            "",
            "",
            "",
            method,
            outcome,
            rotation_generation,
            None,
        );
    }
    let subject = CredentialAuditDigest::after_authorization(subject_identity);
    let resource = CredentialAuditDigest::after_authorization(credential_name);
    let idempotency = idempotency_key.map(CredentialAuditDigest::after_authorization);
    CredentialAuditRecord::authorized_service(
        true,
        provider,
        zone,
        subject.as_str(),
        resource.as_str(),
        method,
        outcome,
        rotation_generation,
        idempotency.map(|digest| digest.as_str().to_owned()),
    )
}

/// Build the one closed telemetry frame for a Credential service call.
pub fn credential_frame(
    provider: CredentialProviderKind,
    zone: &str,
    operation: CredentialTelemetryOperation,
    outcome: CredentialTelemetryOutcome,
    placement: PlacementBinding,
    rotation_generation: u64,
    service_version: &'static str,
) -> Result<CredentialTelemetryFrame, CredentialObservabilityError> {
    CredentialTelemetryFrame::new(
        provider,
        zone,
        operation,
        outcome,
        placement,
        rotation_generation,
        service_version,
    )
}

/// Run one admitted dispatch to completion for a caller that holds no runtime.
///
/// The realizer crates implement [`CredentialProvider::dispatch_async`] and
/// reach the synchronous half of the trait through here, so the two halves
/// cannot diverge. The service loop itself never takes this path: it awaits
/// the asynchronous half directly.
pub fn dispatch_blocking<P: CredentialProvider + ?Sized>(
    provider: &P,
    method: CredentialMethod,
    request: &CredentialRequest,
    authorization: &CredentialAuthorization,
) -> Result<CredentialResponse, CredentialServiceError> {
    block_on(dispatch_authorized_provider_async(
        provider,
        method,
        request,
        authorization,
    ))
}

/// Drive a future to completion on a current-thread runtime.
///
/// The future is always polled inside a runtime, never on a bare first poll:
/// a future that constructs a timer while polling, as the Credential clients
/// do through `tokio::time::timeout`, reads the runtime's timer handle at
/// construction time and would panic outside one even when it is ready
/// immediately. The runtime is built once per thread and reused, so a
/// caller that dispatches repeatedly pays for it once.
fn block_on<F: Future>(future: F) -> F::Output {
    thread_local! {
        static RUNTIME: tokio::runtime::Runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime is available in a Provider process");
    }
    RUNTIME.with(|runtime| runtime.block_on(future))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_denied_call_yields_no_identity_bearing_record() {
        let marker = format!("toolkit-canary-{:x}", std::process::id());
        let record = authorized_service_record(
            CredentialProviderKind::Entra,
            false,
            "dev",
            marker.as_bytes(),
            marker.as_bytes(),
            CredentialMethod::AcquireToken,
            CredentialAuditOutcome::Denied,
            1,
            Some(marker.as_bytes()),
        )
        .unwrap();
        assert!(record.is_none());
    }

    #[test]
    fn an_authorized_call_never_renders_the_presented_identity() {
        let marker = format!("toolkit-canary-{:x}", std::process::id());
        let record = authorized_service_record(
            CredentialProviderKind::ManagedIdentity,
            true,
            "dev",
            marker.as_bytes(),
            marker.as_bytes(),
            CredentialMethod::AcquireToken,
            CredentialAuditOutcome::Success,
            1,
            Some(marker.as_bytes()),
        )
        .unwrap()
        .unwrap();
        assert!(!record.to_wire_record().contains(&marker));
        assert!(!format!("{record:?}").contains(&marker));
    }

    #[test]
    fn a_frame_carries_only_closed_values() {
        let frame = credential_frame(
            CredentialProviderKind::SecretService,
            "dev",
            CredentialTelemetryOperation::AcquireToken,
            CredentialTelemetryOutcome::Success,
            PlacementBinding::UserAgent,
            1,
            env!("CARGO_PKG_VERSION"),
        )
        .unwrap();
        assert!(
            CredentialTelemetryFrame::validate_collector_fields(frame.all_fields()).is_ok(),
            "every field of a frame built here is a closed value"
        );
    }

    #[test]
    fn a_future_that_arms_a_timer_while_polling_completes_without_a_runtime() {
        // The Credential clients bound every call with `tokio::time::timeout`,
        // which reads the runtime's timer handle while the future is being
        // polled. Driving the synchronous dispatch half outside a runtime must
        // therefore enter one, not poll bare.
        let value = block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(30), async { 7_u8 }).await
        });
        assert_eq!(value.expect("the inner future is ready"), 7);
    }
}
