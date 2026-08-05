# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_d2b_global_optspecs
	string join \n zone= json human deadline= no-deadline h/help V/version
end

function __fish_d2b_needs_command
	# Figure out if the current invocation already has a command.
	set -l cmd (commandline -opc)
	set -e cmd[1]
	argparse -s (__fish_d2b_global_optspecs) -- $cmd 2>/dev/null
	or return
	if set -q argv[1]
		# Also print the command, so this can be used to figure out what it is.
		echo $argv[1]
		return 1
	end
	return 0
end

function __fish_d2b_using_subcommand
	set -l cmd (__fish_d2b_needs_command)
	test -z "$cmd"
	and return 1
	contains -- $cmd[1] $argv
end

complete -c d2b -n "__fish_d2b_needs_command" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_needs_command" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_needs_command" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_needs_command" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_needs_command" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_needs_command" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_needs_command" -s V -l version -d 'Print version'
complete -c d2b -n "__fish_d2b_needs_command" -f -a "get"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "list"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "watch"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "create"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "update-spec"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "delete"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "status"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "upgrade"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "reconcile"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "host"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "guest"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "process"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "exec"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "shell"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "volume" -d 'Typed noun commands reuse the generic resource verbs while documenting the noun\'s default ResourceType in clap help'
complete -c d2b -n "__fish_d2b_needs_command" -f -a "network" -d 'Typed noun commands reuse the generic resource verbs while documenting the noun\'s default ResourceType in clap help'
complete -c d2b -n "__fish_d2b_needs_command" -f -a "device" -d 'Typed noun commands reuse the generic resource verbs while documenting the noun\'s default ResourceType in clap help'
complete -c d2b -n "__fish_d2b_needs_command" -f -a "endpoint"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "export"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "import"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "resource" -d 'The `d2b resource` namespace also carries the generic authority read projection. It never creates or mutates an authority'
complete -c d2b -n "__fish_d2b_needs_command" -f -a "user" -d 'Typed noun commands reuse the generic resource verbs while documenting the noun\'s default ResourceType in clap help'
complete -c d2b -n "__fish_d2b_needs_command" -f -a "credential" -d 'Typed noun commands reuse the generic resource verbs while documenting the noun\'s default ResourceType in clap help'
complete -c d2b -n "__fish_d2b_needs_command" -f -a "provider"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "zone"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "quota" -d 'Typed noun commands reuse the generic resource verbs while documenting the noun\'s default ResourceType in clap help'
complete -c d2b -n "__fish_d2b_needs_command" -f -a "emergency-policy" -d 'Typed noun commands reuse the generic resource verbs while documenting the noun\'s default ResourceType in clap help'
complete -c d2b -n "__fish_d2b_needs_command" -f -a "activation"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "audit"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "op"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "auth"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "complete"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "audio"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "clipboard"
complete -c d2b -n "__fish_d2b_needs_command" -f -a "display"
complete -c d2b -n "__fish_d2b_using_subcommand get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand list" -l execution-ref -r
complete -c d2b -n "__fish_d2b_using_subcommand list" -l domain -r
complete -c d2b -n "__fish_d2b_using_subcommand list" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand list" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand list" -l page-token -r
complete -c d2b -n "__fish_d2b_using_subcommand list" -l limit -r
complete -c d2b -n "__fish_d2b_using_subcommand list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand list" -l updates
complete -c d2b -n "__fish_d2b_using_subcommand list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand watch" -l since-revision -r
complete -c d2b -n "__fish_d2b_using_subcommand watch" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand watch" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand watch" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand watch" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand watch" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand watch" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand watch" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand watch" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand create" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand create" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand create" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand create" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand create" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand create" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand create" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand create" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand create" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand create" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand update-spec" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand update-spec" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand update-spec" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand update-spec" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand update-spec" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand update-spec" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand update-spec" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand update-spec" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand update-spec" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand update-spec" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand update-spec" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand delete" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand delete" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand delete" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand upgrade" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand upgrade" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand upgrade" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand upgrade" -l recursive
complete -c d2b -n "__fish_d2b_using_subcommand upgrade" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand upgrade" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand upgrade" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand upgrade" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand upgrade" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand reconcile" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand reconcile" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand reconcile" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand reconcile" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand reconcile" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand reconcile" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand reconcile" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -f -a "check"
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -f -a "prepare"
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -f -a "destroy"
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -f -a "doctor"
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -f -a "install"
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -f -a "reconcile"
complete -c d2b -n "__fish_d2b_using_subcommand host; and not __fish_seen_subcommand_from get list status check prepare destroy doctor install reconcile validate" -f -a "validate"
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from list" -l execution-ref -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from list" -l domain -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from list" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from list" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from list" -l page-token -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from list" -l limit -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from list" -l updates
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from check" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from check" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from check" -l read-only
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from check" -l strict
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from check" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from check" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from check" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from check" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from prepare" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from prepare" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from prepare" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from prepare" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from prepare" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from prepare" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from prepare" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from prepare" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from destroy" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from destroy" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from destroy" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from destroy" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from destroy" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from destroy" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from destroy" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from destroy" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from doctor" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from doctor" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from doctor" -l read-only
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from doctor" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from doctor" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from doctor" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from doctor" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from install" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from install" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from install" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from install" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from install" -l enable
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from install" -l start
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from install" -l no-start
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from install" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from install" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from install" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from install" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from reconcile" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from reconcile" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from reconcile" -l network
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from reconcile" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from reconcile" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from reconcile" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from reconcile" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from reconcile" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from reconcile" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from validate" -l wave -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from validate" -l evidence-dir -r -F
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from validate" -l scripts-dir -r -F
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from validate" -l operator-signature -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from validate" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from validate" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from validate" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from validate" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from validate" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from validate" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from validate" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand host; and __fish_seen_subcommand_from validate" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -f -a "start"
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -f -a "stop"
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -f -a "restart"
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -f -a "create"
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -f -a "update-spec"
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -f -a "delete"
complete -c d2b -n "__fish_d2b_using_subcommand guest; and not __fish_seen_subcommand_from get list status start stop restart create update-spec delete console" -f -a "console"
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from list" -l execution-ref -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from list" -l domain -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from list" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from list" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from list" -l page-token -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from list" -l limit -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from list" -l updates
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from start" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from start" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from start" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from start" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from start" -l no-wait-ready
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from start" -s f -l force
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from start" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from start" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from start" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from start" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from stop" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from stop" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from stop" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from stop" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from stop" -l no-wait-ready
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from stop" -s f -l force
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from stop" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from stop" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from stop" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from stop" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from restart" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from restart" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from restart" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from restart" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from restart" -l no-wait-ready
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from restart" -s f -l force
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from restart" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from restart" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from restart" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from restart" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from create" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from create" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from create" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from create" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from create" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from create" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from create" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from create" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from create" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from create" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from update-spec" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from update-spec" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from update-spec" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from update-spec" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from update-spec" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from update-spec" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from update-spec" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from update-spec" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from update-spec" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from update-spec" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from update-spec" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from delete" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from delete" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from delete" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from console" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from console" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from console" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from console" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from console" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand guest; and __fish_seen_subcommand_from console" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand process; and not __fish_seen_subcommand_from get list status start stop create update-spec delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and not __fish_seen_subcommand_from get list status start stop create update-spec delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and not __fish_seen_subcommand_from get list status start stop create update-spec delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand process; and not __fish_seen_subcommand_from get list status start stop create update-spec delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand process; and not __fish_seen_subcommand_from get list status start stop create update-spec delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand process; and not __fish_seen_subcommand_from get list status start stop create update-spec delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand process; and not __fish_seen_subcommand_from get list status start stop create update-spec delete" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand process; and not __fish_seen_subcommand_from get list status start stop create update-spec delete" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand process; and not __fish_seen_subcommand_from get list status start stop create update-spec delete" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand process; and not __fish_seen_subcommand_from get list status start stop create update-spec delete" -f -a "start"
complete -c d2b -n "__fish_d2b_using_subcommand process; and not __fish_seen_subcommand_from get list status start stop create update-spec delete" -f -a "stop"
complete -c d2b -n "__fish_d2b_using_subcommand process; and not __fish_seen_subcommand_from get list status start stop create update-spec delete" -f -a "create"
complete -c d2b -n "__fish_d2b_using_subcommand process; and not __fish_seen_subcommand_from get list status start stop create update-spec delete" -f -a "update-spec"
complete -c d2b -n "__fish_d2b_using_subcommand process; and not __fish_seen_subcommand_from get list status start stop create update-spec delete" -f -a "delete"
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from list" -l execution-ref -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from list" -l domain -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from list" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from list" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from list" -l page-token -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from list" -l limit -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from list" -l updates
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from start" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from start" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from start" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from start" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from start" -l no-wait-ready
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from start" -s f -l force
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from start" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from start" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from start" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from start" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from stop" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from stop" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from stop" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from stop" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from stop" -l no-wait-ready
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from stop" -s f -l force
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from stop" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from stop" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from stop" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from stop" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from create" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from create" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from create" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from create" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from create" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from create" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from create" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from create" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from create" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from create" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from update-spec" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from update-spec" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from update-spec" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from update-spec" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from update-spec" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from update-spec" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from update-spec" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from update-spec" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from update-spec" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from update-spec" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from update-spec" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from delete" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from delete" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from delete" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand process; and __fish_seen_subcommand_from delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and not __fish_seen_subcommand_from run attach wait status list logs kill" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and not __fish_seen_subcommand_from run attach wait status list logs kill" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and not __fish_seen_subcommand_from run attach wait status list logs kill" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and not __fish_seen_subcommand_from run attach wait status list logs kill" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and not __fish_seen_subcommand_from run attach wait status list logs kill" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and not __fish_seen_subcommand_from run attach wait status list logs kill" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and not __fish_seen_subcommand_from run attach wait status list logs kill" -f -a "run"
complete -c d2b -n "__fish_d2b_using_subcommand exec; and not __fish_seen_subcommand_from run attach wait status list logs kill" -f -a "attach"
complete -c d2b -n "__fish_d2b_using_subcommand exec; and not __fish_seen_subcommand_from run attach wait status list logs kill" -f -a "wait"
complete -c d2b -n "__fish_d2b_using_subcommand exec; and not __fish_seen_subcommand_from run attach wait status list logs kill" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand exec; and not __fish_seen_subcommand_from run attach wait status list logs kill" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand exec; and not __fish_seen_subcommand_from run attach wait status list logs kill" -f -a "logs"
complete -c d2b -n "__fish_d2b_using_subcommand exec; and not __fish_seen_subcommand_from run attach wait status list logs kill" -f -a "kill"
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from run" -l name -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from run" -l domain -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from run" -l user -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from run" -l provider -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from run" -l env -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from run" -l cwd -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from run" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from run" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from run" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from run" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from run" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from run" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from attach" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from attach" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from attach" -s i -l interactive
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from attach" -s t -l tty
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from attach" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from attach" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from attach" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from attach" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from wait" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from wait" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from wait" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from wait" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from wait" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from wait" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from list" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from logs" -l stdout-offset -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from logs" -l stderr-offset -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from logs" -l max-len -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from logs" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from logs" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from logs" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from logs" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from logs" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from logs" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from kill" -l signal -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from kill" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from kill" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from kill" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from kill" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from kill" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand exec; and __fish_seen_subcommand_from kill" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and not __fish_seen_subcommand_from open attach list detach kill status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and not __fish_seen_subcommand_from open attach list detach kill status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and not __fish_seen_subcommand_from open attach list detach kill status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and not __fish_seen_subcommand_from open attach list detach kill status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and not __fish_seen_subcommand_from open attach list detach kill status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and not __fish_seen_subcommand_from open attach list detach kill status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and not __fish_seen_subcommand_from open attach list detach kill status" -f -a "open"
complete -c d2b -n "__fish_d2b_using_subcommand shell; and not __fish_seen_subcommand_from open attach list detach kill status" -f -a "attach"
complete -c d2b -n "__fish_d2b_using_subcommand shell; and not __fish_seen_subcommand_from open attach list detach kill status" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand shell; and not __fish_seen_subcommand_from open attach list detach kill status" -f -a "detach"
complete -c d2b -n "__fish_d2b_using_subcommand shell; and not __fish_seen_subcommand_from open attach list detach kill status" -f -a "kill"
complete -c d2b -n "__fish_d2b_using_subcommand shell; and not __fish_seen_subcommand_from open attach list detach kill status" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from open" -l name -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from open" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from open" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from open" -l force
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from open" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from open" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from open" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from open" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from attach" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from attach" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from attach" -l force
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from attach" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from attach" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from attach" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from attach" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from detach" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from detach" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from detach" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from detach" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from detach" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from detach" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from kill" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from kill" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from kill" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from kill" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from kill" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from kill" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand shell; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "watch"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "create"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "update-spec"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "delete"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "upgrade"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "reconcile"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "verify"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "usb"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "security-key"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from list" -l execution-ref -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from list" -l domain -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from list" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from list" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from list" -l page-token -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from list" -l limit -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from list" -l updates
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from watch" -l since-revision -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from watch" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from watch" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from watch" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from watch" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from watch" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from watch" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from watch" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from watch" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from create" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from create" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from create" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from create" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from create" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from create" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from create" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from create" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from create" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from create" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from update-spec" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from update-spec" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from update-spec" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from update-spec" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from update-spec" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from update-spec" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from update-spec" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from update-spec" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from update-spec" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from update-spec" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from update-spec" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from delete" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from delete" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from delete" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from upgrade" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from upgrade" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from upgrade" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from upgrade" -l recursive
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from upgrade" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from upgrade" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from upgrade" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from upgrade" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from upgrade" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from reconcile" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from reconcile" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from reconcile" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from reconcile" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from reconcile" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from reconcile" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from reconcile" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from verify" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from verify" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from verify" -l repair
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from verify" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from verify" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from verify" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from verify" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from usb" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from usb" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from usb" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from usb" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from usb" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from usb" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from usb" -f -a "attach"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from usb" -f -a "detach"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from usb" -f -a "probe"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from security-key" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from security-key" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from security-key" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from security-key" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from security-key" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from security-key" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from security-key" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from security-key" -f -a "sessions"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from security-key" -f -a "cancel"
complete -c d2b -n "__fish_d2b_using_subcommand volume; and __fish_seen_subcommand_from security-key" -f -a "test"
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "watch"
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "create"
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "update-spec"
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "delete"
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "upgrade"
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "reconcile"
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "verify"
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "usb"
complete -c d2b -n "__fish_d2b_using_subcommand network; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "security-key"
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from list" -l execution-ref -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from list" -l domain -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from list" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from list" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from list" -l page-token -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from list" -l limit -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from list" -l updates
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from watch" -l since-revision -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from watch" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from watch" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from watch" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from watch" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from watch" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from watch" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from watch" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from watch" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from create" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from create" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from create" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from create" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from create" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from create" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from create" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from create" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from create" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from create" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from update-spec" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from update-spec" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from update-spec" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from update-spec" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from update-spec" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from update-spec" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from update-spec" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from update-spec" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from update-spec" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from update-spec" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from update-spec" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from delete" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from delete" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from delete" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from upgrade" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from upgrade" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from upgrade" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from upgrade" -l recursive
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from upgrade" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from upgrade" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from upgrade" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from upgrade" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from upgrade" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from reconcile" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from reconcile" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from reconcile" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from reconcile" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from reconcile" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from reconcile" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from reconcile" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from verify" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from verify" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from verify" -l repair
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from verify" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from verify" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from verify" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from verify" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from usb" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from usb" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from usb" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from usb" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from usb" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from usb" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from usb" -f -a "attach"
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from usb" -f -a "detach"
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from usb" -f -a "probe"
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from security-key" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from security-key" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from security-key" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from security-key" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from security-key" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from security-key" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from security-key" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from security-key" -f -a "sessions"
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from security-key" -f -a "cancel"
complete -c d2b -n "__fish_d2b_using_subcommand network; and __fish_seen_subcommand_from security-key" -f -a "test"
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "watch"
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "create"
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "update-spec"
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "delete"
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "upgrade"
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "reconcile"
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "verify"
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "usb"
complete -c d2b -n "__fish_d2b_using_subcommand device; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "security-key"
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from list" -l execution-ref -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from list" -l domain -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from list" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from list" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from list" -l page-token -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from list" -l limit -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from list" -l updates
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from watch" -l since-revision -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from watch" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from watch" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from watch" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from watch" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from watch" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from watch" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from watch" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from watch" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from create" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from create" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from create" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from create" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from create" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from create" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from create" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from create" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from create" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from create" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from update-spec" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from update-spec" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from update-spec" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from update-spec" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from update-spec" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from update-spec" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from update-spec" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from update-spec" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from update-spec" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from update-spec" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from update-spec" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from delete" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from delete" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from delete" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from upgrade" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from upgrade" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from upgrade" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from upgrade" -l recursive
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from upgrade" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from upgrade" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from upgrade" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from upgrade" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from upgrade" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from reconcile" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from reconcile" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from reconcile" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from reconcile" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from reconcile" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from reconcile" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from reconcile" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from verify" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from verify" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from verify" -l repair
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from verify" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from verify" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from verify" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from verify" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from usb" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from usb" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from usb" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from usb" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from usb" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from usb" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from usb" -f -a "attach"
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from usb" -f -a "detach"
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from usb" -f -a "probe"
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from security-key" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from security-key" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from security-key" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from security-key" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from security-key" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from security-key" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from security-key" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from security-key" -f -a "sessions"
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from security-key" -f -a "cancel"
complete -c d2b -n "__fish_d2b_using_subcommand device; and __fish_seen_subcommand_from security-key" -f -a "test"
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and not __fish_seen_subcommand_from get list watch status resolve" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and not __fish_seen_subcommand_from get list watch status resolve" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and not __fish_seen_subcommand_from get list watch status resolve" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and not __fish_seen_subcommand_from get list watch status resolve" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and not __fish_seen_subcommand_from get list watch status resolve" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and not __fish_seen_subcommand_from get list watch status resolve" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and not __fish_seen_subcommand_from get list watch status resolve" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and not __fish_seen_subcommand_from get list watch status resolve" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and not __fish_seen_subcommand_from get list watch status resolve" -f -a "watch"
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and not __fish_seen_subcommand_from get list watch status resolve" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and not __fish_seen_subcommand_from get list watch status resolve" -f -a "resolve"
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from list" -l endpoint-class -r
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from list" -l updates
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from watch" -l endpoint-class -r
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from watch" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from watch" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from watch" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from watch" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from watch" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from watch" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from resolve" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from resolve" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from resolve" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from resolve" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from resolve" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand endpoint; and __fish_seen_subcommand_from resolve" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand export; and not __fish_seen_subcommand_from get list watch status create update-spec delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and not __fish_seen_subcommand_from get list watch status create update-spec delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and not __fish_seen_subcommand_from get list watch status create update-spec delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand export; and not __fish_seen_subcommand_from get list watch status create update-spec delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand export; and not __fish_seen_subcommand_from get list watch status create update-spec delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand export; and not __fish_seen_subcommand_from get list watch status create update-spec delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand export; and not __fish_seen_subcommand_from get list watch status create update-spec delete" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand export; and not __fish_seen_subcommand_from get list watch status create update-spec delete" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand export; and not __fish_seen_subcommand_from get list watch status create update-spec delete" -f -a "watch"
complete -c d2b -n "__fish_d2b_using_subcommand export; and not __fish_seen_subcommand_from get list watch status create update-spec delete" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand export; and not __fish_seen_subcommand_from get list watch status create update-spec delete" -f -a "create"
complete -c d2b -n "__fish_d2b_using_subcommand export; and not __fish_seen_subcommand_from get list watch status create update-spec delete" -f -a "update-spec"
complete -c d2b -n "__fish_d2b_using_subcommand export; and not __fish_seen_subcommand_from get list watch status create update-spec delete" -f -a "delete"
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from list" -l exported-type -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from watch" -l exported-type -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from watch" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from watch" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from watch" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from watch" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from watch" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from watch" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from create" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from create" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from create" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from create" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from create" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from create" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from create" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from create" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from update-spec" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from update-spec" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from update-spec" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from update-spec" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from update-spec" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from update-spec" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from update-spec" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from update-spec" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from update-spec" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from delete" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand export; and __fish_seen_subcommand_from delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -f -a "watch"
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -f -a "projection"
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -f -a "graph"
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -f -a "create"
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -f -a "update-spec"
complete -c d2b -n "__fish_d2b_using_subcommand import; and not __fish_seen_subcommand_from get list watch status projection graph create update-spec delete" -f -a "delete"
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from list" -l expected-type -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from watch" -l expected-type -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from watch" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from watch" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from watch" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from watch" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from watch" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from watch" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from projection" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from projection" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from projection" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from projection" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from projection" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from projection" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from graph" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from graph" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from graph" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from graph" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from graph" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from graph" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from create" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from create" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from create" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from create" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from create" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from create" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from create" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from create" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from update-spec" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from update-spec" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from update-spec" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from update-spec" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from update-spec" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from update-spec" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from update-spec" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from update-spec" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from update-spec" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from delete" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand import; and __fish_seen_subcommand_from delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -f -a "watch"
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -f -a "create"
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -f -a "update-spec"
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -f -a "delete"
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -f -a "upgrade"
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -f -a "reconcile"
complete -c d2b -n "__fish_d2b_using_subcommand resource; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile authorities" -f -a "authorities"
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from list" -l execution-ref -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from list" -l domain -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from list" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from list" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from list" -l page-token -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from list" -l limit -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from list" -l updates
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from watch" -l since-revision -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from watch" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from watch" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from watch" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from watch" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from watch" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from watch" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from watch" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from watch" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from create" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from create" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from create" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from create" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from create" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from create" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from create" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from create" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from create" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from create" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from update-spec" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from update-spec" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from update-spec" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from update-spec" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from update-spec" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from update-spec" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from update-spec" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from update-spec" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from update-spec" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from update-spec" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from update-spec" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from delete" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from delete" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from delete" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from upgrade" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from upgrade" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from upgrade" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from upgrade" -l recursive
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from upgrade" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from upgrade" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from upgrade" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from upgrade" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from upgrade" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from reconcile" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from reconcile" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from reconcile" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from reconcile" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from reconcile" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from reconcile" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from reconcile" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from authorities" -l scope -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from authorities" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from authorities" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from authorities" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from authorities" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from authorities" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from authorities" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from authorities" -f -a "holders"
complete -c d2b -n "__fish_d2b_using_subcommand resource; and __fish_seen_subcommand_from authorities" -f -a "conflict"
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "watch"
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "create"
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "update-spec"
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "delete"
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "upgrade"
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "reconcile"
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "verify"
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "usb"
complete -c d2b -n "__fish_d2b_using_subcommand user; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "security-key"
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from list" -l execution-ref -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from list" -l domain -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from list" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from list" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from list" -l page-token -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from list" -l limit -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from list" -l updates
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from watch" -l since-revision -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from watch" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from watch" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from watch" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from watch" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from watch" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from watch" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from watch" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from watch" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from create" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from create" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from create" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from create" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from create" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from create" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from create" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from create" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from create" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from create" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from update-spec" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from update-spec" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from update-spec" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from update-spec" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from update-spec" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from update-spec" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from update-spec" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from update-spec" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from update-spec" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from update-spec" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from update-spec" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from delete" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from delete" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from delete" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from upgrade" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from upgrade" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from upgrade" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from upgrade" -l recursive
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from upgrade" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from upgrade" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from upgrade" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from upgrade" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from upgrade" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from reconcile" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from reconcile" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from reconcile" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from reconcile" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from reconcile" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from reconcile" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from reconcile" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from verify" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from verify" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from verify" -l repair
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from verify" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from verify" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from verify" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from verify" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from usb" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from usb" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from usb" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from usb" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from usb" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from usb" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from usb" -f -a "attach"
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from usb" -f -a "detach"
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from usb" -f -a "probe"
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from security-key" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from security-key" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from security-key" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from security-key" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from security-key" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from security-key" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from security-key" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from security-key" -f -a "sessions"
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from security-key" -f -a "cancel"
complete -c d2b -n "__fish_d2b_using_subcommand user; and __fish_seen_subcommand_from security-key" -f -a "test"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "watch"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "create"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "update-spec"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "delete"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "upgrade"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "reconcile"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "verify"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "usb"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "security-key"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from list" -l execution-ref -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from list" -l domain -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from list" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from list" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from list" -l page-token -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from list" -l limit -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from list" -l updates
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from watch" -l since-revision -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from watch" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from watch" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from watch" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from watch" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from watch" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from watch" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from watch" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from watch" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from create" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from create" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from create" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from create" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from create" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from create" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from create" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from create" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from create" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from create" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from update-spec" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from update-spec" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from update-spec" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from update-spec" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from update-spec" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from update-spec" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from update-spec" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from update-spec" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from update-spec" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from update-spec" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from update-spec" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from delete" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from delete" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from delete" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from upgrade" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from upgrade" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from upgrade" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from upgrade" -l recursive
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from upgrade" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from upgrade" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from upgrade" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from upgrade" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from upgrade" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from reconcile" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from reconcile" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from reconcile" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from reconcile" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from reconcile" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from reconcile" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from reconcile" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from verify" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from verify" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from verify" -l repair
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from verify" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from verify" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from verify" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from verify" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from usb" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from usb" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from usb" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from usb" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from usb" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from usb" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from usb" -f -a "attach"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from usb" -f -a "detach"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from usb" -f -a "probe"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from security-key" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from security-key" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from security-key" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from security-key" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from security-key" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from security-key" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from security-key" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from security-key" -f -a "sessions"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from security-key" -f -a "cancel"
complete -c d2b -n "__fish_d2b_using_subcommand credential; and __fish_seen_subcommand_from security-key" -f -a "test"
complete -c d2b -n "__fish_d2b_using_subcommand provider; and not __fish_seen_subcommand_from list get status inspect" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand provider; and not __fish_seen_subcommand_from list get status inspect" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand provider; and not __fish_seen_subcommand_from list get status inspect" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and not __fish_seen_subcommand_from list get status inspect" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and not __fish_seen_subcommand_from list get status inspect" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and not __fish_seen_subcommand_from list get status inspect" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and not __fish_seen_subcommand_from list get status inspect" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand provider; and not __fish_seen_subcommand_from list get status inspect" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand provider; and not __fish_seen_subcommand_from list get status inspect" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand provider; and not __fish_seen_subcommand_from list get status inspect" -f -a "inspect"
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from list" -l package-only
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from inspect" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from inspect" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from inspect" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from inspect" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from inspect" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand provider; and __fish_seen_subcommand_from inspect" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and not __fish_seen_subcommand_from get list status audit doctor support-bundle" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand zone; and not __fish_seen_subcommand_from get list status audit doctor support-bundle" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand zone; and not __fish_seen_subcommand_from get list status audit doctor support-bundle" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and not __fish_seen_subcommand_from get list status audit doctor support-bundle" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and not __fish_seen_subcommand_from get list status audit doctor support-bundle" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and not __fish_seen_subcommand_from get list status audit doctor support-bundle" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and not __fish_seen_subcommand_from get list status audit doctor support-bundle" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand zone; and not __fish_seen_subcommand_from get list status audit doctor support-bundle" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand zone; and not __fish_seen_subcommand_from get list status audit doctor support-bundle" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand zone; and not __fish_seen_subcommand_from get list status audit doctor support-bundle" -f -a "audit"
complete -c d2b -n "__fish_d2b_using_subcommand zone; and not __fish_seen_subcommand_from get list status audit doctor support-bundle" -f -a "doctor" -d 'Arguments for `d2b zone doctor`'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and not __fish_seen_subcommand_from get list status audit doctor support-bundle" -f -a "support-bundle" -d 'Arguments for `d2b zone support-bundle`'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from audit" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from audit" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from audit" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from audit" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from audit" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from audit" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from audit" -f -a "export" -d 'Arguments for `d2b zone audit export`'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from doctor" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from doctor" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from doctor" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from doctor" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from doctor" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from doctor" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from support-bundle" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from support-bundle" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from support-bundle" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from support-bundle" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from support-bundle" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand zone; and __fish_seen_subcommand_from support-bundle" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "watch"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "create"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "update-spec"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "delete"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "upgrade"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "reconcile"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "verify"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "usb"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "security-key"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from list" -l execution-ref -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from list" -l domain -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from list" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from list" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from list" -l page-token -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from list" -l limit -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from list" -l updates
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from watch" -l since-revision -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from watch" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from watch" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from watch" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from watch" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from watch" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from watch" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from watch" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from watch" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from create" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from create" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from create" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from create" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from create" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from create" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from create" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from create" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from create" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from create" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from update-spec" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from update-spec" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from update-spec" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from update-spec" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from update-spec" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from update-spec" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from update-spec" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from update-spec" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from update-spec" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from update-spec" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from update-spec" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from delete" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from delete" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from delete" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from upgrade" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from upgrade" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from upgrade" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from upgrade" -l recursive
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from upgrade" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from upgrade" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from upgrade" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from upgrade" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from upgrade" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from reconcile" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from reconcile" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from reconcile" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from reconcile" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from reconcile" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from reconcile" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from reconcile" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from verify" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from verify" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from verify" -l repair
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from verify" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from verify" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from verify" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from verify" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from usb" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from usb" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from usb" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from usb" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from usb" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from usb" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from usb" -f -a "attach"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from usb" -f -a "detach"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from usb" -f -a "probe"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from security-key" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from security-key" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from security-key" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from security-key" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from security-key" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from security-key" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from security-key" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from security-key" -f -a "sessions"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from security-key" -f -a "cancel"
complete -c d2b -n "__fish_d2b_using_subcommand quota; and __fish_seen_subcommand_from security-key" -f -a "test"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "get"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "watch"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "create"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "update-spec"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "delete"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "upgrade"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "reconcile"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "verify"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "usb"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and not __fish_seen_subcommand_from get list watch create update-spec delete status upgrade reconcile verify usb security-key" -f -a "security-key"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from get" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from get" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from get" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from get" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from get" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from get" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from list" -l execution-ref -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from list" -l domain -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from list" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from list" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from list" -l page-token -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from list" -l limit -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from list" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from list" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from list" -l updates
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from list" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from list" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from list" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from watch" -l since-revision -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from watch" -l phase -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from watch" -l label-selector -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from watch" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from watch" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from watch" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from watch" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from watch" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from watch" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from create" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from create" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from create" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from create" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from create" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from create" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from create" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from create" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from create" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from create" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from update-spec" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from update-spec" -l spec-file -r -F
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from update-spec" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from update-spec" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from update-spec" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from update-spec" -l spec-stdin
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from update-spec" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from update-spec" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from update-spec" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from update-spec" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from update-spec" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from delete" -l revision -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from delete" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from delete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from delete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from delete" -l wait-for-reconcile
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from delete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from delete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from delete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from delete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from status" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from upgrade" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from upgrade" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from upgrade" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from upgrade" -l recursive
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from upgrade" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from upgrade" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from upgrade" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from upgrade" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from upgrade" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from reconcile" -l reconcile-deadline -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from reconcile" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from reconcile" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from reconcile" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from reconcile" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from reconcile" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from reconcile" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from verify" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from verify" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from verify" -l repair
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from verify" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from verify" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from verify" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from verify" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from usb" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from usb" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from usb" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from usb" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from usb" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from usb" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from usb" -f -a "attach"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from usb" -f -a "detach"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from usb" -f -a "probe"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from security-key" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from security-key" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from security-key" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from security-key" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from security-key" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from security-key" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from security-key" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from security-key" -f -a "sessions"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from security-key" -f -a "cancel"
complete -c d2b -n "__fish_d2b_using_subcommand emergency-policy; and __fish_seen_subcommand_from security-key" -f -a "test"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -f -a "apply"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -f -a "build"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -f -a "generations"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -f -a "switch"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -f -a "boot"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -f -a "test"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -f -a "rollback"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -f -a "gc"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -f -a "migrate"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -f -a "keys"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -f -a "trust"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -f -a "rotate-known-host"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and not __fish_seen_subcommand_from apply build generations switch boot test rollback gc migrate keys trust rotate-known-host config" -f -a "config"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from apply" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from apply" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from apply" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from apply" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from apply" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from apply" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from apply" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from build" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from build" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from build" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from build" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from build" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from build" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from generations" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from generations" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from generations" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from generations" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from generations" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from generations" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from switch" -l to-generation -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from switch" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from switch" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from switch" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from switch" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from switch" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from switch" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from switch" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from switch" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from boot" -l to-generation -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from boot" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from boot" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from boot" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from boot" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from boot" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from boot" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from boot" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from boot" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from test" -l to-generation -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from test" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from test" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from test" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from test" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from test" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from test" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from test" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from test" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rollback" -l to-generation -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rollback" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rollback" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rollback" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rollback" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rollback" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rollback" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rollback" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rollback" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from gc" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from gc" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from gc" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from gc" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from gc" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from gc" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from gc" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from gc" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from migrate" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from migrate" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from migrate" -l dry-run
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from migrate" -l apply
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from migrate" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from migrate" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from migrate" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from migrate" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from keys" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from keys" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from keys" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from keys" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from keys" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from keys" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from keys" -f -a "list"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from keys" -f -a "show"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from keys" -f -a "rotate"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from trust" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from trust" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from trust" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from trust" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from trust" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from trust" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rotate-known-host" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rotate-known-host" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rotate-known-host" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rotate-known-host" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rotate-known-host" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from rotate-known-host" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from config" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from config" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from config" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from config" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from config" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from config" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from config" -f -a "sync"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from config" -f -a "diff"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from config" -f -a "approve"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from config" -f -a "reject"
complete -c d2b -n "__fish_d2b_using_subcommand activation; and __fish_seen_subcommand_from config" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand audit" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand audit" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand audit" -l strict
complete -c d2b -n "__fish_d2b_using_subcommand audit" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand audit" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand audit" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand audit" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand op; and not __fish_seen_subcommand_from inspect" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand op; and not __fish_seen_subcommand_from inspect" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand op; and not __fish_seen_subcommand_from inspect" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand op; and not __fish_seen_subcommand_from inspect" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand op; and not __fish_seen_subcommand_from inspect" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand op; and not __fish_seen_subcommand_from inspect" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand op; and not __fish_seen_subcommand_from inspect" -f -a "inspect"
complete -c d2b -n "__fish_d2b_using_subcommand op; and __fish_seen_subcommand_from inspect" -l operation-id -r
complete -c d2b -n "__fish_d2b_using_subcommand op; and __fish_seen_subcommand_from inspect" -l trace-id -r
complete -c d2b -n "__fish_d2b_using_subcommand op; and __fish_seen_subcommand_from inspect" -l span-id -r
complete -c d2b -n "__fish_d2b_using_subcommand op; and __fish_seen_subcommand_from inspect" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand op; and __fish_seen_subcommand_from inspect" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand op; and __fish_seen_subcommand_from inspect" -l watch
complete -c d2b -n "__fish_d2b_using_subcommand op; and __fish_seen_subcommand_from inspect" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand op; and __fish_seen_subcommand_from inspect" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand op; and __fish_seen_subcommand_from inspect" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand op; and __fish_seen_subcommand_from inspect" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand auth; and not __fish_seen_subcommand_from status" -l test-uid -d 'Test-only identity override retained as a hidden fixture seam' -r
complete -c d2b -n "__fish_d2b_using_subcommand auth; and not __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand auth; and not __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand auth; and not __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand auth; and not __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand auth; and not __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand auth; and not __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand auth; and not __fish_seen_subcommand_from status" -f -a "status"
complete -c d2b -n "__fish_d2b_using_subcommand auth; and __fish_seen_subcommand_from status" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand auth; and __fish_seen_subcommand_from status" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand auth; and __fish_seen_subcommand_from status" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand auth; and __fish_seen_subcommand_from status" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand auth; and __fish_seen_subcommand_from status" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand auth; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand complete" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand complete" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand complete" -l list-commands
complete -c d2b -n "__fish_d2b_using_subcommand complete" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand complete" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand complete" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand complete" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand audio" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand audio" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand audio" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand audio" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand audio" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand audio" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand clipboard" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand clipboard" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand clipboard" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand clipboard" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand clipboard" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand clipboard" -s h -l help -d 'Print help'
complete -c d2b -n "__fish_d2b_using_subcommand display" -l zone -d 'Address a declared Zone. Without this flag the nearest local runtime is selected' -r
complete -c d2b -n "__fish_d2b_using_subcommand display" -l deadline -d 'Bound all Zone requests and streams' -r
complete -c d2b -n "__fish_d2b_using_subcommand display" -l json -d 'Emit the stable JSON envelope'
complete -c d2b -n "__fish_d2b_using_subcommand display" -l human -d 'Force human-readable terminal output'
complete -c d2b -n "__fish_d2b_using_subcommand display" -l no-deadline -d 'Suppress the command default deadline'
complete -c d2b -n "__fish_d2b_using_subcommand display" -s h -l help -d 'Print help'
