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
complete -c nixling -n "__fish_nixling_needs_command" -f -a "keys" -d 'Managed-key lifecycle (list / show / rotate)'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "trust" -d 'Trust a VM\'s host key on first use (TOFU)'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "rotate-known-host" -d 'Rotate the consumer\'s recorded known-host entry for a VM'
complete -c nixling -n "__fish_nixling_needs_command" -f -a "migrate" -d 'Analyse the host config and emit a migration plan'
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
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole help" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole help" -f -a "start" -d 'Start the per-VM DAG (virtiofsd → CH → readiness probes)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole help" -f -a "stop" -d 'Stop the per-VM DAG in reverse topo order'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole help" -f -a "restart" -d 'Stop then start; same envelope contract as start'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole help" -f -a "list" -d 'Daemon-side runtime view (different from `nixling list`, which is the static manifest view)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole help" -f -a "status" -d 'Daemon-side readiness state for a VM (api-ready phase)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole help" -f -a "konsole" -d 'Open an SSH session to the VM in a host terminal. Resolves the per-VM SSH key from the bundle\'s `managed_keys.effective_key_path(<vm>)` (honors `nixling.site.keysDir` + per-VM overrides; legacy `/var/lib/nixling/keys/<vm>_ed25519` is the fallback) and the IP from the manifest\'s `static_ip`. Default terminal: konsole'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and not __fish_seen_subcommand_from start stop restart list status konsole help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
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
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -l user -d 'SSH user inside the guest. Defaults to the per-VM `ssh_user` from the manifest; falls back to `$USER` if the manifest entry is absent. Override for ad-hoc per-user sessions' -r
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -l host -d 'Override the SSH host (IP or hostname). Default: manifest `static_ip` (bundle-resolved LAN address)' -r
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -l key -d 'Override the SSH key path. Default: the bundle\'s `managed_keys.effective_key_path(<vm>)` (honors `nixling.site.keysDir` + per-VM overrides). Legacy `/var/lib/nixling/keys/<vm>_ed25519` is only the fallback when no bundle is staged' -r -F
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -l dry-run -d 'Print the would-be command without executing'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -l json
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -l human
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from konsole" -s h -l help -d 'Print help'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from help" -f -a "start" -d 'Start the per-VM DAG (virtiofsd → CH → readiness probes)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from help" -f -a "stop" -d 'Stop the per-VM DAG in reverse topo order'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from help" -f -a "restart" -d 'Stop then start; same envelope contract as start'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from help" -f -a "list" -d 'Daemon-side runtime view (different from `nixling list`, which is the static manifest view)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from help" -f -a "status" -d 'Daemon-side readiness state for a VM (api-ready phase)'
complete -c nixling -n "__fish_nixling_using_subcommand vm; and __fish_seen_subcommand_from help" -f -a "konsole" -d 'Open an SSH session to the VM in a host terminal. Resolves the per-VM SSH key from the bundle\'s `managed_keys.effective_key_path(<vm>)` (honors `nixling.site.keysDir` + per-VM overrides; legacy `/var/lib/nixling/keys/<vm>_ed25519` is the fallback) and the IP from the manifest\'s `static_ip`. Default terminal: konsole'
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
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "list" -d 'List declared VMs from the static manifest'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "status" -d 'Show per-VM runtime status plus bridge health'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "usb" -d 'USBIP attach / detach / probe'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "console" -d 'Foreground serial console bridge for headless VMs (not yet implemented)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "audio" -d 'Per-VM audio grant bridge (not yet implemented)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "audit" -d 'Tail the broker audit log'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "host" -d 'Host-side preflight, install, doctor, and reconcile verbs'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "auth" -d 'Authorisation introspection'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "vm" -d 'Per-VM lifecycle verbs (start / stop / restart / list / status / konsole)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "up" -d 'Alias for `vm start <vm>`'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "down" -d 'Alias for `vm stop <vm>`'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "restart" -d 'Alias for `vm restart <vm>`'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "build" -d 'Non-destructive eval + build of the per-VM toplevel'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "generations" -d 'List current / booted / numbered generations for a VM'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "switch" -d 'Atomically activate a new per-VM closure'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "boot" -d 'Stage a per-VM closure for the next boot only'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "test" -d 'Activate a per-VM closure with rollback on reboot'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "rollback" -d 'Roll a VM back to its previous generation'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "gc" -d 'Garbage-collect the per-VM /nix/store hardlink farm'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "keys" -d 'Managed-key lifecycle (list / show / rotate)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "trust" -d 'Trust a VM\'s host key on first use (TOFU)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "rotate-known-host" -d 'Rotate the consumer\'s recorded known-host entry for a VM'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "migrate" -d 'Analyse the host config and emit a migration plan'
complete -c nixling -n "__fish_nixling_using_subcommand help; and not __fish_seen_subcommand_from list status usb console audio audit host auth vm up down restart build generations switch boot test rollback gc keys trust rotate-known-host migrate help" -f -a "help" -d 'Print this message or the help of the given subcommand(s)'
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
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from vm" -f -a "konsole" -d 'Open an SSH session to the VM in a host terminal. Resolves the per-VM SSH key from the bundle\'s `managed_keys.effective_key_path(<vm>)` (honors `nixling.site.keysDir` + per-VM overrides; legacy `/var/lib/nixling/keys/<vm>_ed25519` is the fallback) and the IP from the manifest\'s `static_ip`. Default terminal: konsole'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from keys" -f -a "list" -d 'List managed keys (per-VM SSH keypair fingerprints)'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from keys" -f -a "show" -d 'Show details for a specific VM\'s managed key'
complete -c nixling -n "__fish_nixling_using_subcommand help; and __fish_seen_subcommand_from keys" -f -a "rotate" -d 'Rotate the framework-managed per-VM SSH keypair. --apply mutates'
