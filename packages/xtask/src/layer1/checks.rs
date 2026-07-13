use std::{
    env,
    path::Path,
    process::{Command, Output},
};

use super::{Layer1Error, Result};

pub fn discover_check_json(root: &Path, requested_system: Option<&str>) -> Result<String> {
    let system = match requested_system {
        Some(system) => {
            validate_system(system)?;
            system.to_owned()
        }
        None => discover_native_system()?,
    };
    eprintln!("test-flake-list: enumerating checks.{system}.*");
    let flake_ref = format!("git+file://{}", root.display());
    let output = nix_command()
        .args([
            "eval",
            "--json",
            &format!("{flake_ref}#checks.{system}"),
            "--apply",
            "builtins.attrNames",
        ])
        .output()
        .map_err(|error| Layer1Error::new(format!("could not execute nix eval: {error}")))?;
    relay_stderr(&output);
    if !output.status.success() {
        return Err(Layer1Error::new(format!(
            "nix eval failed while discovering checks.{system} (exit {})",
            output.status.code().unwrap_or(1)
        )));
    }
    normalize_check_json(&output.stdout)
}

fn discover_native_system() -> Result<String> {
    let output = nix_command()
        .args([
            "eval",
            "--raw",
            "--impure",
            "--expr",
            "builtins.currentSystem",
        ])
        .output()
        .map_err(|error| Layer1Error::new(format!("could not execute nix eval: {error}")))?;
    if !output.status.success() {
        return Err(Layer1Error::new(format!(
            "nix eval builtins.currentSystem failed (exit {}): {}",
            output.status.code().unwrap_or(1),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let system = String::from_utf8(output.stdout)
        .map_err(|_| Layer1Error::new("nix current-system output was not UTF-8"))?;
    let system = system.trim();
    validate_system(system)?;
    Ok(system.to_owned())
}

fn nix_command() -> Command {
    let mut command = Command::new("nix");
    if env::var("NIX_CONFIG").map_or(true, |value| value.is_empty()) {
        command.env("NIX_CONFIG", "experimental-features = nix-command flakes");
    }
    command
}

fn relay_stderr(output: &Output) {
    if !output.stderr.is_empty() {
        eprint!("{}", String::from_utf8_lossy(&output.stderr));
    }
}

fn normalize_check_json(bytes: &[u8]) -> Result<String> {
    let checks: Vec<String> = serde_json::from_slice(bytes)
        .map_err(|error| Layer1Error::new(format!("invalid flake-check JSON: {error}")))?;
    if checks.is_empty() {
        return Err(Layer1Error::new(
            "flake check discovery returned an empty list",
        ));
    }
    let mut previous: Option<&str> = None;
    for check in &checks {
        validate_check_name(check)?;
        if let Some(previous) = previous
            && check.as_str() <= previous
        {
            return Err(Layer1Error::new(
                "flake check discovery output must be sorted and unique",
            ));
        }
        previous = Some(check);
    }
    serde_json::to_string(&checks)
        .map_err(|error| Layer1Error::new(format!("cannot encode flake-check JSON: {error}")))
}

fn validate_system(system: &str) -> Result<()> {
    if system.is_empty()
        || !system
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(Layer1Error::new(format!(
            "invalid Nix system name {system:?}"
        )));
    }
    Ok(())
}

fn validate_check_name(check: &str) -> Result<()> {
    if check.is_empty()
        || !check
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(Layer1Error::new(format!(
            "invalid flake check name {check:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_output_is_compact_sorted_json() {
        assert_eq!(
            normalize_check_json(br#"["alpha","nix-unit-1","z.last"]"#).unwrap(),
            r#"["alpha","nix-unit-1","z.last"]"#
        );
    }

    #[test]
    fn malformed_or_unsafe_check_output_fails_closed() {
        assert!(normalize_check_json(br#"{"alpha":true}"#).is_err());
        assert!(normalize_check_json(br#"["z","a"]"#).is_err());
        assert!(normalize_check_json(br#"["same","same"]"#).is_err());
        assert!(normalize_check_json(br#"["$(command)"]"#).is_err());
        assert!(normalize_check_json(br#"[]"#).is_err());
    }
}
