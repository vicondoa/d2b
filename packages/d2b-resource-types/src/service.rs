//! Zone/session service metadata a provider declares.

/// The fd carriage one method leg declares.
///
/// A leg with no carriage declares `max_fds: 0` and no `fd_kind`, which
/// admits only the empty leg; a leg that declares carriage names the ceiling
/// and the single kernel kind every attached descriptor must present (the
/// mirror of the committed row's fd facet, U1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodFdContract {
    /// The most descriptors this leg may carry. 0 when the leg declares no
    /// carriage.
    pub max_fds: u8,
    /// The kernel kind every descriptor on this leg must present, required
    /// when [`Self::max_fds`] is nonzero.
    pub fd_kind: Option<&'static str>,
}

impl MethodFdContract {
    /// The empty leg: no descriptor carriage.
    pub const NONE: Self = Self {
        max_fds: 0,
        fd_kind: None,
    };
}

/// One method a declared service answers, with its per-method contract
/// facets (R2/U7).
///
/// A method either serves the zone plane - the session layer addresses it
/// directly and no committed operation row stands behind it - or serves one
/// committed broker operation row, named by [`Self::operation`]. The
/// operation name is the row's committed spelling, so the declaration
/// surface carries the row → service+method resolution the broker envelope
/// performs on the committed rows: an operation resolves to the declaring
/// service exactly when a declared method names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceMethod {
    /// The method name the session layer addresses.
    pub name: &'static str,
    /// The committed broker operation row this method serves, when the
    /// method is the operation's service surface. Zone-plane methods leave
    /// it absent.
    pub operation: Option<&'static str>,
    /// The payload schema reference: the committed operation row's schema
    /// this method's invocations validate against. The broker envelope
    /// validates the payload bytes; the declaration carries the reference so
    /// the provider-side surface can state which row schema governs the
    /// method. Absent for methods whose rows carry no schema.
    pub payload_schema: Option<&'static str>,
    /// The fd contract of the request leg.
    pub request_fds: MethodFdContract,
    /// The fd contract of the response leg.
    pub response_fds: MethodFdContract,
    /// The declared state-cell names this method's broker-owned state lives
    /// on (U3/KTD4).
    pub state_cells: &'static [&'static str],
    /// The privileges this method requires: the grants an invocation must
    /// hold before the method runs.
    pub privileges: &'static [&'static str],
    /// The deadline tier (KTD4) this method serves under: the concrete
    /// budget the broker mints into the attested context block. `None` sits
    /// on the standard tier, mirroring a committed row with no declared
    /// tier.
    pub deadline_tier: Option<&'static str>,
}

impl ServiceMethod {
    /// A zone-plane method: addressed by the session layer, with no
    /// committed operation row behind it.
    pub const fn zone_plane(name: &'static str) -> Self {
        Self {
            name,
            operation: None,
            payload_schema: None,
            request_fds: MethodFdContract::NONE,
            response_fds: MethodFdContract::NONE,
            state_cells: &[],
            privileges: &[],
            deadline_tier: None,
        }
    }

    /// An operation-serving method: the service surface of one committed
    /// broker operation row, on the standard deadline tier.
    pub const fn serving(operation: &'static str, name: &'static str) -> Self {
        Self {
            name,
            operation: Some(operation),
            payload_schema: None,
            request_fds: MethodFdContract::NONE,
            response_fds: MethodFdContract::NONE,
            state_cells: &[],
            privileges: &[],
            deadline_tier: None,
        }
    }

    /// An operation-serving method carrying its full contract facets (R6):
    /// the payload schema reference, the request and response fd legs, the
    /// declared state cells, the required privileges, and the deadline
    /// tier.
    ///
    /// This is the declaration surface a family lane uses when its method
    /// reaches daemon-structural state, carries descriptors, or serves
    /// under a non-standard tier; the hosting side builds the service
    /// invocation's capability object from these facets (U3).
    #[allow(clippy::too_many_arguments, reason = "each argument is one declared facet (R6)")]
    pub const fn serving_with(
        operation: &'static str,
        name: &'static str,
        payload_schema: Option<&'static str>,
        request_fds: MethodFdContract,
        response_fds: MethodFdContract,
        state_cells: &'static [&'static str],
        privileges: &'static [&'static str],
        deadline_tier: Option<&'static str>,
    ) -> Self {
        Self {
            name,
            operation: Some(operation),
            payload_schema,
            request_fds,
            response_fds,
            state_cells,
            privileges,
            deadline_tier,
        }
    }
}

/// One service a provider serves.
///
/// The declaration is the service metadata source the session layer and the
/// CLI read: both resolve a service to its declaring driver from here instead
/// of keeping per-type service tables.
///
/// The declaration carries two planes. The zone-plane facet - attach kinds,
/// streams, and the endpoint policy - describes how sessions attach to the
/// service (R4); the per-method facets describe the contract each method
/// serves (R2/U7). Both coexist on one declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceDecl {
    /// The service identity the session layer addresses.
    pub id: &'static str,
    /// The methods the service answers, each with its contract facets.
    pub methods: &'static [ServiceMethod],
    /// The attachment kinds the service accepts.
    pub attach_kinds: &'static [&'static str],
    /// The stream kinds the service exposes.
    pub streams: &'static [&'static str],
    /// The endpoint policy the session layer enforces for the service, when
    /// the service declares one rather than the layer's default.
    pub endpoint_policy: Option<&'static str>,
}

impl ServiceDecl {
    /// Whether one declared method name is among the service's request
    /// methods.
    pub fn declares_method(&self, method: &str) -> bool {
        self.methods.iter().any(|declared| declared.name == method)
    }

    /// The declared method of one name, when the service answers it.
    pub fn method(&self, method: &str) -> Option<&ServiceMethod> {
        self.methods.iter().find(|declared| declared.name == method)
    }
}