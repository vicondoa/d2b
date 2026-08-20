use d2b_contracts_zone_session::v3::{
    DEFAULT_LIST_PAGE_SIZE, MAX_FILTER_VALUES, MAX_LIST_FILTERS, MAX_LIST_PAGE_SIZE,
    MAX_LIST_RESOURCE_TYPES, MAX_PAGE_CURSOR_BYTES, ResourceErrorKind, ResourceName, ResourceRef,
    ResourceTypeName,
};
use d2b_resource_store::{StoreFilter, StoreProjection};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceRuntimeError {
    /// The broker response did not describe the requested opaque store.
    BrokerResponseMismatch,
    /// The broker response did not carry exactly one descriptor.
    BrokerFdCountMismatch,
    /// The response disposition was not in the closed contract.
    BrokerDispositionInvalid,
    /// The Zone store id was not the canonical per-Zone id.
    ZoneStoreIdInvalid,
    /// The descriptor was not accepted by the production backend.
    StoreOpenFailed,
    /// The native API authorizer or store seal could not be constructed.
    AuthorizationUnavailable,
    /// The API could not consume its one store admission binding.
    ResourceApiBindFailed,
    /// The runtime could not issue the store-instance seal acceptor.
    StoreSealUnavailable,
    /// The fixed core process could not reach readiness.
    CoreStartupFailed,
    /// The Zone is already owned by this plane.
    DuplicateZone,
    /// The runtime has no ready resource plane.
    PlaneUnavailable,
    /// A CLI request did not match the authoritative Zone route.
    RouteMismatch,
    /// The CLI request is outside the bounded read-only adapter surface.
    RequestInvalid,
    /// The Resource API returned a malformed canonical response.
    ResponseInvalid,
    /// The underlying store refused a read.
    StoreReadFailed,
    /// The installed native policy is not available for this Zone.
    PolicyUnavailable,
    /// The production ResourceService endpoint is not registered.
    ControllerEndpointUnavailable,
    /// The authenticated Zone session is not available.
    AuthenticationUnavailable,
    /// The store watch/recovery admission is not available.
    WatchUnavailable,
    /// Durable Host-global authority proofs have not crossed the startup
    /// barrier.
    AuthorityUnavailable,
    /// The fixed core handlers have not converged.
    HandlerNotReady,
    /// The provider path has not completed startup.
    ProviderPathUnavailable,
    /// Committed interaction Provider configuration is absent or invalid.
    InteractionConfigurationUnavailable,
    /// A shutdown was refused because request owners are still live.
    LiveRequestOwners,
    /// No authenticated subject was bound to the request.
    IdentityUnbound,
    /// The requested operation is not exposed by the registered service.
    CapabilityUnavailable,
    /// The public Resource API returned a typed error while loading a
    /// resource for provider reconciliation.
    ResourceGetFailed(ResourceErrorKind),
    /// The public Resource API refused a provider status update with a typed
    /// error.
    ResourceStatusUpdateFailed(ResourceErrorKind),
    /// The Wave 6 operator acceptance boundary did not converge.
    Wave6AcceptanceFailed,
}

impl ResourceRuntimeError {
    /// Stable, identity-free error label.
    pub const fn code(self) -> &'static str {
        match self {
            Self::BrokerResponseMismatch => "resource-runtime-broker-response-mismatch",
            Self::BrokerFdCountMismatch => "resource-runtime-broker-fd-count-mismatch",
            Self::BrokerDispositionInvalid => "resource-runtime-broker-disposition-invalid",
            Self::ZoneStoreIdInvalid => "resource-runtime-zone-store-id-invalid",
            Self::StoreOpenFailed => "resource-runtime-store-open-failed",
            Self::AuthorizationUnavailable => "resource-runtime-authorization-unavailable",
            Self::ResourceApiBindFailed => "resource-runtime-api-bind-failed",
            Self::StoreSealUnavailable => "resource-runtime-store-seal-unavailable",
            Self::CoreStartupFailed => "resource-runtime-core-startup-failed",
            Self::DuplicateZone => "resource-runtime-duplicate-zone",
            Self::PlaneUnavailable => "resource-runtime-plane-unavailable",
            Self::RouteMismatch => "resource-runtime-route-mismatch",
            Self::RequestInvalid => "resource-runtime-request-invalid",
            Self::ResponseInvalid => "resource-runtime-response-invalid",
            Self::StoreReadFailed => "resource-runtime-store-read-failed",
            Self::PolicyUnavailable => "resource-runtime-policy-unavailable",
            Self::ControllerEndpointUnavailable => {
                "resource-runtime-controller-endpoint-unavailable"
            }
            Self::AuthenticationUnavailable => "resource-runtime-authentication-unavailable",
            Self::WatchUnavailable => "resource-runtime-watch-unavailable",
            Self::AuthorityUnavailable => "resource-runtime-authority-unavailable",
            Self::HandlerNotReady => "resource-runtime-handler-not-ready",
            Self::ProviderPathUnavailable => "resource-runtime-provider-path-unavailable",
            Self::InteractionConfigurationUnavailable => {
                "resource-runtime-interaction-configuration-unavailable"
            }
            Self::LiveRequestOwners => "resource-runtime-live-request-owners",
            Self::IdentityUnbound => "resource-runtime-identity-unbound",
            Self::CapabilityUnavailable => "resource-runtime-capability-unavailable",
            Self::ResourceGetFailed(_) => "resource-runtime-resource-get-failed",
            Self::ResourceStatusUpdateFailed(_) => "resource-runtime-resource-status-update-failed",
            Self::Wave6AcceptanceFailed => "resource-runtime-wave6-acceptance-failed",
        }
    }
}

impl core::fmt::Display for ResourceRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ResourceRuntimeError {}

pub fn resource_runtime_error_frame(error: ResourceRuntimeError) -> Value {
    let code = error.code();
    let (kind, retry_class, message, remediation) = match error {
        ResourceRuntimeError::AuthenticationUnavailable
        | ResourceRuntimeError::ControllerEndpointUnavailable
        | ResourceRuntimeError::PolicyUnavailable
        | ResourceRuntimeError::IdentityUnbound => (
            "authorization-denied",
            "reauthorize",
            "authenticated Zone ComponentSession or installed policy is unavailable",
            "establish an authenticated local Zone session and install its matching policy before retrying",
        ),
        ResourceRuntimeError::WatchUnavailable
        | ResourceRuntimeError::HandlerNotReady
        | ResourceRuntimeError::ProviderPathUnavailable
        | ResourceRuntimeError::PlaneUnavailable
        | ResourceRuntimeError::CoreStartupFailed => (
            "resource-plane-unavailable",
            "after-delay",
            "the Zone resource runtime has not completed readiness",
            "wait for authoritative Zone startup and retry after the resource plane is published",
        ),
        ResourceRuntimeError::CapabilityUnavailable => (
            code,
            "never",
            "the requested resource operation is not registered",
            "use a method exposed by the registered Zone service",
        ),
        ResourceRuntimeError::ResourceGetFailed(kind) => (
            kind.as_str(),
            match kind {
                ResourceErrorKind::AuthorizationDenied => "reauthorize",
                ResourceErrorKind::ResourcePlaneUnavailable => "after-delay",
                ResourceErrorKind::Backpressure => "immediate",
                _ => "never",
            },
            "the public Resource API returned a typed error",
            "follow the typed Resource API error before retrying",
        ),
        ResourceRuntimeError::ResourceStatusUpdateFailed(kind) => (
            kind.as_str(),
            match kind {
                ResourceErrorKind::AuthorizationDenied => "reauthorize",
                ResourceErrorKind::ResourcePlaneUnavailable => "after-delay",
                ResourceErrorKind::Backpressure => "immediate",
                _ => "never",
            },
            "the public Resource API refused the status update",
            "follow the typed Resource API error before retrying",
        ),
        _ => (
            code,
            "never",
            code,
            "inspect the typed resource error and the authoritative Zone route",
        ),
    };
    json!({
        "type": "error",
        "error": {
            "kind": kind,
            "errorClass": kind,
            "retryClass": retry_class,
            "message": message,
            "remediation": remediation,
        }
    })
}

pub fn route_service_matches(
    service: Option<&Value>,
    method: &str,
) -> Result<bool, ResourceRuntimeError> {
    let Some(service) = service else {
        return Ok(true);
    };
    let service = service
        .as_str()
        .ok_or(ResourceRuntimeError::RequestInvalid)?;
    Ok(match method {
        "ZoneList" | "ZoneStatus" => service == "d2b.zone.v3",
        _ => service == "d2b.resource.v3",
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedListRequest {
    pub resource_types: Vec<ResourceTypeName>,
    pub resource_names: Vec<ResourceName>,
    pub filters: Vec<StoreFilter>,
    pub page_size: u32,
    pub cursor: Option<String>,
    pub projection: StoreProjection,
}

pub fn parse_list_request(request: &Value) -> Result<ParsedListRequest, ResourceRuntimeError> {
    const LIST_FIELDS: &[&str] = &[
        "type",
        "service",
        "method",
        "zoneRef",
        "schemaVersion",
        "sessionVerb",
        "operationId",
        "resourceType",
        "resourceTypes",
        "resourceNames",
        "filters",
        "limit",
        "pageSize",
        "pageToken",
        "cursor",
        "pageCursor",
        "executionRef",
        "domain",
        "phase",
        "labelSelector",
        "updates",
        "revisionCursor",
        "sinceRevision",
        "afterRevision",
        "revision",
        "projection",
    ];
    if request.as_object().is_none_or(|object| {
        object
            .keys()
            .any(|key| !LIST_FIELDS.contains(&key.as_str()))
    }) {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    let singular_type = request
        .get("resourceType")
        .and_then(Value::as_str)
        .map(|value| {
            ResourceTypeName::parse(value).map_err(|_| ResourceRuntimeError::RequestInvalid)
        })
        .transpose()?;
    let resource_types = match request.get("resourceTypes") {
        None | Some(Value::Null) => singular_type
            .clone()
            .map(|resource_type| vec![resource_type])
            .ok_or(ResourceRuntimeError::RequestInvalid)?,
        Some(value) => {
            let values = value
                .as_array()
                .ok_or(ResourceRuntimeError::RequestInvalid)?;
            if values.is_empty() || values.len() > MAX_LIST_RESOURCE_TYPES {
                return Err(ResourceRuntimeError::RequestInvalid);
            }
            let parsed = values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .ok_or(ResourceRuntimeError::RequestInvalid)
                        .and_then(|value| {
                            ResourceTypeName::parse(value)
                                .map_err(|_| ResourceRuntimeError::RequestInvalid)
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(singular_type) = singular_type
                && (parsed.len() != 1 || parsed[0] != singular_type)
            {
                return Err(ResourceRuntimeError::RequestInvalid);
            }
            parsed
        }
    };

    let mut resource_names = parse_resource_names(request)?;
    let mut filters = parse_typed_filters(request)?;
    for filter in &filters {
        if filter.field == "metadata.name" {
            resource_names.extend(
                filter
                    .values
                    .iter()
                    .map(|value| {
                        ResourceName::parse(value).map_err(|_| ResourceRuntimeError::RequestInvalid)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
    }
    if let Some(selector) = request.get("labelSelector")
        && !selector.is_null()
    {
        let selector = selector
            .as_str()
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        if !selector.is_empty() {
            let (field, value) = selector
                .split_once('=')
                .ok_or(ResourceRuntimeError::RequestInvalid)?;
            if filters.len() >= MAX_LIST_FILTERS {
                return Err(ResourceRuntimeError::RequestInvalid);
            }
            let filter = typed_filter(field, vec![value.to_owned()])?;
            if field == "metadata.name" {
                resource_names.extend(
                    filter
                        .values
                        .iter()
                        .map(|value| {
                            ResourceName::parse(value)
                                .map_err(|_| ResourceRuntimeError::RequestInvalid)
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
            filters.push(filter);
        }
    }
    if let Some(domain) = request.get("domain").and_then(Value::as_str)
        && !domain.is_empty()
        && !matches!(domain, "system" | "user")
    {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    if let Some(execution_ref) = request.get("executionRef")
        && !execution_ref.is_null()
    {
        let execution_ref = execution_ref
            .as_str()
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        if !execution_ref.is_empty() {
            ResourceRef::parse(execution_ref).map_err(|_| ResourceRuntimeError::RequestInvalid)?;
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
    }
    optional_capability_string(request, "domain", 16)?;
    optional_capability_string(request, "phase", 128)?;
    if let Some(schema_version) = request.get("schemaVersion")
        && !schema_version.is_null()
        && schema_version.as_u64() != Some(1)
    {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    if let Some(session_verb) = request.get("sessionVerb")
        && !session_verb.is_null()
        && session_verb.as_str() != Some("Invoke")
    {
        return Err(ResourceRuntimeError::CapabilityUnavailable);
    }
    if request.get("updates").is_some_and(|value| !value.is_null()) {
        let updates = request
            .get("updates")
            .and_then(Value::as_bool)
            .ok_or(ResourceRuntimeError::RequestInvalid)?;
        if updates {
            return Err(ResourceRuntimeError::CapabilityUnavailable);
        }
    }

    let cursor = aliased_cursor(request)?;
    let page_size = aliased_page_size(request)?;
    let projection = parse_projection(request.get("projection"))?;
    for field in [
        "revisionCursor",
        "sinceRevision",
        "afterRevision",
        "revision",
    ] {
        if let Some(value) = request.get(field) {
            if value.is_null() {
                continue;
            }
            let value = value.as_u64().ok_or(ResourceRuntimeError::RequestInvalid)?;
            if value != 0 {
                return Err(ResourceRuntimeError::CapabilityUnavailable);
            }
        }
    }

    Ok(ParsedListRequest {
        resource_types,
        resource_names,
        filters,
        page_size,
        cursor,
        projection,
    })
}

pub fn parse_resource_names(request: &Value) -> Result<Vec<ResourceName>, ResourceRuntimeError> {
    let Some(value) = request.get("resourceNames") else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let values = value
        .as_array()
        .ok_or(ResourceRuntimeError::RequestInvalid)?;
    if values.len() > MAX_FILTER_VALUES {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or(ResourceRuntimeError::RequestInvalid)
                .and_then(|value| {
                    ResourceName::parse(value).map_err(|_| ResourceRuntimeError::RequestInvalid)
                })
        })
        .collect()
}

pub fn parse_typed_filters(request: &Value) -> Result<Vec<StoreFilter>, ResourceRuntimeError> {
    let Some(value) = request.get("filters") else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let values = value
        .as_array()
        .ok_or(ResourceRuntimeError::RequestInvalid)?;
    if values.len() > MAX_LIST_FILTERS {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    values
        .iter()
        .map(|value| {
            let object = value
                .as_object()
                .ok_or(ResourceRuntimeError::RequestInvalid)?;
            if object
                .keys()
                .any(|key| !matches!(key.as_str(), "field" | "values"))
            {
                return Err(ResourceRuntimeError::RequestInvalid);
            }
            let field = object
                .get("field")
                .and_then(Value::as_str)
                .ok_or(ResourceRuntimeError::RequestInvalid)?;
            let values = object
                .get("values")
                .and_then(Value::as_array)
                .ok_or(ResourceRuntimeError::RequestInvalid)?
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .map(ToOwned::to_owned)
                        .ok_or(ResourceRuntimeError::RequestInvalid)
                })
                .collect::<Result<Vec<_>, _>>()?;
            typed_filter(field, values)
        })
        .collect()
}

pub fn typed_filter(field: &str, values: Vec<String>) -> Result<StoreFilter, ResourceRuntimeError> {
    if field.is_empty()
        || field.len() > 64
        || !field
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        || values.is_empty()
        || values.len() > MAX_FILTER_VALUES
        || values
            .iter()
            .any(|value| value.is_empty() || value.len() > 256)
    {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    if !matches!(field, "metadata.name" | "type") {
        return Err(ResourceRuntimeError::CapabilityUnavailable);
    }
    if field == "metadata.name" {
        for value in &values {
            ResourceName::parse(value).map_err(|_| ResourceRuntimeError::RequestInvalid)?;
        }
    } else {
        for value in &values {
            ResourceTypeName::parse(value).map_err(|_| ResourceRuntimeError::RequestInvalid)?;
        }
    }
    Ok(StoreFilter {
        field: field.to_owned(),
        values,
    })
}

pub fn optional_capability_string(
    request: &Value,
    field: &str,
    max_bytes: usize,
) -> Result<(), ResourceRuntimeError> {
    let Some(value) = request.get(field) else {
        return Ok(());
    };
    if value.is_null() {
        return Ok(());
    }
    let value = value.as_str().ok_or(ResourceRuntimeError::RequestInvalid)?;
    if value.len() > max_bytes {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    if !value.is_empty() {
        return Err(ResourceRuntimeError::CapabilityUnavailable);
    }
    Ok(())
}

pub fn aliased_cursor(request: &Value) -> Result<Option<String>, ResourceRuntimeError> {
    let mut values = Vec::new();
    for field in ["cursor", "pageCursor", "pageToken"] {
        let Some(value) = request.get(field) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        let value = value.as_str().ok_or(ResourceRuntimeError::RequestInvalid)?;
        if value.len() > MAX_PAGE_CURSOR_BYTES {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        values.push(value.to_owned());
    }
    if values.windows(2).any(|pair| pair[0] != pair[1]) {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    Ok(values.into_iter().next().filter(|value| !value.is_empty()))
}

pub fn aliased_page_size(request: &Value) -> Result<u32, ResourceRuntimeError> {
    let mut values = Vec::new();
    for field in ["pageSize", "limit"] {
        let Some(value) = request.get(field) else {
            continue;
        };
        if value.is_null() {
            continue;
        }
        let value = value.as_u64().ok_or(ResourceRuntimeError::RequestInvalid)?;
        if value == 0 || value > u64::from(MAX_LIST_PAGE_SIZE) {
            return Err(ResourceRuntimeError::RequestInvalid);
        }
        values.push(u32::try_from(value).map_err(|_| ResourceRuntimeError::RequestInvalid)?);
    }
    if values.windows(2).any(|pair| pair[0] != pair[1]) {
        return Err(ResourceRuntimeError::RequestInvalid);
    }
    Ok(values.first().copied().unwrap_or(DEFAULT_LIST_PAGE_SIZE))
}

pub fn parse_projection(value: Option<&Value>) -> Result<StoreProjection, ResourceRuntimeError> {
    let Some(value) = value else {
        return Ok(StoreProjection::Full);
    };
    if value.is_null() {
        return Ok(StoreProjection::Full);
    }
    let kind = match value {
        Value::String(value) => value.as_str(),
        Value::Object(object) => {
            if object.len() != 1 {
                return Err(ResourceRuntimeError::RequestInvalid);
            }
            object
                .get("kind")
                .and_then(Value::as_str)
                .ok_or(ResourceRuntimeError::RequestInvalid)?
        }
        _ => return Err(ResourceRuntimeError::RequestInvalid),
    };
    match kind {
        "full" | "FULL" | "projection-kind-full" => Ok(StoreProjection::Full),
        "baseOnly" | "base-only" | "BASE_ONLY" => Ok(StoreProjection::BaseOnly),
        "metadataOnly" | "metadata-only" | "METADATA_ONLY" => Ok(StoreProjection::MetadataOnly),
        _ => Err(ResourceRuntimeError::RequestInvalid),
    }
}
