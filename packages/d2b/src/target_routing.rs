//! CLI target routing (ADR 0043 access-contract transition).
//!
//! `d2b vm <verb> <target>` accepts either a **local** VM name (the v1 fast
//! path, preserved exactly) or a realm-native target in the
//! `<workload>.<realm-path>.d2b` form. This module builds the
//! Wave-5 `RealmAccessResolver*` contract shape, then maps it back to the
//! current local/gateway CLI dispatch paths until the daemon-side access API
//! exists.
//!
//! Invariants:
//! - A bare workload name with no configured default realm or alias table routes
//!   **local** — the existing host-daemon fast path is never disturbed.
//! - An unknown realm fails **closed** (`NoRealmEntrypoint`); resolution never
//!   silently defaults an unconfigured realm to local dispatch.
//! - Old node-qualified targets are surfaced as typed migration diagnostics when
//!   the caller supplies the legacy node labels it knows about; they are never
//!   accepted as canonical realm targets through this resolver.

use d2b_realm_core::{
    AccessBindingRef, Capability, CapabilityPreflightStatus, CapabilitySet, ControllerGenerationId,
    DefaultRealmSelectionMetadata, DefaultRealmSelectionSource, HostLocalPeerCredentialSemantics,
    NodeId, ProtocolToken, RealmAccessAliasBinding, RealmAccessAliasSource, RealmAccessBinding,
    RealmAccessCapabilityPreflight, RealmAccessClientBinding, RealmAccessClientBindingKind,
    RealmAccessClientContract, RealmAccessConflictCandidate, RealmAccessResolverDiagnostic,
    RealmAccessResolverError, RealmAccessResolverRequest, RealmAccessResolverResponse,
    RealmAccessTargetInput, RealmControllerPlacement, RealmId, RealmPath, RealmTarget,
    RealmTargetParseError, RealmTargetParser, RealmTransportBinding, TargetName, UnixSocketPath,
    WorkloadId,
};
use d2b_realm_router::{DispatchTarget, RealmEntrypointTable, ResolveError};

const DEFAULT_PUBLIC_SOCKET_PATH: &str = "/run/d2b/public.sock";
const CLI_ROUTING_GENERATION: &str = "cli-routing-contract-v1";
const GATEWAY_TRANSPORT_TOKEN: &str = "realm-gateway-v1";
const GATEWAY_BINDING_REF: &str = "local-gateway-convention";

/// The routing decision for a `vm start/exec <target>` argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Dispatch through the existing host-daemon local fast path. Carries the
    /// workload name the local path expects.
    Local { vm: String },
    /// A realm target fronted by a gateway guest. The host cannot dispatch into
    /// the realm; the realm's gateway-mode `d2bd` owns it.
    Gateway { gateway: String, target: String },
}

/// Why a target could not be routed (fail-closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    /// Access-contract resolver diagnostics that are safe to return to a CLI
    /// caller and audit.
    AccessResolver {
        error: Box<RealmAccessResolverError>,
    },
    /// A syntactically realm-shaped target was malformed.
    InvalidTarget { target: String, reason: String },
    /// `target` is a realm target but no entrypoint is configured for its realm
    /// on this daemon. Never defaults to local.
    NoRealmEntrypoint { target: String, realm: String },
    /// A gateway-backed realm whose table entry is missing its gateway target
    /// (malformed table).
    MissingGateway { target: String, realm: String },
    /// The conventional local gateway VM name for a realm cannot be represented
    /// as a target address.
    InvalidGatewayTarget {
        realm: String,
        gateway: String,
        reason: String,
    },
}

impl core::fmt::Display for RouteError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RouteError::AccessResolver { error } => write_resolver_error(f, error),
            RouteError::InvalidTarget { target, reason } => {
                write!(f, "target `{target}` is not a valid realm target: {reason}")
            }
            RouteError::NoRealmEntrypoint { target, realm } => write!(
                f,
                "target `{target}` is in realm `{realm}`, which has no entrypoint on this daemon; \
                 a gateway-backed realm is dispatched by the realm gateway's d2bd, not the host \
                 daemon (the host holds no realm configuration)"
            ),
            RouteError::MissingGateway { target, realm } => write!(
                f,
                "target `{target}` is in gateway-backed realm `{realm}`, but its entrypoint is \
                 missing a gateway address (malformed realm entrypoint table)"
            ),
            RouteError::InvalidGatewayTarget {
                realm,
                gateway,
                reason,
            } => write!(
                f,
                "realm `{realm}` maps to gateway VM `{gateway}`, but that gateway target is \
                 invalid: {reason}"
            ),
        }
    }
}

fn write_resolver_error(
    f: &mut core::fmt::Formatter<'_>,
    error: &RealmAccessResolverError,
) -> core::fmt::Result {
    match &error.diagnostic {
        RealmAccessResolverDiagnostic::AliasAmbiguous { alias, candidates } => {
            let rendered = candidates
                .iter()
                .map(|candidate| candidate.target.to_canonical())
                .collect::<Vec<_>>()
                .join(", ");
            write!(
                f,
                "target alias `{alias}` is ambiguous; use one of: {rendered}"
            )
        }
        RealmAccessResolverDiagnostic::OldNodeQualifiedTarget {
            legacy_target,
            suggested,
        } => write!(
            f,
            "target `{}` uses the old node-qualified grammar; use `{}`",
            legacy_target.as_str(),
            suggested.to_canonical()
        ),
        RealmAccessResolverDiagnostic::MissingRealmBinding { target, realm } => write!(
            f,
            "target `{}` is in realm `{}`, but this CLI cannot use the selected realm access binding",
            target.to_canonical(),
            realm.target_form()
        ),
        RealmAccessResolverDiagnostic::UnsupportedCrossRealmCapability {
            target,
            capability,
            placement,
        } => write!(
            f,
            "target `{}` requires unsupported cross-realm capability `{}` for placement {:?}",
            target.to_canonical(),
            capability.code(),
            placement
        ),
        RealmAccessResolverDiagnostic::StaleRealmController {
            realm,
            expected_generation,
            observed_generation,
        } => write!(
            f,
            "realm `{}` has stale controller generation (expected {}, observed {})",
            realm.target_form(),
            expected_generation,
            observed_generation
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "none".to_owned())
        ),
        RealmAccessResolverDiagnostic::MissingRealmController { realm } => {
            write!(
                f,
                "realm `{}` has no reachable controller",
                realm.target_form()
            )
        }
    }
}

fn resolver_error(diagnostic: RealmAccessResolverDiagnostic) -> RouteError {
    RouteError::AccessResolver {
        error: Box::new(RealmAccessResolverError {
            diagnostic,
            related: Vec::new(),
        }),
    }
}

/// Why a `d2b realm <verb> <realm>` argument was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RealmArgError {
    Empty,
    BadLabel { label: String, reason: String },
    BadPath { realm: String },
}

impl core::fmt::Display for RealmArgError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RealmArgError::Empty => write!(f, "realm path is empty"),
            RealmArgError::BadLabel { label, reason } => {
                write!(f, "realm label `{label}` is invalid: {reason}")
            }
            RealmArgError::BadPath { realm } => {
                write!(
                    f,
                    "realm path `{realm}` is empty, too long, or has too many labels"
                )
            }
        }
    }
}

/// A target's conventional local gateway entrypoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayHint {
    pub target: String,
    pub realm: RealmPath,
    pub gateway_vm: String,
    pub gateway_target: String,
}

/// Context for the transitional access-contract resolver.
#[derive(Debug, Clone)]
pub struct AccessRouteContext {
    pub default_realm: Option<DefaultRealmSelectionMetadata>,
    pub aliases: Vec<RealmAccessAliasBinding>,
    pub legacy_node_labels: Vec<NodeId>,
    pub required_capabilities: CapabilitySet,
    pub client: RealmAccessClientContract,
}

impl AccessRouteContext {
    /// Compatibility context for today's CLI: bare names stay local, while the
    /// existing gateway helper path is still available for gateway-backed realms.
    pub fn compatibility() -> Self {
        Self {
            default_realm: None,
            aliases: Vec::new(),
            legacy_node_labels: Vec::new(),
            required_capabilities: CapabilitySet::empty(),
            client: RealmAccessClientContract {
                supported_bindings: vec![
                    RealmAccessClientBindingKind::DirectHostLocalUnixSocket,
                    RealmAccessClientBindingKind::RemoteRealmTransportRef,
                ],
                require_direct_local_so_peercred: true,
            },
        }
    }

    #[cfg(test)]
    fn with_default_realm(mut self, realm: RealmPath) -> Self {
        self.default_realm = Some(DefaultRealmSelectionMetadata {
            realm,
            source: DefaultRealmSelectionSource::ExplicitRequest,
            applied: true,
        });
        self
    }

    #[cfg(test)]
    fn with_alias(
        mut self,
        alias: WorkloadId,
        target: RealmTarget,
        source_ref: AccessBindingRef,
    ) -> Self {
        self.aliases.push(RealmAccessAliasBinding {
            alias,
            target,
            source_ref,
        });
        self
    }

    #[cfg(test)]
    fn with_legacy_node_label(mut self, node: NodeId) -> Self {
        self.legacy_node_labels.push(node);
        self
    }

    #[cfg(test)]
    fn with_client(mut self, client: RealmAccessClientContract) -> Self {
        self.client = client;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessRoute {
    pub route: Route,
    pub response: RealmAccessResolverResponse,
    pub request: RealmAccessResolverRequest,
}

/// Parse a CLI realm argument (`work`, `payments.work`) into a realm path.
pub fn parse_realm_arg(raw: &str) -> Result<RealmPath, RealmArgError> {
    if raw.is_empty() {
        return Err(RealmArgError::Empty);
    }
    let labels = raw
        .split('.')
        .map(|label| {
            RealmId::parse(label).map_err(|err| RealmArgError::BadLabel {
                label: label.to_owned(),
                reason: err.to_string(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    RealmPath::new(labels).ok_or_else(|| RealmArgError::BadPath {
        realm: raw.to_owned(),
    })
}

/// Conventional local gateway VM name for a realm (`work` ->
/// `sys-work-gateway`).
pub fn gateway_vm_name(realm: &RealmPath) -> String {
    format!("sys-{}-gateway", realm.target_form().replace('.', "-"))
}

/// Conventional local gateway target address for a realm gateway VM.
pub fn gateway_target_name(realm: &RealmPath) -> Result<TargetName, RouteError> {
    let gateway = gateway_vm_name(realm);
    let raw = format!("{gateway}.local.d2b");
    TargetName::parse(&raw).map_err(|err| RouteError::InvalidGatewayTarget {
        realm: realm.target_form(),
        gateway,
        reason: err.to_string(),
    })
}

/// Classify and resolve a `vm`/target argument against `table`.
pub fn route(raw: &str, table: &RealmEntrypointTable) -> Result<Route, RouteError> {
    resolve_access_route(raw, table, &AccessRouteContext::compatibility()).map(|r| r.route)
}

/// Resolve a CLI target through the access-contract DTO shape, then map it back
/// to today's local/gateway dispatch route.
pub fn resolve_access_route(
    raw: &str,
    table: &RealmEntrypointTable,
    context: &AccessRouteContext,
) -> Result<AccessRoute, RouteError> {
    let request = resolver_request(raw, context)?;
    let Some((target, alias_source)) = parse_target_for_access(raw, context)? else {
        return Ok(local_passthrough_route(raw, request));
    };

    match table.resolve(&target) {
        Ok(DispatchTarget::HostResident { target }) => {
            let response = access_response(
                target.clone(),
                RealmControllerPlacement::HostLocal,
                host_local_binding(target.realm.clone()),
                alias_source,
                context,
            )?;
            Ok(AccessRoute {
                route: Route::Local {
                    vm: target.workload.as_str().to_owned(),
                },
                response,
                request,
            })
        }
        Ok(DispatchTarget::GatewayBacked { gateway, target }) => {
            let response = access_response(
                target.clone(),
                RealmControllerPlacement::GatewayVm,
                gateway_binding(target.realm.clone()),
                alias_source,
                context,
            )?;
            Ok(AccessRoute {
                route: Route::Gateway {
                    gateway: gateway.to_string(),
                    target: target.to_string(),
                },
                response,
                request,
            })
        }
        Err(ResolveError::NoEntrypoint(realm)) => Err(RouteError::NoRealmEntrypoint {
            target: target.to_string(),
            realm: realm.target_form(),
        }),
        Err(ResolveError::MissingGateway(realm)) => Err(RouteError::MissingGateway {
            target: target.to_string(),
            realm: realm.target_form(),
        }),
    }
}

fn resolver_request(
    raw: &str,
    context: &AccessRouteContext,
) -> Result<RealmAccessResolverRequest, RouteError> {
    let requested_target =
        RealmAccessTargetInput::parse(raw.to_owned()).ok_or_else(|| RouteError::InvalidTarget {
            target: raw.to_owned(),
            reason: "target input is empty, too long, contains NUL, or contains whitespace"
                .to_owned(),
        })?;
    Ok(RealmAccessResolverRequest {
        requested_target,
        default_realm: context.default_realm.clone(),
        aliases: context.aliases.clone(),
        required_capabilities: context.required_capabilities.clone(),
        client: context.client.clone(),
    })
}

fn parse_target_for_access(
    raw: &str,
    context: &AccessRouteContext,
) -> Result<Option<(RealmTarget, RealmAccessAliasSource)>, RouteError> {
    let mut parser = RealmTargetParser::new();
    if let Some(selection) = context
        .default_realm
        .as_ref()
        .filter(|selection| selection.applied)
    {
        parser = parser.with_default_realm(selection.realm.clone());
    }
    for alias in &context.aliases {
        parser = parser.with_alias(alias.alias.clone(), alias.target.clone());
    }
    for node in &context.legacy_node_labels {
        parser = parser.with_legacy_node_label(node.clone());
    }

    let has_explicit_scheme = raw.starts_with("d2b://");

    match parser.parse(raw) {
        Ok(target) => Ok(Some((target, alias_source(raw, context)))),
        Err(RealmTargetParseError::BareAliasRequiresContext) => Ok(None),
        Err(RealmTargetParseError::MissingSuffix) if !has_explicit_scheme => Ok(None),
        Err(RealmTargetParseError::MissingRealm)
            if !has_explicit_scheme && local_vm_from_compat_target(raw).is_some() =>
        {
            Ok(None)
        }
        Err(RealmTargetParseError::AliasAmbiguous { alias, candidates }) => Err(resolver_error(
            RealmAccessResolverDiagnostic::AliasAmbiguous {
                alias: alias.clone(),
                candidates: candidates
                    .into_iter()
                    .map(|target| RealmAccessConflictCandidate {
                        realm: target.realm.clone(),
                        target,
                        alias_source: context
                            .aliases
                            .iter()
                            .find(|binding| binding.alias == alias)
                            .map(|binding| RealmAccessAliasSource::AliasTable {
                                alias: alias.clone(),
                                source_ref: binding.source_ref.clone(),
                            })
                            .unwrap_or(RealmAccessAliasSource::FullyQualified),
                        placement: None,
                    })
                    .collect(),
            },
        )),
        Err(RealmTargetParseError::LegacyNodeQualified { legacy, suggested }) => {
            let legacy_target = RealmAccessTargetInput::parse(legacy.diagnostic_form())
                .expect("legacy diagnostic form is valid target input");
            Err(resolver_error(
                RealmAccessResolverDiagnostic::OldNodeQualifiedTarget {
                    legacy_target,
                    suggested,
                },
            ))
        }
        Err(err) => Err(RouteError::InvalidTarget {
            target: raw.to_owned(),
            reason: err.to_string(),
        }),
    }
}

fn alias_source(raw: &str, context: &AccessRouteContext) -> RealmAccessAliasSource {
    let body = raw.strip_prefix("d2b://").unwrap_or(raw);
    if !body.contains('.') {
        if let Ok(alias) = WorkloadId::parse(body) {
            let matches = context
                .aliases
                .iter()
                .filter(|binding| binding.alias == alias)
                .collect::<Vec<_>>();
            if let [binding] = matches.as_slice() {
                return RealmAccessAliasSource::AliasTable {
                    alias,
                    source_ref: binding.source_ref.clone(),
                };
            }
        }
        if let Some(selection) = context
            .default_realm
            .as_ref()
            .filter(|selection| selection.applied)
        {
            return RealmAccessAliasSource::DefaultRealm {
                selection: selection.clone(),
            };
        }
    }
    RealmAccessAliasSource::FullyQualified
}

fn access_response(
    target: RealmTarget,
    placement: RealmControllerPlacement,
    access_binding: RealmAccessBinding,
    alias_source: RealmAccessAliasSource,
    context: &AccessRouteContext,
) -> Result<RealmAccessResolverResponse, RouteError> {
    let selected_kind = binding_kind(&access_binding.transport);
    if !context.client.supported_bindings.contains(&selected_kind) {
        return Err(resolver_error(
            RealmAccessResolverDiagnostic::MissingRealmBinding {
                realm: target.realm.clone(),
                target,
            },
        ));
    }
    if let Some(capability) =
        unsupported_cross_realm_capability(&placement, &context.required_capabilities)
    {
        return Err(resolver_error(
            RealmAccessResolverDiagnostic::UnsupportedCrossRealmCapability {
                target,
                capability,
                placement,
            },
        ));
    }
    let client_binding = RealmAccessClientBinding::from_transport(&access_binding.transport);
    Ok(RealmAccessResolverResponse {
        canonical_target: target.clone(),
        resolved_realm: target.realm.clone(),
        placement,
        access_binding,
        client_binding,
        capability_preflight: RealmAccessCapabilityPreflight {
            required: context.required_capabilities.clone(),
            advertised: context.required_capabilities.clone(),
            status: CapabilityPreflightStatus::Satisfied,
        },
        alias_source,
        default_realm: context.default_realm.clone(),
        diagnostics: Vec::new(),
    })
}

fn binding_kind(transport: &RealmTransportBinding) -> RealmAccessClientBindingKind {
    match transport {
        RealmTransportBinding::LocalUnixSocket { .. } => {
            RealmAccessClientBindingKind::DirectHostLocalUnixSocket
        }
        RealmTransportBinding::RemoteRealmTransport { .. } => {
            RealmAccessClientBindingKind::RemoteRealmTransportRef
        }
        RealmTransportBinding::ProviderRealmTransport { .. } => {
            RealmAccessClientBindingKind::ProviderRealmTransportRef
        }
    }
}

fn unsupported_cross_realm_capability(
    placement: &RealmControllerPlacement,
    required: &CapabilitySet,
) -> Option<Capability> {
    if matches!(placement, RealmControllerPlacement::HostLocal) {
        return None;
    }
    required
        .iter()
        .find(|capability| !is_cross_realm_exportable(*capability))
}

fn is_cross_realm_exportable(capability: Capability) -> bool {
    !matches!(capability, Capability::GpuAccel)
}

fn host_local_binding(realm: RealmPath) -> RealmAccessBinding {
    RealmAccessBinding {
        realm,
        controller_generation: ControllerGenerationId::parse(CLI_ROUTING_GENERATION)
            .expect("static generation id is valid"),
        placement: RealmControllerPlacement::HostLocal,
        transport: RealmTransportBinding::LocalUnixSocket {
            socket_path: UnixSocketPath::parse(DEFAULT_PUBLIC_SOCKET_PATH)
                .expect("static socket path is valid"),
        },
    }
}

fn gateway_binding(realm: RealmPath) -> RealmAccessBinding {
    RealmAccessBinding {
        realm,
        controller_generation: ControllerGenerationId::parse(CLI_ROUTING_GENERATION)
            .expect("static generation id is valid"),
        placement: RealmControllerPlacement::GatewayVm,
        transport: RealmTransportBinding::RemoteRealmTransport {
            transport: ProtocolToken::parse(GATEWAY_TRANSPORT_TOKEN)
                .expect("static transport token is valid"),
            binding_ref: AccessBindingRef::parse(GATEWAY_BINDING_REF)
                .expect("static binding ref is valid"),
        },
    }
}

fn local_passthrough_route(raw: &str, request: RealmAccessResolverRequest) -> AccessRoute {
    let vm = local_vm_from_compat_target(raw).unwrap_or_else(|| raw.to_owned());
    let workload =
        WorkloadId::parse(&vm).unwrap_or_else(|_| WorkloadId::parse("local-vm").unwrap());
    let target = RealmTarget::new(workload, RealmPath::local());
    AccessRoute {
        route: Route::Local { vm },
        response: local_compat_response(target),
        request,
    }
}

fn local_vm_from_compat_target(raw: &str) -> Option<String> {
    let body = raw.strip_prefix("d2b://").unwrap_or(raw);
    let labels = body.split('.').collect::<Vec<_>>();
    match labels.as_slice() {
        [vm, "d2b"] if WorkloadId::parse(*vm).is_ok() => Some((*vm).to_owned()),
        [vm, "this", "local", "d2b"] if WorkloadId::parse(*vm).is_ok() => Some((*vm).to_owned()),
        _ => None,
    }
}

fn local_compat_response(target: RealmTarget) -> RealmAccessResolverResponse {
    RealmAccessResolverResponse {
        canonical_target: target,
        resolved_realm: RealmPath::local(),
        placement: RealmControllerPlacement::HostLocal,
        access_binding: host_local_binding(RealmPath::local()),
        client_binding: RealmAccessClientBinding::DirectHostLocalUnix {
            socket_path: UnixSocketPath::parse(DEFAULT_PUBLIC_SOCKET_PATH)
                .expect("static socket path is valid"),
            peer_credentials: HostLocalPeerCredentialSemantics::direct_client_peercred(),
        },
        capability_preflight: RealmAccessCapabilityPreflight {
            required: CapabilitySet::empty(),
            advertised: CapabilitySet::empty(),
            status: CapabilityPreflightStatus::Satisfied,
        },
        alias_source: RealmAccessAliasSource::DefaultRealm {
            selection: DefaultRealmSelectionMetadata {
                realm: RealmPath::local(),
                source: DefaultRealmSelectionSource::LocalCompatibility,
                applied: true,
            },
        },
        default_realm: None,
        diagnostics: Vec::new(),
    }
}

/// Return the canonical gateway target for a fully-qualified non-local target.
/// Bare/local names return `None` and stay on the local fast path.
#[cfg(test)]
pub fn gateway_candidate(raw: &str) -> Option<String> {
    let target = RealmTarget::parse(raw).ok()?;
    if target.realm == RealmPath::local() {
        None
    } else {
        Some(target.to_canonical())
    }
}

/// Return the realm/gateway hint for a fully-qualified non-local target.
/// Bare/local names return `None` and stay on the local fast path.
pub fn gateway_hint(raw: &str) -> Result<Option<GatewayHint>, RouteError> {
    let has_explicit_scheme = raw.starts_with("d2b://");
    let target = match RealmTarget::parse(raw) {
        Ok(target) => target,
        Err(RealmTargetParseError::BareAliasRequiresContext) => {
            return Ok(None);
        }
        Err(RealmTargetParseError::MissingSuffix) if !has_explicit_scheme => {
            return Ok(None);
        }
        Err(RealmTargetParseError::MissingRealm)
            if !has_explicit_scheme && local_vm_from_compat_target(raw).is_some() =>
        {
            return Ok(None);
        }
        Err(err) => {
            return Err(RouteError::InvalidTarget {
                target: raw.to_owned(),
                reason: err.to_string(),
            });
        }
    };
    if target.realm == RealmPath::local() {
        return Ok(None);
    }
    let realm = target.realm.clone();
    let gateway_vm = gateway_vm_name(&realm);
    let gateway_target = gateway_target_name(&realm)?;
    Ok(Some(GatewayHint {
        target: target.to_canonical(),
        realm,
        gateway_vm,
        gateway_target: gateway_target.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_realm_core::{EntrypointMode, RealmId};
    use d2b_realm_router::RealmEntrypoint;

    fn realm(labels: &[&str]) -> RealmPath {
        RealmPath::new(labels.iter().map(|l| RealmId::parse(*l).unwrap()).collect()).unwrap()
    }

    fn workload(raw: &str) -> WorkloadId {
        WorkloadId::parse(raw).unwrap()
    }

    fn node(raw: &str) -> NodeId {
        NodeId::parse(raw).unwrap()
    }

    fn target(raw: &str) -> RealmTarget {
        RealmTarget::parse(raw).unwrap()
    }

    fn access_ref(raw: &str) -> AccessBindingRef {
        AccessBindingRef::parse(raw).unwrap()
    }

    #[test]
    fn bare_name_is_local_fast_path() {
        let table = RealmEntrypointTable::with_local_default();
        assert_eq!(
            route("work-aca", &table).unwrap(),
            Route::Local {
                vm: "work-aca".to_owned()
            }
        );
    }

    #[test]
    fn unparseable_passes_through_to_local() {
        let table = RealmEntrypointTable::with_local_default();
        assert_eq!(
            route("demo.aca.work", &table).unwrap(),
            Route::Local {
                vm: "demo.aca.work".to_owned()
            }
        );
    }

    #[test]
    fn explicit_scheme_missing_suffix_is_invalid_target() {
        let table = RealmEntrypointTable::with_local_default();
        for raw in ["d2b://builder", "d2b://builder.dev"] {
            let err = route(raw, &table).unwrap_err();
            match err {
                RouteError::InvalidTarget { target, reason } => {
                    assert_eq!(target, raw);
                    assert!(
                        reason.contains("must end in the reserved `.d2b` suffix"),
                        "unexpected reason: {reason}"
                    );
                }
                other => panic!("expected InvalidTarget for {raw}, got {other:?}"),
            }
        }
    }

    #[test]
    fn unschemed_missing_suffix_still_uses_local_fast_path() {
        let table = RealmEntrypointTable::with_local_default();
        assert_eq!(
            route("builder", &table).unwrap(),
            Route::Local {
                vm: "builder".to_owned()
            }
        );
        assert_eq!(
            route("builder.dev", &table).unwrap(),
            Route::Local {
                vm: "builder.dev".to_owned()
            }
        );
    }

    #[test]
    fn old_local_target_forms_preserve_local_fast_path() {
        let table = RealmEntrypointTable::with_local_default();
        assert_eq!(
            route("demo.d2b", &table).unwrap(),
            Route::Local {
                vm: "demo".to_owned()
            }
        );
        assert_eq!(
            route("demo.this.local.d2b", &table).unwrap(),
            Route::Local {
                vm: "demo".to_owned()
            }
        );
    }

    #[test]
    fn host_default_table_fails_closed_on_a_realm_target() {
        let table = RealmEntrypointTable::with_local_default();
        let err = route("demo.work.d2b", &table).unwrap_err();
        match err {
            RouteError::NoRealmEntrypoint { realm, .. } => assert_eq!(realm, "work"),
            other => panic!("expected NoRealmEntrypoint, got {other:?}"),
        }
    }

    #[test]
    fn gateway_backed_realm_routes_to_its_gateway() {
        let mut table = RealmEntrypointTable::with_local_default();
        let gateway = TargetName::parse("gw.work.d2b").unwrap();
        table.gateway_backed(realm(&["work"]), gateway);
        let r = route("demo.work.d2b", &table).unwrap();
        match r {
            Route::Gateway { gateway, target } => {
                assert_eq!(gateway, "gw.work.d2b");
                assert_eq!(target, "demo.work.d2b");
            }
            other => panic!("expected Gateway, got {other:?}"),
        }
    }

    #[test]
    fn host_resident_realm_routes_local() {
        let mut table = RealmEntrypointTable::new();
        table.host_resident(realm(&["work"]));
        let r = route("demo.work.d2b", &table).unwrap();
        assert_eq!(
            r,
            Route::Local {
                vm: "demo".to_owned()
            }
        );
    }

    #[test]
    fn access_response_carries_contract_shape_for_gateway_route() {
        let mut table = RealmEntrypointTable::with_local_default();
        table.gateway_backed(realm(&["work"]), target("gw.work.d2b"));
        let resolved = resolve_access_route(
            "demo.work.d2b",
            &table,
            &AccessRouteContext::compatibility(),
        )
        .unwrap();
        assert_eq!(
            resolved.response.canonical_target.to_canonical(),
            "demo.work.d2b"
        );
        assert_eq!(resolved.response.resolved_realm.target_form(), "work");
        assert_eq!(
            resolved.response.placement,
            RealmControllerPlacement::GatewayVm
        );
        assert!(matches!(
            resolved.response.client_binding,
            RealmAccessClientBinding::RemoteRealmTransportRef { .. }
        ));
        assert!(matches!(
            resolved.response.alias_source,
            RealmAccessAliasSource::FullyQualified
        ));
    }

    #[test]
    fn gateway_candidate_detects_fully_qualified_non_local() {
        assert_eq!(gateway_candidate("vm-a"), None);
        assert_eq!(gateway_candidate("demo.aca.work"), None);
        assert_eq!(
            gateway_candidate("demo.work.d2b").as_deref(),
            Some("demo.work.d2b")
        );
    }

    #[test]
    fn realm_arg_and_gateway_name_follow_gateway_vm_convention() {
        let work = parse_realm_arg("work").unwrap();
        assert_eq!(work.target_form(), "work");
        assert_eq!(gateway_vm_name(&work), "sys-work-gateway");
        assert_eq!(
            gateway_target_name(&work).unwrap().to_string(),
            "sys-work-gateway.local.d2b"
        );

        let nested = parse_realm_arg("payments.work").unwrap();
        assert_eq!(nested.target_form(), "payments.work");
        assert_eq!(gateway_vm_name(&nested), "sys-payments-work-gateway");
    }

    #[test]
    fn realm_arg_rejects_bad_labels() {
        assert!(matches!(parse_realm_arg(""), Err(RealmArgError::Empty)));
        assert!(matches!(
            parse_realm_arg("Work"),
            Err(RealmArgError::BadLabel { .. })
        ));
        assert!(matches!(
            parse_realm_arg("work."),
            Err(RealmArgError::BadLabel { .. })
        ));
    }

    #[test]
    fn gateway_hint_describes_gateway_backed_target_without_routing_it() {
        let hint = gateway_hint("demo.work.d2b")
            .unwrap()
            .expect("realm target has a gateway hint");
        assert_eq!(hint.target, "demo.work.d2b");
        assert_eq!(hint.realm.target_form(), "work");
        assert_eq!(hint.gateway_vm, "sys-work-gateway");
        assert_eq!(hint.gateway_target, "sys-work-gateway.local.d2b");
        assert!(gateway_hint("vm-a").unwrap().is_none());
        assert!(gateway_hint("demo.aca.work").unwrap().is_none());
        assert!(gateway_hint("demo.local.d2b").unwrap().is_none());
    }

    #[test]
    fn gateway_hint_rejects_explicit_scheme_missing_suffix() {
        for raw in ["d2b://builder", "d2b://builder.dev"] {
            let err = gateway_hint(raw).unwrap_err();
            match err {
                RouteError::InvalidTarget { target, reason } => {
                    assert_eq!(target, raw);
                    assert!(
                        reason.contains("must end in the reserved `.d2b` suffix"),
                        "unexpected reason: {reason}"
                    );
                }
                other => panic!("expected InvalidTarget for {raw}, got {other:?}"),
            }
        }
    }

    #[test]
    fn malformed_gateway_backed_entry_without_gateway_fails_closed() {
        let mut table = RealmEntrypointTable::new();
        table.insert(
            realm(&["work"]),
            RealmEntrypoint {
                mode: EntrypointMode::GatewayBacked,
                gateway: None,
            },
        );
        let err = route("demo.work.d2b", &table).unwrap_err();
        match err {
            RouteError::MissingGateway { realm, .. } => assert_eq!(realm, "work"),
            other => panic!("expected MissingGateway, got {other:?}"),
        }
    }

    #[test]
    fn default_realm_resolves_bare_alias_through_access_contract() {
        let mut table = RealmEntrypointTable::with_local_default();
        table.gateway_backed(realm(&["work"]), target("gw.work.d2b"));
        let context = AccessRouteContext::compatibility().with_default_realm(realm(&["work"]));
        let resolved = resolve_access_route("demo", &table, &context).unwrap();
        assert_eq!(
            resolved.response.canonical_target.to_canonical(),
            "demo.work.d2b"
        );
        assert!(matches!(
            resolved.response.alias_source,
            RealmAccessAliasSource::DefaultRealm { .. }
        ));
        assert_eq!(
            resolved.route,
            Route::Gateway {
                gateway: "gw.work.d2b".to_owned(),
                target: "demo.work.d2b".to_owned()
            }
        );
    }

    #[test]
    fn alias_conflict_returns_typed_access_diagnostic() {
        let table = RealmEntrypointTable::with_local_default();
        let context = AccessRouteContext::compatibility()
            .with_alias(
                workload("browser"),
                target("browser.work.d2b"),
                access_ref("aliases-v1"),
            )
            .with_alias(
                workload("browser"),
                target("browser.dev.d2b"),
                access_ref("aliases-v1"),
            );
        let err = resolve_access_route("browser", &table, &context).unwrap_err();
        match err {
            RouteError::AccessResolver { error } => match error.diagnostic {
                RealmAccessResolverDiagnostic::AliasAmbiguous { alias, candidates } => {
                    assert_eq!(alias.as_str(), "browser");
                    let rendered = candidates
                        .iter()
                        .map(|candidate| candidate.target.to_canonical())
                        .collect::<Vec<_>>();
                    assert_eq!(rendered, vec!["browser.work.d2b", "browser.dev.d2b"]);
                }
                other => panic!("expected AliasAmbiguous, got {other:?}"),
            },
            other => panic!("expected access resolver error, got {other:?}"),
        }
    }

    #[test]
    fn old_node_qualified_target_returns_migration_diagnostic() {
        let table = RealmEntrypointTable::with_local_default();
        let context = AccessRouteContext::compatibility().with_legacy_node_label(node("aca"));
        let err = resolve_access_route("demo.aca.work.d2b", &table, &context).unwrap_err();
        match err {
            RouteError::AccessResolver { error } => match error.diagnostic {
                RealmAccessResolverDiagnostic::OldNodeQualifiedTarget {
                    legacy_target,
                    suggested,
                } => {
                    assert_eq!(legacy_target.as_str(), "demo.aca.work.d2b");
                    assert_eq!(suggested.to_canonical(), "demo.work.d2b");
                }
                other => panic!("expected OldNodeQualifiedTarget, got {other:?}"),
            },
            other => panic!("expected access resolver error, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_realm_binding_denies_with_typed_diagnostic() {
        let mut table = RealmEntrypointTable::with_local_default();
        table.gateway_backed(realm(&["work"]), target("gw.work.d2b"));
        let direct_only = RealmAccessClientContract {
            supported_bindings: vec![RealmAccessClientBindingKind::DirectHostLocalUnixSocket],
            require_direct_local_so_peercred: true,
        };
        let context = AccessRouteContext::compatibility().with_client(direct_only);
        let err = resolve_access_route("demo.work.d2b", &table, &context).unwrap_err();
        match err {
            RouteError::AccessResolver { error } => match error.diagnostic {
                RealmAccessResolverDiagnostic::MissingRealmBinding { target, realm } => {
                    assert_eq!(target.to_canonical(), "demo.work.d2b");
                    assert_eq!(realm.target_form(), "work");
                }
                other => panic!("expected MissingRealmBinding, got {other:?}"),
            },
            other => panic!("expected access resolver error, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_cross_realm_capability_denies_with_typed_diagnostic() {
        let mut table = RealmEntrypointTable::with_local_default();
        table.gateway_backed(realm(&["work"]), target("gw.work.d2b"));
        let mut context = AccessRouteContext::compatibility();
        context.required_capabilities = CapabilitySet::from_caps([Capability::GpuAccel]);

        let err = resolve_access_route("demo.work.d2b", &table, &context).unwrap_err();

        match err {
            RouteError::AccessResolver { error } => match error.diagnostic {
                RealmAccessResolverDiagnostic::UnsupportedCrossRealmCapability {
                    target,
                    capability,
                    placement,
                } => {
                    assert_eq!(target.to_canonical(), "demo.work.d2b");
                    assert_eq!(capability, Capability::GpuAccel);
                    assert_eq!(placement, RealmControllerPlacement::GatewayVm);
                }
                other => panic!("expected UnsupportedCrossRealmCapability, got {other:?}"),
            },
            other => panic!("expected access resolver error, got {other:?}"),
        }
    }

    #[test]
    fn host_resident_route_keeps_local_capability_preflight_satisfied() {
        let mut table = RealmEntrypointTable::new();
        table.host_resident(realm(&["work"]));
        let mut context = AccessRouteContext::compatibility();
        context.required_capabilities = CapabilitySet::from_caps([Capability::GpuAccel]);

        let resolved = resolve_access_route("demo.work.d2b", &table, &context)
            .expect("host-local capability placeholder remains satisfied");

        assert_eq!(
            resolved.route,
            Route::Local {
                vm: "demo".to_owned()
            }
        );
        assert_eq!(
            resolved.response.capability_preflight.status,
            CapabilityPreflightStatus::Satisfied
        );
        assert_eq!(
            resolved.response.capability_preflight.required,
            CapabilitySet::from_caps([Capability::GpuAccel])
        );
    }
}
