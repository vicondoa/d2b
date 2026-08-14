//! `d2b-provider-relay`: the Azure Relay transport auth/credential core
//! for the realm gateway (ADR 0032).
//!
//! This crate is the d2b-native home for the **credential model +
//! connect contract** that the gateway's relay transport and the in-sandbox
//! sender are built on.
//!
//! ## Three-plane mapping
//! - The **gateway** (host side) holds the relay **Listen** credential and
//!   opens the listener control channel. Listen auth is a gateway-side SAS
//!   minted from the `gateway-listen` rule key, or (later) the gateway's own
//!   Entra **Listener** role.
//! - The **container** (sandbox sender) authenticates with either an **Entra
//!   bearer token from its managed identity** or a **gateway-minted,
//!   short-lived Send SAS bearer**. The ACA display path uses the latter because
//!   ACA Relay Entra substreams closed during Waypipe forwarding; the long-lived
//!   SAS rule key still never enters the sandbox.
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
pub const RELAY_TOKEN_RESOURCE: &str = "https://relay.azure.net/";

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
    /// Hybrid connection (entity) name, e.g. `hc-d2b-display`.
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
    /// A pre-minted Shared Access Signature bearer. The gateway uses this for
    /// short-lived Send tokens handed to ACA sandboxes without exposing the
    /// underlying rule key.
    SasToken(String),
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
            RelayCredential::SasToken(_) => f.write_str("RelayCredential::SasToken(<redacted>)"),
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

/// The maximum minted-SAS lifetime (seconds) accepted for relay sessions.
pub const MAX_SAS_TTL_SECS: u64 = 15 * 60;

/// The default minted-SAS lifetime (seconds). The gateway mints short-lived
/// SAS bearers; a long-lived token is never persisted.
pub const DEFAULT_SAS_TTL_SECS: u64 = MAX_SAS_TTL_SECS;

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
    if ttl_secs > MAX_SAS_TTL_SECS {
        return Err(RelayError::TtlTooLong {
            requested: ttl_secs,
            max: MAX_SAS_TTL_SECS,
        });
    }

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
/// `ServiceBusAuthorization: Bearer <jwt>` header. A pre-minted SAS bearer is
/// also accepted for the ACA path; it is already scoped/expiring, so this
/// function only URL-encodes it into `sb-hc-token`.
pub fn build_connect(
    endpoint: &RelayEndpoint,
    role: RelayRole,
    credential: &RelayCredential,
    ttl_secs: u64,
) -> Result<RelayConnect, RelayError> {
    // The sender does NOT supply its own `sb-hc-id`. Azure Relay generates the
    // rendezvous correlation id (a GUID) and embeds it in the accept message's
    // address; a caller-supplied non-GUID id yields an unserviceable rendezvous
    // address that the listener's accept connect rejects with 400. This matches
    // the official Relay SDKs, which omit `sb-hc-id` on connect.
    let base = format!(
        "wss://{}/$hc/{}?sb-hc-action={}",
        endpoint.namespace,
        urlencoding::encode(&endpoint.entity),
        role.action(),
    );
    match credential {
        RelayCredential::EntraBearer(token) => Ok(RelayConnect {
            url: base,
            auth_header: Some(format!("Bearer {token}")),
        }),
        RelayCredential::SasToken(token) => Ok(RelayConnect {
            url: format!("{base}&sb-hc-token={}", urlencoding::encode(token)),
            auth_header: None,
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

/// Errors building relay auth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayError {
    /// The system clock was before the Unix epoch.
    Clock,
    /// The SAS key was not valid HMAC key material.
    Key,
    /// The requested SAS TTL exceeded the short-lived bearer bound.
    TtlTooLong {
        /// Requested lifetime in seconds.
        requested: u64,
        /// Maximum permitted lifetime in seconds.
        max: u64,
    },
}

impl fmt::Display for RelayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelayError::Clock => write!(f, "system clock is before the unix epoch"),
            RelayError::Key => write!(f, "relay SAS key is invalid"),
            RelayError::TtlTooLong { requested, max } => write!(
                f,
                "relay SAS TTL {requested}s exceeds maximum short-lived bound {max}s"
            ),
        }
    }
}

impl std::error::Error for RelayError {}
