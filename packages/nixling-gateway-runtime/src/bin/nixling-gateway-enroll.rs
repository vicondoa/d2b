//! In-guest gateway credential enrollment helper.
//!
//! Secrets are read from stdin and sealed into the gateway runtime state. They
//! are never accepted as command-line arguments.

use std::io::Read;
use std::path::PathBuf;

use nixling_gateway_runtime::{
    CredentialEnvelopeMeta, CredentialError, CredentialFilePolicy, GatewayCredential, SealingKey,
};

type Err = Box<dyn std::error::Error>;

fn usage() -> Err {
    "usage: nixling-gateway-enroll <enroll|rotate> <credential-path> <seal-key-path> [not-after-unix]"
        .into()
}

fn not_after(raw: Option<String>) -> Result<Option<u64>, Err> {
    raw.map(|value| value.parse().map_err(Into::into))
        .transpose()
}

fn read_material() -> Result<nixling_gateway_runtime::GatewayCredentialMaterial, Err> {
    let mut raw = String::new();
    std::io::stdin().read_to_string(&mut raw)?;
    Ok(GatewayCredential::material_from_json(&raw)?)
}

fn validate_paths(credential_path: &PathBuf, seal_key_path: &PathBuf) -> Result<(), Err> {
    let state_dir = std::env::var("NIXLING_GATEWAY_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/var/lib/nixling/gateways"));
    for path in [credential_path, seal_key_path] {
        if !path.is_absolute()
            || path.starts_with("/nix/store")
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            || !path.starts_with(&state_dir)
        {
            return Err(
                "credential and seal key must be absolute runtime paths under the gateway state directory"
                    .into(),
            );
        }
    }
    Ok(())
}

fn main() -> Result<(), Err> {
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();
    let mut args = std::env::args().skip(1);
    let op = args.next().ok_or_else(usage)?;
    let credential_path = PathBuf::from(args.next().ok_or_else(usage)?);
    let seal_key_path = PathBuf::from(args.next().ok_or_else(usage)?);
    let not_after = not_after(args.next())?;
    if args.next().is_some() {
        return Err(usage());
    }
    validate_paths(&credential_path, &seal_key_path)?;

    let policy = CredentialFilePolicy::default();
    let material = read_material()?;
    let now = now_unix();
    let generation = match op.as_str() {
        "enroll" => {
            let key = SealingKey::load_or_generate(&seal_key_path, &policy)?;
            match GatewayCredential::enroll_sealed(
                &credential_path,
                &key,
                material,
                CredentialEnvelopeMeta::first(not_after),
                now,
            ) {
                Ok(()) => 1,
                Err(CredentialError::AlreadyExists) => {
                    GatewayCredential::load_sealed(&credential_path, &key, &policy, now)?
                        .generation()
                }
                Err(err) => return Err(err.into()),
            }
        }
        "rotate" => {
            let key = SealingKey::load(&seal_key_path, &policy)?;
            GatewayCredential::rotate_sealed(
                &credential_path,
                &key,
                material,
                &policy,
                now,
                not_after,
            )?
        }
        _ => return Err(usage()),
    };

    tracing::info!(action = %op, generation, "gateway credential lifecycle updated");
    println!(
        "{}",
        serde_json::json!({
            "status": op,
            "generation": generation,
        })
    );
    Ok(())
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
