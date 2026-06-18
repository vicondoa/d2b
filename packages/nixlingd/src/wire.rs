use crate::typed_error::{ErrorEnvelope, TypedError};
use nixling_core::host::IfName;
use nixling_ipc::{
    FeatureFlag, Hello, HelloOk, HelloRejected, HelloRejectedReason, Version,
    broker_wire::ExportBrokerAuditResponse,
    public_wire::{self, AuthStatusResponse},
};
use semver::{Version as SemverVersion, VersionReq};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use nixling_ipc::MAX_FRAME_SIZE;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HostCheckRequestExt {
    #[serde(flatten)]
    pub request: public_wire::HostCheckRequest,
    pub if_name: Option<IfName>,
}

#[derive(Debug, Clone)]
pub enum Request {
    List(public_wire::ListRequest),
    Status(public_wire::StatusRequest),
    Audit(public_wire::AuditRequest),
    HostCheck(HostCheckRequestExt),
    AuthStatus,
    KeysList,
    KeysShow(public_wire::KeysShowRequest),
    // Mutating-verb dispatch entry points. Each variant carries
    // its public_wire request payload verbatim; `mutating_verb_preflight`
    // emits the typed dry-run/invalid-request envelope and apply
    // requests dispatch directly to the matching
    // `dispatch_broker_<verb>` helper.
    VmStart(public_wire::VmLifecycleRequest),
    VmStop(public_wire::VmLifecycleRequest),
    VmRestart(public_wire::VmLifecycleRequest),
    Switch(public_wire::ActivationRequest),
    Boot(public_wire::ActivationRequest),
    Test(public_wire::ActivationRequest),
    Rollback(public_wire::ActivationRequest),
    Gc(public_wire::GcRequest),
    KeysRotate(public_wire::KeysRotateRequest),
    Trust(public_wire::TrustRequest),
    RotateKnownHost(public_wire::RotateKnownHostRequest),
    UsbipBind(public_wire::UsbipBindCliRequest),
    UsbipUnbind(public_wire::UsbipUnbindCliRequest),
    UsbipProbe,
    StoreVerify(public_wire::StoreVerifyRequest),
    Migrate(public_wire::MigrateRequest),
    HostPrepare(public_wire::HostPrepareRequest),
    HostDestroy(public_wire::HostDestroyRequest),
    HostInstall(public_wire::HostInstallRequest),
    HostReconcile(public_wire::HostReconcileRequest),
    ReadGuestConfig(public_wire::ReadGuestConfigRequest),
    Exec(public_wire::ExecOp),
}

impl Request {
    pub fn verb_name(&self) -> &'static str {
        match self {
            Self::List(_) => "list",
            Self::Status(_) => "status",
            Self::Audit(_) => "audit",
            Self::HostCheck(_) => "hostCheck",
            Self::AuthStatus => "authStatus",
            Self::KeysList => "keysList",
            Self::KeysShow(_) => "keysShow",
            Self::VmStart(_) => "vmStart",
            Self::VmStop(_) => "vmStop",
            Self::VmRestart(_) => "vmRestart",
            Self::Switch(_) => "switch",
            Self::Boot(_) => "boot",
            Self::Test(_) => "test",
            Self::Rollback(_) => "rollback",
            Self::Gc(_) => "gc",
            Self::KeysRotate(_) => "keysRotate",
            Self::Trust(_) => "trust",
            Self::RotateKnownHost(_) => "rotateKnownHost",
            Self::UsbipBind(_) => "usbipBind",
            Self::UsbipUnbind(_) => "usbipUnbind",
            Self::UsbipProbe => "usbipProbe",
            Self::StoreVerify(_) => "storeVerify",
            Self::Migrate(_) => "migrate",
            Self::HostPrepare(_) => "hostPrepare",
            Self::HostDestroy(_) => "hostDestroy",
            Self::HostInstall(_) => "hostInstall",
            Self::HostReconcile(_) => "hostReconcile",
            Self::ReadGuestConfig(_) => "readGuestConfig",
            Self::Exec(_) => "exec",
        }
    }

    /// Concurrency lock class for this request. Read-only verbs take no
    /// lock and run fully in parallel; per-VM mutating verbs serialize on
    /// the named VM; global mutating verbs are mutually exclusive with
    /// every per-VM op. The accept loop never holds this lock — it is
    /// acquired on the worker thread inside `dispatch_request`.
    pub fn lock_class(&self) -> crate::concurrency::OpLockClass {
        use crate::concurrency::OpLockClass;
        match self {
            // Per-VM mutating lifecycle verbs (all carry a `vm` field).
            Self::VmStart(req) | Self::VmStop(req) | Self::VmRestart(req) => {
                OpLockClass::PerVm(req.vm.clone())
            }
            Self::Switch(req) | Self::Boot(req) | Self::Test(req) | Self::Rollback(req) => {
                OpLockClass::PerVm(req.vm.clone())
            }
            Self::UsbipBind(req) => OpLockClass::PerVm(req.vm.clone()),
            Self::UsbipUnbind(req) => OpLockClass::PerVm(req.vm.clone()),
            Self::StoreVerify(req) => OpLockClass::PerVm(req.vm.clone()),
            Self::RotateKnownHost(req) => OpLockClass::PerVm(req.vm.clone()),
            // Global mutating verbs: mutually exclusive with all per-VM ops.
            Self::Gc(_)
            | Self::KeysRotate(_)
            | Self::Trust(_)
            | Self::Migrate(_)
            | Self::HostPrepare(_)
            | Self::HostDestroy(_)
            | Self::HostInstall(_)
            | Self::HostReconcile(_) => OpLockClass::Global,
            // Read-only / status / session-managed verbs: no lock.
            Self::List(_)
            | Self::Status(_)
            | Self::Audit(_)
            | Self::HostCheck(_)
            | Self::AuthStatus
            | Self::KeysList
            | Self::KeysShow(_)
            | Self::UsbipProbe
            | Self::ReadGuestConfig(_)
            | Self::Exec(_) => OpLockClass::ReadOnly,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloOkFrame {
    #[serde(rename = "type")]
    pub type_name: &'static str,
    #[serde(flatten)]
    pub payload: HelloOk,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelloRejectedFrame {
    #[serde(rename = "type")]
    pub type_name: &'static str,
    #[serde(flatten)]
    pub payload: HelloRejected,
    pub error: ErrorEnvelope,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorFrame {
    #[serde(rename = "type")]
    pub type_name: &'static str,
    pub error: ErrorEnvelope,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditResponseFrame {
    #[serde(rename = "type")]
    pub type_name: &'static str,
    #[serde(flatten)]
    pub payload: ExportBrokerAuditResponse,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatusResponseFrame {
    #[serde(rename = "type")]
    pub type_name: &'static str,
    pub auth: AuthStatusResponse,
}

pub fn parse_hello(bytes: &[u8]) -> Result<Hello, TypedError> {
    let mut value: Value =
        serde_json::from_slice(bytes).map_err(|err| TypedError::WireBadHello {
            detail: err.to_string(),
        })?;
    let kind =
        value
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| TypedError::WireBadHello {
                detail: "missing type=hello discriminator".to_owned(),
            })?;
    if kind != "hello" {
        return Err(TypedError::WireBadHello {
            detail: format!("expected type=hello, got {kind}"),
        });
    }
    value
        .as_object_mut()
        .ok_or_else(|| TypedError::WireBadHello {
            detail: "hello frame must be a JSON object".to_owned(),
        })?
        .remove("type");
    serde_json::from_value(value).map_err(map_parse_error)
}

pub fn parse_request(bytes: &[u8]) -> Result<Request, TypedError> {
    let mut value: Value =
        serde_json::from_slice(bytes).map_err(|err| TypedError::WireInvalidFrame {
            detail: err.to_string(),
        })?;
    let request_type = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| TypedError::WireInvalidFrame {
            detail: "missing request type".to_owned(),
        })?
        .to_owned();
    let object = value
        .as_object_mut()
        .ok_or_else(|| TypedError::WireInvalidFrame {
            detail: "request frame must be a JSON object".to_owned(),
        })?;
    object.remove("type");
    match request_type.as_str() {
        "list" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::List)
            .map_err(map_parse_error),
        "status" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::Status)
            .map_err(map_parse_error),
        "audit" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::Audit)
            .map_err(map_parse_error),
        "hostCheck" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::HostCheck)
            .map_err(map_parse_error),
        "authStatus" => {
            if object.is_empty() {
                Ok(Request::AuthStatus)
            } else {
                Err(TypedError::WireUnknownField {
                    detail: format!("authStatus request must not contain extra fields: {object:?}"),
                })
            }
        }
        "keysList" => {
            if object.is_empty() {
                Ok(Request::KeysList)
            } else {
                Err(TypedError::WireUnknownField {
                    detail: format!("keysList request must not contain extra fields: {object:?}"),
                })
            }
        }
        "keysShow" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::KeysShow)
            .map_err(map_parse_error),
        "vmStart" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::VmStart)
            .map_err(map_parse_error),
        "vmStop" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::VmStop)
            .map_err(map_parse_error),
        "vmRestart" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::VmRestart)
            .map_err(map_parse_error),
        "switch" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::Switch)
            .map_err(map_parse_error),
        "boot" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::Boot)
            .map_err(map_parse_error),
        "test" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::Test)
            .map_err(map_parse_error),
        "rollback" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::Rollback)
            .map_err(map_parse_error),
        "gc" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::Gc)
            .map_err(map_parse_error),
        "keysRotate" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::KeysRotate)
            .map_err(map_parse_error),
        "trust" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::Trust)
            .map_err(map_parse_error),
        "rotateKnownHost" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::RotateKnownHost)
            .map_err(map_parse_error),
        "usbipBind" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::UsbipBind)
            .map_err(map_parse_error),
        "usbipUnbind" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::UsbipUnbind)
            .map_err(map_parse_error),
        "usbipProbe" => {
            if object.is_empty() {
                Ok(Request::UsbipProbe)
            } else {
                Err(TypedError::WireUnknownField {
                    detail: format!("usbipProbe request must not contain extra fields: {object:?}"),
                })
            }
        }
        "storeVerify" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::StoreVerify)
            .map_err(map_parse_error),
        "migrate" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::Migrate)
            .map_err(map_parse_error),
        "hostPrepare" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::HostPrepare)
            .map_err(map_parse_error),
        "hostDestroy" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::HostDestroy)
            .map_err(map_parse_error),
        "hostInstall" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::HostInstall)
            .map_err(map_parse_error),
        "hostReconcile" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::HostReconcile)
            .map_err(map_parse_error),
        "readGuestConfig" => serde_json::from_value(Value::Object(object.clone()))
            .map(Request::ReadGuestConfig)
            .map_err(map_parse_error),
        "exec" => {
            // `opId` is an envelope-level correlation id; it is not a
            // field of the adjacently-tagged `ExecOp`, so strip it before the
            // closed-enum deserialize.
            object.remove("opId");
            serde_json::from_value(Value::Object(object.clone()))
                .map(Request::Exec)
                .map_err(map_parse_error)
        }
        _ => Err(TypedError::WireUnsupportedRequest { request_type }),
    }
}

/// Extract the envelope-level `opId` from an exec request frame, defaulting to
/// `0` when absent. The owner connection echoes this id on the matching
/// response so a long-poll reply and an urgent control reply can be correlated
/// out of order without the CLI mismatching frames.
pub fn exec_op_id(bytes: &[u8]) -> u64 {
    serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| value.get("opId").and_then(Value::as_u64))
        .unwrap_or(0)
}

/// Parse an exec op frame into its correlating `opId` and the multiplexed op.
/// Used by the owner reader so it can dispatch each op to the session worker
/// without blocking on the previous op's reply.
pub fn parse_exec_op(bytes: &[u8]) -> Result<(u64, public_wire::ExecOp), TypedError> {
    let mut value: Value =
        serde_json::from_slice(bytes).map_err(|err| TypedError::WireInvalidFrame {
            detail: err.to_string(),
        })?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| TypedError::WireInvalidFrame {
            detail: "request frame must be a JSON object".to_owned(),
        })?;
    let request_type = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| TypedError::WireInvalidFrame {
            detail: "missing request type".to_owned(),
        })?
        .to_owned();
    if request_type != "exec" {
        return Err(TypedError::WireUnsupportedRequest { request_type });
    }
    object.remove("type");
    let op_id = object
        .remove("opId")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let op = serde_json::from_value(Value::Object(object.clone())).map_err(map_parse_error)?;
    Ok((op_id, op))
}

pub fn negotiate_version(
    client_range: &str,
    accepted_range: &str,
    server_version: &str,
) -> Result<String, TypedError> {
    let client_req =
        VersionReq::parse(client_range).map_err(|err| TypedError::WireVersionMismatch {
            client_range: client_range.to_owned(),
            accepted_range: format!("{accepted_range} ({err})"),
        })?;
    let accepted_req =
        VersionReq::parse(accepted_range).map_err(|err| TypedError::InternalConfig {
            detail: format!("bad acceptedClientVersionRange {accepted_range}: {err}"),
        })?;
    let server =
        SemverVersion::parse(server_version).map_err(|err| TypedError::InternalConfig {
            detail: format!("bad serverVersion {server_version}: {err}"),
        })?;
    if client_req.matches(&server) && accepted_req.matches(&server) {
        Ok(server.to_string())
    } else {
        Err(TypedError::WireVersionMismatch {
            client_range: client_range.to_owned(),
            accepted_range: accepted_range.to_owned(),
        })
    }
}

pub fn hello_ok(
    server_version: &str,
    selected_version: &str,
    capabilities: &[FeatureFlag],
) -> Result<HelloOkFrame, TypedError> {
    Ok(HelloOkFrame {
        type_name: "helloOk",
        payload: HelloOk {
            server_version: Version::new(server_version).map_err(|err| {
                TypedError::InternalConfig {
                    detail: format!("bad serverVersion {server_version}: {err}"),
                }
            })?,
            selected_version: Version::new(selected_version).map_err(|err| {
                TypedError::InternalConfig {
                    detail: format!("bad selectedVersion {selected_version}: {err}"),
                }
            })?,
            capabilities: capabilities.to_vec(),
        },
    })
}

pub fn hello_rejected(error: &TypedError) -> HelloRejectedFrame {
    HelloRejectedFrame {
        type_name: "helloRejected",
        payload: HelloRejected {
            reason: hello_rejected_reason(error),
        },
        error: error.to_envelope(),
    }
}

pub fn error_frame(error: &TypedError) -> ErrorFrame {
    ErrorFrame {
        type_name: "error",
        error: error.to_envelope(),
    }
}

pub fn list_response(vms: Vec<Value>) -> Value {
    json!({ "type": "listResponse", "vms": vms })
}

pub fn status_response(status: Value) -> Value {
    json!({ "type": "statusResponse", "status": status })
}

pub fn audit_response(lines: Vec<String>) -> AuditResponseFrame {
    AuditResponseFrame {
        type_name: "auditResponse",
        payload: ExportBrokerAuditResponse { lines },
    }
}

pub fn host_check_response(summary: Value, checks: Vec<Value>) -> Value {
    json!({ "type": "hostCheckResponse", "summary": summary, "checks": checks })
}

pub fn keys_list_response(payload: public_wire::KeysListResponse) -> Value {
    let mut value = serde_json::to_value(&payload).unwrap_or_else(|_| json!({}));
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "type".to_owned(),
            Value::String("keysListResponse".to_owned()),
        );
    }
    value
}

pub fn keys_show_response(payload: public_wire::KeysShowResponse) -> Value {
    let mut value = serde_json::to_value(&payload).unwrap_or_else(|_| json!({}));
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "type".to_owned(),
            Value::String("keysShowResponse".to_owned()),
        );
    }
    value
}

pub fn usbip_probe_response(payload: public_wire::UsbipProbeResponse) -> Value {
    let mut value = serde_json::to_value(&payload).unwrap_or_else(|_| json!({}));
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "type".to_owned(),
            Value::String("usbipProbeResponse".to_owned()),
        );
    }
    value
}

pub fn store_verify_response(payload: public_wire::StoreVerifyResponse) -> Value {
    let mut value = serde_json::to_value(&payload).unwrap_or_else(|_| json!({}));
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "type".to_owned(),
            Value::String("storeVerifyResponse".to_owned()),
        );
    }
    value
}

/// Serialize a `MutatingVerbResponse` as the daemon wire frame
/// the CLI client expects.
pub fn mutating_verb_response(payload: public_wire::MutatingVerbResponse) -> Value {
    let mut value = serde_json::to_value(&payload).unwrap_or_else(|_| json!({}));
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "type".to_owned(),
            Value::String("mutatingVerbResponse".to_owned()),
        );
    }
    value
}

/// Serialize a `ReadGuestConfigResponse` as the daemon wire frame the CLI
/// `config sync` client expects. The `contentBase64` field is the standard
/// padded base64 of the raw guest config bytes.
pub fn read_guest_config_response(payload: public_wire::ReadGuestConfigResponse) -> Value {
    let mut value = serde_json::to_value(&payload).unwrap_or_else(|_| json!({}));
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "type".to_owned(),
            Value::String("readGuestConfigResponse".to_owned()),
        );
    }
    value
}

/// Serialize an `ExecOpResponse` as the `execResponse` daemon wire frame the
/// CLI `vm exec` owner connection expects. The adjacently-tagged
/// `{ "op": …, "result": … }` body is preserved and a `type` tag is added.
pub fn exec_response(payload: &public_wire::ExecOpResponse) -> Value {
    let mut value = serde_json::to_value(payload).unwrap_or_else(|_| json!({}));
    if let Some(obj) = value.as_object_mut() {
        obj.insert("type".to_owned(), Value::String("execResponse".to_owned()));
    }
    value
}

/// `execResponse` frame tagged with the correlating envelope `opId`.
pub fn exec_response_with_id(op_id: u64, payload: &public_wire::ExecOpResponse) -> Value {
    let mut value = exec_response(payload);
    if let Some(obj) = value.as_object_mut() {
        obj.insert("opId".to_owned(), Value::from(op_id));
    }
    value
}

/// `error` frame tagged with the correlating envelope `opId` so the owner
/// connection can return an out-of-order op error without the CLI mismatching
/// it against a different in-flight op.
pub fn error_frame_with_id(op_id: u64, error: &TypedError) -> Value {
    let mut value =
        serde_json::to_value(error_frame(error)).unwrap_or_else(|_| json!({ "type": "error" }));
    if let Some(obj) = value.as_object_mut() {
        obj.insert("opId".to_owned(), Value::from(op_id));
    }
    value
}

pub fn auth_status_response(payload: AuthStatusResponse) -> AuthStatusResponseFrame {
    AuthStatusResponseFrame {
        type_name: "authStatusResponse",
        auth: payload,
    }
}

fn hello_rejected_reason(error: &TypedError) -> HelloRejectedReason {
    match error {
        TypedError::WireVersionMismatch { .. } => HelloRejectedReason::VersionMismatch,
        _ => HelloRejectedReason::InternalError,
    }
}

fn map_parse_error(error: serde_json::Error) -> TypedError {
    let detail = error.to_string();
    if detail.contains("unknown field") {
        TypedError::WireUnknownField { detail }
    } else if detail.contains("interface name") {
        TypedError::WireIfNameInvalid { detail }
    } else {
        TypedError::WireInvalidFrame { detail }
    }
}
