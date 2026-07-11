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
            d2b,audio)
                cmd="d2b__subcmd__audio"
                ;;
            d2b,audit)
                cmd="d2b__subcmd__audit"
                ;;
            d2b,auth)
                cmd="d2b__subcmd__auth"
                ;;
            d2b,boot)
                cmd="d2b__subcmd__boot"
                ;;
            d2b,build)
                cmd="d2b__subcmd__build"
                ;;
            d2b,clipboard)
                cmd="d2b__subcmd__clipboard"
                ;;
            d2b,config)
                cmd="d2b__subcmd__config"
                ;;
            d2b,console)
                cmd="d2b__subcmd__console"
                ;;
            d2b,down)
                cmd="d2b__subcmd__down"
                ;;
            d2b,gc)
                cmd="d2b__subcmd__gc"
                ;;
            d2b,generations)
                cmd="d2b__subcmd__generations"
                ;;
            d2b,help)
                cmd="d2b__subcmd__help"
                ;;
            d2b,host)
                cmd="d2b__subcmd__host"
                ;;
            d2b,keys)
                cmd="d2b__subcmd__keys"
                ;;
            d2b,launch)
                cmd="d2b__subcmd__launch"
                ;;
            d2b,list)
                cmd="d2b__subcmd__list"
                ;;
            d2b,migrate)
                cmd="d2b__subcmd__migrate"
                ;;
            d2b,op)
                cmd="d2b__subcmd__op"
                ;;
            d2b,realm)
                cmd="d2b__subcmd__realm"
                ;;
            d2b,restart)
                cmd="d2b__subcmd__restart"
                ;;
            d2b,rollback)
                cmd="d2b__subcmd__rollback"
                ;;
            d2b,rotate-known-host)
                cmd="d2b__subcmd__rotate__subcmd__known__subcmd__host"
                ;;
            d2b,shell)
                cmd="d2b__subcmd__shell"
                ;;
            d2b,status)
                cmd="d2b__subcmd__status"
                ;;
            d2b,store)
                cmd="d2b__subcmd__store"
                ;;
            d2b,switch)
                cmd="d2b__subcmd__switch"
                ;;
            d2b,test)
                cmd="d2b__subcmd__test"
                ;;
            d2b,trust)
                cmd="d2b__subcmd__trust"
                ;;
            d2b,up)
                cmd="d2b__subcmd__up"
                ;;
            d2b,usb)
                cmd="d2b__subcmd__usb"
                ;;
            d2b,vm)
                cmd="d2b__subcmd__vm"
                ;;
            d2b__subcmd__audio,help)
                cmd="d2b__subcmd__audio__subcmd__help"
                ;;
            d2b__subcmd__audio,mic)
                cmd="d2b__subcmd__audio__subcmd__mic"
                ;;
            d2b__subcmd__audio,off)
                cmd="d2b__subcmd__audio__subcmd__off"
                ;;
            d2b__subcmd__audio,speaker)
                cmd="d2b__subcmd__audio__subcmd__speaker"
                ;;
            d2b__subcmd__audio,status)
                cmd="d2b__subcmd__audio__subcmd__status"
                ;;
            d2b__subcmd__audio__subcmd__help,help)
                cmd="d2b__subcmd__audio__subcmd__help__subcmd__help"
                ;;
            d2b__subcmd__audio__subcmd__help,mic)
                cmd="d2b__subcmd__audio__subcmd__help__subcmd__mic"
                ;;
            d2b__subcmd__audio__subcmd__help,off)
                cmd="d2b__subcmd__audio__subcmd__help__subcmd__off"
                ;;
            d2b__subcmd__audio__subcmd__help,speaker)
                cmd="d2b__subcmd__audio__subcmd__help__subcmd__speaker"
                ;;
            d2b__subcmd__audio__subcmd__help,status)
                cmd="d2b__subcmd__audio__subcmd__help__subcmd__status"
                ;;
            d2b__subcmd__auth,help)
                cmd="d2b__subcmd__auth__subcmd__help"
                ;;
            d2b__subcmd__auth,status)
                cmd="d2b__subcmd__auth__subcmd__status"
                ;;
            d2b__subcmd__auth__subcmd__help,help)
                cmd="d2b__subcmd__auth__subcmd__help__subcmd__help"
                ;;
            d2b__subcmd__auth__subcmd__help,status)
                cmd="d2b__subcmd__auth__subcmd__help__subcmd__status"
                ;;
            d2b__subcmd__clipboard,arm)
                cmd="d2b__subcmd__clipboard__subcmd__arm"
                ;;
            d2b__subcmd__clipboard,help)
                cmd="d2b__subcmd__clipboard__subcmd__help"
                ;;
            d2b__subcmd__clipboard__subcmd__help,arm)
                cmd="d2b__subcmd__clipboard__subcmd__help__subcmd__arm"
                ;;
            d2b__subcmd__clipboard__subcmd__help,help)
                cmd="d2b__subcmd__clipboard__subcmd__help__subcmd__help"
                ;;
            d2b__subcmd__config,approve)
                cmd="d2b__subcmd__config__subcmd__approve"
                ;;
            d2b__subcmd__config,diff)
                cmd="d2b__subcmd__config__subcmd__diff"
                ;;
            d2b__subcmd__config,help)
                cmd="d2b__subcmd__config__subcmd__help"
                ;;
            d2b__subcmd__config,reject)
                cmd="d2b__subcmd__config__subcmd__reject"
                ;;
            d2b__subcmd__config,status)
                cmd="d2b__subcmd__config__subcmd__status"
                ;;
            d2b__subcmd__config,sync)
                cmd="d2b__subcmd__config__subcmd__sync"
                ;;
            d2b__subcmd__config__subcmd__help,approve)
                cmd="d2b__subcmd__config__subcmd__help__subcmd__approve"
                ;;
            d2b__subcmd__config__subcmd__help,diff)
                cmd="d2b__subcmd__config__subcmd__help__subcmd__diff"
                ;;
            d2b__subcmd__config__subcmd__help,help)
                cmd="d2b__subcmd__config__subcmd__help__subcmd__help"
                ;;
            d2b__subcmd__config__subcmd__help,reject)
                cmd="d2b__subcmd__config__subcmd__help__subcmd__reject"
                ;;
            d2b__subcmd__config__subcmd__help,status)
                cmd="d2b__subcmd__config__subcmd__help__subcmd__status"
                ;;
            d2b__subcmd__config__subcmd__help,sync)
                cmd="d2b__subcmd__config__subcmd__help__subcmd__sync"
                ;;
            d2b__subcmd__help,audio)
                cmd="d2b__subcmd__help__subcmd__audio"
                ;;
            d2b__subcmd__help,audit)
                cmd="d2b__subcmd__help__subcmd__audit"
                ;;
            d2b__subcmd__help,auth)
                cmd="d2b__subcmd__help__subcmd__auth"
                ;;
            d2b__subcmd__help,boot)
                cmd="d2b__subcmd__help__subcmd__boot"
                ;;
            d2b__subcmd__help,build)
                cmd="d2b__subcmd__help__subcmd__build"
                ;;
            d2b__subcmd__help,clipboard)
                cmd="d2b__subcmd__help__subcmd__clipboard"
                ;;
            d2b__subcmd__help,config)
                cmd="d2b__subcmd__help__subcmd__config"
                ;;
            d2b__subcmd__help,console)
                cmd="d2b__subcmd__help__subcmd__console"
                ;;
            d2b__subcmd__help,down)
                cmd="d2b__subcmd__help__subcmd__down"
                ;;
            d2b__subcmd__help,gc)
                cmd="d2b__subcmd__help__subcmd__gc"
                ;;
            d2b__subcmd__help,generations)
                cmd="d2b__subcmd__help__subcmd__generations"
                ;;
            d2b__subcmd__help,help)
                cmd="d2b__subcmd__help__subcmd__help"
                ;;
            d2b__subcmd__help,host)
                cmd="d2b__subcmd__help__subcmd__host"
                ;;
            d2b__subcmd__help,keys)
                cmd="d2b__subcmd__help__subcmd__keys"
                ;;
            d2b__subcmd__help,launch)
                cmd="d2b__subcmd__help__subcmd__launch"
                ;;
            d2b__subcmd__help,list)
                cmd="d2b__subcmd__help__subcmd__list"
                ;;
            d2b__subcmd__help,migrate)
                cmd="d2b__subcmd__help__subcmd__migrate"
                ;;
            d2b__subcmd__help,op)
                cmd="d2b__subcmd__help__subcmd__op"
                ;;
            d2b__subcmd__help,realm)
                cmd="d2b__subcmd__help__subcmd__realm"
                ;;
            d2b__subcmd__help,restart)
                cmd="d2b__subcmd__help__subcmd__restart"
                ;;
            d2b__subcmd__help,rollback)
                cmd="d2b__subcmd__help__subcmd__rollback"
                ;;
            d2b__subcmd__help,rotate-known-host)
                cmd="d2b__subcmd__help__subcmd__rotate__subcmd__known__subcmd__host"
                ;;
            d2b__subcmd__help,shell)
                cmd="d2b__subcmd__help__subcmd__shell"
                ;;
            d2b__subcmd__help,status)
                cmd="d2b__subcmd__help__subcmd__status"
                ;;
            d2b__subcmd__help,store)
                cmd="d2b__subcmd__help__subcmd__store"
                ;;
            d2b__subcmd__help,switch)
                cmd="d2b__subcmd__help__subcmd__switch"
                ;;
            d2b__subcmd__help,test)
                cmd="d2b__subcmd__help__subcmd__test"
                ;;
            d2b__subcmd__help,trust)
                cmd="d2b__subcmd__help__subcmd__trust"
                ;;
            d2b__subcmd__help,up)
                cmd="d2b__subcmd__help__subcmd__up"
                ;;
            d2b__subcmd__help,usb)
                cmd="d2b__subcmd__help__subcmd__usb"
                ;;
            d2b__subcmd__help,vm)
                cmd="d2b__subcmd__help__subcmd__vm"
                ;;
            d2b__subcmd__help__subcmd__audio,mic)
                cmd="d2b__subcmd__help__subcmd__audio__subcmd__mic"
                ;;
            d2b__subcmd__help__subcmd__audio,off)
                cmd="d2b__subcmd__help__subcmd__audio__subcmd__off"
                ;;
            d2b__subcmd__help__subcmd__audio,speaker)
                cmd="d2b__subcmd__help__subcmd__audio__subcmd__speaker"
                ;;
            d2b__subcmd__help__subcmd__audio,status)
                cmd="d2b__subcmd__help__subcmd__audio__subcmd__status"
                ;;
            d2b__subcmd__help__subcmd__auth,status)
                cmd="d2b__subcmd__help__subcmd__auth__subcmd__status"
                ;;
            d2b__subcmd__help__subcmd__clipboard,arm)
                cmd="d2b__subcmd__help__subcmd__clipboard__subcmd__arm"
                ;;
            d2b__subcmd__help__subcmd__config,approve)
                cmd="d2b__subcmd__help__subcmd__config__subcmd__approve"
                ;;
            d2b__subcmd__help__subcmd__config,diff)
                cmd="d2b__subcmd__help__subcmd__config__subcmd__diff"
                ;;
            d2b__subcmd__help__subcmd__config,reject)
                cmd="d2b__subcmd__help__subcmd__config__subcmd__reject"
                ;;
            d2b__subcmd__help__subcmd__config,status)
                cmd="d2b__subcmd__help__subcmd__config__subcmd__status"
                ;;
            d2b__subcmd__help__subcmd__config,sync)
                cmd="d2b__subcmd__help__subcmd__config__subcmd__sync"
                ;;
            d2b__subcmd__help__subcmd__host,check)
                cmd="d2b__subcmd__help__subcmd__host__subcmd__check"
                ;;
            d2b__subcmd__help__subcmd__host,destroy)
                cmd="d2b__subcmd__help__subcmd__host__subcmd__destroy"
                ;;
            d2b__subcmd__help__subcmd__host,doctor)
                cmd="d2b__subcmd__help__subcmd__host__subcmd__doctor"
                ;;
            d2b__subcmd__help__subcmd__host,install)
                cmd="d2b__subcmd__help__subcmd__host__subcmd__install"
                ;;
            d2b__subcmd__help__subcmd__host,migrate-storage)
                cmd="d2b__subcmd__help__subcmd__host__subcmd__migrate__subcmd__storage"
                ;;
            d2b__subcmd__help__subcmd__host,prepare)
                cmd="d2b__subcmd__help__subcmd__host__subcmd__prepare"
                ;;
            d2b__subcmd__help__subcmd__host,reconcile)
                cmd="d2b__subcmd__help__subcmd__host__subcmd__reconcile"
                ;;
            d2b__subcmd__help__subcmd__host,validate)
                cmd="d2b__subcmd__help__subcmd__host__subcmd__validate"
                ;;
            d2b__subcmd__help__subcmd__keys,list)
                cmd="d2b__subcmd__help__subcmd__keys__subcmd__list"
                ;;
            d2b__subcmd__help__subcmd__keys,rotate)
                cmd="d2b__subcmd__help__subcmd__keys__subcmd__rotate"
                ;;
            d2b__subcmd__help__subcmd__keys,show)
                cmd="d2b__subcmd__help__subcmd__keys__subcmd__show"
                ;;
            d2b__subcmd__help__subcmd__op,inspect)
                cmd="d2b__subcmd__help__subcmd__op__subcmd__inspect"
                ;;
            d2b__subcmd__help__subcmd__realm,enter)
                cmd="d2b__subcmd__help__subcmd__realm__subcmd__enter"
                ;;
            d2b__subcmd__help__subcmd__realm,inspect)
                cmd="d2b__subcmd__help__subcmd__realm__subcmd__inspect"
                ;;
            d2b__subcmd__help__subcmd__realm,list)
                cmd="d2b__subcmd__help__subcmd__realm__subcmd__list"
                ;;
            d2b__subcmd__help__subcmd__realm,run)
                cmd="d2b__subcmd__help__subcmd__realm__subcmd__run"
                ;;
            d2b__subcmd__help__subcmd__store,verify)
                cmd="d2b__subcmd__help__subcmd__store__subcmd__verify"
                ;;
            d2b__subcmd__help__subcmd__usb,attach)
                cmd="d2b__subcmd__help__subcmd__usb__subcmd__attach"
                ;;
            d2b__subcmd__help__subcmd__usb,detach)
                cmd="d2b__subcmd__help__subcmd__usb__subcmd__detach"
                ;;
            d2b__subcmd__help__subcmd__usb,probe)
                cmd="d2b__subcmd__help__subcmd__usb__subcmd__probe"
                ;;
            d2b__subcmd__help__subcmd__usb,security-key)
                cmd="d2b__subcmd__help__subcmd__usb__subcmd__security__subcmd__key"
                ;;
            d2b__subcmd__help__subcmd__usb__subcmd__security__subcmd__key,cancel)
                cmd="d2b__subcmd__help__subcmd__usb__subcmd__security__subcmd__key__subcmd__cancel"
                ;;
            d2b__subcmd__help__subcmd__usb__subcmd__security__subcmd__key,sessions)
                cmd="d2b__subcmd__help__subcmd__usb__subcmd__security__subcmd__key__subcmd__sessions"
                ;;
            d2b__subcmd__help__subcmd__usb__subcmd__security__subcmd__key,status)
                cmd="d2b__subcmd__help__subcmd__usb__subcmd__security__subcmd__key__subcmd__status"
                ;;
            d2b__subcmd__help__subcmd__usb__subcmd__security__subcmd__key,test)
                cmd="d2b__subcmd__help__subcmd__usb__subcmd__security__subcmd__key__subcmd__test"
                ;;
            d2b__subcmd__help__subcmd__vm,display)
                cmd="d2b__subcmd__help__subcmd__vm__subcmd__display"
                ;;
            d2b__subcmd__help__subcmd__vm,exec)
                cmd="d2b__subcmd__help__subcmd__vm__subcmd__exec"
                ;;
            d2b__subcmd__help__subcmd__vm,list)
                cmd="d2b__subcmd__help__subcmd__vm__subcmd__list"
                ;;
            d2b__subcmd__help__subcmd__vm,restart)
                cmd="d2b__subcmd__help__subcmd__vm__subcmd__restart"
                ;;
            d2b__subcmd__help__subcmd__vm,start)
                cmd="d2b__subcmd__help__subcmd__vm__subcmd__start"
                ;;
            d2b__subcmd__help__subcmd__vm,status)
                cmd="d2b__subcmd__help__subcmd__vm__subcmd__status"
                ;;
            d2b__subcmd__help__subcmd__vm,stop)
                cmd="d2b__subcmd__help__subcmd__vm__subcmd__stop"
                ;;
            d2b__subcmd__help__subcmd__vm__subcmd__display,close)
                cmd="d2b__subcmd__help__subcmd__vm__subcmd__display__subcmd__close"
                ;;
            d2b__subcmd__help__subcmd__vm__subcmd__display,list)
                cmd="d2b__subcmd__help__subcmd__vm__subcmd__display__subcmd__list"
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
            d2b__subcmd__host,help)
                cmd="d2b__subcmd__host__subcmd__help"
                ;;
            d2b__subcmd__host,install)
                cmd="d2b__subcmd__host__subcmd__install"
                ;;
            d2b__subcmd__host,migrate-storage)
                cmd="d2b__subcmd__host__subcmd__migrate__subcmd__storage"
                ;;
            d2b__subcmd__host,prepare)
                cmd="d2b__subcmd__host__subcmd__prepare"
                ;;
            d2b__subcmd__host,reconcile)
                cmd="d2b__subcmd__host__subcmd__reconcile"
                ;;
            d2b__subcmd__host,validate)
                cmd="d2b__subcmd__host__subcmd__validate"
                ;;
            d2b__subcmd__host__subcmd__help,check)
                cmd="d2b__subcmd__host__subcmd__help__subcmd__check"
                ;;
            d2b__subcmd__host__subcmd__help,destroy)
                cmd="d2b__subcmd__host__subcmd__help__subcmd__destroy"
                ;;
            d2b__subcmd__host__subcmd__help,doctor)
                cmd="d2b__subcmd__host__subcmd__help__subcmd__doctor"
                ;;
            d2b__subcmd__host__subcmd__help,help)
                cmd="d2b__subcmd__host__subcmd__help__subcmd__help"
                ;;
            d2b__subcmd__host__subcmd__help,install)
                cmd="d2b__subcmd__host__subcmd__help__subcmd__install"
                ;;
            d2b__subcmd__host__subcmd__help,migrate-storage)
                cmd="d2b__subcmd__host__subcmd__help__subcmd__migrate__subcmd__storage"
                ;;
            d2b__subcmd__host__subcmd__help,prepare)
                cmd="d2b__subcmd__host__subcmd__help__subcmd__prepare"
                ;;
            d2b__subcmd__host__subcmd__help,reconcile)
                cmd="d2b__subcmd__host__subcmd__help__subcmd__reconcile"
                ;;
            d2b__subcmd__host__subcmd__help,validate)
                cmd="d2b__subcmd__host__subcmd__help__subcmd__validate"
                ;;
            d2b__subcmd__keys,help)
                cmd="d2b__subcmd__keys__subcmd__help"
                ;;
            d2b__subcmd__keys,list)
                cmd="d2b__subcmd__keys__subcmd__list"
                ;;
            d2b__subcmd__keys,rotate)
                cmd="d2b__subcmd__keys__subcmd__rotate"
                ;;
            d2b__subcmd__keys,show)
                cmd="d2b__subcmd__keys__subcmd__show"
                ;;
            d2b__subcmd__keys__subcmd__help,help)
                cmd="d2b__subcmd__keys__subcmd__help__subcmd__help"
                ;;
            d2b__subcmd__keys__subcmd__help,list)
                cmd="d2b__subcmd__keys__subcmd__help__subcmd__list"
                ;;
            d2b__subcmd__keys__subcmd__help,rotate)
                cmd="d2b__subcmd__keys__subcmd__help__subcmd__rotate"
                ;;
            d2b__subcmd__keys__subcmd__help,show)
                cmd="d2b__subcmd__keys__subcmd__help__subcmd__show"
                ;;
            d2b__subcmd__op,help)
                cmd="d2b__subcmd__op__subcmd__help"
                ;;
            d2b__subcmd__op,inspect)
                cmd="d2b__subcmd__op__subcmd__inspect"
                ;;
            d2b__subcmd__op__subcmd__help,help)
                cmd="d2b__subcmd__op__subcmd__help__subcmd__help"
                ;;
            d2b__subcmd__op__subcmd__help,inspect)
                cmd="d2b__subcmd__op__subcmd__help__subcmd__inspect"
                ;;
            d2b__subcmd__realm,enter)
                cmd="d2b__subcmd__realm__subcmd__enter"
                ;;
            d2b__subcmd__realm,help)
                cmd="d2b__subcmd__realm__subcmd__help"
                ;;
            d2b__subcmd__realm,inspect)
                cmd="d2b__subcmd__realm__subcmd__inspect"
                ;;
            d2b__subcmd__realm,list)
                cmd="d2b__subcmd__realm__subcmd__list"
                ;;
            d2b__subcmd__realm,run)
                cmd="d2b__subcmd__realm__subcmd__run"
                ;;
            d2b__subcmd__realm__subcmd__help,enter)
                cmd="d2b__subcmd__realm__subcmd__help__subcmd__enter"
                ;;
            d2b__subcmd__realm__subcmd__help,help)
                cmd="d2b__subcmd__realm__subcmd__help__subcmd__help"
                ;;
            d2b__subcmd__realm__subcmd__help,inspect)
                cmd="d2b__subcmd__realm__subcmd__help__subcmd__inspect"
                ;;
            d2b__subcmd__realm__subcmd__help,list)
                cmd="d2b__subcmd__realm__subcmd__help__subcmd__list"
                ;;
            d2b__subcmd__realm__subcmd__help,run)
                cmd="d2b__subcmd__realm__subcmd__help__subcmd__run"
                ;;
            d2b__subcmd__store,help)
                cmd="d2b__subcmd__store__subcmd__help"
                ;;
            d2b__subcmd__store,verify)
                cmd="d2b__subcmd__store__subcmd__verify"
                ;;
            d2b__subcmd__store__subcmd__help,help)
                cmd="d2b__subcmd__store__subcmd__help__subcmd__help"
                ;;
            d2b__subcmd__store__subcmd__help,verify)
                cmd="d2b__subcmd__store__subcmd__help__subcmd__verify"
                ;;
            d2b__subcmd__usb,attach)
                cmd="d2b__subcmd__usb__subcmd__attach"
                ;;
            d2b__subcmd__usb,detach)
                cmd="d2b__subcmd__usb__subcmd__detach"
                ;;
            d2b__subcmd__usb,help)
                cmd="d2b__subcmd__usb__subcmd__help"
                ;;
            d2b__subcmd__usb,probe)
                cmd="d2b__subcmd__usb__subcmd__probe"
                ;;
            d2b__subcmd__usb,security-key)
                cmd="d2b__subcmd__usb__subcmd__security__subcmd__key"
                ;;
            d2b__subcmd__usb__subcmd__help,attach)
                cmd="d2b__subcmd__usb__subcmd__help__subcmd__attach"
                ;;
            d2b__subcmd__usb__subcmd__help,detach)
                cmd="d2b__subcmd__usb__subcmd__help__subcmd__detach"
                ;;
            d2b__subcmd__usb__subcmd__help,help)
                cmd="d2b__subcmd__usb__subcmd__help__subcmd__help"
                ;;
            d2b__subcmd__usb__subcmd__help,probe)
                cmd="d2b__subcmd__usb__subcmd__help__subcmd__probe"
                ;;
            d2b__subcmd__usb__subcmd__help,security-key)
                cmd="d2b__subcmd__usb__subcmd__help__subcmd__security__subcmd__key"
                ;;
            d2b__subcmd__usb__subcmd__help__subcmd__security__subcmd__key,cancel)
                cmd="d2b__subcmd__usb__subcmd__help__subcmd__security__subcmd__key__subcmd__cancel"
                ;;
            d2b__subcmd__usb__subcmd__help__subcmd__security__subcmd__key,sessions)
                cmd="d2b__subcmd__usb__subcmd__help__subcmd__security__subcmd__key__subcmd__sessions"
                ;;
            d2b__subcmd__usb__subcmd__help__subcmd__security__subcmd__key,status)
                cmd="d2b__subcmd__usb__subcmd__help__subcmd__security__subcmd__key__subcmd__status"
                ;;
            d2b__subcmd__usb__subcmd__help__subcmd__security__subcmd__key,test)
                cmd="d2b__subcmd__usb__subcmd__help__subcmd__security__subcmd__key__subcmd__test"
                ;;
            d2b__subcmd__usb__subcmd__security__subcmd__key,cancel)
                cmd="d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__cancel"
                ;;
            d2b__subcmd__usb__subcmd__security__subcmd__key,help)
                cmd="d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help"
                ;;
            d2b__subcmd__usb__subcmd__security__subcmd__key,sessions)
                cmd="d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__sessions"
                ;;
            d2b__subcmd__usb__subcmd__security__subcmd__key,status)
                cmd="d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__status"
                ;;
            d2b__subcmd__usb__subcmd__security__subcmd__key,test)
                cmd="d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__test"
                ;;
            d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help,cancel)
                cmd="d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help__subcmd__cancel"
                ;;
            d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help,help)
                cmd="d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help__subcmd__help"
                ;;
            d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help,sessions)
                cmd="d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help__subcmd__sessions"
                ;;
            d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help,status)
                cmd="d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help__subcmd__status"
                ;;
            d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help,test)
                cmd="d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help__subcmd__test"
                ;;
            d2b__subcmd__vm,display)
                cmd="d2b__subcmd__vm__subcmd__display"
                ;;
            d2b__subcmd__vm,exec)
                cmd="d2b__subcmd__vm__subcmd__exec"
                ;;
            d2b__subcmd__vm,help)
                cmd="d2b__subcmd__vm__subcmd__help"
                ;;
            d2b__subcmd__vm,list)
                cmd="d2b__subcmd__vm__subcmd__list"
                ;;
            d2b__subcmd__vm,restart)
                cmd="d2b__subcmd__vm__subcmd__restart"
                ;;
            d2b__subcmd__vm,start)
                cmd="d2b__subcmd__vm__subcmd__start"
                ;;
            d2b__subcmd__vm,status)
                cmd="d2b__subcmd__vm__subcmd__status"
                ;;
            d2b__subcmd__vm,stop)
                cmd="d2b__subcmd__vm__subcmd__stop"
                ;;
            d2b__subcmd__vm__subcmd__display,close)
                cmd="d2b__subcmd__vm__subcmd__display__subcmd__close"
                ;;
            d2b__subcmd__vm__subcmd__display,help)
                cmd="d2b__subcmd__vm__subcmd__display__subcmd__help"
                ;;
            d2b__subcmd__vm__subcmd__display,list)
                cmd="d2b__subcmd__vm__subcmd__display__subcmd__list"
                ;;
            d2b__subcmd__vm__subcmd__display__subcmd__help,close)
                cmd="d2b__subcmd__vm__subcmd__display__subcmd__help__subcmd__close"
                ;;
            d2b__subcmd__vm__subcmd__display__subcmd__help,help)
                cmd="d2b__subcmd__vm__subcmd__display__subcmd__help__subcmd__help"
                ;;
            d2b__subcmd__vm__subcmd__display__subcmd__help,list)
                cmd="d2b__subcmd__vm__subcmd__display__subcmd__help__subcmd__list"
                ;;
            d2b__subcmd__vm__subcmd__help,display)
                cmd="d2b__subcmd__vm__subcmd__help__subcmd__display"
                ;;
            d2b__subcmd__vm__subcmd__help,exec)
                cmd="d2b__subcmd__vm__subcmd__help__subcmd__exec"
                ;;
            d2b__subcmd__vm__subcmd__help,help)
                cmd="d2b__subcmd__vm__subcmd__help__subcmd__help"
                ;;
            d2b__subcmd__vm__subcmd__help,list)
                cmd="d2b__subcmd__vm__subcmd__help__subcmd__list"
                ;;
            d2b__subcmd__vm__subcmd__help,restart)
                cmd="d2b__subcmd__vm__subcmd__help__subcmd__restart"
                ;;
            d2b__subcmd__vm__subcmd__help,start)
                cmd="d2b__subcmd__vm__subcmd__help__subcmd__start"
                ;;
            d2b__subcmd__vm__subcmd__help,status)
                cmd="d2b__subcmd__vm__subcmd__help__subcmd__status"
                ;;
            d2b__subcmd__vm__subcmd__help,stop)
                cmd="d2b__subcmd__vm__subcmd__help__subcmd__stop"
                ;;
            d2b__subcmd__vm__subcmd__help__subcmd__display,close)
                cmd="d2b__subcmd__vm__subcmd__help__subcmd__display__subcmd__close"
                ;;
            d2b__subcmd__vm__subcmd__help__subcmd__display,list)
                cmd="d2b__subcmd__vm__subcmd__help__subcmd__display__subcmd__list"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        d2b)
            opts="-h -V --help --version list status launch usb console audio audit host auth realm shell op vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config clipboard help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__audio)
            opts="-h --json --help status mic speaker off help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__audio__subcmd__help)
            opts="status mic speaker off help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__audio__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__audio__subcmd__help__subcmd__mic)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__audio__subcmd__help__subcmd__off)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__audio__subcmd__help__subcmd__speaker)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__audio__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__audio__subcmd__mic)
            opts="-h --json --help on off <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__audio__subcmd__off)
            opts="-h --json --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__audio__subcmd__speaker)
            opts="-h --json --help on off <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__audio__subcmd__status)
            opts="-h --json --help [VM]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__audit)
            opts="-h --strict --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__auth)
            opts="-h --help status help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__auth__subcmd__help)
            opts="status help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__auth__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__auth__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__auth__subcmd__status)
            opts="-h --json --human --test-uid --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --test-uid)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__boot)
            opts="-h --dry-run --apply --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__build)
            opts="-h --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__clipboard)
            opts="-h --help arm help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__clipboard__subcmd__arm)
            opts="-h --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__clipboard__subcmd__help)
            opts="arm help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__clipboard__subcmd__help__subcmd__arm)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__clipboard__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__config)
            opts="-h --help sync diff approve reject status help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__config__subcmd__approve)
            opts="-h --to --json --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --to)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__config__subcmd__diff)
            opts="-h --against --json --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --against)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__config__subcmd__help)
            opts="sync diff approve reject status help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__config__subcmd__help__subcmd__approve)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__config__subcmd__help__subcmd__diff)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__config__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__config__subcmd__help__subcmd__reject)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__config__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__config__subcmd__help__subcmd__sync)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__config__subcmd__reject)
            opts="-h --json --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__config__subcmd__status)
            opts="-h --all --json --help [VM]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__config__subcmd__sync)
            opts="-h --guest-path --host --user --key --known-hosts --dry-run --json --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --guest-path)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --host)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --user)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --key)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --known-hosts)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__console)
            opts="-h --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__down)
            opts="-f -h --dry-run --apply --force --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__gc)
            opts="-h --dry-run --apply --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__generations)
            opts="-h --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help)
            opts="list status launch usb console audio audit host auth realm shell op vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config clipboard help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__audio)
            opts="status mic speaker off"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__audio__subcmd__mic)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__audio__subcmd__off)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__audio__subcmd__speaker)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__audio__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__audit)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__auth)
            opts="status"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__auth__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__boot)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__build)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__clipboard)
            opts="arm"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__clipboard__subcmd__arm)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__config)
            opts="sync diff approve reject status"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__config__subcmd__approve)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__config__subcmd__diff)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__config__subcmd__reject)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__config__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__config__subcmd__sync)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__console)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__down)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__gc)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__generations)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__host)
            opts="check prepare destroy doctor migrate-storage install reconcile validate"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__host__subcmd__check)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__host__subcmd__destroy)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__host__subcmd__doctor)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__host__subcmd__install)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__host__subcmd__migrate__subcmd__storage)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__host__subcmd__prepare)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__host__subcmd__reconcile)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__host__subcmd__validate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__keys)
            opts="list show rotate"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__keys__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__keys__subcmd__rotate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__keys__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__launch)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__migrate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__op)
            opts="inspect"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__op__subcmd__inspect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__realm)
            opts="list inspect enter run"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__realm__subcmd__enter)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__realm__subcmd__inspect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__realm__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__realm__subcmd__run)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__restart)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__rollback)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__rotate__subcmd__known__subcmd__host)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__shell)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__store)
            opts="verify"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__store__subcmd__verify)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__switch)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__test)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__trust)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__up)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__usb)
            opts="attach detach probe security-key"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__usb__subcmd__attach)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__usb__subcmd__detach)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__usb__subcmd__probe)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__usb__subcmd__security__subcmd__key)
            opts="status sessions cancel test"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__usb__subcmd__security__subcmd__key__subcmd__cancel)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__usb__subcmd__security__subcmd__key__subcmd__sessions)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__usb__subcmd__security__subcmd__key__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__usb__subcmd__security__subcmd__key__subcmd__test)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__vm)
            opts="start stop restart list status exec display"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__vm__subcmd__display)
            opts="list close"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__vm__subcmd__display__subcmd__close)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__vm__subcmd__display__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__vm__subcmd__exec)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__vm__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__vm__subcmd__restart)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__vm__subcmd__start)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__vm__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__help__subcmd__vm__subcmd__stop)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host)
            opts="-h --help check prepare destroy doctor migrate-storage install reconcile validate help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__check)
            opts="-h --read-only --strict --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__destroy)
            opts="-h --dry-run --apply --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__doctor)
            opts="-h --read-only --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__help)
            opts="check prepare destroy doctor migrate-storage install reconcile validate help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__help__subcmd__check)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__help__subcmd__destroy)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__help__subcmd__doctor)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__help__subcmd__install)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__help__subcmd__migrate__subcmd__storage)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__help__subcmd__prepare)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__help__subcmd__reconcile)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__help__subcmd__validate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__install)
            opts="-h --dry-run --apply --enable --start --no-start --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__migrate__subcmd__storage)
            opts="-h --dry-run --apply --rollback --from-checkpoint --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --from-checkpoint)
                    COMPREPLY=($(compgen -f "${cur}"))
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
            opts="-h --dry-run --apply --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__reconcile)
            opts="-h --network --dry-run --apply --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__host__subcmd__validate)
            opts="-h --dry-run --apply --wave --operator-signature --evidence-dir --scripts-dir --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --wave)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --operator-signature)
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
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__keys)
            opts="-h --help list show rotate help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__keys__subcmd__help)
            opts="list show rotate help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__keys__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__keys__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__keys__subcmd__help__subcmd__rotate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__keys__subcmd__help__subcmd__show)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__keys__subcmd__list)
            opts="-h --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__keys__subcmd__rotate)
            opts="-h --dry-run --apply --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__keys__subcmd__show)
            opts="-h --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__launch)
            opts="-h --item --json --human --help <TARGET>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --item)
                    COMPREPLY=($(compgen -f "${cur}"))
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
            opts="-h --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__migrate)
            opts="-h --dry-run --apply --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__op)
            opts="-h --help inspect help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__op__subcmd__help)
            opts="inspect help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__op__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__op__subcmd__help__subcmd__inspect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__op__subcmd__inspect)
            opts="-h --trace-id --span-id --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --trace-id)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --span-id)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__realm)
            opts="-h --help list inspect enter run help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__realm__subcmd__enter)
            opts="-h --help <REALM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__realm__subcmd__help)
            opts="list inspect enter run help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__realm__subcmd__help__subcmd__enter)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__realm__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__realm__subcmd__help__subcmd__inspect)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__realm__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__realm__subcmd__help__subcmd__run)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__realm__subcmd__inspect)
            opts="-h --json --human --help <REALM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__realm__subcmd__list)
            opts="-h --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__realm__subcmd__run)
            opts="-h --json --human --help <REALM> <ARGV>..."
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__restart)
            opts="-f -h --dry-run --apply --force --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__rollback)
            opts="-h --dry-run --apply --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__rotate__subcmd__known__subcmd__host)
            opts="-h --dry-run --apply --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__shell)
            opts="-h --name --force --json --human --help <TARGET> attach list detach kill"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --name)
                    COMPREPLY=($(compgen -f "${cur}"))
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
            opts="-h --json --human --check-bridges --vm --help [VM]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --vm)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__store)
            opts="-h --help verify help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__store__subcmd__help)
            opts="verify help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__store__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__store__subcmd__help__subcmd__verify)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__store__subcmd__verify)
            opts="-h --repair --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__switch)
            opts="-h --dry-run --apply --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__test)
            opts="-h --dry-run --apply --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__trust)
            opts="-h --dry-run --apply --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__up)
            opts="-h --dry-run --apply --no-wait-api --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb)
            opts="-h --help attach detach probe security-key help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__attach)
            opts="-h --dry-run --apply --json --human --help <VM> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__detach)
            opts="-h --dry-run --apply --json --human --help <VM> <BUSID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__help)
            opts="attach detach probe security-key help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__help__subcmd__attach)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__help__subcmd__detach)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__help__subcmd__probe)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__help__subcmd__security__subcmd__key)
            opts="status sessions cancel test"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__help__subcmd__security__subcmd__key__subcmd__cancel)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__help__subcmd__security__subcmd__key__subcmd__sessions)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__help__subcmd__security__subcmd__key__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__help__subcmd__security__subcmd__key__subcmd__test)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__probe)
            opts="-h --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__security__subcmd__key)
            opts="-h --help status sessions cancel test help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__cancel)
            opts="-h --current --dry-run --apply --json --human --help [SESSION_ID]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help)
            opts="status sessions cancel test help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help__subcmd__cancel)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help__subcmd__sessions)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__help__subcmd__test)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__sessions)
            opts="-h --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__status)
            opts="-h --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__usb__subcmd__security__subcmd__key__subcmd__test)
            opts="-h --dry-run --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm)
            opts="-h --help start stop restart list status exec display help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__display)
            opts="-h --help list close help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__display__subcmd__close)
            opts="-h --json --human --help <SESSION_ID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__display__subcmd__help)
            opts="list close help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__display__subcmd__help__subcmd__close)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__display__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__display__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__display__subcmd__list)
            opts="-h --target --json --human --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --target)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__exec)
            opts="-d -i -t -h --detach --interactive --tty --env --cwd --json --human --help <VM> [MANAGEMENT]... [COMMAND]..."
            if [[ " ${COMP_WORDS[*]} " == *" logs "* ]] ; then
                opts="${opts} --stdout-offset --stderr-offset --max-len"
            fi
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --env)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --cwd)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --stdout-offset|--stderr-offset|--max-len)
                    COMPREPLY=()
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__help)
            opts="start stop restart list status exec display help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__help__subcmd__display)
            opts="list close"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__help__subcmd__display__subcmd__close)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__help__subcmd__display__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__help__subcmd__exec)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__help__subcmd__restart)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__help__subcmd__start)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__help__subcmd__stop)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__list)
            opts="-h --json --human --realm --all --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --realm)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__restart)
            opts="-f -h --dry-run --apply --force --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__start)
            opts="-h --dry-run --apply --no-wait-api --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__status)
            opts="-h --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        d2b__subcmd__vm__subcmd__stop)
            opts="-f -h --dry-run --apply --force --json --human --help <VM>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
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
