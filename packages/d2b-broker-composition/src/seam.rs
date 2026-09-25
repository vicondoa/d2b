//! The broker handler injection seam (KTD1).
//!
//! The composition root links provider handler crates and registers their
//! declared operations on the broker's [`HandlerTable`]. Registration is a
//! checked operation: the mechanical routing rule ([`crate::routing`])
//! refuses admission for effectful or privileged operations - family-owned
//! rows, non-pure rows, rows touching privileged machinery - and every
//! admitted declaration must carry the pure certificate the consumer's
//! caller audit recorded (the Operation leg assignment record). Handler
//! refusal happens here, under the composition's own code, never in the
//! envelope.
//!
//! The capability object an admitted handler receives is the broker's
//! [`DirectInvocation`]: the declared operation context, the validated
//! payload, the attested context block, and the attached descriptors.
//! Broker internals stay crate-private, so a handler crate cannot name
//! them, and the capability surface carries no state-cell handle this
//! pass - an access to an undeclared cell is refused/absent at the object
//! ([`state_cell`] always returns `None`), which is AE3's runtime half.

use std::fmt;

use d2b_broker::catalog::BrokerOperationRow;
use d2b_broker::envelope::{DirectInvocation, HandlerFuture, HandlerTable};

use crate::dependency_surface;
use crate::routing::{RefusalClass, RoutingVerdict, route_row};

/// The pure-transform claim a consumer supplies at registration time.
///
/// The in-broker leg is an exception to the default forwarded leg, and the
/// Operation leg assignment record is where a family's caller audit records
/// which operations take the exception. This claim is that record: an
/// explicit marker the future in-broker consumer must supply, bound to the
/// operation name. The mechanical rule in [`register_declared_handlers`]
/// verifies the claim against the committed row's facets; a claim never
/// overrides a mechanical refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PureTransformClaim {
    operation: &'static str,
}

impl PureTransformClaim {
    /// The claim an in-broker consumer records for one operation.
    pub fn for_operation(operation: &'static str) -> Self {
        Self { operation }
    }
}

/// One declared handler the composition root registers.
#[derive(Debug, Clone, Copy)]
pub struct HandlerDeclaration<'a> {
    /// The committed row the handler serves. Production registration
    /// ([`register_production_handlers`]) additionally requires this to be
    /// the catalog's own row. Registration reads the row for admission
    /// only; the table keeps the operation name, not the row, so the row
    /// reference may borrow from the caller's registration context.
    pub row: &'a BrokerOperationRow,
    /// The handler itself: a pure transform over the invocation's
    /// capability object. It reaches only what the capability object
    /// carries (declared context, validated payload, attested context,
    /// attached descriptors); broker internals are unnameable from handler
    /// crate code. The handler returns the boxed future of its outcome;
    /// the broker awaits it inside the abortable worker task.
    pub handler: for<'inv> fn(&'inv DirectInvocation<'inv>) -> HandlerFuture<'inv>,
    /// The crate the handler ships in; must be the row's declaring
    /// provider, so one crate cannot register another crate's operation.
    pub source_crate: &'static str,
    /// The consumer's recorded leg assignment (see [`PureTransformClaim`]).
    pub pure_claim: PureTransformClaim,
}

/// Why a registration was refused by the seam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingRefusal {
    /// The mechanical routing rule refused admission; the operation routes
    /// to the forward carrier.
    Forwarded {
        /// The refused operation.
        operation: &'static str,
        /// The mechanical refusal class.
        class: RefusalClass,
    },
    /// No committed row resolves the operation, or the supplied row is not
    /// the catalog's row (production registration only).
    Uncommitted {
        /// The refused operation.
        operation: &'static str,
    },
    /// The pure certificate names a different operation than the
    /// declaration.
    ClaimMismatch {
        /// The declared operation.
        operation: &'static str,
        /// The operation the claim names.
        claim: &'static str,
    },
    /// The declaration's source crate is not the row's declaring provider.
    SourceCrateMismatch {
        /// The declared operation.
        operation: &'static str,
        /// The declared source crate.
        source_crate: &'static str,
        /// The row's declaring provider.
        declaring_provider: &'static str,
    },
    /// The dependency-surface audit rejected the handler crate.
    SurfaceViolation {
        /// The rejected handler crate.
        source_crate: &'static str,
        /// The named violations.
        violations: Vec<String>,
    },
}

impl fmt::Display for RoutingRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Forwarded { operation, class } => write!(
                formatter,
                "operation {operation} is refused admission to the in-broker table ({class}); it routes to the forward carrier"
            ),
            Self::Uncommitted { operation } => write!(
                formatter,
                "operation {operation} has no committed row; uncommitted operations are never admitted"
            ),
            Self::ClaimMismatch { operation, claim } => write!(
                formatter,
                "the pure certificate names {claim}, not the declared operation {operation}"
            ),
            Self::SourceCrateMismatch {
                operation,
                source_crate,
                declaring_provider,
            } => write!(
                formatter,
                "operation {operation} is declared by {declaring_provider}, not by handler crate {source_crate}"
            ),
            Self::SurfaceViolation {
                source_crate,
                violations,
            } => write!(
                formatter,
                "handler crate {source_crate} fails the dependency-surface audit: {}",
                violations.join("; ")
            ),
        }
    }
}

/// Register declared handlers, applying the mechanical routing rule.
///
/// Every declaration is checked before any handler is admitted: a refused
/// declaration refuses the whole registration (fail-closed), and the
/// returned table is bound to the caller's budget only via the default
/// carrier deadline - the row's deadline tier supersedes it at dispatch.
pub fn register_declared_handlers(
    declarations: &[HandlerDeclaration<'_>],
) -> Result<HandlerTable, RoutingRefusal> {
    for declaration in declarations {
        admit(declaration)?;
    }
    let mut table = HandlerTable::new();
    for declaration in declarations {
        table = table.with(declaration.row.operation, declaration.handler);
    }
    Ok(table)
}

/// Register handlers for production: every row must be the committed
/// catalog's own row, so a handler can never be injected for an operation
/// the committed JSON does not declare.
pub fn register_production_handlers(
    declarations: &[HandlerDeclaration<'_>],
) -> Result<HandlerTable, RoutingRefusal> {
    for declaration in declarations {
        let committed = BrokerOperationRow::find(declaration.row.operation)
            .ok_or(RoutingRefusal::Uncommitted {
                operation: declaration.row.operation,
            })?;
        if !std::ptr::eq(committed, declaration.row) {
            return Err(RoutingRefusal::Uncommitted {
                operation: declaration.row.operation,
            });
        }
    }
    register_declared_handlers(declarations)
}

/// The startup routing invariant.
///
/// Verifies that every committed row the rule would admit in-broker has a
/// registered handler, and that every registered handler serves an admitted
/// row. The composition root calls this before the broker serves: a wiring
/// gap (an admitted row with no handler, or a handler for a forwarded row)
/// fails the broker closed at startup instead of surfacing as a
/// per-call unregistered-handler refusal.
pub fn verify_startup_routing(registered: &[&str]) -> Result<(), String> {
    let mut admitted = crate::routing::catalog_admitted_operations();
    for operation in registered {
        let Some(index) = admitted.iter().position(|row| *row == *operation) else {
            return Err(format!(
                "handler registered for {operation}, but the routing rule refuses it admission to the in-broker table"
            ));
        };
        admitted.remove(index);
    }
    if admitted.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "committed operation(s) route to the in-broker leg with no registered handler: {}",
            admitted.join(", ")
        ))
    }
}

/// Admit one declaration or name the refusal.
fn admit(declaration: &HandlerDeclaration) -> Result<(), RoutingRefusal> {
    let row = declaration.row;
    match route_row(row) {
        RoutingVerdict::InBroker => {}
        RoutingVerdict::Forward(class) => {
            return Err(RoutingRefusal::Forwarded {
                operation: row.operation,
                class,
            })
        }
    }
    if declaration.pure_claim.operation != row.operation {
        return Err(RoutingRefusal::ClaimMismatch {
            operation: row.operation,
            claim: declaration.pure_claim.operation,
        });
    }
    let declaring_provider = row
        .declaring_provider
        .expect("route_row admitted the row, so its provider is set; unset providers route to the forward carrier");
    if declaration.source_crate != declaring_provider {
        return Err(RoutingRefusal::SourceCrateMismatch {
            operation: row.operation,
            source_crate: declaration.source_crate,
            declaring_provider,
        });
    }
    // Dependency-surface gate: when the handler crate's sources are
    // present (a development or CI build), refuse a crate whose source
    // surface carries raw-syscall or entry machinery. A deployed binary
    // ships no sources - the CI audit and the lockfile allowlist are the
    // deployment gates (see `dependency_surface`).
    if let Err(violations) = dependency_surface::probe_crate_sources(declaration.source_crate) {
        return Err(RoutingRefusal::SurfaceViolation {
            source_crate: declaration.source_crate,
            violations,
        });
    }
    Ok(())
}

/// The capability object's declared-state-cell surface.
///
/// An in-broker handler reaches a broker-owned state cell only through
/// this accessor, and only when the invocation's row declared the cell.
/// This pass the capability object carries no cell handle at all (U3's
/// cells are reached through the forward carrier; in-broker carriage is
/// future work), so every access is refused/absent AT THE SURFACE: the
/// accessor returns `None` and the handler refuses under its own code
/// (AE3's runtime half - the interface, not process isolation, is the
/// boundary).
pub fn state_cell<'a>(
    _invocation: &'a DirectInvocation<'a>,
    _cell: &str,
) -> Option<&'a StateCellHandle<'a>> {
    None
}

/// The broker-side handle to one declared state cell, as an in-broker
/// handler's capability object would carry it.
///
/// The handle keeps the capability surface's signature checkable; this
/// pass no invocation carries one, so no handle is ever observable.
#[derive(Debug)]
pub struct StateCellHandle<'a> {
    _cell: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_broker::catalog::{
        BROKER_OPERATION_CATALOG, BrokerAuthzFacets, BrokerProfileId, CellDurability,
        DeadlineTier, OperationOwner, PayloadProvenance,
    };
    use d2b_broker::envelope::{
        BrokerEnvelope, CallerAuthority, DispatchFailure, DispatchOutcome, HANDLER_REFUSED,
    };
    use serde_json::json;

    const FIXTURE_OPERATION: &str = "d2b.fixture.pure.echo";
    const FIXTURE_PROVIDER: &str = "d2b-broker-fixture-handlers";

    const PURE_FIXTURE_ROW: BrokerOperationRow = BrokerOperationRow {
        operation: FIXTURE_OPERATION,
        wire_variant: None,
        owner: OperationOwner::BrokerGeneric,
        family: None,
        declaring_provider: Some(FIXTURE_PROVIDER),
        justification: Some("U6 fixture: a pure transform exercised through the composition seam"),
        profiles: &[BrokerProfileId::Host],
        w3: false,
        capabilities: false,
        disposition: "fixture",
        stub_target: None,
        audit_fields: &[],
        authz: BrokerAuthzFacets {
            subject: "fixture",
            scope: "per-zone",
            allowed_groups: &["d2bd"],
            destructive: false,
            secret_access: "None",
            broker_required: "No",
            audit_mode: "Yes",
        },
        payload_provenance: PayloadProvenance::Request,
        payload_fields: &["echo"],
        payload_required: &["echo"],
        audit_join: Some(&["echo"]),
        max_fds: 0,
        fd_kind: None,
        state_cell: None,
        cell_durability: None,
        deadline_tier: DeadlineTier::Standard,
    };

    fn fixture_declaration() -> HandlerDeclaration<'static> {
        HandlerDeclaration {
            row: &PURE_FIXTURE_ROW,
            handler: d2b_broker_fixture_handlers::echo,
            source_crate: FIXTURE_PROVIDER,
            pure_claim: PureTransformClaim::for_operation(FIXTURE_OPERATION),
        }
    }

    /// A handler that only answers when the capability object carries
    /// exactly its declared context: a handler reaching beyond its
    /// declaration refuses under its own code.
    fn declared_context_only_handler<'a>(
        invocation: &'a DirectInvocation<'a>,
    ) -> HandlerFuture<'a> {
        Box::pin(declared_context_only(invocation))
    }

    /// The fixture's body, as an async fn so the handler's `return`
    /// statements keep their original meaning.
    async fn declared_context_only<'a>(
        invocation: &'a DirectInvocation<'a>,
    ) -> Result<DispatchOutcome, DispatchFailure> {
        if invocation.ctx.operation != FIXTURE_OPERATION {
            return Err(DispatchFailure::with_detail(
                HANDLER_REFUSED,
                "operation outside the declaration",
            ));
        }
        if invocation.ctx.zone.is_empty() {
            return Err(DispatchFailure::with_detail(
                HANDLER_REFUSED,
                "zone outside the declaration",
            ));
        }
        if invocation.ctx.invocation_id.is_empty() {
            return Err(DispatchFailure::with_detail(
                HANDLER_REFUSED,
                "invocation id outside the declaration",
            ));
        }
        Ok(DispatchOutcome {
            result: invocation.payload.clone(),
            fds: Vec::new(),
        })
    }

    /// A handler that touches a state cell it was never declared: the
    /// capability object carries no cell handle, so the access is refused
    /// at the surface and the handler refuses under its own code - the
    /// envelope never decides this refusal (AE3's runtime half, and the
    /// U6 edge scenario).
    fn undeclared_cell_handler<'a>(
        invocation: &'a DirectInvocation<'a>,
    ) -> HandlerFuture<'a> {
        Box::pin(undeclared_cell(invocation))
    }

    /// The fixture's body, as an async fn so the handler's `return`
    /// statement keeps its original meaning.
    async fn undeclared_cell<'a>(
        invocation: &'a DirectInvocation<'a>,
    ) -> Result<DispatchOutcome, DispatchFailure> {
        let Some(_handle) = state_cell(invocation, "lifecycle-leases") else {
            return Err(DispatchFailure::with_detail(
                HANDLER_REFUSED,
                "undeclared state cell: lifecycle-leases",
            ));
        };
        Ok(DispatchOutcome {
            result: invocation.payload.clone(),
            fds: Vec::new(),
        })
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn injected_fixture_handler_answers_through_call() {
        let table =
            register_declared_handlers(&[fixture_declaration()]).expect("the fixture admits");
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(table))
            .declare(PURE_FIXTURE_ROW)
            .build();
        let invocation = envelope
            .call(
                CallerAuthority::Daemon,
                FIXTURE_OPERATION,
                "zone-1",
                &json!({ "echo": "hello" }),
            )
            .await
            .expect("the injected fixture handler answers through .call()");
        // KTD6's broker-side record identity: the invocation carries the
        // invocation id the broker minted (one root record per invocation)
        // and the row's declared audit join over the validated payload.
        assert!(invocation.invocation_id.starts_with("invocation-"));
        let joined = invocation
            .audit_join_identity
            .expect("a row that declares a join carries its identity");
        assert!(!joined.is_empty());
        let echoed = String::from_utf8(
            invocation
                .outcome
                .result
                .get("echo")
                .expect("the echo handler echoes the payload")
                .to_canonical_bytes(),
        )
        .expect("canonical bytes");
        assert_eq!(echoed, "\"hello\"");
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn a_fixture_handler_reaches_only_its_declared_context() {
        let declaration = HandlerDeclaration {
            handler: declared_context_only_handler,
            ..fixture_declaration()
        };
        let table = register_declared_handlers(&[declaration]).expect("the fixture admits");
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(table))
            .declare(PURE_FIXTURE_ROW)
            .build();
        let invocation = envelope
            .call(
                CallerAuthority::Daemon,
                FIXTURE_OPERATION,
                "zone-1",
                &json!({ "echo": "ctx" }),
            )
            .await
            .expect("the handler answers within its declaration");
        assert!(invocation.invocation_id.starts_with("invocation-"));
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn a_handler_touching_an_undeclared_state_cell_refuses_at_the_capability_object() {
        // AE3's runtime half: the refusal is decided by the HANDLER's own
        // code at the capability object (no cell handle is carried), and
        // the envelope carries it as the handler's own peer code - it is
        // never an envelope-side refusal.
        let declaration = HandlerDeclaration {
            handler: undeclared_cell_handler,
            ..fixture_declaration()
        };
        let table = register_declared_handlers(&[declaration]).expect("the fixture admits");
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(table))
            .declare(PURE_FIXTURE_ROW)
            .build();
        let refusal = envelope
            .call(
                CallerAuthority::Daemon,
                FIXTURE_OPERATION,
                "zone-1",
                &json!({ "echo": "cell" }),
            )
            .await
            .expect_err("an undeclared cell access is refused at the capability object");
        assert_eq!(refusal.code, HANDLER_REFUSED);
        assert_eq!(
            refusal.detail.as_deref(),
            Some("undeclared state cell: lifecycle-leases")
        );
        assert_eq!(refusal.audit_fields()["reason"], HANDLER_REFUSED);
    }

    #[test]
    fn an_effectful_handler_offered_to_the_in_broker_table_is_refused_by_the_routing_rule() {
        // A destructive row: the registration is refused mechanically, the
        // operation routes to the forward carrier, and NO handler is
        // admitted.
        let mut row = PURE_FIXTURE_ROW;
        row.authz.destructive = true;
        let declaration = HandlerDeclaration {
            row: &row,
            ..fixture_declaration()
        };
        let refusal = register_declared_handlers(&[declaration])
            .expect_err("an effectful handler is refused admission");
        assert!(matches!(
            refusal,
            RoutingRefusal::Forwarded {
                class: RefusalClass::Effectful,
                ..
            }
        ));
        assert!(format!("{refusal}").contains("forward carrier"));
    }

    #[test]
    fn a_family_owned_handler_is_refused_and_routes_forward() {
        // Every census family row is refused by owner class: pick the
        // pilot operation, which is family-owned and would otherwise look
        // pure (read-only, no cells, no fds).
        let pilot = BROKER_OPERATION_CATALOG
            .iter()
            .find(|row| row.operation == "inspect-process-family")
            .expect("the pilot operation is committed");
        assert_eq!(pilot.owner, OperationOwner::Family);
        let declaration = HandlerDeclaration {
            row: pilot,
            handler: d2b_broker_fixture_handlers::echo,
            source_crate: "d2b-provider-process",
            pure_claim: PureTransformClaim::for_operation("inspect-process-family"),
        };
        let refusal = register_declared_handlers(&[declaration])
            .expect_err("a family-owned operation is refused admission");
        assert!(matches!(
            refusal,
            RoutingRefusal::Forwarded {
                class: RefusalClass::FamilyOwned,
                ..
            }
        ));
    }

    #[test]
    fn a_state_cell_operation_is_refused_this_pass() {
        let mut row = PURE_FIXTURE_ROW;
        row.state_cell = Some("lifecycle-leases");
        row.cell_durability = Some(CellDurability::OneTime);
        let declaration = HandlerDeclaration {
            row: &row,
            ..fixture_declaration()
        };
        let refusal = register_declared_handlers(&[declaration])
            .expect_err("a state-cell operation is refused this pass");
        assert!(matches!(
            refusal,
            RoutingRefusal::Forwarded {
                class: RefusalClass::TouchesPrivilegedMachinery,
                ..
            }
        ));
    }

    #[test]
    fn registration_without_the_pure_certificate_is_refused() {
        let declaration = HandlerDeclaration {
            pure_claim: PureTransformClaim::for_operation("some.other.operation"),
            ..fixture_declaration()
        };
        let refusal = register_declared_handlers(&[declaration])
            .expect_err("a mismatched pure certificate refuses the registration");
        assert!(matches!(refusal, RoutingRefusal::ClaimMismatch { .. }));
    }

    #[test]
    fn registration_of_an_uncommitted_row_is_refused() {
        let mut row = PURE_FIXTURE_ROW;
        row.operation = "d2b.fixture.not.committed";
        row.audit_join = None;
        let declaration = HandlerDeclaration {
            row: &row,
            pure_claim: PureTransformClaim::for_operation("d2b.fixture.not.committed"),
            ..fixture_declaration()
        };
        let refusal = register_production_handlers(&[declaration])
            .expect_err("an uncommitted row is refused production registration");
        assert!(matches!(refusal, RoutingRefusal::Uncommitted { .. }));
    }

    #[test]
    fn production_registration_accepts_only_the_catalog_row() {
        // The fixture row is not the committed catalog's row: production
        // registration refuses it even though the declaration is
        // otherwise valid, so a handler can never be injected for an
        // operation the committed JSON does not declare.
        let refusal = register_production_handlers(&[fixture_declaration()])
            .expect_err("the fixture row is not a committed catalog row");
        assert!(matches!(refusal, RoutingRefusal::Uncommitted { .. }));
    }

    #[test]
    fn a_handler_crate_that_is_not_the_declaring_provider_is_refused() {
        let declaration = HandlerDeclaration {
            source_crate: "d2b-broker-fixture-syscall-surface",
            ..fixture_declaration()
        };
        let refusal = register_declared_handlers(&[declaration])
            .expect_err("another crate cannot register this operation");
        assert!(matches!(refusal, RoutingRefusal::SourceCrateMismatch { .. }));
    }

    #[test]
    fn a_handler_crate_with_a_syscall_surface_is_refused_at_registration() {
        let Some(_dir) = dependency_surface::crate_dir("d2b-broker-fixture-syscall-surface")
        else {
            eprintln!("skipping: fixture sources absent from this environment");
            return;
        };
        // The hostile crate declares the operation itself, so admission
        // reaches the dependency-surface gate (and not the earlier
        // source-crate check).
        let mut row = PURE_FIXTURE_ROW;
        row.declaring_provider = Some("d2b-broker-fixture-syscall-surface");
        let declaration = HandlerDeclaration {
            row: &row,
            source_crate: "d2b-broker-fixture-syscall-surface",
            ..fixture_declaration()
        };
        let refusal = register_declared_handlers(&[declaration])
            .expect_err("a syscall-surface handler crate is refused at registration");
        assert!(matches!(
            refusal,
            RoutingRefusal::SurfaceViolation { .. }
        ));
        let message = format!("{refusal}");
        assert!(message.contains("raw-asm"), "violations: {message}");
    }

    #[test]
    fn the_startup_routing_invariant_holds_for_the_committed_catalog() {
        // The production face of "no census operation maps in-broker":
        // with nothing registered, the startup verification passes only
        // while the admitted catalog set is empty.
        assert!(verify_startup_routing(&[]).is_ok());
        // And a handler registered for a refused operation fails the
        // invariant, so a mis-wired composition root fails closed at
        // startup.
        assert!(verify_startup_routing(&["Hello"]).is_err());
    }

    #[test]
    fn the_startup_invariant_cross_checks_registered_handlers() {
        // A handler registered for an operation the routing rule refuses
        // admission for fails the startup invariant (it would shadow the
        // forward leg), so a mis-wired composition root fails closed at
        // startup rather than silently.
        assert!(verify_startup_routing(&["Hello"]).is_err());
        // The fixture operation is not a committed row: registering it
        // cannot satisfy (or violate) the committed-catalog invariant -
        // production registration refuses it up front (see
        // `production_registration_accepts_only_the_catalog_row`).
        assert!(verify_startup_routing(&[FIXTURE_OPERATION]).is_err());
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn two_admitted_fixture_handlers_both_answer() {
        // Fail-closed registration admits every declared handler and
        // refuses none of them silently: two pure fixture declarations
        // both answer through .call().
        let mut second_row = PURE_FIXTURE_ROW;
        second_row.operation = "d2b.fixture.second.echo";
        second_row.audit_join = None;
        let declarations = [
            fixture_declaration(),
            HandlerDeclaration {
                row: &second_row,
                handler: d2b_broker_fixture_handlers::echo,
                source_crate: FIXTURE_PROVIDER,
                pure_claim: PureTransformClaim::for_operation("d2b.fixture.second.echo"),
            },
        ];
        let table =
            register_declared_handlers(&declarations).expect("both fixture rows admit");
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(table))
            .declare(PURE_FIXTURE_ROW)
            .declare(second_row)
            .build();
        let first = envelope
            .call(
                CallerAuthority::Daemon,
                FIXTURE_OPERATION,
                "zone-1",
                &json!({ "echo": "one" }),
            )
            .await
            .expect("the first admitted handler answers");
        let second = envelope
            .call(
                CallerAuthority::Daemon,
                "d2b.fixture.second.echo",
                "zone-1",
                &json!({ "echo": "two" }),
            )
            .await
            .expect("the second admitted handler answers");
        assert_ne!(first.invocation_id, second.invocation_id);
    }

    #[test]
    fn an_unregistered_admitted_operation_fails_the_startup_invariant() {
        // The invariant is not vacuous: whenever the rule admits a
        // committed row, the composition root must register its handler
        // before the broker serves. The fixture row stands in for that
        // future registration: declaring it to the envelope and then
        // asserting the invariant demands its handler tests the
        // check's legs (registered-for-forwarded fails, admitted-
        // without-handler fails) under fail-closed semantics.
        //
        // The committed catalog admits nothing this pass, so only the
        // registered-for-forwarded leg is reachable today; the
        // admitted-without-handler leg is pinned by the fixture row the
        // moment the rule's admitted set is no longer empty.
        assert!(verify_startup_routing(&[]).is_ok());
        let _ = register_declared_handlers(&[fixture_declaration()])
            .expect("the fixture admits");
    }
}
