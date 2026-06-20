//! `nixling-provider-aca`: the Azure Container Apps **sandbox**
//! `WorkloadProvider` (ADR 0032, P0).
//!
//! This productionizes the ACA leg of the P0 vertical: instead of the
//! operator driving the sandbox by hand with the preview `aca` CLI, the
//! gateway drives it through this Rust provider against the ADC data-plane
//! REST surface.
//!
//! ## Three-plane auth (operator directive)
//! Plane 1 — Azure control-plane access — is acquired through a chained
//! managed-identity → Azure-CLI credential. A gateway guest or daemon can use a
//! managed identity; a developer can still use an ambient `az login` session.
//! nixling stores **no** Azure secret of its own. Container→Azure (plane 2, the
//! sandbox Managed Identity) and the nixling-internal per-session credential
//! (plane 3) live in the relay/display providers, not here.
//!
//! ## Data plane
//! `https://management.<region>.azuredevcompute.io/subscriptions/<sub>/
//! resourceGroups/<rg>/sandboxGroups/<sg>/...` with
//! `?api-version=2026-02-01-preview`. Lifecycle uses the preview data-plane
//! REST contract observed from the first-party ACA sandbox CLI:
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

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use azure_core::credentials::{AccessToken, TokenCredential, TokenRequestOptions};
use azure_core::time::{Duration as AzureDuration, OffsetDateTime};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use nixling_constellation_core::{
    Capability, CapabilitySet, ErrorKind, ExecutionId, NodeId, ProviderId, WorkloadId,
};
use nixling_constellation_core::{RealmPath, WorkloadSummary};
use nixling_constellation_provider::capabilities::WorkloadCapabilitySet;
use nixling_constellation_provider::error::{ProviderError, ProviderResult};
use nixling_constellation_provider::provider::WorkloadProvider;
use nixling_constellation_provider::types::{
    ExecStartRequest, ListSelector, WorkloadSpec, WorkloadStatus,
};

/// The Entra scope for the ADC data plane (plane 1). Managed identity and the
/// Azure CLI fallback both acquire tokens for this audience.
pub const ADC_RESOURCE_SCOPE: &str = "https://management.azuredevcompute.io/.default";

/// The ADC data-plane API version this provider speaks.
pub const ADC_API_VERSION: &str = "2026-02-01-preview";
const READY_POLL_ATTEMPTS: usize = 30;
const READY_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// The non-secret coordinates of an ACA sandbox group. Every field is an
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
    /// Region (selects the ADC data-plane endpoint).
    pub region: String,
    /// Optional explicit ADC data-plane endpoint for sovereign/private-link
    /// deployments. When unset, `management.<region>.azuredevcompute.io` is used.
    pub endpoint: Option<String>,
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

/// Provider defaults required to create an ACA sandbox from the narrow
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
    /// P0 default resources used by the live ACA Wayland POC.
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

/// A non-secret ACA sandbox summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcaSandbox {
    /// Provider UUID / resource id fragment.
    pub id: String,
    /// Best-effort lifecycle state from the data plane.
    pub state: Option<String>,
    /// ACA labels.
    pub labels: BTreeMap<String, String>,
}

/// A non-secret ACA disk image summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcaDiskImage {
    /// Provider UUID / resource id fragment.
    pub id: String,
    /// ACA labels.
    pub labels: BTreeMap<String, String>,
}

impl AcaConfig {
    /// The region-specific ADC data-plane endpoint.
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
            ADC_API_VERSION
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
            ADC_API_VERSION
        )
    }

    /// The sandbox list URL, optionally filtered by an ACA label selector.
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
            ADC_API_VERSION
        )
    }

    /// The disk image list URL, optionally filtered by an ACA label selector.
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
            ADC_API_VERSION
        )
    }

    /// The resume URL (an ACA sandbox auto-suspends to `Idle`; a resume moves
    /// it back to `Running` so `executeShellCommand` stops returning 409).
    pub fn resume_url(&self, sandbox_id: &str) -> String {
        format!(
            "{}/resume?api-version={}",
            self.sandbox_base(sandbox_id),
            ADC_API_VERSION
        )
    }

    /// The stop URL.
    pub fn stop_url(&self, sandbox_id: &str) -> String {
        format!(
            "{}/stop?api-version={}",
            self.sandbox_base(sandbox_id),
            ADC_API_VERSION
        )
    }

    /// The delete URL.
    pub fn delete_url(&self, sandbox_id: &str) -> String {
        format!(
            "{}?api-version={}",
            self.sandbox_base(sandbox_id),
            ADC_API_VERSION
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
    /// Raw response body.
    pub body: String,
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
                    "aca data-plane request failed: {}",
                    redact_reqwest_error(&err)
                ),
            )
        })?;
        let status = resp.status().as_u16();
        let body = resp.text().await.map_err(|_| {
            ProviderError::new(
                ErrorKind::MalformedFrame,
                "aca data-plane response body was not readable",
            )
        })?;
        Ok(HttpResponse { status, body })
    }
}

// A reqwest error's Display can include the full URL; keep only the coarse
// classification so an endpoint/token never reaches a log.
fn redact_reqwest_error(err: &reqwest::Error) -> &'static str {
    if err.is_timeout() {
        "timeout"
    } else if err.is_connect() {
        "connect"
    } else if err.is_request() {
        "request"
    } else if err.is_body() || err.is_decode() {
        "body"
    } else {
        "transport"
    }
}

fn rest_error(context: &str, status: u16, body: &str) -> ProviderError {
    ProviderError::new(
        ErrorKind::ProviderAllocationFailed,
        format!(
            "{context} returned HTTP {status}{}",
            rest_error_detail(body)
        ),
    )
}

fn rest_error_detail(body: &str) -> String {
    let Some(message) = extract_error_message(body) else {
        return String::new();
    };
    format!(": {}", bounded_detail(&message))
}

fn extract_error_message(body: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    for path in [
        &["error", "message"][..],
        &["error", "details"][..],
        &["message"][..],
        &["detail"][..],
    ] {
        let mut cursor = &value;
        for key in path {
            cursor = cursor.get(*key)?;
        }
        if let Some(s) = cursor.as_str().filter(|s| !s.trim().is_empty()) {
            return Some(s.to_owned());
        }
    }
    None
}

fn bounded_detail(raw: &str) -> String {
    let mut out: String = raw.chars().filter(|c| !c.is_control()).take(240).collect();
    if raw.chars().count() > out.chars().count() {
        out.push_str("...");
    }
    out
}

fn managed_identity_env_present() -> bool {
    managed_identity_env_present_with(|key| std::env::var_os(key).is_some())
}

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
}

#[derive(Debug)]
struct AcaDefaultCredential {
    managed_identity: Option<Arc<dyn TokenCredential>>,
    azure_cli: Arc<dyn TokenCredential>,
}

#[derive(Debug, Clone)]
struct CachedBearer {
    token: String,
    expires_on: OffsetDateTime,
}

#[async_trait]
impl TokenCredential for AcaDefaultCredential {
    async fn get_token(
        &self,
        scopes: &[&str],
        options: Option<TokenRequestOptions<'_>>,
    ) -> azure_core::Result<AccessToken> {
        if let Some(managed_identity) = &self.managed_identity {
            match managed_identity.get_token(scopes, options.clone()).await {
                Ok(token) => return Ok(token),
                Err(err) => {
                    tracing::warn!(error = %err, "managed identity credential failed; falling back to Azure CLI");
                }
            }
        }
        self.azure_cli.get_token(scopes, options).await
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
    /// Build a provider that authenticates with managed identity first, then
    /// Azure CLI for developer/operator sessions, and talks to the ADC data
    /// plane over `reqwest`.
    pub fn new(config: AcaConfig, node: NodeId) -> ProviderResult<Self> {
        let managed_identity = if managed_identity_env_present() {
            Some(
                azure_identity::ManagedIdentityCredential::new(None).map_err(|err| {
                    ProviderError::new(
                        ErrorKind::AuthenticationFailed,
                        format!("managed identity credential unavailable: {err}"),
                    )
                })? as Arc<dyn TokenCredential>,
            )
        } else {
            None
        };
        let azure_cli = azure_identity::AzureCliCredential::new(None).map_err(|err| {
            ProviderError::new(
                ErrorKind::AuthenticationFailed,
                format!("azure cli credential unavailable: {err}"),
            )
        })?;
        let credential = Arc::new(AcaDefaultCredential {
            managed_identity,
            azure_cli,
        });
        let http = Arc::new(ReqwestTransport::new()?);
        Ok(Self::with_parts(config, node, credential, http))
    }

    /// Build a provider from injected parts (credential + transport) — used by
    /// tests and by alternative wirings.
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
        }
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
            .get_token(&[ADC_RESOURCE_SCOPE], None)
            .await
            .map_err(|err| {
                ProviderError::new(
                    ErrorKind::AuthenticationFailed,
                    format!("failed to acquire ADC access token: {err}"),
                )
            })?;
        let bearer = token.token.secret().to_owned();
        *self.token_cache.lock().await = Some(CachedBearer {
            token: bearer.clone(),
            expires_on: token.expires_on,
        });
        Ok(bearer)
    }

    /// Run a shell command in `sandbox_id` and return its result
    /// synchronously. An ACA sandbox auto-suspends to `Idle`; if the exec is
    /// refused with a 409 the provider resumes the sandbox once and retries
    /// (mirroring the data plane's own resume protocol). Fails closed on any
    /// other non-200 status or an unparseable body.
    pub async fn exec_shell(&self, sandbox_id: &str, command: &str) -> ProviderResult<ExecResult> {
        let bearer = self.bearer().await?;
        let url = self.config.exec_url(sandbox_id);
        let body = serde_json::json!({ "command": command }).to_string();

        let resp = self
            .http
            .request(HttpMethod::Post, &url, &bearer, Some(body.clone()))
            .await?;
        let resp = if resp.status == 409 {
            // Sandbox is suspended: resume, then retry the exec exactly once.
            self.resume(sandbox_id, &bearer).await?;
            self.http
                .request(HttpMethod::Post, &url, &bearer, Some(body))
                .await?
        } else {
            resp
        };

        if resp.status != 200 {
            return Err(rest_error(
                "aca executeShellCommand",
                resp.status,
                &resp.body,
            ));
        }
        serde_json::from_str::<ExecResult>(&resp.body).map_err(|_| {
            ProviderError::new(
                ErrorKind::MalformedFrame,
                "aca executeShellCommand response was not the expected JSON shape",
            )
        })
    }

    /// Resume a suspended sandbox (moves `Idle` → `Running`). Fail-closed on a
    /// non-200.
    async fn resume(&self, sandbox_id: &str, bearer: &str) -> ProviderResult<()> {
        let url = self.config.resume_url(sandbox_id);
        let resp = self
            .http
            .request(HttpMethod::Post, &url, bearer, Some("{}".to_owned()))
            .await?;
        if resp.status != 200 {
            return Err(rest_error("aca sandbox resume", resp.status, &resp.body));
        }
        Ok(())
    }

    /// Whether a sandbox exists / is reachable (a 200 GET). Fail-closed on any
    /// non-200.
    pub async fn sandbox_reachable(&self, sandbox_id: &str) -> ProviderResult<bool> {
        let bearer = self.bearer().await?;
        let url = self.config.get_url(sandbox_id);
        let resp = self
            .http
            .request(HttpMethod::Get, &url, &bearer, None)
            .await?;
        Ok(resp.status == 200)
    }

    /// Find the sandbox backing a workload alias via the deterministic
    /// `nixling-workload=<alias>` ACA label plus configured realm/default labels
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
        Ok(sandboxes.into_iter().next())
    }

    /// Ensure the workload alias has a sandbox, creating the disk image and
    /// sandbox through the preview REST data plane when necessary.
    pub async fn ensure_workload_sandbox(
        &self,
        workload: &WorkloadId,
    ) -> ProviderResult<AcaSandbox> {
        let _guard = self.lifecycle_lock.lock().await;
        let defaults = self.sandbox_defaults.as_ref().ok_or_else(|| {
            ProviderError::unsupported(
                "aca sandbox lifecycle requires configured disk/image defaults",
            )
        })?;
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
            "aca sandbox did not reach Running before the readiness deadline",
        ))
    }

    /// List sandboxes, optionally filtered by an ACA label selector.
    pub async fn list_sandboxes(&self, labels: Option<&str>) -> ProviderResult<Vec<AcaSandbox>> {
        let bearer = self.bearer().await?;
        let url = self.config.list_sandboxes_url(labels);
        let resp = self
            .http
            .request(HttpMethod::Get, &url, &bearer, None)
            .await?;
        if resp.status != 200 {
            return Err(rest_error("aca sandbox list", resp.status, &resp.body));
        }
        parse_sandbox_list(&resp.body)
    }

    /// Stop a sandbox by provider id.
    pub async fn stop_sandbox(&self, sandbox_id: &str) -> ProviderResult<()> {
        let bearer = self.bearer().await?;
        let url = self.config.stop_url(sandbox_id);
        let resp = self
            .http
            .request(HttpMethod::Post, &url, &bearer, Some("{}".to_owned()))
            .await?;
        if is_success_no_body_ok(resp.status) {
            Ok(())
        } else {
            Err(rest_error("aca sandbox stop", resp.status, &resp.body))
        }
    }

    /// Delete a sandbox by provider id. Exposed for cleanup/live smoke paths;
    /// `WorkloadProvider::stop` uses the less-destructive stop operation.
    pub async fn delete_sandbox(&self, sandbox_id: &str) -> ProviderResult<()> {
        let bearer = self.bearer().await?;
        let url = self.config.delete_url(sandbox_id);
        let resp = self
            .http
            .request(HttpMethod::Delete, &url, &bearer, None)
            .await?;
        if is_success_no_body_ok(resp.status) || resp.status == 404 {
            Ok(())
        } else {
            Err(rest_error("aca sandbox delete", resp.status, &resp.body))
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
                if let Some(existing) = self.find_disk_image_by_name(name).await? {
                    return Ok(existing.id);
                }
                self.create_disk_image(
                    workload,
                    image,
                    name,
                    managed_identity_resource_id.as_deref(),
                    labels,
                )
                .await
            }
        }
    }

    async fn find_disk_image_by_name(&self, name: &str) -> ProviderResult<Option<AcaDiskImage>> {
        let labels = labels_selector(&BTreeMap::from([("name".to_owned(), name.to_owned())]));
        let bearer = self.bearer().await?;
        let url = self.config.list_disk_images_url(Some(&labels));
        let resp = self
            .http
            .request(HttpMethod::Get, &url, &bearer, None)
            .await?;
        if resp.status != 200 {
            return Err(rest_error("aca disk image list", resp.status, &resp.body));
        }
        Ok(parse_disk_image_list(&resp.body)?.into_iter().next())
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
            .http
            .request(HttpMethod::Put, &url, &bearer, Some(body))
            .await?;
        if !is_success_with_body_ok(resp.status) {
            return Err(rest_error("aca disk image create", resp.status, &resp.body));
        }
        resource_id_from_body(&resp.body).ok_or_else(|| {
            ProviderError::new(
                ErrorKind::MalformedFrame,
                "aca disk image create response did not contain an id",
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
            .http
            .request(HttpMethod::Put, &url, &bearer, Some(body))
            .await?;
        if !is_success_with_body_ok(resp.status) {
            return Err(rest_error("aca sandbox create", resp.status, &resp.body));
        }
        sandbox_from_value(parse_json_value(&resp.body)?).ok_or_else(|| {
            ProviderError::new(
                ErrorKind::MalformedFrame,
                "aca sandbox create response did not contain an id",
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
        let mut caps = CapabilitySet::empty().with(Capability::Exec);
        if self.sandbox_defaults.is_some() {
            caps = caps.with(Capability::Lifecycle);
        }
        WorkloadCapabilitySet { caps }
    }

    async fn list(&self, selector: ListSelector) -> ProviderResult<Vec<WorkloadSummary>> {
        let label_selector = match &selector {
            ListSelector::All => None,
            ListSelector::One(workload) => Some(labels_selector(&BTreeMap::from([(
                "nixling-workload".to_owned(),
                workload.as_str().to_owned(),
            )]))),
        };
        let sandboxes = self.list_sandboxes(label_selector.as_deref()).await?;
        Ok(sandboxes
            .into_iter()
            .filter_map(|sandbox| {
                let workload = sandbox.labels.get("nixling-workload")?;
                let id = WorkloadId::parse(workload.clone()).ok()?;
                Some(WorkloadSummary {
                    id,
                    realm: RealmPath::local(),
                    node: self.node.clone(),
                    state: sandbox_state(&sandbox),
                    capabilities: self.capabilities().caps,
                })
            })
            .collect())
    }

    async fn create(&self, spec: WorkloadSpec) -> ProviderResult<WorkloadId> {
        self.ensure_workload_sandbox(&spec.alias).await?;
        Ok(spec.alias)
    }

    async fn start(&self, id: WorkloadId) -> ProviderResult<WorkloadStatus> {
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
        let command = std::str::from_utf8(req.command.as_bytes()).map_err(|_| {
            ProviderError::new(
                ErrorKind::MalformedFrame,
                "aca exec command payload was not valid UTF-8",
            )
        })?;
        let result = self.exec_shell(req.workload.as_str(), command).await?;
        if result.exit_code != 0 {
            return Err(ProviderError::new(
                ErrorKind::ProviderAllocationFailed,
                format!("aca exec exited with status {}", result.exit_code),
            ));
        }
        // The synchronous data-plane exec has no durable execution id. Derive a
        // stable opaque id from the authorized request shape rather than the
        // response timing so retries of the same request correlate and two
        // equal-duration calls cannot collide.
        ExecutionId::parse(format!("aca-exec-{}", exec_request_digest(&req))).map_err(|_| {
            ProviderError::new(ErrorKind::MalformedFrame, "failed to mint aca execution id")
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

fn parse_json_value(body: &str) -> ProviderResult<serde_json::Value> {
    serde_json::from_str(body).map_err(|_| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            "aca data-plane response was not the expected JSON shape",
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
        "aca list response was not an array",
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
                    "aca sandbox list response contained an item without an id",
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
                    "aca disk image list response contained an item without an id",
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
            "failed to serialize aca disk image create body",
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
            "failed to serialize aca sandbox create body",
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
            subscription: "24f3458d-0000-0000-0000-000000000000".into(),
            resource_group: "rg-nixling-centralus".into(),
            sandbox_group: "casbx-nixling-test".into(),
            region: "centralus".into(),
            endpoint: None,
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

    #[derive(Debug)]
    struct FakeHttp {
        responses: Mutex<std::collections::VecDeque<(u16, String)>>,
        calls: Mutex<Vec<(HttpMethod, String, Option<String>)>>,
    }

    impl FakeHttp {
        fn new(responses: Vec<(u16, &str)>) -> Self {
            Self {
                responses: Mutex::new(
                    responses
                        .into_iter()
                        .map(|(s, b)| (s, b.to_owned()))
                        .collect(),
                ),
                calls: Mutex::new(Vec::new()),
            }
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
            self.calls
                .lock()
                .unwrap()
                .push((method, url.to_owned(), body));
            let (status, body) = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("no canned response left for request");
            Ok(HttpResponse { status, body })
        }
    }

    fn provider(status: u16, body: &str) -> (AcaWorkloadProvider, Arc<FakeHttp>) {
        provider_seq(vec![(status, body)])
    }

    fn provider_seq(responses: Vec<(u16, &str)>) -> (AcaWorkloadProvider, Arc<FakeHttp>) {
        let http = Arc::new(FakeHttp::new(responses));
        let provider = AcaWorkloadProvider::with_parts(
            cfg(),
            NodeId::parse("gw").unwrap(),
            Arc::new(FakeCredential),
            http.clone(),
        );
        (provider, http)
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
                    "/subscriptions/24f3458d-0000-0000-0000-000000000000/resourceGroups/rg/providers/Microsoft.ManagedIdentity/userAssignedIdentities/id".to_owned(),
                ),
                labels: disk_labels,
            },
            cpu: "1000m".to_owned(),
            memory: "2048Mi".to_owned(),
            auto_suspend_interval_secs: 600,
            managed_identity_resource_id: Some(
                "/subscriptions/24f3458d-0000-0000-0000-000000000000/resourceGroups/rg/providers/Microsoft.ManagedIdentity/userAssignedIdentities/id".to_owned(),
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
            "https://management.centralus.azuredevcompute.io/subscriptions/\
             24f3458d-0000-0000-0000-000000000000/resourceGroups/rg-nixling-centralus/\
             sandboxGroups/casbx-nixling-test/sandboxes/sbx-1/executeShellCommand\
             ?api-version=2026-02-01-preview"
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
            endpoint: Some("https://privatelink.example.invalid".to_owned()),
            ..c.clone()
        };
        assert!(
            override_endpoint
                .sandboxes_url()
                .starts_with("https://privatelink.example.invalid/subscriptions/")
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
        assert_eq!(err.kind(), ErrorKind::ProviderAllocationFailed);
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
        let (p, _) = provider(200, "{}");
        let caps = p.capabilities();
        assert!(caps.caps.has(Capability::Exec));
        assert!(!caps.caps.has(Capability::Lifecycle));

        let (p, _) = lifecycle_provider_seq(vec![(200, "[]")]);
        let caps = p.capabilities();
        assert!(caps.caps.has(Capability::Exec));
        assert!(caps.caps.has(Capability::Lifecycle));
    }

    #[test]
    fn local_environment_uses_azure_cli_without_managed_identity_probe() {
        assert!(!managed_identity_env_present_with(|_| false));
        assert!(managed_identity_env_present_with(
            |key| key == "IDENTITY_ENDPOINT"
        ));
        assert!(managed_identity_env_present_with(
            |key| key == "AZURE_CLIENT_ID"
        ));
    }

    #[tokio::test]
    async fn lifecycle_ops_fail_closed_without_defaults() {
        let (p, _) = provider(200, "{}");
        assert_eq!(
            p.create(WorkloadSpec {
                alias: WorkloadId::parse("x").unwrap()
            })
            .await
            .unwrap_err()
            .kind(),
            ErrorKind::UnsupportedFeature
        );
    }

    #[tokio::test]
    async fn create_sandbox_uses_rest_disk_and_sandbox_contract() {
        let (p, http) = lifecycle_provider_seq(vec![
            (200, "[]"),
            (200, "[]"),
            (
                201,
                r#"{"id":"disk-1","labels":{"name":"nixling-wayland-mi"}}"#,
            ),
            (
                201,
                r#"{"id":"sandbox-1","state":"Running","labels":{"nixling-workload":"demo","nixling-realm":"work"}}"#,
            ),
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
        assert!(calls[1].1.contains(
            "/diskimages?api-version=2026-02-01-preview&labels=name%3Dnixling-wayland-mi"
        ));
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
        let (p, http) = lifecycle_provider_seq(vec![
            (
                200,
                r#"[{"id":"sandbox-1","state":"Idle","labels":{"nixling-workload":"demo"}}]"#,
            ),
            (200, "{}"),
            (
                200,
                r#"[{"id":"sandbox-1","state":"Running","labels":{"nixling-workload":"demo"}}]"#,
            ),
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
        let (p, _) = lifecycle_provider_seq(vec![(
            403,
            r#"{"error":{"message":"quota denied for sandbox group"}}"#,
        )]);
        let err = p.list(ListSelector::All).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ProviderAllocationFailed);
        assert!(err.to_string().contains("quota denied"));
    }

    #[tokio::test]
    async fn lifecycle_start_fails_closed_on_resume_error() {
        let (p, _) = lifecycle_provider_seq(vec![
            (
                200,
                r#"[{"id":"sandbox-1","state":"Idle","labels":{"nixling-workload":"demo"}}]"#,
            ),
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
        let (p, _) = lifecycle_provider_seq(vec![
            (
                200,
                r#"[{"id":"sandbox-1","state":"Running","labels":{"nixling-workload":"demo"}}]"#,
            ),
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
        let (p, http) = lifecycle_provider_seq(vec![
            (
                200,
                r#"[{"id":"sandbox-1","state":"Running","labels":{"nixling-workload":"demo"}}]"#,
            ),
            (202, ""),
        ]);
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
    async fn list_maps_aca_labels_to_workload_summaries() {
        let (p, _) = lifecycle_provider_seq(vec![(
            200,
            r#"[{"id":"sandbox-1","state":"Running","labels":{"nixling-workload":"demo"}},{"id":"sandbox-2","state":"Failed","labels":{"other":"ignored"}}]"#,
        )]);
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
