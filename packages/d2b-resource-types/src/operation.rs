//! Broker operation declarations and the handler contract they carry.

use async_trait::async_trait;

use d2b_contracts_resource::v3::{CanonicalJsonObject, ResourceRef, ZoneId};

/// One committed broker operation and the handler that serves it.
///
/// The descriptor declares the operation reference and the handler code. It
/// never carries a grant: authorization comes from the committed role and
/// role-binding rows, and the envelope authorizes the caller before the
/// handler runs. A handler must not treat its own presence in a descriptor as
/// authority for anything.
///
/// The operation reference is an owned, validated value, so a declaring crate
/// assembles its descriptor tables once rather than declaring them as
/// `const`.
pub struct OperationDef {
    /// The committed operation row this handler serves.
    pub operation_ref: ResourceRef,
    /// The handler code that runs in the declaring crate's process.
    pub handler: &'static dyn OperationHandler,
}

/// The handler of one broker operation.
///
/// Implementations live in the declaring per-type crate; the registry holds
/// them behind the descriptor's `&'static dyn OperationHandler`.
#[async_trait]
pub trait OperationHandler: Send + Sync {
    /// Run one validated, authorized invocation.
    ///
    /// The payload was validated against the operation row's payload schema
    /// and the caller was authorized against the committed grants before this
    /// call. Grants are read from those rows, never from the descriptor.
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure>;
}

/// The envelope context of one operation invocation.
#[derive(Debug)]
pub struct OperationCtx<'a> {
    /// The zone the invocation runs in.
    pub zone: &'a ZoneId,
    /// The authenticated caller.
    pub caller: &'a ResourceRef,
    /// The committed operation row being invoked.
    pub operation: &'a ResourceRef,
    /// The invocation identifier the audit record carries.
    pub invocation_id: &'a str,
}

/// The canonical payload of one invocation, already validated against the
/// operation row's payload schema.
///
/// Both fields and rendering are canonical, so a handler compares and hashes
/// exactly what the envelope validated. `Debug` reports only the field count
/// and never payload values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedPayload(CanonicalJsonObject);

impl ValidatedPayload {
    /// Wrap an already validated canonical payload object.
    pub fn new(payload: CanonicalJsonObject) -> Self {
        Self(payload)
    }

    /// Borrow the canonical payload object.
    pub fn object(&self) -> &CanonicalJsonObject {
        &self.0
    }

    /// Consume the wrapper, returning the canonical payload object.
    pub fn into_object(self) -> CanonicalJsonObject {
        self.0
    }
}

/// The canonical result payload of one successful invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationResult(CanonicalJsonObject);

impl OperationResult {
    /// Wrap a canonical result payload object.
    pub fn new(payload: CanonicalJsonObject) -> Self {
        Self(payload)
    }

    /// Borrow the canonical result payload object.
    pub fn object(&self) -> &CanonicalJsonObject {
        &self.0
    }

    /// Consume the wrapper, returning the canonical result payload object.
    pub fn into_object(self) -> CanonicalJsonObject {
        self.0
    }
}

/// A refused invocation: a named code plus optional detail.
///
/// The code is the closed, operator-facing label for the refusal; the detail
/// is redacted text and must never carry payload bytes, secret material, or
/// host paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationFailure {
    code: &'static str,
    detail: Option<String>,
}

impl OperationFailure {
    /// Refuse the invocation with a named code and no detail.
    pub const fn new(code: &'static str) -> Self {
        Self {
            code,
            detail: None,
        }
    }

    /// Refuse the invocation with a named code and detail text.
    pub fn with_detail(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: Some(detail.into()),
        }
    }

    /// The named refusal code.
    pub const fn code(&self) -> &'static str {
        self.code
    }

    /// The optional detail text.
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}
