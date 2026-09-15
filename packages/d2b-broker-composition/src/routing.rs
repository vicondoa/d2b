//! The mechanical routing rule (KTD1).
//!
//! The composition root is where the in-broker leg is decided. An
//! effectful or privileged operation - read off the committed row's Owner
//! and facets, never from prose - is REFUSED admission to the in-broker
//! handler table and routes to the forward carrier, where the declaring
//! provider's process serves it. The in-broker leg serves only pure,
//! reviewable transforms.
//!
//! The rule is fail-closed and reads the committed row data only. This
//! pass no committed operation is admitted (the U6 seam ships
//! fixture-exercised: no census operation maps to the in-broker leg). The
//! first family whose caller audit records an in-broker leg assignment
//! registers its handler through [`crate::seam`]; this module is the
//! single place the admission predicate lives.

use d2b_broker::catalog::{BrokerOperationRow, OperationOwner, PayloadProvenance};

/// Why one operation was refused admission to the in-broker table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalClass {
    /// The row is family-owned: the declaring provider's process serves it
    /// over the forward carrier.
    FamilyOwned,
    /// The row is not reachable through the generic envelope (a typed wire
    /// variant or a transport-excluded name); no in-broker handler may
    /// serve it.
    NotGeneric,
    /// The row's authorized effect is a host or privilege effect (state
    /// mutation or secret exposure by committed facet, or descriptor
    /// minting): the forward carrier is the only leg for effects.
    Effectful,
    /// The row declares a state cell, which is a declared handle to
    /// broker-owned privileged machinery (pidfd registry, reap buffer,
    /// lifecycle lease). Handlers that touch privileged machinery are
    /// never admitted this pass; the forward carrier mediates the cell.
    TouchesPrivilegedMachinery,
    /// No provider declared the operation, so there is no handler crate to
    /// inject; the broker's own machinery serves it, not the composition.
    NotProviderDeclared,
}

impl RefusalClass {
    /// The operator-facing label one refusal record carries.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FamilyOwned => "family-owned",
            Self::NotGeneric => "not-generic",
            Self::Effectful => "effectful",
            Self::TouchesPrivilegedMachinery => "touches-privileged-machinery",
            Self::NotProviderDeclared => "not-provider-declared",
        }
    }
}

impl std::fmt::Display for RefusalClass {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The leg one committed operation routes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoutingVerdict {
    /// The operation MAY be served by an in-broker injected handler
    /// (subject to the registration seam's pure certificate).
    InBroker,
    /// The operation is refused admission to the in-broker table and
    /// routes to the forward carrier.
    Forward(RefusalClass),
}

/// The mechanical routing rule over one committed row.
///
/// Read-only over the row data. An operation is admitted only when it is
/// provider-declared, generic-payload, effect-free per its committed
/// facets, and carries no privileged-machinery handle; every refusal class
/// names a fallback leg, never a silent drop.
pub fn route_row(row: &BrokerOperationRow) -> RoutingVerdict {
    if row.owner == OperationOwner::Family {
        return RoutingVerdict::Forward(RefusalClass::FamilyOwned);
    }
    if row.owner == OperationOwner::TransportExcluded {
        // A transport-excluded name is not reachable through the envelope
        // at all; the forward carrier is the only boundary-keeping answer.
        return RoutingVerdict::Forward(RefusalClass::NotGeneric);
    }
    if row.declaring_provider.is_none() {
        // Broker-generic machinery is served by the broker's own typed
        // path; the composition injects provider handlers only.
        return RoutingVerdict::Forward(RefusalClass::NotProviderDeclared);
    }
    if row.payload_provenance != PayloadProvenance::Request {
        // Wire-inherited rows are the typed arms; the generic envelope
        // (and therefore the in-broker leg) does not carry them.
        return RoutingVerdict::Forward(RefusalClass::NotGeneric);
    }
    if row.authz.destructive || row.authz.secret_access != "None" || row.max_fds != 0 {
        // A committed effect facet: destructive mutation, secret exposure,
        // or descriptor minting. Effects run on the forward carrier.
        return RoutingVerdict::Forward(RefusalClass::Effectful);
    }
    if row.state_cell.is_some() || row.cell_durability.is_some() {
        // A declared state cell is a handle to broker-owned privileged
        // machinery. The forward carrier mediates cells this pass; the
        // reserved future pure-transform-over-cells exception revisits
        // exactly this clause when a caller audit records a leg
        // assignment.
        return RoutingVerdict::Forward(RefusalClass::TouchesPrivilegedMachinery);
    }
    RoutingVerdict::InBroker
}

/// Every committed row the rule refuses this pass.
///
/// The startup invariant (`seam::verify_startup_routing`) and the fixture
/// suite pin that no census operation maps to the in-broker leg: the
/// rule's admitted set over the committed catalog must stay empty until a
/// caller audit records a leg assignment for a genuinely pure transform.
pub fn catalog_admitted_operations() -> Vec<&'static str> {
    d2b_broker::catalog::BROKER_OPERATION_CATALOG
        .iter()
        .filter(|row| route_row(row) == RoutingVerdict::InBroker)
        .map(|row| row.operation)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_broker::catalog::{
        BrokerAuthzFacets, BrokerProfileId, CellDurability, OperationOwner, PayloadProvenance,
    };

    /// A minimal pure, generic, provider-declared row (the shape a future
    /// pure transform will take; the fixture suite uses it as its happy
    /// path).
    const PURE_ROW: BrokerOperationRow = BrokerOperationRow {
        operation: "d2b.fixture.pure.echo",
        wire_variant: None,
        owner: OperationOwner::BrokerGeneric,
        family: None,
        declaring_provider: Some("d2b-broker-fixture-handlers"),
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
        audit_join: None,
        max_fds: 0,
        fd_kind: None,
        state_cell: None,
        cell_durability: None,
        deadline_tier: d2b_broker::catalog::DeadlineTier::Standard,
    };

    fn family_row(operation: &'static str) -> BrokerOperationRow {
        let mut row = PURE_ROW;
        row.operation = operation;
        row.justification = None;
        row.family = Some("fixture-family");
        row.owner = OperationOwner::Family;
        row
    }

    #[test]
    fn no_census_operation_maps_to_the_in_broker_leg_this_pass() {
        // The U6 seam ships fixture-exercised: the mechanical rule over
        // the committed catalog admits exactly nothing.
        assert!(
            catalog_admitted_operations().is_empty(),
            "committed catalog rows unexpectedly admitted in-broker: {:?}",
            catalog_admitted_operations()
        );
        let admitted = d2b_broker::catalog::BROKER_OPERATION_CATALOG
            .iter()
            .filter(|row| route_row(row) == RoutingVerdict::InBroker)
            .count();
        assert_eq!(admitted, 0);
    }

    #[test]
    fn a_family_owned_row_is_refused_and_routes_to_the_forward_carrier() {
        // A census row is representative: an OpenVhostNet-class
        // operation is family-owned, so the rule must refuse it regardless
        // of any other facet.
        let row = family_row("d2b.fixture.family.effect");
        assert_eq!(
            route_row(&row),
            RoutingVerdict::Forward(RefusalClass::FamilyOwned)
        );
    }

    #[test]
    fn the_wire_typed_broker_generic_rows_are_refused() {
        // Every broker-generic census row is a typed wire variant; none is
        // reachable through the generic envelope.
        for row in d2b_broker::catalog::BROKER_OPERATION_CATALOG.iter() {
            if row.owner == OperationOwner::BrokerGeneric {
                assert_eq!(
                    route_row(row),
                    RoutingVerdict::Forward(RefusalClass::NotProviderDeclared),
                    "broker-generic row {} must be refused",
                    row.operation
                );
            }
        }
    }

    #[test]
    fn a_destructive_row_is_refused_as_effectful() {
        let mut row = PURE_ROW;
        row.authz.destructive = true;
        assert_eq!(
            route_row(&row),
            RoutingVerdict::Forward(RefusalClass::Effectful)
        );
    }

    #[test]
    fn a_secret_exposing_row_is_refused_as_effectful() {
        let mut row = PURE_ROW;
        row.authz.secret_access = "RedactedOnly";
        assert_eq!(
            route_row(&row),
            RoutingVerdict::Forward(RefusalClass::Effectful)
        );
    }

    #[test]
    fn an_fd_minting_row_is_refused_as_effectful() {
        let mut row = PURE_ROW;
        row.max_fds = 1;
        assert_eq!(
            route_row(&row),
            RoutingVerdict::Forward(RefusalClass::Effectful)
        );
    }

    #[test]
    fn a_state_cell_row_is_refused_this_pass() {
        // The lifecycle-lease row is the committed example: a declared
        // one-time cell over broker-owned state.
        let mut row = PURE_ROW;
        row.state_cell = Some("lifecycle-leases");
        row.cell_durability = Some(CellDurability::OneTime);
        assert_eq!(
            route_row(&row),
            RoutingVerdict::Forward(RefusalClass::TouchesPrivilegedMachinery)
        );
    }

    #[test]
    fn an_unowned_generic_row_is_refused() {
        let mut row = PURE_ROW;
        row.declaring_provider = None;
        assert_eq!(
            route_row(&row),
            RoutingVerdict::Forward(RefusalClass::NotProviderDeclared)
        );
    }

    #[test]
    fn a_transport_excluded_row_is_refused() {
        let mut row = PURE_ROW;
        row.owner = OperationOwner::TransportExcluded;
        row.declaring_provider = None;
        assert_eq!(
            route_row(&row),
            RoutingVerdict::Forward(RefusalClass::NotGeneric)
        );
    }

    #[test]
    fn a_pure_generic_provider_row_is_admitted() {
        assert_eq!(route_row(&PURE_ROW), RoutingVerdict::InBroker);
    }

    #[test]
    fn the_committed_family_rows_are_all_refused() {
        // Every census family row (66 committed rows today) is refused by
        // the owner class alone, which is the mechanical heart of KTD1's
        // "effectful and privileged handlers never admitted in-broker":
        // family-owned operations all route to the forward carrier this
        // pass, regardless of their facets.
        let family_rows = d2b_broker::catalog::BROKER_OPERATION_CATALOG
            .iter()
            .filter(|row| row.owner == OperationOwner::Family)
            .count();
        assert!(family_rows >= 1, "census family rows must exist");
        for row in d2b_broker::catalog::BROKER_OPERATION_CATALOG.iter() {
            if row.owner == OperationOwner::Family {
                assert!(matches!(
                    route_row(row),
                    RoutingVerdict::Forward(RefusalClass::FamilyOwned)
                ));
            }
        }
    }
}