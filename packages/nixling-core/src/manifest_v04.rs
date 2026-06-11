use crate::error::Error;
use schemars::{
    gen::SchemaGenerator,
    schema::{InstanceType, Metadata, ObjectValidation, Schema, SchemaObject, SingleOrVec},
    JsonSchema,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

/// Daemon-only end-state: the single supported `_manifest.manifestVersion`.
///
/// Bumped from `3` to `4` to mark the native SigNoz observability
/// backend: Grafana / Cloud Hypervisor exporter metadata in
/// `_observability` is replaced by SigNoz UI and collector-ingress
/// metadata while the vsock transport contract remains. There is no
/// legacy compatibility window —
/// the broker / daemon refuse to load a bundle whose `vms.json` does
/// not pin this exact integer (`manifest-version-mismatch` typed error).
pub const MANIFEST_VERSION_CURRENT: u32 = 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManifestV04 {
    #[serde(rename = "_manifest")]
    pub manifest: ManifestMeta,
    #[serde(rename = "_observability")]
    pub observability: ObservabilityMeta,
    #[serde(flatten)]
    pub vms: BTreeMap<String, VmEntry>,
}

impl ManifestV04 {
    pub fn from_slice(bytes: &[u8]) -> Result<Self, Error> {
        let parsed: Self = serde_json::from_slice(bytes).map_err(|error| {
            Error::manifest_parse_error("vms.json", manifest_parse_reason(&error.to_string()))
        })?;
        if parsed.manifest.manifest_version != MANIFEST_VERSION_CURRENT {
            return Err(Error::manifest_version_mismatch(
                "vms.json",
                "manifest-version-mismatch",
            ));
        }
        Ok(parsed)
    }

    pub fn from_path(path: &Path) -> Result<Self, Error> {
        let bytes = std::fs::read(path).map_err(|_| Error::internal_io("manifest-v04-read"))?;
        Self::from_slice(&bytes)
    }

    pub fn to_compact_json(&self) -> Result<String, Error> {
        let mut rendered = serde_json::to_string(self)
            .map_err(|_| Error::manifest_parse_error("vms.json", "serialize-failed"))?;
        rendered.push('\n');
        Ok(rendered)
    }
}

impl<'de> Deserialize<'de> for ManifestV04 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut entries = BTreeMap::<String, Value>::deserialize(deserializer)?;
        let manifest_value = entries
            .remove("_manifest")
            .ok_or_else(|| serde::de::Error::missing_field("_manifest"))?;
        let observability_value = entries
            .remove("_observability")
            .ok_or_else(|| serde::de::Error::missing_field("_observability"))?;

        let manifest = serde_json::from_value::<ManifestMeta>(manifest_value)
            .map_err(serde::de::Error::custom)?;
        let observability = serde_json::from_value::<ObservabilityMeta>(observability_value)
            .map_err(serde::de::Error::custom)?;

        let mut vms = BTreeMap::new();
        for (key, value) in entries {
            if key.starts_with('_') || !vm_key_ok(&key) {
                return Err(serde::de::Error::custom(format!(
                    "unknown field `{key}`, expected `_manifest`, `_observability`, or a VM name matching ^[a-z][a-z0-9-]*$"
                )));
            }
            let vm = serde_json::from_value::<VmEntry>(value).map_err(serde::de::Error::custom)?;
            if vm.name != key {
                return Err(serde::de::Error::custom(format!(
                    "vm entry name `{}` does not match object key `{key}`",
                    vm.name
                )));
            }
            vms.insert(key, vm);
        }

        Ok(Self {
            manifest,
            observability,
            vms,
        })
    }
}

impl JsonSchema for ManifestV04 {
    fn schema_name() -> String {
        "ManifestV04".to_owned()
    }

    fn json_schema(gen: &mut SchemaGenerator) -> Schema {
        let mut object = SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Object))),
            ..Default::default()
        };
        let mut validation = ObjectValidation::default();
        validation.required.insert("_manifest".to_owned());
        validation.required.insert("_observability".to_owned());
        validation
            .properties
            .insert("_manifest".to_owned(), gen.subschema_for::<ManifestMeta>());
        validation.properties.insert(
            "_observability".to_owned(),
            gen.subschema_for::<ObservabilityMeta>(),
        );
        validation.pattern_properties.insert(
            "^[a-z][a-z0-9-]*$".to_owned(),
            gen.subschema_for::<VmEntry>(),
        );
        validation.additional_properties = Some(Box::new(Schema::Bool(false)));
        object.object = Some(Box::new(validation));
        object.metadata = Some(Box::new(Metadata {
            description: Some(
                "Typed v0.4.0 public vms.json manifest with reserved sentinels and dynamic VM keys."
                    .to_owned(),
            ),
            ..Default::default()
        }));
        Schema::Object(object)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestMeta {
    pub manifest_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservabilityMeta {
    pub enabled: bool,
    pub obs_vsock_cid: u32,
    pub obs_vsock_host_socket: String,
    pub signoz_otlp_grpc_port: u16,
    pub signoz_otlp_http_port: u16,
    pub signoz_url: String,
    pub vm_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmEntry {
    pub api_socket: String,
    pub audio: bool,
    pub audio_service: String,
    pub audio_state_file: String,
    pub bridge: Option<String>,
    pub env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtu: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mss_clamp: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lan: Option<VmLanPolicy>,
    pub gpu_socket: String,
    pub graphics: bool,
    pub is_net_vm: bool,
    pub name: String,
    pub net_vm: Option<String>,
    pub observability: VmObservability,
    pub ssh_user: Option<String>,
    pub state_dir: String,
    pub static_ip: Option<String>,
    pub tap: String,
    pub tpm: bool,
    pub tpm_socket: String,
    pub usbip_yubikey: bool,
    pub usbipd_host_ip: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmLanPolicy {
    pub allow_east_west: bool,
    pub effective_east_west: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VmObservability {
    pub agent_socket: String,
    pub enabled: bool,
    pub vsock_cid: u32,
    pub vsock_host_socket: String,
}

fn vm_key_ok(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(first) if first.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn manifest_parse_reason(message: &str) -> &'static str {
    if message.contains("unknown field") {
        "unknown-field"
    } else if message.contains("missing field") {
        "missing-field"
    } else if message.contains("does not match object key") {
        "name-key-mismatch"
    } else if message.contains("invalid type") {
        "invalid-type"
    } else if message.contains("expected value") || message.contains("EOF while parsing") {
        "invalid-json"
    } else {
        "parse-failed"
    }
}

#[cfg(test)]
mod tests {
    use super::ManifestV04;

    // Embed the golden fixtures via `include_str!` so the rust-tests
    // sandbox does not need to read outside its `src` set.
    // Historical `vms.json-*` fixtures remain frozen for the
    // vms-json-parity gate. This fixture tracks the current manifest
    // version and is byte-identical with the Nix-rendered smoke manifest.
    const BASELINE_VMS_JSON: &str = include_str!("../../../tests/golden/vms.json-signoz-v4");
    const NETWORKING_FIXTURE: &str =
        include_str!("../../../tests/golden/manifest_v04/baseline-vms.json");

    #[test]
    fn baseline_fixture_round_trips_compact() {
        let manifest =
            ManifestV04::from_slice(BASELINE_VMS_JSON.as_bytes()).expect("baseline parses");
        let rendered = manifest.to_compact_json().expect("baseline serializes");
        assert_eq!(rendered, BASELINE_VMS_JSON);
    }

    #[test]
    fn networking_fixture_round_trips_with_explicit_fields() {
        let manifest = ManifestV04::from_slice(NETWORKING_FIXTURE.as_bytes())
            .expect("networking fixture parses");
        let rendered = manifest
            .to_compact_json()
            .expect("networking fixture serializes");
        assert_eq!(rendered, NETWORKING_FIXTURE);

        let corp_vm = manifest
            .vms
            .get("corp-vm")
            .expect("corp-vm fixture present");
        assert_eq!(corp_vm.mtu, Some(1280));
        assert_eq!(corp_vm.mss_clamp, Some(1240));
        let corp_lan = corp_vm.lan.as_ref().expect("corp-vm lan metadata present");
        assert!(corp_lan.allow_east_west);
        assert!(corp_lan.effective_east_west);

        let net_vm = manifest
            .vms
            .get("sys-work-net")
            .expect("sys-work-net fixture present");
        assert_eq!(net_vm.mtu, Some(1280));
        assert_eq!(net_vm.mss_clamp, Some(1240));
        let net_lan = net_vm
            .lan
            .as_ref()
            .expect("sys-work-net lan metadata present");
        assert!(net_lan.allow_east_west);
        assert!(net_lan.effective_east_west);
    }

    #[test]
    fn unknown_reserved_keys_fail_closed() {
        let error = ManifestV04::from_slice(
            br#"{"_manifest":{"manifestVersion":4},"_observability":{"enabled":false,"vmName":"sys-obs","obsVsockCid":1000,"obsVsockHostSocket":"/var/lib/nixling/vms/sys-obs/vsock.sock","signozUrl":"http://10.40.0.10:8080","signozOtlpGrpcPort":4317,"signozOtlpHttpPort":4318},"_future":{}}"#,
        )
        .expect_err("reserved keys are closed in v0.4.0 parser");
        assert_eq!(error.kind().as_str(), "manifest-parse-error");
        assert!(error.message().contains("opaque reason: unknown-field"));
    }

    #[test]
    fn mismatched_vm_name_is_rejected() {
        let error = ManifestV04::from_slice(
            br#"{"_manifest":{"manifestVersion":4},"_observability":{"enabled":false,"vmName":"sys-obs","obsVsockCid":1000,"obsVsockHostSocket":"/var/lib/nixling/vms/sys-obs/vsock.sock","signozUrl":"http://10.40.0.10:8080","signozOtlpGrpcPort":4317,"signozOtlpHttpPort":4318},"corp-vm":{"apiSocket":"/var/lib/nixling/vms/corp-vm/corp-vm.sock","audio":false,"audioService":"nixling-corp-vm-snd.service","audioStateFile":"/var/lib/nixling/vms/corp-vm/state/audio-state.json","bridge":"br-work-lan","env":"work","gpuSocket":"/var/lib/nixling/vms/corp-vm/corp-vm-gpu.sock","graphics":false,"isNetVm":false,"name":"wrong-name","netVm":"sys-work-net","observability":{"agentSocket":"/run/nixling/otlp.sock","enabled":false,"vsockCid":110,"vsockHostSocket":"/var/lib/nixling/vms/corp-vm/vsock.sock"},"sshUser":"alice","stateDir":"/var/lib/nixling/vms/corp-vm","staticIp":"10.20.0.10","tap":"work-l10","tpm":false,"tpmSocket":"/run/swtpm/corp-vm/sock","usbipYubikey":false,"usbipdHostIp":"192.0.2.1"}}"#,
        )
        .expect_err("name mismatch fails");
        assert_eq!(error.kind().as_str(), "manifest-parse-error");
        assert!(error.message().contains("opaque reason: name-key-mismatch"));
    }

    // Regression: the daemon-only end state pins
    // `_manifest.manifestVersion` to a single supported
    // integer. A bundle stamped with the previous (legacy) version is
    // rejected with `manifest-version-mismatch`, and the new version
    // must load cleanly.
    #[test]
    fn legacy_manifest_version_is_rejected() {
        let error = ManifestV04::from_slice(
            br#"{"_manifest":{"manifestVersion":3},"_observability":{"enabled":false,"vmName":"sys-obs","obsVsockCid":1000,"obsVsockHostSocket":"/var/lib/nixling/vms/sys-obs/vsock.sock","signozUrl":"http://10.40.0.10:8080","signozOtlpGrpcPort":4317,"signozOtlpHttpPort":4318}}"#,
        )
        .expect_err("legacy v3 manifest must be rejected after the SigNoz manifest bump");
        assert_eq!(error.kind().as_str(), "manifest-version-mismatch");
        assert!(
            error
                .message()
                .contains("opaque reason: manifest-version-mismatch"),
            "unexpected error message: {}",
            error.message()
        );
    }

    #[test]
    fn current_manifest_version_loads() {
        let manifest = ManifestV04::from_slice(
            br#"{"_manifest":{"manifestVersion":4},"_observability":{"enabled":false,"vmName":"sys-obs","obsVsockCid":1000,"obsVsockHostSocket":"/var/lib/nixling/vms/sys-obs/vsock.sock","signozUrl":"http://10.40.0.10:8080","signozOtlpGrpcPort":4317,"signozOtlpHttpPort":4318}}"#,
        )
        .expect("current manifest version parses");
        assert_eq!(
            manifest.manifest.manifest_version,
            super::MANIFEST_VERSION_CURRENT
        );
    }

    #[test]
    fn future_manifest_version_is_rejected() {
        let error = ManifestV04::from_slice(
            br#"{"_manifest":{"manifestVersion":99},"_observability":{"enabled":false,"vmName":"sys-obs","obsVsockCid":1000,"obsVsockHostSocket":"/var/lib/nixling/vms/sys-obs/vsock.sock","signozUrl":"http://10.40.0.10:8080","signozOtlpGrpcPort":4317,"signozOtlpHttpPort":4318}}"#,
        )
        .expect_err("future manifest version must fail closed");
        assert_eq!(error.kind().as_str(), "manifest-version-mismatch");
        assert!(error
            .message()
            .contains("opaque reason: manifest-version-mismatch"));
    }
}
