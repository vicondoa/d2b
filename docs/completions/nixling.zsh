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
'-h[Print help]' \
'--help[Print help]' \
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
'--read-only[Mandatory\: the W3 doctor verb is read-only. Mutation forms are W4 deliverables]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(install)
_arguments "${_arguments_options[@]}" : \
'(--apply --enable --start --no-start)--dry-run[W9\: \`--dry-run\` reports the planned install steps]' \
'(--dry-run)--apply[W15\: \`--apply\` performs the install through the daemon → broker \`RunHostInstall\` path]' \
'(--dry-run)--enable[W9\: After \`--apply\`, enable nixlingd.service via systemctl]' \
'(--dry-run --no-start)--start[W9\: After \`--apply --enable\`, start nixlingd.service]' \
'(--dry-run --start)--no-start[W9\: Explicitly do NOT start nixlingd.service post-install]' \
'(--human)--json[]' \
'(--json)--human[]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
'--network[Required for P3\: re-run the network slice of \`host prepare\` and clear the daemon'\''s net-route preflight counter. Today this is the only available scope; future P-phases may add other scopes (e.g. \`--ownership\`)]' \
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
'--wave=[Restrict to a single wave (e.g. \`--wave p1\`). Other waves are reported as \`skipped\`]:WAVE:_default' \
'--operator-signature=[Override the per-wave operator signature. When unset, the verb derives a deterministic sha256 signature from \`hostname|wave|scripts_dir|timestamp\`]:SIGNATURE:_default' \
'--evidence-dir=[Override the evidence directory. Default\: \`/var/lib/nixling/validated\` (the W18 gate path)]:PATH:_files' \
'--scripts-dir=[Override the scripts directory. Default\: best-effort discovery of the installed \`tests/\` share, then \`./tests\`]:PATH:_files' \
'(--apply)--dry-run[Plan\: report which W18 readiness waves WOULD be attested. No evidence is written]' \
'(--dry-run)--apply[Apply\: write the canonical \`/var/lib/nixling/validated/<wave>.json\` evidence record for every wave whose declared validators are present on disk]' \
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
'list:' \
'status:' \
'usb:' \
'console:Foreground serial console bridge for headless VMs. P7fu2\: the bash runtime that backed this verb pre-v1.0 was retired in P6; the daemon-native console surface is queued for v1.1+. Calling this in v1.0 surfaces a typed exit-78 envelope per ADR 0015' \
'audio:Per-VM audio grant bridge. P7fu2\: the bash runtime that backed this verb pre-v1.0 was retired in P6; the daemon-native audio surface is queued for v1.1+. Calling this in v1.0 surfaces a typed exit-78 envelope per ADR 0015' \
'audit:' \
'host:' \
'auth:' \
'vm:W4-H7 / P4\: per-VM lifecycle verbs routed through \`nixlingd\`. \`--apply\` is daemon-only; failure modes surface as typed envelopes. \`--dry-run\` returns the DAG the supervisor would drive' \
'up:P4 alias for \`vm start <vm>\`. Daemon-native; no bash fallback' \
'down:P4 alias for \`vm stop <vm>\`. Daemon-native; no bash fallback' \
'restart:P4 alias for \`vm restart <vm>\`. Daemon-native; no bash fallback' \
'build:W7-H1\: \`nixling build <vm>\` — non-destructive eval+build of the per-VM toplevel' \
'generations:W7-H2\: \`nixling generations <vm>\` — lists current/booted/N' \
'switch:W7-H3\: \`nixling switch <vm> \[--apply|--dry-run\]\` — atomic activation. \`--apply\` dispatches through \`nixlingd\` → broker \`RunActivation\` (v1.0 daemon-only per ADR 0015); \`--dry-run\` returns the planned activation' \
'boot:W7-H4\: \`nixling boot <vm>\` — stage for next boot only' \
'test:W7-H5\: \`nixling test <vm>\` — activate-but-rollback-on-reboot' \
'rollback:W7-H6\: \`nixling rollback <vm>\` — back to the previous generation' \
'gc:W7-H7\: \`nixling gc \[--apply|--dry-run\]\` — store cleanup' \
'keys:W8\: managed-key + trust lifecycle verbs (list / show / rotate). \`--apply\` dispatches through \`nixlingd\` → broker \`RunKeysRotate\` (v1.0 daemon-only per ADR 0015)' \
'trust:W8\: \`nixling trust <vm>\` (top-level, NOT under \`keys\`). Trust a host key on first use (TOFU) through the daemon / broker \`RunHostKeyTrust\` op. Bash runtime retired in P6' \
'rotate-known-host:W8\: \`nixling rotate-known-host <vm>\` (top-level, NOT under \`keys\`). Rotate the consumer'\''s recorded known-host entry via the daemon / broker \`RunRotateKnownHost\` op. Bash runtime retired in P6' \
'migrate:W9\: \`nixling migrate\` — analyze the current host config and emit a migration plan to the daemon-experimental path. \`--apply\` dispatches the broker \`RunMigrate\` op (daemon-only since P6; the historical bash dispatch path was retired in the same wave)' \
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
'list:' \
'status:' \
'usb:' \
'console:Foreground serial console bridge for headless VMs. P7fu2\: the bash runtime that backed this verb pre-v1.0 was retired in P6; the daemon-native console surface is queued for v1.1+. Calling this in v1.0 surfaces a typed exit-78 envelope per ADR 0015' \
'audio:Per-VM audio grant bridge. P7fu2\: the bash runtime that backed this verb pre-v1.0 was retired in P6; the daemon-native audio surface is queued for v1.1+. Calling this in v1.0 surfaces a typed exit-78 envelope per ADR 0015' \
'audit:' \
'host:' \
'auth:' \
'vm:W4-H7 / P4\: per-VM lifecycle verbs routed through \`nixlingd\`. \`--apply\` is daemon-only; failure modes surface as typed envelopes. \`--dry-run\` returns the DAG the supervisor would drive' \
'up:P4 alias for \`vm start <vm>\`. Daemon-native; no bash fallback' \
'down:P4 alias for \`vm stop <vm>\`. Daemon-native; no bash fallback' \
'restart:P4 alias for \`vm restart <vm>\`. Daemon-native; no bash fallback' \
'build:W7-H1\: \`nixling build <vm>\` — non-destructive eval+build of the per-VM toplevel' \
'generations:W7-H2\: \`nixling generations <vm>\` — lists current/booted/N' \
'switch:W7-H3\: \`nixling switch <vm> \[--apply|--dry-run\]\` — atomic activation. \`--apply\` dispatches through \`nixlingd\` → broker \`RunActivation\` (v1.0 daemon-only per ADR 0015); \`--dry-run\` returns the planned activation' \
'boot:W7-H4\: \`nixling boot <vm>\` — stage for next boot only' \
'test:W7-H5\: \`nixling test <vm>\` — activate-but-rollback-on-reboot' \
'rollback:W7-H6\: \`nixling rollback <vm>\` — back to the previous generation' \
'gc:W7-H7\: \`nixling gc \[--apply|--dry-run\]\` — store cleanup' \
'keys:W8\: managed-key + trust lifecycle verbs (list / show / rotate). \`--apply\` dispatches through \`nixlingd\` → broker \`RunKeysRotate\` (v1.0 daemon-only per ADR 0015)' \
'trust:W8\: \`nixling trust <vm>\` (top-level, NOT under \`keys\`). Trust a host key on first use (TOFU) through the daemon / broker \`RunHostKeyTrust\` op. Bash runtime retired in P6' \
'rotate-known-host:W8\: \`nixling rotate-known-host <vm>\` (top-level, NOT under \`keys\`). Rotate the consumer'\''s recorded known-host entry via the daemon / broker \`RunRotateKnownHost\` op. Bash runtime retired in P6' \
'migrate:W9\: \`nixling migrate\` — analyze the current host config and emit a migration plan to the daemon-experimental path. \`--apply\` dispatches the broker \`RunMigrate\` op (daemon-only since P6; the historical bash dispatch path was retired in the same wave)' \
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
'check:' \
'prepare:W3fu1 H1 (product-1, software-1)\: native \`host prepare\` verb. \`--apply\` is mandatory for mutation; without it the command refuses with \`--apply-or-dry-run-required\` exit 78' \
'destroy:W3fu1 H1\: native \`host destroy\` verb. Same mandatory-flag contract as \`prepare\`' \
'doctor:W3fu1 H1\: native \`host doctor\` verb. \`--read-only\` is mandatory' \
'install:W15 (software-1, product-1)\: native \`host install\` routes \`--apply\` through the daemon → broker \`RunHostInstall\` path' \
'reconcile:P3 ph3-p3-net-route-degraded-mode\: SOLE mutating recovery verb after the daemon-side net-route preflight has engaged operator-only mode. Re-runs the broker-side net slice of \`host prepare\` (nftables host scope + per-env routes + per-env ipv6 sysctls) and clears the persistent consecutive-failure counter on success' \
'validate:P5 ph5-p5-host-validate-verb\: composite preflight that inventories per-wave Layer-2 validators and (with \`--apply\`) writes the canonical W18 evidence records consumed by \`nixos-modules/options-daemon.nix\:validationEvidencePresent\`' \
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
    )
    _describe -t commands 'nixling help vm commands' commands "$@"
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
(( $+functions[_nixling__subcmd__help__subcmd__vm__subcmd__stop_commands] )) ||
_nixling__subcmd__help__subcmd__vm__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'nixling help vm stop commands' commands "$@"
}
(( $+functions[_nixling__subcmd__host_commands] )) ||
_nixling__subcmd__host_commands() {
    local commands; commands=(
'check:' \
'prepare:W3fu1 H1 (product-1, software-1)\: native \`host prepare\` verb. \`--apply\` is mandatory for mutation; without it the command refuses with \`--apply-or-dry-run-required\` exit 78' \
'destroy:W3fu1 H1\: native \`host destroy\` verb. Same mandatory-flag contract as \`prepare\`' \
'doctor:W3fu1 H1\: native \`host doctor\` verb. \`--read-only\` is mandatory' \
'install:W15 (software-1, product-1)\: native \`host install\` routes \`--apply\` through the daemon → broker \`RunHostInstall\` path' \
'reconcile:P3 ph3-p3-net-route-degraded-mode\: SOLE mutating recovery verb after the daemon-side net-route preflight has engaged operator-only mode. Re-runs the broker-side net slice of \`host prepare\` (nftables host scope + per-env routes + per-env ipv6 sysctls) and clears the persistent consecutive-failure counter on success' \
'validate:P5 ph5-p5-host-validate-verb\: composite preflight that inventories per-wave Layer-2 validators and (with \`--apply\`) writes the canonical W18 evidence records consumed by \`nixos-modules/options-daemon.nix\:validationEvidencePresent\`' \
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
'check:' \
'prepare:W3fu1 H1 (product-1, software-1)\: native \`host prepare\` verb. \`--apply\` is mandatory for mutation; without it the command refuses with \`--apply-or-dry-run-required\` exit 78' \
'destroy:W3fu1 H1\: native \`host destroy\` verb. Same mandatory-flag contract as \`prepare\`' \
'doctor:W3fu1 H1\: native \`host doctor\` verb. \`--read-only\` is mandatory' \
'install:W15 (software-1, product-1)\: native \`host install\` routes \`--apply\` through the daemon → broker \`RunHostInstall\` path' \
'reconcile:P3 ph3-p3-net-route-degraded-mode\: SOLE mutating recovery verb after the daemon-side net-route preflight has engaged operator-only mode. Re-runs the broker-side net slice of \`host prepare\` (nftables host scope + per-env routes + per-env ipv6 sysctls) and clears the persistent consecutive-failure counter on success' \
'validate:P5 ph5-p5-host-validate-verb\: composite preflight that inventories per-wave Layer-2 validators and (with \`--apply\`) writes the canonical W18 evidence records consumed by \`nixos-modules/options-daemon.nix\:validationEvidencePresent\`' \
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
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling vm commands' commands "$@"
}
(( $+functions[_nixling__subcmd__vm__subcmd__help_commands] )) ||
_nixling__subcmd__vm__subcmd__help_commands() {
    local commands; commands=(
'start:Start the per-VM DAG (virtiofsd → CH → readiness probes)' \
'stop:Stop the per-VM DAG in reverse topo order' \
'restart:Stop then start; same envelope contract as start' \
'list:Daemon-side runtime view (different from \`nixling list\`, which is the static manifest view)' \
'help:Print this message or the help of the given subcommand(s)' \
    )
    _describe -t commands 'nixling vm help commands' commands "$@"
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
