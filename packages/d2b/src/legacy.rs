use super::{
    AuthCommand, CliFailure, ClipboardCommand, ConfigCommand, HostCommand, LegacyContext,
    NativeCommand, OpCommand, RealmCommand, StoreCommand, UsbCommand, UsbSecurityKeyCommand,
    VmCommand, cmd_audio, cmd_audit, cmd_auth_status, cmd_boot, cmd_build, cmd_clipboard_arm,
    cmd_config_approve, cmd_config_diff, cmd_config_reject, cmd_config_status, cmd_config_sync,
    cmd_console, cmd_gc, cmd_generations, cmd_host_check, cmd_host_destroy, cmd_host_doctor,
    cmd_host_install, cmd_host_migrate_storage, cmd_host_prepare, cmd_host_reconcile,
    cmd_host_validate, cmd_keys_list, cmd_keys_rotate, cmd_keys_rotate_known_host, cmd_keys_show,
    cmd_keys_trust, cmd_launch, cmd_list, cmd_migrate, cmd_op_inspect, cmd_realm_enter,
    cmd_realm_inspect, cmd_realm_list, cmd_realm_run, cmd_rollback, cmd_shell, cmd_status,
    cmd_store_verify, cmd_switch, cmd_test, cmd_usb_attach, cmd_usb_detach, cmd_usb_probe,
    cmd_usb_sk_cancel, cmd_usb_sk_sessions, cmd_usb_sk_status, cmd_usb_sk_test, cmd_vm_display,
    cmd_vm_exec, cmd_vm_list, cmd_vm_restart, cmd_vm_start, cmd_vm_status, cmd_vm_stop,
};
use clap::Parser;
use std::ffi::OsString;

#[derive(Debug, Parser)]
#[command(
    version,
    about = "d2b - opinionated NixOS desktop microVM CLI.",
    long_about = "d2b - daemon-native CLI for d2b microVMs.\n\nMutating verbs dispatch through d2bd; privileged host mutations additionally use d2b-priv-broker. \
        Read-only verbs (list, status, audit, host check) prefer d2bd's \
        public socket and fall back to static/local sources where documented. \
        See `d2b <COMMAND> --help` for per-verb usage."
)]
pub(super) struct NativeCli {
    #[command(subcommand)]
    pub(super) command: NativeCommand,
}

fn dispatch(
    context: &LegacyContext,
    cli: &NativeCli,
    original_args: &[OsString],
) -> Result<i32, CliFailure> {
    match &cli.command {
        NativeCommand::List(args) => cmd_list(context, args),
        NativeCommand::Status(args) => cmd_status(context, args),
        NativeCommand::Launch(args) => cmd_launch(context, args),
        NativeCommand::Usb(args) => match &args.command {
            UsbCommand::Attach(args) => cmd_usb_attach(context, args),
            UsbCommand::Detach(args) => cmd_usb_detach(context, args),
            UsbCommand::Probe(args) => cmd_usb_probe(context, args),
            UsbCommand::SecurityKey(args) => match &args.command {
                UsbSecurityKeyCommand::Status(args) => cmd_usb_sk_status(context, args),
                UsbSecurityKeyCommand::Sessions(args) => cmd_usb_sk_sessions(context, args),
                UsbSecurityKeyCommand::Cancel(args) => cmd_usb_sk_cancel(context, args),
                UsbSecurityKeyCommand::Test(args) => cmd_usb_sk_test(context, args),
            },
        },
        NativeCommand::Console(args) => cmd_console(context, args, original_args),
        NativeCommand::Audio(args) => cmd_audio(context, args, original_args),
        NativeCommand::Audit(args) => cmd_audit(context, args, original_args),
        NativeCommand::Host(args) => match &args.command {
            HostCommand::Check(args) => cmd_host_check(context, args),
            HostCommand::Prepare(args) => cmd_host_prepare(context, args),
            HostCommand::Destroy(args) => cmd_host_destroy(context, args),
            HostCommand::Doctor(args) => cmd_host_doctor(context, args),
            HostCommand::MigrateStorage(args) => cmd_host_migrate_storage(context, args),
            HostCommand::Install(args) => cmd_host_install(context, args, original_args),
            HostCommand::Reconcile(args) => cmd_host_reconcile(context, args, original_args),
            HostCommand::Validate(args) => cmd_host_validate(context, args),
        },
        NativeCommand::Auth(args) => match &args.command {
            AuthCommand::Status(args) => cmd_auth_status(context, args),
        },
        NativeCommand::Realm(args) => match &args.command {
            RealmCommand::List(args) => cmd_realm_list(context, args),
            RealmCommand::Inspect(args) => cmd_realm_inspect(context, args),
            RealmCommand::Enter(args) => cmd_realm_enter(context, args),
            RealmCommand::Run(args) => cmd_realm_run(context, args),
        },
        NativeCommand::Shell(args) => cmd_shell(context, args),
        NativeCommand::Op(args) => match &args.command {
            OpCommand::Inspect(args) => cmd_op_inspect(context, args),
        },
        NativeCommand::Vm(args) => match &args.command {
            VmCommand::Start(args) => cmd_vm_start(context, args),
            VmCommand::Stop(args) => cmd_vm_stop(context, args),
            VmCommand::Restart(args) => cmd_vm_restart(context, args),
            VmCommand::List(args) => cmd_vm_list(context, args),
            VmCommand::Status(args) => cmd_vm_status(context, args),
            VmCommand::Exec(args) => cmd_vm_exec(context, args),
            VmCommand::Display(args) => cmd_vm_display(context, args),
        },
        NativeCommand::Up(args) => cmd_vm_start(context, args),
        NativeCommand::Down(args) => cmd_vm_stop(context, args),
        NativeCommand::Restart(args) => cmd_vm_restart(context, args),
        NativeCommand::Build(args) => cmd_build(context, args),
        NativeCommand::Generations(args) => cmd_generations(context, args),
        NativeCommand::Switch(args) => cmd_switch(context, args, original_args),
        NativeCommand::Boot(args) => cmd_boot(context, args, original_args),
        NativeCommand::Test(args) => cmd_test(context, args, original_args),
        NativeCommand::Rollback(args) => cmd_rollback(context, args, original_args),
        NativeCommand::Gc(args) => cmd_gc(context, args, original_args),
        NativeCommand::Store(args) => match &args.command {
            StoreCommand::Verify(args) => cmd_store_verify(context, args),
        },
        NativeCommand::Keys(args) => match &args.command {
            super::KeysCommand::List(args) => cmd_keys_list(context, args, original_args),
            super::KeysCommand::Show(args) => cmd_keys_show(context, args, original_args),
            super::KeysCommand::Rotate(args) => cmd_keys_rotate(context, args, original_args),
        },
        NativeCommand::Trust(args) => cmd_keys_trust(context, args, original_args),
        NativeCommand::RotateKnownHost(args) => {
            cmd_keys_rotate_known_host(context, args, original_args)
        }
        NativeCommand::Migrate(args) => cmd_migrate(context, args, original_args),
        NativeCommand::Config(args) => match &args.command {
            ConfigCommand::Sync(args) => cmd_config_sync(context, args),
            ConfigCommand::Diff(args) => cmd_config_diff(args),
            ConfigCommand::Approve(args) => cmd_config_approve(args),
            ConfigCommand::Reject(args) => cmd_config_reject(args),
            ConfigCommand::Status(args) => cmd_config_status(args),
        },
        NativeCommand::Clipboard(args) => match &args.command {
            ClipboardCommand::Arm(args) => cmd_clipboard_arm(context, args),
        },
    }
}
