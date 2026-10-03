//! The nested Cloud Hypervisor Guest acceptance check, ported from its
//! fixture.
//!
//! It boots the writable-store host node and drives the controller-owned Guest
//! lifecycle end to end: the daemon, the broker socket and the broker service
//! come up; the preflight capture reads the public, redacted Resource
//! projection before anything waits on the nested VMM; the two storage
//! providers establish their live ResourceV3 controller sessions; the host's
//! KVM, vhost-net and cgroup-v2 postures are asserted; the artifact catalog,
//! the per-Guest closure spec and the zone's resource bundle are read back;
//! the fixture enrolls the Guest's ComponentSession key pair and record; the
//! VMM API socket, the runner process, the Guest's endpoints and system
//! Volume, the store view and the declared `state` Volume's binding all reach
//! their declared end state; the runner process survives a `d2bd` restart
//! while the Guest's session generation advances behind it; and the Guest and
//! the Volume are then deleted and drained - socket gone, processes gone,
//! binding gone - so no serving effect outlives its owner.
//!
//! The assertions are the fixture's, in the fixture's order, with the
//! fixture's own command text and bounds. What the fixture expressed as
//! `machine.*` calls is [`GuestControl`]'s own operations, and what it
//! expressed as `stage`, `diag`, `diag_unit` and `diag_wait` calls are the
//! same primitives the fixtures' diagnostics prelude provides, so a failure
//! reported here reads the way the fixture's failure read. The fixture's row
//! projection and its three row builders (`live_rows`, `saved_rows`,
//! `summary_rows`) ride with it unchanged, as does the runner search and the
//! pid and start time a later assertion checks it by.
//!
//! Two tokens the fixture interpolated are the one thing this module cannot
//! carry: `${fixtureKeys}/host.key` and `${fixtureKeys}/guest.pub` name files
//! in a nix store path, and a store path is not addressable from the lane's
//! Rust. The node installs the same four `printf`'d key files at
//! `/etc/d2b/fixture-keys/`, and the enrollment command reads the two it needs
//! from there; every other byte of that command, and of every other command
//! here, is the fixture's own.
//!
//! The guest is the writable-store shape plus the fixture's own
//! contributions - the acceptance artifacts and their publisher keys, the two
//! zones and their rows, the checked guest system with its store image, and
//! the four userspace tools its commands drive it with - declared in
//! `nix/test-support/host-integration-node.nix`.
//!
//! `start_all()` is not restated here: it is the lane's own boot of the guest
//! the check runs against.

use std::time::Duration;

use crate::legacy::{DiagRow, GuestControl, LegacyError, LegacyResult};

/// The bound the daemon's activation gets, the fixture's own, before and
/// after the restart.
const DAEMON_BOUND: Duration = Duration::from_secs(180);

/// The bound the broker socket unit and the two public-socket file waits get,
/// the fixture's own.
const SOCKET_BOUND: Duration = Duration::from_secs(30);

/// The bound the broker service's activation gets, the fixture's own.
const BROKER_BOUND: Duration = Duration::from_secs(30);

/// The bound the two controller waits get, the fixture's own: cold artifact
/// extraction inside a fresh VM varies widely on shared hardware, and the
/// fixture waited an eventual state rather than a timing SLO.
const CONTROLLER_BOUND: Duration = Duration::from_secs(180);

/// The bound the Guest console's boot identity gets, the fixture's own.
const CONSOLE_BOUND: Duration = Duration::from_secs(30);

/// The bound the Guest's own row waits get, the fixture's own, plus the VMM
/// Process drain's.
const GUEST_BOUND: Duration = Duration::from_secs(30);

/// The bound the Guest's system Volume gets, the fixture's own.
const GUEST_VOLUME_BOUND: Duration = Duration::from_secs(180);

/// The bound the declared `state` Volume's binding gets, the fixture's own.
const BINDING_BOUND: Duration = Duration::from_secs(180);

/// The bound the binding's worker and endpoint get, the fixture's own.
const BINDING_WORKER_BOUND: Duration = Duration::from_secs(60);

/// The bound the two VMM API-socket waits get, the fixture's own.
const API_SOCKET_BOUND: Duration = Duration::from_secs(30);

/// The bound the two drain-requested waits get, the fixture's own.
const DRAINING_BOUND: Duration = Duration::from_secs(30);

/// The bound the Guest's own drain gets, the fixture's own.
const DRAINED_BOUND: Duration = Duration::from_secs(60);

/// The bound the binding's own drain gets, the fixture's own.
const BINDING_DRAIN_BOUND: Duration = Duration::from_secs(120);

/// The row projection the shared diagnostics print on a timed-out wait; it
/// mirrors the fields each wait asserts on (issue #513).
const DIAG_PROJECTION: &str = concat!(
    "[.resources[] | {type: .type, name: .metadata.name, ",
    "owner: .metadata.ownerRef, uid: .metadata.uid, ",
    "gen: .metadata.generation, phase: .status.phase, ",
    "obs: .status.observedGeneration, ",
    "provider: .spec.providerRef, execution: .spec.executionRef, ",
    "processClass: .spec.processClass, template: .spec.template, ",
    "conditions: [.status.conditions[]? | ",
    "{type: .type, status: .status, reason: .reason}], ",
    "outcome: (.status.outcome | ",
    "if . == null then null else ",
    "{code: .code, retryable: .retryable} end), ",
    "resource: .status.resource}]",
);

/// The broker service's activation, the fixture's own command.
const BROKER_START: &str = "systemctl start d2b-broker.service";

/// The preflight capture: one public, redacted row projection per
/// resource type, then the daemon's ComponentSession terminal-error count, all
/// written to `/run/d2b-preflight-summary.log` and echoed.
const PREFLIGHT_CAPTURE: &str = concat!(
    "set -o pipefail; ",
    ": > /run/d2b-preflight-summary.log; ",
    "for resource_type in Guest Process Endpoint Volume Provider VolumeBinding; do ",
    "printf '%s: ' \"$resource_type\" >> /run/d2b-preflight-summary.log; ",
    "timeout 5s runuser -u alice -- env ",
    "D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list \"$resource_type\" ",
    "2>/dev/null | ",
    "jq -c '[.resources[] | ",
    "{type: .type, ",
    "metadata: {name: .metadata.name, uid: .metadata.uid, ",
    "generation: .metadata.generation, ownerRef: .metadata.ownerRef, ",
    "zone: .metadata.zone}, ",
    "spec: {providerRef: .spec.providerRef, ",
    "executionRef: .spec.executionRef, ",
    "processClass: .spec.processClass, template: .spec.template}, ",
    "status: {phase: .status.phase, ",
    "observedGeneration: .status.observedGeneration, ",
    "conditions: [.status.conditions[]? | ",
    "{type: .type, status: .status, reason: .reason}], ",
    "outcome: (.status.outcome | ",
    "if . == null then null else ",
    "{code: .code, retryable: .retryable} end), ",
    "resource: .status.resource}}]' ",
    ">> /run/d2b-preflight-summary.log 2>/dev/null || ",
    "printf 'unavailable\\n' >> /run/d2b-preflight-summary.log; ",
    "done; ",
    "session_errors=$(journalctl -u d2bd.service --no-pager -b ",
    "2>/dev/null | grep -Ec ",
    "'session-authentication-failed|session-generation-stale' || true); ",
    "printf 'ComponentSession terminal error count: %s\\n' \"$session_errors\" ",
    ">> /run/d2b-preflight-summary.log; ",
    "cat /run/d2b-preflight-summary.log",
);

/// The daemon's journal carries both external providers' live ResourceV3
/// sessions, one per acceptance controller.
const CONTROLLER_SESSIONS: &str = concat!(
    "test \"$(journalctl -u d2bd.service --no-pager -o cat -b ",
    "| grep -Fc 'external Provider controller ResourceV3 session live')\" -ge 2",
);

/// One `Ready`, generation-settled controller Process per volume provider.
const VOLUME_CONTROLLER_PROCESSES: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Process ",
    ">/run/d2b-volume-controller-processes.json && ",
    "jq -e '",
    "([.resources[] | select(.type == \"Process\" and ",
    ".metadata.ownerRef == \"Provider/volume-local\" and ",
    ".spec.providerRef == \"Provider/system-minijail\" and ",
    ".spec.processClass == \"controller\" and ",
    ".spec.template == \"controller-volume-acceptance-provider-acceptance-controller\" and ",
    "",
    ".status.phase == \"Ready\" and ",
    ".status.observedGeneration == .metadata.generation)] | length == 1) and ",
    "([.resources[] | select(.type == \"Process\" and ",
    ".metadata.ownerRef == \"Provider/volume-virtiofs\" and ",
    ".spec.providerRef == \"Provider/system-minijail\" and ",
    ".spec.processClass == \"controller\" and ",
    ".spec.template == \"controller-volume-acceptance-provider-acceptance-controller\" and ",
    "",
    ".status.phase == \"Ready\" and ",
    ".status.observedGeneration == .metadata.generation)] | length == 1)' ",
    "/run/d2b-volume-controller-processes.json",
);

/// Both controller Processes are live, and no `pause` process is: the
/// two are the fixture's own awk and ps pipeline.
const ACCEPTANCE_CONTROLLERS: &str = concat!(
    "test \"$(ps -eo pid=,args= | awk '$NF ~ /acceptance-controller$/ {print $1}' ",
    "| wc -l)\" -ge 2 && ",
    "! ps -eo args= | grep -E '(^|/)pause([[:space:]]|$)' | grep -v grep",
);

/// The nested VMM's API socket, waited for the fixture's own 180 attempts,
/// with its full failure report behind the wait.
const NESTED_VMM_API_SOCKET: &str = concat!(
    "for attempt in $(seq 1 180); do ",
    "test -S /var/lib/d2b/zones/work/guests/acceptance-guest/acceptance-guest.sock ",
    "&& exit 0; ",
    "sleep 1; ",
    "done; ",
    "echo 'Cloud Hypervisor API socket did not become ready within 180s'; ",
    "cat /run/d2b-preflight-summary.log; ",
    "for resource_type in Volume VolumeBinding; do ",
    "echo \"=== $resource_type ===\"; ",
    "timeout 10s runuser -u alice -- env ",
    "D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list \"$resource_type\" 2>/dev/null | ",
    "jq -c '.resources[] | {name: .metadata.name, phase: .status.phase, ",
    "observedGeneration: .status.observedGeneration, ",
    "conditions: [.status.conditions[]? | {type: .type, reason: .reason}], ",
    "ready: .status.resource.ready}' || true; ",
    "done; ",
    "echo '=== Process ==='; ",
    "timeout 10s runuser -u alice -- env ",
    "D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Process 2>/dev/null | ",
    "jq -c '.resources[] | select(.name | startswith(\"vol-vfd\")) | ",
    "{name: .metadata.name, phase: .status.phase, ",
    "conditions: [.status.conditions[]? | {type: .type, reason: .reason}], ",
    "outcome: .status.outcome, update: .status.update}' || true; ",
    "exit 1",
);

/// The Guest's boot identity reached the host journal through its console.
const GUEST_CONSOLE_BOOT_ID: &str = concat!(
    "journalctl --no-pager -b ",
    "| grep -q 'D2B_GUEST_BOOT_ID='",
);

/// The journal tail the boot-id wait prints.
const HOST_JOURNAL_TAIL: &str =
    "journalctl --no-pager -o cat -b -n 120 2>/dev/null || true";

/// The Guest's ComponentSession enrollment: the Guest's uid, the boot
/// digest of the boot id the console printed, the key pair and the
/// `guest.json` enrollment record.
///
/// The two `install` lines read the fixture's own key material from
/// `/etc/d2b/fixture-keys`, where the node installs the same four `printf`'d
/// files the fixture's `let` built. Those two source tokens are the only
/// bytes of this command that are not the fixture's own: the fixture
/// interpolated a nix store path there, and a store path is not addressable
/// from the lane's Rust. The destination paths, the owners, the modes and
/// every other byte are the fixture's.
const GUEST_SESSION_ENROLLMENT: &str = concat!(
    "guest_uid=$(runuser -u alice -- env ",
    "D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Guest | ",
    "jq -er '.resources[] | select(.metadata.name == \"acceptance-guest\") ",
    "| .metadata.uid') && ",
    "boot_id=$(journalctl --no-pager -b ",
    "| sed -n 's/.*D2B_GUEST_BOOT_ID=\\([0-9a-f-]*\\).*/\\1/p' ",
    "| tail -1) && ",
    "boot_digest=$(printf 'd2b-kernel-boot-id-v1\\0%s' \"$boot_id\" ",
    "| sha256sum | cut -d' ' -f1) && ",
    "install -d -o d2bd -g d2bd -m 0700 ",
    "/var/lib/d2b/zones/work/guests/acceptance-guest/component-session && ",
    "install -o d2bd -g d2bd -m 0600 /etc/d2b/fixture-keys/host.key ",
    "/var/lib/d2b/zones/work/guests/acceptance-guest/component-session/host.key && ",
    "install -o d2bd -g d2bd -m 0600 /etc/d2b/fixture-keys/guest.pub ",
    "/var/lib/d2b/zones/work/guests/acceptance-guest/component-session/guest.pub && ",
    "cat > /var/lib/d2b/zones/work/guests/acceptance-guest/component-session/guest.json ",
    "<<EOF\n",
    "{\"guestRef\":\"Guest/acceptance-guest\",",
    "\"guestUid\":\"$guest_uid\",",
    "\"zone\":\"work\",",
    "\"bootIdentityDigest\":\"sha256:$boot_digest\",",
    "\"purpose\":\"component-session\",",
    "\"schemaFingerprint\":\"sha256:65e20cc53efdd2354931c5cf2ad722612dd9bc4e26e0b238b9048f24",
    "4db6c737\",",
    "\"reconnectGeneration\":1,",
    "\"providerGeneration\":1,",
    "\"controllerGeneration\":1,",
    "\"assignmentEpoch\":1}\n",
    "EOF\n",
    "chown d2bd:d2bd ",
    "/var/lib/d2b/zones/work/guests/acceptance-guest/component-session/guest.json && ",
    "chmod 0600 ",
    "/var/lib/d2b/zones/work/guests/acceptance-guest/component-session/guest.json",
);

/// The nested KVM posture, which this check fails closed without.
const KVM_CAPABILITY: &str = concat!(
    "test -e /dev/kvm && test -r /dev/kvm && test -w /dev/kvm || ",
    "{ echo 'required KVM capability unavailable: /dev/kvm'; exit 1; }",
);

/// The vhost-net device the VMM's networking needs.
const VHOST_NET_CAPABILITY: &str = concat!(
    "test -e /dev/vhost-net && test -r /dev/vhost-net && test -w /dev/vhost-net || ",
    "{ echo 'required Cloud Hypervisor vhost capability unavailable: /dev/vhost-net'; ",
    "exit 1; }",
);

/// The cgroup v2 unified hierarchy, readable as the daemon reads it.
const CGROUP_V2_CAPABILITY: &str = concat!(
    "test -r /sys/fs/cgroup/cgroup.controllers || ",
    "{ echo 'required cgroup v2 capability unavailable'; exit 1; }",
);

/// Every controller the daemon delegates has to be there.
const CGROUP_CONTROLLERS: &str = concat!(
    "for controller in cpu memory io pids cpuset; do ",
    "grep -qw \"$controller\" /sys/fs/cgroup/cgroup.controllers || ",
    "{ echo \"required cgroup controller unavailable: $controller\"; exit 1; }; ",
    "done",
);

/// The delegated `d2b.slice` posture: the slice exists and has the
/// three controllers the runners need delegated into it.
const DELEGATED_CGROUP_POSTURE: &str = concat!(
    "test -d /sys/fs/cgroup/d2b.slice && ",
    "grep -qw 'cpu' /sys/fs/cgroup/d2b.slice/cgroup.subtree_control && ",
    "grep -qw 'memory' /sys/fs/cgroup/d2b.slice/cgroup.subtree_control && ",
    "grep -qw 'pids' /sys/fs/cgroup/d2b.slice/cgroup.subtree_control || ",
    "{ echo 'required delegated d2b.slice cgroup posture unavailable'; exit 1; }",
);

/// The daemon's bundle resolver did not refuse the evaluated bundle.
const BUNDLE_RESOLVER_LOADED: &str = concat!(
    "! journalctl -u d2bd.service --no-pager -b 2>/dev/null ",
    "| grep -F 'Bundle resolver could not load'",
);

/// The installed artifact catalog declares this Guest's setup descriptor
/// and its store view, from the same file the daemon reads.
const GUEST_SETUP_DESCRIPTOR: &str = concat!(
    "test -r /etc/d2b/artifact-catalog.json && ",
    "jq -e '",
    "(.guestSetupDescriptors | any(.[]; ",
    ".zone == \"work\" and .guest == \"acceptance-guest\" and ",
    ".providerArtifactId == \"runtime-cloud-hypervisor\" and ",
    ".descriptor.providerRef == \"Provider/runtime-cloud-hypervisor\" and ",
    ".descriptor.systemArtifactId == \"acceptance-system\" and ",
    ".descriptor.childRoles == [\"vmm\", \"ch-api\", \"guest-control\", \"system\"])) and ",
    "",
    "(.guestClosures | any(.[]; ",
    ".zone == \"work\" and .guest == \"acceptance-guest\" and ",
    ".artifactId == \"acceptance-system\" and (.closurePaths | length > 0) and ",
    "(. as $guest | ($guest.closurePaths | index($guest.toplevel)) != null) and ",
    ".storeView.mountPoint == \"/nix/store\" and ",
    "(.storeView.root | endswith(\"/zones/work/guests/acceptance-guest/store-view\")) and ",
    "",
    "(.vmm.binaryPath | endswith(\"/bin/cloud-hypervisor\"))))' ",
    "/etc/d2b/artifact-catalog.json",
);

/// The per-Guest closure spec the brokered store sync reads.
const GUEST_CLOSURE: &str = concat!(
    "test -r /etc/d2b/closures/zones/work/acceptance-guest.json && ",
    "jq -e '",
    ".schemaVersion == \"v3\" and .artifactId == \"acceptance-system\" and ",
    "(.closurePaths | length > 0) and ",
    "(. as $guest | ($guest.closurePaths | index($guest.toplevel)) != null) and ",
    ".storeView.mountPoint == \"/nix/store\" and ",
    ".storeView.sync == \"broker-store-sync\" and ",
    "(.vmm.argv | index(\"--api-socket\")) != null' ",
    "/etc/d2b/closures/zones/work/acceptance-guest.json",
);

/// The zone's resource bundle carries the Guest row and no store path or
/// VMM argv.
const RESOURCE_BUNDLE: &str = concat!(
    "jq -e '",
    ".resources | any(.[]; .type == \"Guest\" and ",
    ".metadata.name == \"acceptance-guest\" and ",
    ".spec.providerRef == \"Provider/runtime-cloud-hypervisor\" and ",
    ".spec.systemArtifactId == \"acceptance-system\") and ",
    "all(.[]; (tostring | contains(\"/nix/store/\") | not) and ",
    "(tostring | contains(\"\\\"argv\\\"\") | not))' ",
    "/etc/d2b/zones/work/resource-bundle.json",
);

/// The Guest's own readiness loop, with its three terminal-failure exits:
/// a failed Guest, a failed VMM Process, and a terminal ComponentSession.
const GUEST_READY: &str = concat!(
    "for attempt in $(seq 1 45); do ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Guest >/run/d2b-guest-ready.json && ",
    "jq -e '",
    "(.resources | map(select(.type == \"Guest\" and ",
    ".metadata.name == \"acceptance-guest\"))) as $guests | ",
    "($guests | length) == 1 and ",
    "$guests[0].status.phase == \"Ready\" and ",
    "$guests[0].status.observedGeneration == $guests[0].metadata.generation and ",
    "$guests[0].status.resource.runtimeReady == true and ",
    "$guests[0].status.resource.bootstrapReady == true and ",
    "$guests[0].status.resource.activeProcessCount == 1' ",
    "/run/d2b-guest-ready.json && exit 0; ",
    "if jq -e 'any(.resources[]; ",
    ".metadata.name == \"acceptance-guest\" and ",
    ".status.phase == \"Failed\")' ",
    "/run/d2b-guest-ready.json >/dev/null; then ",
    "echo 'Guest reported a terminal failure'; exit 1; fi; ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Process >/run/d2b-vmm-fast-fail.json && ",
    "if jq -e 'any(.resources[]; ",
    ".metadata.name == \"acceptance-guest-vmm\" and ",
    ".status.phase == \"Failed\" and ",
    ".status.outcome.retryable != true)' ",
    "/run/d2b-vmm-fast-fail.json >/dev/null; then ",
    "echo 'VMM Process reported a terminal failure'; exit 1; fi; ",
    "if journalctl -u d2bd.service --no-pager -b ",
    "| grep -q 'session-authentication-failed\\|session-generation-stale'; then ",
    "echo 'ComponentSession reported a terminal failure'; exit 1; fi; ",
    "sleep 1; done; ",
    "echo 'Guest readiness failed:'; ",
    "jq -c '.resources[] | select(.type == \"Guest\" and ",
    ".metadata.name == \"acceptance-guest\") | ",
    "{name: .metadata.name, uid: .metadata.uid, ",
    "owner: .metadata.ownerRef, phase: .status.phase, ",
    "observedGeneration: .status.observedGeneration, ",
    "conditions: [.status.conditions[]? | ",
    "{type: .type, status: .status, reason: .reason}], ",
    "outcome: (.status.outcome | ",
    "if . == null then null else ",
    "{code: .code, retryable: .retryable} end), ",
    "resource: .status.resource}' ",
    "/run/d2b-guest-ready.json; ",
    "echo 'Dependent Process status:'; ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Process | ",
    "jq -c '.resources[] | ",
    "{name: .metadata.name, uid: .metadata.uid, ",
    "owner: .metadata.ownerRef, provider: .spec.providerRef, ",
    "execution: .spec.executionRef, processClass: .spec.processClass, ",
    "template: .spec.template, phase: .status.phase, ",
    "observedGeneration: .status.observedGeneration, ",
    "conditions: [.status.conditions[]? | ",
    "{type: .type, status: .status, reason: .reason}], ",
    "outcome: (.status.outcome | ",
    "if . == null then null else ",
    "{code: .code, retryable: .retryable} end), ",
    "resource: .status.resource}'; ",
    "for resource_type in Endpoint Volume Provider; do ",
    "echo \"$resource_type status:\"; ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list \"$resource_type\" | ",
    "jq -c '.resources[] | {name: .metadata.name, uid: .metadata.uid, ",
    "owner: .metadata.ownerRef, provider: .spec.providerRef, ",
    "execution: .spec.executionRef, phase: .status.phase, ",
    "observedGeneration: .status.observedGeneration, ",
    "conditions: [.status.conditions[]? | ",
    "{type: .type, status: .status, reason: .reason}], ",
    "outcome: (.status.outcome | ",
    "if . == null then null else ",
    "{code: .code, retryable: .retryable} end), ",
    "resource: .status.resource}'; done; exit 1",
);

/// The Guest's VMM runner Process and the runtime controller are both
/// `Ready`, and the Guest has exactly one child Process.
const GUEST_VMM_PROCESS_READY: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Process ",
    ">/run/d2b-process-ready.json && ",
    "jq -e '",
    "([.resources[] | select(.type == \"Process\" and ",
    ".metadata.ownerRef == \"Guest/acceptance-guest\")] | length == 1) and ",
    "([.resources[] | select(.type == \"Process\" and ",
    ".metadata.name == \"acceptance-guest-vmm\" and ",
    ".metadata.ownerRef == \"Guest/acceptance-guest\" and ",
    ".spec.providerRef == \"Provider/system-minijail\" and ",
    ".spec.executionRef == \"Host/host-system\" and ",
    ".spec.processClass == \"worker\" and ",
    ".spec.template == \"cloud-hypervisor-runner\" and ",
    ".status.phase == \"Ready\")] | length == 1) and ",
    "([.resources[] | select(.type == \"Process\" and ",
    ".metadata.ownerRef == \"Provider/runtime-cloud-hypervisor\" and ",
    ".spec.providerRef == \"Provider/system-minijail\" and ",
    ".spec.executionRef == \"Host/host-system\" and ",
    ".spec.processClass == \"controller\" and ",
    ".spec.template == \"controller-runtime-cloud-hypervisor-cloud-hypervisor-controller\" ",
    "and ",
    ".status.phase == \"Ready\")] | length == 1)' ",
    "/run/d2b-process-ready.json",
);

/// The Guest's two endpoints are `Ready`.
const GUEST_ENDPOINTS_READY: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Endpoint ",
    ">/run/d2b-endpoint-ready.json && ",
    "jq -e '",
    "([.resources[] | select(.type == \"Endpoint\" and ",
    ".metadata.ownerRef == \"Guest/acceptance-guest\" and ",
    ".status.phase == \"Ready\")] | length == 2)' ",
    "/run/d2b-endpoint-ready.json",
);

/// The Guest's system Volume is `Ready` from its `nix-closure` source.
const GUEST_VOLUME_READY: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Volume ",
    ">/run/d2b-volume-ready.json && ",
    "jq -e '",
    "([.resources[] | select(.type == \"Volume\" and ",
    ".metadata.name == \"acceptance-guest-system\" and ",
    ".metadata.ownerRef == \"Guest/acceptance-guest\" and ",
    ".spec.source.settings.kind == \"nix-closure\" and ",
    ".spec.source.settings.sourcePolicyId == null and ",
    ".spec.source.settings.systemArtifactId == \"acceptance-system\" and ",
    ".status.phase == \"Ready\" and ",
    ".status.observedGeneration == .metadata.generation)] | length == 1)' ",
    "/run/d2b-volume-ready.json",
);

/// The store view is its own Volume, owned by nobody, `Ready` and settled.
const STORE_VIEW_VOLUME: &str = concat!(
    "jq -e '",
    "([.resources[] | select(.type == \"Volume\" and ",
    ".metadata.name == \"store-view-acceptance-guest\" and ",
    ".metadata.ownerRef == null and ",
    ".spec.source.settings.kind == \"nix-closure\" and ",
    ".spec.source.settings.sourcePolicyId == null and ",
    ".spec.source.settings.systemArtifactId == \"acceptance-system\" and ",
    ".status.phase == \"Ready\" and ",
    ".status.observedGeneration == .metadata.generation)] | length == 1)' ",
    "/run/d2b-volume-ready.json",
);

/// The store view and the Guest's system Volume are two distinct rows.
const STORE_VIEW_UIDS: &str = concat!(
    "jq -e '",
    "([.resources[] | select(.type == \"Volume\" and ",
    "(.metadata.name == \"store-view-acceptance-guest\" or ",
    ".metadata.name == \"acceptance-guest-system\")) ",
    "| .metadata.uid]) as $uids | ",
    "$uids | length == 2 and (unique | length == 2)' ",
    "/run/d2b-volume-ready.json",
);

/// The declared `state` Volume's attachment is served end to end: exactly
/// one deterministically named binding, owned by the Volume, `Ready`
/// under a current fence.
const VOLUME_BINDING_READY: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list VolumeBinding ",
    ">/run/d2b-binding-ready.json && ",
    "jq -e '",
    "([.resources[] | select(.type == \"VolumeBinding\" and ",
    ".metadata.ownerRef == \"Volume/state\")] | length) == 1 and ",
    "([.resources[] | select(.type == \"VolumeBinding\" and ",
    ".metadata.name == \"vol-binding-6a8ea4307a30f7ceae6533f2\" and ",
    ".metadata.ownerRef == \"Volume/state\" and ",
    ".status.phase == \"Ready\" and ",
    ".status.observedGeneration == .metadata.generation and ",
    ".status.resource.ready == true and ",
    ".status.resource.fence.uid == .metadata.uid and ",
    ".status.resource.fence.generation == .metadata.generation and ",
    ".status.resource.fence.revision > 0)] | length) == 1' ",
    "/run/d2b-binding-ready.json",
);

/// The binding's virtiofs worker Process and its private endpoint are
/// `Ready`.
const BINDING_WORKER_READY: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Process ",
    ">/run/d2b-binding-worker.json; process_status=$?; ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Endpoint ",
    ">/run/d2b-binding-endpoint.json; endpoint_status=$?; ",
    "test \"$process_status\" -eq 0 && ",
    "test \"$endpoint_status\" -eq 0 && ",
    "jq -e '",
    "([.resources[] | select(.type == \"Process\" and ",
    ".metadata.ownerRef == ",
    "\"VolumeBinding/vol-binding-6a8ea4307a30f7ceae6533f2\" and ",
    ".spec.providerRef == \"Provider/system-minijail\" and ",
    ".spec.executionRef == \"Host/host-system\" and ",
    ".spec.processClass == \"worker\" and ",
    ".spec.template == \"virtiofsd-worker\" and ",
    ".status.phase == \"Ready\")] | length) == 1' ",
    "/run/d2b-binding-worker.json && ",
    "jq -e '",
    "([.resources[] | select(.type == \"Endpoint\" and ",
    ".metadata.ownerRef == ",
    "\"VolumeBinding/vol-binding-6a8ea4307a30f7ceae6533f2\" and ",
    ".status.phase == \"Ready\")] | length) == 1' ",
    "/run/d2b-binding-endpoint.json",
);

/// The VMM's API socket, as a socket.
const GUEST_API_SOCKET: &str = concat!(
    "test -S ",
    "/var/lib/d2b/zones/work/guests/acceptance-guest/acceptance-guest.sock",
);

/// The Guest's state directory listing, printed by both socket waits.
const GUEST_STATE_DIR: &str =
    "ls -la /var/lib/d2b/zones/work/guests/acceptance-guest/ 2>&1 || true";

/// The VMM's API socket and the store view's two current links exist.
const GUEST_STATE_CHAIN: &str = concat!(
    "test -S /var/lib/d2b/zones/work/guests/acceptance-guest/acceptance-guest.sock && ",
    "test -L /var/lib/d2b/zones/work/guests/acceptance-guest/store-view/state/current && ",
    "",
    "test -L /var/lib/d2b/zones/work/guests/acceptance-guest/store-view/meta/current && ",
    "",
    "test -d /var/lib/d2b/zones/work/guests/acceptance-guest/store-view/live",
);

/// The one runner serving this Guest: its pid and its start time, read from
/// `/proc` the fixture's own way.
const RUNNER_PROCESS: &str = concat!(
    "set -- $(for proc in /proc/[0-9]*; do ",
    "exe=$(readlink \"$proc/exe\" 2>/dev/null || true); ",
    "case \"$exe\" in */bin/cloud-hypervisor) ",
    "cmd=$(tr '\\0' ' ' < \"$proc/cmdline\"); ",
    "case \"$cmd\" in *--api-socket*acceptance-guest*) ",
    "pid=${proc#/proc/}; ",
    "printf '%s %s ' \"$pid\" \"$(awk '{print $22}' \"$proc/stat\")\";; ",
    "esac;; esac; done); ",
    "test \"$#\" -eq 2; printf '%s %s' \"$1\" \"$2\"",
);

/// The enrollment record read back before the restart: the Guest's uid,
/// the four generations, and the session generation the console logged.
const GUEST_SESSION_BEFORE: &str = concat!(
    "guest_uid=$(runuser -u alice -- env ",
    "D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Guest | ",
    "jq -er '.resources[] | select(.type == \"Guest\" and ",
    ".metadata.name == \"acceptance-guest\" and ",
    ".spec.providerRef == \"Provider/runtime-cloud-hypervisor\" and ",
    ".spec.executionRef == \"Host/host-system\") | .metadata.uid') && ",
    "jq -c ",
    "'{guestRef, guestUid, zone, reconnectGeneration, ",
    "providerGeneration, controllerGeneration, assignmentEpoch}' ",
    "/var/lib/d2b/zones/work/guests/acceptance-guest/component-session/guest.json ",
    ">/run/d2b-guest-session-before.json && ",
    "jq -e --arg guest_uid \"$guest_uid\" ",
    "'.guestRef == \"Guest/acceptance-guest\" and .guestUid == $guest_uid ",
    "and .zone == \"work\" and .reconnectGeneration > 0 and ",
    ".providerGeneration > 0 and .controllerGeneration > 0 and ",
    ".assignmentEpoch > 0' ",
    "/run/d2b-guest-session-before.json >/dev/null && ",
    "session_generation=$(journalctl --no-pager -b 2>/dev/null | ",
    "grep -F 'Guest ComponentSession Resource API server starting' | ",
    "grep -oE 'generation[[:space:]]*=[[:space:]]*[0-9]+' | ",
    "grep -oE '[0-9]+' | tail -1) && ",
    "test -n \"$session_generation\" && ",
    "test \"$session_generation\" -ge 1 && ",
    "printf '%s\\n' \"$session_generation\" ",
    ">/run/d2b-guest-session-generation-before",
);

/// The Guest target agent never reported a bundle-validation failure:
/// the console the Guest's read failures are forwarded into is clean.
const BUNDLE_VALIDATION_FAILED: &str = concat!(
    "journalctl --no-pager -o cat -b ",
    "| grep -F 'Guest process bundle validation failed'",
);

/// The restart boundary the adoption is asserted across.
const DAEMON_RESTART: &str = "systemctl restart d2bd.service";

/// The VMM's API socket is still there after the daemon restarted.
const API_SOCKET_AFTER_RESTART: &str = concat!(
    "test -S ",
    "/var/lib/d2b/zones/work/guests/acceptance-guest/acceptance-guest.sock",
);

/// The 60-attempt loop that waits for the Guest's session generation to
/// advance and the adopted Guest, VMM runner and controller to be `Ready`
/// again at the same uid.
const SESSION_GENERATION_ADVANCE: &str = concat!(
    "rm -f /run/d2b-guest-adopted.json /run/d2b-process-adopted.json; ",
    "for attempt in $(seq 1 60); do ",
    "session_generation_before=$(cat ",
    "/run/d2b-guest-session-generation-before) && ",
    "session_generation_after=$(journalctl --no-pager -b 2>/dev/null | ",
    "grep -F 'Guest ComponentSession Resource API server starting' | ",
    "grep -oE 'generation[[:space:]]*=[[:space:]]*[0-9]+' | ",
    "grep -oE '[0-9]+' | tail -1) && ",
    "test -n \"$session_generation_after\" && ",
    "test \"$session_generation_after\" -gt \"$session_generation_before\" && ",
    "jq -c '{guestRef, guestUid, zone, reconnectGeneration, ",
    "providerGeneration, controllerGeneration, assignmentEpoch}' ",
    "/var/lib/d2b/zones/work/guests/acceptance-guest/component-session/guest.json ",
    ">/run/d2b-guest-session-after.json && ",
    "jq -e --slurpfile expected /run/d2b-guest-session-before.json ",
    "'. == $expected[0]' /run/d2b-guest-session-after.json >/dev/null && ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Guest ",
    ">/run/d2b-guest-adopted.json && ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Process ",
    ">/run/d2b-process-adopted.json && ",
    "jq -e '",
    "([.resources[] | select(.type == \"Process\" and ",
    ".metadata.ownerRef == \"Guest/acceptance-guest\")] | length == 1) and ",
    "([.resources[] | select(.type == \"Process\" and ",
    ".metadata.name == \"acceptance-guest-vmm\" and ",
    ".metadata.ownerRef == \"Guest/acceptance-guest\" and ",
    ".spec.providerRef == \"Provider/system-minijail\" and ",
    ".spec.executionRef == \"Host/host-system\" and ",
    ".spec.processClass == \"worker\" and ",
    ".spec.template == \"cloud-hypervisor-runner\" and ",
    ".status.phase == \"Ready\")] | length == 1) and ",
    "([.resources[] | select(.type == \"Process\" and ",
    ".metadata.ownerRef == \"Provider/runtime-cloud-hypervisor\" and ",
    ".spec.providerRef == \"Provider/system-minijail\" and ",
    ".spec.executionRef == \"Host/host-system\" and ",
    ".spec.processClass == \"controller\" and ",
    ".spec.template == \"controller-runtime-cloud-hypervisor-cloud-hypervisor-controller\" ",
    "and ",
    ".status.phase == \"Ready\")] | length == 1)' ",
    "/run/d2b-process-adopted.json && ",
    "jq -e --slurpfile session /run/d2b-guest-session-after.json ",
    "'any(.resources[]; ",
    ".type == \"Guest\" and ",
    ".metadata.name == \"acceptance-guest\" and ",
    ".metadata.zone == \"work\" and ",
    ".metadata.uid == $session[0].guestUid and ",
    ".spec.providerRef == \"Provider/runtime-cloud-hypervisor\" and ",
    ".spec.executionRef == \"Host/host-system\" and ",
    ".status.phase == \"Ready\" and ",
    ".status.observedGeneration == .metadata.generation and ",
    ".status.resource.runtimeReady == true and ",
    ".status.resource.bootstrapReady == true and ",
    ".status.resource.activeProcessCount == 1)' ",
    "/run/d2b-guest-adopted.json && exit 0; ",
    "sleep 1; done; ",
    "echo 'Guest ComponentSession generation did not advance after restart:'; ",
    "printf 'before=%s after=%s\\n' ",
    "\"$(cat /run/d2b-guest-session-generation-before 2>/dev/null || true)\" ",
    "\"$(journalctl --no-pager -b 2>/dev/null | ",
    "grep -F 'Guest ComponentSession Resource API server starting' | ",
    "grep -oE 'generation[[:space:]]*=[[:space:]]*[0-9]+' | ",
    "grep -oE '[0-9]+' | tail -1)\"; ",
    "jq -c '.' /run/d2b-guest-session-before.json || true; ",
    "jq -c '.' /run/d2b-guest-session-after.json || true; ",
    "echo 'Post-restart Guest readiness failed:'; ",
    "jq -c '.resources[] | select(.type == \"Guest\" and ",
    ".metadata.name == \"acceptance-guest\") | ",
    "{name: .metadata.name, uid: .metadata.uid, ",
    "owner: .metadata.ownerRef, phase: .status.phase, ",
    "observedGeneration: .status.observedGeneration, ",
    "conditions: [.status.conditions[]? | ",
    "{type: .type, status: .status, reason: .reason}], ",
    "outcome: (.status.outcome | ",
    "if . == null then null else ",
    "{code: .code, retryable: .retryable} end), ",
    "resource: .status.resource}' ",
    "/run/d2b-guest-adopted.json || true; ",
    "echo 'Post-restart Process readiness failed:'; ",
    "jq -c '.resources[] | ",
    "{name: .metadata.name, uid: .metadata.uid, ",
    "owner: .metadata.ownerRef, provider: .spec.providerRef, ",
    "execution: .spec.executionRef, processClass: .spec.processClass, ",
    "template: .spec.template, phase: .status.phase, ",
    "observedGeneration: .status.observedGeneration, ",
    "conditions: [.status.conditions[]? | ",
    "{type: .type, status: .status, reason: .reason}], ",
    "outcome: (.status.outcome | ",
    "if . == null then null else ",
    "{code: .code, retryable: .retryable} end), ",
    "resource: .status.resource}' ",
    "/run/d2b-process-adopted.json || true; ",
    "exit 1",
);

/// The journal sources that explain a refused [`SESSION_GENERATION_ADVANCE`],
/// as the prelude's `diag_step` takes them.
///
/// The command can be refused four ways, and one dump answers all of them:
/// the Guest's own session and component lines, the controller session setup
/// warn that carries the Provider and the stage, the authentication
/// closures that carry the stage, and the ComponentSession line the loop
/// itself greps the generation out of.
const SESSION_GENERATION_EXPLAIN: &[DiagRow<'static>] = &[
    ("d2bd.service", "acceptance-guest"),
    // A session that never went live refuses with a stage, and the stage is
    // the only line that says which handshake step was unavailable. It
    // carries the Provider it was for.
    ("d2bd.service", "ResourceV3 session"),
    ("d2bd.service", "controller authentication"),
    // The generation the loop compares is the one the Guest's own
    // ComponentSession logged, so that line is the loop's own input.
    ("d2bd.service", "ComponentSession"),
];

/// Controller-session and Process launch evidence needed when the Guest VMM
/// wait finds the runtime controller still Pending.
const GUEST_VMM_PROCESS_EXPLAIN: &[DiagRow<'static>] = &[
    ("d2bd.service", "external Provider controller"),
    ("d2bd.service", "controller session reconciliation degraded"),
    ("d2bd.service", "controller assignment"),
    ("d2bd.service", "controller session service task finished"),
    ("d2bd.service", "supervisor launch effect failed"),
    ("d2bd.service", "broker refused a process request"),
    ("d2bd.service", "broker spawn invocation failed"),
    ("d2bd.service", "broker transport failed for a process request"),
    ("d2bd.service", "process provider effect failed"),
    ("d2bd.service", "process launch failed"),
    ("d2bd.service", "launch request rejected"),
    ("d2bd.service", "forwarded invocation refused with a reason"),
    ("d2bd.service", "broker observe invocation failed"),
    (
        "d2b-broker.service",
        "ObserveRunner registered runner verification is incomplete",
    ),
    ("d2b-broker.service", "runner process identity changed"),
    ("d2b-broker.service", "spawn"),
];

/// The Guest's deletion, retried the fixture's own 30 attempts.
const GUEST_DELETE: &str = concat!(
    "for attempt in $(seq 1 30); do ",
    "guest_revision=$(runuser -u alice -- env ",
    "D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Guest ",
    "| jq -er '.resources[] | select(.type == \"Guest\" and ",
    ".metadata.name == \"acceptance-guest\") | .metadata.revision') && ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json delete Guest/acceptance-guest ",
    "--revision \"$guest_revision\" ",
    ">/run/d2b-guest-delete.json 2>/run/d2b-guest-delete.err && exit 0; ",
    "sleep 1; done; ",
    "echo 'Guest deletion did not complete within 30s:'; ",
    "jq -c '{resourceRef: .resourceRef, revision: .revision}' ",
    "/run/d2b-guest-delete.json || true; ",
    "echo 'last delete stderr:'; ",
    "cat /run/d2b-guest-delete.err 2>/dev/null || true; ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Guest | ",
    "jq -c '.resources[] | select(.type == \"Guest\" and ",
    ".metadata.name == \"acceptance-guest\") | ",
    "{name: .metadata.name, uid: .metadata.uid, ",
    "owner: .metadata.ownerRef, phase: .status.phase, ",
    "observedGeneration: .status.observedGeneration, ",
    "conditions: [.status.conditions[]? | ",
    "{type: .type, status: .status, reason: .reason}], ",
    "outcome: (.status.outcome | ",
    "if . == null then null else ",
    "{code: .code, retryable: .retryable} end), ",
    "resource: .status.resource}' || true; ",
    "exit 1",
);

/// The deletion was accepted for the Guest the check deleted.
const GUEST_DELETE_REVISION: &str = concat!(
    "jq -e '.resourceRef == \"Guest/acceptance-guest\" and ",
    ".revision > 0' ",
    "/run/d2b-guest-delete.json",
);

/// The Guest's deletion was recorded, so its drain has begun.
const GUEST_DRAINING: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json reconcile Guest/acceptance-guest ",
    ">/run/d2b-guest-finalize.json 2>/dev/null || true; ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Guest ",
    ">/run/d2b-guest-draining.json && ",
    "jq -e 'any(.resources[]; .type == \"Guest\" and ",
    ".metadata.name == \"acceptance-guest\" and ",
    ".metadata.deletionRequestedAt != null)' ",
    "/run/d2b-guest-draining.json",
);

/// The Guest row is gone.
const GUEST_DRAINED: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json reconcile Guest/acceptance-guest ",
    ">/run/d2b-guest-finalize.json 2>/dev/null || true; ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Guest ",
    "| jq -e 'all(.resources[]; .metadata.name != \"acceptance-guest\")'",
);

/// The Guest's VMM runner Process is gone.
const GUEST_VMM_PROCESS_DRAINED: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Process ",
    "| jq -e 'all(.resources[]; .metadata.name != \"acceptance-guest-vmm\")'",
);

/// The VMM's API socket is gone once the Guest drained.
const GUEST_API_SOCKET_GONE: &str =
    "test ! -S /var/lib/d2b/zones/work/guests/acceptance-guest/acceptance-guest.sock";

/// The owning Volume's deletion, which drives the binding's drain.
const VOLUME_STATE_DELETE: &str = concat!(
    "volume_revision=$(runuser -u alice -- env ",
    "D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Volume | ",
    "jq -er '.resources[] | select(.type == \"Volume\" and ",
    ".metadata.name == \"state\") | .metadata.revision') && ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json delete Volume/state ",
    "--revision \"$volume_revision\" >/run/d2b-volume-state-delete.json",
);

/// The binding's deletion was recorded, so its drain has begun.
const VOLUME_BINDING_DRAINING: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list VolumeBinding ",
    ">/run/d2b-binding-draining.json && ",
    "jq -e 'any(.resources[]; .type == \"VolumeBinding\" and ",
    ".metadata.name == \"vol-binding-6a8ea4307a30f7ceae6533f2\" and ",
    ".metadata.deletionRequestedAt != null)' ",
    "/run/d2b-binding-draining.json",
);

/// The binding, its worker Process and its endpoint are all gone.
const VOLUME_BINDING_DRAINED: &str = concat!(
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list VolumeBinding ",
    ">/run/d2b-binding-drained.json && ",
    "jq -e 'all(.resources[]; .metadata.ownerRef != \"Volume/state\")' ",
    "/run/d2b-binding-drained.json && ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Process | ",
    "jq -e 'all(.resources[]; .metadata.ownerRef != ",
    "\"VolumeBinding/vol-binding-6a8ea4307a30f7ceae6533f2\")' && ",
    "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock ",
    "d2b --zone work --json list Endpoint | ",
    "jq -e 'all(.resources[]; .metadata.ownerRef != ",
    "\"VolumeBinding/vol-binding-6a8ea4307a30f7ceae6533f2\")'",
);

/// The check's assertions, in the order its fixture made them.
pub fn assertions(control: &mut GuestControl) -> LegacyResult<()> {
    control.stage("boot");
    control.diag_unit("daemon-up", "d2bd.service", DAEMON_BOUND)?;
    control.wait_for_unit("d2b-broker.socket", None, SOCKET_BOUND)?;
    control.wait_for_file("/run/d2b/public.sock", SOCKET_BOUND)?;
    control.succeed(&[BROKER_START], None)?;
    control.diag_unit("broker-service", "d2b-broker.service", BROKER_BOUND)?;

    // Capture only the public, redacted Resource projection before waiting on
    // the nested VMM. This keeps a missing API socket diagnostic without
    // waiting for unrelated fixture controller sessions.
    control.stage("preflight-capture");
    control.succeed(&[PREFLIGHT_CAPTURE], None)?;
    control.diag("cat /run/d2b-preflight-summary.log", "preflight summary");

    // Both storage Providers use the authenticated host acceptance controller.
    // Their separate owner identities must establish live ResourceV3 sessions;
    // a stale pause fixture would leave these controller Processes pending.
    let summary = summary_rows();
    let process_rows = live_rows("Process rows", "Process");
    let volume_rows = live_rows("Volume rows", "Volume");
    let guest_rows = live_rows("Guest rows", "Guest");
    let state_dir = ("guest state dir", GUEST_STATE_DIR);
    let controller_rows = saved_rows(
        "Controller Process rows",
        "/run/d2b-volume-controller-processes.json",
    );
    control.diag_wait(
        "controller-sessions",
        CONTROLLER_SESSIONS,
        CONTROLLER_BOUND,
        &[row(&summary), row(&process_rows)],
        &[
            ("d2bd.service", "ResourceV3 session"),
            // A session that never went live refuses with a stage, and the
            // stage is the only line that says which authentication step was
            // unavailable. The token above does not match it.
            ("d2bd.service", "controller authentication"),
        ],
    )?;
    // The rows hold the phase and generation the status was published for,
    // per controller row.
    control.diag_wait(
        "volume-controller-processes",
        VOLUME_CONTROLLER_PROCESSES,
        CONTROLLER_BOUND,
        &[row(&controller_rows), row(&summary)],
        // A controller Process the supervisor refused names the effect error
        // it projected and the broker backend names the leg under it; the
        // template name appears in no journal line, so it cannot explain one.
        &[
            ("d2bd.service", "external Provider controller"),
            ("d2bd.service", "supervisor launch effect failed"),
            ("d2bd.service", "broker refused a process request"),
            ("d2bd.service", "broker spawn invocation failed"),
            ("d2bd.service", "broker transport failed for a process request"),
            ("d2bd.service", "process provider effect failed"),
            ("d2bd.service", "process launch failed"),
            ("d2bd.service", "launch request rejected"),
            // The relay drops a refusal's detail from the response envelope
            // and logs it here instead, so this is the only line that says
            // WHY the broker refused a spawn.
            ("d2bd.service", "forwarded invocation refused with a reason"),
        ],
    )?;
    control.succeed(&[ACCEPTANCE_CONTROLLERS], None)?;

    // The VMM API socket is the first nested-VM proof. Volume convergence can
    // legitimately precede the nested boot, so keep this bound aligned with
    // Guest readiness rather than failing before the U7 Runner re-enters.
    control.stage("nested-vmm-api-socket");
    control.diag_run(
        "nested-vmm-api-socket",
        NESTED_VMM_API_SOCKET,
        &[
            ("live process table", "ps -eo pid=,ppid=,args= --no-headers 2>/dev/null | head -n 80 || true"),
            ("guest state rows", "find /var/lib/d2b/zones/work/guests -maxdepth 3 2>/dev/null | head -n 60 || true"),
        ],
        &[("d2bd.service", ""), ("d2b-broker.service", "")],
    )?;
    control.diag_wait(
        "guest-console-boot-id",
        GUEST_CONSOLE_BOOT_ID,
        CONSOLE_BOUND,
        &[("host journal tail", HOST_JOURNAL_TAIL)],
        &[("", "D2B_GUEST_BOOT_ID")],
    )?;
    control.sleep(5)?;

    // The Guest's enrollment, then the host posture the nested boot needs.
    control.stage("guest-session-enrollment");
    control.succeed(&[GUEST_SESSION_ENROLLMENT], None)?;
    control.succeed(&[KVM_CAPABILITY], None)?;
    control.succeed(&[VHOST_NET_CAPABILITY], None)?;
    control.succeed(&[CGROUP_V2_CAPABILITY], None)?;
    control.succeed(&[CGROUP_CONTROLLERS], None)?;
    control.succeed(&[DELEGATED_CGROUP_POSTURE], None)?;
    control.succeed(&[BUNDLE_RESOLVER_LOADED], None)?;
    control.succeed(&[GUEST_SETUP_DESCRIPTOR], None)?;
    control.succeed(&[GUEST_CLOSURE], None)?;
    control.succeed(&[RESOURCE_BUNDLE], None)?;

    // The Guest's readiness, then each of its rows in the order the fixture
    // declared them: the VMM and controller processes, the endpoints, the
    // system Volume, the store view, and the declared Volume's binding.
    control.stage("guest-ready");
    control.succeed(&[GUEST_READY], None)?;
    let process_ready_rows = saved_rows("Process rows", "/run/d2b-process-ready.json");
    control.diag_wait(
        "guest-vmm-process-ready",
        GUEST_VMM_PROCESS_READY,
        GUEST_BOUND,
        &[row(&process_ready_rows)],
        GUEST_VMM_PROCESS_EXPLAIN,
    )?;
    let endpoint_ready_rows = saved_rows("Endpoint rows", "/run/d2b-endpoint-ready.json");
    control.diag_wait(
        "guest-endpoints-ready",
        GUEST_ENDPOINTS_READY,
        GUEST_BOUND,
        &[row(&endpoint_ready_rows)],
        &[("d2bd.service", "Guest/acceptance-guest")],
    )?;
    let volume_ready_rows = saved_rows("Volume rows", "/run/d2b-volume-ready.json");
    control.diag_wait(
        "guest-volume-ready",
        GUEST_VOLUME_READY,
        GUEST_VOLUME_BOUND,
        &[row(&volume_ready_rows)],
        &[("d2bd.service", "acceptance-guest-system")],
    )?;
    control.succeed(&[STORE_VIEW_VOLUME], None)?;
    control.succeed(&[STORE_VIEW_UIDS], None)?;

    // U7: the declared Volume/state attachment is served end to end through
    // the neutral binding chain. The Volume side mints exactly one
    // deterministically named binding owned by the Volume, and only a
    // current fence (binding UID and generation) can report it ready.
    let binding_ready_rows = saved_rows("VolumeBinding rows", "/run/d2b-binding-ready.json");
    control.diag_wait(
        "volume-binding-ready",
        VOLUME_BINDING_READY,
        BINDING_BOUND,
        &[row(&binding_ready_rows), row(&volume_rows)],
        &[("d2bd.service", "vol-binding-6a8ea4307a30f7ceae6533f2")],
    )?;
    // The virtiofs serving side owns only its worker Process and private
    // Endpoint as binding-owned children; the worker adopts the per-Volume
    // vfd principal synthesized from the declared attachment.
    let binding_worker_rows = saved_rows("Process rows", "/run/d2b-binding-worker.json");
    let binding_endpoint_rows = saved_rows("Endpoint rows", "/run/d2b-binding-endpoint.json");
    control.diag_wait(
        "binding-worker-ready",
        BINDING_WORKER_READY,
        BINDING_WORKER_BOUND,
        &[
            row(&binding_worker_rows),
            row(&binding_endpoint_rows),
            (
                "virtiofsd process table",
                "ps -eo pid=,ppid=,stat=,args= --no-headers 2>/dev/null | grep -F virtiofsd || true",
            ),
            (
                "virtiofsd socket tree",
                "find /run/d2b/vms/acceptance-guest -maxdepth 3 -printf '%M %u:%g %p -> %l\n' 2>/dev/null || true",
            ),
            (
                "virtiofsd runtime ACLs",
                "getfacl -pn /run/d2b/vms/acceptance-guest 2>/dev/null | head -40 || true",
            ),
            (
                "broker child reaped records",
                "grep -h 'ChildReaped' /var/lib/d2b/audit/broker-*.jsonl 2>/dev/null | tail -20 || true",
            ),
        ],
        &[
            ("d2bd.service", "vol-binding-6a8ea4307a30f7ceae6533f2"),
            ("d2bd.service", "virtiofsd"),
            ("d2bd.service", "supervisor launch effect failed"),
            ("d2bd.service", "broker refused a process request"),
            ("d2bd.service", "broker spawn invocation failed"),
            ("d2bd.service", "broker transport failed for a process request"),
            ("d2bd.service", "process provider effect failed"),
            ("d2bd.service", "process launch failed"),
            ("d2bd.service", "launch request rejected"),
            ("d2bd.service", "forwarded invocation refused with a reason"),
            ("d2bd.service", "reply timeout"),
            ("d2bd.service", "broker observe invocation failed"),
            ("d2bd.service", "reserve: reclaiming"),
            ("d2bd.service", "broker pidfd reply carried no result"),
            ("d2b-broker.service", "virtiofsd"),
        ],
    )?;
    control.diag_wait(
        "guest-api-socket",
        GUEST_API_SOCKET,
        API_SOCKET_BOUND,
        &[row(&summary), (state_dir.0, state_dir.1)],
        &[("d2bd.service", "acceptance-guest")],
    )?;
    control.succeed(&[GUEST_STATE_CHAIN], None)?;

    // The runner process the restart below has to find again: its pid and its
    // start time, so the adoption is about the same process and not about a
    // search that found a replacement.
    let runner = control.succeed(&[RUNNER_PROCESS], None)?;
    let runner_fields = runner_fields(&runner)?;
    let runner_pid = runner_fields[0];
    let runner_start = runner_fields[1];
    control.succeed(&[&format!("test -d /proc/{runner_pid}")], None)?;
    control.succeed(
        &[&format!("test \"$(awk '{{print $22}}' /proc/{runner_pid}/stat)\" = {runner_start}")],
        None,
    )?;
    control.succeed(
        &[&format!("tr '\\0' ' ' < /proc/{runner_pid}/cmdline | grep -F -- '--api-socket' | \
             grep -F -- 'acceptance-guest'")],
        None,
    )?;

    // The enrollment record read back before the restart, and the failure the
    // Guest's own console must not have reported.
    control.succeed(&[GUEST_SESSION_BEFORE], None)?;

    // The Guest target agent boots from its enrolled bundle and key pair, and
    // the Guest console is forwarded into the host journal: a read it cannot
    // make fails closed inside the Guest, so the host journal must never
    // carry that failure. This is the gate the shell-pool fixture cannot
    // provide (no vsock device there).
    control.fail(&[BUNDLE_VALIDATION_FAILED], None)?;

    // The restart boundary: the same runner process, and a Guest whose session
    // generation advanced behind it.
    control.stage("restart-adoption");
    control.succeed(&[DAEMON_RESTART], None)?;
    control.diag_unit("daemon-restarted", "d2bd.service", DAEMON_BOUND)?;
    control.wait_for_file("/run/d2b/public.sock", SOCKET_BOUND)?;
    control.diag_wait(
        "api-socket-after-restart",
        API_SOCKET_AFTER_RESTART,
        API_SOCKET_BOUND,
        &[row(&guest_rows), (state_dir.0, state_dir.1)],
        &[("d2bd.service", "acceptance-guest")],
    )?;
    control.succeed(&[&format!("test -d /proc/{runner_pid}")], None)?;
    control.succeed(
        &[&format!("test \"$(awk '{{print $22}}' /proc/{runner_pid}/stat)\" = {runner_start}")],
        None,
    )?;
    control.stage("session-generation-advance");
    // Both commands below assert a settled state, so a refusal here is a
    // fact about the Guest rather than a wait that ran out, and it is the
    // one failure this check cannot explain from its own output: the loop
    // can be refused by the session, by the controller handshake behind it,
    // or by the Process it reads back, and the count by a runner the search
    // replaced. The journal sources are what say which, so the stage reports
    // its own cause instead of naming only the command that noticed it.
    control.diag_run(
        "session-generation-advance/adopted-session-generation",
        SESSION_GENERATION_ADVANCE,
        &[row(&guest_rows), row(&process_rows)],
        SESSION_GENERATION_EXPLAIN,
    )?;
    control.diag_run(
        "session-generation-advance/runner-process-count",
        &format!("set -- $(for proc in /proc/[0-9]*; do exe=$(readlink \"$proc/exe\" \
             2>/dev/null || true); case \"$exe\" in */bin/cloud-hypervisor) cmd=$(tr \
             '\\0' ' ' < \"$proc/cmdline\"); case \"$cmd\" in \
             *--api-socket*acceptance-guest*) pid=${{proc#/proc/}}; printf '%s %s ' \
             \"$pid\" \"$(awk '{{print $22}}' \"$proc/stat\")\";; esac;; esac; \
             done); test \"$#\" -eq 2 && test \"$1\" = {runner_pid} && test \"$2\" = \
             {runner_start}"),
        &[row(&process_rows)],
        // The count is about the runner, and the runner's lifecycle is what
        // the Guest's own lines record.
        &[("d2bd.service", "acceptance-guest")],
    )?;

    // The Guest's teardown, then the Volume's: both drain, and neither leaves
    // a socket, a process or a binding behind.
    control.stage("guest-teardown");
    control.succeed(&[GUEST_DELETE], None)?;
    control.succeed(&[GUEST_DELETE_REVISION], None)?;
    let draining_rows = saved_rows("Guest rows", "/run/d2b-guest-draining.json");
    control.diag_wait(
        "guest-draining",
        GUEST_DRAINING,
        DRAINING_BOUND,
        &[row(&draining_rows), row(&summary)],
        &[("d2bd.service", "acceptance-guest")],
    )?;
    control.diag_wait(
        "guest-drained",
        GUEST_DRAINED,
        DRAINED_BOUND,
        &[row(&guest_rows)],
        &[("d2bd.service", "acceptance-guest")],
    )?;
    control.diag_wait(
        "guest-vmm-process-drained",
        GUEST_VMM_PROCESS_DRAINED,
        GUEST_BOUND,
        &[row(&process_rows)],
        &[("d2bd.service", "acceptance-guest-vmm")],
    )?;
    control.succeed(&[GUEST_API_SOCKET_GONE], None)?;

    // U7 teardown (F2/AE6): deleting the owning Volume drives the binding
    // through its drain: deletion is requested first, then the worker and
    // the private endpoint are gone before the binding disappears, leaving
    // no orphaned serving effects.
    control.stage("volume-teardown");
    control.succeed(&[VOLUME_STATE_DELETE], None)?;
    let binding_draining_rows = saved_rows(
        "VolumeBinding rows",
        "/run/d2b-binding-draining.json",
    );
    control.diag_wait(
        "volume-binding-draining",
        VOLUME_BINDING_DRAINING,
        DRAINING_BOUND,
        &[row(&binding_draining_rows)],
        &[("d2bd.service", "vol-binding-6a8ea4307a30f7ceae6533f2")],
    )?;
    let binding_drained_rows = saved_rows(
        "VolumeBinding rows",
        "/run/d2b-binding-drained.json",
    );
    control.diag_wait(
        "volume-binding-drained",
        VOLUME_BINDING_DRAINED,
        BINDING_DRAIN_BOUND,
        &[
            row(&binding_drained_rows),
            row(&process_rows),
            row(&live_rows("Endpoint rows", "Endpoint")),
        ],
        &[("d2bd.service", "vol-binding-6a8ea4307a30f7ceae6533f2")],
    )?;

    Ok(())
}

/// The live rows of one resource type, as the fixture's own `live_rows` built
/// them: a labelled jq projection of what the public surface answers.
fn live_rows(label: &str, resource_type: &str) -> (String, String) {
    (
        label.to_owned(),
        format!(
            "runuser -u alice -- env D2B_PUBLIC_SOCKET=/run/d2b/public.sock \
             d2b --zone work --json list {resource_type} 2>/dev/null | \
             jq -c '{DIAG_PROJECTION}' 2>/dev/null || true"
        ),
    )
}

/// The rows of a file the check just saved, as the fixture's own `saved_rows`
/// built them: the same projection, falling back to the file itself.
fn saved_rows(label: &str, path: &str) -> (String, String) {
    (
        label.to_owned(),
        format!(
            "jq -c '{DIAG_PROJECTION}' {path} 2>/dev/null \
             || cat {path} 2>/dev/null || true"
        ),
    )
}

/// The preflight capture's own rows, as the fixture's `summary_rows` built
/// them.
fn summary_rows() -> (String, String) {
    (
        "preflight summary".to_owned(),
        "cat /run/d2b-preflight-summary.log 2>/dev/null || true".to_owned(),
    )
}

/// One row of a labelled pair, as the diagnostics rows are passed.
fn row<'a>(pair: &'a (String, String)) -> DiagRow<'a> {
    (pair.0.as_str(), pair.1.as_str())
}

/// The pid and the start time the runner search answered with, unpacked the
/// way the fixture unpacked them - on the same terms, too: a search that
/// answered with any other number of fields is the failure the fixture's own
/// `runner.split()` raised.
fn runner_fields(runner: &str) -> LegacyResult<Vec<&str>> {
    let fields = runner.split_whitespace().collect::<Vec<_>>();
    if fields.len() == 2 {
        return Ok(fields);
    }
    let complaint = if fields.len() < 2 {
        "not enough values to unpack"
    } else {
        "too many values to unpack"
    };
    Err(LegacyError::Assertion(format!(
        "{complaint} (expected 2, got {})",
        fields.len()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy::test_support::{answering, journal_token, lines_matching};

    /// The d2bd lines the `session-generation-advance` stage's explain
    /// sources select, in the form the daemon writes them.
    ///
    /// The last line is the point of the negative case below: it is a real
    /// `d2bd.service` line that no explain source asks for, so a report
    /// carrying it would be a report that dumped the whole unit rather than
    /// the sources it named.
    const FAKE_JOURNAL: &str = concat!(
        "Jul 29 12:00:01 host d2bd[1]: external Provider controller ResourceV3 session setup failed provider=Provider/system-minijail stage=open\n",
        "Jul 29 12:00:02 host d2bd[1]: external Provider controller authentication failed zone=work stage=load-guest-credential\n",
        "Jul 29 12:00:03 host d2bd[1]: Guest ComponentSession Resource API server starting generation=41\n",
        "Jul 29 12:00:04 host d2bd[1]: unrelated broker line that names no explain source\n",
    );

    /// A guest that refuses the stage's command and answers the explanation
    /// the way the guest's own shell would.
    ///
    /// The stage's command is answered with a non-zero status, which is what
    /// makes the assertion refuse and the explanation run. The journal dumps
    /// that follow are answered out of the fake journal and filtered by the
    /// token each dump carries - the guest's shell is what applies that
    /// filter, so a test that ignored it would be asserting against an
    /// answer the real guest never gives. Anything else, the composed zone
    /// explanation included, is refused, which is what a diagnostic is.
    fn refusing_guest(command: &str) -> (i32, String) {
        // A dump is the prelude's own `journalctl -u UNIT ...` form. The
        // stage's command greps `journalctl --no-pager -b` for the
        // generation instead, so the unit-scoped form is what tells the two
        // apart: without it the stage's own command would be answered as a
        // dump and would succeed.
        if command.contains("journalctl -u ") {
            let token = journal_token(command);
            return (0, lines_matching(FAKE_JOURNAL, token.as_deref()).join("\n"));
        }
        (1, String::new())
    }

    /// The report a refused `session-generation-advance` stage leaves behind.
    ///
    /// The stage is driven through the same [`SESSION_GENERATION_EXPLAIN`]
    /// the check itself passes, so what the assertions below read is the
    /// report this stage's own explain sources produce.
    fn session_generation_failure() -> String {
        let (outcome, notes, asked, _) = answering(refusing_guest, |control| {
            control
                .diag_run(
                    "session-generation-advance/adopted-session-generation",
                    SESSION_GENERATION_ADVANCE,
                    &[],
                    SESSION_GENERATION_EXPLAIN,
                )
                .map(|_| ())
        });
        assert!(outcome.is_err(), "the stage was expected to refuse");
        format!("{notes}{}", asked.concat())
    }

    /// A refused stage explains itself from the journal sources it named.
    ///
    /// This is the failure that was undiagnosable: the stage reported that
    /// it failed and named nothing else, so learning which session,
    /// handshake or generation had gone wrong meant reading the journal by
    /// hand. Every line a source names must now reach the check's own report.
    #[test]
    fn a_refused_generation_advance_names_its_journal_sources() {
        let report = session_generation_failure();
        for source in [
            "ResourceV3 session setup failed",
            "external Provider controller authentication failed",
            "Guest ComponentSession Resource API server starting",
        ] {
            assert!(
                report.contains(source),
                "the stage's own report must carry its journal source {source:?}:\n{report}"
            );
        }
    }

    /// A source that names no line the journal holds explains nothing.
    ///
    /// Without this the test above cannot tell a real filter from a dump of
    /// the whole unit, which is what an empty explain list amounts to: the
    /// journal would carry everything, and every assertion above would pass.
    #[test]
    fn an_explanation_reports_the_sources_it_named_and_nothing_else() {
        let report = session_generation_failure();
        assert!(
            !report.contains("unrelated broker line"),
            "a source that names no line must not drag the rest of the unit \
             into the report:\n{report}"
        );
    }
}
