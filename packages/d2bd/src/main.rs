use std::path::PathBuf;

use clap::{Parser, Subcommand};
use d2bd::{
    DEFAULT_CONFIG_PATH, LockOnlyOptions, ServeOptions, banner, banner_note, lock_only, serve,
};

#[derive(Debug, Parser)]
#[command(name = "d2bd", about = "d2b daemon skeleton")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
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
    };

    if let Err(error) = result {
        let _ = error.to_envelope();
        eprintln!("{}: {}", error.kind(), error.message());
        std::process::exit(i32::from(error.exit_code()));
    }
}
