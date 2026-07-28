use core::fmt;

use d2b_contracts::v3::ServiceName;
use d2b_resource_api::authz::SessionVerb;

use crate::{Result, SessionError, contract::SessionErrorCode};

/// Whether an exact generated operation is unary or streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OperationKind {
    Method,
    Stream,
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
        let diagnostic_valid = match service.as_str() {
            "d2b.audit.v3" => member.is_method() && member.as_str() == "AuditService/Export",
            "d2b.support.v3" => {
                member.is_method() && member.as_str() == "SupportService/GenerateBundle"
            }
            _ => true,
        };
        if !diagnostic_valid {
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
        match self.service.as_str() {
            "d2b.audit.v3" => Some(SessionVerb::AuditExport),
            "d2b.support.v3" => Some(SessionVerb::SupportBundle),
            _ => None,
        }
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
