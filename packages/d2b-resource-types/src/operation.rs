//! Broker operation declarations and the handler contract they carry.

use std::os::fd::RawFd;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_contracts_resource::v3::{CanonicalJsonObject, ResourceRef, ZoneId};
use d2b_core::bundle_resolver::BundleResolver;

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
    /// The descriptors the caller attached to this invocation,when any.
    /// They belong to the transport's frame,not to the handler;the handler
    /// borrows them for the duration of the invocation only.
    pub fds: &'a [RawFd],
    /// The evidence chain this invocation runs under (U10, KTD6): the
    /// ordered identities, root first, of the chain the broker minted for
    /// the root call. A handler that invokes a broker-generic kernel as
    /// the nested core of its family operation presents this chain with
    /// its own identity appended, so the graft rule authorizes the kernel
    /// call against the chain's initiating principal and the in-broker leg
    /// records the correlation leg. Empty for a root call.
    pub chain_identities: &'a [String],
    /// The U10 family seam: the broker kernel socket, the caller role the
    /// family handler presents, the Zone's trusted bundle, and the
    /// daemon-side runner lookup the family handlers validate against.
    /// Absent when the composition point wired no seam (direct and test
    /// invocations).
    pub kernel: Option<&'a KernelCaller>,
}

/// The U10 family seam a forwarded family handler invokes kernels through.
///
/// The sandwich serves each process-family operation's privileged,
/// resource-agnostic kernel in-broker as a broker-generic committed row
/// while the family operation itself stays forwarded to the declaring
/// process. This caller carries everything the daemon-side family handler
/// needs to invoke the kernel as a nested envelope call: the broker's
/// origination socket, the caller role the daemon presents, the Zone's
/// trusted bundle the family logic resolves intents from, and the
/// daemon-side runner lookup the observation handlers validate against.
#[derive(Clone)]
pub struct KernelCaller {
    /// The broker's origination socket the kernel calls dial.
    pub socket_path: PathBuf,
    /// The caller role the daemon presents to the broker.
    pub caller_role: BrokerCallerRole,
    /// The Zone's trusted bundle the family handlers resolve runner
    /// intents and launch plans from.
    pub bundle: Arc<BundleResolver>,
    /// The daemon-side runner lookup: `(vm, role)` to the retained
    /// `(pid, start_time_ticks)`, when the composition point wired one.
    pub runner_lookup: Option<Arc<dyn RunnerLookup>>,
}

impl std::fmt::Debug for KernelCaller {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KernelCaller")
            .field("socket_path", &self.socket_path)
            .field("caller_role", &self.caller_role)
            .field("bundle", &"<redacted>")
            .field("runner_lookup", &self.runner_lookup.is_some())
            .finish()
    }
}

/// The daemon-side runner lookup the family observation handlers validate
/// against.
///
/// The daemon retains the authoritative `(pid, start_time_ticks)` for the
/// runners it tracks (the pidfd table); the family handler reads it through
/// this seam rather than reaching daemon internals.
pub trait RunnerLookup: Send + Sync {
    /// The retained `(pid, start_time_ticks)` for one `(vm, role)`, when
    /// the daemon tracks it.
    fn lookup(&self, vm: &str, role: &str) -> Option<(i32, u64)>;
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
#[derive(Debug)]
pub struct OperationResult {
    payload: CanonicalJsonObject,
    /// The descriptors the handler minted for this invocation, when the
    /// operation's result carries any (U10).
    ///
    /// The descriptors travel with the result over the forward carrier's
    /// fd leg; the caller owns them once the reply frame has gone. A
    /// handler that mints no descriptor leaves the vector empty.
    fds: Vec<std::os::fd::OwnedFd>,
}

impl OperationResult {
    /// Wrap a canonical result payload object.
    pub fn new(payload: CanonicalJsonObject) -> Self {
        Self {
            payload,
            fds: Vec::new(),
        }
    }

    /// Wrap a canonical result payload object plus the descriptors the
    /// handler minted for this invocation.
    pub fn with_fds(payload: CanonicalJsonObject, fds: Vec<std::os::fd::OwnedFd>) -> Self {
        Self { payload, fds }
    }

    /// Borrow the canonical result payload object.
    pub fn object(&self) -> &CanonicalJsonObject {
        &self.payload
    }

    /// Consume the wrapper, returning the canonical payload object.
    pub fn into_object(self) -> CanonicalJsonObject {
        self.payload
    }

    /// The descriptors the handler minted for this invocation, in frame
    /// order.
    pub fn fds(&self) -> &[std::os::fd::OwnedFd] {
        &self.fds
    }

    /// Consume the wrapper, returning the canonical payload object and the
    /// descriptors the handler minted.
    pub fn into_parts(self) -> (CanonicalJsonObject, Vec<std::os::fd::OwnedFd>) {
        (self.payload, self.fds)
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
