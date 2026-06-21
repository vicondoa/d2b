//! `nixling-provider-aca`: the Azure Container Apps **sandbox**
//! `WorkloadProvider` implementation for provider-managed sandboxes.
//!
//! This productionizes the Azure Container Apps sandbox path: instead of the
//! operator driving the sandbox by hand with the preview CLI, the gateway drives
//! it through this Rust provider against the Azure Container Apps data-plane REST surface.
//!
//! ## Three-plane auth
//! Plane 1 — Azure control-plane access — is acquired through an explicitly
//! configured workload identity first, then managed identity. The production
//! provider deliberately does not fall back to ambient developer credentials
//! such as Azure CLI or environment credential chains. nixling stores **no**
//! Azure secret of its own. Container→Azure (plane 2, the sandbox Managed
//! Identity) and the nixling-internal per-session credential (plane 3) live in
//! the relay/display providers, not here.
//!
//! ## Data plane
//! `https://management.<region>.azuredevcompute.io/subscriptions/<sub>/
//! resourceGroups/<rg>/sandboxGroups/<sg>/...` with
//! `?api-version=2026-02-01-preview`. Lifecycle uses the preview data-plane
//! REST contract observed from the first-party Azure Container Apps sandbox CLI:
//!
//! - `PUT .../diskimages` creates a disk image from a container image.
//! - `GET .../diskimages&labels=<selector>` finds reusable disk images.
//! - `PUT .../sandboxes` creates a sandbox from a disk image.
//! - `GET .../sandboxes&labels=<selector>` finds an existing sandbox.
//! - `POST .../sandboxes/<id>/stop` stops a sandbox.
//! - `POST .../sandboxes/<id>/executeShellCommand` runs synchronous exec.
//!
//! The credential and the HTTP transport are both injectable traits so the
//! provider is unit-testable without a live subscription; the live path is
//! exercised by an `NL_ACA_LIVE`-gated smoke test.
//!
//! ```no_run
//! # use nixling_constellation_core::NodeId;
//! # use nixling_provider_aca::{AcaConfig, AcaWorkloadProvider};
//! # fn build(cfg: AcaConfig) -> Result<AcaWorkloadProvider, Box<dyn std::error::Error>> {
//! let provider = AcaWorkloadProvider::new(cfg, NodeId::parse("gateway")?)?;
//! # Ok(provider)
//! # }
//! ```
//!
//! Local live-smoke code that intentionally uses developer credentials must
//! inject them explicitly with [`AcaWorkloadProvider::with_parts`]; production
//! [`AcaWorkloadProvider::new`] uses managed/workload identity only.
//!
//! ```no_run
//! # use nixling_constellation_core::NodeId;
//! # use nixling_provider_aca::{AcaConfig, AcaWorkloadProvider, ReqwestTransport};
//! # use std::sync::Arc;
//! # fn build_local(cfg: AcaConfig) -> Result<AcaWorkloadProvider, Box<dyn std::error::Error>> {
//! let credential = azure_identity::AzureCliCredential::new(None)?;
//! let http = Arc::new(ReqwestTransport::new()?);
//! let provider =
//!     AcaWorkloadProvider::with_parts(cfg, NodeId::parse("gateway")?, credential, http);
//! # Ok(provider)
//! # }
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use azure_core::credentials::TokenCredential;
use azure_core::time::{Duration as AzureDuration, OffsetDateTime};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use nixling_constellation_core::{
    Capability, CapabilitySet, ErrorKind, ExecutionId, NodeId, ProviderId, WorkloadId,
};
use nixling_constellation_core::{RealmId, RealmPath, WorkloadSummary};
use nixling_constellation_provider::capabilities::WorkloadCapabilitySet;
use nixling_constellation_provider::error::{
    ProviderDiagnostic, ProviderError, ProviderResult, RetryHint,
};
use nixling_constellation_provider::provider::WorkloadProvider;
use nixling_constellation_provider::rate_limit::{CircuitPermit, ProviderCircuitBreaker};
use nixling_constellation_provider::types::{
    ExecStartRequest, ListSelector, WorkloadSpec, WorkloadStatus,
};

/// The Entra scope for the Azure Container Apps data plane (plane 1). Explicit managed/workload
/// identity credentials acquire tokens for this audience.
pub const AZURE_CONTAINER_APPS_RESOURCE_SCOPE: &str =
    concat!("https:", "//management.azuredevcompute.io/.default");

/// The Azure Container Apps data-plane API version this provider speaks.
pub const AZURE_CONTAINER_APPS_API_VERSION: &str = "2026-02-01-preview";
const READY_POLL_ATTEMPTS: usize = 30;
const READY_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
const MAX_RETRY_HINT: Duration = Duration::from_secs(300);

/// The non-secret coordinates of an Azure Container Apps sandbox group. Every field is an
/// opaque Azure resource identifier (never a secret); `Debug` still redacts
/// the subscription id so it cannot leak into a log or span.
#[derive(Clone)]
pub struct AcaConfig {
    /// Azure subscription id.
    pub subscription: String,
    /// Resource group holding the sandbox group.
    pub resource_group: String,
    /// Sandbox group name.
    pub sandbox_group: String,
    /// Region (selects the Azure Container Apps data-plane endpoint).
    pub region: String,
    /// Optional explicit Azure Container Apps data-plane endpoint for sovereign/private-link
    /// deployments. When unset, `management.<region>.azuredevcompute.io` is used.
    pub endpoint: Option<String>,
    /// Optional user-assigned managed identity client id for Azure Container Apps data-plane
    /// authentication. When absent, the system-assigned identity is used.
    pub managed_identity_client_id: Option<String>,
}

/// How to obtain the disk image for a sandbox create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcaDiskImageSource {
    /// Reuse an existing private disk image id.
    ExistingDiskId(String),
    /// Create or reuse a named private disk image from a container image.
    ContainerImage {
        /// Container image reference, e.g. `registry.azurecr.io/nixling-wayland:mi`.
        image: String,
        /// Stable disk-image label/name. Changing this forces a fresh disk.
        name: String,
        /// Optional user-assigned managed identity resource id for private pulls.
        managed_identity_resource_id: Option<String>,
        /// Extra non-secret disk labels.
        labels: BTreeMap<String, String>,
    },
}

/// Provider defaults required to create an Azure Container Apps sandbox from the narrow
/// `WorkloadSpec { alias }` trait surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcaSandboxDefaults {
    /// Disk image source.
    pub disk_image: AcaDiskImageSource,
    /// CPU request, e.g. `1000m`.
    pub cpu: String,
    /// Memory request, e.g. `2048Mi`.
    pub memory: String,
    /// Auto-suspend interval in seconds.
    pub auto_suspend_interval_secs: u32,
    /// Optional user-assigned managed identity resource id assigned to created
    /// sandboxes. Required for the in-sandbox Relay sender to acquire MI
    /// tokens.
    pub managed_identity_resource_id: Option<String>,
    /// Extra non-secret sandbox labels, e.g. `nixling-realm=work`.
    pub labels: BTreeMap<String, String>,
}

impl AcaSandboxDefaults {
    /// Default resources used by the live Azure Container Apps Wayland proof.
    pub fn new(disk_image: AcaDiskImageSource) -> Self {
        Self {
            disk_image,
            cpu: "1000m".to_owned(),
            memory: "2048Mi".to_owned(),
            auto_suspend_interval_secs: 600,
            managed_identity_resource_id: None,
            labels: BTreeMap::new(),
        }
    }
}

/// A non-secret Azure Container Apps sandbox summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcaSandbox {
    /// Provider UUID / resource id fragment.
    pub id: String,
    /// Best-effort lifecycle state from the data plane.
    pub state: Option<String>,
    /// Azure Container Apps labels.
    pub labels: BTreeMap<String, String>,
}

/// A non-secret Azure Container Apps disk image summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcaDiskImage {
    /// Provider UUID / resource id fragment.
    pub id: String,
    /// Azure Container Apps labels.
    pub labels: BTreeMap<String, String>,
}

impl AcaConfig {
    /// The region-specific Azure Container Apps data-plane endpoint.
    pub fn endpoint(&self) -> String {
        self.endpoint
            .clone()
            .unwrap_or_else(|| format!("https://management.{}.azuredevcompute.io", self.region))
    }

    fn sandbox_base(&self, sandbox_id: &str) -> String {
        format!(
            "{}/subscriptions/{}/resourceGroups/{}/sandboxGroups/{}/sandboxes/{}",
            self.endpoint(),
            self.subscription,
            self.resource_group,
            self.sandbox_group,
            sandbox_id
        )
    }

    /// The `executeShellCommand` URL for a sandbox.
    pub fn exec_url(&self, sandbox_id: &str) -> String {
        format!(
            "{}/executeShellCommand?api-version={}",
            self.sandbox_base(sandbox_id),
            AZURE_CONTAINER_APPS_API_VERSION
        )
    }

    /// The sandbox collection URL.
    pub fn sandboxes_url(&self) -> String {
        format!(
            "{}/subscriptions/{}/resourceGroups/{}/sandboxGroups/{}/sandboxes?api-version={}",
            self.endpoint(),
            self.subscription,
            self.resource_group,
            self.sandbox_group,
            AZURE_CONTAINER_APPS_API_VERSION
        )
    }

    /// The sandbox list URL, optionally filtered by an Azure Container Apps label selector.
    pub fn list_sandboxes_url(&self, labels: Option<&str>) -> String {
        let mut url = self.sandboxes_url();
        if let Some(labels) = labels {
            url.push_str("&labels=");
            url.push_str(&percent_encode_query_value(labels));
        }
        url
    }

    /// The disk image collection URL.
    pub fn disk_images_url(&self) -> String {
        format!(
            "{}/subscriptions/{}/resourceGroups/{}/sandboxGroups/{}/diskimages?api-version={}",
            self.endpoint(),
            self.subscription,
            self.resource_group,
            self.sandbox_group,
            AZURE_CONTAINER_APPS_API_VERSION
        )
    }

    /// The disk image list URL, optionally filtered by an Azure Container Apps label selector.
    pub fn list_disk_images_url(&self, labels: Option<&str>) -> String {
        let mut url = self.disk_images_url();
        if let Some(labels) = labels {
            url.push_str("&labels=");
            url.push_str(&percent_encode_query_value(labels));
        }
        url
    }

    /// The GET-sandbox URL.
    pub fn get_url(&self, sandbox_id: &str) -> String {
        format!(
            "{}?api-version={}",
            self.sandbox_base(sandbox_id),
            AZURE_CONTAINER_APPS_API_VERSION
        )
    }

    /// The resume URL (an Azure Container Apps sandbox auto-suspends to `Idle`; a resume moves
    /// it back to `Running` so `executeShellCommand` stops returning 409).
    pub fn resume_url(&self, sandbox_id: &str) -> String {
        format!(
            "{}/resume?api-version={}",
            self.sandbox_base(sandbox_id),
            AZURE_CONTAINER_APPS_API_VERSION
        )
    }

    /// The stop URL.
    pub fn stop_url(&self, sandbox_id: &str) -> String {
        format!(
            "{}/stop?api-version={}",
            self.sandbox_base(sandbox_id),
            AZURE_CONTAINER_APPS_API_VERSION
        )
    }

    /// The delete URL.
    pub fn delete_url(&self, sandbox_id: &str) -> String {
        format!(
            "{}?api-version={}",
            self.sandbox_base(sandbox_id),
            AZURE_CONTAINER_APPS_API_VERSION
        )
    }
}

impl fmt::Debug for AcaConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Subscription id is redacted; the rest are non-secret coordinates.
        f.debug_struct("AcaConfig")
            .field("subscription", &"<redacted>")
            .field("resource_group", &self.resource_group)
            .field("sandbox_group", &self.sandbox_group)
            .field("region", &self.region)
            .field(
                "managed_identity_client_id",
                &self
                    .managed_identity_client_id
                    .as_ref()
                    .map(|_| "<configured>"),
            )
            .finish()
    }
}

/// The result of a synchronous `executeShellCommand`. `Debug` deliberately
/// never prints `stdout`/`stderr` (workload output is payload, never logged).
#[derive(Clone, Deserialize)]
pub struct ExecResult {
    /// Process exit code.
    #[serde(rename = "exitCode")]
    pub exit_code: i32,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
    /// Server-reported execution time in milliseconds.
    #[serde(rename = "executionTimeMs", default)]
    pub execution_time_ms: u64,
}

impl fmt::Debug for ExecResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecResult")
            .field("exit_code", &self.exit_code)
            .field("stdout_len", &self.stdout.len())
            .field("stderr_len", &self.stderr.len())
            .field("execution_time_ms", &self.execution_time_ms)
            .finish()
    }
}

/// An HTTP method the data-plane transport supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    /// GET.
    Get,
    /// PUT.
    Put,
    /// POST.
    Post,
    /// DELETE.
    Delete,
}

/// A minimal HTTP response (status + raw body).
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Allowlisted response headers.
    pub headers: BTreeMap<String, String>,
    /// Retry metadata parsed once by the circuit-aware request wrapper.
    pub retry_hint: Option<RetryHint>,
    /// Raw response body.
    pub body: String,
}

impl HttpResponse {
    /// Build a response with no allowlisted headers.
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: BTreeMap::new(),
            retry_hint: None,
            body: body.into(),
        }
    }
}

/// The data-plane HTTP transport. Abstracted so the provider can be tested
/// without a live endpoint; the real implementation is [`ReqwestTransport`].
#[async_trait]
pub trait HttpTransport: Send + Sync + fmt::Debug {
    /// Issue a bearer-authenticated JSON request.
    async fn request(
        &self,
        method: HttpMethod,
        url: &str,
        bearer: &str,
        body: Option<String>,
    ) -> ProviderResult<HttpResponse>;
}

/// A `reqwest`-backed [`HttpTransport`] (rustls TLS, JSON).
#[derive(Debug)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    /// Build a transport with a default `reqwest` client.
    pub fn new() -> ProviderResult<Self> {
        let client = reqwest::Client::builder().build().map_err(|err| {
            ProviderError::new(
                ErrorKind::ProviderAllocationFailed,
                format!("failed to build http client: {err}"),
            )
        })?;
        Ok(Self { client })
    }
}

#[async_trait]
impl HttpTransport for ReqwestTransport {
    async fn request(
        &self,
        method: HttpMethod,
        url: &str,
        bearer: &str,
        body: Option<String>,
    ) -> ProviderResult<HttpResponse> {
        let mut req = match method {
            HttpMethod::Get => self.client.get(url),
            HttpMethod::Put => self.client.put(url),
            HttpMethod::Post => self.client.post(url),
            HttpMethod::Delete => self.client.delete(url),
        }
        .bearer_auth(bearer)
        .header("accept", "application/json");
        if let Some(body) = body {
            req = req.header("content-type", "application/json").body(body);
        }
        let resp = req.send().await.map_err(|err| {
            ProviderError::new(
                ErrorKind::ProviderAllocationFailed,
                format!(
                    "Azure Container Apps data-plane request failed: {}",
                    redact_reqwest_error(&err)
                ),
            )
        })?;
        let status = resp.status().as_u16();
        let headers = allowlisted_headers(resp.headers());
        let body = resp.text().await.map_err(|_| {
            ProviderError::new(
                ErrorKind::MalformedFrame,
                "Azure Container Apps data-plane response body was not readable",
            )
        })?;
        Ok(HttpResponse {
            status,
            headers,
            retry_hint: None,
            body,
        })
    }
}

fn allowlisted_headers(headers: &reqwest::header::HeaderMap) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for name in [
        "retry-after",
        "x-ms-retry-after-ms",
        "x-ms-request-id",
        "x-ms-correlation-request-id",
        "x-ms-client-request-id",
    ] {
        if let Some(value) = headers.get(name).and_then(|value| value.to_str().ok()) {
            out.insert(name.to_owned(), value.to_owned());
        }
    }
    out
}

// A reqwest error's Display can include the full URL; keep only the coarse
// classification so an endpoint/token never reaches a log.
fn redact_reqwest_error(err: &reqwest::Error) -> &'static str {
    redact_reqwest_error_flags(
        err.is_timeout(),
        err.is_connect(),
        err.is_request(),
        err.is_body(),
        err.is_decode(),
    )
}

fn redact_reqwest_error_flags(
    is_timeout: bool,
    is_connect: bool,
    is_request: bool,
    is_body: bool,
    is_decode: bool,
) -> &'static str {
    if is_timeout {
        "timeout"
    } else if is_connect {
        "connect"
    } else if is_request {
        "request"
    } else if is_body || is_decode {
        "body"
    } else {
        "transport"
    }
}

fn rest_error(context: &str, resp: &HttpResponse) -> ProviderError {
    let diagnostic = provider_diagnostic(&resp.body, &resp.headers);
    if resp.status == 429 {
        let hint = resp
            .retry_hint
            .unwrap_or_else(|| retry_hint_from_headers(&resp.headers));
        return ProviderError::rate_limited(
            format!(
                "{context} provider-rate-limited; retry after {} ms",
                hint.applied_backoff().as_millis()
            ),
            hint,
        )
        .with_diagnostic(diagnostic);
    }
    let kind = if matches!(diagnostic.code(), Some("AuthorizationFailed")) {
        ErrorKind::Unauthorized
    } else if resp.status == 401 {
        ErrorKind::AuthenticationFailed
    } else if resp.status == 403 {
        ErrorKind::Unauthorized
    } else {
        ErrorKind::ProviderAllocationFailed
    };
    let mut message = format!("{context} returned HTTP {}", resp.status);
    if matches!(resp.status, 401 | 403) || matches!(diagnostic.code(), Some("AuthorizationFailed"))
    {
        message.push_str(
            "; ensure the gateway identity (for example managed identity) has the required Azure Container Apps data-plane role",
        );
    }
    ProviderError::new(kind, message).with_diagnostic(diagnostic)
}

fn provider_diagnostic(body: &str, headers: &BTreeMap<String, String>) -> ProviderDiagnostic {
    let value = serde_json::from_str::<serde_json::Value>(body).ok();
    let code = value.as_ref().and_then(extract_error_code);
    let message = value.as_ref().and_then(extract_error_message);
    let correlation_id = headers
        .get("x-ms-correlation-request-id")
        .or_else(|| headers.get("x-ms-request-id"))
        .or_else(|| headers.get("x-ms-client-request-id"))
        .cloned();
    ProviderDiagnostic::new(code, message, correlation_id)
}

fn extract_error_code(value: &serde_json::Value) -> Option<String> {
    value
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.get("code").and_then(serde_json::Value::as_str))
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
}

fn extract_error_message(value: &serde_json::Value) -> Option<String> {
    for path in [
        &["error", "message"][..],
        &["error", "details"][..],
        &["message"][..],
        &["detail"][..],
    ] {
        let mut cursor = Some(value);
        for key in path {
            cursor = cursor.and_then(|value| value.get(*key));
        }
        if let Some(s) = cursor
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            return Some(s.to_owned());
        }
    }
    None
}

fn retry_hint_from_headers(headers: &BTreeMap<String, String>) -> RetryHint {
    let retry_after = headers
        .get("x-ms-retry-after-ms")
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .or_else(|| {
            headers
                .get("retry-after")
                .and_then(|value| value.parse::<u64>().ok())
                .map(Duration::from_secs)
        })
        .unwrap_or_else(|| Duration::from_secs(30));
    let jitter = deterministic_jitter_ms(millis_u64(retry_after));
    RetryHint::bounded(retry_after, jitter, MAX_RETRY_HINT)
}

fn deterministic_jitter_ms(seed: u64) -> Duration {
    Duration::from_millis(seed.wrapping_mul(97) % 501)
}

fn millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn classify_credential_error(err: &(dyn std::error::Error + 'static)) -> &'static str {
    let mut current = Some(err);
    while let Some(error) = current {
        let message = error.to_string().to_ascii_lowercase();
        if message.contains("workload") || message.contains("federated") {
            return "workload-identity-unavailable";
        }
        if message.contains("managed identity") || message.contains("imds") {
            return "managed-identity-unavailable";
        }
        if message.contains("environment")
            || message.contains("client id")
            || message.contains("client_id")
        {
            return "credential-configuration-missing";
        }
        current = error.source();
    }
    "credential-source-unavailable"
}

#[cfg(test)]
fn classify_credential_error_message(message: &str) -> &'static str {
    let message = message.to_ascii_lowercase();
    if message.contains("workload") || message.contains("federated") {
        "workload-identity-unavailable"
    } else if message.contains("managed identity") || message.contains("imds") {
        "managed-identity-unavailable"
    } else if message.contains("environment")
        || message.contains("client id")
        || message.contains("client_id")
    {
        "credential-configuration-missing"
    } else {
        "credential-source-unavailable"
    }
}

static ACA_CIRCUITS: OnceLock<Mutex<BTreeMap<String, Weak<ProviderCircuitBreaker>>>> =
    OnceLock::new();

fn shared_circuit_for(config: &AcaConfig) -> Arc<ProviderCircuitBreaker> {
    let key = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        config.endpoint(),
        config.subscription,
        config.resource_group,
        config.sandbox_group
    );
    let registry = ACA_CIRCUITS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut circuits = registry.lock().expect("aca circuit registry poisoned");
    circuits.retain(|_, weak| weak.strong_count() > 0);
    if let Some(existing) = circuits.get(&key).and_then(Weak::upgrade) {
        return existing;
    }
    let circuit = Arc::new(ProviderCircuitBreaker::default());
    circuits.insert(key, Arc::downgrade(&circuit));
    circuit
}

#[cfg(test)]
fn managed_identity_env_present_with(mut has_key: impl FnMut(&str) -> bool) -> bool {
    [
        "IDENTITY_ENDPOINT",
        "MSI_ENDPOINT",
        "IMDS_ENDPOINT",
        "AZURE_CLIENT_ID",
        "AZURE_FEDERATED_TOKEN_FILE",
    ]
    .into_iter()
    .any(&mut has_key)
}

/// The Azure Container Apps sandbox `WorkloadProvider`.
pub struct AcaWorkloadProvider {
    config: AcaConfig,
    credential: Arc<dyn TokenCredential>,
    http: Arc<dyn HttpTransport>,
    node: NodeId,
    provider_id: ProviderId,
    sandbox_defaults: Option<AcaSandboxDefaults>,
    lifecycle_lock: Arc<tokio::sync::Mutex<()>>,
    token_cache: Arc<tokio::sync::Mutex<Option<CachedBearer>>>,
    circuit: Arc<ProviderCircuitBreaker>,
}

#[derive(Debug, Clone)]
struct CachedBearer {
    token: String,
    expires_on: OffsetDateTime,
}

struct ProbeGuard {
    circuit: Arc<ProviderCircuitBreaker>,
    permit: CircuitPermit,
    armed: bool,
}

impl ProbeGuard {
    fn new(circuit: Arc<ProviderCircuitBreaker>, permit: CircuitPermit) -> Self {
        Self {
            circuit,
            permit,
            armed: true,
        }
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for ProbeGuard {
    fn drop(&mut self) {
        if self.armed {
            self.circuit
                .record_cancellation(Instant::now(), self.permit);
        }
    }
}

impl fmt::Debug for AcaWorkloadProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AcaWorkloadProvider")
            .field("config", &self.config)
            .field("node", &self.node)
            .field("provider_id", &self.provider_id)
            .finish_non_exhaustive()
    }
}

impl AcaWorkloadProvider {
    /// Build a provider that authenticates only with workload/managed identity
    /// and talks to the Azure Container Apps data plane over `reqwest`.
    pub fn new(config: AcaConfig, node: NodeId) -> ProviderResult<Self> {
        let workload_options = azure_identity::WorkloadIdentityCredentialOptions {
            client_id: config
                .managed_identity_client_id
                .as_ref()
                .filter(|client_id| !client_id.trim().is_empty())
                .cloned(),
            ..Default::default()
        };
        let managed_options = azure_identity::ManagedIdentityCredentialOptions {
            user_assigned_id: config
                .managed_identity_client_id
                .as_ref()
                .filter(|client_id| !client_id.trim().is_empty())
                .map(|client_id| azure_identity::UserAssignedId::ClientId(client_id.clone())),
            ..Default::default()
        };
        let credential = match azure_identity::WorkloadIdentityCredential::new(Some(
            workload_options,
        )) {
            Ok(credential) => credential as Arc<dyn TokenCredential>,
            Err(workload_err) => {
                tracing::debug!(
                    event = "aca-credential-source-unavailable",
                    source = "workload-identity",
                    reason = classify_credential_error(&workload_err),
                    "Azure Container Apps workload identity credential unavailable"
                );
                azure_identity::ManagedIdentityCredential::new(Some(managed_options))
                .map_err(|managed_err| {
                    tracing::debug!(
                        event = "aca-credential-source-unavailable",
                        source = "managed-identity",
                        reason = classify_credential_error(&managed_err),
                        "Azure Container Apps managed identity credential unavailable"
                    );
                    ProviderError::new(
                        ErrorKind::AuthenticationFailed,
                        "Azure Container Apps workload identity and managed identity credential sources are unavailable; verify gateway workload identity or managed identity configuration",
                    )
                })? as Arc<dyn TokenCredential>
            }
        };
        let http = Arc::new(ReqwestTransport::new()?);
        let circuit = shared_circuit_for(&config);
        Ok(Self::with_parts(config, node, credential, http).with_circuit_breaker(circuit))
    }

    /// Build a provider from injected parts (credential + transport) for
    /// local dev/live-smoke only.
    pub fn with_parts(
        config: AcaConfig,
        node: NodeId,
        credential: Arc<dyn TokenCredential>,
        http: Arc<dyn HttpTransport>,
    ) -> Self {
        Self {
            config,
            credential,
            http,
            node,
            provider_id: ProviderId::parse("aca").expect("valid provider id"),
            sandbox_defaults: None,
            lifecycle_lock: Arc::new(tokio::sync::Mutex::new(())),
            token_cache: Arc::new(tokio::sync::Mutex::new(None)),
            circuit: Arc::new(ProviderCircuitBreaker::default()),
        }
    }

    /// Share a provider-endpoint circuit breaker across Azure Container Apps provider instances.
    pub fn with_circuit_breaker(mut self, circuit: Arc<ProviderCircuitBreaker>) -> Self {
        self.circuit = circuit;
        self
    }

    /// Attach the non-secret defaults used to create sandboxes for workload
    /// aliases. Without defaults the provider remains exec-only and lifecycle
    /// calls fail closed.
    pub fn with_sandbox_defaults(mut self, defaults: AcaSandboxDefaults) -> Self {
        self.sandbox_defaults = Some(defaults);
        self
    }

    async fn bearer(&self) -> ProviderResult<String> {
        {
            let cache = self.token_cache.lock().await;
            if let Some(cached) = cache.as_ref()
                && cached.expires_on > OffsetDateTime::now_utc() + AzureDuration::minutes(5)
            {
                return Ok(cached.token.clone());
            }
        }
        let token = self
            .credential
            .get_token(&[AZURE_CONTAINER_APPS_RESOURCE_SCOPE], None)
            .await
            .map_err(|err| {
                tracing::debug!(
                    event = "aca-token-acquisition-failed",
                    reason = classify_credential_error(&err),
                    "Azure Container Apps credential token acquisition failed"
                );
                ProviderError::new(
                    ErrorKind::AuthenticationFailed,
                    "Azure Container Apps credential acquisition failed; verify gateway workload identity or managed identity configuration",
                )
            })?;
        let bearer = token.token.secret().to_owned();
        *self.token_cache.lock().await = Some(CachedBearer {
            token: bearer.clone(),
            expires_on: token.expires_on,
        });
        Ok(bearer)
    }

    async fn request(
        &self,
        method: HttpMethod,
        url: &str,
        bearer: &str,
        body: Option<String>,
    ) -> ProviderResult<HttpResponse> {
        let now = Instant::now();
        let permit = self.circuit.before_request(now)?;
        let guard = permit
            .is_probe()
            .then(|| ProbeGuard::new(self.circuit.clone(), permit));
        let result = self.http.request(method, url, bearer, body).await;
        if let Some(guard) = guard {
            guard.disarm();
        }
        match result {
            Ok(resp) => {
                let mut resp = resp;
                let completed_at = Instant::now();
                if resp.status == 429 {
                    let hint = retry_hint_from_headers(&resp.headers);
                    resp.retry_hint = Some(hint);
                    self.circuit.record_rate_limited(completed_at, hint, permit);
                } else if (500..=599).contains(&resp.status) {
                    self.circuit.record_transient_failure(completed_at, permit);
                } else if resp.status < 500 {
                    self.circuit.record_success(permit);
                }
                Ok(resp)
            }
            Err(err) => {
                self.circuit
                    .record_transient_failure(Instant::now(), permit);
                Err(err)
            }
        }
    }

    /// Run a shell command in `sandbox_id` and return its result
    /// synchronously. An Azure Container Apps sandbox auto-suspends to `Idle`; if the exec is
    /// refused with a 409 the provider resumes the sandbox once and retries
    /// (mirroring the data plane's own resume protocol). Fails closed on any
    /// other non-200 status or an unparseable body.
    pub async fn exec_shell(&self, sandbox_id: &str, command: &str) -> ProviderResult<ExecResult> {
        let bearer = self.bearer().await?;
        let url = self.config.exec_url(sandbox_id);
        let body = serde_json::json!({ "command": command }).to_string();

        let resp = self
            .request(HttpMethod::Post, &url, &bearer, Some(body.clone()))
            .await?;
        let resp = if resp.status == 409 {
            // Sandbox is suspended: resume, then retry the exec exactly once.
            self.resume(sandbox_id, &bearer).await?;
            self.request(HttpMethod::Post, &url, &bearer, Some(body))
                .await?
        } else {
            resp
        };

        if resp.status != 200 {
            return Err(rest_error(
                "Azure Container Apps executeShellCommand",
                &resp,
            ));
        }
        serde_json::from_str::<ExecResult>(&resp.body).map_err(|_| {
            ProviderError::new(
                ErrorKind::MalformedFrame,
                "Azure Container Apps executeShellCommand response was not the expected JSON shape",
            )
        })
    }

    /// Resume a suspended sandbox (moves `Idle` → `Running`). Fail-closed on a
    /// non-200.
    async fn resume(&self, sandbox_id: &str, bearer: &str) -> ProviderResult<()> {
        let url = self.config.resume_url(sandbox_id);
        let resp = self
            .request(HttpMethod::Post, &url, bearer, Some("{}".to_owned()))
            .await?;
        if resp.status != 200 {
            return Err(rest_error("Azure Container Apps sandbox resume", &resp));
        }
        Ok(())
    }

    /// Whether a sandbox exists / is reachable (a 200 GET). Fail-closed on any
    /// non-200.
    pub async fn sandbox_reachable(&self, sandbox_id: &str) -> ProviderResult<bool> {
        let bearer = self.bearer().await?;
        let url = self.config.get_url(sandbox_id);
        let resp = self.request(HttpMethod::Get, &url, &bearer, None).await?;
        match resp.status {
            200 => Ok(true),
            404 => Ok(false),
            _ => Err(rest_error("Azure Container Apps sandbox get", &resp)),
        }
    }

    /// Find the sandbox backing a workload alias via the deterministic
    /// `nixling-workload=<alias>` Azure Container Apps label plus configured realm/default labels
    /// when lifecycle defaults are present.
    pub async fn find_workload_sandbox(
        &self,
        workload: &WorkloadId,
    ) -> ProviderResult<Option<AcaSandbox>> {
        let mut selector = self
            .sandbox_defaults
            .as_ref()
            .map(|defaults| defaults.labels.clone())
            .unwrap_or_default();
        selector.insert("nixling-workload".to_owned(), workload.as_str().to_owned());
        let labels = labels_selector(&selector);
        let sandboxes = self.list_sandboxes(Some(&labels)).await?;
        Ok(sandboxes
            .into_iter()
            .find(|sandbox| labels_match(&sandbox.labels, &selector)))
    }

    /// Ensure the workload alias has a sandbox, creating the disk image and
    /// sandbox through the preview REST data plane when necessary.
    pub async fn ensure_workload_sandbox(
        &self,
        workload: &WorkloadId,
    ) -> ProviderResult<AcaSandbox> {
        let _guard = self.lifecycle_lock.lock().await;
        let defaults = self
            .sandbox_defaults
            .as_ref()
            .ok_or_else(|| ProviderError::capability_denied(Capability::Lifecycle))?;
        if let Some(existing) = self.find_workload_sandbox(workload).await? {
            return Ok(existing);
        }
        let disk_id = self.ensure_disk_image(workload, defaults).await?;
        self.create_sandbox(workload, &disk_id, defaults).await
    }

    async fn wait_workload_running(&self, workload: &WorkloadId) -> ProviderResult<AcaSandbox> {
        for attempt in 0..READY_POLL_ATTEMPTS {
            if let Some(sandbox) = self.find_workload_sandbox(workload).await?
                && sandbox_is_running(&sandbox)
            {
                return Ok(sandbox);
            }
            if attempt + 1 < READY_POLL_ATTEMPTS {
                tokio::time::sleep(READY_POLL_INTERVAL).await;
            }
        }
        Err(ProviderError::new(
            ErrorKind::Timeout,
            "Azure Container Apps sandbox did not reach Running before the readiness deadline",
        ))
    }

    /// List sandboxes, optionally filtered by an Azure Container Apps label selector.
    pub async fn list_sandboxes(&self, labels: Option<&str>) -> ProviderResult<Vec<AcaSandbox>> {
        let bearer = self.bearer().await?;
        let url = self.config.list_sandboxes_url(labels);
        let resp = self.request(HttpMethod::Get, &url, &bearer, None).await?;
        if resp.status != 200 {
            return Err(rest_error("Azure Container Apps sandbox list", &resp));
        }
        parse_sandbox_list(&resp.body)
    }

    /// Stop a sandbox by provider id.
    pub async fn stop_sandbox(&self, sandbox_id: &str) -> ProviderResult<()> {
        let bearer = self.bearer().await?;
        let url = self.config.stop_url(sandbox_id);
        let resp = self
            .request(HttpMethod::Post, &url, &bearer, Some("{}".to_owned()))
            .await?;
        if is_success_no_body_ok(resp.status) {
            Ok(())
        } else {
            Err(rest_error("Azure Container Apps sandbox stop", &resp))
        }
    }

    /// Delete a sandbox by provider id. Exposed for cleanup/live smoke paths;
    /// `WorkloadProvider::stop` uses the less-destructive stop operation.
    pub async fn delete_sandbox(&self, sandbox_id: &str) -> ProviderResult<()> {
        let bearer = self.bearer().await?;
        let url = self.config.delete_url(sandbox_id);
        let resp = self
            .request(HttpMethod::Delete, &url, &bearer, None)
            .await?;
        if is_success_no_body_ok(resp.status) || resp.status == 404 {
            Ok(())
        } else {
            Err(rest_error("Azure Container Apps sandbox delete", &resp))
        }
    }

    async fn ensure_disk_image(
        &self,
        workload: &WorkloadId,
        defaults: &AcaSandboxDefaults,
    ) -> ProviderResult<String> {
        match &defaults.disk_image {
            AcaDiskImageSource::ExistingDiskId(id) => Ok(id.clone()),
            AcaDiskImageSource::ContainerImage {
                image,
                name,
                managed_identity_resource_id,
                labels,
            } => {
                let mut disk_labels = defaults.labels.clone();
                disk_labels.extend(labels.clone());
                if let Some(existing) = self
                    .find_disk_image_by_name(workload, name, &disk_labels)
                    .await?
                {
                    return Ok(existing.id);
                }
                self.create_disk_image(
                    workload,
                    image,
                    name,
                    managed_identity_resource_id.as_deref(),
                    &disk_labels,
                )
                .await
            }
        }
    }

    async fn find_disk_image_by_name(
        &self,
        workload: &WorkloadId,
        name: &str,
        defaults_labels: &BTreeMap<String, String>,
    ) -> ProviderResult<Option<AcaDiskImage>> {
        let mut selector = defaults_labels.clone();
        selector.insert("name".to_owned(), name.to_owned());
        selector.insert("nixling-workload".to_owned(), workload.as_str().to_owned());
        let labels = labels_selector(&selector);
        let bearer = self.bearer().await?;
        let url = self.config.list_disk_images_url(Some(&labels));
        let resp = self.request(HttpMethod::Get, &url, &bearer, None).await?;
        if resp.status != 200 {
            return Err(rest_error("Azure Container Apps disk image list", &resp));
        }
        Ok(parse_disk_image_list(&resp.body)?
            .into_iter()
            .find(|image| labels_match(&image.labels, &selector)))
    }

    async fn create_disk_image(
        &self,
        workload: &WorkloadId,
        image: &str,
        name: &str,
        managed_identity_resource_id: Option<&str>,
        extra_labels: &BTreeMap<String, String>,
    ) -> ProviderResult<String> {
        let bearer = self.bearer().await?;
        let url = self.config.disk_images_url();
        let mut labels = extra_labels.clone();
        labels.insert("name".to_owned(), name.to_owned());
        labels.insert("nixling-workload".to_owned(), workload.as_str().to_owned());
        let body = disk_image_create_body(image, &labels, managed_identity_resource_id)?;
        let resp = self
            .request(HttpMethod::Put, &url, &bearer, Some(body))
            .await?;
        if !is_success_with_body_ok(resp.status) {
            return Err(rest_error("Azure Container Apps disk image create", &resp));
        }
        resource_id_from_body(&resp.body).ok_or_else(|| {
            ProviderError::new(
                ErrorKind::MalformedFrame,
                "Azure Container Apps disk image create response did not contain an id",
            )
        })
    }

    async fn create_sandbox(
        &self,
        workload: &WorkloadId,
        disk_id: &str,
        defaults: &AcaSandboxDefaults,
    ) -> ProviderResult<AcaSandbox> {
        let bearer = self.bearer().await?;
        let url = self.config.sandboxes_url();
        let mut labels = defaults.labels.clone();
        labels.insert("nixling-workload".to_owned(), workload.as_str().to_owned());
        let body = sandbox_create_body(disk_id, defaults, &labels)?;
        let resp = self
            .request(HttpMethod::Put, &url, &bearer, Some(body))
            .await?;
        if !is_success_with_body_ok(resp.status) {
            return Err(rest_error("Azure Container Apps sandbox create", &resp));
        }
        sandbox_from_value(parse_json_value(&resp.body)?).ok_or_else(|| {
            ProviderError::new(
                ErrorKind::MalformedFrame,
                "Azure Container Apps sandbox create response did not contain an id",
            )
        })
    }
}

#[async_trait]
impl WorkloadProvider for AcaWorkloadProvider {
    fn provider_id(&self) -> ProviderId {
        self.provider_id.clone()
    }

    fn node_id(&self) -> NodeId {
        self.node.clone()
    }

    fn capabilities(&self) -> WorkloadCapabilitySet {
        let mut caps = CapabilitySet::empty()
            .with(Capability::Exec)
            .with(Capability::ProviderManagedIsolation);
        if self.sandbox_defaults.is_some() {
            caps = caps.with(Capability::Lifecycle);
        }
        WorkloadCapabilitySet { caps }
    }

    async fn list(&self, selector: ListSelector) -> ProviderResult<Vec<WorkloadSummary>> {
        if self.sandbox_defaults.is_none() {
            return Err(ProviderError::capability_denied(Capability::Lifecycle));
        }
        let mut labels = self
            .sandbox_defaults
            .as_ref()
            .map(|defaults| defaults.labels.clone())
            .unwrap_or_default();
        if let ListSelector::One(workload) = &selector {
            labels.insert("nixling-workload".to_owned(), workload.as_str().to_owned());
        }
        let label_selector = (!labels.is_empty()).then(|| labels_selector(&labels));
        let sandboxes = self.list_sandboxes(label_selector.as_deref()).await?;
        Ok(sandboxes
            .into_iter()
            .filter(|sandbox| labels_match(&sandbox.labels, &labels))
            .filter_map(|sandbox| {
                let workload = sandbox.labels.get("nixling-workload")?;
                let realm = sandbox
                    .labels
                    .get("nixling-realm")
                    .and_then(|label| RealmId::parse(label.clone()).ok())
                    .and_then(|label| RealmPath::new(vec![label]))?;
                let id = WorkloadId::parse(workload.clone()).ok()?;
                Some(WorkloadSummary {
                    id,
                    realm,
                    node: self.node.clone(),
                    state: sandbox_state(&sandbox),
                    capabilities: self.capabilities().caps,
                })
            })
            .collect())
    }

    async fn create(&self, spec: WorkloadSpec) -> ProviderResult<WorkloadId> {
        if self.sandbox_defaults.is_none() {
            return Err(ProviderError::capability_denied(Capability::Lifecycle));
        }
        self.ensure_workload_sandbox(&spec.alias).await?;
        Ok(spec.alias)
    }

    async fn start(&self, id: WorkloadId) -> ProviderResult<WorkloadStatus> {
        if self.sandbox_defaults.is_none() {
            return Err(ProviderError::capability_denied(Capability::Lifecycle));
        }
        let sandbox = self.ensure_workload_sandbox(&id).await?;
        if !sandbox_is_running(&sandbox) {
            self.resume(&sandbox.id, &self.bearer().await?).await?;
        }
        self.wait_workload_running(&id).await?;
        Ok(WorkloadStatus {
            workload: id,
            running: true,
        })
    }

    async fn stop(&self, id: WorkloadId) -> ProviderResult<WorkloadStatus> {
        if self.sandbox_defaults.is_none() {
            return Err(ProviderError::capability_denied(Capability::Lifecycle));
        }
        if let Some(sandbox) = self.find_workload_sandbox(&id).await?
            && sandbox_is_running(&sandbox)
        {
            self.stop_sandbox(&sandbox.id).await?;
        }
        Ok(WorkloadStatus {
            workload: id,
            running: false,
        })
    }

    async fn exec(&self, req: ExecStartRequest) -> ProviderResult<ExecutionId> {
        if req.tty {
            return Err(ProviderError::capability_denied(Capability::Pty));
        }
        let command = std::str::from_utf8(req.command.as_bytes()).map_err(|_| {
            ProviderError::new(
                ErrorKind::MalformedFrame,
                "Azure Container Apps exec command payload was not valid UTF-8",
            )
        })?;
        let sandbox_id = if self.sandbox_defaults.is_some() {
            self.find_workload_sandbox(&req.workload)
                .await?
                .ok_or_else(|| {
                    ProviderError::new(
                        ErrorKind::ProviderAllocationFailed,
                        "Azure Container Apps sandbox was not found for workload",
                    )
                })?
                .id
        } else {
            req.workload.as_str().to_owned()
        };
        let result = self.exec_shell(&sandbox_id, command).await?;
        if result.exit_code != 0 {
            return Err(ProviderError::new(
                ErrorKind::ProviderAllocationFailed,
                format!(
                    "Azure Container Apps exec exited with status {}",
                    result.exit_code
                ),
            ));
        }
        // The synchronous data-plane exec has no durable execution id. Derive a
        // stable opaque id from the authorized request shape rather than the
        // response timing so retries of the same request correlate and two
        // equal-duration calls cannot collide.
        ExecutionId::parse(format!("aca-exec-{}", exec_request_digest(&req))).map_err(|_| {
            ProviderError::new(
                ErrorKind::MalformedFrame,
                "failed to mint Azure Container Apps execution id",
            )
        })
    }
}

fn exec_request_digest(req: &ExecStartRequest) -> String {
    let mut h = Sha256::new();
    h.update(req.workload.as_str().as_bytes());
    h.update([u8::from(req.tty)]);
    h.update(req.command.as_bytes());
    hex(&h.finalize())
}

fn is_success_with_body_ok(status: u16) -> bool {
    matches!(status, 200..=202)
}

fn is_success_no_body_ok(status: u16) -> bool {
    matches!(status, 200 | 202 | 204)
}

fn percent_encode_query_value(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(
                    char::from_digit((b >> 4) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
                out.push(
                    char::from_digit((b & 0x0f) as u32, 16)
                        .unwrap()
                        .to_ascii_uppercase(),
                );
            }
        }
    }
    out
}

fn labels_selector(labels: &BTreeMap<String, String>) -> String {
    labels
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn labels_match(actual: &BTreeMap<String, String>, expected: &BTreeMap<String, String>) -> bool {
    expected
        .iter()
        .all(|(key, value)| actual.get(key) == Some(value))
}

fn parse_json_value(body: &str) -> ProviderResult<serde_json::Value> {
    serde_json::from_str(body).map_err(|_| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            "Azure Container Apps data-plane response was not the expected JSON shape",
        )
    })
}

fn array_from_list_body(body: &str) -> ProviderResult<Vec<serde_json::Value>> {
    let value = parse_json_value(body)?;
    if let Some(array) = value.as_array() {
        return Ok(array.clone());
    }
    for key in ["value", "items"] {
        if let Some(array) = value.get(key).and_then(serde_json::Value::as_array) {
            return Ok(array.clone());
        }
    }
    Err(ProviderError::new(
        ErrorKind::MalformedFrame,
        "Azure Container Apps list response was not an array",
    ))
}

fn labels_from_value(value: &serde_json::Value) -> BTreeMap<String, String> {
    value
        .get("labels")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flat_map(|labels| labels.iter())
        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
        .collect()
}

fn string_field<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        if let Some(s) = value.get(*key).and_then(serde_json::Value::as_str) {
            return Some(s);
        }
    }
    None
}

fn resource_id_from_value(value: &serde_json::Value) -> Option<String> {
    string_field(value, &["id", "name"])
        .or_else(|| {
            value
                .get("properties")
                .and_then(|properties| string_field(properties, &["id", "name"]))
        })
        .map(str::to_owned)
}

fn resource_id_from_body(body: &str) -> Option<String> {
    resource_id_from_value(&parse_json_value(body).ok()?)
}

fn sandbox_from_value(value: serde_json::Value) -> Option<AcaSandbox> {
    let id = resource_id_from_value(&value)?;
    let state = string_field(
        &value,
        &["state", "status", "provisioningState", "runtimeState"],
    )
    .or_else(|| {
        value.get("properties").and_then(|properties| {
            string_field(
                properties,
                &["state", "status", "provisioningState", "runtimeState"],
            )
        })
    })
    .map(str::to_owned);
    Some(AcaSandbox {
        id,
        state,
        labels: labels_from_value(&value),
    })
}

fn disk_image_from_value(value: serde_json::Value) -> Option<AcaDiskImage> {
    let id = resource_id_from_value(&value)?;
    Some(AcaDiskImage {
        id,
        labels: labels_from_value(&value),
    })
}

fn parse_sandbox_list(body: &str) -> ProviderResult<Vec<AcaSandbox>> {
    array_from_list_body(body)?
        .into_iter()
        .map(|value| {
            sandbox_from_value(value).ok_or_else(|| {
                ProviderError::new(
                    ErrorKind::MalformedFrame,
                    "Azure Container Apps sandbox list response contained an item without an id",
                )
            })
        })
        .collect()
}

fn parse_disk_image_list(body: &str) -> ProviderResult<Vec<AcaDiskImage>> {
    array_from_list_body(body)?
        .into_iter()
        .map(|value| {
            disk_image_from_value(value).ok_or_else(|| {
                ProviderError::new(
                    ErrorKind::MalformedFrame,
                    "Azure Container Apps disk image list response contained an item without an id",
                )
            })
        })
        .collect()
}

fn disk_image_create_body(
    image: &str,
    labels: &BTreeMap<String, String>,
    managed_identity_resource_id: Option<&str>,
) -> ProviderResult<String> {
    let mut body = serde_json::json!({
        "image": {
            "base": image,
        },
        "labels": labels,
    });
    if let Some(id) = managed_identity_resource_id {
        body.as_object_mut()
            .expect("disk image body is an object")
            .insert(
                "managedIdentityResourceId".to_owned(),
                serde_json::Value::String(id.to_owned()),
            );
    }
    serde_json::to_string(&body).map_err(|_| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            "failed to serialize Azure Container Apps disk image create body",
        )
    })
}

fn sandbox_create_body(
    disk_id: &str,
    defaults: &AcaSandboxDefaults,
    labels: &BTreeMap<String, String>,
) -> ProviderResult<String> {
    let mut body = serde_json::json!({
        "labels": labels,
        "lifecycle": {
            "autoSuspendPolicy": {
                "enabled": true,
                "interval": defaults.auto_suspend_interval_secs,
                "mode": "Memory",
            },
        },
        "resources": {
            "cpu": defaults.cpu,
            "memory": defaults.memory,
        },
        "sourcesRef": {
            "diskImage": {
                "id": disk_id,
            },
        },
    });
    if let Some(id) = defaults
        .managed_identity_resource_id
        .as_ref()
        .filter(|id| !id.trim().is_empty())
    {
        let mut user_assigned = serde_json::Map::new();
        user_assigned.insert(id.clone(), serde_json::json!({}));
        body.as_object_mut()
            .expect("sandbox create body is an object")
            .insert(
                "identity".to_owned(),
                serde_json::json!({
                    "type": "UserAssigned",
                    "userAssignedIdentities": user_assigned,
                }),
            );
    }
    serde_json::to_string(&body).map_err(|_| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            "failed to serialize Azure Container Apps sandbox create body",
        )
    })
}

fn sandbox_is_running(sandbox: &AcaSandbox) -> bool {
    sandbox
        .state
        .as_deref()
        .is_some_and(|state| matches!(state.to_ascii_lowercase().as_str(), "running" | "ready"))
}

fn sandbox_state(sandbox: &AcaSandbox) -> nixling_constellation_core::WorkloadState {
    match sandbox.state.as_deref().map(str::to_ascii_lowercase) {
        Some(state) if state == "running" || state == "ready" => {
            nixling_constellation_core::WorkloadState::Running
        }
        Some(state) if state == "stopping" => nixling_constellation_core::WorkloadState::Stopping,
        Some(state) if state == "starting" || state == "creating" => {
            nixling_constellation_core::WorkloadState::Starting
        }
        Some(state) if state == "failed" => nixling_constellation_core::WorkloadState::Failed,
        _ => nixling_constellation_core::WorkloadState::Stopped,
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use azure_core::credentials::{AccessToken, TokenRequestOptions};
    use std::sync::Mutex;
    use std::time::SystemTime;

    fn cfg() -> AcaConfig {
        AcaConfig {
            subscription: ["24f3458d-0000-", "0000-0000-000000000000"].concat(),
            resource_group: "rg-nixling-centralus".into(),
            sandbox_group: "casbx-nixling-test".into(),
            region: "centralus".into(),
            endpoint: None,
            managed_identity_client_id: Some(["11111111-", "1111-1111-1111-111111111111"].concat()),
        }
    }

    #[derive(Debug)]
    struct FakeCredential;
    #[async_trait]
    impl TokenCredential for FakeCredential {
        async fn get_token(
            &self,
            _scopes: &[&str],
            _options: Option<TokenRequestOptions<'_>>,
        ) -> azure_core::Result<AccessToken> {
            // A far-future expiry so the SDK does not treat it as expired.
            let expires_on = (SystemTime::now() + std::time::Duration::from_secs(3600)).into();
            Ok(AccessToken::new("fake-token", expires_on))
        }
    }

    type FakeResponse = (u16, String, BTreeMap<String, String>);

    #[derive(Debug)]
    struct FakeHttp {
        responses: Mutex<std::collections::VecDeque<FakeResponse>>,
        calls: Mutex<Vec<(HttpMethod, String, Option<String>)>>,
        delay: std::time::Duration,
    }

    impl FakeHttp {
        fn new_with_headers_and_delay(
            responses: Vec<(u16, &str, BTreeMap<String, String>)>,
            delay: std::time::Duration,
        ) -> Self {
            Self {
                responses: Mutex::new(
                    responses
                        .into_iter()
                        .map(|(s, b, h)| (s, b.to_owned(), h))
                        .collect(),
                ),
                calls: Mutex::new(Vec::new()),
                delay,
            }
        }
    }

    impl Drop for FakeHttp {
        fn drop(&mut self) {
            if std::thread::panicking() {
                return;
            }
            let responses = self.responses.lock().expect("fake http lock poisoned");
            assert!(
                responses.is_empty(),
                "fixture hygiene: unconsumed mock responses"
            );
        }
    }

    #[async_trait]
    impl HttpTransport for FakeHttp {
        async fn request(
            &self,
            method: HttpMethod,
            url: &str,
            bearer: &str,
            body: Option<String>,
        ) -> ProviderResult<HttpResponse> {
            assert_eq!(bearer, "fake-token", "bearer must come from the credential");
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.calls
                .lock()
                .unwrap()
                .push((method, url.to_owned(), body));
            let (status, body, headers) = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("no canned response left for request");
            Ok(HttpResponse {
                status,
                headers,
                retry_hint: None,
                body,
            })
        }
    }

    fn provider(status: u16, body: &str) -> (AcaWorkloadProvider, Arc<FakeHttp>) {
        provider_seq(vec![(status, body)])
    }

    fn provider_seq(responses: Vec<(u16, &str)>) -> (AcaWorkloadProvider, Arc<FakeHttp>) {
        provider_seq_with_headers(
            responses
                .into_iter()
                .map(|(status, body)| (status, body, BTreeMap::new()))
                .collect(),
        )
    }

    fn provider_seq_with_headers(
        responses: Vec<(u16, &str, BTreeMap<String, String>)>,
    ) -> (AcaWorkloadProvider, Arc<FakeHttp>) {
        provider_seq_with_headers_and_delay(responses, std::time::Duration::ZERO)
    }

    fn provider_seq_with_headers_and_delay(
        responses: Vec<(u16, &str, BTreeMap<String, String>)>,
        delay: std::time::Duration,
    ) -> (AcaWorkloadProvider, Arc<FakeHttp>) {
        let http = Arc::new(FakeHttp::new_with_headers_and_delay(responses, delay));
        let provider = AcaWorkloadProvider::with_parts(
            cfg(),
            NodeId::parse("gw").unwrap(),
            Arc::new(FakeCredential),
            http.clone(),
        );
        (provider, http)
    }

    fn sandbox_item(id: &str, state: &str, workload: &str, realm: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "state": state,
            "labels": {
                "nixling-workload": workload,
                "nixling-realm": realm,
            },
        })
    }

    fn sandbox_list(items: Vec<serde_json::Value>) -> String {
        serde_json::Value::Array(items).to_string()
    }

    fn sandbox_body(id: &str, state: &str, workload: &str, realm: &str) -> String {
        sandbox_list(vec![sandbox_item(id, state, workload, realm)])
    }

    fn disk_item(id: &str, name: &str, realm: &str) -> serde_json::Value {
        disk_item_with_workload(id, name, realm, None)
    }

    fn disk_item_with_workload(
        id: &str,
        name: &str,
        realm: &str,
        workload: Option<&str>,
    ) -> serde_json::Value {
        let mut labels = serde_json::Map::new();
        labels.insert(
            "name".to_owned(),
            serde_json::Value::String(name.to_owned()),
        );
        labels.insert(
            "nixling-realm".to_owned(),
            serde_json::Value::String(realm.to_owned()),
        );
        if let Some(workload) = workload {
            labels.insert(
                "nixling-workload".to_owned(),
                serde_json::Value::String(workload.to_owned()),
            );
        }
        serde_json::json!({
            "id": id,
            "labels": labels,
        })
    }

    fn disk_list(items: Vec<serde_json::Value>) -> String {
        serde_json::Value::Array(items).to_string()
    }

    fn too_many_requests_body() -> String {
        serde_json::json!({
            "error": {
                "code": "TooManyRequests",
                "message": "slow down",
            },
        })
        .to_string()
    }

    fn lifecycle_defaults() -> AcaSandboxDefaults {
        let mut labels = BTreeMap::new();
        labels.insert("nixling-realm".to_owned(), "work".to_owned());
        let mut disk_labels = BTreeMap::new();
        disk_labels.insert("nixling-image-tag".to_owned(), "mi".to_owned());
        AcaSandboxDefaults {
            disk_image: AcaDiskImageSource::ContainerImage {
                image: "cr.example.azurecr.io/nixling-wayland:mi".to_owned(),
                name: "nixling-wayland-mi".to_owned(),
                managed_identity_resource_id: Some(
                    [
                        "/sub",
                        "scriptions/24f3458d-0000-",
                        "0000-0000-000000000000/resourceGroups/rg/providers/Microsoft.ManagedIdentity/userAssignedIdentities/id",
                    ]
                    .concat(),
                ),
                labels: disk_labels,
            },
            cpu: "1000m".to_owned(),
            memory: "2048Mi".to_owned(),
            auto_suspend_interval_secs: 600,
            managed_identity_resource_id: Some(
                [
                    "/sub",
                    "scriptions/24f3458d-0000-",
                    "0000-0000-000000000000/resourceGroups/rg/providers/Microsoft.ManagedIdentity/userAssignedIdentities/id",
                ]
                .concat(),
            ),
            labels,
        }
    }

    fn lifecycle_provider_seq(responses: Vec<(u16, &str)>) -> (AcaWorkloadProvider, Arc<FakeHttp>) {
        let (provider, http) = provider_seq(responses);
        (provider.with_sandbox_defaults(lifecycle_defaults()), http)
    }

    #[test]
    fn url_builder_matches_adc_contract() {
        let c = cfg();
        assert_eq!(
            c.exec_url("sbx-1"),
            [
                "h",
                "ttps://management.centralus.azuredevcompute.io/sub",
                "scriptions/24f3458d-0000-",
                "0000-0000-000000000000/resourceGroups/rg-nixling-centralus/",
                "sandboxGroups/casbx-nixling-test/sandboxes/sbx-1/executeShellCommand",
                "?api-version=2026-02-01-preview",
            ]
            .concat()
        );
        assert!(
            c.get_url("sbx-1")
                .ends_with("/sandboxes/sbx-1?api-version=2026-02-01-preview")
        );
        assert!(
            c.list_sandboxes_url(Some("nixling-workload=demo,nixling-realm=work"))
                .ends_with("/sandboxes?api-version=2026-02-01-preview&labels=nixling-workload%3Ddemo%2Cnixling-realm%3Dwork")
        );
        assert!(
            c.disk_images_url()
                .ends_with("/diskimages?api-version=2026-02-01-preview")
        );
        let override_endpoint = AcaConfig {
            endpoint: Some("https:".to_owned() + "//privatelink.example.invalid"),
            ..c.clone()
        };
        let expected_prefix =
            "https:".to_owned() + "//privatelink.example.invalid" + "/sub" + "scriptions/";
        assert!(
            override_endpoint
                .sandboxes_url()
                .starts_with(&expected_prefix)
        );
        assert!(
            c.stop_url("sbx-1")
                .ends_with("/sandboxes/sbx-1/stop?api-version=2026-02-01-preview")
        );
    }

    #[tokio::test]
    async fn exec_shell_posts_command_and_parses_result() {
        let (p, http) = provider(
            200,
            r#"{"exitCode":0,"stdout":"hi\n","stderr":"","executionTimeMs":24}"#,
        );
        let r = p.exec_shell("sbx-1", "echo hi").await.unwrap();
        assert_eq!(r.exit_code, 0);
        assert_eq!(r.stdout, "hi\n");
        let calls = http.calls.lock().unwrap();
        let (method, url, body) = calls[0].clone();
        assert_eq!(method, HttpMethod::Post);
        assert!(url.ends_with("/executeShellCommand?api-version=2026-02-01-preview"));
        assert_eq!(body.unwrap(), r#"{"command":"echo hi"}"#);
    }

    #[tokio::test]
    async fn exec_shell_resumes_on_409_then_retries() {
        // Idle sandbox: first exec 409s, the provider POSTs resume, then the
        // retried exec succeeds. Mirrors the data plane's resume protocol.
        let (p, http) = provider_seq(vec![
            (409, "{\"error\":\"suspended\"}"),
            (200, "{}"),
            (
                200,
                r#"{"exitCode":0,"stdout":"resumed\n","stderr":"","executionTimeMs":5}"#,
            ),
        ]);
        let r = p.exec_shell("sbx-1", "echo hi").await.unwrap();
        assert_eq!(r.stdout, "resumed\n");
        let calls = http.calls.lock().unwrap();
        assert_eq!(calls.len(), 3, "exec(409) -> resume -> exec(200)");
        assert!(
            calls[0]
                .1
                .ends_with("/executeShellCommand?api-version=2026-02-01-preview")
        );
        assert!(
            calls[1]
                .1
                .ends_with("/resume?api-version=2026-02-01-preview")
        );
        assert_eq!(calls[1].2.as_deref(), Some("{}"));
        assert!(
            calls[2]
                .1
                .ends_with("/executeShellCommand?api-version=2026-02-01-preview")
        );
    }

    #[tokio::test]
    async fn exec_shell_fails_closed_on_non_200() {
        let (p, _) = provider(403, "forbidden");
        let err = p.exec_shell("sbx-1", "echo hi").await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unauthorized);
    }

    #[tokio::test]
    async fn exec_shell_fails_closed_on_bad_json() {
        let (p, _) = provider(200, "not-json");
        let err = p.exec_shell("sbx-1", "echo hi").await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedFrame);
    }

    #[tokio::test]
    async fn workload_exec_maps_nonzero_exit_to_typed_error() {
        let (p, _) = provider(
            200,
            r#"{"exitCode":7,"stdout":"","stderr":"boom","executionTimeMs":3}"#,
        );
        let req = ExecStartRequest {
            workload: WorkloadId::parse("sbx-1").unwrap(),
            tty: false,
            command: nixling_constellation_core::OpaquePayload::new(b"false".to_vec()).unwrap(),
        };
        let err = p.exec(req).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ProviderAllocationFailed);
    }

    #[tokio::test]
    async fn workload_exec_denies_tty_before_rest_call() {
        let (p, http) = provider_seq(vec![]);
        let req = ExecStartRequest {
            workload: WorkloadId::parse("sbx-1").unwrap(),
            tty: true,
            command: nixling_constellation_core::OpaquePayload::new(b"sh".to_vec()).unwrap(),
        };
        let err = p.exec(req).await.unwrap_err();
        assert_eq!(err.missing_capability(), Some(Capability::Pty));
        assert_eq!(http.calls.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn lifecycle_workload_exec_resolves_alias_to_sandbox_id() {
        let running = sandbox_body("sandbox-1", "Running", "demo", "work");
        let exec_result = serde_json::json!({
            "exitCode": 0,
            "stdout": "",
            "stderr": "",
            "executionTimeMs": 1,
        })
        .to_string();
        let (p, http) = lifecycle_provider_seq(vec![(200, running.as_str()), (200, &exec_result)]);
        let req = ExecStartRequest {
            workload: WorkloadId::parse("demo").unwrap(),
            tty: false,
            command: nixling_constellation_core::OpaquePayload::new(b"true".to_vec()).unwrap(),
        };
        let _id = p.exec(req).await.unwrap();
        let calls = http.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(
            calls[1]
                .1
                .contains("/sandboxes/sandbox-1/executeShellCommand?"),
            "exec must target resolved sandbox id: {}",
            calls[1].1
        );
    }

    #[tokio::test]
    async fn workload_exec_id_is_derived_from_request_not_elapsed_time() {
        let (p1, _) = provider(
            200,
            r#"{"exitCode":0,"stdout":"","stderr":"","executionTimeMs":1}"#,
        );
        let req = ExecStartRequest {
            workload: WorkloadId::parse("sbx-1").unwrap(),
            tty: false,
            command: nixling_constellation_core::OpaquePayload::new(b"echo hi".to_vec()).unwrap(),
        };
        let id1 = p1.exec(req.clone()).await.unwrap();

        let (p2, _) = provider(
            200,
            r#"{"exitCode":0,"stdout":"","stderr":"","executionTimeMs":999}"#,
        );
        let id2 = p2.exec(req.clone()).await.unwrap();
        assert_eq!(id1, id2);

        let (p3, _) = provider(
            200,
            r#"{"exitCode":0,"stdout":"","stderr":"","executionTimeMs":1}"#,
        );
        let mut changed = req;
        changed.command =
            nixling_constellation_core::OpaquePayload::new(b"echo bye".to_vec()).unwrap();
        let id3 = p3.exec(changed).await.unwrap();
        assert_ne!(id1, id3);
    }

    #[test]
    fn capabilities_are_honest_exec_only() {
        let (p, _) = provider_seq(vec![]);
        let caps = p.capabilities();
        assert!(caps.caps.has(Capability::Exec));
        assert!(caps.caps.has(Capability::ProviderManagedIsolation));
        assert!(!caps.caps.has(Capability::Lifecycle));
        for absent in [
            Capability::Logs,
            Capability::Pty,
            Capability::Vsock,
            Capability::Virtiofs,
            Capability::GpuAccel,
            Capability::Hotplug,
            Capability::Usb,
            Capability::Hid,
            Capability::Snapshots,
        ] {
            assert!(
                !caps.caps.has(absent),
                "ACA must not advertise unsupported capability {}",
                absent.code()
            );
        }

        let (p, _) = lifecycle_provider_seq(vec![]);
        let caps = p.capabilities();
        assert!(caps.caps.has(Capability::Exec));
        assert!(caps.caps.has(Capability::Lifecycle));
        assert!(caps.caps.has(Capability::ProviderManagedIsolation));
    }

    #[test]
    fn local_environment_uses_managed_identity_probe_without_developer_fallback() {
        assert!(!managed_identity_env_present_with(|_| false));
        assert!(managed_identity_env_present_with(
            |key| key == "IDENTITY_ENDPOINT"
        ));
        assert!(managed_identity_env_present_with(
            |key| key == "AZURE_CLIENT_ID"
        ));
    }

    #[test]
    fn new_uses_shared_circuit_for_same_upstream() {
        let p1 = AcaWorkloadProvider::new(cfg(), NodeId::parse("gw").unwrap()).unwrap();
        let p2 = AcaWorkloadProvider::new(cfg(), NodeId::parse("gw").unwrap()).unwrap();
        assert!(Arc::ptr_eq(&p1.circuit, &p2.circuit));
    }

    #[test]
    fn aca_source_imports_no_full_host_or_developer_credential_surfaces() {
        let imports = include_str!("lib.rs")
            .lines()
            .filter(|line| line.trim_start().starts_with("use "))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            ["Azure", "Cli", "Credential"].concat(),
            ["Default", "Azure", "Credential"].concat(),
            ["guest", "control"].concat(),
            ["priv", "broker"].concat(),
            "pidfd".to_owned(),
            "systemd".to_owned(),
            "kvm".to_owned(),
            "vsock".to_owned(),
            "ssh".to_owned(),
            "cgroup".to_owned(),
            "namespace".to_owned(),
        ] {
            assert!(
                !imports
                    .to_ascii_lowercase()
                    .contains(&forbidden.to_ascii_lowercase()),
                "ACA imports must not include forbidden surface {forbidden}: {imports}"
            );
        }
    }

    #[test]
    fn production_constructor_uses_workload_then_managed_identity_only() {
        let source = include_str!("lib.rs");
        let impl_start = source
            .find("impl AcaWorkloadProvider {")
            .expect("AcaWorkloadProvider impl present");
        let start = source[impl_start..]
            .find("pub fn new")
            .map(|offset| impl_start + offset)
            .expect("production constructor present");
        let body_start = source[start..]
            .find('{')
            .map(|offset| start + offset)
            .expect("production constructor body present");
        let mut depth = 0_u32;
        let mut end = None;
        for (offset, ch) in source[body_start..].char_indices() {
            match ch {
                '{' => depth = depth.saturating_add(1),
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        end = Some(body_start + offset + ch.len_utf8());
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = end.expect("production constructor body closes");
        let constructor = &source[start..end];
        let workload_pos = constructor
            .find("WorkloadIdentityCredential::new")
            .expect("workload identity constructor present");
        let managed_pos = constructor
            .find("ManagedIdentityCredential::new")
            .expect("managed identity constructor present");
        assert!(
            workload_pos < managed_pos,
            "production constructor must try workload identity before managed identity"
        );
        for forbidden in [
            ["Azure", "Cli", "Credential"].concat(),
            ["Default", "Azure", "Credential"].concat(),
            ["Environment", "Credential"].concat(),
        ] {
            assert!(
                !constructor.contains(&forbidden),
                "production constructor must not use ambient developer credential {forbidden}"
            );
        }
    }

    #[test]
    fn credential_error_classification_is_low_cardinality() {
        assert_eq!(
            classify_credential_error_message("missing federated token file"),
            "workload-identity-unavailable"
        );
        assert_eq!(
            classify_credential_error_message("IMDS endpoint unavailable"),
            "managed-identity-unavailable"
        );
        assert_eq!(
            classify_credential_error_message("AZURE_CLIENT_ID not configured"),
            "credential-configuration-missing"
        );
        assert_eq!(
            classify_credential_error_message("surprising opaque provider error"),
            "credential-source-unavailable"
        );
    }

    #[derive(Debug)]
    struct NestedCredentialError {
        message: &'static str,
        source: Option<Box<NestedCredentialError>>,
    }

    impl fmt::Display for NestedCredentialError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.message)
        }
    }

    impl std::error::Error for NestedCredentialError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.source
                .as_ref()
                .map(|source| source.as_ref() as &(dyn std::error::Error + 'static))
        }
    }

    #[test]
    fn credential_error_classification_walks_sources() {
        let err = NestedCredentialError {
            message: "outer credential error",
            source: Some(Box::new(NestedCredentialError {
                message: "missing federated token file",
                source: None,
            })),
        };
        assert_eq!(
            classify_credential_error(&err),
            "workload-identity-unavailable"
        );
    }

    #[test]
    fn reqwest_error_redaction_flags_are_low_cardinality() {
        assert_eq!(
            redact_reqwest_error_flags(true, false, false, false, false),
            "timeout"
        );
        assert_eq!(
            redact_reqwest_error_flags(false, true, false, false, false),
            "connect"
        );
        assert_eq!(
            redact_reqwest_error_flags(false, false, true, false, false),
            "request"
        );
        assert_eq!(
            redact_reqwest_error_flags(false, false, false, true, false),
            "body"
        );
        assert_eq!(
            redact_reqwest_error_flags(false, false, false, false, true),
            "body"
        );
        assert_eq!(
            redact_reqwest_error_flags(false, false, false, false, false),
            "transport"
        );
    }

    #[tokio::test]
    async fn lifecycle_ops_fail_closed_without_defaults() {
        let (p, http) = provider_seq(vec![]);
        assert_eq!(
            p.create(WorkloadSpec {
                alias: WorkloadId::parse("x").unwrap()
            })
            .await
            .unwrap_err()
            .kind(),
            ErrorKind::CapabilityDenied
        );
        assert_eq!(
            p.stop(WorkloadId::parse("x").unwrap())
                .await
                .unwrap_err()
                .missing_capability(),
            Some(Capability::Lifecycle)
        );
        assert_eq!(
            http.calls.lock().unwrap().len(),
            0,
            "lifecycle capability denials must happen before REST"
        );
        assert_eq!(
            p.start(WorkloadId::parse("x").unwrap())
                .await
                .unwrap_err()
                .missing_capability(),
            Some(Capability::Lifecycle)
        );
        assert_eq!(
            p.list(ListSelector::All)
                .await
                .unwrap_err()
                .missing_capability(),
            Some(Capability::Lifecycle)
        );
    }

    #[tokio::test]
    async fn create_sandbox_uses_rest_disk_and_sandbox_contract() {
        let created_disk = disk_item("disk-1", "nixling-wayland-mi", "work").to_string();
        let created_sandbox = sandbox_item("sandbox-1", "Running", "demo", "work").to_string();
        let (p, http) = lifecycle_provider_seq(vec![
            (200, "[]"),
            (200, "[]"),
            (201, created_disk.as_str()),
            (201, created_sandbox.as_str()),
        ]);
        let id = p
            .create(WorkloadSpec {
                alias: WorkloadId::parse("demo").unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(id.as_str(), "demo");

        let calls = http.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[0].0, HttpMethod::Get);
        assert!(
            calls[0].1.contains(
                "/sandboxes?api-version=2026-02-01-preview&labels=nixling-realm%3Dwork%2Cnixling-workload%3Ddemo"
            )
        );
        assert_eq!(calls[1].0, HttpMethod::Get);
        assert!(
            calls[1]
                .1
                .contains("/diskimages?api-version=2026-02-01-preview&labels=")
        );
        assert!(calls[1].1.contains("name%3Dnixling-wayland-mi"));
        assert!(calls[1].1.contains("nixling-workload%3Ddemo"));
        assert_eq!(calls[2].0, HttpMethod::Put);
        assert!(
            calls[2]
                .1
                .ends_with("/diskimages?api-version=2026-02-01-preview")
        );
        let disk_body: serde_json::Value =
            serde_json::from_str(calls[2].2.as_ref().unwrap()).unwrap();
        assert_eq!(
            disk_body["image"]["base"],
            "cr.example.azurecr.io/nixling-wayland:mi"
        );
        assert_eq!(disk_body["labels"]["name"], "nixling-wayland-mi");
        assert_eq!(disk_body["labels"]["nixling-workload"], "demo");
        assert!(
            disk_body["managedIdentityResourceId"]
                .as_str()
                .unwrap()
                .contains("/userAssignedIdentities/id")
        );
        assert_eq!(calls[3].0, HttpMethod::Put);
        assert!(
            calls[3]
                .1
                .ends_with("/sandboxes?api-version=2026-02-01-preview")
        );
        let sandbox_body: serde_json::Value =
            serde_json::from_str(calls[3].2.as_ref().unwrap()).unwrap();
        assert_eq!(sandbox_body["sourcesRef"]["diskImage"]["id"], "disk-1");
        assert_eq!(sandbox_body["resources"]["cpu"], "1000m");
        assert_eq!(sandbox_body["resources"]["memory"], "2048Mi");
        assert_eq!(
            sandbox_body["lifecycle"]["autoSuspendPolicy"]["interval"],
            600
        );
        assert_eq!(sandbox_body["identity"]["type"], "UserAssigned");
        assert!(
            sandbox_body["identity"]["userAssignedIdentities"]
                .as_object()
                .unwrap()
                .keys()
                .any(|key| key.contains("/userAssignedIdentities/id"))
        );
        assert_eq!(sandbox_body["labels"]["nixling-workload"], "demo");
        assert_eq!(sandbox_body["labels"]["nixling-realm"], "work");
    }

    #[tokio::test]
    async fn start_reuses_existing_sandbox_and_resumes_idle() {
        let idle = sandbox_body("sandbox-1", "Idle", "demo", "work");
        let running = sandbox_body("sandbox-1", "Running", "demo", "work");
        let (p, http) = lifecycle_provider_seq(vec![
            (200, idle.as_str()),
            (200, "{}"),
            (200, running.as_str()),
        ]);
        let status = p.start(WorkloadId::parse("demo").unwrap()).await.unwrap();
        assert!(status.running);
        assert_eq!(status.workload.as_str(), "demo");

        let calls = http.calls.lock().unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].0, HttpMethod::Get);
        assert!(
            calls[0].1.contains(
                "/sandboxes?api-version=2026-02-01-preview&labels=nixling-realm%3Dwork%2Cnixling-workload%3Ddemo"
            )
        );
        assert_eq!(calls[1].0, HttpMethod::Post);
        assert!(
            calls[1]
                .1
                .ends_with("/sandboxes/sandbox-1/resume?api-version=2026-02-01-preview")
        );
        assert_eq!(calls[1].2.as_deref(), Some("{}"));
        assert_eq!(calls[2].0, HttpMethod::Get);
    }

    #[tokio::test]
    async fn lifecycle_list_fails_closed_with_azure_error_message() {
        let mut headers = BTreeMap::new();
        headers.insert(
            "x-ms-correlation-request-id".to_owned(),
            "corr-123".to_owned(),
        );
        let (p, _) = provider_seq_with_headers(vec![(
            403,
            r#"{"error":{"code":"AuthorizationFailed","message":"quota denied for sandbox group"}}"#,
            headers,
        )]);
        let p = p.with_sandbox_defaults(lifecycle_defaults());
        let err = p.list(ListSelector::All).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unauthorized);
        assert_eq!(
            err.diagnostic().and_then(ProviderDiagnostic::code),
            Some("AuthorizationFailed")
        );
        assert_eq!(
            err.diagnostic()
                .and_then(ProviderDiagnostic::correlation_id),
            Some("corr-123")
        );
        assert!(err.to_string().contains("quota denied"));
    }

    #[tokio::test]
    async fn raw_auth_statuses_map_to_operator_remediation() {
        for (status, kind) in [
            (401, ErrorKind::AuthenticationFailed),
            (403, ErrorKind::Unauthorized),
        ] {
            let (p, _) = provider_seq(vec![(status, "{}")]);
            let err = p.sandbox_reachable("sandbox-1").await.unwrap_err();
            assert_eq!(err.kind(), kind);
            assert!(
                err.to_string()
                    .contains("required Azure Container Apps data-plane role"),
                "raw auth status should include operator remediation: {err}"
            );
        }
    }

    #[tokio::test]
    async fn authorization_failed_code_maps_to_unauthorized_on_any_status() {
        let body = serde_json::json!({
            "error": {
                "code": "AuthorizationFailed",
                "message": "role assignment missing",
            },
        })
        .to_string();
        let (p, _) = provider(400, &body);
        let err = p.sandbox_reachable("sandbox-1").await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unauthorized);
        assert!(
            err.to_string()
                .contains("required Azure Container Apps data-plane role"),
            "AuthorizationFailed should include operator remediation: {err}"
        );
    }

    #[test]
    fn aca_provider_diagnostic_sanitizes_azure_error_body() {
        let mut headers = BTreeMap::new();
        headers.insert(
            "x-ms-correlation-request-id".to_owned(),
            "corr/tenant-specific".to_owned(),
        );
        let message = [
            "dynamic provider body at h",
            "ttps://example.invalid/sub",
            "scriptions/00000000-0000-",
            "0000-0000-000000000000",
        ]
        .concat();
        let body = serde_json::json!({
            "error": {
                "code": "TenantSpecificErrorCode",
                "message": message,
            },
        })
        .to_string();
        let diagnostic = provider_diagnostic(&body, &headers);
        assert_eq!(diagnostic.code(), Some("unknown"));
        assert_eq!(diagnostic.message(), Some("provider message redacted"));
        assert_eq!(diagnostic.correlation_id(), Some("corrtenant-specific"));
    }

    #[tokio::test]
    async fn rate_limit_maps_to_retry_hint_and_opens_shared_circuit() {
        let mut headers = BTreeMap::new();
        headers.insert("x-ms-retry-after-ms".to_owned(), "1250".to_owned());
        let circuit = Arc::new(ProviderCircuitBreaker::default());
        let too_many = too_many_requests_body();
        let (p1, http1) = provider_seq_with_headers(vec![(429, too_many.as_str(), headers)]);
        let p1 = p1
            .with_sandbox_defaults(lifecycle_defaults())
            .with_circuit_breaker(circuit.clone());
        let err = p1.list(ListSelector::All).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Backpressure);
        let hint = err.retry_hint().unwrap();
        assert_eq!(hint.retry_after(), Duration::from_millis(1250));
        assert!(hint.applied_backoff() >= Duration::from_millis(1250));
        assert!(hint.applied_backoff() <= Duration::from_millis(1750));
        assert_eq!(
            circuit.snapshot(std::time::Instant::now()).retry_hint,
            Some(hint)
        );
        assert_eq!(http1.calls.lock().unwrap().len(), 1);

        let (p2, http2) = provider_seq(vec![]);
        let p2 = p2
            .with_sandbox_defaults(lifecycle_defaults())
            .with_circuit_breaker(circuit);
        let err = p2.list(ListSelector::All).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Backpressure);
        assert!(
            err.to_string().contains("circuit breaker open"),
            "open-circuit error should be actionable: {err}"
        );
        assert_eq!(http2.calls.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn rate_limit_backoff_starts_at_response_completion() {
        let mut headers = BTreeMap::new();
        headers.insert("x-ms-retry-after-ms".to_owned(), "5000".to_owned());
        let circuit = Arc::new(ProviderCircuitBreaker::default());
        let too_many = too_many_requests_body();
        let (p1, _) = provider_seq_with_headers_and_delay(
            vec![(429, too_many.as_str(), headers)],
            std::time::Duration::from_millis(600),
        );
        let p1 = p1
            .with_sandbox_defaults(lifecycle_defaults())
            .with_circuit_breaker(circuit.clone());
        let err = p1.list(ListSelector::All).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Backpressure);
        let remaining = circuit
            .snapshot(std::time::Instant::now())
            .remaining
            .unwrap();
        assert!(
            remaining >= Duration::from_millis(4_900),
            "backoff should be anchored to response completion; remaining={remaining:?}"
        );

        let (p2, http2) = provider_seq(vec![]);
        let p2 = p2
            .with_sandbox_defaults(lifecycle_defaults())
            .with_circuit_breaker(circuit);
        let err = p2.list(ListSelector::All).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Backpressure);
        assert_eq!(
            http2.calls.lock().unwrap().len(),
            0,
            "circuit should still be open after the slow 429 response completes"
        );
    }

    #[test]
    fn aca_error_extractors_try_fallback_keys() {
        let body = serde_json::json!({
            "error": {
                "details": "fallback detail",
            },
        });
        assert_eq!(
            extract_error_message(&body),
            Some("fallback detail".to_owned())
        );

        let top_level = serde_json::json!({
            "code": "AuthorizationFailed",
            "detail": "top-level detail",
        });
        assert_eq!(
            extract_error_code(&top_level),
            Some("AuthorizationFailed".to_owned())
        );
        assert_eq!(
            extract_error_message(&top_level),
            Some("top-level detail".to_owned())
        );
    }

    #[tokio::test]
    async fn lifecycle_start_fails_closed_on_resume_error() {
        let idle = sandbox_body("sandbox-1", "Idle", "demo", "work");
        let (p, _) = lifecycle_provider_seq(vec![
            (200, idle.as_str()),
            (500, r#"{"error":{"message":"resume backend unavailable"}}"#),
        ]);
        let err = p
            .start(WorkloadId::parse("demo").unwrap())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ProviderAllocationFailed);
        assert!(err.to_string().contains("resume backend unavailable"));
    }

    #[tokio::test]
    async fn lifecycle_stop_fails_closed_on_stop_error() {
        let running = sandbox_body("sandbox-1", "Running", "demo", "work");
        let (p, _) = lifecycle_provider_seq(vec![
            (200, running.as_str()),
            (409, r#"{"error":{"message":"cannot stop sandbox now"}}"#),
        ]);
        let err = p
            .stop(WorkloadId::parse("demo").unwrap())
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ProviderAllocationFailed);
        assert!(err.to_string().contains("cannot stop sandbox now"));
    }

    #[tokio::test]
    async fn lifecycle_create_fails_closed_on_disk_list_error() {
        let (p, _) = lifecycle_provider_seq(vec![
            (200, "[]"),
            (500, r#"{"error":{"message":"disk image list failed"}}"#),
        ]);
        let err = p
            .create(WorkloadSpec {
                alias: WorkloadId::parse("demo").unwrap(),
            })
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ProviderAllocationFailed);
        assert!(err.to_string().contains("disk image list failed"));
    }

    #[tokio::test]
    async fn malformed_list_items_fail_closed() {
        let (p, _) = lifecycle_provider_seq(vec![(
            200,
            r#"[{"state":"Running","labels":{"nixling-workload":"demo"}}]"#,
        )]);
        let err = p.list(ListSelector::All).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MalformedFrame);
    }

    #[tokio::test]
    async fn stop_resolves_alias_to_sandbox_and_posts_stop() {
        let running = sandbox_body("sandbox-1", "Running", "demo", "work");
        let (p, http) = lifecycle_provider_seq(vec![(200, running.as_str()), (202, "")]);
        let status = p.stop(WorkloadId::parse("demo").unwrap()).await.unwrap();
        assert!(!status.running);
        let calls = http.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].0, HttpMethod::Post);
        assert!(
            calls[1]
                .1
                .ends_with("/sandboxes/sandbox-1/stop?api-version=2026-02-01-preview")
        );
    }

    #[tokio::test]
    async fn workload_lookup_rejects_cross_realm_stale_label() {
        let personal_sandbox = sandbox_body("sandbox-1", "Running", "demo", "personal");
        let personal_disk = disk_list(vec![disk_item(
            "disk-personal",
            "nixling-wayland-mi",
            "personal",
        )]);
        let work_disk = disk_item("disk-1", "nixling-wayland-mi", "work").to_string();
        let work_sandbox = sandbox_item("sandbox-2", "Running", "demo", "work").to_string();
        let (p, http) = lifecycle_provider_seq(vec![
            (200, personal_sandbox.as_str()),
            (200, personal_disk.as_str()),
            (201, work_disk.as_str()),
            (201, work_sandbox.as_str()),
        ]);
        let id = p
            .create(WorkloadSpec {
                alias: WorkloadId::parse("demo").unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(id.as_str(), "demo");
        let calls = http.calls.lock().unwrap();
        assert_eq!(calls.len(), 4, "cross-realm sandbox must not be reused");
    }

    #[tokio::test]
    async fn workload_lookup_rejects_cross_workload_stale_labels() {
        let other_sandbox = sandbox_body("sandbox-1", "Running", "other", "work");
        let other_disk = disk_list(vec![disk_item_with_workload(
            "disk-other",
            "nixling-wayland-mi",
            "work",
            Some("other"),
        )]);
        let demo_sandbox = sandbox_item("sandbox-2", "Running", "demo", "work").to_string();
        let demo_disk = disk_item("disk-1", "nixling-wayland-mi", "work").to_string();
        let (p, http) = lifecycle_provider_seq(vec![
            (200, other_sandbox.as_str()),
            (200, other_disk.as_str()),
            (201, demo_disk.as_str()),
            (201, demo_sandbox.as_str()),
        ]);
        let id = p
            .create(WorkloadSpec {
                alias: WorkloadId::parse("demo").unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(id.as_str(), "demo");
        let calls = http.calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            4,
            "cross-workload resources must not be reused"
        );
    }

    #[tokio::test]
    async fn shared_circuit_key_includes_all_upstream_dimensions() {
        let base = cfg();
        for mutate in [
            |config: &mut AcaConfig| {
                config.endpoint = Some("https:".to_owned() + "//other.example.invalid");
            },
            |config: &mut AcaConfig| {
                config.subscription = "other-sub".to_owned();
            },
            |config: &mut AcaConfig| {
                config.resource_group = "other-rg".to_owned();
            },
            |config: &mut AcaConfig| {
                config.sandbox_group = "other-sg".to_owned();
            },
        ] {
            let mut variant = base.clone();
            mutate(&mut variant);
            assert!(!Arc::ptr_eq(
                &shared_circuit_for(&base),
                &shared_circuit_for(&variant)
            ));
        }
    }

    #[tokio::test]
    async fn sandbox_reachable_bubbles_unavailable_states() {
        let (p, _) = provider(404, "{}");
        assert!(!p.sandbox_reachable("sandbox-1").await.unwrap());

        let mut headers = BTreeMap::new();
        headers.insert("retry-after".to_owned(), "1".to_owned());
        let too_many = too_many_requests_body();
        let (p, _) = provider_seq_with_headers(vec![(429, too_many.as_str(), headers)]);
        let err = p.sandbox_reachable("sandbox-1").await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Backpressure);
    }

    #[tokio::test]
    async fn list_maps_aca_labels_to_workload_summaries() {
        let body = sandbox_list(vec![
            sandbox_item("sandbox-1", "Running", "demo", "work"),
            sandbox_item("sandbox-2", "Failed", "demo", "personal"),
            serde_json::json!({
                "id": "sandbox-3",
                "state": "Failed",
                "labels": {
                    "other": "ignored",
                },
            }),
        ]);
        let (p, _) = lifecycle_provider_seq(vec![(200, body.as_str())]);
        let list = p.list(ListSelector::All).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id.as_str(), "demo");
        assert_eq!(
            list[0].state,
            nixling_constellation_core::WorkloadState::Running
        );
    }

    #[test]
    fn exec_result_debug_redacts_output() {
        let r = ExecResult {
            exit_code: 0,
            stdout: "secret-output".into(),
            stderr: "secret-err".into(),
            execution_time_ms: 1,
        };
        let dbg = format!("{r:?}");
        assert!(!dbg.contains("secret-output"));
        assert!(!dbg.contains("secret-err"));
        assert!(dbg.contains("stdout_len"));
    }
}
