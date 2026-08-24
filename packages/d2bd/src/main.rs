use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use d2bd::{
    DEFAULT_CONFIG_PATH, GuestServeOptions, LockOnlyOptions, ServeOptions, TestClientOptions,
    banner, banner_note, lock_only, run_test_client, serve, serve_guest,
};

#[derive(Debug, Parser)]
#[command(name = "d2bd", about = "d2b daemon skeleton")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the fixed Host authority mode.
    Host(HostArgs),
    /// Run the fixed Guest target-agent mode.
    Guest(GuestArgs),
    Serve {
        #[arg(long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        #[arg(long)]
        public_socket: Option<PathBuf>,
        #[arg(long)]
        broker_socket: Option<PathBuf>,
        #[arg(long)]
        state_lock: Option<PathBuf>,
        #[arg(long)]
        locks_dir: Option<PathBuf>,
        #[arg(long)]
        once: bool,
        #[arg(long, hide = true)]
        test_listen_on: Option<PathBuf>,
        #[arg(long, hide = true)]
        allow_unprivileged_runtime_dir: bool,
        #[arg(long)]
        no_drop_privileges: bool,
        #[arg(long, hide = true)]
        daemon_state_dir: Option<PathBuf>,
        #[arg(long, hide = true)]
        test_state_restore_report: Option<PathBuf>,
    },
    LockOnly {
        #[arg(long, default_value = DEFAULT_CONFIG_PATH)]
        config: PathBuf,
        #[arg(long)]
        state_lock: Option<PathBuf>,
        #[arg(long, default_value_t = 30)]
        hold_seconds: u64,
        #[arg(long, hide = true)]
        allow_unprivileged_runtime_dir: bool,
    },
    TestClient {
        #[arg(long)]
        socket: PathBuf,
        #[arg(long = "frame-json", required = true)]
        frame_json: Vec<String>,
    },
}

#[derive(Debug, Clone, Args)]
struct HostArgs {
    #[arg(long, default_value = DEFAULT_CONFIG_PATH)]
    config: PathBuf,
    #[arg(long)]
    public_socket: Option<PathBuf>,
    #[arg(long)]
    broker_socket: Option<PathBuf>,
    #[arg(long)]
    state_lock: Option<PathBuf>,
    #[arg(long)]
    locks_dir: Option<PathBuf>,
    #[arg(long)]
    once: bool,
    #[arg(long, hide = true)]
    test_listen_on: Option<PathBuf>,
    #[arg(long, hide = true)]
    allow_unprivileged_runtime_dir: bool,
    #[arg(long)]
    no_drop_privileges: bool,
    #[arg(long, hide = true)]
    daemon_state_dir: Option<PathBuf>,
    #[arg(long, hide = true)]
    test_state_restore_report: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
struct GuestArgs {
    #[arg(long)]
    guest_ref: String,
    #[arg(long)]
    guest_uid: String,
    #[arg(long)]
    zone: String,
    #[arg(long, default_value = "zone-link")]
    purpose: String,
    #[arg(long)]
    schema_fingerprint: String,
    #[arg(long, default_value_t = 1)]
    reconnect_generation: u64,
    #[arg(long, default_value_t = 1)]
    provider_generation: u64,
    #[arg(long, default_value_t = 1)]
    controller_generation: u64,
    #[arg(long, default_value_t = 1)]
    assignment_epoch: u64,
    #[arg(long, default_value = "/run/d2b/guest-broker.sock")]
    broker_socket: PathBuf,
    #[arg(long, default_value_t = 997)]
    broker_uid: u32,
    #[arg(long, default_value = "/var/lib/d2b/guest-state")]
    state_dir: PathBuf,
    #[arg(long, hide = true, default_value = "/etc/d2b/guest-bundle.json")]
    bundle_path: PathBuf,
    #[arg(long, default_value = "/var/lib/d2b-guest/guest-config.nix")]
    guest_config_path: PathBuf,
    #[arg(long, hide = true, default_value = "/proc/sys/kernel/random/boot_id")]
    boot_id_path: PathBuf,
    #[arg(long, hide = true)]
    local_private_key: Option<PathBuf>,
    #[arg(long, hide = true)]
    parent_public_key: Option<PathBuf>,
    #[arg(long, hide = true)]
    validate_only: bool,
    #[arg(long, hide = true)]
    once: bool,
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    // v1.1.1 live-deploy fu9: route tracing to stderr so
    // RUST_LOG controls visibility under systemd.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let result = match cli.command {
        None => {
            println!("{}", banner());
            println!("{}", banner_note());
            Ok(())
        }
        Some(Command::Host(args)) => {
            serve(ServeOptions {
                config_path: args.config,
                public_socket_path: args.test_listen_on.or(args.public_socket),
                broker_socket_path: args.broker_socket,
                state_lock_path: args.state_lock,
                locks_dir: args.locks_dir,
                once: args.once,
                allow_unprivileged_runtime_dir: args.allow_unprivileged_runtime_dir,
                drop_privileges: !args.no_drop_privileges,
                daemon_state_dir: args.daemon_state_dir,
                test_state_restore_report_path: args.test_state_restore_report,
            })
            .await
        }
        Some(Command::Guest(args)) => {
            serve_guest(GuestServeOptions {
                guest_ref: args.guest_ref,
                guest_uid: args.guest_uid,
                zone: args.zone,
                purpose: args.purpose,
                schema_fingerprint: args.schema_fingerprint,
                reconnect_generation: args.reconnect_generation,
                provider_generation: args.provider_generation,
                controller_generation: args.controller_generation,
                assignment_epoch: args.assignment_epoch,
                broker_socket_path: args.broker_socket,
                broker_uid: args.broker_uid,
                state_dir: args.state_dir,
                bundle_path: args.bundle_path,
                guest_config_path: args.guest_config_path,
                boot_id_path: args.boot_id_path,
                local_private_key_path: args.local_private_key,
                parent_public_key_path: args.parent_public_key,
                validate_only: args.validate_only,
                once: args.once,
            })
            .await
        }
        Some(Command::Serve {
            config,
            public_socket,
            broker_socket,
            state_lock,
            locks_dir,
            once,
            test_listen_on,
            allow_unprivileged_runtime_dir,
            no_drop_privileges,
            daemon_state_dir,
            test_state_restore_report,
        }) => {
            let effective_public_socket = test_listen_on.or(public_socket);
            serve(ServeOptions {
                config_path: config,
                public_socket_path: effective_public_socket,
                broker_socket_path: broker_socket,
                state_lock_path: state_lock,
                locks_dir,
                once,
                allow_unprivileged_runtime_dir,
                drop_privileges: !no_drop_privileges,
                daemon_state_dir,
                test_state_restore_report_path: test_state_restore_report,
            })
            .await
        }
        Some(Command::LockOnly {
            config,
            state_lock,
            hold_seconds,
            allow_unprivileged_runtime_dir,
        }) => {
            lock_only(LockOnlyOptions {
                config_path: config,
                state_lock_path: state_lock,
                allow_unprivileged_runtime_dir,
                hold_seconds,
            })
            .await
        }
        Some(Command::TestClient { socket, frame_json }) => run_test_client(TestClientOptions {
            socket_path: socket,
            frame_json,
        })
        .map(|exit_code| {
            if exit_code != 0 {
                std::process::exit(i32::from(exit_code));
            }
        }),
    };

    if let Err(error) = result {
        let _ = error.to_envelope();
        eprintln!("{}: {}", error.kind(), error.message());
        std::process::exit(i32::from(error.exit_code()));
    }
}
