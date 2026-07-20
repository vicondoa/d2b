use std::{env, ffi::OsString, path::PathBuf, process};

use d2b_guestd::{
    activation::{ACTIVATION_MAX_TIMEOUT_MS, ActivationRuntimeConfig},
    login_session::{WorkloadUserUid, classify_workload_user},
    production_guest::{ProductionExecConfig, ProductionGuestConfig, ProductionShellConfig},
    service::{
        DEFAULT_LOGIN_SHELL_PATH, DEFAULT_SEALED_IDENTITY_PATH, DEFAULT_SYSTEMD_CREDS_PATH,
        GuestdServeConfig, GuestdServiceError,
    },
};

fn main() {
    if let Err(error) = run(env::args_os().skip(1).collect()) {
        eprintln!("d2b-guestd: {}", error.public_message());
        process::exit(78);
    }
}

fn run(args: Vec<OsString>) -> Result<(), GuestdServiceError> {
    match args.first().and_then(|arg| arg.to_str()) {
        Some("--version") if args.len() == 1 => {
            println!("d2b-guestd {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--serve") => {
            let parsed = parse_serve_args(&args[1..])?;
            let production = parsed.production_config()?;
            let config = GuestdServeConfig::new(
                parsed.vm_id.clone(),
                parsed.sealed_identity_path,
                parsed.systemd_creds_path,
            )?
            .with_production(production)?
            .with_configured_launches_sha256(parsed.configured_launches_sha256);
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|_| GuestdServiceError::Transport)?;
            runtime.block_on(d2b_guestd::service::serve_vsock(config))
        }
        _ => Err(GuestdServiceError::InvalidConfiguration),
    }
}

struct ServeArgs {
    vm_id: String,
    workload_id: Option<String>,
    sealed_identity_path: PathBuf,
    systemd_creds_path: PathBuf,
    exec_enable: bool,
    exec_user: Option<String>,
    systemd_run_path: Option<PathBuf>,
    exec_runner_path: Option<PathBuf>,
    detached_max_runtime_sec: Option<u64>,
    interactive_max_runtime_sec: Option<u64>,
    guest_config_path: Option<PathBuf>,
    shell_enable: bool,
    shell_default_name: Option<String>,
    shell_max_sessions: Option<u32>,
    shell_max_attached: Option<u32>,
    shell_runner_path: Option<PathBuf>,
    shell_systemctl_path: Option<PathBuf>,
    shutdown_systemctl_path: Option<PathBuf>,
    activation_systemd_run_path: Option<PathBuf>,
    activation_systemctl_path: Option<PathBuf>,
    activation_status_dir: PathBuf,
    configured_launches_sha256: Option<[u8; 32]>,
}

impl ServeArgs {
    fn production_config(&self) -> Result<ProductionGuestConfig, GuestdServiceError> {
        let workload_id = self
            .workload_id
            .clone()
            .ok_or(GuestdServiceError::InvalidConfiguration)?;
        let exec = if self.exec_enable {
            Some(ProductionExecConfig {
                exec_user: self
                    .exec_user
                    .clone()
                    .ok_or(GuestdServiceError::InvalidConfiguration)?,
                systemd_run_path: self
                    .systemd_run_path
                    .clone()
                    .ok_or(GuestdServiceError::InvalidConfiguration)?,
                exec_runner_path: self
                    .exec_runner_path
                    .clone()
                    .ok_or(GuestdServiceError::InvalidConfiguration)?,
                login_shell_path: PathBuf::from(DEFAULT_LOGIN_SHELL_PATH),
                detached_max_runtime_sec: self
                    .detached_max_runtime_sec
                    .ok_or(GuestdServiceError::InvalidConfiguration)?,
                interactive_max_runtime_sec: self
                    .interactive_max_runtime_sec
                    .ok_or(GuestdServiceError::InvalidConfiguration)?,
            })
        } else {
            None
        };
        let shell = if self.shell_enable {
            let user = self
                .exec_user
                .as_deref()
                .ok_or(GuestdServiceError::InvalidConfiguration)?;
            let uid = match classify_workload_user(user) {
                WorkloadUserUid::NonRoot(uid) => uid,
                WorkloadUserUid::Root | WorkloadUserUid::Unresolved => {
                    return Err(GuestdServiceError::InvalidConfiguration);
                }
            };
            Some(ProductionShellConfig {
                default_name: self
                    .shell_default_name
                    .clone()
                    .ok_or(GuestdServiceError::InvalidConfiguration)?,
                max_sessions: self
                    .shell_max_sessions
                    .ok_or(GuestdServiceError::InvalidConfiguration)?,
                max_attached: self
                    .shell_max_attached
                    .ok_or(GuestdServiceError::InvalidConfiguration)?,
                runner_path: self
                    .shell_runner_path
                    .clone()
                    .ok_or(GuestdServiceError::InvalidConfiguration)?,
                systemctl_path: self
                    .shell_systemctl_path
                    .clone()
                    .ok_or(GuestdServiceError::InvalidConfiguration)?,
                socket_path: PathBuf::from(format!("/run/user/{uid}/d2b-shpool.sock")),
            })
        } else {
            None
        };
        let activation = match (
            self.activation_systemd_run_path.clone(),
            self.activation_systemctl_path.clone(),
        ) {
            (Some(systemd_run_path), Some(systemctl_path)) => Some(ActivationRuntimeConfig {
                workload_id: workload_id.clone(),
                systemd_run_path,
                systemctl_path,
                status_dir: self.activation_status_dir.clone(),
                switch_store_root: PathBuf::from("/nix/store"),
                max_timeout_ms: ACTIVATION_MAX_TIMEOUT_MS,
            }),
            (None, None) => None,
            _ => return Err(GuestdServiceError::InvalidConfiguration),
        };
        Ok(ProductionGuestConfig {
            workload_id,
            exec,
            shell,
            guest_config_path: self.guest_config_path.clone(),
            shutdown_systemctl_path: self.shutdown_systemctl_path.clone(),
            activation,
            configured_launches: Default::default(),
            configured_launch_realm_id: None,
            configured_launch_workload_digest: None,
            security_key: None,
        })
    }
}

fn parse_serve_args(args: &[OsString]) -> Result<ServeArgs, GuestdServiceError> {
    let mut iter = args.iter();
    let mut vm_id = None;
    let mut sealed_identity_path = PathBuf::from(DEFAULT_SEALED_IDENTITY_PATH);
    let mut systemd_creds_path = PathBuf::from(DEFAULT_SYSTEMD_CREDS_PATH);
    let mut workload_id = None;
    let mut exec_enable = false;
    let mut exec_user = None;
    let mut systemd_run_path = None;
    let mut exec_runner_path = None;
    let mut detached_max_runtime_sec = None;
    let mut interactive_max_runtime_sec = None;
    let mut guest_config_path = None;
    let mut shell_enable = false;
    let mut shell_default_name = None;
    let mut shell_max_sessions = None;
    let mut shell_max_attached = None;
    let mut shell_runner_path = None;
    let mut shell_systemctl_path = None;
    let mut shutdown_systemctl_path = None;
    let mut activation_systemd_run_path = None;
    let mut activation_systemctl_path = None;
    let mut activation_status_dir = PathBuf::from("/run/d2b-guestd/activations");
    let mut configured_launches_sha256 = None;
    while let Some(arg) = iter.next() {
        match arg.to_str() {
            Some("--vm-id") => {
                vm_id = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .map(str::to_owned);
            }
            Some("--workload-id") => workload_id = Some(string_value(iter.next())?),
            Some("--sealed-identity-path") => {
                sealed_identity_path = absolute_path(iter.next())?;
            }
            Some("--systemd-creds-path") => {
                systemd_creds_path = absolute_path(iter.next())?;
            }
            Some("--exec-enable") => exec_enable = true,
            Some("--shell-enable") => shell_enable = true,
            Some("--exec-user") => exec_user = Some(string_value(iter.next())?),
            Some("--systemd-run-path") => systemd_run_path = Some(absolute_path(iter.next())?),
            Some("--exec-runner-path") => exec_runner_path = Some(absolute_path(iter.next())?),
            Some("--detached-max-runtime-sec") => {
                detached_max_runtime_sec = Some(number_value(iter.next())?)
            }
            Some("--interactive-max-runtime-sec") => {
                interactive_max_runtime_sec = Some(number_value(iter.next())?)
            }
            Some("--guest-config-path") => guest_config_path = Some(absolute_path(iter.next())?),
            Some("--shell-default-name") => shell_default_name = Some(string_value(iter.next())?),
            Some("--shell-max-sessions") => shell_max_sessions = Some(number_value(iter.next())?),
            Some("--shell-max-attached") => shell_max_attached = Some(number_value(iter.next())?),
            Some("--shell-runner-path") => shell_runner_path = Some(absolute_path(iter.next())?),
            Some("--shell-systemctl-path") => {
                shell_systemctl_path = Some(absolute_path(iter.next())?)
            }
            Some("--activation-systemctl-path") => {
                let path = absolute_path(iter.next())?;
                shutdown_systemctl_path = Some(path.clone());
                activation_systemctl_path = Some(path);
            }
            Some("--activation-systemd-run-path") => {
                activation_systemd_run_path = Some(absolute_path(iter.next())?)
            }
            Some("--activation-status-dir") => activation_status_dir = absolute_path(iter.next())?,
            Some("--configured-launches-sha256") => {
                configured_launches_sha256 = Some(digest_value(iter.next())?)
            }
            Some("--usbip-path" | "--wpctl-path") => {
                let _ = string_value(iter.next())?;
            }
            _ => return Err(GuestdServiceError::InvalidConfiguration),
        }
    }
    Ok(ServeArgs {
        vm_id: vm_id.ok_or(GuestdServiceError::InvalidConfiguration)?,
        workload_id,
        sealed_identity_path,
        systemd_creds_path,
        exec_enable,
        exec_user,
        systemd_run_path,
        exec_runner_path,
        detached_max_runtime_sec,
        interactive_max_runtime_sec,
        guest_config_path,
        shell_enable,
        shell_default_name,
        shell_max_sessions,
        shell_max_attached,
        shell_runner_path,
        shell_systemctl_path,
        shutdown_systemctl_path,
        activation_systemd_run_path,
        activation_systemctl_path,
        activation_status_dir,
        configured_launches_sha256,
    })
}

fn string_value(value: Option<&OsString>) -> Result<String, GuestdServiceError> {
    value
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or(GuestdServiceError::InvalidConfiguration)
}

fn number_value<T: std::str::FromStr>(value: Option<&OsString>) -> Result<T, GuestdServiceError> {
    string_value(value)?
        .parse()
        .map_err(|_| GuestdServiceError::InvalidConfiguration)
}

fn digest_value(value: Option<&OsString>) -> Result<[u8; 32], GuestdServiceError> {
    let value = string_value(value)?;
    if value.len() != 64 {
        return Err(GuestdServiceError::InvalidConfiguration);
    }
    let mut digest = [0_u8; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| GuestdServiceError::InvalidConfiguration)?;
    }
    if digest == [0; 32] {
        return Err(GuestdServiceError::InvalidConfiguration);
    }
    Ok(digest)
}

fn absolute_path(value: Option<&OsString>) -> Result<PathBuf, GuestdServiceError> {
    let path = value
        .map(PathBuf::from)
        .ok_or(GuestdServiceError::InvalidConfiguration)?;
    if !path.is_absolute() {
        return Err(GuestdServiceError::InvalidConfiguration);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_canonical_workload_and_digest_pinned_inventory_handoff() {
        let parsed = parse_serve_args(&args(&[
            "--vm-id",
            "work",
            "--workload-id",
            "bbbbbbbbbbbbbbbbbbba",
            "--configured-launches-sha256",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ]))
        .unwrap();
        assert_eq!(parsed.workload_id.as_deref(), Some("bbbbbbbbbbbbbbbbbbba"));
        assert_eq!(parsed.configured_launches_sha256, Some([0xaa; 32]));
    }

    #[test]
    fn rejects_inventory_path_and_noncanonical_digest_inputs() {
        assert!(
            parse_serve_args(&args(&[
                "--vm-id",
                "work",
                "--workload-id",
                "bbbbbbbbbbbbbbbbbbba",
                "--configured-launches-path",
                "/run/ambient",
            ]))
            .is_err()
        );
        assert!(
            parse_serve_args(&args(&[
                "--vm-id",
                "work",
                "--workload-id",
                "bbbbbbbbbbbbbbbbbbba",
                "--configured-launches-sha256",
                "not-a-digest",
            ]))
            .is_err()
        );
    }
}
