use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolVersionRange {
    pub min: u16,
    pub max: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientHello {
    pub protocol_version_range: ProtocolVersionRange,
    pub picker_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Select {
    pub selected_protocol_version: u16,
    pub request_id: String,
    pub entry_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cancel {
    pub selected_protocol_version: u16,
    pub request_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PickerToDaemonMessage {
    ClientHello(ClientHello),
    Select(Select),
    Cancel(Cancel),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonToPickerMessage {
    OpenRequest(Box<OpenRequest>),
    Close(CloseFrame),
    Error(ErrorFrame),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenRequest {
    pub selected_protocol_version: u16,
    pub clipd_version: String,
    pub picker_version: String,
    pub request_id: String,
    pub destination: DestinationMetadata,
    pub requested_mime_type: String,
    pub expires_at_unix_ms: u64,
    pub placement_hints: Option<PlacementHint>,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseFrame {
    pub request_id: String,
    pub code: crate::policy::ReasonCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorFrame {
    pub request_id: String,
    pub code: crate::policy::ReasonCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub entry_id: String,
    pub source_realm: String,
    pub source_realm_kind: RealmKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_canonical_target: Option<String>,
    pub source_app: Option<String>,
    pub source_app_id: Option<String>,
    pub source_attribution: AttributionQuality,
    pub preview_text: Option<String>,
    pub content_type: String,
    pub timestamp_unix_ms: u64,
    pub thumbnail_png_base64: Option<String>,
    pub byte_count: Option<u64>,
    pub confirmation_required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_preflight: Option<ClipboardCapabilityPreflight>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationMetadata {
    pub realm: String,
    pub realm_kind: RealmKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_target: Option<String>,
    pub application: Option<String>,
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub workspace: Option<String>,
    pub output: Option<String>,
    pub attribution: AttributionQuality,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_preflight: Option<ClipboardCapabilityPreflight>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardCapabilityPreflight {
    pub status: ClipboardCapabilityPreflightStatus,
    pub required_capabilities: Vec<String>,
    pub advertised_capabilities: Vec<String>,
    pub missing_capabilities: Vec<String>,
    pub authority: ClipboardTransferAuthority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardCapabilityPreflightStatus {
    Satisfied,
    Denied,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardTransferAuthority {
    PickerClipd,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacementHint {
    pub pointer_x: Option<f64>,
    pub pointer_y: Option<f64>,
    pub output_width: Option<i32>,
    pub output_height: Option<i32>,
    pub overlay_width: Option<i32>,
    pub overlay_height: Option<i32>,
    pub output: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RealmKind {
    Host,
    Vm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionQuality {
    ExactClient,
    FocusedWindowGuess,
    CacheStaleFocusedWindowGuess,
    BrokerInjectedDebug,
}

pub fn negotiate_version(range: &ProtocolVersionRange, daemon_supported: u16) -> Option<u16> {
    (range.min <= daemon_supported && daemon_supported <= range.max).then_some(daemon_supported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_hello_contains_no_token_or_request_id() {
        let hello = PickerToDaemonMessage::ClientHello(ClientHello {
            protocol_version_range: ProtocolVersionRange { min: 1, max: 1 },
            picker_version: "picker-test".to_owned(),
        });

        let json = serde_json::to_string(&hello).expect("serialize hello");
        assert!(!json.contains("token"));
        assert!(!json.contains("request_id"));
        let decoded: PickerToDaemonMessage = serde_json::from_str(&json).expect("decode hello");
        assert_eq!(decoded, hello);
    }

    #[test]
    fn daemon_received_messages_reject_unknown_fields() {
        let json = r#"{"type":"select","selected_protocol_version":1,"request_id":"r","entry_id":"e","extra":true}"#;
        let err = serde_json::from_str::<PickerToDaemonMessage>(json).expect_err("extra field");
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn display_side_candidate_tolerates_additive_metadata() {
        let json = r#"{
          "entry_id":"e",
          "source_realm":"Host",
          "source_realm_kind":"host",
          "source_canonical_target":null,
          "source_app":null,
          "source_app_id":null,
          "source_attribution":"focused_window_guess",
          "preview_text":"hello",
          "content_type":"text/plain",
          "timestamp_unix_ms":7,
          "thumbnail_png_base64":null,
          "byte_count":12,
          "confirmation_required":false,
          "capability_preflight":{
            "status":"satisfied",
            "required_capabilities":["clipboard"],
            "advertised_capabilities":["clipboard"],
            "missing_capabilities":[],
            "authority":"picker_clipd"
          },
          "future_display_field":"ignored"
        }"#;
        let candidate: Candidate = serde_json::from_str(json).expect("candidate");
        assert_eq!(candidate.entry_id, "e");
        assert_eq!(
            candidate.capability_preflight.as_ref().map(|p| p.status),
            Some(ClipboardCapabilityPreflightStatus::Satisfied)
        );
    }

    #[test]
    fn daemon_open_request_can_carry_canonical_realm_metadata() {
        let msg = DaemonToPickerMessage::OpenRequest(Box::new(OpenRequest {
            selected_protocol_version: 1,
            clipd_version: "0.0.0".to_owned(),
            picker_version: "picker".to_owned(),
            request_id: "req".to_owned(),
            destination: DestinationMetadata {
                realm: "builder".to_owned(),
                realm_kind: RealmKind::Vm,
                canonical_target: Some("builder.local.d2b".to_owned()),
                application: None,
                app_id: None,
                title: None,
                workspace: None,
                output: None,
                attribution: AttributionQuality::ExactClient,
                capability_preflight: Some(ClipboardCapabilityPreflight {
                    status: ClipboardCapabilityPreflightStatus::Satisfied,
                    required_capabilities: vec!["clipboard".to_owned()],
                    advertised_capabilities: vec!["clipboard".to_owned()],
                    missing_capabilities: Vec::new(),
                    authority: ClipboardTransferAuthority::PickerClipd,
                }),
            },
            requested_mime_type: "text/plain".to_owned(),
            expires_at_unix_ms: 7,
            placement_hints: None,
            candidates: vec![Candidate {
                entry_id: "entry".to_owned(),
                source_realm: "builder".to_owned(),
                source_realm_kind: RealmKind::Vm,
                source_canonical_target: Some("builder.local.d2b".to_owned()),
                source_app: None,
                source_app_id: None,
                source_attribution: AttributionQuality::ExactClient,
                preview_text: None,
                content_type: "text/plain".to_owned(),
                timestamp_unix_ms: 7,
                thumbnail_png_base64: None,
                byte_count: Some(4),
                confirmation_required: false,
                capability_preflight: Some(ClipboardCapabilityPreflight {
                    status: ClipboardCapabilityPreflightStatus::Satisfied,
                    required_capabilities: vec!["clipboard".to_owned()],
                    advertised_capabilities: vec!["clipboard".to_owned()],
                    missing_capabilities: Vec::new(),
                    authority: ClipboardTransferAuthority::PickerClipd,
                }),
            }],
        }));

        let json = serde_json::to_string(&msg).expect("serialize open request");
        assert!(json.contains(r#""canonical_target":"builder.local.d2b""#));
        assert!(json.contains(r#""source_canonical_target":"builder.local.d2b""#));
        assert!(json.contains(r#""authority":"picker_clipd""#));
    }

    #[test]
    fn negotiates_supported_protocol() {
        assert_eq!(
            negotiate_version(&ProtocolVersionRange { min: 1, max: 1 }, 1),
            Some(1)
        );
        assert_eq!(
            negotiate_version(&ProtocolVersionRange { min: 2, max: 3 }, 1),
            None
        );
    }
}
