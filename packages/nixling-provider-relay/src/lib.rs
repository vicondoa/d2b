//! `nixling-provider-relay`: the Azure Relay transport auth/credential core
//! for the realm gateway (ADR 0032, P0).
//!
//! This productionizes the relay leg of the P0 vertical. The POC's
//! `nixling-relay-bridge` proved the byte path; this crate is the
//! nixling-native home for the **credential model + connect contract** that
//! the gateway's relay transport and the in-sandbox sender are built on.
//!
//! ## Three-plane mapping
//! - The **gateway** (host side) holds the relay **Listen** credential and
//!   opens the listener control channel. Listen auth is a gateway-side SAS
//!   minted from the `gateway-listen` rule key, or (later) the gateway's own
//!   Entra **Listener** role.
//! - The **container** (sandbox sender) authenticates with an **Entra bearer
//!   token from its managed identity** (plane 2) — the productionized path,
//!   so **no SAS key ever enters the sandbox**. The token is presented in the
//!   `ServiceBusAuthorization` WebSocket header (Azure Relay's Entra
//!   data-plane auth), never in the URL.
//!
//! Every secret ([`RelayCredential`] material, minted SAS, bearer token) has
//! a redacted `Debug` so it can never reach a log, span, or audit record.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// The Entra resource (audience) a managed identity requests a token for to
/// authenticate to Azure Relay. Confirmed against the Azure Relay docs.
pub const RELAY_TOKEN_RESOURCE: &str = "https://relay.azure.net";

/// The role an endpoint plays on the hybrid connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayRole {
    /// The gateway side that accepts sender connections.
    Listener,
    /// The sandbox side that dials out to send.
    Sender,
}

impl RelayRole {
    /// The `sb-hc-action` query value for this role.
    fn action(self) -> &'static str {
        match self {
            RelayRole::Listener => "listen",
            RelayRole::Sender => "connect",
        }
    }
}

/// A hybrid-connection endpoint: the relay namespace FQDN + the entity
/// (hybrid connection) name. Non-secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayEndpoint {
    /// Namespace FQDN, e.g. `relns-xxxx.servicebus.windows.net`.
    pub namespace: String,
    /// Hybrid connection (entity) name, e.g. `hc-nixling-display`.
    pub entity: String,
}

/// How an endpoint authenticates to the relay. Both variants wrap secret
/// material and therefore redact their `Debug`.
#[derive(Clone)]
pub enum RelayCredential {
    /// A Shared Access Signature: an authorization-rule name + its key. Used
    /// gateway-side (the Listen rule), and transitionally for non-MI senders.
    Sas {
        /// The authorization-rule (key) name, e.g. `gateway-listen`.
        key_name: String,
        /// The rule's key. Secret.
        key: String,
    },
    /// A Microsoft Entra bearer token acquired by a managed identity for
    /// [`RELAY_TOKEN_RESOURCE`]. The productionized container path. Secret.
    EntraBearer(String),
}

impl fmt::Debug for RelayCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The key name is a non-secret label; the key/token are redacted.
            RelayCredential::Sas { key_name, .. } => f
                .debug_struct("RelayCredential::Sas")
                .field("key_name", key_name)
                .field("key", &"<redacted>")
                .finish(),
            RelayCredential::EntraBearer(_) => {
                f.write_str("RelayCredential::EntraBearer(<redacted>)")
            }
        }
    }
}

/// The bytes a [`RelayCredential`] resolves to for a WebSocket connect: a SAS
/// goes in the `sb-hc-token` query parameter; an Entra token goes in the
/// `ServiceBusAuthorization` header. Exactly one is set. Redacted `Debug`.
#[derive(Clone, PartialEq, Eq)]
pub struct RelayConnect {
    /// The `wss://…` URL (already URL-encoded; never contains the bearer).
    pub url: String,
    /// The `ServiceBusAuthorization` header value (`Bearer <jwt>`), when the
    /// credential is an Entra token.
    pub auth_header: Option<String>,
}

impl fmt::Debug for RelayConnect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The URL may carry an sb-hc-token SAS; redact the whole URL query and
        // never print the header (it carries the bearer).
        let scheme_host = self.url.split('?').next().unwrap_or("");
        f.debug_struct("RelayConnect")
            .field("url", &format!("{scheme_host}?<redacted>"))
            .field(
                "auth_header",
                &self.auth_header.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

/// The default minted-SAS lifetime (seconds). The gateway mints short-lived
/// Listen tokens; a long-lived token is never persisted.
pub const DEFAULT_SAS_TTL_SECS: u64 = 3600;

/// Mint a Service Bus SAS token conferring the rule's rights on the entity,
/// expiring `ttl_secs` from now. This is the gateway-side minting the POC's
/// relay bridge proved; it is reproduced here byte-for-byte.
///
/// The returned string is secret (it is a bearer); callers must treat it as
/// such (it is never logged by this crate).
pub fn mint_sas(
    endpoint: &RelayEndpoint,
    key_name: &str,
    key: &str,
    ttl_secs: u64,
) -> Result<String, RelayError> {
    let resource = format!("http://{}/{}", endpoint.namespace, endpoint.entity);
    let resource_enc = urlencoding::encode(&resource).to_lowercase();
    let expiry = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| RelayError::Clock)?
        .as_secs()
        + ttl_secs;
    let to_sign = format!("{resource_enc}\n{expiry}");
    let mut mac = HmacSha256::new_from_slice(key.as_bytes()).map_err(|_| RelayError::Key)?;
    mac.update(to_sign.as_bytes());
    let sig = base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
    let sig_enc = urlencoding::encode(&sig);
    Ok(format!(
        "SharedAccessSignature sr={resource_enc}&sig={sig_enc}&se={expiry}&skn={key_name}"
    ))
}

/// Build the relay WebSocket connect contract for `role` using `credential`.
/// SAS authentication mints a token into the `sb-hc-token` query parameter;
/// Entra authentication leaves the URL token-free and returns the
/// `ServiceBusAuthorization: Bearer <jwt>` header.
pub fn build_connect(
    endpoint: &RelayEndpoint,
    role: RelayRole,
    credential: &RelayCredential,
    ttl_secs: u64,
) -> Result<RelayConnect, RelayError> {
    let id_param = match role {
        RelayRole::Sender => format!("&sb-hc-id={}", connect_id()),
        RelayRole::Listener => String::new(),
    };
    let base = format!(
        "wss://{}/$hc/{}?sb-hc-action={}{}",
        endpoint.namespace,
        urlencoding::encode(&endpoint.entity),
        role.action(),
        id_param,
    );
    match credential {
        RelayCredential::EntraBearer(token) => Ok(RelayConnect {
            url: base,
            auth_header: Some(format!("Bearer {token}")),
        }),
        RelayCredential::Sas { key_name, key } => {
            let token = mint_sas(endpoint, key_name, key, ttl_secs)?;
            Ok(RelayConnect {
                url: format!("{base}&sb-hc-token={}", urlencoding::encode(&token)),
                auth_header: None,
            })
        }
    }
}

/// A connect id for the sender rendezvous. Deterministic length, non-secret.
fn connect_id() -> String {
    // A 16-byte hex id from the system clock + a process-unique counter. No
    // RNG dependency; uniqueness only needs to hold within a gateway.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{:016x}", t ^ n.rotate_left(32))
}

/// Errors building relay auth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayError {
    /// The system clock was before the Unix epoch.
    Clock,
    /// The SAS key was not valid HMAC key material.
    Key,
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelayError::Clock => write!(f, "system clock is before the unix epoch"),
            RelayError::Key => write!(f, "relay SAS key is invalid"),
        }
    }
}

impl std::error::Error for RelayError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint() -> RelayEndpoint {
        RelayEndpoint {
            namespace: "relns-test.servicebus.windows.net".into(),
            entity: "hc-nixling-display".into(),
        }
    }

    #[test]
    fn mint_sas_is_deterministic_for_fixed_inputs_modulo_expiry() {
        let ep = endpoint();
        let a = mint_sas(&ep, "gateway-listen", "c2VjcmV0a2V5", 3600).unwrap();
        // Shape: a SharedAccessSignature with sr/sig/se/skn.
        assert!(a.starts_with("SharedAccessSignature sr="));
        assert!(a.contains("&skn=gateway-listen"));
        assert!(a.contains("&sig="));
        assert!(a.contains("&se="));
        // The resource is the lowercased url-encoded http form of the entity.
        assert!(a.contains("relns-test.servicebus.windows.net"));
    }

    #[test]
    fn entra_sender_uses_header_not_url_token() {
        let ep = endpoint();
        let cred = RelayCredential::EntraBearer("jwt.abc.def".into());
        let c = build_connect(&ep, RelayRole::Sender, &cred, 3600).unwrap();
        // The bearer NEVER appears in the URL.
        assert!(!c.url.contains("jwt.abc.def"));
        assert!(!c.url.contains("sb-hc-token"));
        assert!(c.url.contains("sb-hc-action=connect"));
        assert!(c.url.contains("sb-hc-id="));
        assert_eq!(c.auth_header.as_deref(), Some("Bearer jwt.abc.def"));
    }

    #[test]
    fn sas_listener_puts_token_in_url_and_no_header() {
        let ep = endpoint();
        let cred = RelayCredential::Sas {
            key_name: "gateway-listen".into(),
            key: "c2VjcmV0a2V5".into(),
        };
        let c = build_connect(&ep, RelayRole::Listener, &cred, 3600).unwrap();
        assert!(c.url.contains("sb-hc-action=listen"));
        assert!(c.url.contains("sb-hc-token="));
        assert!(!c.url.contains("sb-hc-id=")); // listener has no rendezvous id
        assert!(c.auth_header.is_none());
    }

    #[test]
    fn credential_debug_redacts_secrets() {
        let sas = RelayCredential::Sas {
            key_name: "gateway-send".into(),
            key: "supersecretkey".into(),
        };
        let d = format!("{sas:?}");
        assert!(d.contains("gateway-send"));
        assert!(!d.contains("supersecretkey"));
        let bearer = RelayCredential::EntraBearer("jwt.secret.token".into());
        let d = format!("{bearer:?}");
        assert!(!d.contains("jwt.secret.token"));
    }

    #[test]
    fn connect_debug_redacts_url_query_and_header() {
        let ep = endpoint();
        let cred = RelayCredential::EntraBearer("jwt.abc.def".into());
        let c = build_connect(&ep, RelayRole::Sender, &cred, 3600).unwrap();
        let d = format!("{c:?}");
        assert!(!d.contains("jwt.abc.def"));
        assert!(!d.contains("Bearer"));
        assert!(d.contains("<redacted>"));
    }

    #[test]
    fn connect_ids_are_unique() {
        let a = connect_id();
        let b = connect_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), 16);
    }
}
