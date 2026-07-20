//! Guest-service boundary for the retained libshpool data plane.

use d2b_contracts::{
    v2_component_session::{EndpointPurpose, EndpointRole, ServicePackage},
    v2_services::{StrictWireMessage, common, method_spec},
};
use protobuf::Enum;

use crate::{name::validate_shell_name, output::ServiceResult};

pub const PARENT_SERVICE_PACKAGE: &str = "d2b.guest.v2";
pub const PARENT_ENDPOINT_PURPOSE: &str = "guest-control";
pub const PARENT_ENDPOINT_ROLE: &str = "guest-agent";
pub const EXTERNAL_DATA_PLANE: &str = "libshpool";
pub const SERVICE_NAME: &str = "GuestService";
pub const METHOD_NAME: &str = "OpenShell";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParentSessionContract {
    pub service: ServicePackage,
    pub purpose: EndpointPurpose,
    pub role: EndpointRole,
}

pub const PARENT_SESSION_CONTRACT: ParentSessionContract = ParentSessionContract {
    service: ServicePackage::GuestV2,
    purpose: EndpointPurpose::GuestControl,
    role: EndpointRole::GuestAgent,
};

#[derive(Clone, PartialEq, Eq)]
pub struct AdmittedOpenShell {
    operation_id: String,
    shell_name: String,
    stream_id: String,
}

impl AdmittedOpenShell {
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    pub fn shell_name(&self) -> &str {
        &self.shell_name
    }

    pub fn stream_id(&self) -> &str {
        &self.stream_id
    }

    pub fn completed(self) -> Result<common::ServiceResponse, crate::output::OutputError> {
        ServiceResult {
            operation_id: self.operation_id,
            resource_handle: self.shell_name,
            stream_id: self.stream_id,
        }
        .into_response()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AdmissionError {
    #[error("retained-shell-service-contract-invalid")]
    Contract,
    #[error("retained-shell-service-shape-invalid")]
    Shape,
}

pub fn admit_open_shell(
    request: &common::ServiceRequest,
) -> Result<AdmittedOpenShell, AdmissionError> {
    let method = method_spec(PARENT_SERVICE_PACKAGE, SERVICE_NAME, METHOD_NAME)
        .ok_or(AdmissionError::Contract)?;
    if !method.mutating || !method.requires_idempotency {
        return Err(AdmissionError::Contract);
    }
    request
        .validate_wire(method.requires_idempotency)
        .map_err(|_| AdmissionError::Contract)?;
    if request.desired_state.value() != common::DesiredState::DESIRED_STATE_ATTACHED.value()
        || request.operation_id.is_empty()
        || request.resource_id.is_empty()
        || request.stream_id.is_empty()
        || !request.page_cursor.is_empty()
        || request.page_size != 0
        || !request.attachment_indexes.is_empty()
        || validate_shell_name(&request.resource_id).is_err()
    {
        return Err(AdmissionError::Shape);
    }
    Ok(AdmittedOpenShell {
        operation_id: request.operation_id.clone(),
        shell_name: request.resource_id.clone(),
        stream_id: request.stream_id.clone(),
    })
}

#[cfg(test)]
mod tests {
    use d2b_contracts::v2_services::{StrictWireMessage, common};
    use protobuf::{EnumOrUnknown, MessageField};

    use super::{
        METHOD_NAME, PARENT_ENDPOINT_PURPOSE, PARENT_ENDPOINT_ROLE, PARENT_SERVICE_PACKAGE,
        PARENT_SESSION_CONTRACT, SERVICE_NAME, admit_open_shell,
    };

    fn request() -> common::ServiceRequest {
        let mut metadata = common::RequestMetadata::new();
        metadata.request_id = vec![0x11; 16];
        metadata.correlation_id = "correlation-1".to_owned();
        metadata.trace_id = vec![0x22; 16];
        metadata.idempotency_key = vec![0x33; 16];
        metadata.issued_at_unix_ms = 1_000;
        metadata.expires_at_unix_ms = 2_000;
        metadata.session_generation = 1;
        let mut scope = common::IdentityScope::new();
        scope.realm_id = "aaaaaaaaaaaaaaaaaaaa".to_owned();
        let mut request = common::ServiceRequest::new();
        request.metadata = MessageField::some(metadata);
        request.scope = MessageField::some(scope);
        request.resource_id = "default".to_owned();
        request.operation_id = "operation-1".to_owned();
        request.stream_id = "terminal-1".to_owned();
        request.desired_state = EnumOrUnknown::new(common::DesiredState::DESIRED_STATE_ATTACHED);
        request
    }

    #[test]
    fn composition_keys_match_typed_component_session_contract() {
        assert_eq!(
            PARENT_SESSION_CONTRACT.service.as_str(),
            PARENT_SERVICE_PACKAGE
        );
        assert_eq!(
            PARENT_SESSION_CONTRACT.purpose.as_str(),
            PARENT_ENDPOINT_PURPOSE
        );
        assert_eq!(PARENT_SESSION_CONTRACT.role.as_str(), PARENT_ENDPOINT_ROLE);
        let method = d2b_contracts::v2_services::method_spec(
            PARENT_SERVICE_PACKAGE,
            SERVICE_NAME,
            METHOD_NAME,
        )
        .unwrap();
        assert!(method.mutating);
        assert!(method.requires_idempotency);
    }

    #[test]
    fn admits_only_strict_attached_open_shell_requests() {
        let admitted = admit_open_shell(&request()).unwrap();
        assert_eq!(admitted.operation_id(), "operation-1");
        assert_eq!(admitted.shell_name(), "default");
        assert_eq!(admitted.stream_id(), "terminal-1");
        admitted.completed().unwrap().validate_wire(false).unwrap();

        let mut invalid = request();
        invalid.metadata.as_mut().unwrap().idempotency_key.clear();
        assert!(admit_open_shell(&invalid).is_err());

        let mut invalid = request();
        invalid.desired_state = EnumOrUnknown::new(common::DesiredState::DESIRED_STATE_DETACHED);
        assert!(admit_open_shell(&invalid).is_err());

        let mut invalid = request();
        invalid.resource_id = "work-{workspace}".to_owned();
        assert!(admit_open_shell(&invalid).is_err());
    }
}
