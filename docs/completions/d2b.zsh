#compdef d2b

autoload -U is-at-least

_d2b() {
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
":: :_d2b_commands" \
"*::: :->d2b" \
&& ret=0
    case $state in
    (d2b)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-command-$line[1]:"
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
(launch)
_arguments "${_arguments_options[@]}" : \
'--item=[Configured launcher item id. Omit to use the declared default or sole item]:ITEM:_default' \
'(--human)--json[Emit a structured JSON result]' \
'(--json)--human[Force human-readable output]' \
'-h[Print help]' \
'--help[Print help]' \
':target -- Canonical workload target or an unambiguous workload id:_default' \
&& ret=0
;;
(usb)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__usb_commands" \
"*::: :->usb" \
&& ret=0

    case $state in
    (usb)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-usb-command-$line[1]:"
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
(security-key)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__usb__subcmd__security-key_commands" \
"*::: :->security-key" \
&& ret=0

    case $state in
    (security-key)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-usb-security-key-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(sessions)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(cancel)
_arguments "${_arguments_options[@]}" : \
'()--current[Cancel the currently active session]' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
'::session_id -- Session ID to cancel. Mutually exclusive with `--current`:_default' \
&& ret=0
;;
(test)
_arguments "${_arguments_options[@]}" : \
'--dry-run[]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__usb__subcmd__security-key__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-usb-security-key-help-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(sessions)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(cancel)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(test)
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
":: :_d2b__subcmd__usb__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-usb-help-command-$line[1]:"
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
(security-key)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__usb__subcmd__help__subcmd__security-key_commands" \
"*::: :->security-key" \
&& ret=0

    case $state in
    (security-key)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-usb-help-security-key-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(sessions)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(cancel)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(test)
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
;;
(console)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
':vm -- VM name whose foreground serial console should be attached:_default' \
&& ret=0
;;
(audio)
_arguments "${_arguments_options[@]}" : \
'--json[Emit machine-readable JSON output]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__audio_commands" \
"*::: :->audio" \
&& ret=0

    case $state in
    (audio)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-audio-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
'--json[Emit machine-readable JSON output]' \
'-h[Print help]' \
'--help[Print help]' \
'::vm -- Optional VM name; omitted lists audio status for every audio-enabled VM:_default' \
&& ret=0
;;
(mic)
_arguments "${_arguments_options[@]}" : \
'--json[Emit machine-readable JSON output]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':state -- The new grant state to apply:((on\:"Enable the selected audio direction"
off\:"Disable the selected audio direction"))' \
':vm -- VM name whose audio grant should be changed:_default' \
&& ret=0
;;
(speaker)
_arguments "${_arguments_options[@]}" : \
'--json[Emit machine-readable JSON output]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':state -- The new grant state to apply:((on\:"Enable the selected audio direction"
off\:"Disable the selected audio direction"))' \
':vm -- VM name whose audio grant should be changed:_default' \
&& ret=0
;;
(off)
_arguments "${_arguments_options[@]}" : \
'--json[Emit machine-readable JSON output]' \
'-h[Print help]' \
'--help[Print help]' \
':vm -- VM name whose microphone and speaker grants should both be disabled:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__audio__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-audio-help-command-$line[1]:"
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
":: :_d2b__subcmd__host_commands" \
"*::: :->host" \
&& ret=0

    case $state in
    (host)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-host-command-$line[1]:"
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
(migrate-storage)
_arguments "${_arguments_options[@]}" : \
'--from-checkpoint=[Checkpoint ID to roll back]:ID:_default' \
'(--apply --rollback)--dry-run[Plan the storage cutover without mutating host state]' \
'(--dry-run --rollback)--apply[Apply the storage cutover. Currently fails closed until broker support lands]' \
'(--dry-run --apply)--rollback[Roll back from a named storage cutover checkpoint]' \
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
'(--dry-run)--enable[After \`--apply\`, enable d2bd.service via systemctl]' \
'(--dry-run --no-start)--start[After \`--apply --enable\`, start d2bd.service]' \
'(--dry-run --start)--no-start[Explicitly do NOT start d2bd.service post-install]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
'--network[Re-run the network slice of \`host prepare\` (bridge/route/nftables reconcile without starting any VM). Currently the only available scope]' \
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
'--evidence-dir=[Override the evidence directory. Default\: \`/var/lib/d2b/validated\`]:PATH:_files' \
'--scripts-dir=[Override the scripts directory. Default\: best-effort discovery of the installed \`tests/\` share, then \`./tests\`]:PATH:_files' \
'(--apply)--dry-run[Plan\: report which readiness validators WOULD be attested. No evidence is written]' \
'(--dry-run)--apply[Apply\: write \`/var/lib/d2b/validated/<wave>.json\` for every wave whose declared validators are present on disk]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__host__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-host-help-command-$line[1]:"
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
(migrate-storage)
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
":: :_d2b__subcmd__auth_commands" \
"*::: :->auth" \
&& ret=0

    case $state in
    (auth)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-auth-command-$line[1]:"
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
":: :_d2b__subcmd__auth__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-auth-help-command-$line[1]:"
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
(realm)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__realm_commands" \
"*::: :->realm" \
&& ret=0

    case $state in
    (realm)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-realm-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(inspect)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':realm -- Realm path, e.g. `work` or `payments.work`:_default' \
&& ret=0
;;
(enter)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
':realm -- Realm path, e.g. `work` or `payments.work`:_default' \
&& ret=0
;;
(run)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[Emit the outer \`vm exec\` result as JSON]' \
'(--json)--human[Force human output]' \
'-h[Print help]' \
'--help[Print help]' \
':realm -- Realm path, e.g. `work` or `payments.work`:_default' \
'*::argv -- Command to run in the gateway VM, after `--`:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__realm__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-realm-help-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(inspect)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(enter)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(run)
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
(shell)
_arguments "${_arguments_options[@]}" : \
'--name=[Persistent shell session name. Omit to use the target'\''s configured default]:NAME:_default' \
'--force[Detach an existing attached client before attaching to this session]' \
'(--human)--json[Render machine-readable JSON]' \
'(--json)--human[Render human-readable output]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
':vm -- Target address. Local VMs use the fast path; gateway-backed targets route through the realm gateway where supported:_default' \
'::action -- Shell action. Omit to attach to the configured default session:((attach\:"Attach to a persistent shell"
list\:"List persistent shell sessions on a target"
detach\:"Detach a persistent shell session without killing it"
kill\:"Kill a persistent shell session by name"))' \
&& ret=0
;;
(op)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__op_commands" \
"*::: :->op" \
&& ret=0

    case $state in
    (op)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-op-command-$line[1]:"
        case $line[1] in
            (inspect)
_arguments "${_arguments_options[@]}" : \
'--trace-id=[Optional trace id to include in the inspection envelope]:TRACE_ID:_default' \
'--span-id=[Optional span id to include in the inspection envelope]:SPAN_ID:_default' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__op__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-op-help-command-$line[1]:"
        case $line[1] in
            (inspect)
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
":: :_d2b__subcmd__vm_commands" \
"*::: :->vm" \
&& ret=0

    case $state in
    (vm)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-vm-command-$line[1]:"
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
':vm -- VM name as declared in `d2b.vms.<name>`:_default' \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'-f[Skip provider graceful shutdown and use the forced cleanup path]' \
'--force[Skip provider graceful shutdown and use the forced cleanup path]' \
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
'-f[Apply force only to the stop phase before starting again]' \
'--force[Apply force only to the stop phase before starting again]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':vm:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'(--all)--realm=[Route list through a realm gateway VM]:REALM:_default' \
'(--human)--json[]' \
'(--json)--human[]' \
'--all[Include configured realm gateway entrypoints in the list]' \
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
'-d[Start the command detached and print its exec id. Incompatible with \`-i\`/\`-t\`; detached execs are managed with \`d2b vm exec <vm> {list|logs|status|kill}\`]' \
'--detach[Start the command detached and print its exec id. Incompatible with \`-i\`/\`-t\`; detached execs are managed with \`d2b vm exec <vm> {list|logs|status|kill}\`]' \
'-i[Forward host stdin into the guest command (\`-i\`). Requires \`-t\`/\`--tty\`; use \`-it\` for an interactive shell]' \
'--interactive[Forward host stdin into the guest command (\`-i\`). Requires \`-t\`/\`--tty\`; use \`-it\` for an interactive shell]' \
'-t[Allocate a PTY in the guest and put the host terminal in raw mode (\`-t\`). Implies stdin forwarding. Human-only (incompatible with \`--json\`)]' \
'--tty[Allocate a PTY in the guest and put the host terminal in raw mode (\`-t\`). Implies stdin forwarding. Human-only (incompatible with \`--json\`)]' \
'(--human)--json[Emit a single terminal JSON envelope (exit code + source/reason + bounded captured output). Non-interactive only]' \
'(--json)--human[Force human output]' \
'-h[Print help]' \
'--help[Print help]' \
':vm -- VM name as declared in `d2b.vms.<name>`:_default' \
'*::management -- Optional detached exec management form\: `list`, `logs <id> \[--stdout-offset N|--stdout-offset=N\] \[--stderr-offset N|--stderr-offset=N\] \[--max-len N|--max-len=N\]`, `status <id>`, or `kill <id>`. Command execs never use this position\: pass a command after `--` instead:_default' \
&& ret=0
;;
(display)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__vm__subcmd__display_commands" \
"*::: :->display" \
&& ret=0

    case $state in
    (display)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-vm-display-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
'--target=[Optional realm target to filter, for example \`demo.work.d2b\`]:TARGET:_default' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(close)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
':session_id -- Display session id from `d2b vm display list`:_default' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__vm__subcmd__display__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-vm-display-help-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(close)
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
":: :_d2b__subcmd__vm__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-vm-help-command-$line[1]:"
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
(display)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__vm__subcmd__help__subcmd__display_commands" \
"*::: :->display" \
&& ret=0

    case $state in
    (display)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-vm-help-display-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(close)
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
':vm -- VM name as declared in `d2b.vms.<name>`:_default' \
&& ret=0
;;
(down)
_arguments "${_arguments_options[@]}" : \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'-f[Skip provider graceful shutdown and use the forced cleanup path]' \
'--force[Skip provider graceful shutdown and use the forced cleanup path]' \
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
'-f[Apply force only to the stop phase before starting again]' \
'--force[Apply force only to the stop phase before starting again]' \
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
":: :_d2b__subcmd__store_commands" \
"*::: :->store" \
&& ret=0

    case $state in
    (store)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-store-command-$line[1]:"
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
":: :_d2b__subcmd__store__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-store-help-command-$line[1]:"
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
":: :_d2b__subcmd__keys_commands" \
"*::: :->keys" \
&& ret=0

    case $state in
    (keys)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-keys-command-$line[1]:"
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
":: :_d2b__subcmd__keys__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-keys-help-command-$line[1]:"
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
":: :_d2b__subcmd__config_commands" \
"*::: :->config" \
&& ret=0

    case $state in
    (config)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-config-command-$line[1]:"
        case $line[1] in
            (sync)
_arguments "${_arguments_options[@]}" : \
'--guest-path=[Path of the editable guest config INSIDE the VM to pull. Honored only by the legacy operator SSH transport; on guest-control VMs the canonical guest config working copy is read by file id and this flag is rejected]:GUEST_PATH:_default' \
'--host=[Override the SSH host (defaults to the manifest \`static_ip\`). SSH transport only; rejected on guest-control VMs]:HOST:_default' \
'--user=[Override the SSH user (defaults to the manifest \`ssh_user\`). SSH transport only; rejected on guest-control VMs]:USER:_default' \
'--key=[Override the SSH private key path. SSH transport only; rejected on guest-control VMs]:KEY:_files' \
'--known-hosts=[known_hosts file used to verify the VM'\''s host key (defaults to the framework-managed \`/var/lib/d2b/known_hosts.d2b\`). SSH transport only; rejected on guest-control VMs]:KNOWN_HOSTS:_files' \
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
":: :_d2b__subcmd__config__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-config-help-command-$line[1]:"
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
(clipboard)
_arguments "${_arguments_options[@]}" : \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__clipboard_commands" \
"*::: :->clipboard" \
&& ret=0

    case $state in
    (clipboard)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-clipboard-command-$line[1]:"
        case $line[1] in
            (arm)
_arguments "${_arguments_options[@]}" : \
'(--human)--json[Emit a structured JSON envelope]' \
'(--json)--human[Emit a human-readable status line]' \
'-h[Print help (see more with '\''--help'\'')]' \
'--help[Print help (see more with '\''--help'\'')]' \
&& ret=0
;;
(help)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__clipboard__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-clipboard-help-command-$line[1]:"
        case $line[1] in
            (arm)
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
":: :_d2b__subcmd__help_commands" \
"*::: :->help" \
&& ret=0

    case $state in
    (help)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-help-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(launch)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(usb)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__help__subcmd__usb_commands" \
"*::: :->usb" \
&& ret=0

    case $state in
    (usb)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-help-usb-command-$line[1]:"
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
(security-key)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__help__subcmd__usb__subcmd__security-key_commands" \
"*::: :->security-key" \
&& ret=0

    case $state in
    (security-key)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-help-usb-security-key-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(sessions)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(cancel)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(test)
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
&& ret=0
;;
(audio)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__help__subcmd__audio_commands" \
"*::: :->audio" \
&& ret=0

    case $state in
    (audio)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-help-audio-command-$line[1]:"
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
":: :_d2b__subcmd__help__subcmd__host_commands" \
"*::: :->host" \
&& ret=0

    case $state in
    (host)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-help-host-command-$line[1]:"
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
(migrate-storage)
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
":: :_d2b__subcmd__help__subcmd__auth_commands" \
"*::: :->auth" \
&& ret=0

    case $state in
    (auth)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-help-auth-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(realm)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__help__subcmd__realm_commands" \
"*::: :->realm" \
&& ret=0

    case $state in
    (realm)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-help-realm-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(inspect)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(enter)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(run)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(shell)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(op)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__help__subcmd__op_commands" \
"*::: :->op" \
&& ret=0

    case $state in
    (op)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-help-op-command-$line[1]:"
        case $line[1] in
            (inspect)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
        esac
    ;;
esac
;;
(vm)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__help__subcmd__vm_commands" \
"*::: :->vm" \
&& ret=0

    case $state in
    (vm)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-help-vm-command-$line[1]:"
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
(display)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__help__subcmd__vm__subcmd__display_commands" \
"*::: :->display" \
&& ret=0

    case $state in
    (display)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-help-vm-display-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
&& ret=0
;;
(close)
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
":: :_d2b__subcmd__help__subcmd__store_commands" \
"*::: :->store" \
&& ret=0

    case $state in
    (store)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-help-store-command-$line[1]:"
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
":: :_d2b__subcmd__help__subcmd__keys_commands" \
"*::: :->keys" \
&& ret=0

    case $state in
    (keys)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-help-keys-command-$line[1]:"
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
":: :_d2b__subcmd__help__subcmd__config_commands" \
"*::: :->config" \
&& ret=0

    case $state in
    (config)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-help-config-command-$line[1]:"
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
(clipboard)
_arguments "${_arguments_options[@]}" : \
":: :_d2b__subcmd__help__subcmd__clipboard_commands" \
"*::: :->clipboard" \
&& ret=0

    case $state in
    (clipboard)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-help-clipboard-command-$line[1]:"
        case $line[1] in
            (arm)
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

(( $+functions[_d2b_commands] )) ||
_d2b_commands() {
    local commands; commands=(
'list:List declared VMs with daemon runtime state when d2bd is reachable' \
'status:Show per-VM runtime status plus bridge health' \
'launch:Launch a trusted configured workload item through its runtime provider' \
'usb:USB attach / detach / probe' \
'console:Foreground serial console bridge for headless VMs' \
'audio:Per-VM audio status and grant controls' \
'audit:Tail the broker audit log' \
'host:Host-side preflight, install, doctor, and reconcile verbs' \
'auth:Authorisation introspection' \
'realm:Low-level realm gateway helpers' \
'shell:Attach to or manage persistent named guest shells' \
'op:Inspect current constellation operation and trace state' \
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
'clipboard:Clipboard authority operations (picker-driven paste replay via d2b-clipd)' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b commands' commands "$@"
}
(( $+functions[_d2b__subcmd__audio_commands] )) ||
_d2b__subcmd__audio_commands() {
    local commands; commands=(
'status:Show current grant state. With no VM, lists every audio-enabled VM' \
'mic:Grant or revoke microphone access' \
'speaker:Grant or revoke speaker access' \
'off:Revoke both mic and speaker access' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b audio commands' commands "$@"
}
(( $+functions[_d2b__subcmd__audio__subcmd__help_commands] )) ||
_d2b__subcmd__audio__subcmd__help_commands() {
    local commands; commands=(
'status:Show current grant state. With no VM, lists every audio-enabled VM' \
'mic:Grant or revoke microphone access' \
'speaker:Grant or revoke speaker access' \
'off:Revoke both mic and speaker access' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b audio help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__audio__subcmd__help__subcmd__help_commands] )) ||
_d2b__subcmd__audio__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'd2b audio help help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__audio__subcmd__help__subcmd__mic_commands] )) ||
_d2b__subcmd__audio__subcmd__help__subcmd__mic_commands() {
    local commands; commands=()
    _describe -t commands 'd2b audio help mic commands' commands "$@"
}
(( $+functions[_d2b__subcmd__audio__subcmd__help__subcmd__off_commands] )) ||
_d2b__subcmd__audio__subcmd__help__subcmd__off_commands() {
    local commands; commands=()
    _describe -t commands 'd2b audio help off commands' commands "$@"
}
(( $+functions[_d2b__subcmd__audio__subcmd__help__subcmd__speaker_commands] )) ||
_d2b__subcmd__audio__subcmd__help__subcmd__speaker_commands() {
    local commands; commands=()
    _describe -t commands 'd2b audio help speaker commands' commands "$@"
}
(( $+functions[_d2b__subcmd__audio__subcmd__help__subcmd__status_commands] )) ||
_d2b__subcmd__audio__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b audio help status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__audio__subcmd__mic_commands] )) ||
_d2b__subcmd__audio__subcmd__mic_commands() {
    local commands; commands=()
    _describe -t commands 'd2b audio mic commands' commands "$@"
}
(( $+functions[_d2b__subcmd__audio__subcmd__off_commands] )) ||
_d2b__subcmd__audio__subcmd__off_commands() {
    local commands; commands=()
    _describe -t commands 'd2b audio off commands' commands "$@"
}
(( $+functions[_d2b__subcmd__audio__subcmd__speaker_commands] )) ||
_d2b__subcmd__audio__subcmd__speaker_commands() {
    local commands; commands=()
    _describe -t commands 'd2b audio speaker commands' commands "$@"
}
(( $+functions[_d2b__subcmd__audio__subcmd__status_commands] )) ||
_d2b__subcmd__audio__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b audio status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__audit_commands] )) ||
_d2b__subcmd__audit_commands() {
    local commands; commands=()
    _describe -t commands 'd2b audit commands' commands "$@"
}
(( $+functions[_d2b__subcmd__auth_commands] )) ||
_d2b__subcmd__auth_commands() {
    local commands; commands=(
'status:' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b auth commands' commands "$@"
}
(( $+functions[_d2b__subcmd__auth__subcmd__help_commands] )) ||
_d2b__subcmd__auth__subcmd__help_commands() {
    local commands; commands=(
'status:' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b auth help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__auth__subcmd__help__subcmd__help_commands] )) ||
_d2b__subcmd__auth__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'd2b auth help help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__auth__subcmd__help__subcmd__status_commands] )) ||
_d2b__subcmd__auth__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b auth help status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__auth__subcmd__status_commands] )) ||
_d2b__subcmd__auth__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b auth status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__boot_commands] )) ||
_d2b__subcmd__boot_commands() {
    local commands; commands=()
    _describe -t commands 'd2b boot commands' commands "$@"
}
(( $+functions[_d2b__subcmd__build_commands] )) ||
_d2b__subcmd__build_commands() {
    local commands; commands=()
    _describe -t commands 'd2b build commands' commands "$@"
}
(( $+functions[_d2b__subcmd__clipboard_commands] )) ||
_d2b__subcmd__clipboard_commands() {
    local commands; commands=(
'arm:Open the picker and request paste replay for the focused target' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b clipboard commands' commands "$@"
}
(( $+functions[_d2b__subcmd__clipboard__subcmd__arm_commands] )) ||
_d2b__subcmd__clipboard__subcmd__arm_commands() {
    local commands; commands=()
    _describe -t commands 'd2b clipboard arm commands' commands "$@"
}
(( $+functions[_d2b__subcmd__clipboard__subcmd__help_commands] )) ||
_d2b__subcmd__clipboard__subcmd__help_commands() {
    local commands; commands=(
'arm:Open the picker and request paste replay for the focused target' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b clipboard help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__clipboard__subcmd__help__subcmd__arm_commands] )) ||
_d2b__subcmd__clipboard__subcmd__help__subcmd__arm_commands() {
    local commands; commands=()
    _describe -t commands 'd2b clipboard help arm commands' commands "$@"
}
(( $+functions[_d2b__subcmd__clipboard__subcmd__help__subcmd__help_commands] )) ||
_d2b__subcmd__clipboard__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'd2b clipboard help help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__config_commands] )) ||
_d2b__subcmd__config_commands() {
    local commands; commands=(
'sync:Pull the VM'\''s in-guest edited config into a host-side staging file' \
'diff:Diff the staged guest config against a live host-side file' \
'approve:Approve the staged guest config by writing it to a target file' \
'reject:Discard the staged guest config' \
'status:Report whether a VM has a pending (un-approved) staged config' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b config commands' commands "$@"
}
(( $+functions[_d2b__subcmd__config__subcmd__approve_commands] )) ||
_d2b__subcmd__config__subcmd__approve_commands() {
    local commands; commands=()
    _describe -t commands 'd2b config approve commands' commands "$@"
}
(( $+functions[_d2b__subcmd__config__subcmd__diff_commands] )) ||
_d2b__subcmd__config__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'd2b config diff commands' commands "$@"
}
(( $+functions[_d2b__subcmd__config__subcmd__help_commands] )) ||
_d2b__subcmd__config__subcmd__help_commands() {
    local commands; commands=(
'sync:Pull the VM'\''s in-guest edited config into a host-side staging file' \
'diff:Diff the staged guest config against a live host-side file' \
'approve:Approve the staged guest config by writing it to a target file' \
'reject:Discard the staged guest config' \
'status:Report whether a VM has a pending (un-approved) staged config' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b config help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__config__subcmd__help__subcmd__approve_commands] )) ||
_d2b__subcmd__config__subcmd__help__subcmd__approve_commands() {
    local commands; commands=()
    _describe -t commands 'd2b config help approve commands' commands "$@"
}
(( $+functions[_d2b__subcmd__config__subcmd__help__subcmd__diff_commands] )) ||
_d2b__subcmd__config__subcmd__help__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'd2b config help diff commands' commands "$@"
}
(( $+functions[_d2b__subcmd__config__subcmd__help__subcmd__help_commands] )) ||
_d2b__subcmd__config__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'd2b config help help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__config__subcmd__help__subcmd__reject_commands] )) ||
_d2b__subcmd__config__subcmd__help__subcmd__reject_commands() {
    local commands; commands=()
    _describe -t commands 'd2b config help reject commands' commands "$@"
}
(( $+functions[_d2b__subcmd__config__subcmd__help__subcmd__status_commands] )) ||
_d2b__subcmd__config__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b config help status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__config__subcmd__help__subcmd__sync_commands] )) ||
_d2b__subcmd__config__subcmd__help__subcmd__sync_commands() {
    local commands; commands=()
    _describe -t commands 'd2b config help sync commands' commands "$@"
}
(( $+functions[_d2b__subcmd__config__subcmd__reject_commands] )) ||
_d2b__subcmd__config__subcmd__reject_commands() {
    local commands; commands=()
    _describe -t commands 'd2b config reject commands' commands "$@"
}
(( $+functions[_d2b__subcmd__config__subcmd__status_commands] )) ||
_d2b__subcmd__config__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b config status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__config__subcmd__sync_commands] )) ||
_d2b__subcmd__config__subcmd__sync_commands() {
    local commands; commands=()
    _describe -t commands 'd2b config sync commands' commands "$@"
}
(( $+functions[_d2b__subcmd__console_commands] )) ||
_d2b__subcmd__console_commands() {
    local commands; commands=()
    _describe -t commands 'd2b console commands' commands "$@"
}
(( $+functions[_d2b__subcmd__down_commands] )) ||
_d2b__subcmd__down_commands() {
    local commands; commands=()
    _describe -t commands 'd2b down commands' commands "$@"
}
(( $+functions[_d2b__subcmd__gc_commands] )) ||
_d2b__subcmd__gc_commands() {
    local commands; commands=()
    _describe -t commands 'd2b gc commands' commands "$@"
}
(( $+functions[_d2b__subcmd__generations_commands] )) ||
_d2b__subcmd__generations_commands() {
    local commands; commands=()
    _describe -t commands 'd2b generations commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help_commands] )) ||
_d2b__subcmd__help_commands() {
    local commands; commands=(
'list:List declared VMs with daemon runtime state when d2bd is reachable' \
'status:Show per-VM runtime status plus bridge health' \
'launch:Launch a trusted configured workload item through its runtime provider' \
'usb:USB attach / detach / probe' \
'console:Foreground serial console bridge for headless VMs' \
'audio:Per-VM audio status and grant controls' \
'audit:Tail the broker audit log' \
'host:Host-side preflight, install, doctor, and reconcile verbs' \
'auth:Authorisation introspection' \
'realm:Low-level realm gateway helpers' \
'shell:Attach to or manage persistent named guest shells' \
'op:Inspect current constellation operation and trace state' \
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
'clipboard:Clipboard authority operations (picker-driven paste replay via d2b-clipd)' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__audio_commands] )) ||
_d2b__subcmd__help__subcmd__audio_commands() {
    local commands; commands=(
'status:Show current grant state. With no VM, lists every audio-enabled VM' \
'mic:Grant or revoke microphone access' \
'speaker:Grant or revoke speaker access' \
'off:Revoke both mic and speaker access' \
    )
    _describe -t commands 'd2b help audio commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__audio__subcmd__mic_commands] )) ||
_d2b__subcmd__help__subcmd__audio__subcmd__mic_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help audio mic commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__audio__subcmd__off_commands] )) ||
_d2b__subcmd__help__subcmd__audio__subcmd__off_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help audio off commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__audio__subcmd__speaker_commands] )) ||
_d2b__subcmd__help__subcmd__audio__subcmd__speaker_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help audio speaker commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__audio__subcmd__status_commands] )) ||
_d2b__subcmd__help__subcmd__audio__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help audio status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__audit_commands] )) ||
_d2b__subcmd__help__subcmd__audit_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help audit commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__auth_commands] )) ||
_d2b__subcmd__help__subcmd__auth_commands() {
    local commands; commands=(
'status:' \
    )
    _describe -t commands 'd2b help auth commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__auth__subcmd__status_commands] )) ||
_d2b__subcmd__help__subcmd__auth__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help auth status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__boot_commands] )) ||
_d2b__subcmd__help__subcmd__boot_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help boot commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__build_commands] )) ||
_d2b__subcmd__help__subcmd__build_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help build commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__clipboard_commands] )) ||
_d2b__subcmd__help__subcmd__clipboard_commands() {
    local commands; commands=(
'arm:Open the picker and request paste replay for the focused target' \
    )
    _describe -t commands 'd2b help clipboard commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__clipboard__subcmd__arm_commands] )) ||
_d2b__subcmd__help__subcmd__clipboard__subcmd__arm_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help clipboard arm commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__config_commands] )) ||
_d2b__subcmd__help__subcmd__config_commands() {
    local commands; commands=(
'sync:Pull the VM'\''s in-guest edited config into a host-side staging file' \
'diff:Diff the staged guest config against a live host-side file' \
'approve:Approve the staged guest config by writing it to a target file' \
'reject:Discard the staged guest config' \
'status:Report whether a VM has a pending (un-approved) staged config' \
    )
    _describe -t commands 'd2b help config commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__config__subcmd__approve_commands] )) ||
_d2b__subcmd__help__subcmd__config__subcmd__approve_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help config approve commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__config__subcmd__diff_commands] )) ||
_d2b__subcmd__help__subcmd__config__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help config diff commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__config__subcmd__reject_commands] )) ||
_d2b__subcmd__help__subcmd__config__subcmd__reject_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help config reject commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__config__subcmd__status_commands] )) ||
_d2b__subcmd__help__subcmd__config__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help config status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__config__subcmd__sync_commands] )) ||
_d2b__subcmd__help__subcmd__config__subcmd__sync_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help config sync commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__console_commands] )) ||
_d2b__subcmd__help__subcmd__console_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help console commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__down_commands] )) ||
_d2b__subcmd__help__subcmd__down_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help down commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__gc_commands] )) ||
_d2b__subcmd__help__subcmd__gc_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help gc commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__generations_commands] )) ||
_d2b__subcmd__help__subcmd__generations_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help generations commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__help_commands] )) ||
_d2b__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__host_commands] )) ||
_d2b__subcmd__help__subcmd__host_commands() {
    local commands; commands=(
'check:Read-only preflight\: inventories host posture without mutation' \
'prepare:Reconcile host-side state (bridges, nftables, sysctls). --apply mutates' \
'destroy:Tear down host-side state owned by d2b. --apply mutates' \
'doctor:Read-only deep diagnostics for the daemon + broker state' \
'migrate-storage:Plan the one-time storage layout cutover. --apply is fail-closed until broker support lands' \
'install:Install d2bd + broker units onto the host. --apply mutates' \
'reconcile:Reconcile host network state (re-run bridge/route/nftables reconcile without starting any VM)' \
'validate:Run the host-side validator suite and write evidence records' \
    )
    _describe -t commands 'd2b help host commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__host__subcmd__check_commands] )) ||
_d2b__subcmd__help__subcmd__host__subcmd__check_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help host check commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__host__subcmd__destroy_commands] )) ||
_d2b__subcmd__help__subcmd__host__subcmd__destroy_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help host destroy commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__host__subcmd__doctor_commands] )) ||
_d2b__subcmd__help__subcmd__host__subcmd__doctor_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help host doctor commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__host__subcmd__install_commands] )) ||
_d2b__subcmd__help__subcmd__host__subcmd__install_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help host install commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__host__subcmd__migrate-storage_commands] )) ||
_d2b__subcmd__help__subcmd__host__subcmd__migrate-storage_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help host migrate-storage commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__host__subcmd__prepare_commands] )) ||
_d2b__subcmd__help__subcmd__host__subcmd__prepare_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help host prepare commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__host__subcmd__reconcile_commands] )) ||
_d2b__subcmd__help__subcmd__host__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help host reconcile commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__host__subcmd__validate_commands] )) ||
_d2b__subcmd__help__subcmd__host__subcmd__validate_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help host validate commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__keys_commands] )) ||
_d2b__subcmd__help__subcmd__keys_commands() {
    local commands; commands=(
'list:List managed keys (per-VM SSH keypair fingerprints)' \
'show:Show details for a specific VM'\''s managed key' \
'rotate:Rotate the framework-managed per-VM SSH keypair. --apply mutates' \
    )
    _describe -t commands 'd2b help keys commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__keys__subcmd__list_commands] )) ||
_d2b__subcmd__help__subcmd__keys__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help keys list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__keys__subcmd__rotate_commands] )) ||
_d2b__subcmd__help__subcmd__keys__subcmd__rotate_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help keys rotate commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__keys__subcmd__show_commands] )) ||
_d2b__subcmd__help__subcmd__keys__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help keys show commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__launch_commands] )) ||
_d2b__subcmd__help__subcmd__launch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help launch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__list_commands] )) ||
_d2b__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__migrate_commands] )) ||
_d2b__subcmd__help__subcmd__migrate_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help migrate commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__op_commands] )) ||
_d2b__subcmd__help__subcmd__op_commands() {
    local commands; commands=(
'inspect:Inspect current operation/trace state with bounded partial results' \
    )
    _describe -t commands 'd2b help op commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__op__subcmd__inspect_commands] )) ||
_d2b__subcmd__help__subcmd__op__subcmd__inspect_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help op inspect commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__realm_commands] )) ||
_d2b__subcmd__help__subcmd__realm_commands() {
    local commands; commands=(
'list:List local realm policy entrypoints' \
'inspect:Inspect one local realm policy entrypoint' \
'enter:Open an interactive shell inside the realm gateway VM' \
'run:Run a one-shot command inside the realm gateway VM' \
    )
    _describe -t commands 'd2b help realm commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__realm__subcmd__enter_commands] )) ||
_d2b__subcmd__help__subcmd__realm__subcmd__enter_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help realm enter commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__realm__subcmd__inspect_commands] )) ||
_d2b__subcmd__help__subcmd__realm__subcmd__inspect_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help realm inspect commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__realm__subcmd__list_commands] )) ||
_d2b__subcmd__help__subcmd__realm__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help realm list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__realm__subcmd__run_commands] )) ||
_d2b__subcmd__help__subcmd__realm__subcmd__run_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help realm run commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__restart_commands] )) ||
_d2b__subcmd__help__subcmd__restart_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help restart commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__rollback_commands] )) ||
_d2b__subcmd__help__subcmd__rollback_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help rollback commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__rotate-known-host_commands] )) ||
_d2b__subcmd__help__subcmd__rotate-known-host_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help rotate-known-host commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__shell_commands] )) ||
_d2b__subcmd__help__subcmd__shell_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help shell commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__status_commands] )) ||
_d2b__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__store_commands] )) ||
_d2b__subcmd__help__subcmd__store_commands() {
    local commands; commands=(
'verify:Verify a VM'\''s hardlink-backed live store-view' \
    )
    _describe -t commands 'd2b help store commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__store__subcmd__verify_commands] )) ||
_d2b__subcmd__help__subcmd__store__subcmd__verify_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help store verify commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__switch_commands] )) ||
_d2b__subcmd__help__subcmd__switch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help switch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__test_commands] )) ||
_d2b__subcmd__help__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help test commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__trust_commands] )) ||
_d2b__subcmd__help__subcmd__trust_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help trust commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__up_commands] )) ||
_d2b__subcmd__help__subcmd__up_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help up commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__usb_commands] )) ||
_d2b__subcmd__help__subcmd__usb_commands() {
    local commands; commands=(
'attach:Bind a host USB busid to a VM via the native daemon path' \
'detach:Unbind a host USB busid from a VM via the native daemon path' \
'probe:List daemon-declared USBIP session claims and qemu-media USB candidates' \
'security-key:CTAP/WebAuthn security-key proxy status, sessions, and diagnostics' \
    )
    _describe -t commands 'd2b help usb commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__usb__subcmd__attach_commands] )) ||
_d2b__subcmd__help__subcmd__usb__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help usb attach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__usb__subcmd__detach_commands] )) ||
_d2b__subcmd__help__subcmd__usb__subcmd__detach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help usb detach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__usb__subcmd__probe_commands] )) ||
_d2b__subcmd__help__subcmd__usb__subcmd__probe_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help usb probe commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__usb__subcmd__security-key_commands] )) ||
_d2b__subcmd__help__subcmd__usb__subcmd__security-key_commands() {
    local commands; commands=(
'status:Show security-key proxy health, configured keys, and current lease' \
'sessions:Show recent and active security-key request sessions' \
'cancel:Cancel a security-key request session' \
'test:Smoke-check that a VM'\''s virtual security-key device and host broker are healthy' \
    )
    _describe -t commands 'd2b help usb security-key commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__usb__subcmd__security-key__subcmd__cancel_commands] )) ||
_d2b__subcmd__help__subcmd__usb__subcmd__security-key__subcmd__cancel_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help usb security-key cancel commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__usb__subcmd__security-key__subcmd__sessions_commands] )) ||
_d2b__subcmd__help__subcmd__usb__subcmd__security-key__subcmd__sessions_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help usb security-key sessions commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__usb__subcmd__security-key__subcmd__status_commands] )) ||
_d2b__subcmd__help__subcmd__usb__subcmd__security-key__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help usb security-key status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__usb__subcmd__security-key__subcmd__test_commands] )) ||
_d2b__subcmd__help__subcmd__usb__subcmd__security-key__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help usb security-key test commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__vm_commands] )) ||
_d2b__subcmd__help__subcmd__vm_commands() {
    local commands; commands=(
'start:Start the per-VM DAG (virtiofsd → CH → readiness probes)' \
'stop:Stop the per-VM DAG in reverse topo order' \
'restart:Stop then start; same envelope contract as start' \
'list:Daemon-side runtime inventory from d2bd'\''s public socket' \
'status:Daemon-side readiness state for a VM (api-ready phase)' \
'exec:Run or manage commands inside a running VM. Use \`d2b vm exec <vm> -- <cmd...>\` for a non-interactive command, \`d2b vm exec -it <vm> -- bash\` for an interactive shell, \`-d\` for a detached command, and \`d2b vm exec <vm> {list|logs|status|kill}\` to manage detached execs' \
'display:Manage gateway display sessions for provider-backed targets' \
    )
    _describe -t commands 'd2b help vm commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__vm__subcmd__display_commands] )) ||
_d2b__subcmd__help__subcmd__vm__subcmd__display_commands() {
    local commands; commands=(
'list:List active gateway display sessions' \
'close:Close a gateway display session by id' \
    )
    _describe -t commands 'd2b help vm display commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__vm__subcmd__display__subcmd__close_commands] )) ||
_d2b__subcmd__help__subcmd__vm__subcmd__display__subcmd__close_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help vm display close commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__vm__subcmd__display__subcmd__list_commands] )) ||
_d2b__subcmd__help__subcmd__vm__subcmd__display__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help vm display list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__vm__subcmd__exec_commands] )) ||
_d2b__subcmd__help__subcmd__vm__subcmd__exec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help vm exec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__vm__subcmd__list_commands] )) ||
_d2b__subcmd__help__subcmd__vm__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help vm list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__vm__subcmd__restart_commands] )) ||
_d2b__subcmd__help__subcmd__vm__subcmd__restart_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help vm restart commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__vm__subcmd__start_commands] )) ||
_d2b__subcmd__help__subcmd__vm__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help vm start commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__vm__subcmd__status_commands] )) ||
_d2b__subcmd__help__subcmd__vm__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help vm status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__help__subcmd__vm__subcmd__stop_commands] )) ||
_d2b__subcmd__help__subcmd__vm__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'd2b help vm stop commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host_commands] )) ||
_d2b__subcmd__host_commands() {
    local commands; commands=(
'check:Read-only preflight\: inventories host posture without mutation' \
'prepare:Reconcile host-side state (bridges, nftables, sysctls). --apply mutates' \
'destroy:Tear down host-side state owned by d2b. --apply mutates' \
'doctor:Read-only deep diagnostics for the daemon + broker state' \
'migrate-storage:Plan the one-time storage layout cutover. --apply is fail-closed until broker support lands' \
'install:Install d2bd + broker units onto the host. --apply mutates' \
'reconcile:Reconcile host network state (re-run bridge/route/nftables reconcile without starting any VM)' \
'validate:Run the host-side validator suite and write evidence records' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b host commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__check_commands] )) ||
_d2b__subcmd__host__subcmd__check_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host check commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__destroy_commands] )) ||
_d2b__subcmd__host__subcmd__destroy_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host destroy commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__doctor_commands] )) ||
_d2b__subcmd__host__subcmd__doctor_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host doctor commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__help_commands] )) ||
_d2b__subcmd__host__subcmd__help_commands() {
    local commands; commands=(
'check:Read-only preflight\: inventories host posture without mutation' \
'prepare:Reconcile host-side state (bridges, nftables, sysctls). --apply mutates' \
'destroy:Tear down host-side state owned by d2b. --apply mutates' \
'doctor:Read-only deep diagnostics for the daemon + broker state' \
'migrate-storage:Plan the one-time storage layout cutover. --apply is fail-closed until broker support lands' \
'install:Install d2bd + broker units onto the host. --apply mutates' \
'reconcile:Reconcile host network state (re-run bridge/route/nftables reconcile without starting any VM)' \
'validate:Run the host-side validator suite and write evidence records' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b host help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__help__subcmd__check_commands] )) ||
_d2b__subcmd__host__subcmd__help__subcmd__check_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host help check commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__help__subcmd__destroy_commands] )) ||
_d2b__subcmd__host__subcmd__help__subcmd__destroy_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host help destroy commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__help__subcmd__doctor_commands] )) ||
_d2b__subcmd__host__subcmd__help__subcmd__doctor_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host help doctor commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__help__subcmd__help_commands] )) ||
_d2b__subcmd__host__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host help help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__help__subcmd__install_commands] )) ||
_d2b__subcmd__host__subcmd__help__subcmd__install_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host help install commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__help__subcmd__migrate-storage_commands] )) ||
_d2b__subcmd__host__subcmd__help__subcmd__migrate-storage_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host help migrate-storage commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__help__subcmd__prepare_commands] )) ||
_d2b__subcmd__host__subcmd__help__subcmd__prepare_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host help prepare commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__help__subcmd__reconcile_commands] )) ||
_d2b__subcmd__host__subcmd__help__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host help reconcile commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__help__subcmd__validate_commands] )) ||
_d2b__subcmd__host__subcmd__help__subcmd__validate_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host help validate commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__install_commands] )) ||
_d2b__subcmd__host__subcmd__install_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host install commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__migrate-storage_commands] )) ||
_d2b__subcmd__host__subcmd__migrate-storage_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host migrate-storage commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__prepare_commands] )) ||
_d2b__subcmd__host__subcmd__prepare_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host prepare commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__reconcile_commands] )) ||
_d2b__subcmd__host__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host reconcile commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__validate_commands] )) ||
_d2b__subcmd__host__subcmd__validate_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host validate commands' commands "$@"
}
(( $+functions[_d2b__subcmd__keys_commands] )) ||
_d2b__subcmd__keys_commands() {
    local commands; commands=(
'list:List managed keys (per-VM SSH keypair fingerprints)' \
'show:Show details for a specific VM'\''s managed key' \
'rotate:Rotate the framework-managed per-VM SSH keypair. --apply mutates' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b keys commands' commands "$@"
}
(( $+functions[_d2b__subcmd__keys__subcmd__help_commands] )) ||
_d2b__subcmd__keys__subcmd__help_commands() {
    local commands; commands=(
'list:List managed keys (per-VM SSH keypair fingerprints)' \
'show:Show details for a specific VM'\''s managed key' \
'rotate:Rotate the framework-managed per-VM SSH keypair. --apply mutates' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b keys help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__keys__subcmd__help__subcmd__help_commands] )) ||
_d2b__subcmd__keys__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'd2b keys help help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__keys__subcmd__help__subcmd__list_commands] )) ||
_d2b__subcmd__keys__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b keys help list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__keys__subcmd__help__subcmd__rotate_commands] )) ||
_d2b__subcmd__keys__subcmd__help__subcmd__rotate_commands() {
    local commands; commands=()
    _describe -t commands 'd2b keys help rotate commands' commands "$@"
}
(( $+functions[_d2b__subcmd__keys__subcmd__help__subcmd__show_commands] )) ||
_d2b__subcmd__keys__subcmd__help__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'd2b keys help show commands' commands "$@"
}
(( $+functions[_d2b__subcmd__keys__subcmd__list_commands] )) ||
_d2b__subcmd__keys__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b keys list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__keys__subcmd__rotate_commands] )) ||
_d2b__subcmd__keys__subcmd__rotate_commands() {
    local commands; commands=()
    _describe -t commands 'd2b keys rotate commands' commands "$@"
}
(( $+functions[_d2b__subcmd__keys__subcmd__show_commands] )) ||
_d2b__subcmd__keys__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'd2b keys show commands' commands "$@"
}
(( $+functions[_d2b__subcmd__launch_commands] )) ||
_d2b__subcmd__launch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b launch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__list_commands] )) ||
_d2b__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__migrate_commands] )) ||
_d2b__subcmd__migrate_commands() {
    local commands; commands=()
    _describe -t commands 'd2b migrate commands' commands "$@"
}
(( $+functions[_d2b__subcmd__op_commands] )) ||
_d2b__subcmd__op_commands() {
    local commands; commands=(
'inspect:Inspect current operation/trace state with bounded partial results' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b op commands' commands "$@"
}
(( $+functions[_d2b__subcmd__op__subcmd__help_commands] )) ||
_d2b__subcmd__op__subcmd__help_commands() {
    local commands; commands=(
'inspect:Inspect current operation/trace state with bounded partial results' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b op help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__op__subcmd__help__subcmd__help_commands] )) ||
_d2b__subcmd__op__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'd2b op help help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__op__subcmd__help__subcmd__inspect_commands] )) ||
_d2b__subcmd__op__subcmd__help__subcmd__inspect_commands() {
    local commands; commands=()
    _describe -t commands 'd2b op help inspect commands' commands "$@"
}
(( $+functions[_d2b__subcmd__op__subcmd__inspect_commands] )) ||
_d2b__subcmd__op__subcmd__inspect_commands() {
    local commands; commands=()
    _describe -t commands 'd2b op inspect commands' commands "$@"
}
(( $+functions[_d2b__subcmd__realm_commands] )) ||
_d2b__subcmd__realm_commands() {
    local commands; commands=(
'list:List local realm policy entrypoints' \
'inspect:Inspect one local realm policy entrypoint' \
'enter:Open an interactive shell inside the realm gateway VM' \
'run:Run a one-shot command inside the realm gateway VM' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b realm commands' commands "$@"
}
(( $+functions[_d2b__subcmd__realm__subcmd__enter_commands] )) ||
_d2b__subcmd__realm__subcmd__enter_commands() {
    local commands; commands=()
    _describe -t commands 'd2b realm enter commands' commands "$@"
}
(( $+functions[_d2b__subcmd__realm__subcmd__help_commands] )) ||
_d2b__subcmd__realm__subcmd__help_commands() {
    local commands; commands=(
'list:List local realm policy entrypoints' \
'inspect:Inspect one local realm policy entrypoint' \
'enter:Open an interactive shell inside the realm gateway VM' \
'run:Run a one-shot command inside the realm gateway VM' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b realm help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__realm__subcmd__help__subcmd__enter_commands] )) ||
_d2b__subcmd__realm__subcmd__help__subcmd__enter_commands() {
    local commands; commands=()
    _describe -t commands 'd2b realm help enter commands' commands "$@"
}
(( $+functions[_d2b__subcmd__realm__subcmd__help__subcmd__help_commands] )) ||
_d2b__subcmd__realm__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'd2b realm help help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__realm__subcmd__help__subcmd__inspect_commands] )) ||
_d2b__subcmd__realm__subcmd__help__subcmd__inspect_commands() {
    local commands; commands=()
    _describe -t commands 'd2b realm help inspect commands' commands "$@"
}
(( $+functions[_d2b__subcmd__realm__subcmd__help__subcmd__list_commands] )) ||
_d2b__subcmd__realm__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b realm help list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__realm__subcmd__help__subcmd__run_commands] )) ||
_d2b__subcmd__realm__subcmd__help__subcmd__run_commands() {
    local commands; commands=()
    _describe -t commands 'd2b realm help run commands' commands "$@"
}
(( $+functions[_d2b__subcmd__realm__subcmd__inspect_commands] )) ||
_d2b__subcmd__realm__subcmd__inspect_commands() {
    local commands; commands=()
    _describe -t commands 'd2b realm inspect commands' commands "$@"
}
(( $+functions[_d2b__subcmd__realm__subcmd__list_commands] )) ||
_d2b__subcmd__realm__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b realm list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__realm__subcmd__run_commands] )) ||
_d2b__subcmd__realm__subcmd__run_commands() {
    local commands; commands=()
    _describe -t commands 'd2b realm run commands' commands "$@"
}
(( $+functions[_d2b__subcmd__restart_commands] )) ||
_d2b__subcmd__restart_commands() {
    local commands; commands=()
    _describe -t commands 'd2b restart commands' commands "$@"
}
(( $+functions[_d2b__subcmd__rollback_commands] )) ||
_d2b__subcmd__rollback_commands() {
    local commands; commands=()
    _describe -t commands 'd2b rollback commands' commands "$@"
}
(( $+functions[_d2b__subcmd__rotate-known-host_commands] )) ||
_d2b__subcmd__rotate-known-host_commands() {
    local commands; commands=()
    _describe -t commands 'd2b rotate-known-host commands' commands "$@"
}
(( $+functions[_d2b__subcmd__shell_commands] )) ||
_d2b__subcmd__shell_commands() {
    local commands; commands=()
    _describe -t commands 'd2b shell commands' commands "$@"
}
(( $+functions[_d2b__subcmd__status_commands] )) ||
_d2b__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__store_commands] )) ||
_d2b__subcmd__store_commands() {
    local commands; commands=(
'verify:Verify a VM'\''s hardlink-backed live store-view' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b store commands' commands "$@"
}
(( $+functions[_d2b__subcmd__store__subcmd__help_commands] )) ||
_d2b__subcmd__store__subcmd__help_commands() {
    local commands; commands=(
'verify:Verify a VM'\''s hardlink-backed live store-view' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b store help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__store__subcmd__help__subcmd__help_commands] )) ||
_d2b__subcmd__store__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'd2b store help help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__store__subcmd__help__subcmd__verify_commands] )) ||
_d2b__subcmd__store__subcmd__help__subcmd__verify_commands() {
    local commands; commands=()
    _describe -t commands 'd2b store help verify commands' commands "$@"
}
(( $+functions[_d2b__subcmd__store__subcmd__verify_commands] )) ||
_d2b__subcmd__store__subcmd__verify_commands() {
    local commands; commands=()
    _describe -t commands 'd2b store verify commands' commands "$@"
}
(( $+functions[_d2b__subcmd__switch_commands] )) ||
_d2b__subcmd__switch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b switch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__test_commands] )) ||
_d2b__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'd2b test commands' commands "$@"
}
(( $+functions[_d2b__subcmd__trust_commands] )) ||
_d2b__subcmd__trust_commands() {
    local commands; commands=()
    _describe -t commands 'd2b trust commands' commands "$@"
}
(( $+functions[_d2b__subcmd__up_commands] )) ||
_d2b__subcmd__up_commands() {
    local commands; commands=()
    _describe -t commands 'd2b up commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb_commands] )) ||
_d2b__subcmd__usb_commands() {
    local commands; commands=(
'attach:Bind a host USB busid to a VM via the native daemon path' \
'detach:Unbind a host USB busid from a VM via the native daemon path' \
'probe:List daemon-declared USBIP session claims and qemu-media USB candidates' \
'security-key:CTAP/WebAuthn security-key proxy status, sessions, and diagnostics' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b usb commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__attach_commands] )) ||
_d2b__subcmd__usb__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb attach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__detach_commands] )) ||
_d2b__subcmd__usb__subcmd__detach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb detach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__help_commands] )) ||
_d2b__subcmd__usb__subcmd__help_commands() {
    local commands; commands=(
'attach:Bind a host USB busid to a VM via the native daemon path' \
'detach:Unbind a host USB busid from a VM via the native daemon path' \
'probe:List daemon-declared USBIP session claims and qemu-media USB candidates' \
'security-key:CTAP/WebAuthn security-key proxy status, sessions, and diagnostics' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b usb help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__help__subcmd__attach_commands] )) ||
_d2b__subcmd__usb__subcmd__help__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb help attach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__help__subcmd__detach_commands] )) ||
_d2b__subcmd__usb__subcmd__help__subcmd__detach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb help detach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__help__subcmd__help_commands] )) ||
_d2b__subcmd__usb__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb help help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__help__subcmd__probe_commands] )) ||
_d2b__subcmd__usb__subcmd__help__subcmd__probe_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb help probe commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__help__subcmd__security-key_commands] )) ||
_d2b__subcmd__usb__subcmd__help__subcmd__security-key_commands() {
    local commands; commands=(
'status:Show security-key proxy health, configured keys, and current lease' \
'sessions:Show recent and active security-key request sessions' \
'cancel:Cancel a security-key request session' \
'test:Smoke-check that a VM'\''s virtual security-key device and host broker are healthy' \
    )
    _describe -t commands 'd2b usb help security-key commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__help__subcmd__security-key__subcmd__cancel_commands] )) ||
_d2b__subcmd__usb__subcmd__help__subcmd__security-key__subcmd__cancel_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb help security-key cancel commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__help__subcmd__security-key__subcmd__sessions_commands] )) ||
_d2b__subcmd__usb__subcmd__help__subcmd__security-key__subcmd__sessions_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb help security-key sessions commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__help__subcmd__security-key__subcmd__status_commands] )) ||
_d2b__subcmd__usb__subcmd__help__subcmd__security-key__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb help security-key status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__help__subcmd__security-key__subcmd__test_commands] )) ||
_d2b__subcmd__usb__subcmd__help__subcmd__security-key__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb help security-key test commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__probe_commands] )) ||
_d2b__subcmd__usb__subcmd__probe_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb probe commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__security-key_commands] )) ||
_d2b__subcmd__usb__subcmd__security-key_commands() {
    local commands; commands=(
'status:Show security-key proxy health, configured keys, and current lease' \
'sessions:Show recent and active security-key request sessions' \
'cancel:Cancel a security-key request session' \
'test:Smoke-check that a VM'\''s virtual security-key device and host broker are healthy' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b usb security-key commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__security-key__subcmd__cancel_commands] )) ||
_d2b__subcmd__usb__subcmd__security-key__subcmd__cancel_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb security-key cancel commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__security-key__subcmd__help_commands] )) ||
_d2b__subcmd__usb__subcmd__security-key__subcmd__help_commands() {
    local commands; commands=(
'status:Show security-key proxy health, configured keys, and current lease' \
'sessions:Show recent and active security-key request sessions' \
'cancel:Cancel a security-key request session' \
'test:Smoke-check that a VM'\''s virtual security-key device and host broker are healthy' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b usb security-key help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__security-key__subcmd__help__subcmd__cancel_commands] )) ||
_d2b__subcmd__usb__subcmd__security-key__subcmd__help__subcmd__cancel_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb security-key help cancel commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__security-key__subcmd__help__subcmd__help_commands] )) ||
_d2b__subcmd__usb__subcmd__security-key__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb security-key help help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__security-key__subcmd__help__subcmd__sessions_commands] )) ||
_d2b__subcmd__usb__subcmd__security-key__subcmd__help__subcmd__sessions_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb security-key help sessions commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__security-key__subcmd__help__subcmd__status_commands] )) ||
_d2b__subcmd__usb__subcmd__security-key__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb security-key help status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__security-key__subcmd__help__subcmd__test_commands] )) ||
_d2b__subcmd__usb__subcmd__security-key__subcmd__help__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb security-key help test commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__security-key__subcmd__sessions_commands] )) ||
_d2b__subcmd__usb__subcmd__security-key__subcmd__sessions_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb security-key sessions commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__security-key__subcmd__status_commands] )) ||
_d2b__subcmd__usb__subcmd__security-key__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb security-key status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__usb__subcmd__security-key__subcmd__test_commands] )) ||
_d2b__subcmd__usb__subcmd__security-key__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'd2b usb security-key test commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm_commands] )) ||
_d2b__subcmd__vm_commands() {
    local commands; commands=(
'start:Start the per-VM DAG (virtiofsd → CH → readiness probes)' \
'stop:Stop the per-VM DAG in reverse topo order' \
'restart:Stop then start; same envelope contract as start' \
'list:Daemon-side runtime inventory from d2bd'\''s public socket' \
'status:Daemon-side readiness state for a VM (api-ready phase)' \
'exec:Run or manage commands inside a running VM. Use \`d2b vm exec <vm> -- <cmd...>\` for a non-interactive command, \`d2b vm exec -it <vm> -- bash\` for an interactive shell, \`-d\` for a detached command, and \`d2b vm exec <vm> {list|logs|status|kill}\` to manage detached execs' \
'display:Manage gateway display sessions for provider-backed targets' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b vm commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__display_commands] )) ||
_d2b__subcmd__vm__subcmd__display_commands() {
    local commands; commands=(
'list:List active gateway display sessions' \
'close:Close a gateway display session by id' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b vm display commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__display__subcmd__close_commands] )) ||
_d2b__subcmd__vm__subcmd__display__subcmd__close_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm display close commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__display__subcmd__help_commands] )) ||
_d2b__subcmd__vm__subcmd__display__subcmd__help_commands() {
    local commands; commands=(
'list:List active gateway display sessions' \
'close:Close a gateway display session by id' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b vm display help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__display__subcmd__help__subcmd__close_commands] )) ||
_d2b__subcmd__vm__subcmd__display__subcmd__help__subcmd__close_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm display help close commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__display__subcmd__help__subcmd__help_commands] )) ||
_d2b__subcmd__vm__subcmd__display__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm display help help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__display__subcmd__help__subcmd__list_commands] )) ||
_d2b__subcmd__vm__subcmd__display__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm display help list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__display__subcmd__list_commands] )) ||
_d2b__subcmd__vm__subcmd__display__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm display list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__exec_commands] )) ||
_d2b__subcmd__vm__subcmd__exec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm exec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__help_commands] )) ||
_d2b__subcmd__vm__subcmd__help_commands() {
    local commands; commands=(
'start:Start the per-VM DAG (virtiofsd → CH → readiness probes)' \
'stop:Stop the per-VM DAG in reverse topo order' \
'restart:Stop then start; same envelope contract as start' \
'list:Daemon-side runtime inventory from d2bd'\''s public socket' \
'status:Daemon-side readiness state for a VM (api-ready phase)' \
'exec:Run or manage commands inside a running VM. Use \`d2b vm exec <vm> -- <cmd...>\` for a non-interactive command, \`d2b vm exec -it <vm> -- bash\` for an interactive shell, \`-d\` for a detached command, and \`d2b vm exec <vm> {list|logs|status|kill}\` to manage detached execs' \
'display:Manage gateway display sessions for provider-backed targets' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'd2b vm help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__help__subcmd__display_commands] )) ||
_d2b__subcmd__vm__subcmd__help__subcmd__display_commands() {
    local commands; commands=(
'list:List active gateway display sessions' \
'close:Close a gateway display session by id' \
    )
    _describe -t commands 'd2b vm help display commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__help__subcmd__display__subcmd__close_commands] )) ||
_d2b__subcmd__vm__subcmd__help__subcmd__display__subcmd__close_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm help display close commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__help__subcmd__display__subcmd__list_commands] )) ||
_d2b__subcmd__vm__subcmd__help__subcmd__display__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm help display list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__help__subcmd__exec_commands] )) ||
_d2b__subcmd__vm__subcmd__help__subcmd__exec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm help exec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__help__subcmd__help_commands] )) ||
_d2b__subcmd__vm__subcmd__help__subcmd__help_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm help help commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__help__subcmd__list_commands] )) ||
_d2b__subcmd__vm__subcmd__help__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm help list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__help__subcmd__restart_commands] )) ||
_d2b__subcmd__vm__subcmd__help__subcmd__restart_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm help restart commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__help__subcmd__start_commands] )) ||
_d2b__subcmd__vm__subcmd__help__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm help start commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__help__subcmd__status_commands] )) ||
_d2b__subcmd__vm__subcmd__help__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm help status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__help__subcmd__stop_commands] )) ||
_d2b__subcmd__vm__subcmd__help__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm help stop commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__list_commands] )) ||
_d2b__subcmd__vm__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__restart_commands] )) ||
_d2b__subcmd__vm__subcmd__restart_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm restart commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__start_commands] )) ||
_d2b__subcmd__vm__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm start commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__status_commands] )) ||
_d2b__subcmd__vm__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__vm__subcmd__stop_commands] )) ||
_d2b__subcmd__vm__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'd2b vm stop commands' commands "$@"
}

if [ "$funcstack[1]" = "_d2b" ]; then
    _d2b "$@"
else
    compdef _d2b d2b
fi
