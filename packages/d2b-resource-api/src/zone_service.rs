//! Authenticated v3 Zone service dispatch seam.
//!
//! This module is the native Rust equivalent of the generated Zone service
//! boundary.  It consumes the already-authenticated subject capability and
//! never accepts a principal or Zone name from a request payload.

use std::{fmt, future::Future};

use d2b_contracts::v3::{CanonicalJsonObject, ResourceRef, ZoneId};

use crate::AuthenticatedSubjectContext;

/// Closed Zone service dispatch errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneServiceError {
    /// The request deadline or payload is invalid.
    InvalidRequest,
    /// The authenticated subject cannot call this Zone.
    AuthorizationDenied,
}

impl fmt::Display for ZoneServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "zone-service-request-invalid",
            Self::AuthorizationDenied => "zone-service-authorization-denied",
        })
    }
}

impl std::error::Error for ZoneServiceError {}

/// The closed v3 Zone method table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZoneMethod {
    /// Get one Resource.
    ResourceGet,
    /// List Resources.
    ResourceList,
    /// Watch Resource changes.
    ResourceWatch,
    /// Create one Resource.
    ResourceCreate,
    /// Update desired spec.
    ResourceUpdateSpec,
    /// Update observed status.
    ResourceUpdateStatus,
    /// Delete one Resource.
    ResourceDelete,
    /// Attach a ComponentSession stream.
    BusAttach,
}

impl ZoneMethod {
    /// Stable wire method name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ResourceGet => "resource-get",
            Self::ResourceList => "resource-list",
            Self::ResourceWatch => "resource-watch",
            Self::ResourceCreate => "resource-create",
            Self::ResourceUpdateSpec => "resource-update-spec",
            Self::ResourceUpdateStatus => "resource-update-status",
            Self::ResourceDelete => "resource-delete",
            Self::BusAttach => "bus-attach",
        }
    }

    /// Every method in stable order.
    pub const ALL: [Self; 8] = [
        Self::ResourceGet,
        Self::ResourceList,
        Self::ResourceWatch,
        Self::ResourceCreate,
        Self::ResourceUpdateSpec,
        Self::ResourceUpdateStatus,
        Self::ResourceDelete,
        Self::BusAttach,
    ];
}

/// Trusted call context derived from authenticated session evidence.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneCallContext {
    zone: ZoneId,
    principal: ResourceRef,
    deadline_ms: u64,
}

impl ZoneCallContext {
    /// Derive a context from the sealed authenticated subject capability.
    pub fn from_authenticated(
        subject: &AuthenticatedSubjectContext,
        deadline_ms: u64,
    ) -> Result<Self, ZoneServiceError> {
        if deadline_ms == 0 || deadline_ms > 900_000 {
            return Err(ZoneServiceError::InvalidRequest);
        }
        let claims = subject.claims();
        if claims.zone_ref().resource_type().as_str() != "Zone" {
            return Err(ZoneServiceError::AuthorizationDenied);
        }
        let zone = ZoneId::parse(claims.zone_ref().name().as_str())
            .map_err(|_| ZoneServiceError::AuthorizationDenied)?;
        Ok(Self {
            zone,
            principal: claims.subject_ref().clone(),
            deadline_ms,
        })
    }

    /// Borrow the authenticated Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the authenticated principal.
    pub const fn principal(&self) -> &ResourceRef {
        &self.principal
    }

    /// Return the bounded operation deadline.
    pub const fn deadline_ms(&self) -> u64 {
        self.deadline_ms
    }
}

impl fmt::Debug for ZoneCallContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ZoneCallContext(<redacted>)")
    }
}

/// Strict wire payload accepted by one Zone method.
pub trait StrictWireMessage: Sized {
    /// Decode a duplicate-rejecting canonical object.
    fn decode_strict(payload: &[u8]) -> Result<Self, ZoneServiceError>;
    /// Encode canonical bytes.
    fn encode_strict(&self) -> Result<Vec<u8>, ZoneServiceError>;
}

impl StrictWireMessage for CanonicalJsonObject {
    fn decode_strict(payload: &[u8]) -> Result<Self, ZoneServiceError> {
        Self::parse(payload).map_err(|_| ZoneServiceError::InvalidRequest)
    }

    fn encode_strict(&self) -> Result<Vec<u8>, ZoneServiceError> {
        Ok(self.to_canonical_bytes())
    }
}

/// Zone service implementation behind the dispatch adapter.
pub trait ZoneServiceHandler: Send + Sync {
    /// Dispatch one authenticated method.
    fn dispatch(
        &self,
        context: &ZoneCallContext,
        method: ZoneMethod,
        payload: CanonicalJsonObject,
    ) -> impl Future<Output = Result<CanonicalJsonObject, ZoneServiceError>> + Send;
}

/// Authenticated Zone service facade.
pub struct ZoneService<S> {
    handler: S,
}

impl<S> ZoneService<S> {
    /// Build a service over a handler.
    pub const fn new(handler: S) -> Self {
        Self { handler }
    }

    /// Borrow the handler.
    pub const fn handler(&self) -> &S {
        &self.handler
    }
}

impl<S> ZoneService<S>
where
    S: ZoneServiceHandler,
{
    /// Dispatch after strict payload decoding.
    pub async fn call(
        &self,
        context: &ZoneCallContext,
        method: ZoneMethod,
        payload: &[u8],
    ) -> Result<Vec<u8>, ZoneServiceError> {
        let payload = CanonicalJsonObject::decode_strict(payload)?;
        let response = self.handler.dispatch(context, method, payload).await?;
        response.encode_strict()
    }
}
