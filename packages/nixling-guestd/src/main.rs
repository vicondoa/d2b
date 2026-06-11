use std::{env, ffi::OsString, process};

use nixling_guestd::exec::ExecPolicy;

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
            let (vm_id, exec_policy) = parse_serve_args(&args[1..])?;
            let token = nixling_guestd::service::load_token_from_credentials_env()?;
            let config = nixling_guestd::service::GuestdServeConfig::with_exec_policy(
                vm_id,
                token,
                exec_policy,
            )?;
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|_| nixling_guestd::service::GuestdServiceError::Ttrpc)?;
            runtime.block_on(nixling_guestd::service::serve_vsock(config))
        }
        _ => Err(nixling_guestd::service::GuestdServiceError::Ttrpc),
    }
}

/// Parse `--serve` arguments: the required `--vm-id <name>` plus the optional
/// host-owned exec policy flags. The policy is fail-closed by default.
fn parse_serve_args(
    args: &[OsString],
) -> Result<(String, ExecPolicy), nixling_guestd::service::GuestdServiceError> {
    let mut iter = args.iter();
    let mut vm_id = None;
    let mut policy = ExecPolicy::disabled();
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
    Ok((vm_id, policy))
}
