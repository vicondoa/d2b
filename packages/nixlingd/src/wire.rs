use crate::typed_error::{ErrorEnvelope, TypedError};
use nixling_core::host::IfName;
use nixling_ipc::{
    broker_wire::ExportBrokerAuditResponse,
    public_wire::{self, AuthStatusResponse},
    FeatureFlag, Hello, HelloOk, HelloRejected, HelloRejectedReason, Version,
};
use semver::{Version as SemverVersion, VersionReq};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
    // W14d: mutating-verb dispatch entry points. Each variant carries
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
        _ => Err(TypedError::WireUnsupportedRequest { request_type }),
    }
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

/// W14b: serialize a `MutatingVerbResponse` as the daemon wire frame
/// the W14c CLI client expects.
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
