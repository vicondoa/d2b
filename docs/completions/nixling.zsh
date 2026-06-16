#compdef nixling

autoload -U is-at-least

_nixling() {
    typeset -A opt_args
    typeset -a _arguments_options
    local ret=1

    if is-at-least 5.2; then
        _arguments_options=(-s -S -C)
    else
        _arguments_options=(-s -C)
    fi

    local context curcontext="$curcontext" state line
    _arguments "${_arguments_options[@]}" : \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
'-V[Print version]' \
'--version[Print version]' \
":: :_nixling_commands" \
"*::: :->nixling" \
&& ret=0
    case $state in
    (nixling)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--vm=[]:VM_FLAG:_default' \
'(--human)--json[]' \
'(--json)--human[]' \
'--check-bridges[]' \
'-h[Print help]' \
'--help[Print help]' \
'::vm:_default' \
&& ret=0
;;
(usb)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_nixling__subcmd__usb_commands" \
"*::: :->usb" \
&& ret=0

    case $state in
    (usb)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-usb-command-$line[1]:"
        case $line[1] in
            (attach)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
':busid:_default' \
&& ret=0
;;
(detach)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
':busid:_default' \
&& ret=0
;;
(probe)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__usb__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-usb-help-command-$line[1]:"
        case $line[1] in
            (attach)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(detach)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(probe)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(console)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(audio)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_nixling__subcmd__audio_commands" \
"*::: :->audio" \
&& ret=0

    case $state in
    (audio)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-audio-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
'::vm:_default' \
&& ret=0
;;
(mic)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
':state:(on off)' \
':vm:_default' \
&& ret=0
;;
(speaker)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
':state:(on off)' \
':vm:_default' \
&& ret=0
;;
(off)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__audio__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-audio-help-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(mic)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(speaker)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(off)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(audit)
_arguments "${_arguments_options[@]}" : \
'--strict[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(host)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_nixling__subcmd__host_commands" \
"*::: :->host" \
&& ret=0

    case $state in
    (host)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-host-command-$line[1]:"
        case $line[1] in
            (check)
_arguments "${_arguments_options[@]}" : \
'--read-only[]' \
'--strict[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(prepare)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[Plan the reconcile without mutating host state]' \
'(--dry-run)--apply[Apply the reconcile (mutates host state)]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(destroy)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(doctor)
_arguments "${_arguments_options[@]}" : \
'--read-only[Mandatory\: doctor is read-only. Mutating forms are separate verbs]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(install)
_arguments "${_arguments_options[@]}" : \
'(--apply --enable --start --no-start)--dry-run[Report the planned install steps without mutating]' \
'(--dry-run)--apply[Perform the install through the daemon → broker \`RunHostInstall\` path]' \
'(--dry-run)--enable[After \`--apply\`, enable nixlingd.service via systemctl]' \
'(--dry-run --no-start)--start[After \`--apply --enable\`, start nixlingd.service]' \
'(--dry-run --start)--no-start[Explicitly do NOT start nixlingd.service post-install]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
'--network[Re-run the network slice of \`host prepare\` and clear the daemon'\''s net-route preflight counter. Currently the only available scope]' \
'(--apply)--dry-run[Plan the reconcile without mutating host state]' \
'(--dry-run)--apply[Apply the reconcile (mutates host state)]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(validate)
_arguments "${_arguments_options[@]}" : \
'--wave=[Restrict to a single wave. Other waves are reported as \`skipped\`]:WAVE:_default' \
'--operator-signature=[Override the per-wave operator signature. When unset, the verb derives a deterministic sha256 signature from \`hostname|wave|scripts_dir|timestamp\`]:SIGNATURE:_default' \
'--evidence-dir=[Override the evidence directory. Default\: \`/var/lib/nixling/validated\`]:PATH:_files' \
'--scripts-dir=[Override the scripts directory. Default\: best-effort discovery of the installed \`tests/\` share, then \`./tests\`]:PATH:_files' \
'(--apply)--dry-run[Plan\: report which readiness validators WOULD be attested. No evidence is written]' \
'(--dry-run)--apply[Apply\: write \`/var/lib/nixling/validated/<wave>.json\` for every wave whose declared validators are present on disk]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__host__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-host-help-command-$line[1]:"
        case $line[1] in
            (check)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(prepare)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(destroy)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(doctor)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(install)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(validate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(auth)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_nixling__subcmd__auth_commands" \
"*::: :->auth" \
&& ret=0

    case $state in
    (auth)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-auth-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
'--test-uid=[]:TEST_UID:_default' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__auth__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-auth-help-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(vm)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_nixling__subcmd__vm_commands" \
"*::: :->vm" \
&& ret=0

    case $state in
    (vm)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-vm-command-$line[1]:"
        case $line[1] in
            (start)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[Plan the DAG without spawning any role]' \
'(--dry-run)--apply[Apply the DAG (drives the supervisor)]' \
'--no-wait-api[Exit 0 on process-alive success without waiting for api-ready. Default behavior is --strict (wait for both process-alive and api-ready)]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm -- VM name as declared in `nixling.vms.<name>`:_default' \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(restart)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm -- VM name:_default' \
&& ret=0
;;
(exec)
_arguments "${_arguments_options[@]}" : \
'*--env=[Set an environment variable in the guest command (\`KEY=VALUE\`). Repeatable]:KEY=VALUE:_default' \
'--cwd=[Working directory for the guest command]:DIR:_default' \
'-d[Start the command detached and print its exec id. Incompatible with \`-i\`/\`-t\`; detached execs are managed with \`nixling vm exec <vm> {list|logs|status|kill}\`]' \
'--detach[Start the command detached and print its exec id. Incompatible with \`-i\`/\`-t\`; detached execs are managed with \`nixling vm exec <vm> {list|logs|status|kill}\`]' \
'-i[Forward host stdin into the guest command (\`-i\`). Requires \`-t\`/\`--tty\`; use \`-it\` for an interactive shell]' \
'--interactive[Forward host stdin into the guest command (\`-i\`). Requires \`-t\`/\`--tty\`; use \`-it\` for an interactive shell]' \
'-t[Allocate a PTY in the guest and put the host terminal in raw mode (\`-t\`). Implies stdin forwarding. Human-only (incompatible with \`--json\`)]' \
'--tty[Allocate a PTY in the guest and put the host terminal in raw mode (\`-t\`). Implies stdin forwarding. Human-only (incompatible with \`--json\`)]' \
'(--human)--json[Emit a single terminal JSON envelope (exit code + source/reason + bounded captured output). Non-interactive only]' \
'(--json)--human[Force human output]' \
'-h[Print help]' \
'--help[Print help]' \
':vm -- VM name as declared in `nixling.vms.<name>`:_default' \
'*::management -- Optional detached exec management form\: `list`, `logs <id> \[--stdout-offset N|--stdout-offset=N\] \[--stderr-offset N|--stderr-offset=N\] \[--max-len N|--max-len=N\]`, `status <id>`, or `kill <id>`. Command execs never use this position\: pass a command after `--` instead:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__vm__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-vm-help-command-$line[1]:"
        case $line[1] in
            (start)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(restart)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(exec)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(up)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[Plan the DAG without spawning any role]' \
'(--dry-run)--apply[Apply the DAG (drives the supervisor)]' \
'--no-wait-api[Exit 0 on process-alive success without waiting for api-ready. Default behavior is --strict (wait for both process-alive and api-ready)]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm -- VM name as declared in `nixling.vms.<name>`:_default' \
&& ret=0
;;
(down)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(restart)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(build)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(generations)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(switch)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(boot)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(test)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(rollback)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(gc)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(store)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_nixling__subcmd__store_commands" \
"*::: :->store" \
&& ret=0

    case $state in
    (store)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-store-command-$line[1]:"
        case $line[1] in
            (verify)
_arguments "${_arguments_options[@]}" : \
'--repair[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__store__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-store-help-command-$line[1]:"
        case $line[1] in
            (verify)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(keys)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_nixling__subcmd__keys_commands" \
"*::: :->keys" \
&& ret=0

    case $state in
    (keys)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-keys-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(show)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(rotate)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__keys__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-keys-help-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(show)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(rotate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(trust)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(rotate-known-host)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(migrate)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(config)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_nixling__subcmd__config_commands" \
"*::: :->config" \
&& ret=0

    case $state in
    (config)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-config-command-$line[1]:"
        case $line[1] in
            (sync)
_arguments "${_arguments_options[@]}" : \
'--guest-path=[Path of the editable guest config INSIDE the VM to pull. Honored only by the legacy operator SSH transport; on guest-control VMs the canonical guest config working copy is read by file id and this flag is rejected]:GUEST_PATH:_default' \
'--host=[Override the SSH host (defaults to the manifest \`static_ip\`). SSH transport only; rejected on guest-control VMs]:HOST:_default' \
'--user=[Override the SSH user (defaults to the manifest \`ssh_user\`). SSH transport only; rejected on guest-control VMs]:USER:_default' \
'--key=[Override the SSH private key path. SSH transport only; rejected on guest-control VMs]:KEY:_files' \
'--known-hosts=[known_hosts file used to verify the VM'\''s host key (defaults to the framework-managed \`/var/lib/nixling/known_hosts.nixling\`). SSH transport only; rejected on guest-control VMs]:KNOWN_HOSTS:_files' \
'--dry-run[Print the planned action instead of running it]' \
'--json[Emit a JSON envelope]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':vm -- VM name (must match the static manifest):_default' \
&& ret=0
;;
(diff)
_arguments "${_arguments_options[@]}" : \
'--against=[The live host-side guest config file to compare the staging against]:AGAINST:_files' \
'--json[Emit a JSON envelope]' \
'-h[Print help]' \
'--help[Print help]' \
':vm -- VM name (must match the static manifest):_default' \
&& ret=0
;;
(approve)
_arguments "${_arguments_options[@]}" : \
'--to=[The host-side file to write the approved staging copy onto. The operator chooses this (typically their \`guestConfigFile\` path)]:TO:_files' \
'--json[Emit a JSON envelope]' \
'-h[Print help]' \
'--help[Print help]' \
':vm -- VM name (must match the static manifest):_default' \
&& ret=0
;;
(reject)
_arguments "${_arguments_options[@]}" : \
'--json[Emit a JSON envelope]' \
'-h[Print help]' \
'--help[Print help]' \
':vm -- VM name (must match the static manifest):_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--all[Report every VM that currently has a pending staging file]' \
'--json[Emit a JSON envelope]' \
'-h[Print help]' \
'--help[Print help]' \
'::vm -- VM name; omit together with `--all` to report every staged VM:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__config__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-config-help-command-$line[1]:"
        case $line[1] in
            (sync)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(diff)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(approve)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(reject)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-help-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(usb)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__help__subcmd__usb_commands" \
"*::: :->usb" \
&& ret=0

    case $state in
    (usb)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-help-usb-command-$line[1]:"
        case $line[1] in
            (attach)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(detach)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(probe)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(console)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(audio)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__help__subcmd__audio_commands" \
"*::: :->audio" \
&& ret=0

    case $state in
    (audio)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-help-audio-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(mic)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(speaker)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(off)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(audit)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(host)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__help__subcmd__host_commands" \
"*::: :->host" \
&& ret=0

    case $state in
    (host)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-help-host-command-$line[1]:"
        case $line[1] in
            (check)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(prepare)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(destroy)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(doctor)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(install)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(validate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(auth)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__help__subcmd__auth_commands" \
"*::: :->auth" \
&& ret=0

    case $state in
    (auth)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-help-auth-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(vm)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__help__subcmd__vm_commands" \
"*::: :->vm" \
&& ret=0

    case $state in
    (vm)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-help-vm-command-$line[1]:"
        case $line[1] in
            (start)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(restart)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(exec)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(up)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(down)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(restart)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(build)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(generations)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(switch)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(boot)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(test)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(rollback)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(gc)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(store)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__help__subcmd__store_commands" \
"*::: :->store" \
&& ret=0

    case $state in
    (store)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-help-store-command-$line[1]:"
        case $line[1] in
            (verify)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(keys)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__help__subcmd__keys_commands" \
"*::: :->keys" \
&& ret=0

    case $state in
    (keys)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-help-keys-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(show)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(rotate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(trust)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(rotate-known-host)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(migrate)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(config)
_arguments "${_arguments_options[@]}" : \
":: :_nixling__subcmd__help__subcmd__config_commands" \
"*::: :->config" \
&& ret=0

    case $state in
    (config)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:nixling-help-config-command-$line[1]:"
        case $line[1] in
            (sync)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(diff)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(approve)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(reject)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(help)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
        esac
    ;;
esac
}

(( $+functions[_nixling_commands] )) ||
_nixling_commands() {
    local commands; commands=(
'list:List declared VMs from the static manifest' \
'status:Show per-VM runtime status plus bridge health' \
'usb:USBIP attach / detach / probe' \
'console:Foreground serial console bridge for headless VMs (not yet implemented)' \
'audio:Per-VM audio grant bridge (not yet implemented)' \
'audit:Tail the broker audit log' \
'host:Host-side preflight, install, doctor, and reconcile verbs' \
'auth:Authorisation introspection' \
'vm:Per-VM lifecycle verbs (start / stop / restart / list / status) plus the admin-only guest-control sub-verb \`exec\`, which runs commands or an interactive session inside a VM over the authenticated guest-control transport (no SSH)' \
'up:Alias for \`vm start <vm>\`' \
'down:Alias for \`vm stop <vm>\`' \
'restart:Alias for \`vm restart <vm>\`' \
'build:Non-destructive eval + build of the per-VM toplevel' \
'generations:List current / booted / numbered generations for a VM' \
'switch:Atomically activate a new per-VM closure' \
'boot:Stage a per-VM closure for the next boot only' \
'test:Activate a per-VM closure with rollback on reboot' \
'rollback:Roll a VM back to its previous generation' \
'gc:Garbage-collect the per-VM /nix/store hardlink farm' \
'store:Store-view maintenance and verification' \
'keys:Managed-key lifecycle (list / show / rotate)' \
'trust:Trust a VM'\''s host key on first use (TOFU)' \
'rotate-known-host:Rotate the consumer'\''s recorded known-host entry for a VM' \
'migrate:Analyse the host config and emit a migration plan' \
'config:Sync / review / approve a VM'\''s guest-editable config (\`guestConfigFile\`)\: pull the operator'\''s in-VM edits to a host-side staging file, diff them, and approve them' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling commands' commands "$@"
}
(( $+functions[_nixling__subcmd__audio_commands] )) ||
_nixling__subcmd__audio_commands() {
    local commands; commands=(
'status:Show current grant state. With no VM, lists every audio-enabled VM' \
'mic:Grant or revoke microphone access' \
'speaker:Grant or revoke speaker access' \
'off:Revoke both mic and speaker access' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling audio commands' commands "$@"
}
(( $+functions[_nixling__subcmd__audio__subcmd__help_commands] )) ||
_nixling__subcmd__audio__subcmd__help_commands() {
    local commands; commands=(
'status:Show current grant state. With no VM, lists every audio-enabled VM' \
'mic:Grant or revoke microphone access' \
'speaker:Grant or revoke speaker access' \
'off:Revoke both mic and speaker access' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling audio help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__audio__subcmd__help__subcmd__help_commands] )) ||
_nixling__subcmd__audio__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'nixling audio help help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__audio__subcmd__help__subcmd__mic_commands] )) ||
_nixling__subcmd__audio__subcmd__help__subcmd__mic_commands() {
    local commands; commands=()
    _describe -t commands 'nixling audio help mic commands' commands "$@"
}
(( $+functions[_nixling__subcmd__audio__subcmd__help__subcmd__off_commands] )) ||
_nixling__subcmd__audio__subcmd__help__subcmd__off_commands() {
    local commands; commands=()
    _describe -t commands 'nixling audio help off commands' commands "$@"
}
(( $+functions[_nixling__subcmd__audio__subcmd__help__subcmd__speaker_commands] )) ||
_nixling__subcmd__audio__subcmd__help__subcmd__speaker_commands() {
    local commands; commands=()
    _describe -t commands 'nixling audio help speaker commands' commands "$@"
}
(( $+functions[_nixling__subcmd__audio__subcmd__help__subcmd__status_commands] )) ||
_nixling__subcmd__audio__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'nixling audio help status commands' commands "$@"
}
(( $+functions[_nixling__subcmd__audio__subcmd__mic_commands] )) ||
_nixling__subcmd__audio__subcmd__mic_commands() {
    local commands; commands=()
    _describe -t commands 'nixling audio mic commands' commands "$@"
}
(( $+functions[_nixling__subcmd__audio__subcmd__off_commands] )) ||
_nixling__subcmd__audio__subcmd__off_commands() {
    local commands; commands=()
    _describe -t commands 'nixling audio off commands' commands "$@"
}
(( $+functions[_nixling__subcmd__audio__subcmd__speaker_commands] )) ||
_nixling__subcmd__audio__subcmd__speaker_commands() {
    local commands; commands=()
    _describe -t commands 'nixling audio speaker commands' commands "$@"
}
(( $+functions[_nixling__subcmd__audio__subcmd__status_commands] )) ||
_nixling__subcmd__audio__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'nixling audio status commands' commands "$@"
}
(( $+functions[_nixling__subcmd__audit_commands] )) ||
_nixling__subcmd__audit_commands() {
    local commands; commands=()
    _describe -t commands 'nixling audit commands' commands "$@"
}
(( $+functions[_nixling__subcmd__auth_commands] )) ||
_nixling__subcmd__auth_commands() {
    local commands; commands=(
'status:' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling auth commands' commands "$@"
}
(( $+functions[_nixling__subcmd__auth__subcmd__help_commands] )) ||
_nixling__subcmd__auth__subcmd__help_commands() {
    local commands; commands=(
'status:' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling auth help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__auth__subcmd__help__subcmd__help_commands] )) ||
_nixling__subcmd__auth__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'nixling auth help help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__auth__subcmd__help__subcmd__status_commands] )) ||
_nixling__subcmd__auth__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'nixling auth help status commands' commands "$@"
}
(( $+functions[_nixling__subcmd__auth__subcmd__status_commands] )) ||
_nixling__subcmd__auth__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'nixling auth status commands' commands "$@"
}
(( $+functions[_nixling__subcmd__boot_commands] )) ||
_nixling__subcmd__boot_commands() {
    local commands; commands=()
    _describe -t commands 'nixling boot commands' commands "$@"
}
(( $+functions[_nixling__subcmd__build_commands] )) ||
_nixling__subcmd__build_commands() {
    local commands; commands=()
    _describe -t commands 'nixling build commands' commands "$@"
}
(( $+functions[_nixling__subcmd__config_commands] )) ||
_nixling__subcmd__config_commands() {
    local commands; commands=(
'sync:Pull the VM'\''s in-guest edited config into a host-side staging file' \
'diff:Diff the staged guest config against a live host-side file' \
'approve:Approve the staged guest config by writing it to a target file' \
'reject:Discard the staged guest config' \
'status:Report whether a VM has a pending (un-approved) staged config' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling config commands' commands "$@"
}
(( $+functions[_nixling__subcmd__config__subcmd__approve_commands] )) ||
_nixling__subcmd__config__subcmd__approve_commands() {
    local commands; commands=()
    _describe -t commands 'nixling config approve commands' commands "$@"
}
(( $+functions[_nixling__subcmd__config__subcmd__diff_commands] )) ||
_nixling__subcmd__config__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'nixling config diff commands' commands "$@"
}
(( $+functions[_nixling__subcmd__config__subcmd__help_commands] )) ||
_nixling__subcmd__config__subcmd__help_commands() {
    local commands; commands=(
'sync:Pull the VM'\''s in-guest edited config into a host-side staging file' \
'diff:Diff the staged guest config against a live host-side file' \
'approve:Approve the staged guest config by writing it to a target file' \
'reject:Discard the staged guest config' \
'status:Report whether a VM has a pending (un-approved) staged config' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling config help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__config__subcmd__help__subcmd__approve_commands] )) ||
_nixling__subcmd__config__subcmd__help__subcmd__approve_commands() {
    local commands; commands=()
    _describe -t commands 'nixling config help approve commands' commands "$@"
}
(( $+functions[_nixling__subcmd__config__subcmd__help__subcmd__diff_commands] )) ||
_nixling__subcmd__config__subcmd__help__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'nixling config help diff commands' commands "$@"
}
(( $+functions[_nixling__subcmd__config__subcmd__help__subcmd__help_commands] )) ||
_nixling__subcmd__config__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'nixling config help help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__config__subcmd__help__subcmd__reject_commands] )) ||
_nixling__subcmd__config__subcmd__help__subcmd__reject_commands() {
    local commands; commands=()
    _describe -t commands 'nixling config help reject commands' commands "$@"
}
(( $+functions[_nixling__subcmd__config__subcmd__help__subcmd__status_commands] )) ||
_nixling__subcmd__config__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'nixling config help status commands' commands "$@"
}
(( $+functions[_nixling__subcmd__config__subcmd__help__subcmd__sync_commands] )) ||
_nixling__subcmd__config__subcmd__help__subcmd__sync_commands() {
    local commands; commands=()
    _describe -t commands 'nixling config help sync commands' commands "$@"
}
(( $+functions[_nixling__subcmd__config__subcmd__reject_commands] )) ||
_nixling__subcmd__config__subcmd__reject_commands() {
    local commands; commands=()
    _describe -t commands 'nixling config reject commands' commands "$@"
}
(( $+functions[_nixling__subcmd__config__subcmd__status_commands] )) ||
_nixling__subcmd__config__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'nixling config status commands' commands "$@"
}
(( $+functions[_nixling__subcmd__config__subcmd__sync_commands] )) ||
_nixling__subcmd__config__subcmd__sync_commands() {
    local commands; commands=()
    _describe -t commands 'nixling config sync commands' commands "$@"
}
(( $+functions[_nixling__subcmd__console_commands] )) ||
_nixling__subcmd__console_commands() {
    local commands; commands=()
    _describe -t commands 'nixling console commands' commands "$@"
}
(( $+functions[_nixling__subcmd__down_commands] )) ||
_nixling__subcmd__down_commands() {
    local commands; commands=()
    _describe -t commands 'nixling down commands' commands "$@"
}
(( $+functions[_nixling__subcmd__gc_commands] )) ||
_nixling__subcmd__gc_commands() {
    local commands; commands=()
    _describe -t commands 'nixling gc commands' commands "$@"
}
(( $+functions[_nixling__subcmd__generations_commands] )) ||
_nixling__subcmd__generations_commands() {
    local commands; commands=()
    _describe -t commands 'nixling generations commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help_commands] )) ||
_nixling__subcmd__help_commands() {
    local commands; commands=(
'list:List declared VMs from the static manifest' \
'status:Show per-VM runtime status plus bridge health' \
'usb:USBIP attach / detach / probe' \
'console:Foreground serial console bridge for headless VMs (not yet implemented)' \
'audio:Per-VM audio grant bridge (not yet implemented)' \
'audit:Tail the broker audit log' \
'host:Host-side preflight, install, doctor, and reconcile verbs' \
'auth:Authorisation introspection' \
'vm:Per-VM lifecycle verbs (start / stop / restart / list / status) plus the admin-only guest-control sub-verb \`exec\`, which runs commands or an interactive session inside a VM over the authenticated guest-control transport (no SSH)' \
'up:Alias for \`vm start <vm>\`' \
'down:Alias for \`vm stop <vm>\`' \
'restart:Alias for \`vm restart <vm>\`' \
'build:Non-destructive eval + build of the per-VM toplevel' \
'generations:List current / booted / numbered generations for a VM' \
'switch:Atomically activate a new per-VM closure' \
'boot:Stage a per-VM closure for the next boot only' \
'test:Activate a per-VM closure with rollback on reboot' \
'rollback:Roll a VM back to its previous generation' \
'gc:Garbage-collect the per-VM /nix/store hardlink farm' \
'store:Store-view maintenance and verification' \
'keys:Managed-key lifecycle (list / show / rotate)' \
'trust:Trust a VM'\''s host key on first use (TOFU)' \
'rotate-known-host:Rotate the consumer'\''s recorded known-host entry for a VM' \
'migrate:Analyse the host config and emit a migration plan' \
'config:Sync / review / approve a VM'\''s guest-editable config (\`guestConfigFile\`)\: pull the operator'\''s in-VM edits to a host-side staging file, diff them, and approve them' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__audio_commands] )) ||
_nixling__subcmd__help__subcmd__audio_commands() {
    local commands; commands=(
'status:Show current grant state. With no VM, lists every audio-enabled VM' \
'mic:Grant or revoke microphone access' \
'speaker:Grant or revoke speaker access' \
'off:Revoke both mic and speaker access' \
    )
    _describe -t commands 'nixling help audio commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__audio__subcmd__mic_commands] )) ||
_nixling__subcmd__help__subcmd__audio__subcmd__mic_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help audio mic commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__audio__subcmd__off_commands] )) ||
_nixling__subcmd__help__subcmd__audio__subcmd__off_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help audio off commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__audio__subcmd__speaker_commands] )) ||
_nixling__subcmd__help__subcmd__audio__subcmd__speaker_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help audio speaker commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__audio__subcmd__status_commands] )) ||
_nixling__subcmd__help__subcmd__audio__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help audio status commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__audit_commands] )) ||
_nixling__subcmd__help__subcmd__audit_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help audit commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__auth_commands] )) ||
_nixling__subcmd__help__subcmd__auth_commands() {
    local commands; commands=(
'status:' \
    )
    _describe -t commands 'nixling help auth commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__auth__subcmd__status_commands] )) ||
_nixling__subcmd__help__subcmd__auth__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help auth status commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__boot_commands] )) ||
_nixling__subcmd__help__subcmd__boot_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help boot commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__build_commands] )) ||
_nixling__subcmd__help__subcmd__build_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help build commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__config_commands] )) ||
_nixling__subcmd__help__subcmd__config_commands() {
    local commands; commands=(
'sync:Pull the VM'\''s in-guest edited config into a host-side staging file' \
'diff:Diff the staged guest config against a live host-side file' \
'approve:Approve the staged guest config by writing it to a target file' \
'reject:Discard the staged guest config' \
'status:Report whether a VM has a pending (un-approved) staged config' \
    )
    _describe -t commands 'nixling help config commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__config__subcmd__approve_commands] )) ||
_nixling__subcmd__help__subcmd__config__subcmd__approve_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help config approve commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__config__subcmd__diff_commands] )) ||
_nixling__subcmd__help__subcmd__config__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help config diff commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__config__subcmd__reject_commands] )) ||
_nixling__subcmd__help__subcmd__config__subcmd__reject_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help config reject commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__config__subcmd__status_commands] )) ||
_nixling__subcmd__help__subcmd__config__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help config status commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__config__subcmd__sync_commands] )) ||
_nixling__subcmd__help__subcmd__config__subcmd__sync_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help config sync commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__console_commands] )) ||
_nixling__subcmd__help__subcmd__console_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help console commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__down_commands] )) ||
_nixling__subcmd__help__subcmd__down_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help down commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__gc_commands] )) ||
_nixling__subcmd__help__subcmd__gc_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help gc commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__generations_commands] )) ||
_nixling__subcmd__help__subcmd__generations_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help generations commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__help_commands] )) ||
_nixling__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__host_commands] )) ||
_nixling__subcmd__help__subcmd__host_commands() {
    local commands; commands=(
'check:Read-only preflight\: inventories host posture without mutation' \
'prepare:Reconcile host-side state (bridges, nftables, sysctls). --apply mutates' \
'destroy:Tear down host-side state owned by nixling. --apply mutates' \
'doctor:Read-only deep diagnostics for the daemon + broker state' \
'install:Install nixlingd + broker units onto the host. --apply mutates' \
'reconcile:Recover host network state after the daemon engaged operator-only mode' \
'validate:Run the host-side validator suite and write evidence records' \
    )
    _describe -t commands 'nixling help host commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__host__subcmd__check_commands] )) ||
_nixling__subcmd__help__subcmd__host__subcmd__check_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help host check commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__host__subcmd__destroy_commands] )) ||
_nixling__subcmd__help__subcmd__host__subcmd__destroy_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help host destroy commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__host__subcmd__doctor_commands] )) ||
_nixling__subcmd__help__subcmd__host__subcmd__doctor_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help host doctor commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__host__subcmd__install_commands] )) ||
_nixling__subcmd__help__subcmd__host__subcmd__install_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help host install commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__host__subcmd__prepare_commands] )) ||
_nixling__subcmd__help__subcmd__host__subcmd__prepare_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help host prepare commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__host__subcmd__reconcile_commands] )) ||
_nixling__subcmd__help__subcmd__host__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help host reconcile commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__host__subcmd__validate_commands] )) ||
_nixling__subcmd__help__subcmd__host__subcmd__validate_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help host validate commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__keys_commands] )) ||
_nixling__subcmd__help__subcmd__keys_commands() {
    local commands; commands=(
'list:List managed keys (per-VM SSH keypair fingerprints)' \
'show:Show details for a specific VM'\''s managed key' \
'rotate:Rotate the framework-managed per-VM SSH keypair. --apply mutates' \
    )
    _describe -t commands 'nixling help keys commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__keys__subcmd__list_commands] )) ||
_nixling__subcmd__help__subcmd__keys__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help keys list commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__keys__subcmd__rotate_commands] )) ||
_nixling__subcmd__help__subcmd__keys__subcmd__rotate_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help keys rotate commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__keys__subcmd__show_commands] )) ||
_nixling__subcmd__help__subcmd__keys__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help keys show commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__list_commands] )) ||
_nixling__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help list commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__migrate_commands] )) ||
_nixling__subcmd__help__subcmd__migrate_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help migrate commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__restart_commands] )) ||
_nixling__subcmd__help__subcmd__restart_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help restart commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__rollback_commands] )) ||
_nixling__subcmd__help__subcmd__rollback_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help rollback commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__rotate-known-host_commands] )) ||
_nixling__subcmd__help__subcmd__rotate-known-host_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help rotate-known-host commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__status_commands] )) ||
_nixling__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help status commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__store_commands] )) ||
_nixling__subcmd__help__subcmd__store_commands() {
    local commands; commands=(
'verify:Verify a VM'\''s hardlink-backed live store-view' \
    )
    _describe -t commands 'nixling help store commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__store__subcmd__verify_commands] )) ||
_nixling__subcmd__help__subcmd__store__subcmd__verify_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help store verify commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__switch_commands] )) ||
_nixling__subcmd__help__subcmd__switch_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help switch commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__test_commands] )) ||
_nixling__subcmd__help__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help test commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__trust_commands] )) ||
_nixling__subcmd__help__subcmd__trust_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help trust commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__up_commands] )) ||
_nixling__subcmd__help__subcmd__up_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help up commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__usb_commands] )) ||
_nixling__subcmd__help__subcmd__usb_commands() {
    local commands; commands=(
'attach:Bind a host USB busid to a VM via the native daemon path' \
'detach:Unbind a host USB busid from a VM via the native daemon path' \
'probe:List daemon-declared USBIP busid claims and lock owners' \
    )
    _describe -t commands 'nixling help usb commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__usb__subcmd__attach_commands] )) ||
_nixling__subcmd__help__subcmd__usb__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help usb attach commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__usb__subcmd__detach_commands] )) ||
_nixling__subcmd__help__subcmd__usb__subcmd__detach_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help usb detach commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__usb__subcmd__probe_commands] )) ||
_nixling__subcmd__help__subcmd__usb__subcmd__probe_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help usb probe commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__vm_commands] )) ||
_nixling__subcmd__help__subcmd__vm_commands() {
    local commands; commands=(
'start:Start the per-VM DAG (virtiofsd → CH → readiness probes)' \
'stop:Stop the per-VM DAG in reverse topo order' \
'restart:Stop then start; same envelope contract as start' \
'list:Daemon-side runtime view (different from \`nixling list\`, which is the static manifest view)' \
'status:Daemon-side readiness state for a VM (api-ready phase)' \
'exec:Run or manage commands inside a running VM. Use \`nixling vm exec <vm> -- <cmd...>\` for a non-interactive command, \`nixling vm exec -it <vm> -- bash\` for an interactive shell, \`-d\` for a detached command, and \`nixling vm exec <vm> {list|logs|status|kill}\` to manage detached execs' \
    )
    _describe -t commands 'nixling help vm commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__vm__subcmd__exec_commands] )) ||
_nixling__subcmd__help__subcmd__vm__subcmd__exec_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help vm exec commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__vm__subcmd__list_commands] )) ||
_nixling__subcmd__help__subcmd__vm__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help vm list commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__vm__subcmd__restart_commands] )) ||
_nixling__subcmd__help__subcmd__vm__subcmd__restart_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help vm restart commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__vm__subcmd__start_commands] )) ||
_nixling__subcmd__help__subcmd__vm__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help vm start commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__vm__subcmd__status_commands] )) ||
_nixling__subcmd__help__subcmd__vm__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help vm status commands' commands "$@"
}
(( $+functions[_nixling__subcmd__help__subcmd__vm__subcmd__stop_commands] )) ||
_nixling__subcmd__help__subcmd__vm__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help vm stop commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host_commands] )) ||
_nixling__subcmd__host_commands() {
    local commands; commands=(
'check:Read-only preflight\: inventories host posture without mutation' \
'prepare:Reconcile host-side state (bridges, nftables, sysctls). --apply mutates' \
'destroy:Tear down host-side state owned by nixling. --apply mutates' \
'doctor:Read-only deep diagnostics for the daemon + broker state' \
'install:Install nixlingd + broker units onto the host. --apply mutates' \
'reconcile:Recover host network state after the daemon engaged operator-only mode' \
'validate:Run the host-side validator suite and write evidence records' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling host commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__check_commands] )) ||
_nixling__subcmd__host__subcmd__check_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host check commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__destroy_commands] )) ||
_nixling__subcmd__host__subcmd__destroy_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host destroy commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__doctor_commands] )) ||
_nixling__subcmd__host__subcmd__doctor_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host doctor commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__help_commands] )) ||
_nixling__subcmd__host__subcmd__help_commands() {
    local commands; commands=(
'check:Read-only preflight\: inventories host posture without mutation' \
'prepare:Reconcile host-side state (bridges, nftables, sysctls). --apply mutates' \
'destroy:Tear down host-side state owned by nixling. --apply mutates' \
'doctor:Read-only deep diagnostics for the daemon + broker state' \
'install:Install nixlingd + broker units onto the host. --apply mutates' \
'reconcile:Recover host network state after the daemon engaged operator-only mode' \
'validate:Run the host-side validator suite and write evidence records' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling host help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__help__subcmd__check_commands] )) ||
_nixling__subcmd__host__subcmd__help__subcmd__check_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host help check commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__help__subcmd__destroy_commands] )) ||
_nixling__subcmd__host__subcmd__help__subcmd__destroy_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host help destroy commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__help__subcmd__doctor_commands] )) ||
_nixling__subcmd__host__subcmd__help__subcmd__doctor_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host help doctor commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__help__subcmd__help_commands] )) ||
_nixling__subcmd__host__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host help help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__help__subcmd__install_commands] )) ||
_nixling__subcmd__host__subcmd__help__subcmd__install_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host help install commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__help__subcmd__prepare_commands] )) ||
_nixling__subcmd__host__subcmd__help__subcmd__prepare_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host help prepare commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__help__subcmd__reconcile_commands] )) ||
_nixling__subcmd__host__subcmd__help__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host help reconcile commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__help__subcmd__validate_commands] )) ||
_nixling__subcmd__host__subcmd__help__subcmd__validate_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host help validate commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__install_commands] )) ||
_nixling__subcmd__host__subcmd__install_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host install commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__prepare_commands] )) ||
_nixling__subcmd__host__subcmd__prepare_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host prepare commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__reconcile_commands] )) ||
_nixling__subcmd__host__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host reconcile commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host__subcmd__validate_commands] )) ||
_nixling__subcmd__host__subcmd__validate_commands() {
    local commands; commands=()
    _describe -t commands 'nixling host validate commands' commands "$@"
}
(( $+functions[_nixling__subcmd__keys_commands] )) ||
_nixling__subcmd__keys_commands() {
    local commands; commands=(
'list:List managed keys (per-VM SSH keypair fingerprints)' \
'show:Show details for a specific VM'\''s managed key' \
'rotate:Rotate the framework-managed per-VM SSH keypair. --apply mutates' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling keys commands' commands "$@"
}
(( $+functions[_nixling__subcmd__keys__subcmd__help_commands] )) ||
_nixling__subcmd__keys__subcmd__help_commands() {
    local commands; commands=(
'list:List managed keys (per-VM SSH keypair fingerprints)' \
'show:Show details for a specific VM'\''s managed key' \
'rotate:Rotate the framework-managed per-VM SSH keypair. --apply mutates' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling keys help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__keys__subcmd__help__subcmd__help_commands] )) ||
_nixling__subcmd__keys__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'nixling keys help help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__keys__subcmd__help__subcmd__list_commands] )) ||
_nixling__subcmd__keys__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'nixling keys help list commands' commands "$@"
}
(( $+functions[_nixling__subcmd__keys__subcmd__help__subcmd__rotate_commands] )) ||
_nixling__subcmd__keys__subcmd__help__subcmd__rotate_commands() {
    local commands; commands=()
    _describe -t commands 'nixling keys help rotate commands' commands "$@"
}
(( $+functions[_nixling__subcmd__keys__subcmd__help__subcmd__show_commands] )) ||
_nixling__subcmd__keys__subcmd__help__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'nixling keys help show commands' commands "$@"
}
(( $+functions[_nixling__subcmd__keys__subcmd__list_commands] )) ||
_nixling__subcmd__keys__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'nixling keys list commands' commands "$@"
}
(( $+functions[_nixling__subcmd__keys__subcmd__rotate_commands] )) ||
_nixling__subcmd__keys__subcmd__rotate_commands() {
    local commands; commands=()
    _describe -t commands 'nixling keys rotate commands' commands "$@"
}
(( $+functions[_nixling__subcmd__keys__subcmd__show_commands] )) ||
_nixling__subcmd__keys__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'nixling keys show commands' commands "$@"
}
(( $+functions[_nixling__subcmd__list_commands] )) ||
_nixling__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'nixling list commands' commands "$@"
}
(( $+functions[_nixling__subcmd__migrate_commands] )) ||
_nixling__subcmd__migrate_commands() {
    local commands; commands=()
    _describe -t commands 'nixling migrate commands' commands "$@"
}
(( $+functions[_nixling__subcmd__restart_commands] )) ||
_nixling__subcmd__restart_commands() {
    local commands; commands=()
    _describe -t commands 'nixling restart commands' commands "$@"
}
(( $+functions[_nixling__subcmd__rollback_commands] )) ||
_nixling__subcmd__rollback_commands() {
    local commands; commands=()
    _describe -t commands 'nixling rollback commands' commands "$@"
}
(( $+functions[_nixling__subcmd__rotate-known-host_commands] )) ||
_nixling__subcmd__rotate-known-host_commands() {
    local commands; commands=()
    _describe -t commands 'nixling rotate-known-host commands' commands "$@"
}
(( $+functions[_nixling__subcmd__status_commands] )) ||
_nixling__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'nixling status commands' commands "$@"
}
(( $+functions[_nixling__subcmd__store_commands] )) ||
_nixling__subcmd__store_commands() {
    local commands; commands=(
'verify:Verify a VM'\''s hardlink-backed live store-view' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling store commands' commands "$@"
}
(( $+functions[_nixling__subcmd__store__subcmd__help_commands] )) ||
_nixling__subcmd__store__subcmd__help_commands() {
    local commands; commands=(
'verify:Verify a VM'\''s hardlink-backed live store-view' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling store help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__store__subcmd__help__subcmd__help_commands] )) ||
_nixling__subcmd__store__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'nixling store help help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__store__subcmd__help__subcmd__verify_commands] )) ||
_nixling__subcmd__store__subcmd__help__subcmd__verify_commands() {
    local commands; commands=()
    _describe -t commands 'nixling store help verify commands' commands "$@"
}
(( $+functions[_nixling__subcmd__store__subcmd__verify_commands] )) ||
_nixling__subcmd__store__subcmd__verify_commands() {
    local commands; commands=()
    _describe -t commands 'nixling store verify commands' commands "$@"
}
(( $+functions[_nixling__subcmd__switch_commands] )) ||
_nixling__subcmd__switch_commands() {
    local commands; commands=()
    _describe -t commands 'nixling switch commands' commands "$@"
}
(( $+functions[_nixling__subcmd__test_commands] )) ||
_nixling__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'nixling test commands' commands "$@"
}
(( $+functions[_nixling__subcmd__trust_commands] )) ||
_nixling__subcmd__trust_commands() {
    local commands; commands=()
    _describe -t commands 'nixling trust commands' commands "$@"
}
(( $+functions[_nixling__subcmd__up_commands] )) ||
_nixling__subcmd__up_commands() {
    local commands; commands=()
    _describe -t commands 'nixling up commands' commands "$@"
}
(( $+functions[_nixling__subcmd__usb_commands] )) ||
_nixling__subcmd__usb_commands() {
    local commands; commands=(
'attach:Bind a host USB busid to a VM via the native daemon path' \
'detach:Unbind a host USB busid from a VM via the native daemon path' \
'probe:List daemon-declared USBIP busid claims and lock owners' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling usb commands' commands "$@"
}
(( $+functions[_nixling__subcmd__usb__subcmd__attach_commands] )) ||
_nixling__subcmd__usb__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'nixling usb attach commands' commands "$@"
}
(( $+functions[_nixling__subcmd__usb__subcmd__detach_commands] )) ||
_nixling__subcmd__usb__subcmd__detach_commands() {
    local commands; commands=()
    _describe -t commands 'nixling usb detach commands' commands "$@"
}
(( $+functions[_nixling__subcmd__usb__subcmd__help_commands] )) ||
_nixling__subcmd__usb__subcmd__help_commands() {
    local commands; commands=(
'attach:Bind a host USB busid to a VM via the native daemon path' \
'detach:Unbind a host USB busid from a VM via the native daemon path' \
'probe:List daemon-declared USBIP busid claims and lock owners' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling usb help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__usb__subcmd__help__subcmd__attach_commands] )) ||
_nixling__subcmd__usb__subcmd__help__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'nixling usb help attach commands' commands "$@"
}
(( $+functions[_nixling__subcmd__usb__subcmd__help__subcmd__detach_commands] )) ||
_nixling__subcmd__usb__subcmd__help__subcmd__detach_commands() {
    local commands; commands=()
    _describe -t commands 'nixling usb help detach commands' commands "$@"
}
(( $+functions[_nixling__subcmd__usb__subcmd__help__subcmd__help_commands] )) ||
_nixling__subcmd__usb__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'nixling usb help help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__usb__subcmd__help__subcmd__probe_commands] )) ||
_nixling__subcmd__usb__subcmd__help__subcmd__probe_commands() {
    local commands; commands=()
    _describe -t commands 'nixling usb help probe commands' commands "$@"
}
(( $+functions[_nixling__subcmd__usb__subcmd__probe_commands] )) ||
_nixling__subcmd__usb__subcmd__probe_commands() {
    local commands; commands=()
    _describe -t commands 'nixling usb probe commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm_commands] )) ||
_nixling__subcmd__vm_commands() {
    local commands; commands=(
'start:Start the per-VM DAG (virtiofsd → CH → readiness probes)' \
'stop:Stop the per-VM DAG in reverse topo order' \
'restart:Stop then start; same envelope contract as start' \
'list:Daemon-side runtime view (different from \`nixling list\`, which is the static manifest view)' \
'status:Daemon-side readiness state for a VM (api-ready phase)' \
'exec:Run or manage commands inside a running VM. Use \`nixling vm exec <vm> -- <cmd...>\` for a non-interactive command, \`nixling vm exec -it <vm> -- bash\` for an interactive shell, \`-d\` for a detached command, and \`nixling vm exec <vm> {list|logs|status|kill}\` to manage detached execs' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling vm commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__exec_commands] )) ||
_nixling__subcmd__vm__subcmd__exec_commands() {
    local commands; commands=()
    _describe -t commands 'nixling vm exec commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__help_commands] )) ||
_nixling__subcmd__vm__subcmd__help_commands() {
    local commands; commands=(
'start:Start the per-VM DAG (virtiofsd → CH → readiness probes)' \
'stop:Stop the per-VM DAG in reverse topo order' \
'restart:Stop then start; same envelope contract as start' \
'list:Daemon-side runtime view (different from \`nixling list\`, which is the static manifest view)' \
'status:Daemon-side readiness state for a VM (api-ready phase)' \
'exec:Run or manage commands inside a running VM. Use \`nixling vm exec <vm> -- <cmd...>\` for a non-interactive command, \`nixling vm exec -it <vm> -- bash\` for an interactive shell, \`-d\` for a detached command, and \`nixling vm exec <vm> {list|logs|status|kill}\` to manage detached execs' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling vm help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__help__subcmd__exec_commands] )) ||
_nixling__subcmd__vm__subcmd__help__subcmd__exec_commands() {
    local commands; commands=()
    _describe -t commands 'nixling vm help exec commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__help__subcmd__help_commands] )) ||
_nixling__subcmd__vm__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'nixling vm help help commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__help__subcmd__list_commands] )) ||
_nixling__subcmd__vm__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'nixling vm help list commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__help__subcmd__restart_commands] )) ||
_nixling__subcmd__vm__subcmd__help__subcmd__restart_commands() {
    local commands; commands=()
    _describe -t commands 'nixling vm help restart commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__help__subcmd__start_commands] )) ||
_nixling__subcmd__vm__subcmd__help__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'nixling vm help start commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__help__subcmd__status_commands] )) ||
_nixling__subcmd__vm__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'nixling vm help status commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__help__subcmd__stop_commands] )) ||
_nixling__subcmd__vm__subcmd__help__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'nixling vm help stop commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__list_commands] )) ||
_nixling__subcmd__vm__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'nixling vm list commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__restart_commands] )) ||
_nixling__subcmd__vm__subcmd__restart_commands() {
    local commands; commands=()
    _describe -t commands 'nixling vm restart commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__start_commands] )) ||
_nixling__subcmd__vm__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'nixling vm start commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__status_commands] )) ||
_nixling__subcmd__vm__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'nixling vm status commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__stop_commands] )) ||
_nixling__subcmd__vm__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'nixling vm stop commands' commands "$@"
}

if [ "$funcstack[1]" = "_nixling" ]; then
    _nixling "$@"
else
    compdef _nixling nixling
fi
