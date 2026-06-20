//! Gateway runtime credential loading and relay-token minting.
//!
//! Credentials are runtime state inside the gateway guest, not Nix data. This
//! module refuses `/nix/store` paths, enforces `0600`, optionally enforces the
//! gateway principal uid, redacts all key/token debug output, and mints
//! short-lived Relay Send tokens from a least-privilege send rule.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use nixling_provider_relay::{
    MAX_SAS_TTL_SECS, RelayCredential, RelayEndpoint, RelayError, mint_sas,
};
use serde_json::Value;

/// Required file mode for gateway credential envelopes.
pub const GATEWAY_CREDENTIAL_MODE: u32 = 0o600;

/// Maximum lifetime for gateway-minted Relay Send SAS bearers in the P0 path.
pub const MAX_RELAY_SEND_TOKEN_TTL_SECS: u64 = MAX_SAS_TTL_SECS;

/// Default lifetime for gateway-minted Relay Send SAS bearers in the P0 path.
pub const DEFAULT_RELAY_SEND_TOKEN_TTL_SECS: u64 = MAX_RELAY_SEND_TOKEN_TTL_SECS;

/// Runtime credential file policy.
#[derive(Debug, Clone, Default)]
pub struct CredentialFilePolicy {
    /// Optional required owner uid (the gateway principal).
    pub required_uid: Option<u32>,
}

/// A loaded gateway credential envelope. `Debug` redacts all secret material.
#[derive(Clone)]
pub struct GatewayCredential {
    listen_key_name: String,
    listen_key: String,
    send_key_name: String,
    send_key: String,
}

impl core::fmt::Debug for GatewayCredential {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("GatewayCredential")
            .field("listen_key_name", &self.listen_key_name)
            .field("listen_key", &"<redacted>")
            .field("send_key_name", &self.send_key_name)
            .field("send_key", &"<redacted>")
            .finish()
    }
}

impl GatewayCredential {
    /// Load and validate the credential envelope at `path`.
    pub fn load(
        path: impl AsRef<Path>,
        policy: &CredentialFilePolicy,
    ) -> Result<Self, CredentialError> {
        let path = path.as_ref();
        if path.starts_with("/nix/store") {
            return Err(CredentialError::NixStorePath);
        }
        let meta = fs::metadata(path).map_err(|_| CredentialError::Unreadable)?;
        if meta.mode() & 0o777 != GATEWAY_CREDENTIAL_MODE {
            return Err(CredentialError::BadMode(meta.mode() & 0o777));
        }
        if let Some(uid) = policy.required_uid
            && meta.uid() != uid
        {
            return Err(CredentialError::BadOwner(meta.uid()));
        }
        let raw = fs::read_to_string(path).map_err(|_| CredentialError::Unreadable)?;
        Self::parse_json(&raw)
    }

    /// Build the gateway listener credential (Listen rule).
    pub fn listener_credential(&self) -> RelayCredential {
        RelayCredential::Sas {
            key_name: self.listen_key_name.clone(),
            key: self.listen_key.clone(),
        }
    }

    /// Mint a short-lived Relay Send SAS token for a container agent. The
    /// returned token is secret and redacts in `Debug`.
    pub fn mint_send_token(
        &self,
        endpoint: &RelayEndpoint,
        ttl_secs: u64,
    ) -> Result<MintedRelaySendToken, RelayError> {
        if ttl_secs > MAX_RELAY_SEND_TOKEN_TTL_SECS {
            return Err(RelayError::TtlTooLong {
                requested: ttl_secs,
                max: MAX_RELAY_SEND_TOKEN_TTL_SECS,
            });
        }

        Ok(MintedRelaySendToken(mint_sas(
            endpoint,
            &self.send_key_name,
            &self.send_key,
            ttl_secs,
        )?))
    }

    fn parse_json(raw: &str) -> Result<Self, CredentialError> {
        let v: Value = serde_json::from_str(raw).map_err(|_| CredentialError::Malformed)?;
        Ok(Self {
            listen_key_name: required_str(&v, &["relayListen", "keyName"])?,
            listen_key: required_str(&v, &["relayListen", "key"])?,
            send_key_name: required_str(&v, &["relaySend", "keyName"])?,
            send_key: required_str(&v, &["relaySend", "key"])?,
        })
    }
}

/// A minted short-lived Relay Send SAS bearer. `Debug` redacts the token.
#[derive(Clone, PartialEq, Eq)]
pub struct MintedRelaySendToken(String);

impl MintedRelaySendToken {
    /// Borrow the token for provider-control-plane delivery.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for MintedRelaySendToken {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("MintedRelaySendToken(<redacted>)")
    }
}

/// Credential load/validation failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialError {
    /// Credential was configured under `/nix/store`.
    NixStorePath,
    /// File could not be read/stat'd.
    Unreadable,
    /// File mode was not `0600`.
    BadMode(u32),
    /// File owner did not match the configured gateway principal.
    BadOwner(u32),
    /// JSON shape was malformed or missing a required field.
    Malformed,
}

impl core::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CredentialError::NixStorePath => {
                f.write_str("gateway credential must not live in /nix/store")
            }
            CredentialError::Unreadable => f.write_str("gateway credential cannot be read"),
            CredentialError::BadMode(mode) => {
                write!(f, "gateway credential mode must be 0600, got {mode:o}")
            }
            CredentialError::BadOwner(uid) => {
                write!(f, "gateway credential owner uid mismatch: {uid}")
            }
            CredentialError::Malformed => f.write_str("gateway credential JSON is malformed"),
        }
    }
}

impl std::error::Error for CredentialError {}

fn required_str(v: &Value, path: &[&str]) -> Result<String, CredentialError> {
    let mut cur = v;
    for key in path {
        cur = cur.get(*key).ok_or(CredentialError::Malformed)?;
    }
    cur.as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or(CredentialError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn fixture(dir: &Path) -> PathBuf {
        let path = dir.join("credential.json");
        fs::write(
            &path,
            r#"{
              "relayListen": { "keyName": "gateway-listen", "key": "listen-secret" },
              "relaySend": { "keyName": "gateway-send", "key": "send-secret" }
            }"#,
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    fn endpoint() -> RelayEndpoint {
        RelayEndpoint {
            namespace: "relns-example.servicebus.windows.net".into(),
            entity: "hc-display".into(),
        }
    }

    fn now_unix_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn sas_param<'a>(token: &'a str, name: &str) -> &'a str {
        let prefix = format!("{name}=");
        token
            .strip_prefix("SharedAccessSignature ")
            .unwrap()
            .split('&')
            .find_map(|part| part.strip_prefix(&prefix))
            .unwrap()
    }

    #[test]
    fn loads_only_runtime_0600_files_and_redacts_debug() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path());
        let cred = GatewayCredential::load(&path, &CredentialFilePolicy::default()).unwrap();
        let dbg = format!("{cred:?}");
        assert!(dbg.contains("gateway-listen"));
        assert!(dbg.contains("gateway-send"));
        assert!(!dbg.contains("listen-secret"));
        assert!(!dbg.contains("send-secret"));
    }

    #[test]
    fn rejects_group_or_other_readable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(
            GatewayCredential::load(&path, &CredentialFilePolicy::default()).unwrap_err(),
            CredentialError::BadMode(0o640)
        );
    }

    #[test]
    fn rejects_nix_store_credential_path() {
        assert_eq!(
            GatewayCredential::load(
                "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-secret.json",
                &CredentialFilePolicy::default()
            )
            .unwrap_err(),
            CredentialError::NixStorePath
        );
    }

    #[test]
    fn rejects_owner_mismatch_when_policy_requires_uid() {
        let dir = tempfile::tempdir().unwrap();
        let path = fixture(dir.path());
        let uid = fs::metadata(&path).unwrap().uid().saturating_add(1);
        assert!(matches!(
            GatewayCredential::load(
                &path,
                &CredentialFilePolicy {
                    required_uid: Some(uid)
                }
            ),
            Err(CredentialError::BadOwner(_))
        ));
    }

    #[test]
    fn mints_redacted_short_lived_send_token() {
        let dir = tempfile::tempdir().unwrap();
        let cred =
            GatewayCredential::load(fixture(dir.path()), &CredentialFilePolicy::default()).unwrap();
        let ttl = 60;
        let before = now_unix_secs();
        let token = cred.mint_send_token(&endpoint(), ttl).unwrap();
        let after = now_unix_secs();
        assert!(token.expose().starts_with("SharedAccessSignature "));
        assert_eq!(sas_param(token.expose(), "skn"), "gateway-send");
        let expiry = sas_param(token.expose(), "se").parse::<u64>().unwrap();
        assert!(expiry >= before + ttl);
        assert!(expiry <= after + ttl);
        assert!(!token.expose().contains("send-secret"));
        assert_eq!(format!("{token:?}"), "MintedRelaySendToken(<redacted>)");
        assert!(!format!("{token:?}").contains("SharedAccessSignature"));
    }

    #[test]
    fn rejects_send_token_ttl_above_short_lived_cap() {
        let dir = tempfile::tempdir().unwrap();
        let cred =
            GatewayCredential::load(fixture(dir.path()), &CredentialFilePolicy::default()).unwrap();
        assert_eq!(
            cred.mint_send_token(&endpoint(), MAX_RELAY_SEND_TOKEN_TTL_SECS + 1)
                .unwrap_err(),
            RelayError::TtlTooLong {
                requested: MAX_RELAY_SEND_TOKEN_TTL_SECS + 1,
                max: MAX_RELAY_SEND_TOKEN_TTL_SECS
            }
        );
    }
}
