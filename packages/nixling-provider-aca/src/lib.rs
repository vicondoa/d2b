//! `nixling-provider-aca`: the Azure Container Apps **sandbox**
//! `WorkloadProvider` (ADR 0032, P0).
//!
//! This productionizes the ACA leg of the P0 vertical: instead of the
//! operator driving the sandbox by hand with the preview `aca` CLI, the
//! gateway drives it through this Rust provider against the ADC data-plane
//! REST surface.
//!
//! ## Three-plane auth (operator directive)
//! Plane 1 — Azure control-plane access — is the operator's **ambient Entra
//! identity via the Azure CLI**, surfaced through
//! [`azure_identity::AzureCliCredential`] (an
//! [`azure_core::credentials::TokenCredential`]). nixling stores **no** Azure
//! secret of its own: the token cache is owned by `az`. The token is acquired
//! for the ADC resource scope and presented as a bearer on each data-plane
//! call. Container→Azure (plane 2, the sandbox Managed Identity) and the
//! nixling-internal per-session credential (plane 3) live in the relay/display
//! providers, not here.
//!
//! ## Data plane
//! `https://management.<region>.azuredevcompute.io/subscriptions/<sub>/
//! resourceGroups/<rg>/sandboxGroups/<sg>/sandboxes/<id>` with
//! `?api-version=2026-02-01-preview`. Exec is a synchronous
//! `POST …/executeShellCommand` with body `{"command": "<sh>"}` returning
//! `{"exitCode","stdout","stderr","executionTimeMs"}`.
//!
//! The credential and the HTTP transport are both injectable traits so the
//! provider is unit-testable without a live subscription; the live path is
//! exercised by an `NL_ACA_LIVE`-gated smoke test.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use azure_core::credentials::TokenCredential;
use serde::Deserialize;

use nixling_constellation_core::WorkloadSummary;
use nixling_constellation_core::{
    Capability, CapabilitySet, ErrorKind, ExecutionId, NodeId, ProviderId, WorkloadId,
};
use nixling_constellation_provider::capabilities::WorkloadCapabilitySet;
use nixling_constellation_provider::error::{ProviderError, ProviderResult};
use nixling_constellation_provider::provider::WorkloadProvider;
use nixling_constellation_provider::types::{
    ExecStartRequest, ListSelector, WorkloadSpec, WorkloadStatus,
};

/// The Entra scope for the ADC data plane (plane 1). `AzureCliCredential`
/// maps this to `az account get-access-token --resource
/// https://management.azuredevcompute.io`.
pub const ADC_RESOURCE_SCOPE: &str = "https://management.azuredevcompute.io/.default";

/// The ADC data-plane API version this provider speaks.
pub const ADC_API_VERSION: &str = "2026-02-01-preview";

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
}

impl AcaConfig {
    /// The region-specific ADC data-plane endpoint.
    pub fn endpoint(&self) -> String {
        format!("https://management.{}.azuredevcompute.io", self.region)
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
    /// POST.
    Post,
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
            HttpMethod::Post => self.client.post(url),
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

/// The Azure Container Apps sandbox `WorkloadProvider`.
pub struct AcaWorkloadProvider {
    config: AcaConfig,
    credential: Arc<dyn TokenCredential>,
    http: Arc<dyn HttpTransport>,
    node: NodeId,
    provider_id: ProviderId,
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
    /// Build a provider that authenticates with the operator's Azure CLI
    /// session (plane 1) and talks to the ADC data plane over `reqwest`.
    pub fn new(config: AcaConfig, node: NodeId) -> ProviderResult<Self> {
        let credential = azure_identity::AzureCliCredential::new(None).map_err(|err| {
            ProviderError::new(
                ErrorKind::AuthenticationFailed,
                format!("azure cli credential unavailable: {err}"),
            )
        })?;
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
        }
    }

    async fn bearer(&self) -> ProviderResult<String> {
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
        Ok(token.token.secret().to_owned())
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
            return Err(ProviderError::new(
                ErrorKind::ProviderAllocationFailed,
                format!("aca executeShellCommand returned HTTP {}", resp.status),
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
            return Err(ProviderError::new(
                ErrorKind::ProviderAllocationFailed,
                format!("aca sandbox resume returned HTTP {}", resp.status),
            ));
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
        // Honest: only exec is wired in this stage. Lifecycle (create/start/
        // stop) lands with the disk/sandbox-management stage.
        WorkloadCapabilitySet {
            caps: CapabilitySet::empty().with(Capability::Exec),
        }
    }

    async fn list(&self, _selector: ListSelector) -> ProviderResult<Vec<WorkloadSummary>> {
        Err(ProviderError::unsupported(
            "aca workload listing is not wired in this P0 stage",
        ))
    }

    async fn create(&self, _spec: WorkloadSpec) -> ProviderResult<WorkloadId> {
        Err(ProviderError::unsupported(
            "aca sandbox creation is not wired in this P0 stage (disk management lands next)",
        ))
    }

    async fn start(&self, _id: WorkloadId) -> ProviderResult<WorkloadStatus> {
        Err(ProviderError::unsupported(
            "aca sandbox start is not wired in this P0 stage",
        ))
    }

    async fn stop(&self, _id: WorkloadId) -> ProviderResult<WorkloadStatus> {
        Err(ProviderError::unsupported(
            "aca sandbox stop is not wired in this P0 stage",
        ))
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
        // The synchronous data-plane exec has no durable execution id; mint a
        // deterministic per-call id so callers can correlate audit records.
        ExecutionId::parse(format!("aca-exec-{}", result.execution_time_ms)).map_err(|_| {
            ProviderError::new(ErrorKind::MalformedFrame, "failed to mint aca execution id")
        })
    }
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

    #[test]
    fn capabilities_are_honest_exec_only() {
        let (p, _) = provider(200, "{}");
        let caps = p.capabilities();
        assert!(caps.caps.has(Capability::Exec));
        assert!(!caps.caps.has(Capability::Lifecycle));
    }

    #[tokio::test]
    async fn lifecycle_ops_fail_closed_until_wired() {
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
