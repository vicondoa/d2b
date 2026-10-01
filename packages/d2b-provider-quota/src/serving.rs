//! The serving `Quota` driver (U40, R8).
//!
//! The declaration-only metadata driver converged a `Quota` row as an opaque
//! JSON object and enforced nothing. This module is the row's actual
//! conversion: the driver reads its own committed ceilings, counts the Zone's
//! committed rows, and publishes the resulting policy where the
//! manager-boundary admission reads it.
//!
//! # What the driver does and does not decide
//!
//! The driver never refuses a mutation. Admission is the manager boundary's
//! job, and the ceiling reaches it by being published rather than by being
//! checked here: a check inside a driver runs after the row is committed, so
//! refusing there would leave the row behind, which is the difference between
//! a limit and a report.
//!
//! # The census is counted, never remembered
//!
//! The Zone's usage is derived from the committed rows on every pass, so a row
//! that was deleted stops consuming ceiling and a row that was committed is
//! counted even before any driver of its own has run. The driver reads that
//! census from the per-Zone runtime the composition root installs, exactly as
//! every other family reads its daemon-owned state.

use d2b_contracts_resource::v3::ResourceRef;
use d2b_resource_runtime::context::{ResourceContext, SpecDecoder, typed_spec_decoder};
use d2b_resource_runtime::driver::{
    DynResourceDriver, RecoveryOutcome, ReconcileOutcome, ResourceDriver, ResourceDriverFactory,
};
use d2b_resource_runtime::error::{DriverFailure, DriverOp, FailureKinds};
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName as IdentityTypeName};

use crate::facets::ZoneQuotaRuntime;
use crate::quota::{QuotaError, QuotaPolicy, QuotaStatusResource, ZoneUsage};

/// The canonical `Quota` ResourceType name.
pub const QUOTA_RESOURCE_TYPE: &str = "Quota";

/// Why one committed `Quota` row was not read as ceilings.
///
/// Every variant is a refusal to decide, never a default. A ceiling this
/// decoder cannot read would leave the Zone without a limit, so the row is
/// refused rather than admitted with its authority dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaRowError {
    /// The committed desired state is not a `Quota` row.
    UnreadableDesiredState,
    /// The row is structurally readable but the ceilings it declares are not
    /// ones this contract admits.
    InvalidCeilings,
    /// The row belongs to another Zone, so this driver will not meter it.
    ForeignZone(String),
}

impl core::fmt::Display for QuotaRowError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnreadableDesiredState => formatter.write_str("quota-desired-state-unreadable"),
            Self::InvalidCeilings => formatter.write_str("quota-ceilings-invalid"),
            Self::ForeignZone(zone) => write!(formatter, "quota-foreign-zone: {zone}"),
        }
    }
}

impl std::error::Error for QuotaRowError {}

/// Read one committed `Quota` row's desired state as the Zone's limits.
///
/// The bytes are the canonical desired-state object the manager stored and
/// the decode is this crate's own, so the admission admits against the same
/// value the driver reconciles from rather than a second reading of it.
pub fn quota_policy_of_spec(desired_state: &[u8]) -> Result<QuotaPolicy, QuotaRowError> {
    QuotaPolicy::decode(desired_state).map_err(|error| match error {
        // A row whose shape this contract does not admit is a row whose
        // ceilings are not readable as ceilings, and a ceiling that could not
        // be read is a refusal rather than a default.
        QuotaError::UnreadableDesiredState
        | QuotaError::UnknownField
        | QuotaError::MissingField
        | QuotaError::InvalidField => QuotaRowError::UnreadableDesiredState,
        _ => QuotaRowError::InvalidCeilings,
    })
}

/// The manager-wired decode hook for the `Quota` type.
///
/// The stored spec envelope IS the ceiling row, so the decode is the policy
/// itself rather than the generic JSON object the metadata driver accepted.
pub fn quota_spec_decoder() -> std::sync::Arc<dyn SpecDecoder> {
    typed_spec_decoder(quota_policy_of_spec)
}

/// The `Quota` resource's driver (U40, R8).
///
/// The driver is built over its Zone's [`ZoneQuotaRuntime`], which the
/// composition root installs. A Zone with no installed runtime has no census
/// to publish, so the driver reports that rather than publishing a ceiling
/// measured against usage it cannot see.
pub struct QuotaDriver {
    zone: d2b_contracts_resource::v3::ZoneId,
    runtime: Option<std::sync::Arc<ZoneQuotaRuntime>>,
}

impl QuotaDriver {
    /// Build the driver for one Zone.
    pub fn new(
        zone: d2b_contracts_resource::v3::ZoneId,
        runtime: Option<std::sync::Arc<ZoneQuotaRuntime>>,
    ) -> Self {
        Self { zone, runtime }
    }

    /// The exact reference this row carries.
    ///
    /// The Zone is checked from the durable key rather than from the decoded
    /// spec: a row whose key names another Zone is refused, so a ceiling
    /// committed under the wrong Zone cannot meter this one.
    fn own_reference(&self, ctx: &ResourceContext) -> Result<ResourceRef, QuotaRowError> {
        let key = ctx.key();
        if key.zone != self.zone.as_str() {
            return Err(QuotaRowError::ForeignZone(key.zone.clone()));
        }
        ResourceRef::parse(&format!("{}/{}", key.type_name, key.name))
            .map_err(|_| QuotaRowError::UnreadableDesiredState)
    }

    /// The accepted policy this row states, bound to its own reference so it
    /// can never measure the row that carries it.
    fn policy(&self, ctx: &ResourceContext) -> Result<QuotaPolicy, QuotaRowError> {
        let reference = self.own_reference(ctx)?;
        let decoded: &QuotaPolicy = ctx
            .spec::<QuotaPolicy>()
            .map_err(|_| QuotaRowError::UnreadableDesiredState)?;
        Ok(decoded.clone().bind_to(reference))
    }
}

#[async_trait::async_trait]
impl ResourceDriver for QuotaDriver {
    type Error = QuotaRowError;

    fn classify_error(&self, error: &Self::Error) -> DriverFailure {
        // A row this crate cannot read as ceilings is terminal against the
        // row, not a transient read failure: retrying the same bytes decodes
        // the same way, and retrying forever would leave the Zone believing a
        // ceiling it never accepted is still in force.
        DriverFailure::refused_because(
            DriverOp::Validate,
            FailureKinds::CORE_SPEC_INVALID,
            error.to_string(),
        )
    }

    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        self.own_reference(ctx)?;
        // The ceilings are read here rather than only at reconcile so a row
        // that cannot state a ceiling never reaches the point where it would
        // publish one.
        let _: &QuotaPolicy = ctx
            .spec::<QuotaPolicy>()
            .map_err(|_| QuotaRowError::UnreadableDesiredState)?;
        Ok(())
    }

    async fn recover(&mut self, _ctx: &mut ResourceContext) -> Result<RecoveryOutcome, Self::Error> {
        // A ceiling row realizes nothing on a target, so there is no local
        // state to discover. The policy it states is the whole row, and
        // `reconcile` republishes it from the committed spec.
        Ok(RecoveryOutcome::Adopted)
    }

    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, Self::Error> {
        let policy = self.policy(ctx)?;
        // The census is counted from the committed rows on this pass, never
        // remembered from an earlier one: a row that was deleted must stop
        // consuming ceiling, and a row committed before this driver ran must
        // already be counted.
        let usage = match self.runtime.as_ref() {
            Some(runtime) => runtime.usage().await,
            // A Zone with no installed runtime has no committed rows this
            // driver can count, so it publishes nothing rather than a ceiling
            // measured against a usage of zero.
            None => return Ok(ReconcileOutcome::Satisfied),
        };
        // The row is only a limit once the manager-boundary admission can
        // read it, so publication is the driver's own effect rather than a
        // side effect of a row that merely converged.
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.publish(&Some(policy.clone()), &usage);
        }
        // The status is the contract's own shape, rendered from the same
        // policy and census that were published, so a reader and the
        // admission cannot see two different usages.
        ctx.set_status(QuotaStatusResource::of(&policy, &usage, None, 0, None));
        Ok(ReconcileOutcome::Satisfied)
    }

    async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), Self::Error> {
        // A removed ceiling row publishes no policy. That is exactly what lets
        // a Zone recover from an excess, so the teardown clears the
        // publication rather than leaving a ceiling in force over a row that
        // is gone.
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.publish(&None, &ZoneUsage::default());
        }
        Ok(())
    }
}

/// [`ResourceDriverFactory`] for the `Quota` type.
pub struct QuotaDriverFactory {
    zone: d2b_contracts_resource::v3::ZoneId,
}

impl QuotaDriverFactory {
    /// Build the factory for one Zone.
    ///
    /// The Zone's runtime is resolved per driver rather than captured here:
    /// the composition root installs it while it builds the plane, and one
    /// registered descriptor serves every Zone a process runs.
    pub fn new(zone: d2b_contracts_resource::v3::ZoneId) -> Self {
        Self { zone }
    }
}

#[async_trait::async_trait]
impl ResourceDriverFactory for QuotaDriverFactory {
    fn resource_types(&self) -> &[IdentityTypeName] {
        static TYPES: std::sync::LazyLock<Vec<IdentityTypeName>> =
            std::sync::LazyLock::new(|| vec![IdentityTypeName::new(QUOTA_RESOURCE_TYPE)]);
        &TYPES
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(QuotaDriver::new(self.zone.clone(), crate::facets::runtime(&self.zone)))
    }
}

/// The `Quota` type's driver declaration (U40, R8).
///
/// This is a serving declaration, not the declaration-only metadata one: the
/// type realizes no target-local state, but it does own the Zone's ceilings,
/// and the decoder and factory below are what carry that.
pub fn quota_descriptor(
    zone: d2b_contracts_resource::v3::ZoneId,
) -> d2b_resource_types::DriverDescriptor {
    use d2b_resource_types::{AllowedSources, CONVERTED_TYPE_VERBS, DriverDescriptor, WellKnownType};
    DriverDescriptor {
        resource_type: WellKnownType::QUOTA,
        allowed_sources: AllowedSources::BUILTIN,
        verbs: CONVERTED_TYPE_VERBS,
        // The type names no placement anchor, so the plane reconciles it on
        // its own Host domain, exactly as the declaration-only form did.
        execution: d2b_resource_runtime::metadata::METADATA_EXECUTION_DOMAINS,
        exportable: false,
        reads: &[],
        operations: &[],
        creations: &[],
        startup: &[],
        services: &[],
        decoder: quota_spec_decoder(),
        factory: std::sync::Arc::new(QuotaDriverFactory::new(zone)),
    }
}
