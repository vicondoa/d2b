use d2b_contracts::v2_services::{StrictWireMessage, common};
use protobuf::{EnumOrUnknown, MessageField};

#[derive(Clone, PartialEq, Eq)]
pub struct ServiceResult {
    pub operation_id: String,
    pub resource_handle: String,
    pub stream_id: String,
}

impl ServiceResult {
    pub fn into_response(self) -> Result<common::ServiceResponse, OutputError> {
        let mut response = common::ServiceResponse::new();
        response.outcome = EnumOrUnknown::new(common::Outcome::OUTCOME_SUCCEEDED);
        response.operation_id = self.operation_id;
        response.resource_handle = self.resource_handle;
        response.stream_id = self.stream_id;
        response
            .validate_wire(false)
            .map_err(|_| OutputError::InvalidServiceResponse)?;
        Ok(response)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum OutputError {
    #[error("retained-shell-service-response-invalid")]
    InvalidServiceResponse,
}

pub fn capability_denied(
    operation_id: String,
    correlation_id: String,
) -> Result<common::ServiceResponse, OutputError> {
    let mut error = common::ErrorEnvelope::new();
    error.kind = EnumOrUnknown::new(common::ErrorKind::ERROR_KIND_CAPABILITY_DENIED);
    error.retry = EnumOrUnknown::new(common::RetryClass::RETRY_CLASS_NEVER);
    error.correlation_id = correlation_id;

    let mut response = common::ServiceResponse::new();
    response.outcome = EnumOrUnknown::new(common::Outcome::OUTCOME_DENIED);
    response.operation_id = operation_id;
    response.error = MessageField::some(error);
    response
        .validate_wire(false)
        .map_err(|_| OutputError::InvalidServiceResponse)?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use d2b_contracts::v2_services::{StrictWireMessage, common};
    use protobuf::Enum;

    #[test]
    fn success_uses_frozen_service_response() {
        let response = super::ServiceResult {
            operation_id: "operation-1".to_owned(),
            resource_handle: "default".to_owned(),
            stream_id: "terminal-1".to_owned(),
        }
        .into_response()
        .unwrap();

        assert_eq!(
            response.outcome.value(),
            common::Outcome::OUTCOME_SUCCEEDED.value()
        );
        response.validate_wire(false).unwrap();
    }

    #[test]
    fn denial_is_typed_and_strict() {
        let response =
            super::capability_denied("operation-1".to_owned(), "correlation-1".to_owned()).unwrap();
        assert_eq!(
            response.outcome.value(),
            common::Outcome::OUTCOME_DENIED.value()
        );
        response.validate_wire(false).unwrap();
    }
}
