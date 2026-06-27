//! Live SSH-key fingerprint + public-key probe.
//!
//! Pure-ish module that shells out to `ssh-keygen -lf <path>` and
//! `ssh-keygen -y -f <path>` for the public-fingerprint and
//! public-key extraction respectively. Returns structured results for
//! the `d2b keys show` CLI surface.
//!
//! The broker is the only caller with read access to the per-VM
//! private key at `<d2b.site.keysDir>/<vm>_ed25519` (0640
//! root:d2b per `nixos-modules/host-keys.nix`); this
//! module's input is therefore expected to be the PUBLIC key path
//! (`<keysDir>/<vm>_ed25519.pub`) for `d2b keys show`, and the
//! private key path only for the broker-side rotate op which runs
//! as root.
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Stdio};

/// Output of a successful `ssh-keygen -lf` call. The fingerprint
/// is the second whitespace-separated field; `bits` is the first
/// numeric field; `key_type` is the trailing parenthesized type
/// (`(ED25519)`, `(RSA)`, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyFingerprint {
    /// Bit length (e.g. 256 for ed25519, 4096 for RSA).
    pub bits: u32,
    /// `SHA256:<base64>` style fingerprint (the modern default).
    pub fingerprint: String,
    /// Comment field if present (often the VM name or operator
    /// email); empty when absent.
    pub comment: String,
    /// Key type as ssh-keygen reports it: `ED25519`, `RSA`,
    /// `ECDSA`, etc.
    pub key_type: String,
}

/// Errors the probe can surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum SshKeygenError {
    SshKeygenMissing { detail: String },
    KeyPathMissing { path: String, detail: String },
    SshKeygenFailed { exit_code: i32, stderr: String },
    OutputUnparseable { stdout: String },
    EmptyPublicKey,
    InvalidSshKeygenBinaryPath { path: String },
    InvalidKeyPath { path: String },
}

impl std::fmt::Display for SshKeygenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SshKeygenMissing { detail } => {
                write!(f, "ssh-keygen binary not available: {detail}")
            }
            Self::KeyPathMissing { path, detail } => {
                write!(f, "key path {path} unreadable: {detail}")
            }
            Self::SshKeygenFailed { exit_code, stderr } => {
                write!(f, "ssh-keygen exited {exit_code}: {stderr}")
            }
            Self::OutputUnparseable { stdout } => {
                write!(
                    f,
                    "ssh-keygen output did not match expected shape: {stdout:?}"
                )
            }
            Self::EmptyPublicKey => f.write_str("ssh-keygen -y emitted empty public key"),
            Self::InvalidSshKeygenBinaryPath { path } => {
                write!(f, "ssh-keygen binary path {path:?} must be absolute")
            }
            Self::InvalidKeyPath { path } => {
                write!(f, "key path {path:?} must be absolute")
            }
        }
    }
}

impl std::error::Error for SshKeygenError {}

/// Run `ssh-keygen -lf <key_path>` and parse the result.
pub fn probe_fingerprint(
    ssh_keygen_binary_path: &Path,
    key_path: &Path,
) -> Result<SshKeyFingerprint, SshKeygenError> {
    if !ssh_keygen_binary_path
        .to_str()
        .map(|s| s.starts_with('/'))
        .unwrap_or(false)
    {
        return Err(SshKeygenError::InvalidSshKeygenBinaryPath {
            path: ssh_keygen_binary_path.display().to_string(),
        });
    }
    if !key_path
        .to_str()
        .map(|s| s.starts_with('/'))
        .unwrap_or(false)
    {
        return Err(SshKeygenError::InvalidKeyPath {
            path: key_path.display().to_string(),
        });
    }
    if !key_path.exists() {
        return Err(SshKeygenError::KeyPathMissing {
            path: key_path.display().to_string(),
            detail: "ENOENT".to_owned(),
        });
    }
    let output = Command::new(ssh_keygen_binary_path)
        .arg("-lf")
        .arg(key_path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| SshKeygenError::SshKeygenMissing {
            detail: e.to_string(),
        })?;
    if !output.status.success() {
        return Err(SshKeygenError::SshKeygenFailed {
            exit_code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    parse_ssh_keygen_lf(&String::from_utf8_lossy(&output.stdout))
}

/// Parse the `ssh-keygen -lf` output line into a structured
/// [`SshKeyFingerprint`]. Pure — testable without invoking
/// `ssh-keygen`.
///
/// Output shape (one line, fields whitespace-separated):
///
/// `<bits> <fingerprint> <comment> (<KEY_TYPE>)`
pub fn parse_ssh_keygen_lf(stdout: &str) -> Result<SshKeyFingerprint, SshKeygenError> {
    let line = stdout.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return Err(SshKeygenError::OutputUnparseable {
            stdout: stdout.to_owned(),
        });
    }
    let (rest, key_type) = match (line.rfind('('), line.rfind(')')) {
        (Some(open), Some(close)) if open < close && close == line.len() - 1 => (
            line[..open].trim_end(),
            line[open + 1..close].trim().to_owned(),
        ),
        _ => {
            return Err(SshKeygenError::OutputUnparseable {
                stdout: stdout.to_owned(),
            });
        }
    };
    let mut parts = rest.splitn(3, char::is_whitespace);
    let bits_str = parts.next().unwrap_or("");
    let fingerprint = parts.next().unwrap_or("");
    let comment = parts.next().unwrap_or("").trim().to_owned();
    let bits = bits_str
        .parse::<u32>()
        .map_err(|_| SshKeygenError::OutputUnparseable {
            stdout: stdout.to_owned(),
        })?;
    if fingerprint.is_empty() {
        return Err(SshKeygenError::OutputUnparseable {
            stdout: stdout.to_owned(),
        });
    }
    Ok(SshKeyFingerprint {
        bits,
        fingerprint: fingerprint.to_owned(),
        comment,
        key_type,
    })
}

/// Run `ssh-keygen -y -f <private_key_path>` to extract the
/// public-key line. Broker-side only: the private key path is
/// root-readable only.
pub fn probe_public_key(
    ssh_keygen_binary_path: &Path,
    private_key_path: &Path,
) -> Result<String, SshKeygenError> {
    if !ssh_keygen_binary_path
        .to_str()
        .map(|s| s.starts_with('/'))
        .unwrap_or(false)
    {
        return Err(SshKeygenError::InvalidSshKeygenBinaryPath {
            path: ssh_keygen_binary_path.display().to_string(),
        });
    }
    if !private_key_path
        .to_str()
        .map(|s| s.starts_with('/'))
        .unwrap_or(false)
    {
        return Err(SshKeygenError::InvalidKeyPath {
            path: private_key_path.display().to_string(),
        });
    }
    if !private_key_path.exists() {
        return Err(SshKeygenError::KeyPathMissing {
            path: private_key_path.display().to_string(),
            detail: "ENOENT".to_owned(),
        });
    }
    let output = Command::new(ssh_keygen_binary_path)
        .arg("-y")
        .arg("-f")
        .arg(private_key_path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| SshKeygenError::SshKeygenMissing {
            detail: e.to_string(),
        })?;
    if !output.status.success() {
        return Err(SshKeygenError::SshKeygenFailed {
            exit_code: output.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    let pubkey = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if pubkey.is_empty() {
        return Err(SshKeygenError::EmptyPublicKey);
    }
    Ok(pubkey)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_modern_ed25519_output() {
        let out = "256 SHA256:abcdef0123456789abcdef0123456789ABCDEFGHIJK corp-vm@host (ED25519)\n";
        let parsed = parse_ssh_keygen_lf(out).unwrap();
        assert_eq!(parsed.bits, 256);
        assert_eq!(
            parsed.fingerprint,
            "SHA256:abcdef0123456789abcdef0123456789ABCDEFGHIJK"
        );
        assert_eq!(parsed.comment, "corp-vm@host");
        assert_eq!(parsed.key_type, "ED25519");
    }

    #[test]
    fn parses_rsa_4096_output_with_email_comment() {
        let out = "4096 SHA256:xyz user@example.com (RSA)\n";
        let parsed = parse_ssh_keygen_lf(out).unwrap();
        assert_eq!(parsed.bits, 4096);
        assert_eq!(parsed.fingerprint, "SHA256:xyz");
        assert_eq!(parsed.comment, "user@example.com");
        assert_eq!(parsed.key_type, "RSA");
    }

    #[test]
    fn parses_output_with_spaces_in_comment() {
        let out = "256 SHA256:abc Multi Word Comment Here (ED25519)\n";
        let parsed = parse_ssh_keygen_lf(out).unwrap();
        assert_eq!(parsed.comment, "Multi Word Comment Here");
    }

    #[test]
    fn parses_output_with_no_comment() {
        let out = "256 SHA256:abc no-comment (ED25519)\n";
        let parsed = parse_ssh_keygen_lf(out).unwrap();
        assert_eq!(parsed.comment, "no-comment");
    }

    #[test]
    fn rejects_empty_output() {
        assert!(matches!(
            parse_ssh_keygen_lf(""),
            Err(SshKeygenError::OutputUnparseable { .. })
        ));
    }

    #[test]
    fn rejects_output_missing_type_parens() {
        let out = "256 SHA256:abc corp-vm\n";
        assert!(matches!(
            parse_ssh_keygen_lf(out),
            Err(SshKeygenError::OutputUnparseable { .. })
        ));
    }

    #[test]
    fn rejects_output_with_non_numeric_bits() {
        let out = "ABC SHA256:abc corp-vm (ED25519)\n";
        assert!(matches!(
            parse_ssh_keygen_lf(out),
            Err(SshKeygenError::OutputUnparseable { .. })
        ));
    }

    #[test]
    fn probe_fingerprint_rejects_non_absolute_binary_path() {
        let result = probe_fingerprint(
            &PathBuf::from("ssh-keygen"),
            &PathBuf::from("/etc/ssh/some.pub"),
        );
        assert!(matches!(
            result,
            Err(SshKeygenError::InvalidSshKeygenBinaryPath { .. })
        ));
    }

    #[test]
    fn probe_fingerprint_rejects_non_absolute_key_path() {
        let result = probe_fingerprint(
            &PathBuf::from("/usr/bin/ssh-keygen"),
            &PathBuf::from("key.pub"),
        );
        assert!(matches!(result, Err(SshKeygenError::InvalidKeyPath { .. })));
    }

    #[test]
    fn probe_fingerprint_rejects_missing_key_path() {
        let result = probe_fingerprint(
            &PathBuf::from("/usr/bin/ssh-keygen"),
            &PathBuf::from("/tmp/nonexistent-d2b-test-key.pub"),
        );
        assert!(matches!(result, Err(SshKeygenError::KeyPathMissing { .. })));
    }

    #[test]
    fn fingerprint_round_trip_serializable() {
        let f = SshKeyFingerprint {
            bits: 256,
            fingerprint: "SHA256:abc".to_owned(),
            comment: "corp-vm".to_owned(),
            key_type: "ED25519".to_owned(),
        };
        let json = serde_json::to_string(&f).unwrap();
        let parsed: SshKeyFingerprint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, f);
    }
}
