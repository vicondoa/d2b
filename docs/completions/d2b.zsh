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
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
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
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--execution-ref=[]:EXECUTION_REF:_default' \
'--domain=[]:DOMAIN:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--page-token=[]:PAGE_TOKEN:_default' \
'--limit=[]:LIMIT:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--updates[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_type:_default' \
&& ret=0
;;
(watch)
_arguments "${_arguments_options[@]}" : \
'--since-revision=[]:SINCE_REVISION:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_type:_default' \
&& ret=0
;;
(create)
_arguments "${_arguments_options[@]}" : \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_type:_default' \
&& ret=0
;;
(update-spec)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(upgrade)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--recursive[]' \
'--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(host)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
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
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--execution-ref=[]:EXECUTION_REF:_default' \
'--domain=[]:DOMAIN:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--page-token=[]:PAGE_TOKEN:_default' \
'--limit=[]:LIMIT:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--updates[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(check)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--read-only[]' \
'--strict[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(prepare)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(destroy)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(doctor)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--read-only[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(install)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply --enable --start --no-start)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--dry-run)--enable[]' \
'(--dry-run --no-start)--start[]' \
'(--dry-run --start)--no-start[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--network[]' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(validate)
_arguments "${_arguments_options[@]}" : \
'--wave=[]:WAVE:_default' \
'--evidence-dir=[]:EVIDENCE_DIR:_files' \
'--scripts-dir=[]:SCRIPTS_DIR:_files' \
'--operator-signature=[]:OPERATOR_SIGNATURE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
        esac
    ;;
esac
;;
(guest)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__guest_commands" \
"*::: :->guest" \
&& ret=0

    case $state in
    (guest)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-guest-command-$line[1]:"
        case $line[1] in
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--execution-ref=[]:EXECUTION_REF:_default' \
'--domain=[]:DOMAIN:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--page-token=[]:PAGE_TOKEN:_default' \
'--limit=[]:LIMIT:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--updates[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(start)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'--no-wait-ready[]' \
'-f[]' \
'--force[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'--no-wait-ready[]' \
'-f[]' \
'--force[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(restart)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'--no-wait-ready[]' \
'-f[]' \
'--force[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(create)
_arguments "${_arguments_options[@]}" : \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(update-spec)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(console)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
        esac
    ;;
esac
;;
(process)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__process_commands" \
"*::: :->process" \
&& ret=0

    case $state in
    (process)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-process-command-$line[1]:"
        case $line[1] in
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--execution-ref=[]:EXECUTION_REF:_default' \
'--domain=[]:DOMAIN:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--page-token=[]:PAGE_TOKEN:_default' \
'--limit=[]:LIMIT:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--updates[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(start)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'--no-wait-ready[]' \
'-f[]' \
'--force[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(stop)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'--no-wait-ready[]' \
'-f[]' \
'--force[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(create)
_arguments "${_arguments_options[@]}" : \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(update-spec)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
        esac
    ;;
esac
;;
(exec)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__exec_commands" \
"*::: :->exec" \
&& ret=0

    case $state in
    (exec)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-exec-command-$line[1]:"
        case $line[1] in
            (run)
_arguments "${_arguments_options[@]}" : \
'--name=[]:NAME:_default' \
'--domain=[]:DOMAIN:_default' \
'--user=[]:USER_REF:_default' \
'--provider=[]:PROVIDER:_default' \
'*--env=[]:KEY=VALUE:_default' \
'--cwd=[]:CWD:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':execution_ref:_default' \
'*::command:_default' \
&& ret=0
;;
(attach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'-i[]' \
'--interactive[]' \
'-t[]' \
'--tty[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(wait)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--phase=[]:PHASE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
'::execution_ref:_default' \
&& ret=0
;;
(logs)
_arguments "${_arguments_options[@]}" : \
'--stdout-offset=[]:STDOUT_OFFSET:_default' \
'--stderr-offset=[]:STDERR_OFFSET:_default' \
'--max-len=[]:MAX_LEN:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(kill)
_arguments "${_arguments_options[@]}" : \
'--signal=[]:SIGNAL:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
        esac
    ;;
esac
;;
(shell)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__shell_commands" \
"*::: :->shell" \
&& ret=0

    case $state in
    (shell)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-shell-command-$line[1]:"
        case $line[1] in
            (open)
_arguments "${_arguments_options[@]}" : \
'--name=[]:NAME:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--force[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':execution_ref:_default' \
&& ret=0
;;
(attach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--force[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
'::execution_ref:_default' \
&& ret=0
;;
(detach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(kill)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
        esac
    ;;
esac
;;
(volume)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__volume_commands" \
"*::: :->volume" \
&& ret=0

    case $state in
    (volume)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-volume-command-$line[1]:"
        case $line[1] in
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--execution-ref=[]:EXECUTION_REF:_default' \
'--domain=[]:DOMAIN:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--page-token=[]:PAGE_TOKEN:_default' \
'--limit=[]:LIMIT:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--updates[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(watch)
_arguments "${_arguments_options[@]}" : \
'--since-revision=[]:SINCE_REVISION:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(create)
_arguments "${_arguments_options[@]}" : \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(update-spec)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(upgrade)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--recursive[]' \
'--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(verify)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--repair[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(usb)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__volume__subcmd__usb_commands" \
"*::: :->usb" \
&& ret=0

    case $state in
    (usb)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-volume-usb-command-$line[1]:"
        case $line[1] in
            (attach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
':busid:_default' \
&& ret=0
;;
(detach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
':busid:_default' \
&& ret=0
;;
(probe)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
        esac
    ;;
esac
;;
(security-key)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__volume__subcmd__security-key_commands" \
"*::: :->security-key" \
&& ret=0

    case $state in
    (security-key)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-volume-security-key-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(sessions)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(cancel)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'()--current[]' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
'::session_id:_default' \
&& ret=0
;;
(test)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--dry-run[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
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
(network)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__network_commands" \
"*::: :->network" \
&& ret=0

    case $state in
    (network)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-network-command-$line[1]:"
        case $line[1] in
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--execution-ref=[]:EXECUTION_REF:_default' \
'--domain=[]:DOMAIN:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--page-token=[]:PAGE_TOKEN:_default' \
'--limit=[]:LIMIT:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--updates[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(watch)
_arguments "${_arguments_options[@]}" : \
'--since-revision=[]:SINCE_REVISION:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(create)
_arguments "${_arguments_options[@]}" : \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(update-spec)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(upgrade)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--recursive[]' \
'--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(verify)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--repair[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(usb)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__network__subcmd__usb_commands" \
"*::: :->usb" \
&& ret=0

    case $state in
    (usb)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-network-usb-command-$line[1]:"
        case $line[1] in
            (attach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
':busid:_default' \
&& ret=0
;;
(detach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
':busid:_default' \
&& ret=0
;;
(probe)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
        esac
    ;;
esac
;;
(security-key)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__network__subcmd__security-key_commands" \
"*::: :->security-key" \
&& ret=0

    case $state in
    (security-key)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-network-security-key-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(sessions)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(cancel)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'()--current[]' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
'::session_id:_default' \
&& ret=0
;;
(test)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--dry-run[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
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
(device)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__device_commands" \
"*::: :->device" \
&& ret=0

    case $state in
    (device)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-device-command-$line[1]:"
        case $line[1] in
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--execution-ref=[]:EXECUTION_REF:_default' \
'--domain=[]:DOMAIN:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--page-token=[]:PAGE_TOKEN:_default' \
'--limit=[]:LIMIT:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--updates[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(watch)
_arguments "${_arguments_options[@]}" : \
'--since-revision=[]:SINCE_REVISION:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(create)
_arguments "${_arguments_options[@]}" : \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(update-spec)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(upgrade)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--recursive[]' \
'--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(verify)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--repair[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(usb)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__device__subcmd__usb_commands" \
"*::: :->usb" \
&& ret=0

    case $state in
    (usb)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-device-usb-command-$line[1]:"
        case $line[1] in
            (attach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
':busid:_default' \
&& ret=0
;;
(detach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
':busid:_default' \
&& ret=0
;;
(probe)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
        esac
    ;;
esac
;;
(security-key)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__device__subcmd__security-key_commands" \
"*::: :->security-key" \
&& ret=0

    case $state in
    (security-key)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-device-security-key-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(sessions)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(cancel)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'()--current[]' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
'::session_id:_default' \
&& ret=0
;;
(test)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--dry-run[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
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
(endpoint)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__endpoint_commands" \
"*::: :->endpoint" \
&& ret=0

    case $state in
    (endpoint)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-endpoint-command-$line[1]:"
        case $line[1] in
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--endpoint-class=[]:ENDPOINT_CLASS:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--updates[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(watch)
_arguments "${_arguments_options[@]}" : \
'--endpoint-class=[]:ENDPOINT_CLASS:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(resolve)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
        esac
    ;;
esac
;;
(export)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__export_commands" \
"*::: :->export" \
&& ret=0

    case $state in
    (export)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-export-command-$line[1]:"
        case $line[1] in
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--exported-type=[]:EXPORTED_TYPE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(watch)
_arguments "${_arguments_options[@]}" : \
'--exported-type=[]:EXPORTED_TYPE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(create)
_arguments "${_arguments_options[@]}" : \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(update-spec)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
        esac
    ;;
esac
;;
(import)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__import_commands" \
"*::: :->import" \
&& ret=0

    case $state in
    (import)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-import-command-$line[1]:"
        case $line[1] in
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--expected-type=[]:EXPECTED_TYPE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(watch)
_arguments "${_arguments_options[@]}" : \
'--expected-type=[]:EXPECTED_TYPE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(projection)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(graph)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(create)
_arguments "${_arguments_options[@]}" : \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(update-spec)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
        esac
    ;;
esac
;;
(resource)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__resource_commands" \
"*::: :->resource" \
&& ret=0

    case $state in
    (resource)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-resource-command-$line[1]:"
        case $line[1] in
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--execution-ref=[]:EXECUTION_REF:_default' \
'--domain=[]:DOMAIN:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--page-token=[]:PAGE_TOKEN:_default' \
'--limit=[]:LIMIT:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--updates[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_type:_default' \
&& ret=0
;;
(watch)
_arguments "${_arguments_options[@]}" : \
'--since-revision=[]:SINCE_REVISION:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_type:_default' \
&& ret=0
;;
(create)
_arguments "${_arguments_options[@]}" : \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_type:_default' \
&& ret=0
;;
(update-spec)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(upgrade)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--recursive[]' \
'--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(authorities)
_arguments "${_arguments_options[@]}" : \
'--scope=[]:SCOPE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__resource__subcmd__authorities_commands" \
"*::: :->authorities" \
&& ret=0

    case $state in
    (authorities)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-resource-authorities-command-$line[1]:"
        case $line[1] in
            (holders)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
&& ret=0
;;
(conflict)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':resource_ref:_default' \
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
(user)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__user_commands" \
"*::: :->user" \
&& ret=0

    case $state in
    (user)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-user-command-$line[1]:"
        case $line[1] in
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--execution-ref=[]:EXECUTION_REF:_default' \
'--domain=[]:DOMAIN:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--page-token=[]:PAGE_TOKEN:_default' \
'--limit=[]:LIMIT:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--updates[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(watch)
_arguments "${_arguments_options[@]}" : \
'--since-revision=[]:SINCE_REVISION:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(create)
_arguments "${_arguments_options[@]}" : \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(update-spec)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(upgrade)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--recursive[]' \
'--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(verify)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--repair[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(usb)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__user__subcmd__usb_commands" \
"*::: :->usb" \
&& ret=0

    case $state in
    (usb)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-user-usb-command-$line[1]:"
        case $line[1] in
            (attach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
':busid:_default' \
&& ret=0
;;
(detach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
':busid:_default' \
&& ret=0
;;
(probe)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
        esac
    ;;
esac
;;
(security-key)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__user__subcmd__security-key_commands" \
"*::: :->security-key" \
&& ret=0

    case $state in
    (security-key)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-user-security-key-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(sessions)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(cancel)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'()--current[]' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
'::session_id:_default' \
&& ret=0
;;
(test)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--dry-run[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
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
(credential)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__credential_commands" \
"*::: :->credential" \
&& ret=0

    case $state in
    (credential)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-credential-command-$line[1]:"
        case $line[1] in
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--execution-ref=[]:EXECUTION_REF:_default' \
'--domain=[]:DOMAIN:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--page-token=[]:PAGE_TOKEN:_default' \
'--limit=[]:LIMIT:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--updates[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(watch)
_arguments "${_arguments_options[@]}" : \
'--since-revision=[]:SINCE_REVISION:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(create)
_arguments "${_arguments_options[@]}" : \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(update-spec)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(upgrade)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--recursive[]' \
'--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(verify)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--repair[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(usb)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__credential__subcmd__usb_commands" \
"*::: :->usb" \
&& ret=0

    case $state in
    (usb)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-credential-usb-command-$line[1]:"
        case $line[1] in
            (attach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
':busid:_default' \
&& ret=0
;;
(detach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
':busid:_default' \
&& ret=0
;;
(probe)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
        esac
    ;;
esac
;;
(security-key)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__credential__subcmd__security-key_commands" \
"*::: :->security-key" \
&& ret=0

    case $state in
    (security-key)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-credential-security-key-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(sessions)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(cancel)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'()--current[]' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
'::session_id:_default' \
&& ret=0
;;
(test)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--dry-run[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
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
(provider)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__provider_commands" \
"*::: :->provider" \
&& ret=0

    case $state in
    (provider)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-provider-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--package-only[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(inspect)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
        esac
    ;;
esac
;;
(zone)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__zone_commands" \
"*::: :->zone" \
&& ret=0

    case $state in
    (zone)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-zone-command-$line[1]:"
        case $line[1] in
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
'::name:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
'::name:_default' \
&& ret=0
;;
        esac
    ;;
esac
;;
(quota)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__quota_commands" \
"*::: :->quota" \
&& ret=0

    case $state in
    (quota)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-quota-command-$line[1]:"
        case $line[1] in
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--execution-ref=[]:EXECUTION_REF:_default' \
'--domain=[]:DOMAIN:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--page-token=[]:PAGE_TOKEN:_default' \
'--limit=[]:LIMIT:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--updates[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(watch)
_arguments "${_arguments_options[@]}" : \
'--since-revision=[]:SINCE_REVISION:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(create)
_arguments "${_arguments_options[@]}" : \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(update-spec)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(upgrade)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--recursive[]' \
'--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(verify)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--repair[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(usb)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__quota__subcmd__usb_commands" \
"*::: :->usb" \
&& ret=0

    case $state in
    (usb)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-quota-usb-command-$line[1]:"
        case $line[1] in
            (attach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
':busid:_default' \
&& ret=0
;;
(detach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
':busid:_default' \
&& ret=0
;;
(probe)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
        esac
    ;;
esac
;;
(security-key)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__quota__subcmd__security-key_commands" \
"*::: :->security-key" \
&& ret=0

    case $state in
    (security-key)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-quota-security-key-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(sessions)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(cancel)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'()--current[]' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
'::session_id:_default' \
&& ret=0
;;
(test)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--dry-run[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
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
(emergency-policy)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__emergency-policy_commands" \
"*::: :->emergency-policy" \
&& ret=0

    case $state in
    (emergency-policy)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-emergency-policy-command-$line[1]:"
        case $line[1] in
            (get)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(list)
_arguments "${_arguments_options[@]}" : \
'--execution-ref=[]:EXECUTION_REF:_default' \
'--domain=[]:DOMAIN:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--page-token=[]:PAGE_TOKEN:_default' \
'--limit=[]:LIMIT:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--updates[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(watch)
_arguments "${_arguments_options[@]}" : \
'--since-revision=[]:SINCE_REVISION:_default' \
'--phase=[]:PHASE:_default' \
'--label-selector=[]:LABEL_SELECTOR:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(create)
_arguments "${_arguments_options[@]}" : \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(update-spec)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'(--spec-stdin)--spec-file=[]:SPEC_FILE:_files' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--spec-file)--spec-stdin[]' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(delete)
_arguments "${_arguments_options[@]}" : \
'--revision=[]:REVISION:_default' \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--wait-for-reconcile[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(upgrade)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--recursive[]' \
'--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(reconcile)
_arguments "${_arguments_options[@]}" : \
'--reconcile-deadline=[]:RECONCILE_DEADLINE:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(verify)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--repair[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(usb)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__emergency-policy__subcmd__usb_commands" \
"*::: :->usb" \
&& ret=0

    case $state in
    (usb)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-emergency-policy-usb-command-$line[1]:"
        case $line[1] in
            (attach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
':busid:_default' \
&& ret=0
;;
(detach)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
':busid:_default' \
&& ret=0
;;
(probe)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
        esac
    ;;
esac
;;
(security-key)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__emergency-policy__subcmd__security-key_commands" \
"*::: :->security-key" \
&& ret=0

    case $state in
    (security-key)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-emergency-policy-security-key-command-$line[1]:"
        case $line[1] in
            (status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(sessions)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(cancel)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'()--current[]' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
'::session_id:_default' \
&& ret=0
;;
(test)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--dry-run[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
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
(activation)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__activation_commands" \
"*::: :->activation" \
&& ret=0

    case $state in
    (activation)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-activation-command-$line[1]:"
        case $line[1] in
            (apply)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--dry-run[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(build)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':guest_ref:_default' \
&& ret=0
;;
(generations)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':guest_ref:_default' \
&& ret=0
;;
(switch)
_arguments "${_arguments_options[@]}" : \
'--to-generation=[]:TO_GENERATION:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':guest_ref:_default' \
&& ret=0
;;
(boot)
_arguments "${_arguments_options[@]}" : \
'--to-generation=[]:TO_GENERATION:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':guest_ref:_default' \
&& ret=0
;;
(test)
_arguments "${_arguments_options[@]}" : \
'--to-generation=[]:TO_GENERATION:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':guest_ref:_default' \
&& ret=0
;;
(rollback)
_arguments "${_arguments_options[@]}" : \
'--to-generation=[]:TO_GENERATION:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':guest_ref:_default' \
&& ret=0
;;
(gc)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(migrate)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(keys)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__activation__subcmd__keys_commands" \
"*::: :->keys" \
&& ret=0

    case $state in
    (keys)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-activation-keys-command-$line[1]:"
        case $line[1] in
            (list)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(show)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(rotate)
_arguments "${_arguments_options[@]}" : \
'--to-generation=[]:TO_GENERATION:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--apply)--dry-run[]' \
'(--dry-run)--apply[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':guest_ref:_default' \
&& ret=0
;;
        esac
    ;;
esac
;;
(trust)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(rotate-known-host)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':name:_default' \
&& ret=0
;;
(config)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
":: :_d2b__subcmd__activation__subcmd__config_commands" \
"*::: :->config" \
&& ret=0

    case $state in
    (config)
        words=($line[1] "${words[@]}")
        (( CURRENT += 1 ))
        curcontext="${curcontext%:*:*}:d2b-activation-config-command-$line[1]:"
        case $line[1] in
            (sync)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--dry-run[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':guest_ref:_default' \
&& ret=0
;;
(diff)
_arguments "${_arguments_options[@]}" : \
'--against=[]:AGAINST:_files' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':guest_ref:_default' \
&& ret=0
;;
(approve)
_arguments "${_arguments_options[@]}" : \
'--to=[]:DESTINATION:_files' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':guest_ref:_default' \
&& ret=0
;;
(reject)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--dry-run[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':guest_ref:_default' \
&& ret=0
;;
(status)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--dry-run[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':guest_ref:_default' \
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
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--strict[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
(op)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
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
'--operation-id=[]:OPERATION_ID:_default' \
'--trace-id=[]:TRACE_ID:_default' \
'--span-id=[]:SPAN_ID:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--watch[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
        esac
    ;;
esac
;;
(auth)
_arguments "${_arguments_options[@]}" : \
'--test-uid=[Test-only identity override retained as a hidden fixture seam]:TEST_UID:_default' \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
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
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
&& ret=0
;;
        esac
    ;;
esac
;;
(complete)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'--list-commands[]' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
'::shell:(bash zsh fish)' \
&& ret=0
;;
(audio)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':verb:_default' \
'*::args:_default' \
&& ret=0
;;
(clipboard)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':verb:_default' \
'*::args:_default' \
&& ret=0
;;
(display)
_arguments "${_arguments_options[@]}" : \
'--zone=[Address a declared Zone. Without this flag the nearest local runtime is selected]:ZONE:_default' \
'--deadline=[Bound all Zone requests and streams]:DURATION:_default' \
'(--human)--json[Emit the stable JSON envelope]' \
'(--json)--human[Force human-readable terminal output]' \
'(--deadline)--no-deadline[Suppress the command default deadline]' \
'-h[Print help]' \
'--help[Print help]' \
':verb:_default' \
'*::args:_default' \
&& ret=0
;;
        esac
    ;;
esac
}

(( $+functions[_d2b_commands] )) ||
_d2b_commands() {
    local commands; commands=(
'get:' \
'list:' \
'watch:' \
'create:' \
'update-spec:' \
'delete:' \
'status:' \
'upgrade:' \
'reconcile:' \
'host:' \
'guest:' \
'process:' \
'exec:' \
'shell:' \
'volume:Typed noun commands reuse the generic resource verbs while documenting the noun'\''s default ResourceType in clap help' \
'network:Typed noun commands reuse the generic resource verbs while documenting the noun'\''s default ResourceType in clap help' \
'device:Typed noun commands reuse the generic resource verbs while documenting the noun'\''s default ResourceType in clap help' \
'endpoint:' \
'export:' \
'import:' \
'resource:The \`d2b resource\` namespace also carries the generic authority read projection. It never creates or mutates an authority' \
'user:Typed noun commands reuse the generic resource verbs while documenting the noun'\''s default ResourceType in clap help' \
'credential:Typed noun commands reuse the generic resource verbs while documenting the noun'\''s default ResourceType in clap help' \
'provider:' \
'zone:' \
'quota:Typed noun commands reuse the generic resource verbs while documenting the noun'\''s default ResourceType in clap help' \
'emergency-policy:Typed noun commands reuse the generic resource verbs while documenting the noun'\''s default ResourceType in clap help' \
'activation:' \
'audit:' \
'op:' \
'auth:' \
'complete:' \
'audio:' \
'clipboard:' \
'display:' \
    )
    _describe -t commands 'd2b commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation_commands] )) ||
_d2b__subcmd__activation_commands() {
    local commands; commands=(
'apply:' \
'build:' \
'generations:' \
'switch:' \
'boot:' \
'test:' \
'rollback:' \
'gc:' \
'migrate:' \
'keys:' \
'trust:' \
'rotate-known-host:' \
'config:' \
    )
    _describe -t commands 'd2b activation commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__apply_commands] )) ||
_d2b__subcmd__activation__subcmd__apply_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation apply commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__boot_commands] )) ||
_d2b__subcmd__activation__subcmd__boot_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation boot commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__build_commands] )) ||
_d2b__subcmd__activation__subcmd__build_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation build commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__config_commands] )) ||
_d2b__subcmd__activation__subcmd__config_commands() {
    local commands; commands=(
'sync:' \
'diff:' \
'approve:' \
'reject:' \
'status:' \
    )
    _describe -t commands 'd2b activation config commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__config__subcmd__approve_commands] )) ||
_d2b__subcmd__activation__subcmd__config__subcmd__approve_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation config approve commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__config__subcmd__diff_commands] )) ||
_d2b__subcmd__activation__subcmd__config__subcmd__diff_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation config diff commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__config__subcmd__reject_commands] )) ||
_d2b__subcmd__activation__subcmd__config__subcmd__reject_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation config reject commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__config__subcmd__status_commands] )) ||
_d2b__subcmd__activation__subcmd__config__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation config status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__config__subcmd__sync_commands] )) ||
_d2b__subcmd__activation__subcmd__config__subcmd__sync_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation config sync commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__gc_commands] )) ||
_d2b__subcmd__activation__subcmd__gc_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation gc commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__generations_commands] )) ||
_d2b__subcmd__activation__subcmd__generations_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation generations commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__keys_commands] )) ||
_d2b__subcmd__activation__subcmd__keys_commands() {
    local commands; commands=(
'list:' \
'show:' \
'rotate:' \
    )
    _describe -t commands 'd2b activation keys commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__keys__subcmd__list_commands] )) ||
_d2b__subcmd__activation__subcmd__keys__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation keys list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__keys__subcmd__rotate_commands] )) ||
_d2b__subcmd__activation__subcmd__keys__subcmd__rotate_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation keys rotate commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__keys__subcmd__show_commands] )) ||
_d2b__subcmd__activation__subcmd__keys__subcmd__show_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation keys show commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__migrate_commands] )) ||
_d2b__subcmd__activation__subcmd__migrate_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation migrate commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__rollback_commands] )) ||
_d2b__subcmd__activation__subcmd__rollback_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation rollback commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__rotate-known-host_commands] )) ||
_d2b__subcmd__activation__subcmd__rotate-known-host_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation rotate-known-host commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__switch_commands] )) ||
_d2b__subcmd__activation__subcmd__switch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation switch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__test_commands] )) ||
_d2b__subcmd__activation__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation test commands' commands "$@"
}
(( $+functions[_d2b__subcmd__activation__subcmd__trust_commands] )) ||
_d2b__subcmd__activation__subcmd__trust_commands() {
    local commands; commands=()
    _describe -t commands 'd2b activation trust commands' commands "$@"
}
(( $+functions[_d2b__subcmd__audio_commands] )) ||
_d2b__subcmd__audio_commands() {
    local commands; commands=()
    _describe -t commands 'd2b audio commands' commands "$@"
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
    )
    _describe -t commands 'd2b auth commands' commands "$@"
}
(( $+functions[_d2b__subcmd__auth__subcmd__status_commands] )) ||
_d2b__subcmd__auth__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b auth status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__clipboard_commands] )) ||
_d2b__subcmd__clipboard_commands() {
    local commands; commands=()
    _describe -t commands 'd2b clipboard commands' commands "$@"
}
(( $+functions[_d2b__subcmd__complete_commands] )) ||
_d2b__subcmd__complete_commands() {
    local commands; commands=()
    _describe -t commands 'd2b complete commands' commands "$@"
}
(( $+functions[_d2b__subcmd__create_commands] )) ||
_d2b__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'd2b create commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential_commands] )) ||
_d2b__subcmd__credential_commands() {
    local commands; commands=(
'get:' \
'list:' \
'watch:' \
'create:' \
'update-spec:' \
'delete:' \
'status:' \
'upgrade:' \
'reconcile:' \
'verify:' \
'usb:' \
'security-key:' \
    )
    _describe -t commands 'd2b credential commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__create_commands] )) ||
_d2b__subcmd__credential__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential create commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__delete_commands] )) ||
_d2b__subcmd__credential__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential delete commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__get_commands] )) ||
_d2b__subcmd__credential__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__list_commands] )) ||
_d2b__subcmd__credential__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__reconcile_commands] )) ||
_d2b__subcmd__credential__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential reconcile commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__security-key_commands] )) ||
_d2b__subcmd__credential__subcmd__security-key_commands() {
    local commands; commands=(
'status:' \
'sessions:' \
'cancel:' \
'test:' \
    )
    _describe -t commands 'd2b credential security-key commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__security-key__subcmd__cancel_commands] )) ||
_d2b__subcmd__credential__subcmd__security-key__subcmd__cancel_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential security-key cancel commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__security-key__subcmd__sessions_commands] )) ||
_d2b__subcmd__credential__subcmd__security-key__subcmd__sessions_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential security-key sessions commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__security-key__subcmd__status_commands] )) ||
_d2b__subcmd__credential__subcmd__security-key__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential security-key status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__security-key__subcmd__test_commands] )) ||
_d2b__subcmd__credential__subcmd__security-key__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential security-key test commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__status_commands] )) ||
_d2b__subcmd__credential__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__update-spec_commands] )) ||
_d2b__subcmd__credential__subcmd__update-spec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential update-spec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__upgrade_commands] )) ||
_d2b__subcmd__credential__subcmd__upgrade_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential upgrade commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__usb_commands] )) ||
_d2b__subcmd__credential__subcmd__usb_commands() {
    local commands; commands=(
'attach:' \
'detach:' \
'probe:' \
    )
    _describe -t commands 'd2b credential usb commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__usb__subcmd__attach_commands] )) ||
_d2b__subcmd__credential__subcmd__usb__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential usb attach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__usb__subcmd__detach_commands] )) ||
_d2b__subcmd__credential__subcmd__usb__subcmd__detach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential usb detach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__usb__subcmd__probe_commands] )) ||
_d2b__subcmd__credential__subcmd__usb__subcmd__probe_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential usb probe commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__verify_commands] )) ||
_d2b__subcmd__credential__subcmd__verify_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential verify commands' commands "$@"
}
(( $+functions[_d2b__subcmd__credential__subcmd__watch_commands] )) ||
_d2b__subcmd__credential__subcmd__watch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b credential watch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__delete_commands] )) ||
_d2b__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'd2b delete commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device_commands] )) ||
_d2b__subcmd__device_commands() {
    local commands; commands=(
'get:' \
'list:' \
'watch:' \
'create:' \
'update-spec:' \
'delete:' \
'status:' \
'upgrade:' \
'reconcile:' \
'verify:' \
'usb:' \
'security-key:' \
    )
    _describe -t commands 'd2b device commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__create_commands] )) ||
_d2b__subcmd__device__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device create commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__delete_commands] )) ||
_d2b__subcmd__device__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device delete commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__get_commands] )) ||
_d2b__subcmd__device__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__list_commands] )) ||
_d2b__subcmd__device__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__reconcile_commands] )) ||
_d2b__subcmd__device__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device reconcile commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__security-key_commands] )) ||
_d2b__subcmd__device__subcmd__security-key_commands() {
    local commands; commands=(
'status:' \
'sessions:' \
'cancel:' \
'test:' \
    )
    _describe -t commands 'd2b device security-key commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__security-key__subcmd__cancel_commands] )) ||
_d2b__subcmd__device__subcmd__security-key__subcmd__cancel_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device security-key cancel commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__security-key__subcmd__sessions_commands] )) ||
_d2b__subcmd__device__subcmd__security-key__subcmd__sessions_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device security-key sessions commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__security-key__subcmd__status_commands] )) ||
_d2b__subcmd__device__subcmd__security-key__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device security-key status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__security-key__subcmd__test_commands] )) ||
_d2b__subcmd__device__subcmd__security-key__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device security-key test commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__status_commands] )) ||
_d2b__subcmd__device__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__update-spec_commands] )) ||
_d2b__subcmd__device__subcmd__update-spec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device update-spec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__upgrade_commands] )) ||
_d2b__subcmd__device__subcmd__upgrade_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device upgrade commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__usb_commands] )) ||
_d2b__subcmd__device__subcmd__usb_commands() {
    local commands; commands=(
'attach:' \
'detach:' \
'probe:' \
    )
    _describe -t commands 'd2b device usb commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__usb__subcmd__attach_commands] )) ||
_d2b__subcmd__device__subcmd__usb__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device usb attach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__usb__subcmd__detach_commands] )) ||
_d2b__subcmd__device__subcmd__usb__subcmd__detach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device usb detach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__usb__subcmd__probe_commands] )) ||
_d2b__subcmd__device__subcmd__usb__subcmd__probe_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device usb probe commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__verify_commands] )) ||
_d2b__subcmd__device__subcmd__verify_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device verify commands' commands "$@"
}
(( $+functions[_d2b__subcmd__device__subcmd__watch_commands] )) ||
_d2b__subcmd__device__subcmd__watch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b device watch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__display_commands] )) ||
_d2b__subcmd__display_commands() {
    local commands; commands=()
    _describe -t commands 'd2b display commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy_commands] )) ||
_d2b__subcmd__emergency-policy_commands() {
    local commands; commands=(
'get:' \
'list:' \
'watch:' \
'create:' \
'update-spec:' \
'delete:' \
'status:' \
'upgrade:' \
'reconcile:' \
'verify:' \
'usb:' \
'security-key:' \
    )
    _describe -t commands 'd2b emergency-policy commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__create_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy create commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__delete_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy delete commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__get_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__list_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__reconcile_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy reconcile commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__security-key_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__security-key_commands() {
    local commands; commands=(
'status:' \
'sessions:' \
'cancel:' \
'test:' \
    )
    _describe -t commands 'd2b emergency-policy security-key commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__security-key__subcmd__cancel_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__security-key__subcmd__cancel_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy security-key cancel commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__security-key__subcmd__sessions_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__security-key__subcmd__sessions_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy security-key sessions commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__security-key__subcmd__status_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__security-key__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy security-key status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__security-key__subcmd__test_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__security-key__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy security-key test commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__status_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__update-spec_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__update-spec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy update-spec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__upgrade_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__upgrade_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy upgrade commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__usb_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__usb_commands() {
    local commands; commands=(
'attach:' \
'detach:' \
'probe:' \
    )
    _describe -t commands 'd2b emergency-policy usb commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__usb__subcmd__attach_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__usb__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy usb attach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__usb__subcmd__detach_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__usb__subcmd__detach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy usb detach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__usb__subcmd__probe_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__usb__subcmd__probe_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy usb probe commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__verify_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__verify_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy verify commands' commands "$@"
}
(( $+functions[_d2b__subcmd__emergency-policy__subcmd__watch_commands] )) ||
_d2b__subcmd__emergency-policy__subcmd__watch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b emergency-policy watch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__endpoint_commands] )) ||
_d2b__subcmd__endpoint_commands() {
    local commands; commands=(
'get:' \
'list:' \
'watch:' \
'status:' \
'resolve:' \
    )
    _describe -t commands 'd2b endpoint commands' commands "$@"
}
(( $+functions[_d2b__subcmd__endpoint__subcmd__get_commands] )) ||
_d2b__subcmd__endpoint__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b endpoint get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__endpoint__subcmd__list_commands] )) ||
_d2b__subcmd__endpoint__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b endpoint list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__endpoint__subcmd__resolve_commands] )) ||
_d2b__subcmd__endpoint__subcmd__resolve_commands() {
    local commands; commands=()
    _describe -t commands 'd2b endpoint resolve commands' commands "$@"
}
(( $+functions[_d2b__subcmd__endpoint__subcmd__status_commands] )) ||
_d2b__subcmd__endpoint__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b endpoint status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__endpoint__subcmd__watch_commands] )) ||
_d2b__subcmd__endpoint__subcmd__watch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b endpoint watch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__exec_commands] )) ||
_d2b__subcmd__exec_commands() {
    local commands; commands=(
'run:' \
'attach:' \
'wait:' \
'status:' \
'list:' \
'logs:' \
'kill:' \
    )
    _describe -t commands 'd2b exec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__exec__subcmd__attach_commands] )) ||
_d2b__subcmd__exec__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b exec attach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__exec__subcmd__kill_commands] )) ||
_d2b__subcmd__exec__subcmd__kill_commands() {
    local commands; commands=()
    _describe -t commands 'd2b exec kill commands' commands "$@"
}
(( $+functions[_d2b__subcmd__exec__subcmd__list_commands] )) ||
_d2b__subcmd__exec__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b exec list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__exec__subcmd__logs_commands] )) ||
_d2b__subcmd__exec__subcmd__logs_commands() {
    local commands; commands=()
    _describe -t commands 'd2b exec logs commands' commands "$@"
}
(( $+functions[_d2b__subcmd__exec__subcmd__run_commands] )) ||
_d2b__subcmd__exec__subcmd__run_commands() {
    local commands; commands=()
    _describe -t commands 'd2b exec run commands' commands "$@"
}
(( $+functions[_d2b__subcmd__exec__subcmd__status_commands] )) ||
_d2b__subcmd__exec__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b exec status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__exec__subcmd__wait_commands] )) ||
_d2b__subcmd__exec__subcmd__wait_commands() {
    local commands; commands=()
    _describe -t commands 'd2b exec wait commands' commands "$@"
}
(( $+functions[_d2b__subcmd__export_commands] )) ||
_d2b__subcmd__export_commands() {
    local commands; commands=(
'get:' \
'list:' \
'watch:' \
'status:' \
'create:' \
'update-spec:' \
'delete:' \
    )
    _describe -t commands 'd2b export commands' commands "$@"
}
(( $+functions[_d2b__subcmd__export__subcmd__create_commands] )) ||
_d2b__subcmd__export__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'd2b export create commands' commands "$@"
}
(( $+functions[_d2b__subcmd__export__subcmd__delete_commands] )) ||
_d2b__subcmd__export__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'd2b export delete commands' commands "$@"
}
(( $+functions[_d2b__subcmd__export__subcmd__get_commands] )) ||
_d2b__subcmd__export__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b export get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__export__subcmd__list_commands] )) ||
_d2b__subcmd__export__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b export list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__export__subcmd__status_commands] )) ||
_d2b__subcmd__export__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b export status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__export__subcmd__update-spec_commands] )) ||
_d2b__subcmd__export__subcmd__update-spec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b export update-spec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__export__subcmd__watch_commands] )) ||
_d2b__subcmd__export__subcmd__watch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b export watch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__get_commands] )) ||
_d2b__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__guest_commands] )) ||
_d2b__subcmd__guest_commands() {
    local commands; commands=(
'get:' \
'list:' \
'status:' \
'start:' \
'stop:' \
'restart:' \
'create:' \
'update-spec:' \
'delete:' \
'console:' \
    )
    _describe -t commands 'd2b guest commands' commands "$@"
}
(( $+functions[_d2b__subcmd__guest__subcmd__console_commands] )) ||
_d2b__subcmd__guest__subcmd__console_commands() {
    local commands; commands=()
    _describe -t commands 'd2b guest console commands' commands "$@"
}
(( $+functions[_d2b__subcmd__guest__subcmd__create_commands] )) ||
_d2b__subcmd__guest__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'd2b guest create commands' commands "$@"
}
(( $+functions[_d2b__subcmd__guest__subcmd__delete_commands] )) ||
_d2b__subcmd__guest__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'd2b guest delete commands' commands "$@"
}
(( $+functions[_d2b__subcmd__guest__subcmd__get_commands] )) ||
_d2b__subcmd__guest__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b guest get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__guest__subcmd__list_commands] )) ||
_d2b__subcmd__guest__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b guest list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__guest__subcmd__restart_commands] )) ||
_d2b__subcmd__guest__subcmd__restart_commands() {
    local commands; commands=()
    _describe -t commands 'd2b guest restart commands' commands "$@"
}
(( $+functions[_d2b__subcmd__guest__subcmd__start_commands] )) ||
_d2b__subcmd__guest__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'd2b guest start commands' commands "$@"
}
(( $+functions[_d2b__subcmd__guest__subcmd__status_commands] )) ||
_d2b__subcmd__guest__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b guest status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__guest__subcmd__stop_commands] )) ||
_d2b__subcmd__guest__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'd2b guest stop commands' commands "$@"
}
(( $+functions[_d2b__subcmd__guest__subcmd__update-spec_commands] )) ||
_d2b__subcmd__guest__subcmd__update-spec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b guest update-spec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host_commands] )) ||
_d2b__subcmd__host_commands() {
    local commands; commands=(
'get:' \
'list:' \
'status:' \
'check:' \
'prepare:' \
'destroy:' \
'doctor:' \
'install:' \
'reconcile:' \
'validate:' \
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
(( $+functions[_d2b__subcmd__host__subcmd__get_commands] )) ||
_d2b__subcmd__host__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__install_commands] )) ||
_d2b__subcmd__host__subcmd__install_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host install commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__list_commands] )) ||
_d2b__subcmd__host__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host list commands' commands "$@"
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
(( $+functions[_d2b__subcmd__host__subcmd__status_commands] )) ||
_d2b__subcmd__host__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__host__subcmd__validate_commands] )) ||
_d2b__subcmd__host__subcmd__validate_commands() {
    local commands; commands=()
    _describe -t commands 'd2b host validate commands' commands "$@"
}
(( $+functions[_d2b__subcmd__import_commands] )) ||
_d2b__subcmd__import_commands() {
    local commands; commands=(
'get:' \
'list:' \
'watch:' \
'status:' \
'projection:' \
'graph:' \
'create:' \
'update-spec:' \
'delete:' \
    )
    _describe -t commands 'd2b import commands' commands "$@"
}
(( $+functions[_d2b__subcmd__import__subcmd__create_commands] )) ||
_d2b__subcmd__import__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'd2b import create commands' commands "$@"
}
(( $+functions[_d2b__subcmd__import__subcmd__delete_commands] )) ||
_d2b__subcmd__import__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'd2b import delete commands' commands "$@"
}
(( $+functions[_d2b__subcmd__import__subcmd__get_commands] )) ||
_d2b__subcmd__import__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b import get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__import__subcmd__graph_commands] )) ||
_d2b__subcmd__import__subcmd__graph_commands() {
    local commands; commands=()
    _describe -t commands 'd2b import graph commands' commands "$@"
}
(( $+functions[_d2b__subcmd__import__subcmd__list_commands] )) ||
_d2b__subcmd__import__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b import list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__import__subcmd__projection_commands] )) ||
_d2b__subcmd__import__subcmd__projection_commands() {
    local commands; commands=()
    _describe -t commands 'd2b import projection commands' commands "$@"
}
(( $+functions[_d2b__subcmd__import__subcmd__status_commands] )) ||
_d2b__subcmd__import__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b import status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__import__subcmd__update-spec_commands] )) ||
_d2b__subcmd__import__subcmd__update-spec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b import update-spec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__import__subcmd__watch_commands] )) ||
_d2b__subcmd__import__subcmd__watch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b import watch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__list_commands] )) ||
_d2b__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network_commands] )) ||
_d2b__subcmd__network_commands() {
    local commands; commands=(
'get:' \
'list:' \
'watch:' \
'create:' \
'update-spec:' \
'delete:' \
'status:' \
'upgrade:' \
'reconcile:' \
'verify:' \
'usb:' \
'security-key:' \
    )
    _describe -t commands 'd2b network commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__create_commands] )) ||
_d2b__subcmd__network__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network create commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__delete_commands] )) ||
_d2b__subcmd__network__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network delete commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__get_commands] )) ||
_d2b__subcmd__network__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__list_commands] )) ||
_d2b__subcmd__network__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__reconcile_commands] )) ||
_d2b__subcmd__network__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network reconcile commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__security-key_commands] )) ||
_d2b__subcmd__network__subcmd__security-key_commands() {
    local commands; commands=(
'status:' \
'sessions:' \
'cancel:' \
'test:' \
    )
    _describe -t commands 'd2b network security-key commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__security-key__subcmd__cancel_commands] )) ||
_d2b__subcmd__network__subcmd__security-key__subcmd__cancel_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network security-key cancel commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__security-key__subcmd__sessions_commands] )) ||
_d2b__subcmd__network__subcmd__security-key__subcmd__sessions_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network security-key sessions commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__security-key__subcmd__status_commands] )) ||
_d2b__subcmd__network__subcmd__security-key__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network security-key status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__security-key__subcmd__test_commands] )) ||
_d2b__subcmd__network__subcmd__security-key__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network security-key test commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__status_commands] )) ||
_d2b__subcmd__network__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__update-spec_commands] )) ||
_d2b__subcmd__network__subcmd__update-spec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network update-spec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__upgrade_commands] )) ||
_d2b__subcmd__network__subcmd__upgrade_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network upgrade commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__usb_commands] )) ||
_d2b__subcmd__network__subcmd__usb_commands() {
    local commands; commands=(
'attach:' \
'detach:' \
'probe:' \
    )
    _describe -t commands 'd2b network usb commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__usb__subcmd__attach_commands] )) ||
_d2b__subcmd__network__subcmd__usb__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network usb attach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__usb__subcmd__detach_commands] )) ||
_d2b__subcmd__network__subcmd__usb__subcmd__detach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network usb detach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__usb__subcmd__probe_commands] )) ||
_d2b__subcmd__network__subcmd__usb__subcmd__probe_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network usb probe commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__verify_commands] )) ||
_d2b__subcmd__network__subcmd__verify_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network verify commands' commands "$@"
}
(( $+functions[_d2b__subcmd__network__subcmd__watch_commands] )) ||
_d2b__subcmd__network__subcmd__watch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b network watch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__op_commands] )) ||
_d2b__subcmd__op_commands() {
    local commands; commands=(
'inspect:' \
    )
    _describe -t commands 'd2b op commands' commands "$@"
}
(( $+functions[_d2b__subcmd__op__subcmd__inspect_commands] )) ||
_d2b__subcmd__op__subcmd__inspect_commands() {
    local commands; commands=()
    _describe -t commands 'd2b op inspect commands' commands "$@"
}
(( $+functions[_d2b__subcmd__process_commands] )) ||
_d2b__subcmd__process_commands() {
    local commands; commands=(
'get:' \
'list:' \
'status:' \
'start:' \
'stop:' \
'create:' \
'update-spec:' \
'delete:' \
    )
    _describe -t commands 'd2b process commands' commands "$@"
}
(( $+functions[_d2b__subcmd__process__subcmd__create_commands] )) ||
_d2b__subcmd__process__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'd2b process create commands' commands "$@"
}
(( $+functions[_d2b__subcmd__process__subcmd__delete_commands] )) ||
_d2b__subcmd__process__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'd2b process delete commands' commands "$@"
}
(( $+functions[_d2b__subcmd__process__subcmd__get_commands] )) ||
_d2b__subcmd__process__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b process get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__process__subcmd__list_commands] )) ||
_d2b__subcmd__process__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b process list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__process__subcmd__start_commands] )) ||
_d2b__subcmd__process__subcmd__start_commands() {
    local commands; commands=()
    _describe -t commands 'd2b process start commands' commands "$@"
}
(( $+functions[_d2b__subcmd__process__subcmd__status_commands] )) ||
_d2b__subcmd__process__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b process status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__process__subcmd__stop_commands] )) ||
_d2b__subcmd__process__subcmd__stop_commands() {
    local commands; commands=()
    _describe -t commands 'd2b process stop commands' commands "$@"
}
(( $+functions[_d2b__subcmd__process__subcmd__update-spec_commands] )) ||
_d2b__subcmd__process__subcmd__update-spec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b process update-spec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__provider_commands] )) ||
_d2b__subcmd__provider_commands() {
    local commands; commands=(
'list:' \
'get:' \
'status:' \
'inspect:' \
    )
    _describe -t commands 'd2b provider commands' commands "$@"
}
(( $+functions[_d2b__subcmd__provider__subcmd__get_commands] )) ||
_d2b__subcmd__provider__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b provider get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__provider__subcmd__inspect_commands] )) ||
_d2b__subcmd__provider__subcmd__inspect_commands() {
    local commands; commands=()
    _describe -t commands 'd2b provider inspect commands' commands "$@"
}
(( $+functions[_d2b__subcmd__provider__subcmd__list_commands] )) ||
_d2b__subcmd__provider__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b provider list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__provider__subcmd__status_commands] )) ||
_d2b__subcmd__provider__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b provider status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota_commands] )) ||
_d2b__subcmd__quota_commands() {
    local commands; commands=(
'get:' \
'list:' \
'watch:' \
'create:' \
'update-spec:' \
'delete:' \
'status:' \
'upgrade:' \
'reconcile:' \
'verify:' \
'usb:' \
'security-key:' \
    )
    _describe -t commands 'd2b quota commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__create_commands] )) ||
_d2b__subcmd__quota__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota create commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__delete_commands] )) ||
_d2b__subcmd__quota__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota delete commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__get_commands] )) ||
_d2b__subcmd__quota__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__list_commands] )) ||
_d2b__subcmd__quota__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__reconcile_commands] )) ||
_d2b__subcmd__quota__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota reconcile commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__security-key_commands] )) ||
_d2b__subcmd__quota__subcmd__security-key_commands() {
    local commands; commands=(
'status:' \
'sessions:' \
'cancel:' \
'test:' \
    )
    _describe -t commands 'd2b quota security-key commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__security-key__subcmd__cancel_commands] )) ||
_d2b__subcmd__quota__subcmd__security-key__subcmd__cancel_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota security-key cancel commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__security-key__subcmd__sessions_commands] )) ||
_d2b__subcmd__quota__subcmd__security-key__subcmd__sessions_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota security-key sessions commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__security-key__subcmd__status_commands] )) ||
_d2b__subcmd__quota__subcmd__security-key__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota security-key status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__security-key__subcmd__test_commands] )) ||
_d2b__subcmd__quota__subcmd__security-key__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota security-key test commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__status_commands] )) ||
_d2b__subcmd__quota__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__update-spec_commands] )) ||
_d2b__subcmd__quota__subcmd__update-spec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota update-spec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__upgrade_commands] )) ||
_d2b__subcmd__quota__subcmd__upgrade_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota upgrade commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__usb_commands] )) ||
_d2b__subcmd__quota__subcmd__usb_commands() {
    local commands; commands=(
'attach:' \
'detach:' \
'probe:' \
    )
    _describe -t commands 'd2b quota usb commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__usb__subcmd__attach_commands] )) ||
_d2b__subcmd__quota__subcmd__usb__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota usb attach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__usb__subcmd__detach_commands] )) ||
_d2b__subcmd__quota__subcmd__usb__subcmd__detach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota usb detach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__usb__subcmd__probe_commands] )) ||
_d2b__subcmd__quota__subcmd__usb__subcmd__probe_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota usb probe commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__verify_commands] )) ||
_d2b__subcmd__quota__subcmd__verify_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota verify commands' commands "$@"
}
(( $+functions[_d2b__subcmd__quota__subcmd__watch_commands] )) ||
_d2b__subcmd__quota__subcmd__watch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b quota watch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__reconcile_commands] )) ||
_d2b__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'd2b reconcile commands' commands "$@"
}
(( $+functions[_d2b__subcmd__resource_commands] )) ||
_d2b__subcmd__resource_commands() {
    local commands; commands=(
'get:' \
'list:' \
'watch:' \
'create:' \
'update-spec:' \
'delete:' \
'status:' \
'upgrade:' \
'reconcile:' \
'authorities:' \
    )
    _describe -t commands 'd2b resource commands' commands "$@"
}
(( $+functions[_d2b__subcmd__resource__subcmd__authorities_commands] )) ||
_d2b__subcmd__resource__subcmd__authorities_commands() {
    local commands; commands=(
'holders:' \
'conflict:' \
    )
    _describe -t commands 'd2b resource authorities commands' commands "$@"
}
(( $+functions[_d2b__subcmd__resource__subcmd__authorities__subcmd__conflict_commands] )) ||
_d2b__subcmd__resource__subcmd__authorities__subcmd__conflict_commands() {
    local commands; commands=()
    _describe -t commands 'd2b resource authorities conflict commands' commands "$@"
}
(( $+functions[_d2b__subcmd__resource__subcmd__authorities__subcmd__holders_commands] )) ||
_d2b__subcmd__resource__subcmd__authorities__subcmd__holders_commands() {
    local commands; commands=()
    _describe -t commands 'd2b resource authorities holders commands' commands "$@"
}
(( $+functions[_d2b__subcmd__resource__subcmd__create_commands] )) ||
_d2b__subcmd__resource__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'd2b resource create commands' commands "$@"
}
(( $+functions[_d2b__subcmd__resource__subcmd__delete_commands] )) ||
_d2b__subcmd__resource__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'd2b resource delete commands' commands "$@"
}
(( $+functions[_d2b__subcmd__resource__subcmd__get_commands] )) ||
_d2b__subcmd__resource__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b resource get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__resource__subcmd__list_commands] )) ||
_d2b__subcmd__resource__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b resource list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__resource__subcmd__reconcile_commands] )) ||
_d2b__subcmd__resource__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'd2b resource reconcile commands' commands "$@"
}
(( $+functions[_d2b__subcmd__resource__subcmd__status_commands] )) ||
_d2b__subcmd__resource__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b resource status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__resource__subcmd__update-spec_commands] )) ||
_d2b__subcmd__resource__subcmd__update-spec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b resource update-spec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__resource__subcmd__upgrade_commands] )) ||
_d2b__subcmd__resource__subcmd__upgrade_commands() {
    local commands; commands=()
    _describe -t commands 'd2b resource upgrade commands' commands "$@"
}
(( $+functions[_d2b__subcmd__resource__subcmd__watch_commands] )) ||
_d2b__subcmd__resource__subcmd__watch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b resource watch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__shell_commands] )) ||
_d2b__subcmd__shell_commands() {
    local commands; commands=(
'open:' \
'attach:' \
'list:' \
'detach:' \
'kill:' \
'status:' \
    )
    _describe -t commands 'd2b shell commands' commands "$@"
}
(( $+functions[_d2b__subcmd__shell__subcmd__attach_commands] )) ||
_d2b__subcmd__shell__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b shell attach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__shell__subcmd__detach_commands] )) ||
_d2b__subcmd__shell__subcmd__detach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b shell detach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__shell__subcmd__kill_commands] )) ||
_d2b__subcmd__shell__subcmd__kill_commands() {
    local commands; commands=()
    _describe -t commands 'd2b shell kill commands' commands "$@"
}
(( $+functions[_d2b__subcmd__shell__subcmd__list_commands] )) ||
_d2b__subcmd__shell__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b shell list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__shell__subcmd__open_commands] )) ||
_d2b__subcmd__shell__subcmd__open_commands() {
    local commands; commands=()
    _describe -t commands 'd2b shell open commands' commands "$@"
}
(( $+functions[_d2b__subcmd__shell__subcmd__status_commands] )) ||
_d2b__subcmd__shell__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b shell status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__status_commands] )) ||
_d2b__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__update-spec_commands] )) ||
_d2b__subcmd__update-spec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b update-spec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__upgrade_commands] )) ||
_d2b__subcmd__upgrade_commands() {
    local commands; commands=()
    _describe -t commands 'd2b upgrade commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user_commands] )) ||
_d2b__subcmd__user_commands() {
    local commands; commands=(
'get:' \
'list:' \
'watch:' \
'create:' \
'update-spec:' \
'delete:' \
'status:' \
'upgrade:' \
'reconcile:' \
'verify:' \
'usb:' \
'security-key:' \
    )
    _describe -t commands 'd2b user commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__create_commands] )) ||
_d2b__subcmd__user__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user create commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__delete_commands] )) ||
_d2b__subcmd__user__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user delete commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__get_commands] )) ||
_d2b__subcmd__user__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__list_commands] )) ||
_d2b__subcmd__user__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__reconcile_commands] )) ||
_d2b__subcmd__user__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user reconcile commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__security-key_commands] )) ||
_d2b__subcmd__user__subcmd__security-key_commands() {
    local commands; commands=(
'status:' \
'sessions:' \
'cancel:' \
'test:' \
    )
    _describe -t commands 'd2b user security-key commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__security-key__subcmd__cancel_commands] )) ||
_d2b__subcmd__user__subcmd__security-key__subcmd__cancel_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user security-key cancel commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__security-key__subcmd__sessions_commands] )) ||
_d2b__subcmd__user__subcmd__security-key__subcmd__sessions_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user security-key sessions commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__security-key__subcmd__status_commands] )) ||
_d2b__subcmd__user__subcmd__security-key__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user security-key status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__security-key__subcmd__test_commands] )) ||
_d2b__subcmd__user__subcmd__security-key__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user security-key test commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__status_commands] )) ||
_d2b__subcmd__user__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__update-spec_commands] )) ||
_d2b__subcmd__user__subcmd__update-spec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user update-spec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__upgrade_commands] )) ||
_d2b__subcmd__user__subcmd__upgrade_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user upgrade commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__usb_commands] )) ||
_d2b__subcmd__user__subcmd__usb_commands() {
    local commands; commands=(
'attach:' \
'detach:' \
'probe:' \
    )
    _describe -t commands 'd2b user usb commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__usb__subcmd__attach_commands] )) ||
_d2b__subcmd__user__subcmd__usb__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user usb attach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__usb__subcmd__detach_commands] )) ||
_d2b__subcmd__user__subcmd__usb__subcmd__detach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user usb detach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__usb__subcmd__probe_commands] )) ||
_d2b__subcmd__user__subcmd__usb__subcmd__probe_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user usb probe commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__verify_commands] )) ||
_d2b__subcmd__user__subcmd__verify_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user verify commands' commands "$@"
}
(( $+functions[_d2b__subcmd__user__subcmd__watch_commands] )) ||
_d2b__subcmd__user__subcmd__watch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b user watch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume_commands] )) ||
_d2b__subcmd__volume_commands() {
    local commands; commands=(
'get:' \
'list:' \
'watch:' \
'create:' \
'update-spec:' \
'delete:' \
'status:' \
'upgrade:' \
'reconcile:' \
'verify:' \
'usb:' \
'security-key:' \
    )
    _describe -t commands 'd2b volume commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__create_commands] )) ||
_d2b__subcmd__volume__subcmd__create_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume create commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__delete_commands] )) ||
_d2b__subcmd__volume__subcmd__delete_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume delete commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__get_commands] )) ||
_d2b__subcmd__volume__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__list_commands] )) ||
_d2b__subcmd__volume__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__reconcile_commands] )) ||
_d2b__subcmd__volume__subcmd__reconcile_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume reconcile commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__security-key_commands] )) ||
_d2b__subcmd__volume__subcmd__security-key_commands() {
    local commands; commands=(
'status:' \
'sessions:' \
'cancel:' \
'test:' \
    )
    _describe -t commands 'd2b volume security-key commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__security-key__subcmd__cancel_commands] )) ||
_d2b__subcmd__volume__subcmd__security-key__subcmd__cancel_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume security-key cancel commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__security-key__subcmd__sessions_commands] )) ||
_d2b__subcmd__volume__subcmd__security-key__subcmd__sessions_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume security-key sessions commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__security-key__subcmd__status_commands] )) ||
_d2b__subcmd__volume__subcmd__security-key__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume security-key status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__security-key__subcmd__test_commands] )) ||
_d2b__subcmd__volume__subcmd__security-key__subcmd__test_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume security-key test commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__status_commands] )) ||
_d2b__subcmd__volume__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume status commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__update-spec_commands] )) ||
_d2b__subcmd__volume__subcmd__update-spec_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume update-spec commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__upgrade_commands] )) ||
_d2b__subcmd__volume__subcmd__upgrade_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume upgrade commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__usb_commands] )) ||
_d2b__subcmd__volume__subcmd__usb_commands() {
    local commands; commands=(
'attach:' \
'detach:' \
'probe:' \
    )
    _describe -t commands 'd2b volume usb commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__usb__subcmd__attach_commands] )) ||
_d2b__subcmd__volume__subcmd__usb__subcmd__attach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume usb attach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__usb__subcmd__detach_commands] )) ||
_d2b__subcmd__volume__subcmd__usb__subcmd__detach_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume usb detach commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__usb__subcmd__probe_commands] )) ||
_d2b__subcmd__volume__subcmd__usb__subcmd__probe_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume usb probe commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__verify_commands] )) ||
_d2b__subcmd__volume__subcmd__verify_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume verify commands' commands "$@"
}
(( $+functions[_d2b__subcmd__volume__subcmd__watch_commands] )) ||
_d2b__subcmd__volume__subcmd__watch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b volume watch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__watch_commands] )) ||
_d2b__subcmd__watch_commands() {
    local commands; commands=()
    _describe -t commands 'd2b watch commands' commands "$@"
}
(( $+functions[_d2b__subcmd__zone_commands] )) ||
_d2b__subcmd__zone_commands() {
    local commands; commands=(
'get:' \
'list:' \
'status:' \
    )
    _describe -t commands 'd2b zone commands' commands "$@"
}
(( $+functions[_d2b__subcmd__zone__subcmd__get_commands] )) ||
_d2b__subcmd__zone__subcmd__get_commands() {
    local commands; commands=()
    _describe -t commands 'd2b zone get commands' commands "$@"
}
(( $+functions[_d2b__subcmd__zone__subcmd__list_commands] )) ||
_d2b__subcmd__zone__subcmd__list_commands() {
    local commands; commands=()
    _describe -t commands 'd2b zone list commands' commands "$@"
}
(( $+functions[_d2b__subcmd__zone__subcmd__status_commands] )) ||
_d2b__subcmd__zone__subcmd__status_commands() {
    local commands; commands=()
    _describe -t commands 'd2b zone status commands' commands "$@"
}

if [ "$funcstack[1]" = "_d2b" ]; then
    _d2b "$@"
else
    compdef _d2b d2b
fi
