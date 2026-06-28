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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonToPickerMessage {
    OpenRequest(Box<OpenRequest>),
    Close(CloseFrame),
    Error(ErrorFrame),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRequest {
    pub selected_protocol_version: u16,
    pub clipd_version: String,
    pub picker_version: String,
    pub request_id: String,
    pub destination: DestinationMetadata,
    pub requested_mime_type: String,
    pub expires_at_unix_ms: u64,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseFrame {
    pub request_id: String,
    pub reason: crate::policy::ReasonCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorFrame {
    pub request_id: String,
    pub reason: crate::policy::ReasonCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub entry_id: String,
    pub source: SourceMetadata,
    pub preview_text: Option<String>,
    pub content_type: String,
    pub timestamp_unix_ms: u64,
    pub thumbnail_png_base64: Option<String>,
    pub confirmation_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceMetadata {
    pub realm: RealmMetadata,
    pub app: Option<String>,
    pub app_id: Option<String>,
    pub attribution: AttributionQuality,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DestinationMetadata {
    pub realm: RealmMetadata,
    pub app: Option<String>,
    pub app_id: Option<String>,
    pub title: Option<String>,
    pub workspace: Option<String>,
    pub output: Option<String>,
    pub attribution: AttributionQuality,
    pub placement: Option<PlacementHint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementHint {
    pub output: Option<String>,
    pub anchor: PickerAnchor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PickerAnchor {
    Pointer,
    FocusedWindow,
    CurrentOutput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RealmMetadata {
    pub id: String,
    pub label: String,
    pub kind: RealmKind,
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
          "source":{"realm":{"id":"host","label":"Host","kind":"host"},"app":null,"app_id":null,"attribution":"focused_window_guess","future_label":"ignored"},
          "preview_text":"hello",
          "content_type":"text/plain",
          "timestamp_unix_ms":7,
          "thumbnail_png_base64":null,
          "confirmation_required":false,
          "future_display_field":"ignored"
        }"#;
        let candidate: Candidate = serde_json::from_str(json).expect("candidate");
        assert_eq!(candidate.entry_id, "e");
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
