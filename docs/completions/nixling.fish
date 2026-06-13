# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_nixling_global_optspecs
	string join \n h/help V/version
end

function __fish_nixling_needs_command
	# Figure out if the current invocation already has a command.
	set -l cmd (commandline -opc)
	set -e cmd[1]
	argparse -s (__fish_nixling_global_optspecs) -- $cmd 2>/dev/null
	or return
	if set -q argv[1]
		# Also print the command, so this can be used to figure out what it is.
		echo $argv[1]
		return 1
	end
	return 0
end

function __fish_nixling_using_subcommand
	set -l cmd (__fish_nixling_needs_command)
	test -z "$cmd"
	and return 1
	contains -- $cmd[1] $argv
end

complete -c nixling -n "__fish_nixling_needs_command" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c nixling -n "__fish_nixling_needs_command" -s V -l version -d 'Print version'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "list" -d 'List declared VMs from the static manifest'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "status" -d 'Show per-VM runtime status plus bridge health'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "usb" -d 'USBIP attach / detach / probe'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "console" -d 'Foreground serial console bridge for headless VMs (not yet implemented)'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "audio" -d 'Per-VM audio grant bridge (not yet implemented)'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "audit" -d 'Tail the broker audit log'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "host" -d 'Host-side preflight, install, doctor, and reconcile verbs'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "auth" -d 'Authorisation introspection'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "vm" -d 'Per-VM lifecycle verbs (start / stop / restart / list / status / konsole)'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "up" -d 'Alias for `vm start <vm>`'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "down" -d 'Alias for `vm stop <vm>`'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "restart" -d 'Alias for `vm restart <vm>`'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "build" -d 'Non-destructive eval + build of the per-VM toplevel'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "generations" -d 'List current / booted / numbered generations for a VM'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "switch" -d 'Atomically activate a new per-VM closure'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "boot" -d 'Stage a per-VM closure for the next boot only'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "test" -d 'Activate a per-VM closure with rollback on reboot'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "rollback" -d 'Roll a VM back to its previous generation'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "gc" -d 'Garbage-collect the per-VM /nix/store hardlink farm'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "store" -d 'Store-view maintenance and verification'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "keys" -d 'Managed-key lifecycle (list / show / rotate)'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "trust" -d 'Trust a VM\'s host key on first use (TOFU)'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "rotate-known-host" -d 'Rotate the consumer\'s recorded known-host entry for a VM'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "migrate" -d 'Analyse the host config and emit a migration plan'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "config" -d 'Sync / review / approve a VM\'s guest-editable config (`guestConfigFile`): pull the operator\'s in-VM edits to a host-side staging file, diff them, and approve them'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand list" -l json
complete -c nixling -n "__fish_nixling_using_subcommand list" -l human
complete -c nixling -n "__fish_nixling_using_subcommand list" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand status" -l vm -r
complete -c nixling -n "__fish_nixling_using_subcommand status" -l json
complete -c nixling -n "__fish_nixling_using_subcommand status" -l human
complete -c nixling -n "__fish_nixling_using_subcommand status" -l check-bridges
complete -c nixling -n "__fish_nixling_using_subcommand status" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand usb; and not __fish_seen_subcommand_from attach detach probe help" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand usb; and not __fish_seen_subcommand_from attach detach probe help" -f -a "attach" -d 'Bind a host USB busid to a VM via the native daemon path'
complete -c nixling -n "__fish_nixling_using_subcommand usb; and not __fish_seen_subcommand_from attach detach probe help" -f -a "detach" -d 'Unbind a host USB busid from a VM via the native daemon path'
complete -c nixling -n "__fish_nixling_using_subcommand usb; and not __fish_seen_subcommand_from attach detach probe help" -f -a "probe" -d 'List daemon-declared USBIP busid claims and lock owners'
complete -c nixling -n "__fish_nixling_using_subcommand usb; and not __fish_seen_subcommand_from attach detach probe help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from attach" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from attach" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from attach" -l json
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from attach" -l human
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from attach" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from detach" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from detach" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from detach" -l json
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from detach" -l human
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from detach" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from probe" -l json
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from probe" -l human
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from probe" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from help" -f -a "attach" -d 'Bind a host USB busid to a VM via the native daemon path'
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from help" -f -a "detach" -d 'Unbind a host USB busid from a VM via the native daemon path'
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from help" -f -a "probe" -d 'List daemon-declared USBIP busid claims and lock owners'
complete -c nixling -n "__fish_nixling_using_subcommand usb; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand console" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and not __fish_seen_subcommand_from status mic speaker off help" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and not __fish_seen_subcommand_from status mic speaker off help" -f -a "status" -d 'Show current grant state. With no VM, lists every audio-enabled VM'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and not __fish_seen_subcommand_from status mic speaker off help" -f -a "mic" -d 'Grant or revoke microphone access'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and not __fish_seen_subcommand_from status mic speaker off help" -f -a "speaker" -d 'Grant or revoke speaker access'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and not __fish_seen_subcommand_from status mic speaker off help" -f -a "off" -d 'Revoke both mic and speaker access'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and not __fish_seen_subcommand_from status mic speaker off help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and __fish_seen_subcommand_from mic" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and __fish_seen_subcommand_from speaker" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and __fish_seen_subcommand_from off" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and __fish_seen_subcommand_from help" -f -a "status" -d 'Show current grant state. With no VM, lists every audio-enabled VM'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and __fish_seen_subcommand_from help" -f -a "mic" -d 'Grant or revoke microphone access'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and __fish_seen_subcommand_from help" -f -a "speaker" -d 'Grant or revoke speaker access'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and __fish_seen_subcommand_from help" -f -a "off" -d 'Revoke both mic and speaker access'
complete -c nixling -n "__fish_nixling_using_subcommand audio; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand audit" -l strict
complete -c nixling -n "__fish_nixling_using_subcommand audit" -l json
complete -c nixling -n "__fish_nixling_using_subcommand audit" -l human
complete -c nixling -n "__fish_nixling_using_subcommand audit" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand host; and not __fish_seen_subcommand_from check prepare destroy doctor install reconcile validate help" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand host; and not __fish_seen_subcommand_from check prepare destroy doctor install reconcile validate help" -f -a "check" -d 'Read-only preflight: inventories host posture without mutation'
complete -c nixling -n "__fish_nixling_using_subcommand host; and not __fish_seen_subcommand_from check prepare destroy doctor install reconcile validate help" -f -a "prepare" -d 'Reconcile host-side state (bridges, nftables, sysctls). --apply mutates'
complete -c nixling -n "__fish_nixling_using_subcommand host; and not __fish_seen_subcommand_from check prepare destroy doctor install reconcile validate help" -f -a "destroy" -d 'Tear down host-side state owned by nixling. --apply mutates'
complete -c nixling -n "__fish_nixling_using_subcommand host; and not __fish_seen_subcommand_from check prepare destroy doctor install reconcile validate help" -f -a "doctor" -d 'Read-only deep diagnostics for the daemon + broker state'
complete -c nixling -n "__fish_nixling_using_subcommand host; and not __fish_seen_subcommand_from check prepare destroy doctor install reconcile validate help" -f -a "install" -d 'Install nixlingd + broker units onto the host. --apply mutates'
complete -c nixling -n "__fish_nixling_using_subcommand host; and not __fish_seen_subcommand_from check prepare destroy doctor install reconcile validate help" -f -a "reconcile" -d 'Recover host network state after the daemon engaged operator-only mode'
complete -c nixling -n "__fish_nixling_using_subcommand host; and not __fish_seen_subcommand_from check prepare destroy doctor install reconcile validate help" -f -a "validate" -d 'Run the host-side validator suite and write evidence records'
complete -c nixling -n "__fish_nixling_using_subcommand host; and not __fish_seen_subcommand_from check prepare destroy doctor install reconcile validate help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from check" -l read-only
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from check" -l strict
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from check" -l json
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from check" -l human
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from check" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from prepare" -l dry-run -d 'Plan the reconcile without mutating host state'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from prepare" -l apply -d 'Apply the reconcile (mutates host state)'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from prepare" -l json
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from prepare" -l human
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from prepare" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from destroy" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from destroy" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from destroy" -l json
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from destroy" -l human
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from destroy" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from doctor" -l read-only -d 'Mandatory: doctor is read-only. Mutating forms are separate verbs'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from doctor" -l json
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from doctor" -l human
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from doctor" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from install" -l dry-run -d 'Report the planned install steps without mutating'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from install" -l apply -d 'Perform the install through the daemon → broker `RunHostInstall` path'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from install" -l enable -d 'After `--apply`, enable nixlingd.service via systemctl'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from install" -l start -d 'After `--apply --enable`, start nixlingd.service'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from install" -l no-start -d 'Explicitly do NOT start nixlingd.service post-install'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from install" -l json
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from install" -l human
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from install" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from reconcile" -l network -d 'Re-run the network slice of `host prepare` and clear the daemon\'s net-route preflight counter. Currently the only available scope'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from reconcile" -l dry-run -d 'Plan the reconcile without mutating host state'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from reconcile" -l apply -d 'Apply the reconcile (mutates host state)'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from reconcile" -l json
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from reconcile" -l human
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from reconcile" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from validate" -l wave -d 'Restrict to a single wave. Other waves are reported as `skipped`' -r
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from validate" -l operator-signature -d 'Override the per-wave operator signature. When unset, the verb derives a deterministic sha256 signature from `hostname|wave|scripts_dir|timestamp`' -r
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from validate" -l evidence-dir -d 'Override the evidence directory. Default: `/var/lib/nixling/validated`' -r -F
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from validate" -l scripts-dir -d 'Override the scripts directory. Default: best-effort discovery of the installed `tests/` share, then `./tests`' -r -F
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from validate" -l dry-run -d 'Plan: report which readiness validators WOULD be attested. No evidence is written'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from validate" -l apply -d 'Apply: write `/var/lib/nixling/validated/<wave>.json` for every wave whose declared validators are present on disk'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from validate" -l json
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from validate" -l human
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from validate" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from help" -f -a "check" -d 'Read-only preflight: inventories host posture without mutation'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from help" -f -a "prepare" -d 'Reconcile host-side state (bridges, nftables, sysctls). --apply mutates'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from help" -f -a "destroy" -d 'Tear down host-side state owned by nixling. --apply mutates'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from help" -f -a "doctor" -d 'Read-only deep diagnostics for the daemon + broker state'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from help" -f -a "install" -d 'Install nixlingd + broker units onto the host. --apply mutates'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from help" -f -a "reconcile" -d 'Recover host network state after the daemon engaged operator-only mode'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from help" -f -a "validate" -d 'Run the host-side validator suite and write evidence records'
complete -c nixling -n "__fish_nixling_using_subcommand host; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand auth; and not __fish_seen_subcommand_from status help" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand auth; and not __fish_seen_subcommand_from status help" -f -a "status"
complete -c nixling -n "__fish_nixling_using_subcommand auth; and not __fish_seen_subcommand_from status help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand auth; and __fish_seen_subcommand_from status" -l test-uid -r
complete -c nixling -n "__fish_nixling_using_subcommand auth; and __fish_seen_subcommand_from status" -l json
complete -c nixling -n "__fish_nixling_using_subcommand auth; and __fish_seen_subcommand_from status" -l human
complete -c nixling -n "__fish_nixling_using_subcommand auth; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand auth; and __fish_seen_subcommand_from help" -f -a "status"
complete -c nixling -n "__fish_nixling_using_subcommand auth; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole exec help" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole exec help" -f -a "start" -d 'Start the per-VM DAG (virtiofsd → CH → readiness probes)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole exec help" -f -a "stop" -d 'Stop the per-VM DAG in reverse topo order'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole exec help" -f -a "restart" -d 'Stop then start; same envelope contract as start'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole exec help" -f -a "list" -d 'Daemon-side runtime view (different from `nixling list`, which is the static manifest view)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole exec help" -f -a "status" -d 'Daemon-side readiness state for a VM (api-ready phase)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole exec help" -f -a "konsole" -d 'Open an interactive guest session in a host terminal. Thin wrapper that hosts `nixling vm exec -it <vm> -- bash -l` in the chosen terminal emulator (default `konsole`, overridable via `--terminal`) over the authenticated guest-control transport. There is no SSH; the retired SSH-only flags `--host`/`--key`/`--user` are rejected with a migration message'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole exec help" -f -a "exec" -d 'Run a command inside the VM over the authenticated guest-control transport (no SSH). `nixling vm exec <vm> -- <cmd...>` runs a non-interactive command; `nixling vm exec -it <vm> -- <cmd...>` allocates a guest PTY for an interactive session. Routed CLI → daemon `public.sock` (admin-only) → authenticated guest-control vsock → guestd exec RPCs'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole exec help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from start" -l dry-run -d 'Plan the DAG without spawning any role'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from start" -l apply -d 'Apply the DAG (drives the supervisor)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from start" -l no-wait-api -d 'Exit 0 on process-alive success without waiting for api-ready. Default behavior is --strict (wait for both process-alive and api-ready)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from start" -l json
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from start" -l human
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from start" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from stop" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from stop" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from stop" -l json
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from stop" -l human
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from stop" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from restart" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from restart" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from restart" -l json
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from restart" -l human
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from restart" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from list" -l json
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from list" -l human
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from status" -l json
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from status" -l human
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -l terminal -d 'Terminal emulator binary to spawn. Must accept `-e` to execute a command. Tested: konsole, alacritty, foot, gnome-terminal, xterm. Default: konsole' -r
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -l user -d 'Retired SSH-only flag. Rejected with a migration message; guest-control exec runs as the guest-control principal' -r
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -l host -d 'Retired SSH-only flag. Rejected with a migration message; the transport is resolved from the trusted bundle' -r
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -l key -d 'Retired SSH-only flag. Rejected with a migration message; guest-control exec needs no SSH key' -r -F
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -l dry-run -d 'Print the would-be command without executing'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -l json
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -l human
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from exec" -l env -d 'Set an environment variable in the guest command (`KEY=VALUE`). Repeatable' -r
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from exec" -l cwd -d 'Working directory for the guest command' -r
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from exec" -s i -l interactive -d 'Forward host stdin into the guest command (`-i`)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from exec" -s t -l tty -d 'Allocate a PTY in the guest and put the host terminal in raw mode (`-t`). Implies stdin forwarding. Human-only (incompatible with `--json`)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from exec" -l json -d 'Emit a single terminal JSON envelope (exit code + source/reason + bounded captured output). Non-interactive only'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from exec" -l human
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from exec" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from help" -f -a "start" -d 'Start the per-VM DAG (virtiofsd → CH → readiness probes)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from help" -f -a "stop" -d 'Stop the per-VM DAG in reverse topo order'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from help" -f -a "restart" -d 'Stop then start; same envelope contract as start'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from help" -f -a "list" -d 'Daemon-side runtime view (different from `nixling list`, which is the static manifest view)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from help" -f -a "status" -d 'Daemon-side readiness state for a VM (api-ready phase)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from help" -f -a "konsole" -d 'Open an interactive guest session in a host terminal. Thin wrapper that hosts `nixling vm exec -it <vm> -- bash -l` in the chosen terminal emulator (default `konsole`, overridable via `--terminal`) over the authenticated guest-control transport. There is no SSH; the retired SSH-only flags `--host`/`--key`/`--user` are rejected with a migration message'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from help" -f -a "exec" -d 'Run a command inside the VM over the authenticated guest-control transport (no SSH). `nixling vm exec <vm> -- <cmd...>` runs a non-interactive command; `nixling vm exec -it <vm> -- <cmd...>` allocates a guest PTY for an interactive session. Routed CLI → daemon `public.sock` (admin-only) → authenticated guest-control vsock → guestd exec RPCs'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand up" -l dry-run -d 'Plan the DAG without spawning any role'
complete -c nixling -n "__fish_nixling_using_subcommand up" -l apply -d 'Apply the DAG (drives the supervisor)'
complete -c nixling -n "__fish_nixling_using_subcommand up" -l no-wait-api -d 'Exit 0 on process-alive success without waiting for api-ready. Default behavior is --strict (wait for both process-alive and api-ready)'
complete -c nixling -n "__fish_nixling_using_subcommand up" -l json
complete -c nixling -n "__fish_nixling_using_subcommand up" -l human
complete -c nixling -n "__fish_nixling_using_subcommand up" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand down" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand down" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand down" -l json
complete -c nixling -n "__fish_nixling_using_subcommand down" -l human
complete -c nixling -n "__fish_nixling_using_subcommand down" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand restart" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand restart" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand restart" -l json
complete -c nixling -n "__fish_nixling_using_subcommand restart" -l human
complete -c nixling -n "__fish_nixling_using_subcommand restart" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand build" -l json
complete -c nixling -n "__fish_nixling_using_subcommand build" -l human
complete -c nixling -n "__fish_nixling_using_subcommand build" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand generations" -l json
complete -c nixling -n "__fish_nixling_using_subcommand generations" -l human
complete -c nixling -n "__fish_nixling_using_subcommand generations" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand switch" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand switch" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand switch" -l json
complete -c nixling -n "__fish_nixling_using_subcommand switch" -l human
complete -c nixling -n "__fish_nixling_using_subcommand switch" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand boot" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand boot" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand boot" -l json
complete -c nixling -n "__fish_nixling_using_subcommand boot" -l human
complete -c nixling -n "__fish_nixling_using_subcommand boot" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand test" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand test" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand test" -l json
complete -c nixling -n "__fish_nixling_using_subcommand test" -l human
complete -c nixling -n "__fish_nixling_using_subcommand test" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand rollback" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand rollback" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand rollback" -l json
complete -c nixling -n "__fish_nixling_using_subcommand rollback" -l human
complete -c nixling -n "__fish_nixling_using_subcommand rollback" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand gc" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand gc" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand gc" -l json
complete -c nixling -n "__fish_nixling_using_subcommand gc" -l human
complete -c nixling -n "__fish_nixling_using_subcommand gc" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand store; and not __fish_seen_subcommand_from verify help" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand store; and not __fish_seen_subcommand_from verify help" -f -a "verify" -d 'Verify a VM\'s hardlink-backed live store-view'
complete -c nixling -n "__fish_nixling_using_subcommand store; and not __fish_seen_subcommand_from verify help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand store; and __fish_seen_subcommand_from verify" -l repair
complete -c nixling -n "__fish_nixling_using_subcommand store; and __fish_seen_subcommand_from verify" -l json
complete -c nixling -n "__fish_nixling_using_subcommand store; and __fish_seen_subcommand_from verify" -l human
complete -c nixling -n "__fish_nixling_using_subcommand store; and __fish_seen_subcommand_from verify" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand store; and __fish_seen_subcommand_from help" -f -a "verify" -d 'Verify a VM\'s hardlink-backed live store-view'
complete -c nixling -n "__fish_nixling_using_subcommand store; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand keys; and not __fish_seen_subcommand_from list show rotate help" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand keys; and not __fish_seen_subcommand_from list show rotate help" -f -a "list" -d 'List managed keys (per-VM SSH keypair fingerprints)'
complete -c nixling -n "__fish_nixling_using_subcommand keys; and not __fish_seen_subcommand_from list show rotate help" -f -a "show" -d 'Show details for a specific VM\'s managed key'
complete -c nixling -n "__fish_nixling_using_subcommand keys; and not __fish_seen_subcommand_from list show rotate help" -f -a "rotate" -d 'Rotate the framework-managed per-VM SSH keypair. --apply mutates'
complete -c nixling -n "__fish_nixling_using_subcommand keys; and not __fish_seen_subcommand_from list show rotate help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from list" -l json
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from list" -l human
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from list" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from show" -l json
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from show" -l human
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from show" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from rotate" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from rotate" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from rotate" -l json
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from rotate" -l human
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from rotate" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from help" -f -a "list" -d 'List managed keys (per-VM SSH keypair fingerprints)'
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from help" -f -a "show" -d 'Show details for a specific VM\'s managed key'
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from help" -f -a "rotate" -d 'Rotate the framework-managed per-VM SSH keypair. --apply mutates'
complete -c nixling -n "__fish_nixling_using_subcommand keys; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand trust" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand trust" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand trust" -l json
complete -c nixling -n "__fish_nixling_using_subcommand trust" -l human
complete -c nixling -n "__fish_nixling_using_subcommand trust" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand rotate-known-host" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand rotate-known-host" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand rotate-known-host" -l json
complete -c nixling -n "__fish_nixling_using_subcommand rotate-known-host" -l human
complete -c nixling -n "__fish_nixling_using_subcommand rotate-known-host" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand migrate" -l dry-run
complete -c nixling -n "__fish_nixling_using_subcommand migrate" -l apply
complete -c nixling -n "__fish_nixling_using_subcommand migrate" -l json
complete -c nixling -n "__fish_nixling_using_subcommand migrate" -l human
complete -c nixling -n "__fish_nixling_using_subcommand migrate" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand config; and not __fish_seen_subcommand_from sync diff approve reject status help" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand config; and not __fish_seen_subcommand_from sync diff approve reject status help" -f -a "sync" -d 'Pull the VM\'s in-guest edited config into a host-side staging file'
complete -c nixling -n "__fish_nixling_using_subcommand config; and not __fish_seen_subcommand_from sync diff approve reject status help" -f -a "diff" -d 'Diff the staged guest config against a live host-side file'
complete -c nixling -n "__fish_nixling_using_subcommand config; and not __fish_seen_subcommand_from sync diff approve reject status help" -f -a "approve" -d 'Approve the staged guest config by writing it to a target file'
complete -c nixling -n "__fish_nixling_using_subcommand config; and not __fish_seen_subcommand_from sync diff approve reject status help" -f -a "reject" -d 'Discard the staged guest config'
complete -c nixling -n "__fish_nixling_using_subcommand config; and not __fish_seen_subcommand_from sync diff approve reject status help" -f -a "status" -d 'Report whether a VM has a pending (un-approved) staged config'
complete -c nixling -n "__fish_nixling_using_subcommand config; and not __fish_seen_subcommand_from sync diff approve reject status help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from sync" -l guest-path -d 'Path of the editable guest config INSIDE the VM to pull. Honored only by the legacy operator SSH transport; on guest-control VMs the canonical guest config working copy is read by file id and this flag is rejected' -r
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from sync" -l host -d 'Override the SSH host (defaults to the manifest `static_ip`). SSH transport only; rejected on guest-control VMs' -r
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from sync" -l user -d 'Override the SSH user (defaults to the manifest `ssh_user`). SSH transport only; rejected on guest-control VMs' -r
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from sync" -l key -d 'Override the SSH private key path. SSH transport only; rejected on guest-control VMs' -r -F
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from sync" -l known-hosts -d 'known_hosts file used to verify the VM\'s host key (defaults to the framework-managed `/var/lib/nixling/known_hosts.nixling`). SSH transport only; rejected on guest-control VMs' -r -F
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from sync" -l dry-run -d 'Print the planned action instead of running it'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from sync" -l json -d 'Emit a JSON envelope'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from sync" -s h -l help -d 'Print help (see more with \'--help\')'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from diff" -l against -d 'The live host-side guest config file to compare the staging against' -r -F
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from diff" -l json -d 'Emit a JSON envelope'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from diff" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from approve" -l to -d 'The host-side file to write the approved staging copy onto. The operator chooses this (typically their `guestConfigFile` path)' -r -F
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from approve" -l json -d 'Emit a JSON envelope'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from approve" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from reject" -l json -d 'Emit a JSON envelope'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from reject" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from status" -l all -d 'Report every VM that currently has a pending staging file'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from status" -l json -d 'Emit a JSON envelope'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from status" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "sync" -d 'Pull the VM\'s in-guest edited config into a host-side staging file'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "diff" -d 'Diff the staged guest config against a live host-side file'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "approve" -d 'Approve the staged guest config by writing it to a target file'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "reject" -d 'Discard the staged guest config'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "status" -d 'Report whether a VM has a pending (un-approved) staged config'
complete -c nixling -n "__fish_nixling_using_subcommand config; and __fish_seen_subcommand_from help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "list" -d 'List declared VMs from the static manifest'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "status" -d 'Show per-VM runtime status plus bridge health'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "usb" -d 'USBIP attach / detach / probe'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "console" -d 'Foreground serial console bridge for headless VMs (not yet implemented)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "audio" -d 'Per-VM audio grant bridge (not yet implemented)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "audit" -d 'Tail the broker audit log'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "host" -d 'Host-side preflight, install, doctor, and reconcile verbs'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "auth" -d 'Authorisation introspection'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "vm" -d 'Per-VM lifecycle verbs (start / stop / restart / list / status / konsole)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "up" -d 'Alias for `vm start <vm>`'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "down" -d 'Alias for `vm stop <vm>`'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "restart" -d 'Alias for `vm restart <vm>`'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "build" -d 'Non-destructive eval + build of the per-VM toplevel'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "generations" -d 'List current / booted / numbered generations for a VM'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "switch" -d 'Atomically activate a new per-VM closure'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "boot" -d 'Stage a per-VM closure for the next boot only'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "test" -d 'Activate a per-VM closure with rollback on reboot'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "rollback" -d 'Roll a VM back to its previous generation'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "gc" -d 'Garbage-collect the per-VM /nix/store hardlink farm'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "store" -d 'Store-view maintenance and verification'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "keys" -d 'Managed-key lifecycle (list / show / rotate)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "trust" -d 'Trust a VM\'s host key on first use (TOFU)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "rotate-known-host" -d 'Rotate the consumer\'s recorded known-host entry for a VM'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "migrate" -d 'Analyse the host config and emit a migration plan'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "config" -d 'Sync / review / approve a VM\'s guest-editable config (`guestConfigFile`): pull the operator\'s in-VM edits to a host-side staging file, diff them, and approve them'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc store keys trust rotate-known-host migrate config help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from usb" -f -a "attach" -d 'Bind a host USB busid to a VM via the native daemon path'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from usb" -f -a "detach" -d 'Unbind a host USB busid from a VM via the native daemon path'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from usb" -f -a "probe" -d 'List daemon-declared USBIP busid claims and lock owners'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from audio" -f -a "status" -d 'Show current grant state. With no VM, lists every audio-enabled VM'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from audio" -f -a "mic" -d 'Grant or revoke microphone access'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from audio" -f -a "speaker" -d 'Grant or revoke speaker access'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from audio" -f -a "off" -d 'Revoke both mic and speaker access'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from host" -f -a "check" -d 'Read-only preflight: inventories host posture without mutation'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from host" -f -a "prepare" -d 'Reconcile host-side state (bridges, nftables, sysctls). --apply mutates'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from host" -f -a "destroy" -d 'Tear down host-side state owned by nixling. --apply mutates'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from host" -f -a "doctor" -d 'Read-only deep diagnostics for the daemon + broker state'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from host" -f -a "install" -d 'Install nixlingd + broker units onto the host. --apply mutates'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from host" -f -a "reconcile" -d 'Recover host network state after the daemon engaged operator-only mode'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from host" -f -a "validate" -d 'Run the host-side validator suite and write evidence records'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from auth" -f -a "status"
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from vm" -f -a "start" -d 'Start the per-VM DAG (virtiofsd → CH → readiness probes)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from vm" -f -a "stop" -d 'Stop the per-VM DAG in reverse topo order'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from vm" -f -a "restart" -d 'Stop then start; same envelope contract as start'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from vm" -f -a "list" -d 'Daemon-side runtime view (different from `nixling list`, which is the static manifest view)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from vm" -f -a "status" -d 'Daemon-side readiness state for a VM (api-ready phase)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from vm" -f -a "konsole" -d 'Open an interactive guest session in a host terminal. Thin wrapper that hosts `nixling vm exec -it <vm> -- bash -l` in the chosen terminal emulator (default `konsole`, overridable via `--terminal`) over the authenticated guest-control transport. There is no SSH; the retired SSH-only flags `--host`/`--key`/`--user` are rejected with a migration message'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from vm" -f -a "exec" -d 'Run a command inside the VM over the authenticated guest-control transport (no SSH). `nixling vm exec <vm> -- <cmd...>` runs a non-interactive command; `nixling vm exec -it <vm> -- <cmd...>` allocates a guest PTY for an interactive session. Routed CLI → daemon `public.sock` (admin-only) → authenticated guest-control vsock → guestd exec RPCs'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from store" -f -a "verify" -d 'Verify a VM\'s hardlink-backed live store-view'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from keys" -f -a "list" -d 'List managed keys (per-VM SSH keypair fingerprints)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from keys" -f -a "show" -d 'Show details for a specific VM\'s managed key'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from keys" -f -a "rotate" -d 'Rotate the framework-managed per-VM SSH keypair. --apply mutates'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "sync" -d 'Pull the VM\'s in-guest edited config into a host-side staging file'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "diff" -d 'Diff the staged guest config against a live host-side file'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "approve" -d 'Approve the staged guest config by writing it to a target file'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "reject" -d 'Discard the staged guest config'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from config" -f -a "status" -d 'Report whether a VM has a pending (un-approved) staged config'
