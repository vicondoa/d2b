use core::fmt;

use d2b_contracts::v3::ServiceName;
use d2b_resource_api::authz::{ApiMethod, SessionVerb};

use crate::{Result, SessionError, contract::SessionErrorCode};

/// Whether an exact generated operation is unary or streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperationKind {
    Method,
    Stream,
}

/// One generated service/member/kind binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationCatalogEntry {
    pub service: &'static str,
    pub member: &'static str,
    pub kind: OperationKind,
    pub resource_method: Option<ApiMethod>,
    pub verb: SessionVerb,
}

pub const GENERATED_OPERATION_CATALOG: &[OperationCatalogEntry] = &[
    resource("ResourceService/Get", OperationKind::Method, ApiMethod::Get),
    resource(
        "ResourceService/List",
        OperationKind::Method,
        ApiMethod::List,
    ),
    resource(
        "ResourceService/Watch",
        OperationKind::Method,
        ApiMethod::Watch,
    ),
    resource(
        "ResourceService/Watch",
        OperationKind::Stream,
        ApiMethod::Watch,
    ),
    resource(
        "ResourceService/Create",
        OperationKind::Method,
        ApiMethod::Create,
    ),
    resource(
        "ResourceService/UpdateSpec",
        OperationKind::Method,
        ApiMethod::UpdateSpec,
    ),
    resource(
        "ResourceService/UpdateStatus",
        OperationKind::Method,
        ApiMethod::UpdateStatus,
    ),
    resource(
        "ResourceService/UpdateMetadata",
        OperationKind::Method,
        ApiMethod::UpdateMetadata,
    ),
    resource(
        "ResourceService/UpdateFinalizers",
        OperationKind::Method,
        ApiMethod::UpdateFinalizers,
    ),
    resource(
        "ResourceService/Delete",
        OperationKind::Method,
        ApiMethod::Delete,
    ),
    resource(
        "ResourceService/CommitBatch",
        OperationKind::Method,
        ApiMethod::CommitBatch,
    ),
    resource(
        "ResourceService/ResolveRef",
        OperationKind::Method,
        ApiMethod::ResolveRef,
    ),
    resource(
        "ResourceService/InspectSchema",
        OperationKind::Method,
        ApiMethod::InspectSchema,
    ),
    resource(
        "ResourceService/Upgrade",
        OperationKind::Method,
        ApiMethod::Upgrade,
    ),
    OperationCatalogEntry {
        service: "d2b.audit.v3",
        member: "AuditService/Export",
        kind: OperationKind::Method,
        resource_method: None,
        verb: SessionVerb::AuditExport,
    },
    OperationCatalogEntry {
        service: "d2b.support.v3",
        member: "SupportService/GenerateBundle",
        kind: OperationKind::Method,
        resource_method: None,
        verb: SessionVerb::SupportBundle,
    },
];

const fn resource(
    member: &'static str,
    kind: OperationKind,
    method: ApiMethod,
) -> OperationCatalogEntry {
    OperationCatalogEntry {
        service: "d2b.resource.v3",
        member,
        kind,
        resource_method: Some(method),
        verb: if matches!(kind, OperationKind::Stream) {
            SessionVerb::OpenStream
        } else {
            SessionVerb::Invoke
        },
    }
}

pub fn operation_catalog_entry(
    service: &str,
    member: &str,
    kind: OperationKind,
) -> Option<&'static OperationCatalogEntry> {
    GENERATED_OPERATION_CATALOG
        .iter()
        .find(|entry| entry.service == service && entry.member == member && entry.kind == kind)
}

pub fn resource_operation(method: ApiMethod) -> &'static OperationCatalogEntry {
    GENERATED_OPERATION_CATALOG
        .iter()
        .find(|entry| entry.resource_method == Some(method) && entry.kind == OperationKind::Method)
        .expect("every ApiMethod has one unary ResourceService member")
}

/// Canonically spelled generated service member.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OperationMember {
    kind: OperationKind,
    value: String,
}

impl OperationMember {
    /// Parse a unary member using the canonical `Service/Member` spelling.
    pub fn method(value: impl Into<String>) -> Result<Self> {
        Self::parse(OperationKind::Method, value.into())
    }

    /// Parse a streaming member using the canonical `Service/Member` spelling.
    pub fn stream(value: impl Into<String>) -> Result<Self> {
        Self::parse(OperationKind::Stream, value.into())
    }

    fn parse(kind: OperationKind, value: String) -> Result<Self> {
        let mut components = value.split('/');
        let service = components.next().unwrap_or_default();
        let member = components.next().unwrap_or_default();
        if value.len() > 128
            || components.next().is_some()
            || !valid_identifier(service)
            || !valid_identifier(member)
        {
            return Err(SessionError::new(SessionErrorCode::PolicyDenied));
        }
        Ok(Self { kind, value })
    }

    /// Borrow the exact canonical member spelling.
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Return whether this member names a unary method.
    pub const fn is_method(&self) -> bool {
        matches!(self.kind, OperationKind::Method)
    }

    /// Return whether this member names a named stream.
    pub const fn is_stream(&self) -> bool {
        matches!(self.kind, OperationKind::Stream)
    }
}

impl fmt::Debug for OperationMember {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationMember")
            .field("kind", &self.kind)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// Exact service and member pair shared by session admission and bus routing.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionOperation {
    service: ServiceName,
    member: OperationMember,
}

impl SessionOperation {
    /// Bind a service to a canonical generated member.
    pub fn new(service: ServiceName, member: OperationMember) -> Result<Self> {
        let known_service = GENERATED_OPERATION_CATALOG
            .iter()
            .any(|entry| entry.service == service.as_str());
        let kind = if member.is_method() {
            OperationKind::Method
        } else {
            OperationKind::Stream
        };
        if known_service
            && operation_catalog_entry(service.as_str(), member.as_str(), kind).is_none()
        {
            return Err(SessionError::new(SessionErrorCode::PolicyDenied));
        }
        Ok(Self { service, member })
    }

    /// Parse a unary operation.
    pub fn method(service: ServiceName, member: impl Into<String>) -> Result<Self> {
        Self::new(service, OperationMember::method(member)?)
    }

    /// Parse a named-stream operation.
    pub fn stream(service: ServiceName, member: impl Into<String>) -> Result<Self> {
        Self::new(service, OperationMember::stream(member)?)
    }

    /// Borrow the exact service.
    pub const fn service(&self) -> &ServiceName {
        &self.service
    }

    /// Borrow the canonical member.
    pub const fn member(&self) -> &OperationMember {
        &self.member
    }

    /// Resolve diagnostic operations to their closed native verb.
    pub fn required_verb(&self, ordinary: SessionVerb) -> SessionVerb {
        self.diagnostic_verb().unwrap_or(ordinary)
    }

    /// Return the closed diagnostic verb, when this is a diagnostic operation.
    pub fn diagnostic_verb(&self) -> Option<SessionVerb> {
        let kind = if self.member.is_method() {
            OperationKind::Method
        } else {
            OperationKind::Stream
        };
        operation_catalog_entry(self.service.as_str(), self.member.as_str(), kind)
            .map(|entry| entry.verb)
            .filter(|verb| matches!(verb, SessionVerb::AuditExport | SessionVerb::SupportBundle))
    }
}

impl fmt::Debug for SessionOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionOperation(<redacted>)")
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(value: &str) -> ServiceName {
        ServiceName::parse(value).unwrap()
    }

    #[test]
    fn canonical_member_has_one_slash_and_two_identifiers() {
        assert!(OperationMember::method("ResourceService/Get").is_ok());
        for invalid in [
            "",
            "ResourceService.Get",
            "/ResourceService/Get",
            "ResourceService/Get/",
            "ResourceService//Get",
            "ResourceService/Get?",
        ] {
            assert!(OperationMember::method(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn diagnostics_are_exact_and_resolve_closed_verbs() {
        let audit =
            SessionOperation::method(service("d2b.audit.v3"), "AuditService/Export").unwrap();
        assert_eq!(
            audit.required_verb(SessionVerb::Invoke),
            SessionVerb::AuditExport
        );
        assert!(SessionOperation::method(service("d2b.audit.v3"), "AuditService/Inspect").is_err());
        assert!(
            SessionOperation::stream(service("d2b.support.v3"), "SupportService/GenerateBundle")
                .is_err()
        );
    }
}
