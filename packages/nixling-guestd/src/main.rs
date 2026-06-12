use std::{env, ffi::OsString, path::PathBuf, process};

use nixling_guestd::exec::ExecPolicy;
use nixling_guestd::service::DetachedRuntimeConfig;

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
            config =
                config.with_interactive_max_runtime_sec(parsed.interactive_max_runtime_sec);
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
    while let Some(arg) = iter.next() {
        match arg.to_str() {
            Some("--vm-id") => {
                vm_id = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .map(str::to_owned);
            }
            Some("--exec-enable") => policy.enabled = true,
            Some("--exec-allow-root") => policy.allow_root = true,
            Some("--systemd-run-path") => {
                systemd_run_path = Some(parse_abs_path(iter.next())?);
            }
            Some("--exec-runner-path") => {
                exec_runner_path = Some(parse_abs_path(iter.next())?);
            }
            Some("--detached-max-runtime-sec") => {
                detached_max_runtime_sec = iter
                    .next()
                    .and_then(|value| value.to_str())
                    .and_then(|value| value.parse::<u64>().ok())
                    .ok_or(nixling_guestd::service::GuestdServiceError::Ttrpc)?;
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

    Ok(ServeArgs {
        vm_id,
        exec_policy: policy,
        detached,
        interactive_max_runtime_sec,
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
