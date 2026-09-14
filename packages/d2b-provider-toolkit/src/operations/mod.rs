//! The operation-envelope surface: the invocation context, the validated
//! payload, the result and failure shapes, the handler contract, and the
//! envelope that runs them.
//!
//! The vocabulary is declared once in `d2b-resource-types`, so a handler has
//! exactly one signature and a descriptor exactly one operation shape. What
//! the toolkit adds here is the envelope itself: resolve the operation among
//! the declared handlers, refuse an uncommitted operation, refuse an
//! ungranted caller, audit the invocation, and only then run the handler.

pub use d2b_resource_types::{
    OperationCtx, OperationDef, OperationFailure, OperationHandler, OperationResult,
    ValidatedPayload,
};

mod envelope;

pub use envelope::{ENVELOPE_REFUSALS, OperationEnvelope, UNCOMMITTED_OPERATION, UNGRANTED_CALLER};
