_d2b() {
    local i cur prev opts cmd
    COMPREPLY=()
    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
        cur="$2"
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
    fi
    prev="$3"
    cmd=""
    opts=""

    for i in "${COMP_WORDS[@]:0:COMP_CWORD}"
    do
        case "${cmd},${i}" in
            ",$1")
                cmd="d2b"
                ;;
            d2b,activation)
                cmd="d2b__subcmd__activation"
                ;;
            d2b,audio)
                cmd="d2b__subcmd__audio"
                ;;
            d2b,audit)
                cmd="d2b__subcmd__audit"
                ;;
            d2b,auth)
                cmd="d2b__subcmd__auth"
                ;;
            d2b,clipboard)
                cmd="d2b__subcmd__clipboard"
                ;;
            d2b,complete)
                cmd="d2b__subcmd__complete"
                ;;
            d2b,create)
                cmd="d2b__subcmd__create"
                ;;
            d2b,credential)
                cmd="d2b__subcmd__credential"
                ;;
            d2b,delete)
                cmd="d2b__subcmd__delete"
                ;;
            d2b,device)
                cmd="d2b__subcmd__device"
                ;;
            d2b,display)
                cmd="d2b__subcmd__display"
                ;;
            d2b,emergency-policy)
                cmd="d2b__subcmd__emergency__subcmd__policy"
                ;;
            d2b,endpoint)
                cmd="d2b__subcmd__endpoint"
                ;;
            d2b,exec)
                cmd="d2b__subcmd__exec"
                ;;
            d2b,export)
                cmd="d2b__subcmd__export"
                ;;
            d2b,get)
                cmd="d2b__subcmd__get"
                ;;
            d2b,guest)
                cmd="d2b__subcmd__guest"
                ;;
            d2b,host)
                cmd="d2b__subcmd__host"
                ;;
            d2b,import)
                cmd="d2b__subcmd__import"
                ;;
            d2b,list)
                cmd="d2b__subcmd__list"
                ;;
            d2b,network)
                cmd="d2b__subcmd__network"
                ;;
            d2b,op)
                cmd="d2b__subcmd__op"
                ;;
            d2b,process)
                cmd="d2b__subcmd__process"
                ;;
            d2b,provider)
                cmd="d2b__subcmd__provider"
                ;;
            d2b,quota)
                cmd="d2b__subcmd__quota"
                ;;
            d2b,reconcile)
                cmd="d2b__subcmd__reconcile"
                ;;
            d2b,resource)
                cmd="d2b__subcmd__resource"
                ;;
            d2b,shell)
                cmd="d2b__subcmd__shell"
                ;;
            d2b,status)
                cmd="d2b__subcmd__status"
                ;;
            d2b,update-spec)
                cmd="d2b__subcmd__update__subcmd__spec"
                ;;
            d2b,upgrade)
                cmd="d2b__subcmd__upgrade"
                ;;
            d2b,user)
                cmd="d2b__subcmd__user"
                ;;
            d2b,volume)
                cmd="d2b__subcmd__volume"
                ;;
            d2b,watch)
                cmd="d2b__subcmd__watch"
                ;;
            d2b,zone)
                cmd="d2b__subcmd__zone"
                ;;
            d2b__subcmd__activation,apply)
                cmd="d2b__subcmd__activation__subcmd__apply"
                ;;
            d2b__subcmd__activation,boot)
                cmd="d2b__subcmd__activation__subcmd__boot"
                ;;
            d2b__subcmd__activation,build)
                cmd="d2b__subcmd__activation__subcmd__build"
                ;;
            d2b__subcmd__activation,config)
                cmd="d2b__subcmd__activation__subcmd__config"
                ;;
            d2b__subcmd__activation,gc)
                cmd="d2b__subcmd__activation__subcmd__gc"
                ;;
            d2b__subcmd__activation,generations)
                cmd="d2b__subcmd__activation__subcmd__generations"
                ;;
            d2b__subcmd__activation,keys)
                cmd="d2b__subcmd__activation__subcmd__keys"
                ;;
            d2b__subcmd__activation,migrate)
                cmd="d2b__subcmd__activation__subcmd__migrate"
                ;;
            d2b__subcmd__activation,rollback)
                cmd="d2b__subcmd__activation__subcmd__rollback"
                ;;
            d2b__subcmd__activation,rotate-known-host)
                cmd="d2b__subcmd__activation__subcmd__rotate__subcmd__known__subcmd__host"
                ;;
            d2b__subcmd__activation,switch)
                cmd="d2b__subcmd__activation__subcmd__switch"
                ;;
            d2b__subcmd__activation,test)
                cmd="d2b__subcmd__activation__subcmd__test"
                ;;
            d2b__subcmd__activation,trust)
                cmd="d2b__subcmd__activation__subcmd__trust"
                ;;
            d2b__subcmd__activation__subcmd__config,approve)
                cmd="d2b__subcmd__activation__subcmd__config__subcmd__approve"
                ;;
            d2b__subcmd__activation__subcmd__config,diff)
                cmd="d2b__subcmd__activation__subcmd__config__subcmd__diff"
                ;;
            d2b__subcmd__activation__subcmd__config,reject)
                cmd="d2b__subcmd__activation__subcmd__config__subcmd__reject"
                ;;
            d2b__subcmd__activation__subcmd__config,status)
                cmd="d2b__subcmd__activation__subcmd__config__subcmd__status"
                ;;
            d2b__subcmd__activation__subcmd__config,sync)
                cmd="d2b__subcmd__activation__subcmd__config__subcmd__sync"
                ;;
            d2b__subcmd__activation__subcmd__keys,list)
                cmd="d2b__subcmd__activation__subcmd__keys__subcmd__list"
                ;;
            d2b__subcmd__activation__subcmd__keys,rotate)
                cmd="d2b__subcmd__activation__subcmd__keys__subcmd__rotate"
                ;;
            d2b__subcmd__activation__subcmd__keys,show)
                cmd="d2b__subcmd__activation__subcmd__keys__subcmd__show"
                ;;
            d2b__subcmd__auth,status)
                cmd="d2b__subcmd__auth__subcmd__status"
                ;;
            d2b__subcmd__credential,create)
                cmd="d2b__subcmd__credential__subcmd__create"
                ;;
            d2b__subcmd__credential,delete)
                cmd="d2b__subcmd__credential__subcmd__delete"
                ;;
            d2b__subcmd__credential,get)
                cmd="d2b__subcmd__credential__subcmd__get"
                ;;
            d2b__subcmd__credential,list)
                cmd="d2b__subcmd__credential__subcmd__list"
                ;;
            d2b__subcmd__credential,reconcile)
                cmd="d2b__subcmd__credential__subcmd__reconcile"
                ;;
            d2b__subcmd__credential,security-key)
                cmd="d2b__subcmd__credential__subcmd__security__subcmd__key"
                ;;
            d2b__subcmd__credential,status)
                cmd="d2b__subcmd__credential__subcmd__status"
                ;;
            d2b__subcmd__credential,update-spec)
                cmd="d2b__subcmd__credential__subcmd__update__subcmd__spec"
                ;;
            d2b__subcmd__credential,upgrade)
                cmd="d2b__subcmd__credential__subcmd__upgrade"
                ;;
            d2b__subcmd__credential,usb)
                cmd="d2b__subcmd__credential__subcmd__usb"
                ;;
            d2b__subcmd__credential,verify)
                cmd="d2b__subcmd__credential__subcmd__verify"
                ;;
            d2b__subcmd__credential,watch)
                cmd="d2b__subcmd__credential__subcmd__watch"
                ;;
            d2b__subcmd__credential__subcmd__security__subcmd__key,cancel)
                cmd="d2b__subcmd__credential__subcmd__security__subcmd__key__subcmd__cancel"
                ;;
            d2b__subcmd__credential__subcmd__security__subcmd__key,sessions)
                cmd="d2b__subcmd__credential__subcmd__security__subcmd__key__subcmd__sessions"
                ;;
            d2b__subcmd__credential__subcmd__security__subcmd__key,status)
                cmd="d2b__subcmd__credential__subcmd__security__subcmd__key__subcmd__status"
                ;;
            d2b__subcmd__credential__subcmd__security__subcmd__key,test)
                cmd="d2b__subcmd__credential__subcmd__security__subcmd__key__subcmd__test"
                ;;
            d2b__subcmd__credential__subcmd__usb,attach)
                cmd="d2b__subcmd__credential__subcmd__usb__subcmd__attach"
                ;;
            d2b__subcmd__credential__subcmd__usb,detach)
                cmd="d2b__subcmd__credential__subcmd__usb__subcmd__detach"
                ;;
            d2b__subcmd__credential__subcmd__usb,probe)
                cmd="d2b__subcmd__credential__subcmd__usb__subcmd__probe"
                ;;
            d2b__subcmd__device,create)
                cmd="d2b__subcmd__device__subcmd__create"
                ;;
            d2b__subcmd__device,delete)
                cmd="d2b__subcmd__device__subcmd__delete"
                ;;
            d2b__subcmd__device,get)
                cmd="d2b__subcmd__device__subcmd__get"
                ;;
            d2b__subcmd__device,list)
                cmd="d2b__subcmd__device__subcmd__list"
                ;;
            d2b__subcmd__device,reconcile)
                cmd="d2b__subcmd__device__subcmd__reconcile"
                ;;
            d2b__subcmd__device,security-key)
                cmd="d2b__subcmd__device__subcmd__security__subcmd__key"
                ;;
            d2b__subcmd__device,status)
                cmd="d2b__subcmd__device__subcmd__status"
                ;;
            d2b__subcmd__device,update-spec)
                cmd="d2b__subcmd__device__subcmd__update__subcmd__spec"
                ;;
            d2b__subcmd__device,upgrade)
                cmd="d2b__subcmd__device__subcmd__upgrade"
                ;;
            d2b__subcmd__device,usb)
                cmd="d2b__subcmd__device__subcmd__usb"
                ;;
            d2b__subcmd__device,verify)
                cmd="d2b__subcmd__device__subcmd__verify"
                ;;
            d2b__subcmd__device,watch)
                cmd="d2b__subcmd__device__subcmd__watch"
                ;;
            d2b__subcmd__device__subcmd__security__subcmd__key,cancel)
                cmd="d2b__subcmd__device__subcmd__security__subcmd__key__subcmd__cancel"
                ;;
            d2b__subcmd__device__subcmd__security__subcmd__key,sessions)
                cmd="d2b__subcmd__device__subcmd__security__subcmd__key__subcmd__sessions"
                ;;
            d2b__subcmd__device__subcmd__security__subcmd__key,status)
                cmd="d2b__subcmd__device__subcmd__security__subcmd__key__subcmd__status"
                ;;
            d2b__subcmd__device__subcmd__security__subcmd__key,test)
                cmd="d2b__subcmd__device__subcmd__security__subcmd__key__subcmd__test"
                ;;
            d2b__subcmd__device__subcmd__usb,attach)
                cmd="d2b__subcmd__device__subcmd__usb__subcmd__attach"
                ;;
            d2b__subcmd__device__subcmd__usb,detach)
                cmd="d2b__subcmd__device__subcmd__usb__subcmd__detach"
                ;;
            d2b__subcmd__device__subcmd__usb,probe)
                cmd="d2b__subcmd__device__subcmd__usb__subcmd__probe"
                ;;
            d2b__subcmd__emergency__subcmd__policy,create)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__create"
                ;;
            d2b__subcmd__emergency__subcmd__policy,delete)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__delete"
                ;;
            d2b__subcmd__emergency__subcmd__policy,get)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__get"
                ;;
            d2b__subcmd__emergency__subcmd__policy,list)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__list"
                ;;
            d2b__subcmd__emergency__subcmd__policy,reconcile)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__reconcile"
                ;;
            d2b__subcmd__emergency__subcmd__policy,security-key)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__security__subcmd__key"
                ;;
            d2b__subcmd__emergency__subcmd__policy,status)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__status"
                ;;
            d2b__subcmd__emergency__subcmd__policy,update-spec)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__update__subcmd__spec"
                ;;
            d2b__subcmd__emergency__subcmd__policy,upgrade)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__upgrade"
                ;;
            d2b__subcmd__emergency__subcmd__policy,usb)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__usb"
                ;;
            d2b__subcmd__emergency__subcmd__policy,verify)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__verify"
                ;;
            d2b__subcmd__emergency__subcmd__policy,watch)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__watch"
                ;;
            d2b__subcmd__emergency__subcmd__policy__subcmd__security__subcmd__key,cancel)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__security__subcmd__key__subcmd__cancel"
                ;;
            d2b__subcmd__emergency__subcmd__policy__subcmd__security__subcmd__key,sessions)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__security__subcmd__key__subcmd__sessions"
                ;;
            d2b__subcmd__emergency__subcmd__policy__subcmd__security__subcmd__key,status)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__security__subcmd__key__subcmd__status"
                ;;
            d2b__subcmd__emergency__subcmd__policy__subcmd__security__subcmd__key,test)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__security__subcmd__key__subcmd__test"
                ;;
            d2b__subcmd__emergency__subcmd__policy__subcmd__usb,attach)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__usb__subcmd__attach"
                ;;
            d2b__subcmd__emergency__subcmd__policy__subcmd__usb,detach)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__usb__subcmd__detach"
                ;;
            d2b__subcmd__emergency__subcmd__policy__subcmd__usb,probe)
                cmd="d2b__subcmd__emergency__subcmd__policy__subcmd__usb__subcmd__probe"
                ;;
            d2b__subcmd__endpoint,get)
                cmd="d2b__subcmd__endpoint__subcmd__get"
                ;;
            d2b__subcmd__endpoint,list)
                cmd="d2b__subcmd__endpoint__subcmd__list"
                ;;
            d2b__subcmd__endpoint,resolve)
                cmd="d2b__subcmd__endpoint__subcmd__resolve"
                ;;
            d2b__subcmd__endpoint,status)
                cmd="d2b__subcmd__endpoint__subcmd__status"
                ;;
            d2b__subcmd__endpoint,watch)
                cmd="d2b__subcmd__endpoint__subcmd__watch"
                ;;
            d2b__subcmd__exec,attach)
                cmd="d2b__subcmd__exec__subcmd__attach"
                ;;
            d2b__subcmd__exec,kill)
                cmd="d2b__subcmd__exec__subcmd__kill"
                ;;
            d2b__subcmd__exec,list)
                cmd="d2b__subcmd__exec__subcmd__list"
                ;;
            d2b__subcmd__exec,logs)
                cmd="d2b__subcmd__exec__subcmd__logs"
                ;;
            d2b__subcmd__exec,run)
                cmd="d2b__subcmd__exec__subcmd__run"
                ;;
            d2b__subcmd__exec,status)
                cmd="d2b__subcmd__exec__subcmd__status"
                ;;
            d2b__subcmd__exec,wait)
                cmd="d2b__subcmd__exec__subcmd__wait"
                ;;
            d2b__subcmd__export,create)
                cmd="d2b__subcmd__export__subcmd__create"
                ;;
            d2b__subcmd__export,delete)
                cmd="d2b__subcmd__export__subcmd__delete"
                ;;
            d2b__subcmd__export,get)
                cmd="d2b__subcmd__export__subcmd__get"
                ;;
            d2b__subcmd__export,list)
                cmd="d2b__subcmd__export__subcmd__list"
                ;;
            d2b__subcmd__export,status)
                cmd="d2b__subcmd__export__subcmd__status"
                ;;
            d2b__subcmd__export,update-spec)
                cmd="d2b__subcmd__export__subcmd__update__subcmd__spec"
                ;;
            d2b__subcmd__export,watch)
                cmd="d2b__subcmd__export__subcmd__watch"
                ;;
            d2b__subcmd__guest,console)
                cmd="d2b__subcmd__guest__subcmd__console"
                ;;
            d2b__subcmd__guest,create)
                cmd="d2b__subcmd__guest__subcmd__create"
                ;;
            d2b__subcmd__guest,delete)
                cmd="d2b__subcmd__guest__subcmd__delete"
                ;;
            d2b__subcmd__guest,get)
                cmd="d2b__subcmd__guest__subcmd__get"
                ;;
            d2b__subcmd__guest,list)
                cmd="d2b__subcmd__guest__subcmd__list"
                ;;
            d2b__subcmd__guest,restart)
                cmd="d2b__subcmd__guest__subcmd__restart"
                ;;
            d2b__subcmd__guest,start)
                cmd="d2b__subcmd__guest__subcmd__start"
                ;;
            d2b__subcmd__guest,status)
                cmd="d2b__subcmd__guest__subcmd__status"
                ;;
            d2b__subcmd__guest,stop)
                cmd="d2b__subcmd__guest__subcmd__stop"
                ;;
            d2b__subcmd__guest,update-spec)
                cmd="d2b__subcmd__guest__subcmd__update__subcmd__spec"
                ;;
            d2b__subcmd__host,check)
                cmd="d2b__subcmd__host__subcmd__check"
                ;;
            d2b__subcmd__host,destroy)
                cmd="d2b__subcmd__host__subcmd__destroy"
                ;;
            d2b__subcmd__host,doctor)
                cmd="d2b__subcmd__host__subcmd__doctor"
                ;;
            d2b__subcmd__host,get)
                cmd="d2b__subcmd__host__subcmd__get"
                ;;
            d2b__subcmd__host,install)
                cmd="d2b__subcmd__host__subcmd__install"
                ;;
            d2b__subcmd__host,list)
                cmd="d2b__subcmd__host__subcmd__list"
                ;;
            d2b__subcmd__host,prepare)
                cmd="d2b__subcmd__host__subcmd__prepare"
                ;;
            d2b__subcmd__host,reconcile)
                cmd="d2b__subcmd__host__subcmd__reconcile"
                ;;
            d2b__subcmd__host,status)
                cmd="d2b__subcmd__host__subcmd__status"
                ;;
            d2b__subcmd__host,validate)
                cmd="d2b__subcmd__host__subcmd__validate"
                ;;
            d2b__subcmd__import,create)
                cmd="d2b__subcmd__import__subcmd__create"
                ;;
            d2b__subcmd__import,delete)
                cmd="d2b__subcmd__import__subcmd__delete"
                ;;
            d2b__subcmd__import,get)
                cmd="d2b__subcmd__import__subcmd__get"
                ;;
            d2b__subcmd__import,graph)
                cmd="d2b__subcmd__import__subcmd__graph"
                ;;
            d2b__subcmd__import,list)
                cmd="d2b__subcmd__import__subcmd__list"
                ;;
            d2b__subcmd__import,projection)
                cmd="d2b__subcmd__import__subcmd__projection"
                ;;
            d2b__subcmd__import,status)
                cmd="d2b__subcmd__import__subcmd__status"
                ;;
            d2b__subcmd__import,update-spec)
                cmd="d2b__subcmd__import__subcmd__update__subcmd__spec"
                ;;
            d2b__subcmd__import,watch)
                cmd="d2b__subcmd__import__subcmd__watch"
                ;;
            d2b__subcmd__network,create)
                cmd="d2b__subcmd__network__subcmd__create"
                ;;
            d2b__subcmd__network,delete)
                cmd="d2b__subcmd__network__subcmd__delete"
                ;;
            d2b__subcmd__network,get)
                cmd="d2b__subcmd__network__subcmd__get"
                ;;
            d2b__subcmd__network,list)
                cmd="d2b__subcmd__network__subcmd__list"
                ;;
            d2b__subcmd__network,reconcile)
                cmd="d2b__subcmd__network__subcmd__reconcile"
                ;;
            d2b__subcmd__network,security-key)
                cmd="d2b__subcmd__network__subcmd__security__subcmd__key"
                ;;
            d2b__subcmd__network,status)
                cmd="d2b__subcmd__network__subcmd__status"
                ;;
            d2b__subcmd__network,update-spec)
                cmd="d2b__subcmd__network__subcmd__update__subcmd__spec"
                ;;
            d2b__subcmd__network,upgrade)
                cmd="d2b__subcmd__network__subcmd__upgrade"
                ;;
            d2b__subcmd__network,usb)
                cmd="d2b__subcmd__network__subcmd__usb"
                ;;
            d2b__subcmd__network,verify)
                cmd="d2b__subcmd__network__subcmd__verify"
                ;;
            d2b__subcmd__network,watch)
                cmd="d2b__subcmd__network__subcmd__watch"
                ;;
            d2b__subcmd__network__subcmd__security__subcmd__key,cancel)
                cmd="d2b__subcmd__network__subcmd__security__subcmd__key__subcmd__cancel"
                ;;
            d2b__subcmd__network__subcmd__security__subcmd__key,sessions)
                cmd="d2b__subcmd__network__subcmd__security__subcmd__key__subcmd__sessions"
                ;;
            d2b__subcmd__network__subcmd__security__subcmd__key,status)
                cmd="d2b__subcmd__network__subcmd__security__subcmd__key__subcmd__status"
                ;;
            d2b__subcmd__network__subcmd__security__subcmd__key,test)
                cmd="d2b__subcmd__network__subcmd__security__subcmd__key__subcmd__test"
                ;;
            d2b__subcmd__network__subcmd__usb,attach)
                cmd="d2b__subcmd__network__subcmd__usb__subcmd__attach"
                ;;
            d2b__subcmd__network__subcmd__usb,detach)
                cmd="d2b__subcmd__network__subcmd__usb__subcmd__detach"
                ;;
            d2b__subcmd__network__subcmd__usb,probe)
                cmd="d2b__subcmd__network__subcmd__usb__subcmd__probe"
                ;;
            d2b__subcmd__op,inspect)
                cmd="d2b__subcmd__op__subcmd__inspect"
                ;;
            d2b__subcmd__process,create)
                cmd="d2b__subcmd__process__subcmd__create"
                ;;
            d2b__subcmd__process,delete)
                cmd="d2b__subcmd__process__subcmd__delete"
                ;;
            d2b__subcmd__process,get)
                cmd="d2b__subcmd__process__subcmd__get"
                ;;
            d2b__subcmd__process,list)
                cmd="d2b__subcmd__process__subcmd__list"
                ;;
            d2b__subcmd__process,start)
                cmd="d2b__subcmd__process__subcmd__start"
                ;;
            d2b__subcmd__process,status)
                cmd="d2b__subcmd__process__subcmd__status"
                ;;
            d2b__subcmd__process,stop)
                cmd="d2b__subcmd__process__subcmd__stop"
                ;;
            d2b__subcmd__process,update-spec)
                cmd="d2b__subcmd__process__subcmd__update__subcmd__spec"
                ;;
            d2b__subcmd__provider,get)
                cmd="d2b__subcmd__provider__subcmd__get"
                ;;
            d2b__subcmd__provider,inspect)
                cmd="d2b__subcmd__provider__subcmd__inspect"
                ;;
            d2b__subcmd__provider,list)
                cmd="d2b__subcmd__provider__subcmd__list"
                ;;
            d2b__subcmd__provider,status)
                cmd="d2b__subcmd__provider__subcmd__status"
                ;;
            d2b__subcmd__quota,create)
                cmd="d2b__subcmd__quota__subcmd__create"
                ;;
            d2b__subcmd__quota,delete)
                cmd="d2b__subcmd__quota__subcmd__delete"
                ;;
            d2b__subcmd__quota,get)
                cmd="d2b__subcmd__quota__subcmd__get"
                ;;
            d2b__subcmd__quota,list)
                cmd="d2b__subcmd__quota__subcmd__list"
                ;;
            d2b__subcmd__quota,reconcile)
                cmd="d2b__subcmd__quota__subcmd__reconcile"
                ;;
            d2b__subcmd__quota,security-key)
                cmd="d2b__subcmd__quota__subcmd__security__subcmd__key"
                ;;
            d2b__subcmd__quota,status)
                cmd="d2b__subcmd__quota__subcmd__status"
                ;;
            d2b__subcmd__quota,update-spec)
                cmd="d2b__subcmd__quota__subcmd__update__subcmd__spec"
                ;;
            d2b__subcmd__quota,upgrade)
                cmd="d2b__subcmd__quota__subcmd__upgrade"
                ;;
            d2b__subcmd__quota,usb)
                cmd="d2b__subcmd__quota__subcmd__usb"
                ;;
            d2b__subcmd__quota,verify)
                cmd="d2b__subcmd__quota__subcmd__verify"
                ;;
            d2b__subcmd__quota,watch)
                cmd="d2b__subcmd__quota__subcmd__watch"
                ;;
            d2b__subcmd__quota__subcmd__security__subcmd__key,cancel)
                cmd="d2b__subcmd__quota__subcmd__security__subcmd__key__subcmd__cancel"
                ;;
            d2b__subcmd__quota__subcmd__security__subcmd__key,sessions)
                cmd="d2b__subcmd__quota__subcmd__security__subcmd__key__subcmd__sessions"
                ;;
            d2b__subcmd__quota__subcmd__security__subcmd__key,status)
                cmd="d2b__subcmd__quota__subcmd__security__subcmd__key__subcmd__status"
                ;;
            d2b__subcmd__quota__subcmd__security__subcmd__key,test)
                cmd="d2b__subcmd__quota__subcmd__security__subcmd__key__subcmd__test"
                ;;
            d2b__subcmd__quota__subcmd__usb,attach)
                cmd="d2b__subcmd__quota__subcmd__usb__subcmd__attach"
                ;;
            d2b__subcmd__quota__subcmd__usb,detach)
                cmd="d2b__subcmd__quota__subcmd__usb__subcmd__detach"
                ;;
            d2b__subcmd__quota__subcmd__usb,probe)
                cmd="d2b__subcmd__quota__subcmd__usb__subcmd__probe"
                ;;
            d2b__subcmd__resource,authorities)
                cmd="d2b__subcmd__resource__subcmd__authorities"
                ;;
            d2b__subcmd__resource,create)
                cmd="d2b__subcmd__resource__subcmd__create"
                ;;
            d2b__subcmd__resource,delete)
                cmd="d2b__subcmd__resource__subcmd__delete"
                ;;
            d2b__subcmd__resource,get)
                cmd="d2b__subcmd__resource__subcmd__get"
                ;;
            d2b__subcmd__resource,list)
                cmd="d2b__subcmd__resource__subcmd__list"
                ;;
            d2b__subcmd__resource,reconcile)
                cmd="d2b__subcmd__resource__subcmd__reconcile"
                ;;
            d2b__subcmd__resource,status)
                cmd="d2b__subcmd__resource__subcmd__status"
                ;;
            d2b__subcmd__resource,update-spec)
                cmd="d2b__subcmd__resource__subcmd__update__subcmd__spec"
                ;;
            d2b__subcmd__resource,upgrade)
                cmd="d2b__subcmd__resource__subcmd__upgrade"
                ;;
            d2b__subcmd__resource,watch)
                cmd="d2b__subcmd__resource__subcmd__watch"
                ;;
            d2b__subcmd__resource__subcmd__authorities,conflict)
                cmd="d2b__subcmd__resource__subcmd__authorities__subcmd__conflict"
                ;;
            d2b__subcmd__resource__subcmd__authorities,holders)
                cmd="d2b__subcmd__resource__subcmd__authorities__subcmd__holders"
                ;;
            d2b__subcmd__shell,attach)
                cmd="d2b__subcmd__shell__subcmd__attach"
                ;;
            d2b__subcmd__shell,detach)
                cmd="d2b__subcmd__shell__subcmd__detach"
                ;;
            d2b__subcmd__shell,kill)
                cmd="d2b__subcmd__shell__subcmd__kill"
                ;;
            d2b__subcmd__shell,list)
                cmd="d2b__subcmd__shell__subcmd__list"
                ;;
            d2b__subcmd__shell,open)
                cmd="d2b__subcmd__shell__subcmd__open"
                ;;
            d2b__subcmd__shell,status)
                cmd="d2b__subcmd__shell__subcmd__status"
                ;;
            d2b__subcmd__user,create)
                cmd="d2b__subcmd__user__subcmd__create"
                ;;
            d2b__subcmd__user,delete)
                cmd="d2b__subcmd__user__subcmd__delete"
                ;;
            d2b__subcmd__user,get)
                cmd="d2b__subcmd__user__subcmd__get"
                ;;
            d2b__subcmd__user,list)
                cmd="d2b__subcmd__user__subcmd__list"
                ;;
            d2b__subcmd__user,reconcile)
                cmd="d2b__subcmd__user__subcmd__reconcile"
                ;;
            d2b__subcmd__user,security-key)
                cmd="d2b__subcmd__user__subcmd__security__subcmd__key"
                ;;
            d2b__subcmd__user,status)
                cmd="d2b__subcmd__user__subcmd__status"
                ;;
            d2b__subcmd__user,update-spec)
                cmd="d2b__subcmd__user__subcmd__update__subcmd__spec"
                ;;
            d2b__subcmd__user,upgrade)
                cmd="d2b__subcmd__user__subcmd__upgrade"
                ;;
            d2b__subcmd__user,usb)
                cmd="d2b__subcmd__user__subcmd__usb"
                ;;
            d2b__subcmd__user,verify)
                cmd="d2b__subcmd__user__subcmd__verify"
                ;;
            d2b__subcmd__user,watch)
                cmd="d2b__subcmd__user__subcmd__watch"
                ;;
            d2b__subcmd__user__subcmd__security__subcmd__key,cancel)
                cmd="d2b__subcmd__user__subcmd__security__subcmd__key__subcmd__cancel"
                ;;
            d2b__subcmd__user__subcmd__security__subcmd__key,sessions)
                cmd="d2b__subcmd__user__subcmd__security__subcmd__key__subcmd__sessions"
                ;;
            d2b__subcmd__user__subcmd__security__subcmd__key,status)
                cmd="d2b__subcmd__user__subcmd__security__subcmd__key__subcmd__status"
                ;;
            d2b__subcmd__user__subcmd__security__subcmd__key,test)
                cmd="d2b__subcmd__user__subcmd__security__subcmd__key__subcmd__test"
                ;;
            d2b__subcmd__user__subcmd__usb,attach)
                cmd="d2b__subcmd__user__subcmd__usb__subcmd__attach"
                ;;
            d2b__subcmd__user__subcmd__usb,detach)
                cmd="d2b__subcmd__user__subcmd__usb__subcmd__detach"
                ;;
            d2b__subcmd__user__subcmd__usb,probe)
                cmd="d2b__subcmd__user__subcmd__usb__subcmd__probe"
                ;;
            d2b__subcmd__volume,create)
                cmd="d2b__subcmd__volume__subcmd__create"
                ;;
            d2b__subcmd__volume,delete)
                cmd="d2b__subcmd__volume__subcmd__delete"
                ;;
            d2b__subcmd__volume,get)
                cmd="d2b__subcmd__volume__subcmd__get"
                ;;
            d2b__subcmd__volume,list)
                cmd="d2b__subcmd__volume__subcmd__list"
                ;;
            d2b__subcmd__volume,reconcile)
                cmd="d2b__subcmd__volume__subcmd__reconcile"
                ;;
            d2b__subcmd__volume,security-key)
                cmd="d2b__subcmd__volume__subcmd__security__subcmd__key"
                ;;
            d2b__subcmd__volume,status)
                cmd="d2b__subcmd__volume__subcmd__status"
                ;;
            d2b__subcmd__volume,update-spec)
                cmd="d2b__subcmd__volume__subcmd__update__subcmd__spec"
                ;;
            d2b__subcmd__volume,upgrade)
                cmd="d2b__subcmd__volume__subcmd__upgrade"
                ;;
            d2b__subcmd__volume,usb)
                cmd="d2b__subcmd__volume__subcmd__usb"
                ;;
            d2b__subcmd__volume,verify)
                cmd="d2b__subcmd__volume__subcmd__verify"
                ;;
            d2b__subcmd__volume,watch)
                cmd="d2b__subcmd__volume__subcmd__watch"
                ;;
            d2b__subcmd__volume__subcmd__security__subcmd__key,cancel)
                cmd="d2b__subcmd__volume__subcmd__security__subcmd__key__subcmd__cancel"
                ;;
            d2b__subcmd__volume__subcmd__security__subcmd__key,sessions)
                cmd="d2b__subcmd__volume__subcmd__security__subcmd__key__subcmd__sessions"
                ;;
            d2b__subcmd__volume__subcmd__security__subcmd__key,status)
                cmd="d2b__subcmd__volume__subcmd__security__subcmd__key__subcmd__status"
                ;;
            d2b__subcmd__volume__subcmd__security__subcmd__key,test)
                cmd="d2b__subcmd__volume__subcmd__security__subcmd__key__subcmd__test"
                ;;
            d2b__subcmd__volume__subcmd__usb,attach)
                cmd="d2b__subcmd__volume__subcmd__usb__subcmd__attach"
                ;;
            d2b__subcmd__volume__subcmd__usb,detach)
                cmd="d2b__subcmd__volume__subcmd__usb__subcmd__detach"
                ;;
            d2b__subcmd__volume__subcmd__usb,probe)
                cmd="d2b__subcmd__volume__subcmd__usb__subcmd__probe"
                ;;
            d2b__subcmd__zone,get)
                cmd="d2b__subcmd__zone__subcmd__get"
                ;;
            d2b__subcmd__zone,list)
                cmd="d2b__subcmd__zone__subcmd__list"
                ;;
            d2b__subcmd__zone,status)
                cmd="d2b__subcmd__zone__subcmd__status"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        d2b)
            opts="-h -V --zone --json --human --deadline --no-deadline --help --version get list watch create update-spec delete status upgrade reconcile host guest process exec shell volume network device endpoint export import resource user credential provider zone quota emergency-policy activation audit op auth complete audio clipboard display"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation)
            opts="-h --zone --json --human --deadline --no-deadline --help apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__apply)
            opts="-h --dry-run --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__boot)
            opts="-h --dry-run --apply --to-generation --zone --json --human --deadline --no-deadline --help <GUEST_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --to-generation)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__build)
            opts="-h --zone --json --human --deadline --no-deadline --help <GUEST_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__config)
            opts="-h --zone --json --human --deadline --no-deadline --help sync diff approve reject status"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__config__subcmd__approve)
            opts="-h --to --zone --json --human --deadline --no-deadline --help <GUEST_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --to)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__config__subcmd__diff)
            opts="-h --against --zone --json --human --deadline --no-deadline --help <GUEST_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --against)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__config__subcmd__reject)
            opts="-h --dry-run --zone --json --human --deadline --no-deadline --help <GUEST_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__config__subcmd__status)
            opts="-h --dry-run --zone --json --human --deadline --no-deadline --help <GUEST_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__config__subcmd__sync)
            opts="-h --dry-run --zone --json --human --deadline --no-deadline --help <GUEST_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__gc)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__generations)
            opts="-h --zone --json --human --deadline --no-deadline --help <GUEST_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__keys)
            opts="-h --zone --json --human --deadline --no-deadline --help list show rotate"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__keys__subcmd__list)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__keys__subcmd__rotate)
            opts="-h --dry-run --apply --to-generation --zone --json --human --deadline --no-deadline --help <GUEST_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --to-generation)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__keys__subcmd__show)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__migrate)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__rollback)
            opts="-h --dry-run --apply --to-generation --zone --json --human --deadline --no-deadline --help <GUEST_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --to-generation)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__rotate__subcmd__known__subcmd__host)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__switch)
            opts="-h --dry-run --apply --to-generation --zone --json --human --deadline --no-deadline --help <GUEST_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --to-generation)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__test)
            opts="-h --dry-run --apply --to-generation --zone --json --human --deadline --no-deadline --help <GUEST_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --to-generation)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__activation__subcmd__trust)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__audio)
            opts="-h --zone --json --human --deadline --no-deadline --help <VERB> [ARGS]..."
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__audit)
            opts="-h --strict --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__auth)
            opts="-h --test-uid --zone --json --human --deadline --no-deadline --help status"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --test-uid)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__auth__subcmd__status)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__clipboard)
            opts="-h --zone --json --human --deadline --no-deadline --help <VERB> [ARGS]..."
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__complete)
            opts="-h --list-commands --zone --json --human --deadline --no-deadline --help bash zsh fish"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__create)
            opts="-h --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <RESOURCE_TYPE>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential)
            opts="-h --zone --json --human --deadline --no-deadline --help get list watch create update-spec delete status upgrade reconcile verify usb security-key"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__create)
            opts="-h --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__delete)
            opts="-h --revision --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__list)
            opts="-h --execution-ref --domain --phase --label-selector --updates --page-token --limit --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --execution-ref)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --page-token)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__reconcile)
            opts="-h --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__security__subcmd__key)
            opts="-h --zone --json --human --deadline --no-deadline --help status sessions cancel test"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__security__subcmd__key__subcmd__cancel)
            opts="-h --current --dry-run --apply --zone --json --human --deadline --no-deadline --help [SESSION_ID]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__security__subcmd__key__subcmd__sessions)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__security__subcmd__key__subcmd__status)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__security__subcmd__key__subcmd__test)
            opts="-h --dry-run --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__update__subcmd__spec)
            opts="-h --revision --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__upgrade)
            opts="-h --recursive --apply --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__usb)
            opts="-h --zone --json --human --deadline --no-deadline --help attach detach probe"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__usb__subcmd__attach)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help <NAME> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__usb__subcmd__detach)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help <NAME> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__usb__subcmd__probe)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__verify)
            opts="-h --repair --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__credential__subcmd__watch)
            opts="-h --since-revision --phase --label-selector --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --since-revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__delete)
            opts="-h --revision --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device)
            opts="-h --zone --json --human --deadline --no-deadline --help get list watch create update-spec delete status upgrade reconcile verify usb security-key"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__create)
            opts="-h --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__delete)
            opts="-h --revision --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__list)
            opts="-h --execution-ref --domain --phase --label-selector --updates --page-token --limit --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --execution-ref)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --page-token)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__reconcile)
            opts="-h --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__security__subcmd__key)
            opts="-h --zone --json --human --deadline --no-deadline --help status sessions cancel test"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__security__subcmd__key__subcmd__cancel)
            opts="-h --current --dry-run --apply --zone --json --human --deadline --no-deadline --help [SESSION_ID]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__security__subcmd__key__subcmd__sessions)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__security__subcmd__key__subcmd__status)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__security__subcmd__key__subcmd__test)
            opts="-h --dry-run --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__update__subcmd__spec)
            opts="-h --revision --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__upgrade)
            opts="-h --recursive --apply --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__usb)
            opts="-h --zone --json --human --deadline --no-deadline --help attach detach probe"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__usb__subcmd__attach)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help <NAME> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__usb__subcmd__detach)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help <NAME> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__usb__subcmd__probe)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__verify)
            opts="-h --repair --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__device__subcmd__watch)
            opts="-h --since-revision --phase --label-selector --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --since-revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__display)
            opts="-h --zone --json --human --deadline --no-deadline --help <VERB> [ARGS]..."
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy)
            opts="-h --zone --json --human --deadline --no-deadline --help get list watch create update-spec delete status upgrade reconcile verify usb security-key"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__create)
            opts="-h --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__delete)
            opts="-h --revision --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__list)
            opts="-h --execution-ref --domain --phase --label-selector --updates --page-token --limit --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --execution-ref)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --page-token)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__reconcile)
            opts="-h --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__security__subcmd__key)
            opts="-h --zone --json --human --deadline --no-deadline --help status sessions cancel test"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__security__subcmd__key__subcmd__cancel)
            opts="-h --current --dry-run --apply --zone --json --human --deadline --no-deadline --help [SESSION_ID]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__security__subcmd__key__subcmd__sessions)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__security__subcmd__key__subcmd__status)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__security__subcmd__key__subcmd__test)
            opts="-h --dry-run --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__update__subcmd__spec)
            opts="-h --revision --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__upgrade)
            opts="-h --recursive --apply --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__usb)
            opts="-h --zone --json --human --deadline --no-deadline --help attach detach probe"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__usb__subcmd__attach)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help <NAME> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__usb__subcmd__detach)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help <NAME> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__usb__subcmd__probe)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__verify)
            opts="-h --repair --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__emergency__subcmd__policy__subcmd__watch)
            opts="-h --since-revision --phase --label-selector --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --since-revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__endpoint)
            opts="-h --zone --json --human --deadline --no-deadline --help get list watch status resolve"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__endpoint__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__endpoint__subcmd__list)
            opts="-h --endpoint-class --updates --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --endpoint-class)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__endpoint__subcmd__resolve)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__endpoint__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__endpoint__subcmd__watch)
            opts="-h --endpoint-class --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --endpoint-class)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__exec)
            opts="-h --zone --json --human --deadline --no-deadline --help run attach wait status list logs kill"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__exec__subcmd__attach)
            opts="-i -t -h --interactive --tty --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__exec__subcmd__kill)
            opts="-h --signal --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --signal)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__exec__subcmd__list)
            opts="-h --phase --zone --json --human --deadline --no-deadline --help [EXECUTION_REF]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__exec__subcmd__logs)
            opts="-h --stdout-offset --stderr-offset --max-len --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --stdout-offset)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --stderr-offset)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --max-len)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__exec__subcmd__run)
            opts="-h --name --domain --user --provider --env --cwd --zone --json --human --deadline --no-deadline --help <EXECUTION_REF> <COMMAND>..."
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --name)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --user)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --provider)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --env)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --cwd)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__exec__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__exec__subcmd__wait)
            opts="-h --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__export)
            opts="-h --zone --json --human --deadline --no-deadline --help get list watch status create update-spec delete"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__export__subcmd__create)
            opts="-h --spec-file --spec-stdin --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__export__subcmd__delete)
            opts="-h --revision --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__export__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__export__subcmd__list)
            opts="-h --exported-type --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --exported-type)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__export__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__export__subcmd__update__subcmd__spec)
            opts="-h --revision --spec-file --spec-stdin --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__export__subcmd__watch)
            opts="-h --exported-type --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --exported-type)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__guest)
            opts="-h --zone --json --human --deadline --no-deadline --help get list status start stop restart create update-spec delete console"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__guest__subcmd__console)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__guest__subcmd__create)
            opts="-h --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__guest__subcmd__delete)
            opts="-h --revision --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__guest__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__guest__subcmd__list)
            opts="-h --execution-ref --domain --phase --label-selector --updates --page-token --limit --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --execution-ref)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --page-token)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__guest__subcmd__restart)
            opts="-f -h --dry-run --apply --no-wait-ready --force --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__guest__subcmd__start)
            opts="-f -h --dry-run --apply --no-wait-ready --force --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__guest__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__guest__subcmd__stop)
            opts="-f -h --dry-run --apply --no-wait-ready --force --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__guest__subcmd__update__subcmd__spec)
            opts="-h --revision --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host)
            opts="-h --zone --json --human --deadline --no-deadline --help get list status check prepare destroy doctor install reconcile validate"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__check)
            opts="-h --read-only --strict --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__destroy)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__doctor)
            opts="-h --read-only --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__install)
            opts="-h --dry-run --apply --enable --start --no-start --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__list)
            opts="-h --execution-ref --domain --phase --label-selector --updates --page-token --limit --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --execution-ref)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --page-token)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__prepare)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__reconcile)
            opts="-h --network --dry-run --apply --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__validate)
            opts="-h --dry-run --apply --wave --evidence-dir --scripts-dir --operator-signature --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --wave)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --evidence-dir)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --scripts-dir)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --operator-signature)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__import)
            opts="-h --zone --json --human --deadline --no-deadline --help get list watch status projection graph create update-spec delete"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__import__subcmd__create)
            opts="-h --spec-file --spec-stdin --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__import__subcmd__delete)
            opts="-h --revision --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__import__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__import__subcmd__graph)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__import__subcmd__list)
            opts="-h --expected-type --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --expected-type)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__import__subcmd__projection)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__import__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__import__subcmd__update__subcmd__spec)
            opts="-h --revision --spec-file --spec-stdin --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__import__subcmd__watch)
            opts="-h --expected-type --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --expected-type)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__list)
            opts="-h --execution-ref --domain --phase --label-selector --updates --page-token --limit --zone --json --human --deadline --no-deadline --help <RESOURCE_TYPE>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --execution-ref)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --page-token)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network)
            opts="-h --zone --json --human --deadline --no-deadline --help get list watch create update-spec delete status upgrade reconcile verify usb security-key"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__create)
            opts="-h --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__delete)
            opts="-h --revision --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__list)
            opts="-h --execution-ref --domain --phase --label-selector --updates --page-token --limit --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --execution-ref)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --page-token)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__reconcile)
            opts="-h --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__security__subcmd__key)
            opts="-h --zone --json --human --deadline --no-deadline --help status sessions cancel test"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__security__subcmd__key__subcmd__cancel)
            opts="-h --current --dry-run --apply --zone --json --human --deadline --no-deadline --help [SESSION_ID]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__security__subcmd__key__subcmd__sessions)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__security__subcmd__key__subcmd__status)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__security__subcmd__key__subcmd__test)
            opts="-h --dry-run --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__update__subcmd__spec)
            opts="-h --revision --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__upgrade)
            opts="-h --recursive --apply --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__usb)
            opts="-h --zone --json --human --deadline --no-deadline --help attach detach probe"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__usb__subcmd__attach)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help <NAME> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__usb__subcmd__detach)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help <NAME> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__usb__subcmd__probe)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__verify)
            opts="-h --repair --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__network__subcmd__watch)
            opts="-h --since-revision --phase --label-selector --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --since-revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__op)
            opts="-h --zone --json --human --deadline --no-deadline --help inspect"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__op__subcmd__inspect)
            opts="-h --operation-id --trace-id --span-id --watch --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --operation-id)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --trace-id)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --span-id)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__process)
            opts="-h --zone --json --human --deadline --no-deadline --help get list status start stop create update-spec delete"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__process__subcmd__create)
            opts="-h --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__process__subcmd__delete)
            opts="-h --revision --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__process__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__process__subcmd__list)
            opts="-h --execution-ref --domain --phase --label-selector --updates --page-token --limit --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --execution-ref)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --page-token)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__process__subcmd__start)
            opts="-f -h --dry-run --apply --no-wait-ready --force --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__process__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__process__subcmd__stop)
            opts="-f -h --dry-run --apply --no-wait-ready --force --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__process__subcmd__update__subcmd__spec)
            opts="-h --revision --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__provider)
            opts="-h --zone --json --human --deadline --no-deadline --help list get status inspect"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__provider__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__provider__subcmd__inspect)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__provider__subcmd__list)
            opts="-h --package-only --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__provider__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota)
            opts="-h --zone --json --human --deadline --no-deadline --help get list watch create update-spec delete status upgrade reconcile verify usb security-key"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__create)
            opts="-h --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__delete)
            opts="-h --revision --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__list)
            opts="-h --execution-ref --domain --phase --label-selector --updates --page-token --limit --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --execution-ref)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --page-token)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__reconcile)
            opts="-h --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__security__subcmd__key)
            opts="-h --zone --json --human --deadline --no-deadline --help status sessions cancel test"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__security__subcmd__key__subcmd__cancel)
            opts="-h --current --dry-run --apply --zone --json --human --deadline --no-deadline --help [SESSION_ID]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__security__subcmd__key__subcmd__sessions)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__security__subcmd__key__subcmd__status)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__security__subcmd__key__subcmd__test)
            opts="-h --dry-run --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__update__subcmd__spec)
            opts="-h --revision --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__upgrade)
            opts="-h --recursive --apply --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__usb)
            opts="-h --zone --json --human --deadline --no-deadline --help attach detach probe"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__usb__subcmd__attach)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help <NAME> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__usb__subcmd__detach)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help <NAME> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__usb__subcmd__probe)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__verify)
            opts="-h --repair --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__quota__subcmd__watch)
            opts="-h --since-revision --phase --label-selector --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --since-revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__reconcile)
            opts="-h --reconcile-deadline --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__resource)
            opts="-h --zone --json --human --deadline --no-deadline --help get list watch create update-spec delete status upgrade reconcile authorities"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__resource__subcmd__authorities)
            opts="-h --scope --zone --json --human --deadline --no-deadline --help holders conflict"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --scope)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__resource__subcmd__authorities__subcmd__conflict)
            opts="-h --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__resource__subcmd__authorities__subcmd__holders)
            opts="-h --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__resource__subcmd__create)
            opts="-h --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <RESOURCE_TYPE>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__resource__subcmd__delete)
            opts="-h --revision --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__resource__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__resource__subcmd__list)
            opts="-h --execution-ref --domain --phase --label-selector --updates --page-token --limit --zone --json --human --deadline --no-deadline --help <RESOURCE_TYPE>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --execution-ref)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --page-token)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__resource__subcmd__reconcile)
            opts="-h --reconcile-deadline --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__resource__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__resource__subcmd__update__subcmd__spec)
            opts="-h --revision --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__resource__subcmd__upgrade)
            opts="-h --recursive --apply --reconcile-deadline --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__resource__subcmd__watch)
            opts="-h --since-revision --phase --label-selector --zone --json --human --deadline --no-deadline --help <RESOURCE_TYPE>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --since-revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__shell)
            opts="-h --zone --json --human --deadline --no-deadline --help open attach list detach kill status"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__shell__subcmd__attach)
            opts="-h --force --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__shell__subcmd__detach)
            opts="-h --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__shell__subcmd__kill)
            opts="-h --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__shell__subcmd__list)
            opts="-h --zone --json --human --deadline --no-deadline --help [EXECUTION_REF]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__shell__subcmd__open)
            opts="-h --name --force --zone --json --human --deadline --no-deadline --help <EXECUTION_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --name)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__shell__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__update__subcmd__spec)
            opts="-h --revision --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__upgrade)
            opts="-h --recursive --apply --reconcile-deadline --zone --json --human --deadline --no-deadline --help <RESOURCE_REF>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user)
            opts="-h --zone --json --human --deadline --no-deadline --help get list watch create update-spec delete status upgrade reconcile verify usb security-key"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__create)
            opts="-h --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__delete)
            opts="-h --revision --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__list)
            opts="-h --execution-ref --domain --phase --label-selector --updates --page-token --limit --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --execution-ref)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --page-token)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__reconcile)
            opts="-h --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__security__subcmd__key)
            opts="-h --zone --json --human --deadline --no-deadline --help status sessions cancel test"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__security__subcmd__key__subcmd__cancel)
            opts="-h --current --dry-run --apply --zone --json --human --deadline --no-deadline --help [SESSION_ID]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__security__subcmd__key__subcmd__sessions)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__security__subcmd__key__subcmd__status)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__security__subcmd__key__subcmd__test)
            opts="-h --dry-run --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__update__subcmd__spec)
            opts="-h --revision --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__upgrade)
            opts="-h --recursive --apply --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__usb)
            opts="-h --zone --json --human --deadline --no-deadline --help attach detach probe"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__usb__subcmd__attach)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help <NAME> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__usb__subcmd__detach)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help <NAME> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__usb__subcmd__probe)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__verify)
            opts="-h --repair --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__user__subcmd__watch)
            opts="-h --since-revision --phase --label-selector --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --since-revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume)
            opts="-h --zone --json --human --deadline --no-deadline --help get list watch create update-spec delete status upgrade reconcile verify usb security-key"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__create)
            opts="-h --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__delete)
            opts="-h --revision --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__list)
            opts="-h --execution-ref --domain --phase --label-selector --updates --page-token --limit --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --execution-ref)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --domain)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --page-token)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__reconcile)
            opts="-h --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__security__subcmd__key)
            opts="-h --zone --json --human --deadline --no-deadline --help status sessions cancel test"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__security__subcmd__key__subcmd__cancel)
            opts="-h --current --dry-run --apply --zone --json --human --deadline --no-deadline --help [SESSION_ID]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__security__subcmd__key__subcmd__sessions)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__security__subcmd__key__subcmd__status)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__security__subcmd__key__subcmd__test)
            opts="-h --dry-run --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__update__subcmd__spec)
            opts="-h --revision --spec-file --spec-stdin --wait-for-reconcile --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --spec-file)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__upgrade)
            opts="-h --recursive --apply --reconcile-deadline --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --reconcile-deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__usb)
            opts="-h --zone --json --human --deadline --no-deadline --help attach detach probe"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__usb__subcmd__attach)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help <NAME> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__usb__subcmd__detach)
            opts="-h --dry-run --apply --zone --json --human --deadline --no-deadline --help <NAME> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__usb__subcmd__probe)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__verify)
            opts="-h --repair --zone --json --human --deadline --no-deadline --help <NAME>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__volume__subcmd__watch)
            opts="-h --since-revision --phase --label-selector --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --since-revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__watch)
            opts="-h --since-revision --phase --label-selector --zone --json --human --deadline --no-deadline --help <RESOURCE_TYPE>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --since-revision)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --phase)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --label-selector)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__zone)
            opts="-h --zone --json --human --deadline --no-deadline --help get list status"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__zone__subcmd__get)
            opts="-h --zone --json --human --deadline --no-deadline --help [NAME]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__zone__subcmd__list)
            opts="-h --zone --json --human --deadline --no-deadline --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__zone__subcmd__status)
            opts="-h --watch --zone --json --human --deadline --no-deadline --help [NAME]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --zone)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --deadline)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _d2b -o nosort -o bashdefault -o default d2b
else
    complete -F _d2b -o bashdefault -o default d2b
fi
