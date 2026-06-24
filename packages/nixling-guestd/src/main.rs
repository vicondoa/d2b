use std::{env, ffi::OsString, path::PathBuf, process};

use nixling_guestd::exec::ExecPolicy;
use nixling_guestd::service::{ActivationRuntimeConfig, DetachedRuntimeConfig, ShellPolicy};

fn main() {
    if let Err(error) = run(env::args_os().skip(1).collect()) {
        eprintln!("nixling-guestd: {}", error.public_message());
        process::exit(78);
    }
}

fn run(args: Vec<OsString>) -> Result<(), nixling_guestd::service::GuestdServiceError> {
    match args.first().and_then(|arg| arg.to_str()) {
        Some("--version") if args.len() == 1 => {
            println!("nixling-guestd {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--serve") => {
            let parsed = parse_serve_args(&args[1..])?;
            let token = nixling_guestd::service::load_token_from_credentials_env()?;
            let mut config = nixling_guestd::service::GuestdServeConfig::with_exec_policy(
                parsed.vm_id,
                token,
                parsed.exec_policy,
            )?;
            if let Some(detached) = parsed.detached {
                config = config.with_detached(detached);
            }
            config = config.with_interactive_max_runtime_sec(parsed.interactive_max_runtime_sec);
            if let Some(guest_config_path) = parsed.guest_config_path {
                config = config.with_guest_config_path(guest_config_path);
            }
            if let Some(usbip_path) = parsed.usbip_path {
                config = config.with_usbip_path(usbip_path);
            }
            if let Some(shell_policy) = parsed.shell_policy {
                config = config.with_shell_policy(shell_policy);
            }
            if let Some(activation) = parsed.activation {
                config = config.with_activation_runtime(activation);
            }
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|_| nixling_guestd::service::GuestdServiceError::Ttrpc)?;
            runtime.block_on(nixling_guestd::service::serve_vsock(config))
        }
        _ => Err(nixling_guestd::service::GuestdServiceError::Ttrpc),
    }
}

/// Parsed `--serve` arguments.
struct ServeArgs {
    vm_id: String,
    exec_policy: ExecPolicy,
    detached: Option<DetachedRuntimeConfig>,
    interactive_max_runtime_sec: u64,
    guest_config_path: Option<PathBuf>,
    usbip_path: Option<PathBuf>,
    shell_policy: Option<ShellPolicy>,
    activation: Option<ActivationRuntimeConfig>,
}

/// Parse `--serve` arguments: the required `--vm-id <name>` plus the optional
/// host-owned exec policy + detached runtime flags. The policy is fail-closed
/// by default. Detached exec is enabled only when BOTH `--systemd-run-path` and
/// `--exec-runner-path` are provided as absolute paths.
fn parse_serve_args(
    args: &[OsString],
) -> Result<ServeArgs, nixling_guestd::service::GuestdServiceError> {
    let mut iter = args.iter();
    let mut vm_id = None;
    let mut policy = ExecPolicy::disabled();
    let mut systemd_run_path: Option<PathBuf> = None;
    let mut exec_runner_path: Option<PathBuf> = None;
    let mut detached_max_runtime_sec: u64 = 0;
    let mut interactive_max_runtime_sec: u64 = 0;
    let mut guest_config_path: Option<PathBuf> = None;
    let mut usbip_path: Option<PathBuf> = None;
    let mut shell_enabled = false;
    let mut shell_default_name = String::from("default");
    let mut shell_max_sessions: u32 = 8;
    let mut shell_max_attached: u32 = 1;
    let mut shell_runner_path: Option<PathBuf> = None;
    let mut shell_systemctl_path: Option<PathBuf> = None;
    let mut activation_systemd_run_path: Option<PathBuf> = None;
    let mut activation_systemctl_path: Option<PathBuf> = None;
    let mut activation_status_dir: PathBuf = PathBuf::from("/run/nixling-guestd/activations");
    while let Some(arg) = iter.next() {
        match arg.to_str() {
            Some("--vm-id") => {
                vm_id = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .map(str::to_owned);
            }
            Some("--exec-enable") => policy.enabled = true,
            Some("--exec-user") => {
                let user = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .map(str::to_owned)
                    .filter(|value| is_valid_workload_user(value))
                    .ok_or(nixling_guestd::service::GuestdServiceError::Ttrpc)?;
                policy.exec_user = Some(user);
            }
            Some("--systemd-run-path") => {
                systemd_run_path = Some(parse_abs_path(iter.next())?);
            }
            Some("--exec-runner-path") => {
                exec_runner_path = Some(parse_abs_path(iter.next())?);
            }
            Some("--guest-config-path") => {
                guest_config_path = Some(parse_abs_path(iter.next())?);
            }
            Some("--usbip-path") => {
                usbip_path = Some(parse_abs_path(iter.next())?);
            }
            Some("--shell-enable") => shell_enabled = true,
            Some("--shell-default-name") => {
                shell_default_name = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .map(str::to_owned)
                    .filter(|value| is_valid_shell_name(value))
                    .ok_or(nixling_guestd::service::GuestdServiceError::Ttrpc)?;
            }
            Some("--shell-max-sessions") => {
                shell_max_sessions = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|value| (1..=256).contains(value))
                    .ok_or(nixling_guestd::service::GuestdServiceError::Ttrpc)?;
            }
            Some("--shell-max-attached") => {
                shell_max_attached = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .and_then(|value| value.parse::<u32>().ok())
                    .filter(|value| (1..=64).contains(value))
                    .ok_or(nixling_guestd::service::GuestdServiceError::Ttrpc)?;
            }
            Some("--shell-runner-path") => {
                shell_runner_path = Some(parse_abs_path(iter.next())?);
            }
            Some("--shell-systemctl-path") => {
                shell_systemctl_path = Some(parse_abs_path(iter.next())?);
            }
            Some("--detached-max-runtime-sec") => {
                detached_max_runtime_sec = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .and_then(|value| value.parse::<u64>().ok())
                    .ok_or(nixling_guestd::service::GuestdServiceError::Ttrpc)?;
            }
            Some("--activation-systemd-run-path") => {
                activation_systemd_run_path = Some(parse_abs_path(iter.next())?);
            }
            Some("--activation-systemctl-path") => {
                activation_systemctl_path = Some(parse_abs_path(iter.next())?);
            }
            Some("--activation-status-dir") => {
                activation_status_dir = parse_abs_path(iter.next())?;
            }
            Some("--interactive-max-runtime-sec") => {
                interactive_max_runtime_sec = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .and_then(|value| value.parse::<u64>().ok())
                    .ok_or(nixling_guestd::service::GuestdServiceError::Ttrpc)?;
            }
            _ => return Err(nixling_guestd::service::GuestdServiceError::Ttrpc),
        }
    }
    let vm_id = vm_id
        .filter(|value| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
        .ok_or(nixling_guestd::service::GuestdServiceError::Ttrpc)?;

    let detached = match (systemd_run_path, exec_runner_path) {
        (Some(systemd_run_path), Some(exec_runner_path)) => Some(DetachedRuntimeConfig {
            systemd_run_path,
            exec_runner_path,
            max_runtime_sec: detached_max_runtime_sec,
        }),
        (None, None) => None,
        // A half-configured detached path is fail-closed (both or neither).
        _ => return Err(nixling_guestd::service::GuestdServiceError::Ttrpc),
    };
    let shell_policy = if shell_enabled {
        if shell_max_attached > shell_max_sessions {
            return Err(nixling_guestd::service::GuestdServiceError::Ttrpc);
        }
        Some(ShellPolicy {
            enabled: true,
            default_name: shell_default_name,
            max_sessions: shell_max_sessions,
            max_attached: shell_max_attached,
            runner_path: shell_runner_path,
            systemctl_path: shell_systemctl_path,
        })
    } else {
        None
    };
    let activation = match (activation_systemd_run_path, activation_systemctl_path) {
        (Some(systemd_run_path), Some(systemctl_path)) => Some(ActivationRuntimeConfig {
            systemd_run_path,
            systemctl_path,
            status_dir: activation_status_dir,
            max_timeout_ms: 60 * 60 * 1_000,
        }),
        (None, None) => None,
        _ => return Err(nixling_guestd::service::GuestdServiceError::Ttrpc),
    };

    Ok(ServeArgs {
        vm_id,
        exec_policy: policy,
        detached,
        interactive_max_runtime_sec,
        guest_config_path,
        usbip_path,
        shell_policy,
        activation,
    })
}

/// Accept only a present, absolute path value for a path-valued flag.
fn parse_abs_path(
    value: Option<&OsString>,
) -> Result<PathBuf, nixling_guestd::service::GuestdServiceError> {
    let path = value
        .map(PathBuf::from)
        .ok_or(nixling_guestd::service::GuestdServiceError::Ttrpc)?;
    if !path.is_absolute() {
        return Err(nixling_guestd::service::GuestdServiceError::Ttrpc);
    }
    Ok(path)
}

/// A valid workload-user name for `--exec-user`: a non-empty POSIX-ish account
/// name that is never `root`. The host (`guest-control.nix`) already asserts the
/// user exists in the guest passwd at eval time; this is the fail-closed
/// runtime guard against an empty/`root`/malformed value reaching the spawn.
fn is_valid_workload_user(value: &str) -> bool {
    !value.is_empty()
        && value != "root"
        && value.len() <= 32
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte == b'_')
}

fn is_valid_shell_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    let first = bytes[0];
    (first.is_ascii_alphanumeric() || first == b'_')
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
